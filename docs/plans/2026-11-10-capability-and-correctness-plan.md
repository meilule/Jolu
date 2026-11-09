# 能力汇聚与转发正确性 · 实施规划（2026-11-10）

**交给：鼠鼠**
**参照物：** [yetone/magpie](https://github.com/yetone/magpie) @ `8016124`（2026-11-10）
**Jolu 基线：** `d4ec7aa`（后端 1154 / 前端 444 全绿）
**本文定位：** 可执行规划——每项给出「magpie 在哪、怎么翻、验收什么、风险是什么」，不是差距清单（差距清单见 `2026-11-09-magpie-parity-full-audit-authoritative.md`）

---

## 0. 两条线的关系

```
转发正确性  ── 是能力汇聚的地基
             （搜索工具要改请求 body、要多轮转发，
               基础转发有 bug 时叠功能只会让故障更难定位）

能力汇聚    ── 是转发正确性的放大器
             （转发对了但模型没工具，等于便宜模型缺手缺脚）
```

**必须先做转发正确性。** 理由具体：工具拦截要往 `body` 里塞 `tool_result`、要把一次客户端请求变成多次上游请求；如果 thinking 块本来就带着空签名在转发，多轮循环会让这个 bug 的触发频率翻倍。

---

## 第一部分 · 转发正确性（约 3.5 天）

### P0-1 无签名 thinking 块过滤（半天）★ 最高优先

**问题（Jolu 实测确认）**

Jolu 有**两条**产生空签名 thinking 块的路径，且转发时零过滤：

| 位置 | 代码 | 场景 |
|---|---|---|
| `translate.rs:264` | `json!({"type":"thinking","thinking":r,"signature":""})` | OpenAI 非流式响应 → Anthropic 协议（重建 thinking 块） |
| `translate.rs:355` | `self.open_block(out, Kind::Thinking, json!({"type":"thinking","thinking":"","signature":""}))` | OpenAI 流式 delta → Anthropic SSE（`reasoning_content` 转 `thinking_delta`） |

同时 `translate.rs:148` 在 Anthropic → OpenAI 方向把 thinking **文本**塞进 `reasoning_content`，**签名直接丢弃**。所以一次跨模型往返后，thinking 块必然是空签名。

**后果**

用户在分组里从 OpenAI 系模型（Grok / DeepSeek / GLM）切回 Claude：Claude Code 保留了上一轮的 thinking 块，带着空签名发给真 Claude，**Claude 侧回 400 `invalid_request_error`**——而且这个错误信息通常不含线索，magpie 的记录是「连续 13 次裸报错」（citricdu case）。

**magpie 的解法**（`thinking_bound.go::withoutForeignThinking`，仅 15 行核心逻辑）

```go
func withoutForeignThinking(p provider.Provider, body []byte) []byte {
    if !bytes.Contains(body, []byte(`"signature"`)) { return body }   // 快速短路
    model := modelOf(body)
    // 只有「上游确实服务 Claude」才需要过滤
    if anthropicReplaysThinking(p.Host(), model) || p.Host() != "api.anthropic.com" && !anthropicModel.MatchString(model) {
        return body
    }
    return withoutThinkingBlocks(body, func(_, _ int, b gjson.Result) bool {
        return b.Get("type").String() == "thinking" && deepseekSignature.MatchString(b.Get("signature").String())
    })
}
// deepseekSignature = UUID 格式；Claude 自己的签名是 base64，800+ 字符，绝不会是 UUID
```

**关键判断**：用**签名格式**区分来源，不用模型名。
- Claude 的签名 = base64，长（Claude Code 的实测 800+ 字符）
- DeepSeek 的 = UUID
- 空字符串 = 一定不是 Claude 的

**Jolu 的翻译方案**

新增 `backend/src/gateway/thinking_guard.rs`：

```rust
/// 判断这次转发是否「上游真的服务 Claude」——只有那时才需要担心签名。
fn serves_claude(target_base: &str, model: &str) -> bool
// 规则：base_url 是 api.anthropic.com，或 model 名匹配 claude-*（含 Bedrock/Vertex 的 id 形态）

/// 对发往 Claude 的请求体，剔除非 Claude 签名的 thinking 块。
pub(crate) fn strip_foreign_thinking(body: &mut Value) -> usize
// 逐条 assistant 消息：content 数组里 type=="thinking" 且 signature
//   - 为空 ""
//   - 或匹配 UUID 格式
// → 移除该块；若该消息 content 变空，整条移除
// 保留：text / tool_use / tool_result，以及后面跟着的 tool_result
```

**接入点**：`wire.rs` 的转发前，与 `mask::prepare_request` 同一层（顺序在后，脱敏不关心签名）。

**验收标准**

1. 单测：UUID 签名块被移除、空签名字符串块被移除、base64 长签名保留、`redacted_thinking` 保留、移除后空消息整条消失
2. 单测：`serves_claude` 对 `api.anthropic.com` / `claude-sonnet-5` / `us.anthropic.claude-*`（Bedrock）/ `claude-3-5-sonnet@vertex` 为 true；对 `api.deepseek.com` / `glm-5.3` 为 false
3. 集成：模拟「Grok 产出 thinking → 分组切到 Claude」，发到 mock 上游，断言请求体里没有空签名块

**风险**

- **误删真签名**：Claude 的签名绝不含 UUID 形态，但保守起见先跑一个「不被误删」的正向用例（真实 Claude 响应样本）
- **DeepSeek 要 thinking 回来**：magpie 有个例外——DeepSeek 的 Anthropic 端点（`anthropicReplaysThinking`）**希望**拿回自己的 thinking，且接受空签名。Jolu 若将来接 DeepSeek 的 Anthropic 协议端点，要保留这个例外
- 不要顺手删 `redacted_thinking`——它没有文本但可能有有效签名

---

### P0-2 WorkBuddy 的模型上限修正（1 小时）

**问题**

magpie 实测：WorkBuddy 的 `deepseek-v4.1-flash` **实际回复上限 128000**，但 models.dev 记录的是 384000。magpie 靠插件（0.2.26）自己声明修正。

**Jolu 的暴露点**

`gateway/models_dev.rs` 拉的就是 models.dev，没有任何来源级覆盖。`models_dev.rs:83` 的 `exceeds_200k`（200k 判定）会被误导——影响可能是上下文窗口提示、也可能是压缩阈值。

**方案**

在 `models_dev` 之外加一层**来源级覆盖表**：

```rust
// gateway/model_overrides.rs
// 来源自己声明的上限优先于 models.dev。
// 只登记「有实测依据」的，不猜。
pub(crate) struct ModelOverride { pub context_window: Option<i64>, pub max_output: Option<i64> }

pub(crate) fn lookup(platform: &str, model: &str) -> Option<ModelOverride>
// 首批只登记：("workbuddy", "deepseek-v4.1-flash") → max_output: 128000
```

**合并顺序**：来源覆盖 > models.dev > 连接自身声明的值。

**验收**：单测覆盖顺序；实机在供应商页看 WorkBuddy 该模型的上下文标注。

**注意**：`model_overrides` 是**技术债类**设计——以后每个来源的偏差都要登记。所以要写清楚「只登记有实测依据的，不要按猜测填」。magpie 用插件声明是因为它的架构支持；Jolu 内置则必须靠这张表。

---

### P0-3 更新下载重试（2 小时）

**magpie**（`a46190e`）：下载对 **502 / 503 / 504 / 500 / 429** 重试。

**Jolu**：`updater.rs` 无重试，一次 5xx 就失败。

**方案**：抽一个 `net::retry_get(url, attempts)`：
- 仅对上述 5 个状态码重试，**不重试 4xx 的其余（尤其 401/404）**
- 指数退避 + 抖动（1s / 2s / 4s，±30%）
- 尊重 `Retry-After`
- 总时长上限（如 60s）后放弃

**验收**：单测 5 个状态码重试、401 不重试、超时放弃；实机断网重连场景。

---

### P0-4 卸载时还原 agent 接线（半天）

**magpie**（`92e8edd`）：卸载时「把每个 agent 放回去」——按接线记录还原用户原始配置。

**Jolu 现状**：NSIS 卸载钩子只删 `Jolu` 自启项。卸载后用户的 Claude Code `settings.json` 还指着 `127.0.0.1:12731`——**网关已不存在，CLI 全部请求失败**。

**方案**

卸载钩子里调用一次 `jolu_cli` 的还原命令（用 §3.11 刚抽的 service 层）：

```rust
// service::gateway::wiring::restore_all(conn) -> 按 agent_wiring 表还原每个 agent
```

但**卸载时的时机问题要处理**：
- NSIS 卸载执行时数据库还在，可以读
- 但 Jolu 进程可能已在退出中；CLI 是独立进程，只读数据库 + 写配置文件，可行
- **必须幂等**：重复执行不破坏已还原的配置

**验收**：
1. 单测：按快照还原后配置文件与接线前一致
2. 单测：已还原的情况下再跑一次不报错、不改动
3. **实机**：装包 → 接线 Claude Code → 卸载 → 检查 `settings.json` 里没有 `127.0.0.1:12731`

**风险**：卸载时用户可能手动改过配置。magpie 的做法是「只还原自己写的那部分键」，Jolu 的 `wiring_store` 快照已记录原始值，**只还原快照里的键，不整体覆盖文件**。

---

## 第二部分 · 能力汇聚（约 10–13 天）

### 依赖关系（决定了顺序）

```
netguard（安全前置）
    ↓
搜索后端抽象 + 2 个实现
    ↓
替身工具拦截（需要内部多轮转发）
    ↓
library（把 MCP 装到各 agent）
    ↓
图像 MCP / 远程 MCP OAuth
```

### C-0 `netguard`：地址安全校验（半天）★ 必须先做

**为什么是前置**：搜索要拉外部 URL、`web_fetch` 拉用户给的 URL。不做校验等于**给用户开了 SSRF**——能访问 `192.168.x.x`、`169.254.169.254`（云元数据）。

**magpie**（97 行）：判地址，不做决定（调用方决定怎么办）。

**Jolu 翻译**：`backend/src/gateway/net_guard.rs`

```rust
/// magpie 自己没选过的下载地址，能不能连。
pub(crate) fn check_url_ok(url: &url::Url) -> Result<(), Refusal>
// 拒绝：回环、私网（10/8, 172.16/12, 192.168/16, fd00::/8）、链路本地（169.254/16，
//       含云元数据 169.254.169.254）、组播、保留段
// 允许：公网地址；DNS 解析后的每个地址都要过一遍（防 DNS rebinding）
```

**magpie 刚修的一个坑要一起翻**（`c6bdc17`）：Clash Verge 的 TUN fake-ip 模式把域名解析到 `198.18.0.0/15`，这是**假 IP 但合法**，不能一律拒绝；同时 fake-ip 的 IPv6 段（`fdfe:dcba:9876::/64`）要给同样待遇。

**验收**：单测覆盖上面每个拒绝段 + 198.18/15 放行 + DNS 多地址全过才放行。

---

### C-1 搜索后端抽象 + 首批实现（1.5 天）

**magpie**（`internal/provider/search_api.go` ~600 行）：9 个厂商，`SearchVendor{ID, Name, Base, KeysURL, Paths}`。

**Jolu 翻译**：`backend/src/gateway/search/mod.rs`

```rust
pub(crate) struct SearchVendor {
    pub id: &'static str,          // "tavily"
    pub name: &'static str,        // "Tavily"
    pub base: &'static str,
    pub keys_url: Option<&'static str>,  // 引导用户去哪拿 Key
    pub paths: &'static [&'static str],
    pub key_optional: bool,
}

pub(crate) trait SearchBackend: Send + Sync {
    async fn search(&self, query: &str, count: usize) -> Result<Vec<SearchHit>, String>;
}

pub(crate) struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}
```

**首批只做 3 个**（覆盖国内外 + 零成本）：

| 后端 | 为什么选它 |
|---|---|
| Tavily | 最常用，文档好 |
| SearXNG | `key_optional`，可自建，零成本兜底 |
| 豆包（Volcengine） | 国内可用 |

**多 Key 轮转**（magpie `KeyList` + `SearchPickTurns`）：先做最简单的一版——**依次尝试，被拒的移到末尾并记冷却时间**（magpie 的语义是「被拒的排最后，可能只是限流」）。

**验收**：单测用 mock HTTP server 验证请求构造与解析；实机配一个 Tavily Key 搜一次。

---

### C-2 替身工具拦截 + 内部多轮（2–3 天）★ 技术难点

**这是整块最硬的部分。** 需要把 `wire::handle` 从「一次请求对一次转发」改成「可循环」。

**magpie 的做法**：网关静默吃掉 `tool_use`、自己执行、把结果作为 `tool_result` 续传。agent 只看到结果。

**Jolu 改造点**：`wire.rs::handle`

```rust
// 现在的形状（简化）
let response = forward(candidates, body).await?;
return response;

// 目标形状
let mut messages = ...;
for round in 0..MAX_TOOL_ROUNDS {           // 上限防死循环
    let response = forward(candidates, &body).await?;
    match extract_managed_tool_calls(&response) {  // 只认网关接管的工具名
        None => return response,
        Some(calls) => {
            append_tool_results(&mut body, execute_tools(calls).await?);
            continue;
        }
    }
}
```

**三个必须解决的细节**：

1. **流式响应**：SSE 里 `tool_use` 的 `input` 是**流式拼接**的（`input_json_delta`）。必须缓冲到 `content_block_stop` 才能执行。→ 这一轮不能边发边透传，要等这一轮结束。
2. **首字延迟**：多轮意味着用户等待变长。需要**在 SSE 里插入进度事件**（magpie 有 `gateway: the tool-loop notice is written in the conversation's language`，`3245b73`）。
3. **工具名白名单**：只接管我们真能执行的（`web_search` / `web_fetch`），其余工具照常透传——否则会把 agent 自己的工具吃掉。

**验收**

1. 单测：mock 上游返回 `tool_use: web_search` → 网关执行（mock 搜索）→ 第二轮请求体里有 `tool_result` → 最终返回文本
2. 单测：非白名单工具（如 `Read`）正常透传给客户端
3. 单测：超过 `MAX_TOOL_ROUNDS` 时如实报错，不无限循环
4. 集成：Claude Code 实机问「搜一下 Rust 1.90 发布说明」

---

### C-3 `library`：只翻 Jolu 的 8 个 agent（4–5 天）

**magpie 59k 行里该翻的只有 2–3k**（`agent_mcp.go` + `backup.go` + `applied_*.go`），其余是 Jolu 没有的 agent 适配。

**Jolu 要写的结构**：

```rust
// gateway/library/mod.rs
pub(crate) enum LibraryItem { McpServer(McpServerSpec), Subagent(...), Instruction(...) }

pub(crate) fn install(conn: &Connection, agent: &str, item: &LibraryItem) -> Result<(), String>
pub(crate) fn remove(conn: &Connection, agent: &str, name: &str) -> Result<(), String>
pub(crate) fn applied(conn: &Connection, agent: &str) -> Vec<String>   // 我写过什么
```

**关键设计（照抄 magpie 的教训）**：
1. **写前快照**：复用 Jolu 的 `wiring_store` 机制
2. **记录 applied**：新表 `library_applied(agent, item_name, installed_ms)`
3. **卸载只删自己写的**：`remove` 只处理 `applied` 表里有的项，用户手写的永不碰

**8 个 agent 的格式差异**（这是主要工作量）：

| agent | MCP 写在哪 | 格式 |
|---|---|---|
| Claude Code | `settings.json` → `mcpServers` | JSON |
| Codex | `config.toml` → `[mcp_servers.*]` | TOML |
| Gemini | `settings.json` → `mcpServers` | JSON |
| OpenCode | `opencode.json` → `mcp` | JSON |
| Qoder / Trae / ZCode / WorkBuddy | 各自配置 | 待逐个确认 |

**验收**：8 个 agent 各一个「装 → 读出验证 → 卸 → 复原」的往返测试（用临时目录）。

---

### C-4 后续（按需）

| 项 | 量 | 依赖 |
|---|---|---|
| 图像/视频 MCP | 1.5 天 | C-3 |
| 远程 MCP OAuth | 2.5 天 | 先有远程 MCP 支持 |
| 浏览器扩展 | 3 天 | 独立，可最后 |

---

## 第三部分 · 排期与批次

| 批次 | 内容 | 天数 | 完成标志 |
|---|---|---|---|
| **批 1** | P0-1 thinking 过滤 + P0-2 WorkBuddy 上限 + P0-3 下载重试 | 1 天 | 后端测试全绿 + thinking 往返集成用例通过 |
| **批 2** | P0-4 卸载还原 + `netguard` | 1 天 | 实机卸载后配置干净 |
| **批 3** | 搜索后端 + 3 个实现 | 1.5 天 | 实机 Tavily 搜通 |
| **批 4** | 替身工具 + 内部多轮 | 2–3 天 | Claude Code 实机能搜 |
| **批 5** | `library` 8 agent | 4–5 天 | 8 个往返测试全绿 |
| 合计 | | **9.5–11.5 天** | |

**每批结束跑一次全量测试并单独提交**——这是 §3.11 抽层时验证过的节奏，别攒。

---

## 第四部分 · 翻译方法论（这次要固定下来）

前几轮反复踩的坑，写成规则：

### 规则 1：先读代码，再估工期

反例：我把 `budget`（651 行 Go）说成「需要预算记账机制做地基」，实际就是「占位 + 释放」。并行会话用 387 行 Rust 就做完了。

**做法**：估工期前必须 `grep` 出真实行数与核心函数，不许凭感觉。

### 规则 2：按行为搜，不按符号搜

反例：搜 `DEFAULT_CONCURRENCY` 搜不到就判「并发队列不存在」，实际它以 `MaxConcurrency` 存在。

**做法**：判断「magpie 没有 X」时，用 commit message 关键词 + 测试文件名 + 行为描述三重搜索。

### 规则 3：插件仓库必须一起搜

反例：判 C5「边缘防火墙保持不存在」，实际在 `magpie-community/plugins/packages/zcode/index.mjs`。

**做法**：任何「ZCode / Qoder / WorkBuddy / Trae 的订阅行为」都要搜两个仓库。

### 规则 4：翻译不是重写

magpie 的 15 行 `withoutForeignThinking` 直接对应 Rust 的 30 行（含类型）。**不要重新设计成一套「thinking 治理框架」**——按原逻辑翻，注释保留「为什么」。

### 规则 5：注释要带来源

抄 magpie 的注释风格：写清「谁踩的坑、什么症状、issue 号」。Jolu 自己的注释也要标「参照 magpie `a6a9971`」，便于以后同步。

---

## 第五部分 · 风险表

| 风险 | 等级 | 规避 |
|---|---|---|
| thinking 过滤误删真签名 | **高** | 用真实 Claude 响应样本做「不误删」正向用例；先只删空签名 + UUID 两种 |
| 内部多轮导致首字延迟 | 中 | 轮次上限（3）；每轮插入进度事件；日志记录轮数 |
| `library` 写坏用户配置 | **高** | 写前快照 + 只还原自己写的键 + 往返测试 |
| `netguard` 误拦合法地址 | 中 | 198.18/15 fake-ip 放行（已有先例）；提供「诊断模式」打印判定过程 |
| 搜索 Key 泄漏到日志 | **高** | Key 只在后端内存与数据库；任何日志/导出/诊断都不带 |
| 卸载还原时用户已改配置 | 中 | 只还原快照里的键，不整体覆盖 |
| magpie 继续快速迭代 | 中 | 每批开工前 `git fetch`；借修复优先于借功能 |
