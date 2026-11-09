# magpie 对齐剩余差距 · 第三批执行计划（2026-11-09）

**交给：鼠鼠团队**
**状态：计划稿。本轮只写文档、不改代码。**
**前置：** `docs/plans/2026-11-08-magpie-parity-roadmap.md`（总路线图）、本批已完成部分见 §0
**参照物：** [yetone/magpie](https://github.com/yetone/magpie) @ `627fd1a`；[magpie-community/plugins](https://github.com/magpie-community/plugins) `packages/zcode`（本地 `/tmp/magpie-plugins`）

---

## 0. 已完成基线（本批之前 + 本批）

### 本批（2026-11-09）已交付

| 提交 | 内容 |
|---|---|
| `f0ec4c0` | 档位友好名 `_NAME`/`_DESCRIPTION` 写入 + models.dev 元数据（name/description/limit）并入模型目录 + 接线面板「· 1M」标注 |
| `ea8ffbc` | smart / pace 接入额度快照注册表；团队套餐坐席链（setting.json 选定 → 项目 `zcode-team-api-key`） |
| `83aab76` | 网关中间件链 + `model-map` / `param-override` 内置（选路前执行、失败隔离） |
| `5097b3e` | Start Plan 忙碌重试（1s/2s + Retry-After）+ `zcode_free_first` 通道顺序开关 |
| `d28e9c1` | 隐私脱敏设置分区 + 本机导入弹窗（frontend-design 工艺） |
| `a723a5d` | 局域网共享 + 命名客户端 Key + 请求限额（后端） |
| `22ed240` | 跟随系统代理显式排除回环 |
| `83d8972` | Start Plan 兜底扩展到网络级失败（502/504） |
| `cbcd628` | 网关上游客户端尊重联网方式（`net::async_builder`）+ 默认来源失效报错给可操作指引 |
| `9ef1cca` | models.dev 拉取与 MCP 桥客户端同样尊重联网方式 |

**验证基线：** 后端 1047 测试全绿（42 ignored）、前端 417 全绿、`tsc` 通过、端到端流式 200（`workbuddy-16f911/glm-5.3` → "OK"）。

### 关键环境事实（影响后续验收）

- **zcode 账号当前无额度**：Start Plan 已用尽，`api.z.ai` 返回 429/1113、Start Plan 通道也未验证通过。**zcode 链路的端到端验收要等额度恢复**，其余项不依赖它。
- 本机系统代理（`127.0.0.1:7897`）曾在代理客户端退出后残留，已由 `22ed240`/`cbcd628`/`9ef1cca` 三处修好；后续验收若再遇 `502 上游不可达`，先查联网方式与代理客户端状态。

---

## 1. ZCode 登录链路验证结论（本轮专项）

### 1.1 你给的地址是什么

```
https://chat.z.ai/api/oauth/authorize
  ?client_id=client_P8X5CMWmlaROgyO-KSqtg
  &redirect_uri=https%3A%2F%2Fzcode.z.ai%2Fapp%2Foauth%2Flogin%3Fredirect%3Dzcode%253A%252F%252Foauth%252Fcallback%26app_version%3D3.17.2
  &state=41693db532e162535e83082604a73446
  &response_type=code
```

实测（2026-11-09，`--noproxy` 直连）：

| 现象 | 结果 |
|---|---|
| `GET chat.z.ai/api/oauth/authorize` | **307 Temporary Redirect**，`location: /auth?response_type=code&client_id=…&redirect_uri=…&state=…` |
| 跟随到 `/auth` | 200，页面标题「Z.ai - Advanced AI Chatbot & Agent powered by GLM-5.3-Flash」，即**网页登录页** |
| `response_type=code` | 授权码模式（authorization code grant），非设备码 |
| `state` | 32 位 hex，CSRF 防护，必须原样回传 |
| 带 `code_challenge` 参数 | 仍 307（服务端不拒绝，**但无法确认是否强制 PKCE**） |
| 回调链 | `chat.z.ai/api/oauth/authorize` → 登录 → `zcode.z.ai/app/oauth/login?redirect=zcode://oauth/callback` → 302 到 `zcode://` 自定义协议 → 桌面 ZCode App 接收 |

### 1.2 与 Jolu 现有实现的关系

**关键结论：这套东西 Jolu 代码里已经有了，只是名字不同。**

```
POST https://zcode.z.ai/api/v1/oauth/cli/init
  {"provider": "zai"}          ← 注意是 "zai"
→ {"code":0, "data":{
     "authorize_url":"https://chat.z.ai/api/oauth/authorize?client_id=client_P8X5CMWmlaROgyO-KSqtg
                      &redirect_uri=https://zcode.z.ai/api/v1/oauth/cli/callback/zai&state=…",
     "flow_id":"…", "poll_token":"…", "poll_interval_sec":2, "expires_at":…}}
```

- **`client_id` 完全一致**：`client_P8X5CMWmlaROgyO-KSqtg`
- **授权端点完全一致**：`chat.z.ai/api/oauth/authorize`
- **唯一区别是 `redirect_uri`**：
  - **命令行版**（`cli/init` 返回）：`zcode.z.ai/api/v1/oauth/cli/callback/zai` —— 服务端接住 code，换好 token 存起来，客户端**轮询 `cli/poll/<flow_id>`** 取
  - **桌面版**（你给的）：`zcode.z.ai/app/oauth/login?redirect=zcode://oauth/callback` —— 服务端不存，302 到 `zcode://` 自定义协议，**由桌面 App 自己接**

### 1.3 另一条路径：BigModel 家族仍在用另一套

```
POST cli/init {"provider":"bigmodel"}
→ authorize_url: https://bigmodel.cn/login?appId=zcode&redirect=https://zcode.z.ai/api/v1/oauth/cli/callback/bigmodel&state=…
```

BigModel 走的是自己的登录页（不是 `chat.z.ai`），**协议不同**。Jolu 现有的 `begin_family("bigmodel")` 就是这条，验证正常。

### 1.4 现状判定

| 项 | 状态 |
|---|---|
| `cli/init` 端点 | ✅ 可用（实测 `code:0`，返回 flow_id/poll_token/authorize_url） |
| `provider: "zai"` | ✅ 返回 `chat.z.ai` 授权地址（与你给的 client_id 一致） |
| `provider: "bigmodel"` | ✅ 返回 `bigmodel.cn` 授权地址 |
| Jolu 现有 `begin_family()` | ✅ 两条家族都走通了发起，代码无需改 |
| **前端入口** | ⚠️ 供应商页把 ZCode 拆成「国内版 / 国际版」两条，但**登录按钮走的还是同一套 `begin_family`**，UI 未区分「命令行版 / 桌面版」 |
| **你贴的桌面版 redirect** | ❌ Jolu 不走这条（也不需要——因为 `cli/init` 的轮询链路已经能拿到 token） |

**结论：不需要改登录实现。** 你贴的地址是「桌面 App 版」的回调形态，Jolu 用「命令行版」的轮询链路属于同一 OAuth 应用的另一种客户端形态，两者拿到的 token 等价。**要补的是 UI 上的可辨识性**（让用户知道点的是哪条家族），列入 §3.3。

### 1.5 若后续要支持桌面版 redirect（低优先，明确不做）

技术上可行但**收益为零且风险高**：
- 需要注册 `zcode://` 自定义协议处理器（Windows 注册表写入、冲突检测、安全审查）
- 需要自己接 code → 换 token 的交换（端点未公开）
- 而 `cli/poll` 已经能做同样的事，且不碰系统协议注册

**决策：不做。** 理由写入 §6「明确不做的事」。

---

## 2. 差距总表（截至 2026-11-09，28 条）

### 2.1 核心体验类（6 条，最高优先）

| # | 差距 | magpie 实现 | Jolu 现状 | 影响 | 工作量 |
|---|---|---|---|---|---|
| C1 | **Start Plan 礼包按余额过滤** | `startPlan()` 读余额接口，余额没有的模型从目录剔除（`giftNames` 过滤） | 静态三模型清单 `GLM-5.3-Flash / GLM-5.2 / GLM-5-Turbo` | 礼包外模型配了也会被上游拒，用户看到原始错误 | 0.5 天 |
| C2 | **双通道按账号记忆** | `routes` map：一个账号第一次落到哪条通道，之后直接走那条 | 每请求按 `zcode_free_first` 固定顺序试 | zcode 无额度时每次先试 Coding Plan（失败）再落 Start Plan，多一次往返 | 0.5 天 |
| C3 | **团队席位状态校验** | `querySubscribeDetail` 判 EFFECTIVE / VALID / UNASSIGNED / EXPIRED，管理员没给席位时如实说 | 只查项目 Key 存在性，不查席位状态 | 席位被撤时报错不精确（用户以为是 Key 问题） | 0.5 天 |
| C4 | **每账号并发队列** | 按账号排队（`DEFAULT_CONCURRENCY`），按并发数间隔请求 | 无并发控制 | 并发高时更易撞上游限流 | 1 天 |
| C5 | **边缘防火墙保持** | zcode.z.ai 的 405（阿里云 WAF）触发 `edgeHeld`，保持期内直接应答不再打上游 | 405 原样透传给 agent | 偶发 405 直接落用户 | 0.5 天 |
| C6 | **UA / App 版本跟随** | 插件版本随 ZCode 版本升级（3.14.3 → 3.17.2 的教训写在 `prompt.mjs` 注释） | 固定 `ZCode/3.17.2` | ZCode 升级换文案后着装请求会被风控拦（405/3012） | 持续性成本 |

### 2.2 Claude Code 深耕（4 条）

| # | 差距 | magpie | Jolu | 工作量 |
|---|---|---|---|---|
| D1 | subagent 独立模型入口 | `CLAUDE_CODE_SUBAGENT_MODEL` 作为 Quiet 字段可单独配（默认跟随主模型） | 只快照还原，无设置入口 | 0.5 天 |
| D2 | 档位 effort 联动 | 换模型时 effort 跟随（每模型有 effort 等级表） | 无 effort 概念 | 1 天 |
| D3 | 云端开关全集 | `claudeClouds` 集合（含 Vertex 项目变量等配套处理） | 只 `CLAUDE_CODE_USE_BEDROCK` / `USE_VERTEX` 两个 | 0.5 天 |
| D4 | `_SUPPORTED_CAPABILITIES` | 会写入（模型能力声明） | 不写（不猜能力） | 0.5 天 |

### 2.3 中间件与路由（4 条）

| # | 差距 | magpie | Jolu | 工作量 |
|---|---|---|---|---|
| E1 | **异步钩子（Bun）** | `magpie.hooks` / `*.hooks.js`：`onRequest(body, ctx)` 可读外部上下文后返回增强 body；`onResponse` 观察完成回复；失败回滚 | 无（只有同步内置） | 3 天 |
| E2 | `system-prompt` / `word-guard` / `think-tags` 三个内置中间件 | 已实现（New API 同款） | 未实现（`middleware.rs` 留了扩展位） | 1.5 天 |
| E3 | 意图路由 | 小模型读每轮意图，测试走强模型、快问题走便宜模型 | 无 | 3 天 |
| E4 | Cap hold / 暂停规则 | 到用量上限的账号保持；分组成员可临时暂停 | 只有 429 冷却 | 1.5 天 |

### 2.4 限额与共享（3 条）

| # | 差距 | magpie | Jolu | 工作量 |
|---|---|---|---|---|
| F1 | Key 的 `ttl` / `models` / `accounts` / `cost` / `readOnly` | 全部支持 | 只有 `requests_per_day`（`tokens_per_day` 存着不强制） | 1.5 天 |
| F2 | **共享与命名 Key 的 UI** | 完整页面（Keys 页、用量、限额编辑） | 后端 4 个命令就绪，**无界面** | 1.5 天 |
| F3 | `tokens_per_day` 强制 | `internal/budget` 精确记账 | 只存不强（归账时序限制） | 2 天 |

### 2.5 工具与分发（未开工，11 条）

| # | 条目 | 工作量 |
|---|---|---|
| G1 | Web 搜索 MCP（Tavily/Brave/Exa/Firecrawl/You.com/DeepSeek/豆包/元宝/SearXNG，多 Key 轮转、被拒 Key 排最后、Tavily 限额展示、`web_search` + `web_fetch`） | 4 天 |
| G2 | 图像生成工具（MCP，选定模型） | 1 天 |
| G3 | 视频工具（Grok 订阅 / MiniMax H3） | 1.5 天 |
| G4 | Library（指令 / MCP server / skills / subagents 按各 agent 格式写入；移除只删自己写的） | 3 天 |
| G5 | CLI 全功能（`provider add` / `claude <model>` / `group add` / `save` / `use` / `quota` / `settings set` / `tool` / `tui` / `serve`）——**需先从 Tauri 后端抽服务函数层** | 5 天 |
| G6 | TUI / Web UI | 5 天 |
| G7 | 远程网关（笔记本用桌面的 providers/groups） | 3 天 |
| G8 | Docker 镜像 + Web UI 管理 | 2 天 |
| G9 | 同步备份（WebDAV / S3 / 加密 GitHub） | 3 天 |
| G10 | agent 版本管理与更新 / WSL 接线 / 每日 warm-up / OTLP 导出 / Keep awake（打包 5 小项） | 4 天 |

---

## 3. 分批执行计划

### 批次 D（建议先做，约 3 天）：ZCode 链路收尾

**为什么先做**：C1+C2+C5 都是 zcode 链路问题，且**你现在无额度无法验收**——正好趁这个窗口把代码写完，额度恢复后一次性验收。

#### 3.1 C1 · Start Plan 礼包按余额过滤

**做什么**
1. 新增 `zcode_start::gift_models(jwt, device) -> Result<Option<Vec<String>>, String>`：
   - 参考 magpie 插件 `startPlan()`（`index.mjs` 189-224 行）：`GET {ZCODE}/api/v1/zcode-plan/billing/balance?app_version={APP_VERSION}`，`Authorization: Bearer <jwt>`，`X-Device-Mid: <device>`
   - 解析 `data.plans[]`：筛 `status == "active"`、未过期（`ends_at`）、未开始（`starts_at`）的条目；收集 `models[]`
   - 返回 `None` 表示「读不到余额」；`Some(vec![])` 表示「余额里没有模型」
2. `zcode_start::models()` 改为：先查余额 —— `Some(list)` 非空则用该列表；`None` 回退静态三模型；空列表回退静态三模型（不做「什么都不给」，否则用户看不到任何模型更糟）。
3. 缓存：余额查询有网络成本，按 `jwt` 哈希 + 5 分钟 TTL 缓存在内存（`LazyLock<Mutex<HashMap>>`，与 `MODEL_CACHE` 同模式）。
4. 失败语义：余额接口 401 → 明确「ZCode 会话已失效，请在原工具重新登录」；其他失败 → 回退静态清单并在日志写明原因。

**验收**
- 单测：余额响应 → 模型列表映射（含 active 过滤、未开始过滤、空列表回退）
- 单测：余额接口失败 → 回退静态清单
- 实机：额度恢复后，网关模型清单应与 ZCode 客户端 picker 里的 Start Plan 模型一致

#### 3.2 C2 · 双通道按账号记忆

**做什么**
1. `zcode.rs` 加进程级 `CHANNEL_MEMORY: LazyLock<Mutex<HashMap<String /*账号指纹*/, Channel>>>`，`Channel ∈ {StartPlan, CodingPlan}`。
2. `forward()` 的通道顺序：
   - `zcode_free_first == false`（默认）：若记忆是 `StartPlan` → 直接走 Start Plan；否则 Coding Plan → 失败落 Start Plan 并**记忆为 StartPlan**（仅当落回成功）
   - `zcode_free_first == true`：镜像逻辑
3. 记忆失效：Coding Plan 成功则改记 `CodingPlan`（额度恢复后自动切回）；账号指纹变化（换账号）自然失效。
4. 不持久化：进程内记忆即可，重启后第一次请求重新探测（代价一次往返）。

**验收**
- 单测：首次 Coding Plan 失败 → 落 Start Plan → 第二次直接走 Start Plan（不发 Coding Plan 请求）
- 单测：Start Plan 失败（额度耗尽）→ 落 Coding Plan → 记忆切回
- 实机：zcode 无额度时，第二次请求应直接打到 `zcode.z.ai`（看路由流水的耗时差异）

#### 3.3 C5 · 边缘防火墙保持

**做什么**
1. `zcode_start.rs` 加 `EDGE_HOLD: LazyLock<Mutex<Option<i64 /*until_ms*/>>>`。
2. Start Plan 通道收到 **405 且响应体含 `3012` 或 `unusual activity`** → 设 `EDGE_HOLD = now + 10 分钟`（magpie 的 `EDGE_HOLD` 同量级），直接返回一个可读的 503（「zcode.z.ai 暂时限制了本机请求，约 N 分钟后重试」）。
3. 保持期内：任何 Start Plan 请求直接返回该 503，不再打上游（省一次必然失败的往返）。
4. 恢复：到期自动清除；成功请求也清除。

**验收**
- 单测：405/3012 触发保持；保持期内不发上游；到期后恢复
- 实机：无法主动构造 405，靠日志观察真实触发（若有）

#### 3.4 §1.4 的 UI 可辨识性（顺带）

**做什么**：供应商页的 ZCode 卡片文案改为明确标出家族与登录形态：

- 「ZCode 国内版」→ 副标题「BigModel · 使用 bigmodel.cn 账号登录」
- 「ZCode 国际版」→ 副标题「Z.ai · 使用 chat.z.ai 账号登录」

**依据**：§1.4 验证的 `cli/init` 返回的两条 authorize_url 域名不同。这是**纯文案**，不动逻辑。

---

### 批次 E（约 4 天）：Claude Code 深耕 + Key 能力补齐

#### 3.5 D1 · subagent 独立模型入口

**做什么**
1. `wiring.rs`：`claude_wire` 加 `subagent_model: Option<&str>` 参数，写 `CLAUDE_CODE_SUBAGENT_MODEL`；不传则删键（跟随主模型）。
2. `Applied` 加 `subagent_model: Option<String>`（快照排除用）。
3. 接线面板 Claude 行加「Subagent 模型」下拉（默认「跟随主模型」+ 目录模型列表）。
4. 断开时按快照还原（同 `_NAME` 的处理模式）。

**验收**：单测三态（不设/设/还原）；实机在 Claude Code 里跑一个 subagent 任务看 `/status` 显示。

#### 3.6 D3 · 云端开关全集

**做什么**
1. 把 `CLAUDE_CLOUD_SWITCHES` 从 2 个扩到 magpie 的 `claudeClouds` 集合：读 magpie `internal/agent/claude.go` 的 `claudeClouds` 定义（`CLAUDE_CODE_USE_BEDROCK`、`CLAUDE_CODE_USE_VERTEX`，以及配套的 `ANTHROPIC_VERTEX_PROJECT_ID`、`CLOUD_ML_REGION` 等**只在开关开启时**才需要暂存的变量）。
2. 暂存语义：只暂存**存在值**的键，断开时精确还原（空值不写回）。

**验收**：单测覆盖每个键的暂存/还原；不写不存在值的键。

#### 3.7 F1 · 命名 Key 能力补齐

**做什么**
1. `gateway_client_keys` 表加列（迁移）：`expires_ms INTEGER`、`models_json TEXT`、`accounts_json TEXT`、`cost_per_day REAL`、`read_only INTEGER DEFAULT 0`。
2. `gateway_client_key_issue` 扩展可选参数：`ttl_secs`、`models`、`accounts`、`cost_per_day`、`read_only`。
3. authorize/limit 检查：
   - `expires_ms` 过期 → 401「Key 已过期」
   - `models` 非空且请求模型不在其中 → 403
   - `read_only` 且非 GET → 403
   - `cost_per_day` → 与 `tokens_per_day` 同样的「存着不强制」（F3 一起做）
4. `gateway_client_keys_list` 返回新字段。

**验收**：单测每个新约束（过期/模型白名单/只读）；迁移测试（旧表升级后新列为 NULL/默认值）。

---

### 批次 F（约 5 天）：两张缺席的 UI

**做法要求**：按 `frontend-design` skill 工艺；`Jolu.pen` 无画板的部分标「待确认」；沿用 `jolu-*` 令牌与现有小字体系。

#### 3.8 F2 · 共享与命名 Key 管理界面

**位置**：统一网关页 → 「接入设置」弹窗新分区，或独立弹窗（建议后者，内容多）。

**结构**
```
局域网共享（待确认）
├─ 开关 + 说明（开启后本机 IP:端口 可被局域网访问；需重启监听生效；观测模式永不共享）
├─ 安全提示（开启前先签发客户端 Key，否则局域网客户端无法通过严格校验）
└─ 命名客户端 Key（表格）
   ├─ 列：名称 / 掩码 / 限额 / 今日用量 / 最近使用 / 操作
   ├─ 签发按钮 → 弹窗（名称、每日请求上限、有效期限、模型白名单、只读）
   │    └─ 签发改成功页：Key 原文 + 复制按钮 + 「仅显示这一次」警示
   └─ 撤销按钮 → 确认（删除立即失效，历史归属保留）
```

**验收**：typecheck + 纯逻辑测试（限额展示、过期状态、只读标记）；实机点一遍（含复制 Key、撤销后旧 Key 401）。

#### 3.9 E2 · 三个内置中间件

**做什么**（`middleware.rs` 扩展，已有 `model-map` / `param-override` 的框架）
1. **system-prompt**：`{"prompt": "...", "agents": ["claude-code"], "models": []}` —— 命中则向 system 追加（Claude 的 system 是数组，追加一个 text 块；OpenAI 的 system 是 message）。
2. **word-guard**：`{"block": ["词"], "mask": ["词"], "mode": "block|mask"}` —— block 命中返回 400 并说明；mask 命中替换为 `***`。
3. **think-tags**：`{"mode": "strip|wrap"}` —— strip 剥掉响应里的 `<think>…</think>`；wrap 把 `reasoning_content` 包进 think 标签。

**关键约束**：这三个都**触碰消息内容**，必须与 P5 隐私脱敏定义执行顺序 —— **脱敏最先，中间件其次，转发最后**（中间件可能引入新的敏感值，脱敏必须能覆盖）。这个顺序要写进 `middleware.rs` 模块注释与 `mask.rs` 注释两处。

**验收**：每个中间件单测（命中/不命中/失败隔离）；与 mask 的先后顺序单测。

#### 3.10 E4 · Cap hold 与暂停规则

**做什么**
1. 路由分组加 `paused_members`（DB 列 or 表）：成员可设暂停到期时间。
2. `ordered_members` 过滤掉暂停中的成员（全员暂停则如实报「所有成员均暂停」）。
3. Cap hold：额度窗口显示 100% 已用的成员，在 `smart`/`pace` 排序里降到最后（不是剔除，避免全满时无候选）。

**验收**：单测暂停过滤（含全员暂停）；实机在管理来源里设一个暂停。

---

### 批次 G（约 6 天）：分发与可观测

#### 3.11 G5 · 服务函数抽层 + CLI（前置工程）

**这是 P8 唯一的硬前置**：CLI 无法直接调 Tauri 命令，必须先把业务逻辑从 `lib.rs` 的 `#[tauri::command]` 里抽成纯函数。

**做什么**
1. 抽层原则：`lib.rs` 的每个 command 变成「薄壳」，只做 `State` 解包 + 调 `service::*`；`service/` 下的函数接收 `&Connection` / `&Settings` / 显式参数，不依赖 `AppHandle`（需要发事件的传一个 `&dyn Fn(&str, Value)` 回调）。
2. 优先抽 5 个高频命令：`gateway_catalog`、`gateway_status`、`connection_quota`、`gateway_group_*`、`gateway_client_keys_*`。
3. CLI 骨架：`backend/src/bin/jolu.rs`（clap），子命令 `provider`/`claude`/`codex`/`group`/`save`/`use`/`quota`/`settings`/`serve`。

**验收**：抽层后既有测试全绿；CLI 能跑 `jolu quota`（读本机 DB，不启网关）。

#### 3.12 G10 · 小项打包

| 项 | 做法 | 工作量 |
|---|---|---|
| agent 版本管理与更新 | 检测各 CLI `--version`，按安装方式给更新命令（不自动执行） | 1 天 |
| WSL 接线 | `wsl.exe` 执行路径解析 + 配置写入（magpie `appdir` 语义） | 1.5 天 |
| 每日 warm-up | 复用自动化调度器，定时发一条最小请求（默认关，明示消耗） | 0.5 天 |
| OTLP 导出 | 用量数据按 OTLP HTTP 推送（导出前脱敏核查） | 1 天 |
| Keep awake | Windows `SetThreadExecutionState` | 0.5 天 |

---

## 4. 排除在计划外（明确不做）

| 项 | 理由 |
|---|---|
| **支持 `zcode://` 桌面版回调**（§1.5） | `cli/poll` 已能拿到等价 token；要注册系统协议 + 自己实现 code→token 交换（端点未公开），收益为零、攻击面增加 |
| **异步钩子（E1，Bun 运行时）** | 需引入 JS 运行时（体积 + 攻击面），且与现有同步中间件职责重叠。**先做 E2 的三个内置中间件**，等真有第三方扩展需求再评估 |
| **意图路由（E3）** | 每轮多一次模型调用（成本 + 延迟），且需要额外的意图模型选型与评测集。**等有真实评测数据支撑再做** |
| **订阅插件化**（P6.2，决策点） | 产品路线决策，非技术任务。magpie 外置的原因是厂商条款风险；Jolu 是否跟进要鼠鼠拍板 |
| **`tokens_per_day` 强制（F3）** | 依赖归账时序（用量在响应流结束后才记账，与准入检查不同步）。精确实现需要预算预留机制（magpie 的 `reservation`），工作量 2 天且容易与并发队列耦合 —— 排在批次 D/E 之后单独评估 |
| **magpie 的每日匿名遥测** | Jolu 无此需求，AGENTS.md 无授权 |
| **magpie 的 `usemagpie.ai` 导流 / 赞助体系** | 非功能 |

---

## 5. 风险与规避（跨批次）

| 风险 | 等级 | 规避 |
|---|---|---|
| **ZCode 着装协议漂移**（C6） | 高 | `zcode_start_prompt.rs` 是生成文件，来源写在模块头；ZCode 版本升级后按该头注释重新生成。**建议**：在 `docs/todo` 加一条「每次 ZCode 升级后检查 prompt 常量」，并把版本号做成常量便于 grep |
| **并发队列（C4）与限额（F1/F3）耦合** | 中 | 先做 C2（通道记忆，无状态）、再做 F1（Key 约束，读时判定）、最后 C4（队列，有状态）。**不要同时动三者**——状态机交织会让测试无法定位 |
| **中间件触碰消息内容**（E2） | 中 | 与 P5 脱敏的执行顺序必须先定义并写在两处注释（§3.9 已列）；每个中间件独立测试，失败隔离沿用现有框架 |
| **服务函数抽层（G5）大范围重构** | 高 | 只抽 5 个高频命令先跑通；**每抽一个跑一次全量测试**；不追求一次性抽完 |
| **CLI 与桌面端并发写配置** | 中 | 复用 `cli_switch::with_switch`（进程内 + 跨进程命名互斥量），CLI 也必须走它 |
| **UI 无画板依据** | 中 | 新 UI 全部标「待确认」；`Jolu.pen` 补齐后再定稿 |
| **zcode 额度恢复后才能验收 C1/C2/C5** | 高 | 这三个的代码可以现在写完 + 单测；**端到端验收单独列一个待办**，等额度恢复 |

---

## 6. 验收清单（交给鼠鼠的实机项）

每项标了「代码验收」与「实机验收」，实机的部分需要鼠鼠操作。

| # | 项 | 代码验收 | 实机验收 |
|---|---|---|---|
| C1 | 礼包过滤 | 单测 | ⏸ 等 zcode 额度 |
| C2 | 通道记忆 | 单测 | ⏸ 等 zcode 额度 |
| C3 | 席位校验 | 单测 | ⏸ 需团队账号 |
| C4 | 并发队列 | 单测 | 高并发压测 |
| C5 | 边缘保持 | 单测 | 日志观察 |
| D1 | subagent 入口 | 单测 | ✅ 可立即（Claude Code 订阅可用） |
| D3 | 云端开关 | 单测 | 需 Bedrock/Vertex 账号 |
| E2 | 三中间件 | 单测 | ✅ 可立即 |
| F1 | Key 能力 | 单测 | ✅ 可立即 |
| F2 | 共享 UI | typecheck + 逻辑测试 | ✅ 可立即 |
| G5 | CLI | 抽层回归 | `jolu quota` |

---

## 7. 推荐执行顺序

```
批次 D（ZCode 收尾，3 天）
  ├─ D-1  §3.4 UI 文案（最快，先做，无风险）
  ├─ D-2  §3.1 C1 礼包过滤（单测可验，实机等额度）
  ├─ D-3  §3.3 C5 边缘保持
  └─ D-4  §3.2 C2 通道记忆（放最后：依赖前两项稳定）

批次 E（Claude 深耕 + Key，4 天）
  ├─ E-1  §3.5 D1 subagent（用户可感知，先做）
  ├─ E-2  §3.6 D3 云端开关
  └─ E-3  §3.7 F1 Key 能力

批次 F（UI + 中间件，5 天）
  ├─ F-1  §3.8 F2 共享与 Key UI（补齐上批后端）
  ├─ F-2  §3.9 E2 三中间件
  └─ F-3  §3.10 E4 Cap hold / 暂停

批次 G（分发，6 天）
  ├─ G-1  §3.11 G5 服务抽层 + CLI 骨架
  └─ G-2  §3.12 G10 小项打包
```

**总计约 18 天**（不含 G1–G4、G6–G9 这些大模块）。

每个批次完成后：全量测试（基线 1047/417 只增不减）+ 按阶段提交（默认 `[skip ci]`）+ 写 `docs/verification/` 记录 + 给出实机验收清单。

---

## 8. 附：本批验证用的关键命令

```bash
# ZCode 两条家族发起（验证登录链路可用）
curl -X POST https://zcode.z.ai/api/v1/oauth/cli/init \
  -H 'content-type: application/json' -H 'User-Agent: ZCode/3.17.2' \
  -H "Authorization: Bearer $(openssl rand -hex 32)" \
  -d '{"provider":"zai"}'      # → chat.z.ai/api/oauth/authorize?client_id=client_P8X5CMWmlaROgyO-KSqtg
  # provider 换成 "bigmodel"   # → bigmodel.cn/login?appId=zcode

# 你给的地址（桌面版回调，Jolu 不走这条）
curl -I 'https://chat.z.ai/api/oauth/authorize?client_id=client_P8X5CMWmlaROgyO-KSqtg&redirect_uri=https%3A%2F%2Fzcode.z.ai%2Fapp%2Foauth%2Flogin...&state=...&response_type=code'
  # → 307 到 /auth（网页登录页）

# 穿网关端到端（用池里真实来源）
curl -X POST http://127.0.0.1:12731/v1/messages \
  -H 'content-type: application/json' -H 'x-api-key: any' -H 'anthropic-version: 2023-06-01' \
  -d '{"model":"workbuddy-16f911/glm-5.3","max_tokens":16,"stream":true,"messages":[{"role":"user","content":"say ok"}]}'
  # → 200 + SSE "OK"
```
