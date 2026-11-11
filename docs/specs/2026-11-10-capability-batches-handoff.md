# 能力汇聚与转发正确性 · 交接文档（2026-11-10）

**交给：接手成员**
**状态：** 批 1–5 后端已实现并已提交（`d8d291f`）；界面 UI 未做（见 §6）。2026-11-11：§7.3 的 Key 掩码改为序列化默认掩码（所有命令与事件都覆盖），§2 P1 已做；剩余项并入 `docs/plans/2026-11-09-complete-remaining-work.md` 阶段 1B
**基线：** 后端 1204 测试全绿（44 ignored）、前端 444、`tsc` 通过、clippy 无 error
**参照物：** [yetone/magpie](https://github.com/yetone/magpie) @ `8016124`
**规划来源：** [`2026-11-10-capability-and-correctness-plan.md`](./2026-11-10-capability-and-correctness-plan.md)、[`2026-11-09-magpie-parity-full-audit-authoritative.md`](./2026-11-09-magpie-parity-full-audit-authoritative.md)

---

## 0. 先读这一节：这批做了什么

分两条线，共五批：

| 批 | 主线 | 一句话 |
|---|---|---|
| 1 | 转发正确性 | 跨模型分组切回 Claude 不再连续 400；来源上限不被 models.dev 误导；下载不再因 CDN 抖动失败 |
| 2 | 安全与卸载 | 外链地址先过 netguard（防 SSRF）；卸载 Jolu 时把 agent 配置还原，不留死端口 |
| 3 | 搜索能力 | 三个搜索后端 + 多 Key 轮转，先给 CLI 用 |
| 4 | 档位深耕 | Claude Code 四档各选模型 + Auto 判别（省钱主力） |
| 5 | 分发层 | 把搜索能力以 MCP server 装进各 agent 的配置 |

**代码位置**（全部新增，未改动既有行为）：

```
backend/src/gateway/thinking_guard.rs    批1 · 无签名 thinking 块过滤
backend/src/gateway/model_overrides.rs   批1 · 来源级模型上限覆盖
backend/src/net.rs                      批1 · retry_blocking_get（新增）
backend/src/gateway/net_guard.rs         批2 · 地址安全校验
backend/src/service/gateway/wiring_restore.rs  批2 · 无 Tauri 上下文的接线还原
backend/src/gateway/search/              批3 · 搜索后端（mod.rs + backends.rs）
backend/src/library/                     批5 · MCP 安装（mod.rs + agents.rs + mcp_search.rs）
backend/src/gateway/wiring.rs            批4 · claude_wire 档位写入（改）
backend/src/gateway/wiring_store.rs      批4 · Applied.tiers（改）
```

---

## 1. 注释规范（**必须遵守**）

这是本项目本轮确立的规范，源自 magpie 的注释习惯（实测：magpie 注释占比 20.1%、6082 处带 issue 号；Jolu 原为 7.2%）。

### 1.1 三档模板

**档 1 · 模块/包头（`//!`）**

```rust
//! <这个模块解决什么问题>（一句话）
//!
//! 管什么：
//! - <场景 A>
//! - <场景 B>
//!
//! 不管什么：<明确的边界，以及谁来负责>
//!
//! 关键取舍：<设计 A>：<为什么不是 B>
//!
//! 参照 magpie `<文件>::<函数>`（commit `<hash>`）；差异：<不同点与原因>
```

**档 2 · 函数（`///`）**

```rust
/// <做什么>（一句话，动词开头）
///
/// 为什么：<触发这条规则的现场症状>
///
/// 参照 magpie `<文件>::<函数>`（commit `<hash>`）
/// 差异：<与 magpie 不同之处，以及为什么>
```

平凡函数（getter、`is_empty`）不需要。

**档 3 · 数据/常量（行内 `//`）**

```rust
const SPECIAL_V4: &[(Ipv4Addr, u8, &str)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), 8, "本网络"),
    (Ipv4Addr::new(100, 64, 0, 0), 10, "CGNAT、Tailscale"),
    (Ipv4Addr::new(198, 18, 0, 0), 15, "基准测试段；Clash 等 fake-ip 解析器用它"),
];
```

**规则：每个元素都标它是什么**，不是笼统写「保留段」。

### 1.2 四个必须

1. **写「为什么」而不是「做什么」**——代码本身说了做什么
2. **非平凡函数标注现场症状**——「Grok 4.7 切到 Claude Opus 5.5 后连续 13 轮裸 400」
3. **参照外部实现写 commit hash + 函数名**，差异要说明
4. **已知缺口必须写明**——本期不做的、依赖未来的，留空等于埋雷

### 1.3 引用可靠度（别引用会消失的东西）

| 引用对象 | 可靠度 |
|---|---|
| commit hash（`a6a9971`） | 高 |
| 文件 + 函数名 | 中 |
| issue 号 | 中 |
| 用户 ID（作为现场描述的一部分） | 中 |
| PR 链接 / Discord 链接 | **低，别用** |

### 1.4 反面例子

```rust
// ❌ 只写来源不写理由
// 参照 magpie thinking_bound.go

// ❌ 当作免责声明
// 抄自 magpie，有问题不负责

// ❌ 解释代码本身
// 遍历 messages 数组
```

---

## 2. 剩余缺口（按优先级）

### P1 · 400 后修复重试（半天）★ 最该先做

**magpie 的做法**：`internal/gateway/thinking_bound.go::refuseBoundThinking`（第 178 行起）

```go
func (s *Server) refuseBoundThinking(p provider.Provider, errBody, body []byte) (fresh bool) {
    if !boundRefusal.Match(errBody) { return false }
    sent := withoutForeignThinking(p, s.withoutRefusedThinking(p, body))
    // 从 400 响应里解析出「哪个消息的哪个块」被拒（boundAt）
    // 从那个块起剔除 thinking，重发一次
}
```

**Jolu 现状**：`thinking_guard::strip_foreign_thinking` 是**预防式**——发之前就剔除空签名与 UUID 签名的块。若上游仍拒（签名格式出乎意料），如实报 400。

**缺口**：没有「拒了之后自动修一次」的兜底。

**要做什么**：
1. `wire.rs::forward_one` 拿到上游响应后，检查是否 400 且错误体匹配「thinking 块被拒」的模式
2. 匹配则：从错误体解析 `messages[N].content[M]` 位置 → 剔除该块 → **重发一次**（只重试一次，防死循环）
3. 重试仍失败 → 原样返回

**难点**：需要确认 Claude 400 响应体的实际文案。magpie 的 `boundRefusal` 正则可以直接抄（在 `thinking_bound.go` 里 grep `boundRefusal`）。

**验收**：mock 上游返回 `{"error":{"message":"...content.1..."}}` → 断言第二次请求体里少了那个块。

---

### P2 · 搜索后端补 6 个（1.5 天）

**magpie 的 9 个**（`internal/provider/search_api.go::SearchVendors`，第 91–106 行）：

| ID | 名称 | Base | 路径 |
|---|---|---|---|
| tavily | Tavily | `https://api.tavily.com` | `/search` | ✅ Jolu 已有 |
| brave | Brave Search | `https://api.search.brave.com` | `/res/v1/web/search` | ❌ |
| exa | Exa | `https://api.exa.ai` | `/search` | ❌ |
| firecrawl | Firecrawl | `https://api.firecrawl.dev` | `/v2/search` | ❌ |
| you | You.com | `https://ydc-index.io` | `/v1/search`、`/v1/agents/search` | ❌ |
| deepseek | DeepSeek | `https://api.deepseek.com` | `/v1/tools/search` | ❌ |
| volcengine | 豆包搜索 | `https://open.feedcoopapi.com` | `/search_api/web_search` | ✅ Jolu 已有 |
| wsa | 元宝搜索（腾讯云） | `https://api.wsa.cloud.tencent.com` | `/SearchPro` | ❌ |
| searxng | SearXNG | 自建 | `/search` | ✅ Jolu 已有 |

**要做什么**：在 `backend/src/gateway/search/backends.rs` 加 6 个模块，形状照 `tavily` 那个：
- 请求构造（鉴权头：Brave 用 `X-Subscription-Token`，Exa 用 `x-api-key`，其余 `Bearer`）
- 响应解析（喂给共用的 `parse_results`；形状不同的自己写解析）
- `VENDORS` 常量加一行
- 每个一个 mock 测试

**注意**：`you` 与 `searxng` 的 `KeyOptional: true`（可免 Key），Jolu 的 `run_search` 已处理 SearXNG 免 Key，`you` 要同样处理。

---

### P3 · 档位 effort 可用值校验（半天）

**magpie**：`internal/agent/claude.go::tierEfforts`（611 行起）——按模型查已知的 efforts 表，模型不知道就退回 Claude Code 的默认集（`low/medium/high/xhigh/max`）。

**Jolu 现状**：`TierModels` 存的是 `model:effort` 原始字符串，**后端不校验 effort 值**，前端填什么写什么。

**风险**：用户填了模型不支持的 effort，Claude Code 可能会静默降级或报错。

**要做什么**：
1. `models_dev::ModelMeta` 加 `efforts: Vec<String>` 字段（models.dev 有这条数据时）
2. `library`/`wiring` 写入前校验：effort 不在该模型的列表里 → 报错或退回默认
3. 未知模型不拦（不猜）

**优先级低于 P1/P2**：这是「配错了才出问题」，不是「正常路径会出问题」。

---

### P4 · 跨 agent 广度（可选，视产品决定）

**magpie 覆盖 25 种 MCP 格式**，Jolu 只做 4 个（`claude-code` / `codex` / `gemini` / `opencode`）。

**为什么够用**：Jolu 的 `wiring::AGENTS` 只有 5 个，其余 20 种（Antigravity / Cursor / Zed / Goose / Crush / Kiro / Devin / Cline / Copilot / Pi / Hermes / Alma / Kimi / Grok / Dsh / LiveAgent / CommandCode / Desktop / Omp / PiNative）Jolu **连接线都不支持**。

**如果要扩**：`backend/src/library/agents.rs` 的模块头写了改造方案（格式描述表 + 5 个 `FieldStyle` 变体），约半天改造，之后每加一个 agent 约 30 分钟。

**zcode 的阻塞**：它的 `provider.json` 用嵌套的 `mcp.servers` 形状，**未实测确认**，按「不猜」原则暂不支持。要接需要先抓一份真实配置样本。

---

### P5 · 远程 MCP（http/sse）（2.5 天，依赖 mcpauth）

Jolu 的 library 只支持本地 stdio server。magpie 支持远程（`Transport: http/sse`）。

**依赖**：远程 MCP 通常要 OAuth（`internal/mcpauth`），这是另一个模块（1863 行）。不先做 mcpauth 的话，远程 MCP 只能支持免登录的。

---

## 3. 已实现但要注意的边界

接手时先知道这些**有意为之**的设计，别当成 bug：

| 位置 | 行为 | 为什么 |
|---|---|---|
| `thinking_guard::serves_claude` | 只对「真 Claude」上游过滤；GLM/DeepSeek 等类 Claude 端点原样发 | 它们要回自己的 thinking，剔了会丢上下文 |
| `thinking_guard` | `redacted_thinking` 不删 | 它没明文但可能有有效签名 |
| `model_overrides` | 只登记有实测依据的；缺数据不猜 | 显示错的上限比不显示更误导 |
| `net_guard::judge` | fake-ip 返回 `FakeIp` 而非 `Public`/`Private` | 让调用方决定「放行连接」还是「要求用户改解析」 |
| `search::run_search` | 被拒的 Key 冷却 10 分钟，期间排最后 | magpie `SearchPickTurns` 同语义 |
| `library::install_at` | 同名第三方条目**拒绝不覆盖** | 静默覆盖是 magpie 特意避免的事故 |
| `library` | 先落库再写文件，写失败回滚记录 | 杜绝「配置已写、记录没有」——那会让同名重装被当第三方拒绝 |
| `library::agent_key` | zcode 返回 None（不支持） | 格式未实测，不猜 |
| `--mcp-search` | 独立进程，不读网关数据库 | MCP server 与网关监听互不依赖 |

---

## 4. 关键实现说明

### 4.1 档位写入（批 4）

`claude_wire` 的档位循环（`wiring.rs`）：

```rust
for tier in CLAUDE_TIERS {
    let (tier_value, tier_name, tier_desc) = match tiers.and_then(|t| t.tier(tier)) {
        Some(own) => { /* 自选：值原样写，关于按档位模型查 */ }
        None => { /* 跟随主模型 */ }
    };
    env.insert(claude_tier_env(tier), tier_value);
    env.insert(format!("{}_NAME", ...), tier_name);
    ...
}
```

**effort 编码**：值是 `model:effort`（magpie `tierWith` 同格式），可带 `[1m]` 后缀。后端不解析，原样写。

**Auto 判别**：`tiers.auto_mode` → `CLAUDE_CODE_AUTO_MODE_MODEL`；None = 删键（Claude Code 自己挑）。「跟随子代理」由**前端**把子代理值填进来——后端不知道这个语义。

### 4.2 幂等与 tier_own

`escape::wire_agents` 的跳过条件（三个都要满足）：

```rust
let tiers_unchanged = record.applied().as_ref().and_then(|a| a.tiers.as_ref()) == tiers;
if record.state == "wired" && tiers_unchanged && status_with(...).wired {
    continue;  // 跳过
}
```

**为什么必须比 tiers**：只改档位（主模型不动）时 `status_with` 仍判 wired，不比 tiers 就会把用户的修改静默吞掉。

**Profiles 一键切换**：`lib.rs::gateway_profiles_apply` 会读出该 agent 现有的 `applied.tiers` 传回去，实现「切主模型时保留档位自选」（magpie `tier_own`）。

### 4.3 library 的两层结构（**测试必须用 `_at`**）

```rust
install(conn, agent, item)      // 生产：内部解析真实路径 → 调 install_at
install_at(path, conn, agent, item)  // 测试：路径显式传入
```

**血泪教训**：最初只有 `install`，测试直接调它，结果把 `jolu-search → C:/theirs/tool.exe` 写进了**真实的** `~/.claude/settings.json`。已清理并分层。

**规矩：任何会写用户文件的测试，必须走 `_at` 版本 + 临时目录。**

### 4.4 卸载还原（批 2）

`main.rs` 的 `--restore-agents` 分支 → `lib.rs::restore_agents_headless`：
1. `service::gateway::wiring_restore::restore_all`（还原接线）
2. `library::restore_all_library`（移除 MCP 项）
3. 返回 0 / 1（NSIS 不因失败中止卸载，只留日志）

NSIS 调用点在 `backend/windows/installer.nsi` 的 `Section Uninstall`，**在 `CheckIfAppIsRunning` 之后、`Delete 主程序` 之前**（顺序很重要：应用已退出才不会重写快照，但程序还在才能执行）。

---

## 5. 验证与验收现状

### 已通过

| 项 | 结果 |
|---|---|
| 后端全量测试 | 1204 通过 / 44 ignored |
| 前端测试 | 444 通过 |
| `tsc -b --noEmit` | 通过 |
| clippy | 无 error |
| thinking 过滤 | 6 个单测（UUID/空签名/长 base64 保留/条内剔除/整条移除/无签名短路） |
| netguard | 6 个单测（14 个特殊段、NAT64/6to4、fake-ip、fake_paired、loon） |
| 搜索 | 10 个单测 + **真实 HTTP 到 Tavily 得 401**（链路通） |
| 档位 | 3 个专项测试（自选写入/全跟随清 Auto/旧快照兼容） |
| MCP server | 3 个单测 + **stdio 握手实测** |
| library | 10 个单测 + **真实装/卸/重装循环** |

### 需要接手成员补的实机验收

| 项 | 怎么验 |
|---|---|
| 搜索出真结果 | 在 `%APPDATA%\dev.ningz.jolu\settings.json` 配真 Key（`search_apis`），跑 `jolu_cli search "rust"` |
| Claude Code 用上搜索 | 装好（已装 `claude-code`）→ 重启 Claude Code → `/mcp` 看到 `jolu-search` → 让 agent 搜一次 |
| thinking 过滤真实生效 | 需跨模型分组（GLM/Grok ↔ Claude 模型切换） |
| 卸载完整流程 | 装包 → 接线 → 卸载 → 检查 `~/.claude/settings.json` 无 `127.0.0.1:12731` |

---

## 6. 界面 UI（未做，交给设计）

**后端已就绪，UI 是纯接入工作。** 后端命令与数据结构：

### 6.1 档位（批 4）

```typescript
// gatewayApi.ts 已加
interface TierModels {
  opus?: string | null      // "workbuddy-16f911/glm-5.3-flash:low" 或 null=跟随
  sonnet?: string | null
  haiku?: string | null
  fable?: string | null
  autoMode?: string | null
}

gatewayApi.setWiringTiers(agent, tiers)   // 只改档位
gatewayApi.setWiring(agent, true, model, subagent, tiers)  // 一起改
```

**UI 需要表达的信息**（语义来自 `2026-11-10-capability-and-correctness-plan.md` §第二部分与 v2 设计稿）：

| 字段 | 展示要求 |
|---|---|
| 主模型 | 单下拉，显示 `来源/模型 · 上下文` |
| 四档 | 每档一个下拉，`null` 显示「跟随主模型 · <主模型名>」，自选显示实际值 + 「自选」标记 |
| 每档 effort | 分段控件（低/中/高/最高），拼成 `model:effort` 写入 |
| 子代理 | 单下拉，写入 `CLAUDE_CODE_SUBAGENT_MODEL` |
| Auto 判别 | 单下拉，`null` 显示「跟随子代理 · <子代理值>」；**选「跟随子代理」时前端要填子代理的值** |
| 多 agent | 一套框架渲染所有 agent——Claude Code 有四档，Codex 只有主模型+子代理，字段由后端能力决定 |

**关键交互**：「自选档位在换主模型时保持不变」——UI 不需要额外处理，后端已按 `tier_own` 语义实现（`Applied.tiers` 绝对值）。但 UI 要让用户看懂这个语义（行内说明）。

### 6.2 Library（批 5）

```typescript
// 后端命令（未加 TS 桥接，需要补）
library_install(agent)  -> { agent, name, created }
library_remove(agent, name) -> boolean
library_list() -> [{ agent, installed: string[], config_present: boolean }]
```

**UI 需要表达**：
- 每个 agent 一行：`已装 [jolu-search] / 未装`
- 安装/卸载按钮（卸载只删 Jolu 装的）
- `config_present: false` 时提示「配置文件还没初始化，安装会新建」
- 错误要原样显示（`ForeignEntry` = 同名条目被第三方占用，`UnsupportedAgent` = zcode 格式待实测）

**事件**：后端发 `library-changed`，前端监听后刷新。

### 6.3 搜索配置（批 3）

```typescript
settingsApi.setOtlpEndpoint 的同层需要新增：
setSearchApis(apis: SearchApiConfig[]) -> AppSettings
```

**数据结构**（`settings.rs::search_apis`）：

```rust
pub struct SearchApiConfig {
    pub vendor: String,      // "tavily" / "searxng" / "doubao"
    pub keys: Vec<String>,   // 多 Key，依次尝试
    pub url: Option<String>, // 自建地址（SearXNG 必填）
    pub off: bool,           // 暂停（Key 保留但不参与）
}
```

**UI 注意**：
- Key 是敏感信息——**列表只显示掩码**（后端目前直接返回原文，需要加掩码处理，见 §7 待办）
- `vendor` 是固定枚举，UI 用下拉不要自由文本

### 6.4 Jolu.pen 画板

设计稿还没画。v2 HTML 设计稿（`gateway-v4-design-v2-multiagent.html`）是语义参考，画板要按 `Jolu.pen` 的变量与画板规范重做（浅深色成对）。**Pencil MCP 可用时再同步。**

---

## 7. 提交前必做（**未提交的原因**）

### 7.1 工作区待提交清单

本轮改动（**不含** `docs/ui/Jolu.pen` 与 `session_threads.rs`）：

```
M  backend/src/bin/jolu_cli.rs
M  backend/src/db.rs                          （library_applied 表）
M  backend/src/gateway/catalog.rs             （CatalogModel.platform）
M  backend/src/gateway/escape.rs              （tiers 参数）
M  backend/src/gateway/mod.rs                 （3 个新模块注册）
M  backend/src/gateway/models_dev.rs          （retry 拉取）
M  backend/src/gateway/wire.rs                （thinking 过滤接入）
M  backend/src/gateway/wiring.rs              （claude_wire 档位 + config_path_for）
M  backend/src/gateway/wiring_store.rs        （Applied.tiers）
M  backend/src/lib.rs                         （命令 + restore + mcp-search 入口）
M  backend/src/main.rs                        （--mcp-search 分支）
M  backend/src/net.rs                         （retry_blocking_get）
M  backend/src/service/gateway/catalog.rs     （model_overrides 合并）
M  backend/src/service/mod.rs                 （wiring_restore）
M  backend/src/service/paths.rs               （config_directory）
M  backend/src/settings.rs                    （search_apis）
M  backend/src/updater.rs                     （check 重试）
M  backend/windows/installer.nsi              （卸载还原钩子）
M  frontend/src/lib/gatewayApi.ts             （TierModels 透传）
A  backend/src/gateway/model_overrides.rs
A  backend/src/gateway/net_guard.rs
A  backend/src/gateway/search/{mod,backends}.rs
A  backend/src/gateway/thinking_guard.rs
A  backend/src/library/{mod,agents,mcp_search}.rs
A  backend/src/service/gateway/wiring_restore.rs
```

### 7.2 明确不提交

| 文件 | 原因 |
|---|---|
| `docs/ui/Jolu.pen` | Pencil 打开时自动重存（+40472 行、34.8MB）。**不是本轮改动**，提交会污染历史。需要的话单独提交并说明 |
| `backend/src/session_threads.rs` | 并行会话的流式读取优化，不属于本轮 |

### 7.3 提交前要补的两件小事

1. ~~`search_apis` 的 Key 掩码~~ **已修**：`Settings::masked()` 在 `get_settings` 出口做掩码（长 Key 保留前 4 后 4，短 Key 全遮），写路径仍用后端原文。实测见 `settings::tests::masked_keeps_search_keys_out_of_the_frontend_copy`。**注意**：其他返回 `AppSettings` 的命令（各 set_* 系列）仍直出原文——UI 若从这些命令读 `search_apis`，需同样走 `.masked()`。
2. **`docs/ui/Jolu.pen` 的处置**：确认是 Pencil 自动重存还是设计改动。若只是重存，`git checkout -- docs/ui/Jolu.pen` 丢弃。

### 7.4 提交信息建议

```
feat: 补齐能力汇聚与转发正确性五批后端 [skip ci]

- 批1 转发正确性：无签名 thinking 块在发往真 Claude 前过滤（跨模型分组切回时不再连续 400）；
  来源级模型上限覆盖（WorkBuddy deepseek-v4.1-flash 实际 128000，models.dev 记的是 384000）；
  下载对 5xx/429 退避重试
- 批2 安全与卸载：外链地址过 netguard（14 个特殊段 + fake-ip 配对 + NAT64/6to4 解包）；
  卸载时按快照还原全部 agent 接线并移除 library 项，不留死端口
- 批3 搜索：Tavily / SearXNG / 豆包三后端 + 多 Key 轮转（被拒排尾并冷却），请求前过 netguard
- 批4 档位深耕：Claude Code 四档各选模型与 effort + Auto 判别档，自选档位换主模型时保持不变
- 批5 分发层：search MCP server（jolu.exe --mcp-search）+ library 安装到 claude-code / codex /
  gemini / opencode，只删自己写的、拒绝覆盖第三方同名条目
- 后端 1204 测试全绿，实机验证装/卸/重装与 stdio 握手
```

---

## 8. 接手第一步建议

1. 跑一遍验证：`cargo test --manifest-path backend/Cargo.toml`（应 1203 全绿）
2. 读 `backend/src/library/agents.rs` 的模块头（解释了为什么不学 magpie 做 25 种格式）
3. 按 §7.3 补 Key 掩码 + 处置 `Jolu.pen`，然后按 §7.4 提交
4. 做 §2 的 P1（400 修复重试）——半天，用户可见收益最大
5. UI 按 §6 接入，设计稿确定后同步 `Jolu.pen`
