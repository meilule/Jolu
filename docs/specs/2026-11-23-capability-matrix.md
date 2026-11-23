# Jolu 能力矩阵与验证基线

日期：2026-11-23
状态：**P0-1 交付物。实现列以代码为准；验证列以 `docs/verification/` 的既有记录为准，两者分开标注。**
配套计划：[产品对齐执行计划](../plans/2026-11-23-product-alignment-execution-plan.md)

---

## 0. 怎么读这张表

这份文档回答两个**不同**的问题，必须分开看：

1. **实现了吗**：代码里有没有这条路径（有文件与行号）
2. **验证过吗**：有没有真实验收记录，还是只有合成测试，或者根本没有

**「实现」不等于「验证」**：13 家里有 8 家的实现是完整的，但真实账号验证只覆盖了一部分。
本文不把「代码存在」写成「通过」。

### 验证强度分档（沿用计划的五档）

| 档 | 含义 |
|---|---|
| **真实端到端** | 真实账号 / 真实工具 / 真实上游，含真实写操作 |
| **真实只读对账** | 只读取真实数据比对，无写操作 |
| **合成 / 模拟** | fixture、假上游、假 IPC、假 CLI |
| **无验证记录** | 代码在，但没有任何验收记录 |
| **不适用** | 该工具本身没有这个能力 |

---

## 1. 13 家 Agent 的实现矩阵

`✅` = 已实现（附入口）｜`—` = 明确没有｜`?` = 需确认

| Agent | 探测 | 接线 | 用量采集 | 会话读取 | Skills | 签到 | 订阅转发 |
|---|---|---|---|---|---|---|---|
| **claude-code** | ✅ `agent_discovery.rs:334` Standard，`bin=claude` | ✅ `wiring_specs.rs:203` → `adapters.rs:19/395`，1 文件，`LegacyLiteral` | ✅ `collector.rs:544`（`~/.claude/projects/*.jsonl`） | ✅ 正文可读 `session_threads.rs:95` | ✅ 可写 `skills.rs:105` | — | ✅ `adapter/claude.rs:207` |
| **codex** | ✅ `:335` Standard，`sign_in=auth.json` | ✅ `:218` → `adapters.rs:101/432`，1 文件，`Ignored` | ✅ `collector.rs:545`（`rollout-*.jsonl`） | ✅ 正文可读 `:97` | ✅ 可写 `:117` | — | ✅ `adapter/codex.rs:260` |
| **gemini** | ✅ `:337` **CliOnly** | ✅ `:237` → `adapters.rs:135/635`，**2 文件** | ✅ `native_sources.rs:42`（整份快照重建） | 🟡 列表有、**正文不可读** | ✅ 可写 `:130` | — | ✅ `adapter/gemini_subscription.rs:329` |
| **opencode** | ✅ `:338` Standard | ✅ `:252` → `adapters.rs:160/451`，1 文件 | ✅ `:40`（sqlite v1/v2 双分支） | 🟡 列表有、正文不可读 | ✅ 可写 `:131` | — | ✅ 复用 codex 适配器 `wire.rs:1088` |
| **zcode** | ✅ `:336` Standard | ✅ `:267` → `adapters.rs:179/608`，**2 文件** | ✅ `:45`（`cli/db/db.sqlite`） | ✅ 正文读 sqlite `:1164`，含子代理过滤 | ✅ 可写 `:134` | ⚠️ **礼包领取**非每日签到 | ✅ `adapter/zcode.rs:595` |
| **kimi** | ✅ `:340` Standard | ✅ `:282` → `adapters.rs:224/470`，1 文件，`Exact` | — **不入库**（见 §4.8） | ✅ `kimi_sessions.rs:193`（`has_source=false`） | — | — | — |
| **dsh** | ✅ `:345` Standard（**file 证据已过期**，见 §4.6） | ✅ `:297` → `adapters.rs:336/555`，**多文件**（N profile + `.env` + credentials） | — | ✅ `dsh_sessions.rs:215`（zstd 多帧） | — | — | — |
| **pi** | ✅ `:342` Standard | ✅ `:312` → `adapters.rs:298/488`，**2 文件** | — | ✅ `pi_sessions.rs:196` | — | — | — |
| **pencil** | ✅ `:344`（**无 CLI**） | ✅ `:327` → `adapters.rs:370/588`，1 文件 | — | — **不产生会话** | — | — | — |
| **qoder** | ✅ `:348` Standard | ✅ `:342` → `adapters.rs:370/588`，1 文件 | ✅ `:54` jsonl + **`qoder_usage.rs` 桌面账本**（见 §4.9） | 🟡 列表有、正文不可读 | ✅ 可写 `:132` | ✅ `checkin_qoder.rs:40` | ✅ `adapter/qoder.rs:650` |
| **qoder-cn** | ✅ `:349` Standard | ✅ `:357` → 同上，1 文件 | ✅ `:60`（`native:qoder:cn`） | 🟡 同 qoder | ⚠️ **无独立入口**（见 §4.5） | ✅ 走 `edition=cn` | ✅ 同一适配器 |
| **qwen** | ✅ `:339` Standard | ✅ `:372` → `adapters.rs:253/519`，1 文件，`Exact` | — | — | — | — | — |
| **workbuddy** | ✅ `:350`（**无 CLI**） | ✅ `:389` → `adapters.rs:276/534`，1 文件，`Prefixed` | ⚠️ `:66` 有读取器，**本机无客户端目录**（见 §4.10） | 🟡 列表有、正文不可读 | 🟡 **只读** `:135` | ✅ `checkin_workbuddy.rs:129` | ✅ `adapter/workbuddy.rs:368` |

### 1.1 特殊之处逐家说明

- **claude-code**：唯一用 `TokenMatch::LegacyLiteral`（兼容历史字面量 `jolu-claude-code`）。会话与工具调用解析最完整。
- **codex**：`token_match=Ignored` 且带 OAuth 分支——登录 ChatGPT 时只改地址、不写 token。
- **gemini**：13 家里唯一 `Detect::CliOnly`（`~/.gemini` 也被 Antigravity / Code Assist 用）。用量是整份快照重建，rewind 会删消息。
- **opencode**：sqlite v1/v2 与旧 JSON 三种格式，共存时只读 sqlite 防重复。
- **zcode**：唯一「库型」会话来源。签到是**礼包领取**（`checkin_zcode.rs:1-24`）而非每日签到。
- **kimi**：会话只能从 `~/.kimi-code/` 只读解析，**用量采集器完全缺失**。
- **dsh**：令牌在 `$DSH_HOME/.env` 而非主文件，所以整个 `is_wired` 交给 adapter 自己判。会话是 **zstd 多帧压缩**，需逐帧解压。
- **pi**：接线要写两个文件（`models.json` + `settings.json`），状态判定要读第二个文件。
- **pencil**：没有 CLI，只能靠配置文件判装。**它本身不产生会话记录。**
- **workbuddy**：`models.json` 是顶层数组（或包装对象），两种形状都要认。Skills 只读。

---

## 2. 验证强度矩阵（与实现分开）

数据来自对 `docs/verification/` 68 份记录的逐份定档：**真实端到端 17 份、真实只读对账 18 份、合成 24 份、未完成 8 份**。

| 能力域 | 真实通过 | 合成 / 模拟 | 无记录 | 强度判断 |
|---|---|---|---|---|
| **账号 / 额度** | WorkBuddy CN、Qoder CN、Trae CN、ZCode、Claude API Key、Codex | 账号卡 UI、多账号界面 | Gemini、Grok、OpenCode、Qoder 国际、WorkBuddy AI 国际 | 中偏强 |
| **签到** | WorkBuddy CN、Trae CN（真实领取）；Qoder CN（真实只读已领） | 签到 UI、批量流程 | Qoder 国际、Trae 第二账号、WorkBuddy 浏览器账号 | 强（国内三平台） |
| **路由 / 订阅转发** | WorkBuddy CN 窄场景（真实 200） | Claude/Codex/Gemini/Grok/OpenCode/ZCode/Trae 转发链路 | 各平台真实账号经网关的生成请求 | **弱到中** |
| **用量** | ZCode 17,127 条全量对账；Claude/Codex 真实读取；OpenCode 15 条 | 用量 UI 全页 | OpenCode V2 新表 | 强（三家），其余弱 |
| **会话** | Claude 149 条 + Codex 39 条真实只读；真实短会话压缩/续接 | 会话 UI | Gemini/Grok/Qoder/WorkBuddy/Trae 真实采集；ZCode 正文真机 | 中 |
| **Skills** | 隔离目录真实文件写入 | 全平台 Skills UI 与冲突检查 | **原工具真实目录的发现与热加载**（全平台） | **弱到中** |
| **工具调用** | — | 真实 CLI 抓包（假上游）、工具转换合成 | **模型发起→执行→回传→回答的真实链路** | **弱（最大缺口）** |
| **灵动岛** | ZCode 真实运行态翻回 running | 隔离桌面 + 合成网关分身 | 安装版 + 两个真实账号各开分身 | 中偏弱 |
| **更新** | 正式包 0.1.0→0.1.1 真实升级 | 更新流程 UI | 运行中自动化阻止安装、回滚、卸载、安装版通知点击 | 中 |

---

## 3. 真实账号不可用的平台（缺样本，不是不支持）

| 平台 | 现状 | 出处 |
|---|---|---|
| **Gemini** | 本机无可用登录，`unauthorized` | `2026-10-26-all-provider-queries.md:24,36` |
| **Qoder 国际** | 本机未登录，未发真实请求 | `2026-10-27-qoder-checkin-protocol.md:52` |
| **OpenCode** | 本机 0 个凭证 | `2026-10-23-real-validation.md:20` |
| **Grok** | 只有设备授权初始化成功 | `2026-10-26-all-provider-browser-login.md:41` |
| **Claude 订阅 OAuth** | `~/.claude/.credentials.json` 不存在 | `2026-10-23-account-backend.md:109` |
| **ZCode 付费档** | 缺付费个人 / 团队 / Start Plan / 国际 ZAI 样本 | `docs/todo/2026-10-22-unverified-integrations.md:14` |
| **Trae 付费国际** | 付费国际套餐、企业私有域未验证 | `2026-10-25-trae-account-access.md:52` |
| **WorkBuddy AI 国际** | 首轮真实返回 401 | `2026-10-26-all-provider-queries.md:23` |

另外两项环境限制被多份记录记为阻塞条件：**Windows Sandbox 不存在**、**goose 未安装**。

---

## 4. 声明与实现的不一致（本次盘点新发现）

以下是**代码与文档/界面互相矛盾**的地方，已逐条核实。4.1 已修复；4.9 / 4.10 是 2026-11-23 逐条对账时新查出的，其中 **4.9 是真实功能缺陷**（代码在跑，但读的路径在这台机器上不存在），其余列为待办。

### 4.1 dsh 平台 id 不一致（已修复 2026-11-23）

- `agentPage.ts` 把 `dsh` 映射成 `"deepseek"`（那是**图标表**的 id），而后端 `dsh_sessions.rs:259` 产出 `platform="dsh"`
- 后果：dsh 详情页的**会话数与技能数恒为 0**（`AgentsView.tsx:192-197` 拿这个值去查 `skillsApi.overview` 与 `sessionsList`）
- 已改为 `"dsh"`，并加测试 `详情页的平台 id 与后端产出的 platform 一致` 锁住

### 4.2 WorkBuddy 会话能力：前端说未接，后端已实现

- `agentPage.ts` 把 workbuddy 标为 `not-yet`（「会话记录格式还没接入」）
- 但 `native_sources.rs:66-77` 有 `.workbuddy/projects` 来源、`:507` 有 `parse::workbuddy`、`:898-950` 还读 `workbuddy.db` 的会话元数据
- **两者结论相反**。需确认 workbuddy 的会话是否真的进了 `requests` 表：若已进，前端应改为 `observable`

### 4.3 Qoder / WorkBuddy 会话验收表与代码矛盾

- `capabilityVerification.ts:114` 写 qoder sessions「none」、`:122` 写 workbuddy「none」
- 但 `native_sources.rs:54-77` 两个平台的采集器与解析器都已存在
- 需按实际数据库内容核对后更正验收表

### 4.4 ZCode 签到：验收表写 `na`，代码有实现

- `capabilityVerification.ts:110` 写 `checkin: na / 签到范围不含 ZCode`
- 但 `checkin_zcode.rs` 完整存在（status:85、claim:151），`daily_checkin.rs:145` 也把 `"zcode"` 列入可签到平台
- 差异在于它是**礼包领取**而非每日签到，验收表未区分这两种语义

### 4.5 qoder-cn 有接线但没有可用的会话/Skills 查询入口

- `agentPage.ts:26` 映射成平台 id `"qoder-cn"`
- 但后端会话数据全部挂在 `platform="qoder"`（`native_sources.rs:60-65`），`skills.rs:1131-1134` 对 `"qoder-cn"` 直接返回「暂未接入」
- 后果：接通 qoder-cn 后，详情页的会话数与技能数永远取不到，只有签到能用

### 4.6 dsh 探测的文件证据与接线主文件对不上

- `agent_discovery.rs:345` 的 `file: "config.yaml"` 是旧版结构
- 新版 dsh 主文件是 `profiles/<name>/cordis.patch.yml`
- 新装环境下文件证据永不命中，只能靠目录证据判装，状态会落到 `cli_missing`

### 4.7 版本探测只覆盖 5 家

- `agent_version.rs:28-33` 的 `AGENTS` 只有 claude / codex / gemini / opencode / zcode
- 可接线清单是 13 家
- 后果：kimi / dsh / pi / pencil / qoder / qoder-cn / qwen / workbuddy 八家的卡片**拿不到版本号**

### 4.8 Kimi / Pi / dsh 的用量「可读但不入库」

- 三个读取器都会算 requests / Tokens 并返回 `SessionItem`
- 但全模块**无任何 DB 写入**（对比 `native_sources.rs:524-577`）
- 后果：会话列表有数字，但**用量统计、成本、归因里没有它们的任何记录**

### 4.9 Qoder 采集器读的路径在本机不存在（2026-11-23 发现并修复）

**问题**：
- 采集器读 `~/.qoder/projects`（`native_sources.rs:58`）与 `%APPDATA%\com.qoder.app.stable\main.sqlite`（`:903`）
- **本机两者都不存在**：`~/.qoder` 下只有 `session-env` / `shared_client` / `skills`
- 真实账本在 `~/.qoder/shared_client/cache/db/local.db` 的 `chat_message` 表：**999 条带 Token 的 assistant 调用**（去重 `request_id` 315，实为 315 轮 agent 对话）、合计 prompt 46.8M / completion 0.62M / cached 38.7M
- Jolu 的 `requests WHERE platform='qoder'` 只有 2 行，全部来自自己的网关转发（`source='gateway-subscription'`）
- **按 ID 逐条对账：0 交集**

**修复（2026-11-23）**：新增 `backend/src/qoder_usage.rs`
- 读 `chat_message` 的 `session_id` / `id` / `gmt_create` / `token_info`，其余列（`content` / `answer` / `question` / `reasoning_content`）一律不碰
- `dedup_key = {session_id}:{message_id}`。**不是 `request_id`**：它是「一轮对话」，一轮里实测最多 57 次调用，按它合并会丢掉 56/57 的用量
- `input_tokens = prompt_tokens - cached_tokens`（`cached` ⊂ `prompt`，999/999 成立），`input_semantics = "includes_cache"`
- `model_key`（ultimate / auto / efficient…）是档位不是模型名，不写 `model`
- 来源可用性：`global` 实例「`projects` 目录 或 账本」任一存在即可用；`cn` 实例只认自己的目录（共用主账本会让同一批记录被两个 source 各写一遍）

**验证**：
- 单测 6 条（缓存扣减 / `request_id` 不是去重键 / 非 assistant 与空 token 跳过 / 档位不冒充模型名 / 时间过滤 / 缺表报错）
- `qoder_source_is_available_from_the_ledger_alone`：只有账本没有 projects 目录时也须可用并采到数据（这条**抓到两个实例重复写同一批记录的 bug**：首跑 4 行而非 2 行）
- `real_qoder_ledger_scans_into_a_temp_db`（`#[ignore]`，真机只读）：真实账本 → 临时库，**999 条**，新鲜输入 8,078,210 + 缓存读 38,745,227 + 输出 619,650，与独立 Python 逐项一致，零重复键

**当前验证边界**：读取器修复和真实账本→临时库的 999 条复验已通过，但正式 `usage.db` 尚未运行修复后的扫描，当前不能声明 Qoder 正式运行态对账通过。`local.db` 的稳定布局也只在本机一份样本上确认。

详见 [P0-3 多平台对账 §3](../verification/2026-11-23-p0-3-multi-platform-reconciliation.md)（字段语义见 §3.3）。

### 4.10 WorkBuddy 本机无客户端

- `~/.workbuddy`、`~/.workbuddy-ai`、`~/.codebuddy` 均不存在，`AppData` 下也无可疑目录
- Jolu 的 10 行 workbuddy 记录全部来自网关转发
- 无法做源端对账，标「缺样本」而非「不支持」

---

## 5. 结论

### 5.1 实现完成度

13 家全部可探测、全部可接线。**用量采集实现覆盖 8 家**（claude/codex/gemini/opencode/zcode/qoder/qoder-cn/workbuddy）。其中 Qoder 已完成读取器临时库复验，正式 `usage.db` 扫描待补，WorkBuddy 因本机无客户端样本仍无法做源端对账，
**会话读取覆盖 11 家**（8 家列表 + gemini/opencode/qoder/workbuddy 正文不可读），
**Skills 覆盖 9 家**（workbuddy 只读），**签到覆盖 4 家**（WorkBuddy CN / Qoder 双版 / Trae CN / ZCode 礼包），
**订阅转发覆盖 10 家**。

### 5.2 验证完成度

| 判定 | 家数 | 说明 |
|---|---:|---|
| 真实端到端通过 | 3 | Claude Code、Codex、WorkBuddy CN（窄场景） |
| 真实只读通过 | 6 | 上述 + ZCode、Qoder CN、Trae CN |
| 仅合成 / 模拟 | 4 | Gemini、OpenCode、Grok、Pencil（Pencil 无会话） |
| 无验证记录 | — | 实现存在但从未真机验证的组合见 §2、§3 |

### 5.3 最大的三个缺口

1. **工具调用整条链路没有真实证据**：模型发起 → 工具执行 → 结果回传 → 最终回答，目前只有假上游的抓包。
2. **Skills 全平台只在隔离目录验过**：原工具真实目录（`~/.claude/skills`、`~/.codex`、`.codebuddy/skills`）的发现与热加载没核对。
3. **订阅转发真实账号生成未验收**：Claude / Codex / Gemini / Grok / OpenCode / ZCode / Trae 的「真实账号经网关发一次生成」都没有记录。

### 5.4 记录之间互相矛盾的项

`2026-11-17-capability-verification.md:23-40` 列出 7 组矛盾，其中与本次判定直接相关的三组：

- WorkBuddy 额度：401 vs ok
- ZCode 额度：四份记录四种结果
- Trae 国内签到：「不声称完成」vs「缺口已解除」

这些需要在下一轮验收时重新取一次真实样本定案，不能只看旧记录。
