# Jolu 能力矩阵与验证基线

日期：2026-11-23（2026-11-24 按 P0-1 要求补齐）
状态：**P0-1 交付物。11 项能力 × 7 档验证已覆盖两个维度：13 家 Agent（§1）与 22 种「平台 × 地区 × 连接类型」（§2）。实现列以代码为准（文件:行号或函数名）；验证列以 `docs/verification/` 及其他已有执行记录为准，两者分开标注。剩余差距是缺真实账号 / 真实样本的组合，见 §4「未验证清单」。**
配套计划：[产品对齐执行计划](../plans/2026-11-23-product-alignment-execution-plan.md)
配套：[连接能力矩阵](2026-11-23-connection-capability-matrix.md)（额度 / 模型发现 / 转发三列的原始核查，本文 §2 已吸收并按地区、连接类型拆开）

---

## 0. 怎么读这份文档

这份文档回答两个**不同**的问题，必须分开看：

1. **实现了吗**：代码里有没有这条路径（附文件与行号 / 函数名）
2. **验证过吗**：有没有真实验收记录，还是只有模拟，或者根本没有

**「实现」不等于「验证」**。本文不把「代码存在」写成「通过」，也不把「有单元测试」写成「有验收记录」。

### 0.1 11 项能力在两个维度里各指什么

| 能力 | Agent 维度（§1） | 连接维度（§2） |
|---|---|---|
| 账号识别 | 网关认出请求来自哪个 Agent（接线令牌 / originator / UA），以及能否认出 Agent 原生会话 | 认出连接背后是哪个账号（身份接口、本机登录、账号指纹） |
| 额度 | ↘ 归连接（Agent 本身没有额度） | 官方额度窗口 / 余额 / 套餐 |
| 签到 | ↘ 归连接；没有签到平台的 Agent 记「不适用」 | 每日签到或礼包领取 |
| 模型发现 | Agent 侧拿到的模型清单从哪来（接线写入目录 / 网关 `/models`） | 从上游实时拉取该账号可用模型 |
| 路由 | Agent 接线后进入网关的协议入口与选路 | 本连接被选中后发往上游的转发链路 |
| 接线 | 一键把 Agent 配置指向 Jolu 网关、断开可还原 | 「启用连接」把凭据写回原 CLI（`cli_apply.rs`，cc-switch 式切换） |
| 用量 | 读 Agent 本机日志 / 库入 `usage.db` | 经网关转发的请求按连接归账 |
| 会话 | 会话列表与正文读取 | ↗ 归 Agent / 平台（无 Agent 的平台直接写结论） |
| 续跑 | 会话「在原工具继续」+ 自动化续接 / 等额度续跑 | 自动化按该连接额度等待后续跑 |
| Skills | Skills 目录发现与读写 | ↗ 归 Agent / 平台（无 Agent 的平台直接写结论） |
| 工具调用 | Agent 的工具请求经网关的处理（MCP 搜索、服务端工具代执行等） | 适配器对调用方 `tools` / 工具调用的转换与转发 |

`↗` / `↘` 表示「这项挂在另一个维度，见对应表」，**不计入 §3 的档位统计**，避免同一事实被数两次。

### 0.2 验证强度：7 档

| 档 | 表中写法 | 含义 |
|---|---|---|
| **真实端到端通过** | 端到端 | 真实账号 / 真实工具 / 真实上游，含真实写操作（如真实领取、真实写入工具配置） |
| **真实只读通过** | 只读 | 只读取真实数据并得到正确结论，无写操作 |
| **部分通过** | 部分 | 部分场景、版本或环节在真实条件下通过，其余缺样本；或真实工具 + 合成上游 |
| **模拟通过** | 模拟 | 有验收 / 执行记录，但只用 fixture、回环假上游、假 CLI、合成凭据 |
| **无验证记录** | 无记录 | 代码在，没有任何验收记录（只有单元测试也记这一档，格内注明「有单测」） |
| **不适用** | 不适用 | 平台本身没有这项能力，或 Jolu 有意不做；Jolu 尚未实现的写「不适用（未实现）」 |
| **失败** | 失败 | 真实条件下执行过且未通过；格内写明原因（缺登录导致的拒绝也记这一档，并注明不是实现缺陷） |

定档规则（逐格适用）：

1. 只按记录写的结果定档，不按代码推断；多份记录冲突时取较新的真实结果，并在格内或 §6 写明。
2. 「只跑到初始化成功」「真实 CLI + 合成上游」最多记「部分」；连接维度里上游是合成的，一律记「模拟」。
3. 真实失败后又有真实成功的，记成功档，失败写入备注（如 WorkBuddy 国内额度首轮 401）。
4. 记录写「部分场景通过」的记「部分」。

### 0.3 验证依据代号

为了表格可读，验证格里用代号引用记录，冒号后是行号或小节。均为仓库相对路径。

| 代号 | 文件 |
|---|---|
| AB | `docs/verification/2026-10-23-account-backend.md` |
| AQ | `docs/verification/2026-10-26-all-provider-queries.md` |
| AR | `docs/verification/2026-10-22-automation-real-run.md` |
| BA | `docs/verification/2026-10-26-provider-browser-authorization.md` |
| BL | `docs/verification/2026-10-26-all-provider-browser-login.md` |
| CD | `docs/verification/2026-10-27-account-connect-dialogs.md` |
| CL | `docs/verification/2026-10-26-claude-local-config-precedence.md` |
| CO | `docs/verification/2026-11-17-agent-context-observation.md` |
| CX | `docs/plans/2026-11-09-complete-remaining-work.md:216`（Codex 远程压缩代办执行记录） |
| DA | `docs/verification/2026-10-22-desktop-acceptance.md` |
| DC | `docs/verification/2026-10-27-daily-checkin.md` |
| GD | `docs/verification/2026-11-06-unified-gateway-desktop.md` |
| J | `AGENTS.md` 模块 J 行（Kimi / dsh / Pencil·Qoder·Crush / qwen·WorkBuddy 接线与接线重构的执行记录；重构计划 `docs/plans/2026-11-20-agent-wiring-refactor-plan.md`） |
| MF | `docs/verification/2026-11-13-account-card-model-fetch.md` |
| P3 | `docs/verification/2026-11-23-p0-3-multi-platform-reconciliation.md` |
| PA | `docs/verification/2026-10-25-provider-access-continuation.md` |
| PF | `docs/verification/2026-10-24-real-acceptance-preflight.md` |
| QA | `docs/verification/2026-11-04-qoder-subscription-adapter.md` |
| QM | `docs/verification/2026-11-14-qoder-model-list.md` |
| RV | `docs/verification/2026-10-23-real-validation.md` |
| SA | `docs/verification/2026-11-04-subscription-api.md` |
| TA | `docs/verification/2026-11-04-trae-subscription-adapter.md` |
| TK | `docs/verification/2026-10-27-trae-checkin-protocol.md` |
| TS | `docs/verification/2026-10-27-trae-official-statistics.md` |
| UI | `docs/todo/2026-10-22-unverified-integrations.md` |
| WC | `docs/verification/2026-11-04-workbuddy-channel-validation.md` |
| WK | `docs/verification/2026-10-27-workbuddy-checkin-protocol.md` |
| WM | `docs/verification/2026-11-04-workbuddy-model-catalog.md` |
| ZB | `docs/verification/2026-11-05-zcode-account-balance.md` |
| ZC | `docs/verification/2026-11-09-zcode-channels-and-client-keys.md` |
| ZS | `docs/verification/2026-11-14-zcode-session-running-state.md` |
| ZV | `docs/verification/2026-11-13-zcode-v2-credential.md` |
| ZZ | `docs/verification/2026-11-05-zcode-zai-quota-endpoint.md` |

---

## 1. Agent 维度（13 家）

### 1.1 实现矩阵 A：探测、接线、用量采集、会话、Skills、签到、订阅转发

`✅` = 已实现（附入口）｜`—` = 明确没有｜`?` = 需确认

| Agent | 探测 | 接线 | 用量采集 | 会话读取 | Skills | 签到 | 订阅转发 |
|---|---|---|---|---|---|---|---|
| **claude-code** | ✅ `agent_discovery.rs:334` Standard，`bin=claude` | ✅ `wiring_specs.rs:203` → `adapters.rs:19/395`，1 文件，`LegacyLiteral` | ✅ `collector.rs:544`（`~/.claude/projects/*.jsonl`） | ✅ 正文可读 `session_threads.rs:95` | ✅ 可写 `skills.rs:105` | — | ✅ `adapter/claude.rs:207` |
| **codex** | ✅ `:335` Standard，`sign_in=auth.json` | ✅ `:218` → `adapters.rs:101/432`，1 文件，`Ignored` | ✅ `collector.rs:545`（`rollout-*.jsonl`） | ✅ 正文可读 `:97` | ✅ 可写 `:117` | — | ✅ `adapter/codex.rs:260` |
| **gemini** | ✅ `:337` **CliOnly** | ✅ `:237` → `adapters.rs:135/635`，**2 文件** | ✅ `native_sources.rs:42`（整份快照重建） | 🟡 列表有、**正文不可读** | ✅ 可写 `:130` | — | ✅ `adapter/gemini_subscription.rs:329` |
| **opencode** | ✅ `:338` Standard | ✅ `:252` → `adapters.rs:160/451`，1 文件 | ✅ `:40`（sqlite v1/v2 双分支） | 🟡 列表有、正文不可读 | ✅ 可写 `:131` | — | ✅ 复用 codex 适配器 `wire.rs:1135` |
| **zcode** | ✅ `:336` Standard | ✅ `:267` → `adapters.rs:179/608`，**2 文件** | ✅ `:45`（`cli/db/db.sqlite`） | ✅ 正文读 sqlite `:1164`，含子代理过滤 | ✅ 可写 `:134` | ⚠️ **礼包领取**非每日签到 | ✅ `adapter/zcode.rs:595` |
| **kimi** | ✅ `:340` Standard | ✅ `:282` → `adapters.rs:224/470`，1 文件，`Exact` | — **不入库**（见 §6.8） | ✅ `kimi_sessions.rs:193`（`has_source=false`） | — | — | — |
| **dsh** | ✅ `:345` Standard（**file 证据已过期**，见 §6.6） | ✅ `:297` → `adapters.rs:336/555`，**多文件**（N profile + `.env` + credentials） | — | ✅ `dsh_sessions.rs:215`（zstd 多帧） | — | — | — |
| **pi** | ✅ `:342` Standard | ✅ `:312` → `adapters.rs:298/488`，**2 文件** | — | ✅ `pi_sessions.rs:196` | — | — | — |
| **pencil** | ✅ `:344`（**无 CLI**） | ✅ `:327` → `adapters.rs:370/588`，1 文件 | — | — **不产生会话** | — | — | — |
| **qoder** | ✅ `:348` Standard | ✅ `:342` → `adapters.rs:370/588`，1 文件 | ✅ `:54` jsonl + **`qoder_usage.rs` 桌面账本**（见 §6.9） | 🟡 列表有、正文不可读 | ✅ 可写 `:132` | ✅ `checkin_qoder.rs:40` | ✅ `adapter/qoder.rs:650` |
| **qoder-cn** | ✅ `:349` Standard | ✅ `:357` → 同上，1 文件 | ✅ `:60`（`native:qoder:cn`） | 🟡 同 qoder | ⚠️ **无独立入口**（见 §6.5） | ✅ 走 `edition=cn` | ✅ 同一适配器 |
| **qwen** | ✅ `:339` Standard | ✅ `:372` → `adapters.rs:253/519`，1 文件，`Exact` | — | — | — | — | — |
| **workbuddy** | ✅ `:350`（**无 CLI**） | ✅ `:389` → `adapters.rs:276/534`，1 文件，`Prefixed` | ⚠️ `:66` 有读取器，**本机无客户端目录**（见 §6.10） | 🟡 列表有、正文不可读 | 🟡 **只读** `:135` | ✅ `checkin_workbuddy.rs:129` | ✅ `adapter/workbuddy.rs:368` |

行号说明：探测列的 `:NNN` 指 `agent_discovery.rs`，接线列指 `gateway/wiring_specs.rs`，用量列指 `native_sources.rs`（另注明的除外），会话列指 `session_threads.rs`，Skills 列指 `skills.rs`。opencode 的订阅转发行号已由旧版的 `wire.rs:1088` 更正为当前的 `:1135`。

### 1.2 实现矩阵 B：账号识别、额度、模型发现、路由、续跑、工具调用

三条公共路径（适用于全部 13 家，下表只写各家差异）：

- **调用方识别**：`gateway/wire.rs:294` `agent_from_headers`（接线令牌 `jolu-<agent>.<实例>` → originator → UA 前缀；UA 前缀登记在 `wiring_specs.rs` 各行的 `ua` 字段）。
- **原生会话识别**：`wire.rs:197` `session_identity`，**只认 claude-code / codex / opencode**，其余 Agent 返回 `None`（`wire.rs:218`），只记流水，不建会话。
- **路由**：所有入口进 `wire.rs:473` `handle` 选路 → `wire.rs:989` `forward_one`；具体转发能力取决于被选中的连接（§2.2）。

| Agent | 账号识别（调用方 / 原生会话） | 额度 | 模型发现（Agent 侧清单来源） | 路由（入口协议） | 续跑 | 工具调用 |
|---|---|---|---|---|---|---|
| **claude-code** | UA `claude-cli` / `claude-code`（`wiring_specs.rs:216`）；会话 `x-claude-code-session-id` 或 `metadata.user_id`（`wire.rs:199-204`） | ↘ | 接线写默认模型、档位、Subagent 模型（`wiring/adapters.rs:19-99` `wire_claude`，`uses_catalog=false` `wiring_specs.rs:215`）；可选项来自网关 `/v1/models`（`server.rs:21`） | Anthropic Messages `/v1/messages`（`server.rs:23`） | ✅ 在原工具继续 `claude --resume`（`session_threads.rs:1239`、`:1275` `continue_session`；前端 `lib/sessionThreads.ts:173` `canContinue`）；自动化续接 / 等额度续跑（`automation.rs:52`、`:1181` `--resume`、`:834-888` 等待额度） | ✅ 透传；上游非真 Claude 时 `tool_loop` 代执行 WebSearch / WebFetch（`wire.rs:905-917`）；可装 MCP 搜索（`library/agents.rs` `mcpServers`） |
| **codex** | UA `codex` / `codex_cli_rs` / `acp-extension-codex`（`wiring_specs.rs:235`）；会话 `session-id`（`wire.rs:205-210`） | ↘ | 只写地址与令牌（`adapters.rs:101-133`），模型由网关 `/v1/models`、`/backend-api/codex/models` 提供（`server.rs:21,31`） | Responses `/v1/responses`；保留 ChatGPT 登录时 `/backend-api/codex/responses`（`server.rs:27-30`） | ✅ `codex resume`（`session_threads.rs:1240`）；自动化（`automation.rs:53`、`:1172`）；第三方模型的远程压缩代办（`gateway/responses_chat.rs`，CX） | ✅ 透传；Responses→Chat 时保留 custom tools（`wire.rs:1245` `original_custom_tools`）；可装 MCP 搜索（`mcp_servers`）；`tool_loop` 不涉及（只管 Anthropic 调用方） |
| **gemini** | UA `geminicli` / `gemini-cli` 及 `proxy_client=geminicli`（`wire.rs:323-331`）；**无会话标识**（API Key 模式请求不带，GD:35） | ↘ | 网关 `/v1beta/models`（`server.rs:33`） | Gemini 原生 `/v1beta/models/{*call}` → `adapter/gemini.rs:28` `handle` | — 未实现（`session_threads.rs:1237-1241` 与 `automation.rs:50-56` 只收 claude / codex） | ✅ `functionDeclarations` ↔ Chat 工具互转（`adapter/gemini.rs:388-410`）；`googleSearch` 等内置工具明确拒绝（`:392`）；可装 MCP 搜索 |
| **opencode** | UA `opencode`；会话 `x-opencode-session-id`（子会话归父，`wire.rs:211-217`） | ↘ | 只写 `provider.jolu`（`@ai-sdk/openai-compatible`，`wiring.rs:666-683`），**不写模型清单**；?（待确认：OpenCode 对该 provider 是否自动请求 `/models`，未核实） | Chat `/v1/chat/completions` | — 未实现 | ✅ 透传；可装 MCP 搜索（`opencode.json` 的 `mcp`） |
| **zcode** | UA `zcode`（`wiring_specs.rs:280`）；无原生会话识别 | ↘ | 网关目录写入配置（`uses_catalog=true`；`escape.rs:190-194`；`wiring.rs:919/968` `anthropic-messages`） | Anthropic Messages | — 未实现 | ✅ 透传；MCP 搜索**不支持**（`library/agents.rs` 头注释：zcode 的 `mcp.servers` 形状未实测） |
| **kimi** | UA `kimicli`（`wiring_specs.rs:295`） | 不适用 | 网关目录写入 `models."jolu/<id>"`（`wiring.rs:1190-1237`，`uses_catalog=true`） | `providers.jolu`（`type=kimi`，`wiring.rs:1190`）；协议未在本次核实 | — 未实现 | 透传；无 MCP 搜索 |
| **dsh** | UA `deepseek-harness`（`wiring_specs.rs:310`） | 不适用 | 网关目录写入各 profile 的 `cordis.patch.yml`（`gateway/wiring_dsh.rs`） | Chat（v1 只写 `openai-completions` 路由，J） | — 未实现 | 透传；无 MCP 搜索 |
| **pi** | **UA 未登记**（`wiring_specs.rs:325` `ua: &[]`），只能靠接线令牌 | 不适用 | 网关目录写入 `models.json`（`gateway/wiring_pi.rs:139` `openai-completions`、`:243` `openai-responses`） | Chat / Responses | — 未实现 | 透传；无 MCP 搜索 |
| **pencil** | UA 未登记（`wiring_specs.rs:340`） | 不适用 | 网关目录写入（`gateway/wiring_extra.rs:104` `openai-completions`） | Chat | 不适用（不产生会话） | 透传；无 MCP 搜索 |
| **qoder** | UA 未登记（`wiring_specs.rs:355`） | ↘ | 网关目录写入 `providers.jolu` + `model.name`（`wiring_extra.rs:121` `protocol=openai`） | Chat | — 未实现 | 透传；无 MCP 搜索 |
| **qoder-cn** | UA 未登记（`wiring_specs.rs:370`） | ↘ | 同 qoder | Chat | — 未实现 | 同 qoder |
| **qwen** | UA `qwencode`（`wiring_specs.rs:387`） | 不适用 | 网关目录写入 `modelProviders.openai`（`gateway/wiring_qwen.rs:165-188`） | Chat | — 未实现 | 透传；无 MCP 搜索 |
| **workbuddy** | UA `workbuddy`（`wiring_specs.rs:404`） | ↘ | 网关目录写入 `models.json`（`gateway/wiring_workbuddy.rs`） | 按 `models.json` 条目；协议未在本次核实 | — 未实现 | 透传；无 MCP 搜索 |

### 1.3 「工具调用」是五种不同的能力

| 能力 | 做什么 | 作用对象 | 代码出处 | 验证 |
|---|---|---|---|---|
| **MCP 搜索** | `jolu.exe --mcp-search` 以 stdio 说 MCP，向 Agent 暴露 `web_search` / `web_fetch` | 只装进 claude-code / gemini / codex / opencode 的 MCP 配置 | `library/mcp_search.rs:28` `handle_line`；`lib.rs:1867` `run_mcp_search`；`library/agents.rs` 头注释的四家表 | 无记录（有单测 `library/agents.rs:346-462`） |
| **服务端工具代执行（tool_loop）** | 请求声明 WebSearch / WebFetch 服务端工具、上游又不是真 Claude 时，由网关执行并续问，最多 5 轮 | Anthropic 协议调用方（实际是 Claude Code；`tool_loop.rs:52` 只接管服务端工具） | `gateway/tool_loop.rs:1-27`；`wire.rs:905-917`、`:1284` `with_tool_loop` | 无记录（有单测） |
| **Claude 订阅工具桥** | 调用方的 `tools` 由网关当 MCP 服务器提供给官方 `claude` 客户端，模型调工具时挂起，等调用方带 `tool_result` 回来续跑 | Claude 订阅连接 | `adapter/claude_bridge.rs:1-12`、`:39` `wanted`、`:403` `mcp`；`adapter/claude.rs:228-229`；`server.rs:35` | 无记录 |
| **订阅适配器的工具转换** | 把调用方的 `tools` / `tool_calls` 翻译成各订阅上游的格式 | 各订阅连接 | 见 §2.2「工具调用」列 | 见 §2.4 |
| **文本工具调用解析（text_calls）** | 把模型写在正文里的 `<tool_call>`、GLM `<arg_key>`、DeepSeek DSML 还原成结构化 `tool_calls` | Qoder、Trae 适配器 | `adapter/text_calls.rs:1-15`、`:75`；`qoder_chat.rs:3,647`；`trae_chat.rs:381-415` | Trae 模拟（TA「转换与失败语义」）；Qoder 无记录（有单测） |

**工具调用的整条真实链路（模型发起 → 工具执行 → 结果回传 → 最终回答）目前没有任何一种组合有记录。** 最接近的是 CO:22：四个真实 CLI 各做两轮工具调用，但上游是回环假模型。

### 1.4 特殊之处逐家说明

- **claude-code**：唯一用 `TokenMatch::LegacyLiteral`（兼容历史字面量 `jolu-claude-code`）。会话与工具调用解析最完整。
- **codex**：`token_match=Ignored` 且带 OAuth 分支——登录 ChatGPT 时只改地址、不写 token。
- **gemini**：13 家里唯一 `Detect::CliOnly`（`~/.gemini` 也被 Antigravity / Code Assist 用）。用量是整份快照重建，rewind 会删消息。请求不带会话标识，网关只记流水。
- **opencode**：sqlite v1/v2 与旧 JSON 三种格式，共存时只读 sqlite 防重复。
- **zcode**：唯一「库型」会话来源。签到是**礼包领取**（`checkin_zcode.rs:1-24`）而非每日签到。
- **kimi**：会话只能从 `~/.kimi-code/` 只读解析，**用量采集器完全缺失**。
- **dsh**：令牌在 `$DSH_HOME/.env` 而非主文件，所以整个 `is_wired` 交给 adapter 自己判。会话是 **zstd 多帧压缩**，需逐帧解压。
- **pi**：接线要写两个文件（`models.json` + `settings.json`），状态判定要读第二个文件。没有登记 UA，未带令牌的请求认不出来。
- **pencil**：没有 CLI，只能靠配置文件判装。**它本身不产生会话记录。**
- **workbuddy**：`models.json` 是顶层数组（或包装对象），两种形状都要认。Skills 只读。

### 1.5 Agent 验证矩阵（13 × 11）

| Agent | 账号识别 | 额度 | 签到 | 模型发现 | 路由 | 接线 | 用量 | 会话 | 续跑 | Skills | 工具调用 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **claude-code** | 部分 [GD:22，真实 CLI + 合成上游] | ↘ | 不适用 | 模拟 [J：`agents-routing-r2.e2e.mjs` 预览] | 部分 [GD:20-22] | 模拟 [J：生产路径往返测试] | 只读 [P3 §1，3,295/3,295] | 只读 [PF:18，149 个会话] | 部分 [AR:17 续接真实通过；AR:19 耗尽后续跑未测；在原工具继续无记录] | 部分 [DA:13-15，隔离目录] | 模拟 [CO:22，假上游] |
| **codex** | 部分 [GD:23] | ↘ | 不适用 | 无记录 | 部分 [GD:20,23] | 模拟 [J] | 部分 [PF:19 只读统计；P3 未覆盖 Codex，未按 ID 对账] | 只读 [PF:19，39 个会话] | 部分 [AR:17,19；CX 真实 Codex 0.169 + 合成上游] | 部分 [DA，隔离目录] | 模拟 [CO:22] |
| **gemini** | 部分 [GD:25-26] | ↘ | 不适用 | 无记录 | 部分 [GD:25] | 模拟 [J] | 无记录 | 无记录 | 不适用（未实现） | 模拟 [UI，合成] | 模拟 [CO:22] |
| **opencode** | 部分 [GD:24] | ↘ | 不适用 | 无记录 | 部分 [GD:20,24] | 模拟 [J] | 只读 [P3 §2，132/132] | 部分 [RV:48；GD:24] | 不适用（未实现） | 模拟 [RV，读写启停只有合成] | 模拟 [CO:22] |
| **zcode** | 无记录（有单测） | ↘ | ↘ | 模拟 [J] | 无记录（CO 未抓 ZCode） | 模拟 [J] | 只读 [P3 §5，18,872] | 部分 [ZS，运行态真机；正文 sqlite 未真机] | 不适用（未实现） | 模拟 [UI] | 无记录 |
| **kimi** | 无记录（有单测） | 不适用 | 不适用 | 部分 [J：真机写入 28 张模型表；Kimi 侧显示 / 选用未记录] | 无记录 | 端到端 [J：真机接入 / 断开往返，0 残留] | 不适用（未实现，§6.8） | 无记录 | 不适用（未实现） | 不适用（未实现） | 无记录 |
| **dsh** | 无记录（有单测） | 不适用 | 不适用 | 部分 [J：副本 + `DSH_HOME` 隔离往返] | 无记录 | 部分 [J：同左，非原配置] | 不适用（未实现，§6.8） | 无记录 | 不适用（未实现） | 不适用（未实现） | 无记录 |
| **pi** | 无记录 | 不适用 | 不适用 | 模拟 [J] | 无记录 | 模拟 [J] | 不适用（未实现，§6.8） | 无记录 | 不适用（未实现） | 不适用（未实现） | 无记录 |
| **pencil** | 无记录 | 不适用 | 不适用 | 模拟 [J：未做真实工具验收] | 无记录 | 模拟 [J] | 不适用（不产生会话） | 不适用（不产生会话） | 不适用 | 不适用（未实现） | 无记录 |
| **qoder** | 无记录 | ↘ | ↘ | 模拟 [J] | 无记录 | 模拟 [J] | 只读 [P3 §3.2c，999/999] | 无记录 | 不适用（未实现） | 模拟 [UI] | 无记录 |
| **qoder-cn** | 无记录 | ↘ | ↘ | 模拟 [J] | 无记录 | 模拟 [J] | 无记录（本机无 `~/.qoder-cn` 样本） | 不适用（缺入口，§6.5） | 不适用（未实现） | 不适用（缺入口，§6.5） | 无记录 |
| **qwen** | 无记录（有单测） | 不适用 | 不适用 | 模拟 [J] | 无记录 | 模拟 [J] | 不适用（未实现） | 不适用（未实现） | 不适用（未实现） | 不适用（未实现） | 无记录 |
| **workbuddy** | 无记录（有单测） | ↘ | ↘ | 模拟 [J] | 无记录 | 模拟 [J] | 无记录（本机无客户端，P3 §4.1） | 无记录 | 不适用（未实现） | 模拟 [UI，只读扫描] | 无记录 |

---

## 2. 平台 × 地区 × 连接类型（22 种连接）

### 2.1 连接清单

来源：`frontend/src/lib/providerCatalog.ts:27-74`（`PROVIDERS`，含 `edition`）、`frontend/src/lib/platformCatalog.json`（平台能力）、`backend/src/connections.rs` `prepare_new`（`:974` 起，平台与 `kind` 的允许组合）。`auth` = 账号 / 订阅 / 本机登录，`api` = API Key。

| # | 连接 | platform / kind / edition | 依据 |
|---|---|---|---|
| C1 | Claude · 订阅 | `claude` / auth | `providerCatalog.ts:30` |
| C2 | Claude · API Key | `claude` / api | 同上 |
| C3 | Codex · 订阅 | `codex` / auth | `:31` |
| C4 | Codex · API Key | `codex` / api | 同上 |
| C5 | Gemini · Google 登录 | `gemini` / auth | `:32` |
| C6 | Gemini · API Key | `gemini` / api（地址锁定官方） | 同上；`provider_key.rs:43` |
| C7 | Grok · OAuth / 设备授权 | `grok` / auth | `:33` |
| C8 | Grok · API Key | `grok` / api（地址锁定官方） | 同上；`provider_key.rs:43` |
| C9 | OpenCode · OpenAI 订阅 | `opencode` / auth（只能从 OpenCode 本机登录读取，`connections.rs:983-984`） | `:34`、`:320` |
| C10 | OpenCode · API Key（OpenCode Go / OpenAI 兼容） | `opencode` / api | `:319`、`:321` |
| C11 | ZCode 国内 · BigModel 账号 | `zcode` / auth / `bigmodel` | `:42` |
| C12 | ZCode 国际 · Z.ai 账号 | `zcode` / auth / `zai` | `:46` |
| C13 | ZCode · Start Plan 通道 | 同一 `zcode` auth 连接的第二通道（会话 JWT，`adapter/zcode_start.rs:1-21`，`start_channel` 可手动锁定，`zcode.rs:606-623`） | AGENTS.md 模块 J |
| C14 | ZCode 国内 · API Key | `zcode` / api，`open.bigmodel.cn`（Anthropic / OpenAI 两协议地址） | `:42-45` |
| C15 | ZCode 国际 · API Key | `zcode` / api，`api.z.ai`（两协议地址） | `:46-49` |
| C16 | Qoder 国际 | `qoder` / auth / `global` | `:51` |
| C17 | Qoder CN | `qoder` / auth / `cn` | `:50` |
| C18 | Trae 国际 | `trae` / auth / `global`（本机来源 `native:trae:ide`） | `:73` |
| C19 | Trae CN | `trae` / auth / `cn` | `:72` |
| C20 | WorkBuddy 国内 | `workbuddy` / auth / `cn`（本机来源 `native:workbuddy:main`） | `:52` |
| C21 | WorkBuddy AI 国际 | `workbuddy` / auth / `ai` | `:53` |
| C22 | 自定义供应商 · API Key（含 DeepSeek、Kimi、OpenRouter 等 12 个预设） | `custom` / api（`connections.rs:978-979` 只允许 api） | `:29`、`:60-71` |

不单列的：ZCode 团队套餐（`platformCatalog.json` zcode `limitation`「团队及 Start Plan 依服务权限」，无样本，归入 C11 / C12 的未验证项）；Qoder、Trae、WorkBuddy 没有 API Key 方式（`providerCatalog.ts` 中 `apiKey: "none"`）。

### 2.2 实现矩阵 A：账号识别、额度、签到、模型发现、路由、工具调用

公共路径：所有连接的账号指纹 `gateway/identity.rs:65` `account_fingerprint`；浏览器授权凭据的身份与官方账号 `provider_browser_auth.rs:15-35`（`Credential`）、`:52` `account_label`；额度查询路线集中在 `quota_registry.rs:38` `route`，命令分派 `commands/quotas.rs:128-218`；签到平台过滤 `daily_checkin.rs:145`（只收 `auth` 且 workbuddy / qoder / trae / zcode）；订阅连接模型拉取 `commands/providers.rs:113-121`，API 连接 `connections.rs:1119` `fetch_models`；路由 API 分支 `wire.rs:1013-1116`，订阅分支 `wire.rs:1118-1227`。

| # | 账号识别 | 额度 | 签到 | 模型发现 | 路由 | 工具调用 |
|---|---|---|---|---|---|---|
| C1 | 本机登录 `login_detect.rs:25` `detect_claude`；按凭据回认账号 `:41-55`；完整身份预检 `claude_auth.rs`（官方 `/api/oauth/profile`，AB:109 起） | `ClaudeOauth`（`quota_registry.rs:53` → `quota::claude_oauth_quota`） | — | — 无实时拉取分支（`providers.rs:120`），用内置清单 | 订阅分支，**只收 Anthropic Messages**（`wire.rs:1124-1125`）→ `adapter/claude.rs:207`，驱动官方 `claude` 客户端 | 有 `tools` / 流式 / 图片时走工具桥（`claude.rs:228-229` → `claude_bridge.rs`） |
| C2 | 只有账号指纹；本机配置读取 `creds.rs`（CL） | `api` 且有地址 → `ApiBaseUrl`（编程套餐域名或 sub2api，`quotas.rs:201-209`）；无地址 → `ApiKeyNoPlan`（`quota_registry.rs:55`） | — | `fetch_models`，`x-api-key` 鉴权（`connections.rs:1258-1269`） | API 分支直连 `adapter/direct.rs`；Chat / Responses 调用方转 Anthropic（`wire.rs:1022-1023`） | 透传；上游不是真 Claude 时 `tool_loop` 代执行（`wire.rs:907`） |
| C3 | `login_detect.rs:26` `detect_codex`；`codex_subscription_for_secret`（`:59`）；浏览器授权 `Credential::Cli` | `CodexOauth`（`quota_registry.rs:54`，`/backend-api/wham/usage`） | — | — 无分支，固定清单 | **只收 OpenAI 协议**（`wire.rs:1121-1122`）→ `adapter/codex.rs:260` | Responses 工具透传；Chat 请求带 `tools` 明确拒绝（`codex.rs:192-200`） |
| C4 | 账号指纹 | 同 C2 | — | `fetch_models`（Bearer） | API 分支；Responses→Chat / Anthropic→Chat 转换（`wire.rs:1019-1020`） | 透传 / 转换保留 custom tools |
| C5 | 浏览器授权（官方 Gemini CLI OAuth，BL:28）；本机登录 `gemini_quota` | `LocalLogin` → `gemini_quota::local_gemini_quota`（`quota_registry.rs:67`） | — | `gemini_subscription::models`（`:143`） | Chat 直译，Anthropic / Responses 先转 Chat（`wire.rs:1146-1171`）→ `gemini_subscription.rs:329` `forward_chat`；原生 `/v1beta` 走 `adapter/gemini.rs:28` | function 工具 ↔ `functionDeclarations`（`gemini_chat.rs:161-176`，回复 `:280-283`） |
| C6 | 账号指纹；地址只允许官方主机（`provider_key.rs:46`） | 目录声明「API Key 不提供额度与余额」（`platformCatalog.json` gemini `limitation`）；代码走 `ApiBaseUrl` | — | `fetch_models`（官方 OpenAI 兼容 `/models`，`providers.rs:148`） | API 分支 | 透传 / 转换 |
| C7 | 设备授权 `grok_browser_auth`，`Credential::Grok`（`provider_browser_auth.rs:31`） | `LocalLogin` → `grok_quota::local_grok_quota`（`quota_registry.rs:66`） | — | `adapter/grok.rs:110` `models` | Chat / Responses 转发到官方 `cli-chat-proxy`（`grok.rs:17`、`:134-170`） | 透传，无专门转换 |
| C8 | 账号指纹；地址只允许官方主机 | 目录：「API 预付余额需独立管理授权」；代码走 `ApiBaseUrl` | — | `fetch_models` | API 分支 | 透传 / 转换 |
| C9 | `login_detect.rs:24`（OpenCode 里已登录 OpenAI） | `LocalLogin` → `opencode::local_openai_quota`（`quota_registry.rs:68`） | — | — 无分支，固定清单 | 复用 Codex 适配器，只收 OpenAI 协议（`wire.rs:1121,1135`） | 同 C3 |
| C10 | 账号指纹 | `ApiBaseUrl`：`opencode.ai/zen/go` 命中编程套餐 `OpencodeGo`（`coding_plan.rs:54`） | — | `fetch_models` | API 分支 | 透传 / 转换 |
| C11 | `zcode_quota.rs:650` `account_identity`（家族 + 令牌种类）；浏览器设备流 `Credential::Zcode` | `ZcodeLocal` / 账号来源 `NativeSource`（`quota_registry.rs:43-44`） | ⚠️ 礼包领取 `checkin_zcode.rs`（preview / claim），用的是 Start Plan 会话 JWT；?（待确认：BigModel 账号能否领 zcode.z.ai 礼包，代码与记录都未说明） | `adapter/zcode.rs:401` `models`；两条链都列不出时用礼包模型补位（ZC:9） | 双通道失败转移（`zcode.rs:595-650`），Coding Plan 通道 `coding_plan_forward` | Coding Plan 透传 |
| C12 | 同 C11（`zai` 家族） | 同 C11 | 同 C11 | 同 C11 | 同 C11 | 同 C11 |
| C13 | 会话 JWT `zcode_start.rs:35` `jwt_at` | 账号余额 `balances[]`（ZB） | 礼包 `checkin_zcode.rs:85`（status）/ `:151`（claim） | 礼包模型 `zcode_start.rs:434,487` | 三种调用方协议先转 Anthropic，按 ZCode 客户端样式「着装」后发 `zcode.z.ai`（`zcode.rs:624-650`、`zcode_start.rs:270`） | 转 Anthropic 后发送；`Skill` 工具按 ZCode 样式处理（`zcode_start.rs:112,170`） |
| C14 | 账号指纹 | `ApiBaseUrl` → `coding_plan::query_zhipu`（`ZhipuCn`，`coding_plan.rs:49,63`） | — | `fetch_models`（协议随所选地址，`providers.rs:153`） | API 分支 | 透传 / 转换 |
| C15 | 账号指纹 | `ApiBaseUrl` → `query_zhipu`（`ZhipuEn`，`coding_plan.rs:50,63`） | — | 同 C14 | API 分支 | 透传 / 转换 |
| C16 | 本机桌面登录 `native_accounts_for`（`login_detect.rs:168-200`）；浏览器授权 `Credential::Qoder` + `qoder_desktop::verify_session` | `QoderSource`（`quota_registry.rs:42`）；国际版仅无桌面文件时读 CLI（`platformCatalog.json` qoder `limitation`） | 每日活动 `checkin_qoder.rs`，国际域名 `openapi.qoder.sh`（`:203-209`） | `adapter/qoder.rs:425` `models`（富缓存，`providers.rs:130`） | Anthropic / Responses 先转 Chat（`wire.rs:1172-1201`）→ `qoder.rs:650` | 原生工具调用 + 文本工具解析（`qoder.rs:715-718`，`qoder_chat.rs:3,647`）；`tool_choice=none` 时不带工具 |
| C17 | 同 C16（`cn`） | 同 C16 | 同 C16，国内域名 `openapi.qoder.com.cn` | 同 C16 | 同 C16 | 同 C16 |
| C18 | `native_accounts_for`（`native:trae:ide`）；`Credential::Trae` + `trae_browser_auth::verify_session` | `NativeSource`（`quota_registry.rs:41`） | — 官方未开放，`checkin_trae.rs:219-228` 明确返回不支持 | `trae_models::models`（`:376`，富缓存） | 同 C16 → `adapter/trae.rs:76`（`wire.rs:1191`） | 工具校验（`trae_chat.rs:70-94`，拒绝 strict / 非对象 schema）+ GLM / DSML 文本工具解析（`trae_chat.rs:381-415`） |
| C19 | 同 C18（`native:trae:cn`） | 同 C18 | ✅ `checkin_trae.rs`（需官方 AHA 设备，TK:57） | 同 C18 | 同 C18 | 同 C18 |
| C20 | `native:workbuddy:main`；`Credential::Workbuddy` + `workbuddy_browser_auth::verify_session` | `NativeSource` | ✅ `checkin_workbuddy.rs`（`require_cn`，`:209-217`） | `adapter/workbuddy.rs:235` `models`（`/v3/config`，WM） | 同 C16 → `workbuddy.rs:368`，**只收 Chat**（`:384`）；请求体按渠道规则规范化（`workbuddy_request.rs`） | 透传；规范化时保留工具定义与参数（`workbuddy_request.rs:3`） |
| C21 | `native:workbuddy:ai`；同 C20 | 同 C20 | — 国际版为自动活跃奖励，`checkin_workbuddy.rs:212-215` 返回不支持 | 同 C20（国际主机 `www.workbuddy.ai`，WM:39） | 同 C20 | 同 C20 |
| C22 | 账号指纹 | `ApiBaseUrl`：编程套餐域名（Kimi / MiniMax / ZenMux / 火山等，`coding_plan.rs:47-56`）或 sub2api；余额 `service_balance.rs`（官方主机精确识别） | — | `fetch_models`；接口不可用时可手填并 `verify_custom_model`（`providers.rs:162`） | API 分支，按所选协议转换 | 透传 / 转换；Anthropic 调用方 + 非 Claude 上游走 `tool_loop` |

### 2.3 实现矩阵 B：接线、用量、会话、续跑、Skills

- **接线（启用到原 CLI）**：只有 Claude、Codex 有。`cli_apply.rs:25-41` `apply_to_cli`：C2 `apply_claude_api`、C4 `apply_codex_api`、C3 `apply_codex_auth`、C1 经 `apply_claude_verified`（`cli_apply.rs:99`，由 `commands/providers.rs:212` 调用）；其余平台 `cli_apply.rs:40` 明确返回「暂不支持」。Agent 侧一键接线见 §1。
- **用量（转发归账）**：API 分支由 `direct` 适配器记账；订阅分支中 grok / workbuddy / zcode 在流结束时读用量入库（`wire.rs:1204-1210` → `gateway/subscription.rs` `record_usage_on_end`），其余订阅适配器自行归账。本机日志采集属于 Agent 维度（§1）。
- **会话 / Skills**：挂在平台（§1）。没有 Agent 行的平台：Grok 无本机会话（`platformCatalog.json` 无 `local_sessions`）、Skills 未接入；Trae 无会话（`native_sources.rs:50-51` 的两个来源只用于登录校验，见连接能力矩阵 §4.2），Skills 可写（`skills.rs:133`，`.trae/skills` 与 `.trae-cn/skills`）；自定义供应商两者都没有（`skills.rs:136` `custom` 不可写、无目录）。
- **续跑（按连接额度等待）**：只对 Claude / Codex 连接（`automation.rs:50-56`）。订阅连接读额度后等待到重置再续（`automation.rs:834-840`）；读不到额度的（如 API Key）到预计时间直接尝试继续（`:864-876`）。Claude 订阅在网关内另有「同一调用方会话续用同一个 claude 会话」（`adapter/claude.rs:130` `resume_for`，空闲 10 分钟失效）。

| # | 接线 | 用量（转发归账） | 会话 | 续跑 | Skills |
|---|---|---|---|---|---|
| C1 | ✅ `apply_claude_verified` | ✅ 适配器归账（SA:12） | ↗ claude-code | ✅ 等额度续跑；网关内 `resume_for` | ↗ |
| C2 | ✅ `apply_claude_api` | ✅ `direct` | ↗ | ✅ 读不到额度时按时间续 | ↗ |
| C3 | ✅ `apply_codex_auth` | ✅ 适配器归账 | ↗ codex | ✅ 等额度续跑 | ↗ |
| C4 | ✅ `apply_codex_api` | ✅ `direct` | ↗ | ✅ 按时间续 | ↗ |
| C5 / C6 | — | ✅ | ↗ gemini | — | ↗ |
| C7 / C8 | — | ✅（C7 `record_usage_on_end`） | — | — | — |
| C9 / C10 | — | ✅ | ↗ opencode | — | ↗ |
| C11–C15 | — | ✅（订阅 `record_usage_on_end`） | ↗ zcode | — | ↗ |
| C16 / C17 | — | ✅ | ↗ qoder | — | ↗ |
| C18 / C19 | — | ✅ | — | — | ✅ `skills.rs:133` |
| C20 / C21 | — | ✅（`record_usage_on_end`） | ↗ workbuddy | — | ↗ |
| C22 | — | ✅ `direct` | — | — | — |

### 2.4 连接验证矩阵（22 × 11）

| # | 账号识别 | 额度 | 签到 | 模型发现 | 路由 | 接线 | 用量 | 会话 | 续跑 | Skills | 工具调用 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| C1 Claude 订阅 | 无记录 [AB:109：`~/.claude/.credentials.json` 不存在，0 次请求] | 无记录 [同左] | 不适用 | 不适用（固定清单） | 模拟 [SA:43,49，假 Claude CLI] | 无记录 | 模拟 [SA:12,49] | ↗ | 无记录 [AR:19] | ↗ | 无记录（工具桥） |
| C2 Claude API | 只读 [CL:23，状态 connected] | 只读 [CL:22；PF:34，第三方中转余额] | 不适用 | 失败 [MF:94 与 §6：上游 Key 被停用，`/v1/models` 403；错误透传正确，非 Jolu 缺陷] | 模拟 [GD:22，共用 api 分支] | 无记录 | 无记录 | ↗ | 无记录 [AR:19] | ↗ | 无记录 |
| C3 Codex 订阅 | 部分 [AR:5 本机 `current: true`；浏览器登录只有合成] | 只读 [AB:45，wham/usage] | 不适用 | 不适用（固定清单） | 模拟 [SA，本地模拟 Codex] | 无记录（「设置为使用」真实切换未测） | 模拟 [SA] | ↗ | 无记录 [AR:19] | ↗ | 无记录（有单测） |
| C4 Codex API | 无记录 | 无记录 | 不适用 | 无记录 | 模拟 [GD:23，共用 api 分支] | 无记录 | 无记录 | ↗ | 无记录 | ↗ | 无记录 |
| C5 Gemini 登录 | 失败 [AQ:24，`unauthorized`：本机无可用登录，非实现缺陷] | 失败 [AQ:24，同左] | 不适用 | 无记录 | 模拟 [SA，本地模拟 Code Assist] | 不适用 | 模拟 [SA] | ↗ | 不适用 | ↗ | 无记录 |
| C6 Gemini API | 无记录 | 不适用（目录声明不提供） | 不适用 | 无记录 | 模拟 [GD:25，原生 `/v1beta` + 合成上游] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C7 Grok 授权 | 部分 [BL:41，仅设备授权初始化真实成功，完整登录未做] | 无记录 [AQ:36] | 不适用 | 无记录 | 模拟 [SA，本地模拟 token / userinfo] | 不适用 | 无记录 | 不适用 | 不适用 | 不适用 | 无记录 |
| C8 Grok API | 无记录 | 不适用（目录：需独立管理授权） | 不适用 | 无记录 | 模拟 [GD，共用 api 分支] | 不适用 | 无记录 | 不适用 | 不适用 | 不适用 | 无记录 |
| C9 OpenCode OpenAI 订阅 | 无记录 [RV:20，0 个凭证] | 无记录 | 不适用 | 不适用（固定清单） | 模拟 [SA，复用 Codex 链] | 不适用 | 模拟 [SA] | ↗ | 不适用 | ↗ | 无记录 |
| C10 OpenCode API | 无记录 | 无记录 | 不适用 | 无记录 | 模拟 [GD，共用 api 分支] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C11 ZCode BigModel 账号 | 只读 [AQ:27；BL:41 设备流初始化 HTTP 200] | 部分 [AQ:27 真实返回「未开通」；付费个人 / 团队缺样本] | 无记录 | 无记录 | 模拟 [ZC:22] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C12 ZCode Z.ai 账号 | 只读 [ZV:76-84，凭据全部解开] | 部分 [ZV:76,87，WEEKEND BUILD、0 个窗口] | 无记录 | 无记录 | 模拟 [ZC:22；ZV:13 只读到转发凭据] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C13 ZCode Start Plan | 只读 [ZV:83，JWT 可读] | 只读 [ZB:59-68，2 个日额度窗口] | 无记录（`checkin_zcode.rs` 无验收） | 部分 [ZC:26，真实返回「没有生效套餐」，有套餐样本缺] | 模拟 [ZC:22] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C14 ZCode 国内 API Key | 无记录 [UI:21，Key 未填写] | 无记录 | 不适用 | 无记录 | 模拟 [GD，共用 api 分支] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C15 ZCode 国际 API Key | 无记录 | 失败 [ZZ:23,38：本机 Z.ai Coding Plan Key 打 `api.z.ai` 额度端点返回业务码 1000「身份验证失败」；用户自填 Key 未测] | 不适用 | 无记录 | 模拟 [GD，共用 api 分支] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 无记录 |
| C16 Qoder 国际 | 只读 [CD:34] | 失败 [AQ:25，CLI 来源 `unauthorized`；CD:34 身份通过后未再记录额度结果] | 只读 [CD:35，`Inactive`；DC:21] | 无记录 | 模拟 [QA:59] | 不适用 | 无记录 [见注 1] | ↗ | 不适用 | ↗ | 无记录（有单测） |
| C17 Qoder CN | 只读 [BA:19；CD:34] | 只读 [BA:19，3 个窗口] | 只读 [DC:20，已领 100，未发 POST] | 只读 [QM:49，12 条] | 模拟 [QA:59] | 不适用 | 无记录 [见注 1] | ↗ | 不适用 | ↗ | 无记录（有单测） |
| C18 Trae 国际 | 只读 [CD:36；TK:102] | 只读 [AQ:26；CD:36] | 不适用 [TK:3，官方区域门禁] | 模拟 [TA，fixture] | 模拟 [TA「本轮验证与审查」] | 不适用 | 模拟 [TA「转换与失败语义」归账] | 不适用 | 不适用 | 模拟 [UI] | 模拟 [TA「转换与失败语义」] |
| C19 Trae CN | 部分 [CD:47 网页授权→保存真实通过；CD:59 第二个账号缺] | 只读 [AQ:26；TS:23,48] | 端到端 [TK:112，Ready→Claimed] | 模拟 [TA] | 模拟 [TA] | 不适用 | 模拟 [TA] | 不适用 | 不适用 | 模拟 [UI] | 模拟 [TA] |
| C20 WorkBuddy 国内 | 只读 [CD:34] | 只读 [BA:17；首轮 AQ:23 的 401 在重新登录后为 200] | 端到端 [WK:51-58，Ready→Claimed/100] | 部分 [WM 补充段：真实账号首轮 HTTP 400 后按 magpie 修复；MF:96 真机显示 17 个模型，但未区分实时目录与内置兜底清单] | 部分 [WC:24-26，hy3 真实 200，窄场景] | 不适用 | 无记录 [见注 2] | ↗ | 不适用 | ↗ | 模拟 [WC:23，工具参数保留] |
| C21 WorkBuddy AI 国际 | 只读 [CD:34] | 无记录（无任何国际版额度结果） | 不适用 [DC:19；CD:35 真实返回 `Unsupported`] | 无记录 [WM:23，未做国际真实验收] | 模拟 [WM / WC，共用代码合成] | 不适用 | 无记录 | ↗ | 不适用 | ↗ | 模拟 [WC:23] |
| C22 自定义 API Key | 无记录 | 模拟 [PA:22-23，真实组件 + 合成桥接、隔离桌面] | 不适用 | 无记录 | 模拟 [GD，共用 api 分支] | 不适用 | 无记录 | 不适用 | 不适用 | 不适用 | 无记录 |

注 1：P3 §3 观察到正式库里有 2 行 Qoder `gateway-subscription` 记录（2026-11-14，`browser:` 连接，版本未记录），说明真实转发发生过，但没有对应的验收结论，不定档。
注 2：P3 §4.1 观察到正式库里有 10 行 WorkBuddy `gateway-subscription` 记录（版本未记录），能证明 Jolu 转发过、记过账，但没有与上游账单核对，不定档。

---

## 3. 档位统计与各能力结论

### 3.1 7 档计数（`↗` / `↘` 不计）

| 档 | Agent 表（§1.5） | 连接表（§2.4） | 合计 |
|---|---:|---:|---:|
| 真实端到端通过 | 1 | 2 | 3 |
| 真实只读通过 | 6 | 19 | 25 |
| 部分通过 | 18 | 8 | 26 |
| 模拟通过 | 28 | 36 | 64 |
| 无验证记录 | 39 | 74 | 113 |
| 不适用 | 39 | 64 | 103 |
| 失败 | 0 | 5 | 5 |
| 计档格数 | 131 | 208 | 339 |
| `↗` / `↘`（不计） | 12 | 34 | 46 |

计数方法：逐格取首个档位词，脚本逐行解析 §1.5 与 §2.4；两表共 13 × 11 + 22 × 11 = 385 格。三格真实端到端是 Kimi 接线、WorkBuddy 国内签到、Trae CN 签到；五格失败是 Claude API 模型拉取、Gemini 登录的账号识别与额度、ZCode 国际 API Key 额度、Qoder 国际额度。

### 3.2 每项能力一句话

| 能力 | 结论 |
|---|---|
| **账号识别** | 连接侧 22 种都有实现；真实只读覆盖 Qoder 双版、WorkBuddy 双版、Trae 国际、ZCode 账号与 Start Plan、Claude API，Codex 订阅与 Trae CN 部分通过，Gemini 登录失败（无登录），Claude 订阅与全部其他 API Key 无记录；Agent 侧只有 4 家 CLI 有真实 CLI 记录（合成上游），pi / pencil / qoder / qoder-cn 连 UA 都没登记。 |
| **额度** | 订阅额度的真实只读覆盖 Codex、Qoder CN、Trae 双版、WorkBuddy 国内、ZCode Start Plan，ZCode 两个账号家族只拿到「未开通 / 无窗口」；Gemini 登录、Qoder 国际、ZCode 国际 API Key 为真实失败；Claude 订阅、Grok、OpenCode、WorkBuddy AI 国际无记录。 |
| **签到** | 只有 WorkBuddy 国内与 Trae CN 真实领取过，Qoder 双版真实只读；WorkBuddy AI、Trae 国际经真实确认为不支持；ZCode 礼包领取有实现、无任何验收。 |
| **模型发现** | 实时拉取只有 Qoder CN 真实只读通过；WorkBuddy 国内、ZCode Start Plan 部分通过；Claude API 唯一一次真实拉取因上游 Key 停用失败；其余订阅只有模拟或无记录，Agent 侧只有 Kimi / dsh 的目录写入有真机记录。 |
| **路由** | 没有任何一种连接有完整的真实转发通过；WorkBuddy 国内 hy3 真实 200 记为部分，其余订阅全部是模拟（假 CLI / 回环上游）；Agent 侧 Claude Code / Codex / OpenCode / Gemini 的识别与选路在真实 CLI + 合成上游下部分通过。 |
| **接线** | 13 家都有接线实现；只有 Kimi 做过真机接入 / 断开往返，dsh 用副本做过，其余靠生产路径往返测试（模拟）；连接侧「启用到原 CLI」只有 Claude / Codex 有，均无真实写入记录。 |
| **用量** | 本机采集真实只读对账通过 Claude Code、OpenCode、Qoder、ZCode，Codex 只有只读统计；Kimi / dsh / pi / qwen 未实现入库，WorkBuddy 缺客户端样本；转发归账除 Trae / Qoder 外只有模拟或无记录。 |
| **会话** | Claude Code、Codex 真实只读，OpenCode、ZCode 部分通过；Gemini、Kimi、dsh、pi、Qoder、WorkBuddy 的会话读取无记录，qoder-cn 缺入口。 |
| **续跑** | 只对 Claude Code / Codex 实现；自动化「续接原会话」真实通过，「额度耗尽 → 等待 → 续跑」与会话页「在原工具继续」都无真实记录；其余 11 家未实现。 |
| **Skills** | 8 个平台有实现；Claude / Codex 只在隔离目录做过真实文件读写（部分），其余只有合成测试，原工具真实目录的发现与热加载全平台都没核对。 |
| **工具调用** | 五种能力（MCP 搜索、tool_loop、Claude 工具桥、订阅工具转换、文本工具解析）里没有一种有真实上游的记录；最接近的是四个真实 CLI + 假上游的两轮工具调用（模拟）。 |

---

## 4. 未验证清单（剩余真正缺的组合）

只列「实现存在、但缺真实样本或真实条件」的组合；未实现的能力不在此表（见 §5.3）。

| 组合 | 当前档 | 缺什么 | 出处 |
|---|---|---|---|
| Claude 订阅：账号识别、额度、路由、工具桥 | 无记录 / 模拟 | 完整订阅 OAuth 登录样本；真实账号经网关生成一次（含流式与工具） | AB:109；SA:49 |
| Codex 订阅：路由、启用到原 CLI | 模拟 / 无记录 | 真实账号经网关的 Chat / Responses 请求；「设置为使用」真实切换 | SA；AR |
| Gemini 登录：账号识别、额度、路由 | 失败 / 模拟 | 有效 Google 登录（含 Code Assist 初始化） | AQ:24 |
| Grok 授权：完整登录、额度、路由 | 部分 / 无记录 / 模拟 | 完整设备授权 → 保存 → 真实查询与生成 | BL:41；AQ:36 |
| OpenCode：OpenAI 订阅与 OpenCode Go 的登录、额度、路由 | 无记录 / 模拟 | 本机 OpenCode 凭证（当前 0 个） | RV:20 |
| ZCode：付费个人 / 团队 / Start Plan 有套餐时的额度与礼包模型 | 部分 | 付费或有生效套餐的账号 | UI:14；ZC:30 |
| ZCode：Coding Plan / Start Plan 双通道真实转发 | 模拟 | 有额度时的真实转发 | ZV |
| ZCode：礼包领取 | 无记录 | 有可领礼包的账号，执行一次真实 claim | `checkin_zcode.rs` |
| ZCode API Key（国内 / 国际）：账号识别、额度、模型发现 | 无记录 / 失败 | 用户自填的有效 Key | UI:21；ZZ |
| Qoder 国际：额度、模型发现 | 失败 / 无记录 | 国际版登录后的真实额度与模型列表 | AQ:25；CD:34 |
| Qoder 双版：真实对话（流式、非流式、工具） | 模拟 | 真实账号对话 | QA:42,59 |
| Trae 双版：模型目录、对话、工具模板 | 模拟 | 真实账号的目录与对话（流式、工具、GLM / DSML 文本工具） | TA |
| Trae CN：第二个账号 | 部分 | 第二个国内真实账号登录 | CD:59 |
| WorkBuddy 国内：模型目录、其他模型、完整工具任务、非流式 | 部分 / 模拟 | 重启后在界面复核目录；完整 Claude Code 工具任务 | WM；WC:28 |
| WorkBuddy AI 国际：额度、模型发现、路由 | 无记录 / 模拟 | 国际版真实账号的额度、目录与生成 | WM:23 |
| WorkBuddy：本机用量与会话 | 无记录 | 装有客户端的机器 | P3 §4 |
| 自定义 / Claude API / Codex API：模型拉取与真实转发 | 失败 / 无记录 / 模拟 | 一把可用的第三方 Key | MF §6 |
| 各 API Key 连接：账号识别与额度 | 无记录 | 对应平台的有效 Key | — |
| 自动化：额度耗尽 → 等待 → 续跑 | 无记录 | 真实耗尽额度的 Claude Code / Codex 账号 | AR:19,98 |
| 会话页「在原工具继续」 | 无记录 | 一次真实点击继续（Claude / Codex 各一） | `session_threads.rs:1275`，无验收记录 |
| Skills：原工具真实目录的发现与热加载 | 部分 / 模拟 | 在 `~/.claude/skills`、`~/.codex`、`.codebuddy/skills` 等真实目录核对 | DA；UI |
| 工具调用：模型发起 → 执行 → 回传 → 最终回答 | 模拟 / 无记录 | 任一真实上游跑通一次完整链路（含流式与失败） | CO:22 |
| MCP 搜索、tool_loop、Claude 工具桥 | 无记录 | 在真实 Agent 中各触发一次 | §1.3 |
| Agent 侧路由：ZCode / Kimi / dsh / pi / Pencil / Qoder / qwen / WorkBuddy | 无记录 | 各工具真实经网关发一次请求 | §1.5 |
| 接线：除 Kimi 外 12 家在原配置上的真实往返 | 部分 / 模拟 | 在真实工具配置上接入 / 断开并核对还原 | J |
| OpenCode 用量：SQLite V2 新表 | 部分 | V2 新表有数据的样本 | RV |
| Qoder：`local.db` 布局跨机器稳定性；旧 projects 日志去重键 | 只读（单机） | 第二台装 Qoder 的机器；旧 projects 日志样本 | P3 §3.2c；`2026-11-23-p0-p1-review.md` P-9 |

环境限制（多份记录记为阻塞）：**Windows Sandbox 不存在**、**goose 未安装**。

---

## 5. 结论

### 5.1 与 P0-1 要求的差距（剩余）

- **结构已齐**：11 项能力 × 7 档，Agent（13 家）与连接（22 种，按平台、地区、连接类型拆开）两个维度都已列出，每格有代码出处与验证依据。
- **剩下的都是样本缺口**，即 §4 的未验证清单。按计划 §3，这些记为「未验证」，不写成通过。
- **两处没核实的实现细节**标了「?」：OpenCode 的 `openai-compatible` provider 是否自动请求 `/models`；ZCode BigModel 账号能否领取 `zcode.z.ai` 礼包。kimi / workbuddy 接线后的请求协议本次也没核实。

### 5.2 实现完成度

13 家 Agent 全部可探测、全部可接线。**用量采集覆盖 8 家**（claude / codex / gemini / opencode / zcode / qoder / qoder-cn / workbuddy），**会话读取覆盖 11 家**（gemini / opencode / qoder / workbuddy 正文不可读），**Skills 覆盖 8 个平台**（claude / codex / gemini / opencode / qoder / trae / zcode / workbuddy，其中 workbuddy 只读），**签到覆盖 4 个平台**（WorkBuddy 国内、Qoder 双版、Trae 国内、ZCode 礼包），**续跑只覆盖 Claude Code / Codex**，**MCP 搜索只装进 4 家**。连接侧 22 种全部有账号识别与转发路径；实时模型拉取覆盖 6 个订阅平台和全部 API Key 连接；额度覆盖除 Gemini / Grok API Key 外的全部连接。

### 5.3 已知未实现（不是样本问题）

- 续跑：gemini、opencode、zcode、kimi、dsh、pi、qoder、qoder-cn、qwen、workbuddy。
- 用量入库：kimi、dsh、pi、qwen（§6.8）。
- 会话 / Skills：qoder-cn 无独立入口（§6.5）；qwen、WorkBuddy 会话前端标「还没接入」（§6.2）。
- MCP 搜索：zcode 及接线清单里其余 8 家。
- UA 识别：pi、pencil、qoder、qoder-cn 没有登记 UA。
- 版本探测：只有 5 家（§6.7）。

### 5.4 最大的三个缺口

1. **工具调用整条链路没有真实证据**：五种工具能力都没有真实上游的记录，最接近的是真实 CLI + 假上游。
2. **订阅转发真实账号生成几乎为零**：只有 WorkBuddy 国内一次窄场景 200；Claude / Codex / Gemini / Grok / OpenCode / ZCode / Qoder / Trae 都没有。
3. **Skills 全平台只在隔离目录验过**：原工具真实目录的发现与热加载没核对。

### 5.5 记录之间互相矛盾的项

`2026-11-17-capability-verification.md:23-40` 列出 7 组矛盾，其中与本次判定直接相关的三组：

- WorkBuddy 额度：401 vs ok。本文按 BA:17 与 UI:11 取较新的真实结果（重新登录后 ok），并更正了旧版本文把 401 归到「WorkBuddy AI 国际」的错误（§6.11）。
- ZCode 额度：四份记录四种结果。本文按账号家族与通道拆成 C11–C13 分别定档。
- Trae 国内签到 / 网页授权：「不声称完成」vs「缺口已解除」。本文按 CD:47 的较新结果定档。

---

## 6. 声明与实现的不一致

2026-11-24 按当前代码逐条复核。**已修复：6.1、6.9。仍存在：6.2–6.8。** 6.11–6.13 是本次复核新发现的。

### 6.1 dsh 平台 id 不一致（已修复 2026-11-23）

- `agentPage.ts` 曾把 `dsh` 映射成 `"deepseek"`（那是**图标表**的 id），而后端 `dsh_sessions.rs:259` 产出 `platform="dsh"`
- 后果：dsh 详情页的**会话数与技能数恒为 0**（`AgentsView.tsx:192-197` 拿这个值去查 `skillsApi.overview` 与 `sessionsList`）
- 已改为 `"dsh"`（当前 `agentPage.ts:22-25`），并加测试 `详情页的平台 id 与后端产出的 platform 一致` 锁住

### 6.2 WorkBuddy 会话能力：前端说未接，后端已实现（仍存在）

- `agentPage.ts:262` 仍把 workbuddy 标为 `not-yet`（「会话记录格式还没接入」）
- 但 `native_sources.rs:66-77` 有 `.workbuddy/projects` / `.workbuddy-ai/projects` 来源，`native_parse.rs:49` 有 `workbuddy` 解析器
- 本机没有客户端（§6.10），后端链路无法真机确认；确认后应改为 `observable`

### 6.3 Qoder / WorkBuddy 会话验收表与代码矛盾（仍存在，且 Qoder 一格已过时）

- `capabilityVerification.ts:114` 写 qoder「会话与用量」为 `none`，`:122` 写 workbuddy 为 `none`
- Qoder：P3 §3.2c 已在正式 `usage.db` 完成 999/999 逐条对账，「用量」这半格已有真实只读记录，验收表应更新（会话列表仍无记录）
- WorkBuddy：`none` 仍然符合事实（缺客户端样本）

### 6.4 ZCode 签到：验收表写 `na`，代码有实现（仍存在）

- `capabilityVerification.ts:110` 仍写 `checkin: na / 签到范围不含 ZCode`
- 但 `checkin_zcode.rs` 完整存在（status:85、claim:151），`daily_checkin.rs:145` 也把 `"zcode"` 列入可签到平台
- 差异在于它是**礼包领取**而非每日签到，验收表未区分这两种语义；应改为「无记录」而非「不适用」

### 6.5 qoder-cn 有接线但没有可用的会话 / Skills 查询入口（仍存在）

- `agentPage.ts:29` 映射成平台 id `"qoder-cn"`
- 后端会话数据全部挂在 `platform="qoder"`（`native_sources.rs:60-65` 的 `cn` 实例）；Skills 的 `qoder` 规格已包含 `.qoder-cn/skills`（`skills.rs:132`），但按 `"qoder-cn"` 查询时 `spec()` 找不到，`skills.rs:1133` / `:1936` 返回「暂未接入 Skills 管理」
- 后果：接通 qoder-cn 后，详情页的会话数与技能数取不到，只有签到能用

### 6.6 dsh 探测的文件证据与接线主文件对不上（仍存在）

- `agent_discovery.rs:345` 的 `file: "config.yaml"` 仍是旧版结构
- 新版 dsh 主文件是 `profiles/<name>/cordis.patch.yml`
- 测试 `batch1_pencil_dsh_copilot_vscode_found_by_config_dirs`（`agent_discovery.rs:771`）说明目录证据（`profiles/default`）能判装；但文件证据在新装环境下永不命中，没有 CLI 时只能靠目录证据，状态可能落到 `cli_missing`

### 6.7 版本探测只覆盖 5 家（仍存在）

- `agent_version.rs:28-34` 的 `AGENTS` 仍只有 claude / codex / gemini / opencode / zcode
- 可接线清单是 13 家
- 后果：kimi / dsh / pi / pencil / qoder / qoder-cn / qwen / workbuddy 八家的卡片**拿不到版本号**

### 6.8 Kimi / Pi / dsh 的用量「可读但不入库」（仍存在）

- 三个读取器都会算 requests / Tokens 并返回 `SessionItem`
- 但 `kimi_sessions.rs`、`pi_sessions.rs`、`dsh_sessions.rs` 里**仍无任何 DB 写入**（无 `INSERT`、无 `source_store::write`；对比 `native_sources.rs:524-577`）
- 后果：会话列表有数字，但**用量统计、成本、归因里没有它们的任何记录**

### 6.9 Qoder 采集器读的路径在本机不存在（2026-11-23 发现并修复，正式库已复验）

**问题**：
- 采集器读 `~/.qoder/projects`（`native_sources.rs:58`）与 `%APPDATA%\com.qoder.app.stable\main.sqlite`（`:903`）
- **本机两者都不存在**：`~/.qoder` 下只有 `session-env` / `shared_client` / `skills`
- 真实账本在 `~/.qoder/shared_client/cache/db/local.db` 的 `chat_message` 表：**999 条带 Token 的 assistant 调用**（去重 `request_id` 315，实为 315 轮 agent 对话）、合计 prompt 46.8M / completion 0.62M / cached 38.7M
- Jolu 的 `requests WHERE platform='qoder'` 只有 2 行，全部来自自己的网关转发（`source='gateway-subscription'`）
- **按 ID 逐条对账：0 交集**

**修复（2026-11-23）**：新增 `backend/src/qoder_usage.rs`
- 读 `chat_message` 的 `session_id` / `id` / `gmt_create` / `token_info`，其余列（`content` / `answer` / `question` / `reasoning_content`）一律不碰
- `dedup_key = {session_id}:{message_id}`。**不是 `request_id`**：它是「一轮对话」，一轮里实测最多 57 次调用，按它合并会丢掉 56/57 的用量
- 入库存原始 `prompt_tokens`，以 `input_semantics = "includes_cache"` 标明含缓存，由消费方扣减（`cached` ⊂ `prompt`，999/999 成立）
- `model_key`（ultimate / auto / efficient…）是档位不是模型名，不写 `model`
- 来源可用性：`global` 实例「`projects` 目录 或 账本」任一存在即可用；`cn` 实例只认自己的目录（共用主账本会让同一批记录被两个 source 各写一遍）

**验证**：
- 单测 6 条（缓存扣减 / `request_id` 不是去重键 / 非 assistant 与空 token 跳过 / 档位不冒充模型名 / 时间过滤 / 缺表报错）
- `qoder_source_is_available_from_the_ledger_alone`：只有账本没有 projects 目录时也须可用并采到数据（这条**抓到两个实例重复写同一批记录的 bug**：首跑 4 行而非 2 行）
- `real_qoder_ledger_scans_into_a_temp_db`（`#[ignore]`，真机只读）：真实账本 → 临时库，**999 条**，与独立 Python 逐项一致，零重复键
- **正式库**：P3 §3.2c 已在正式 `usage.db` 完成修复后扫描，与源端 999/999 逐条匹配、只在源端 0

**当前验证边界**：`local.db` 的布局只在本机一份样本上确认，跨机器稳定性待第二个环境；旧 projects 日志与新账本的去重键不一致，需要旧日志样本才能定迁移（`2026-11-23-p0-p1-review.md` P-9）。

详见 [P0-3 多平台对账 §3](../verification/2026-11-23-p0-3-multi-platform-reconciliation.md)（字段语义见 §3.3）。

### 6.10 WorkBuddy 本机无客户端（仍为样本缺口）

- `~/.workbuddy`、`~/.workbuddy-ai`、`~/.codebuddy` 均不存在，`%APPDATA%` / `%LOCALAPPDATA%` 下也无可疑目录
- Jolu 的 10 行 workbuddy 记录全部来自网关转发
- 无法做源端对账，标「缺样本」而非「不支持」

### 6.11 本文旧版把 WorkBuddy 的 401 记到了「AI 国际」头上（本次新发现，已在本文更正）

- 旧版 §3 写「WorkBuddy AI 国际：首轮真实返回 401」，依据 AQ:23
- AQ:23 记录的是本机 WorkBuddy 5.8.3 官方客户端的额度请求，没写是国际版；UI:11 写明「首轮 WorkBuddy 401 … 已由上行续轮处理」，BA:17 记录重新登录后 HTTP 200、`state=ok windows=1`，`capabilityVerification.ts` 也把它判为国内版
- WorkBuddy AI 国际版的事实是：身份真实只读通过（CD:34），签到真实确认不支持（CD:35），**额度没有任何真实结果**
- 同样的错误还在 [连接能力矩阵](2026-11-23-connection-capability-matrix.md) 的 §0 与 §1 workbuddy 行（「AI 国际首轮 401」），本次按约束只改本文，那份需另行更正

### 6.12 连接能力矩阵把 WorkBuddy 模型发现记成「真实」，证据不够（本次新发现）

- 连接能力矩阵 §3 写 `workbuddy CN 模型发现 ✅ 真实`
- WM 的补充段：真实账号首轮 `/v1/models` 报 HTTP 400，按 magpie 重写后「真实账号目录与对话需重启 Jolu 后由鼠鼠在界面上复核」，没有复核记录
- MF:96 的真机卡片显示「17 个模型」，但重写后的实现在目录读不到时会退到上次清单或 magpie 内置清单（WM 补充段最后一条），记录没有区分来源
- 本文记为「部分通过」

### 6.13 Trae 适配器记录与当前选路不一致（本次新发现，不是缺陷）

- TA「转换与失败语义」写「Responses 明确拒绝」
- 当前 `wire.rs:1172-1201` 会先把 Anthropic / Responses 调用方转成 Chat 再交给 Trae / Qoder / WorkBuddy 适配器，适配器入口仍只收 Chat
- 也就是说 Responses 调用方现在能经网关用 Trae，只是 TA 的验证不覆盖这条转换路径
