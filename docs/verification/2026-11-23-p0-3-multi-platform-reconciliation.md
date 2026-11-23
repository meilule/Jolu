# P0-3 多平台按请求 ID 逐条对账

日期：2026-11-23
状态：**只读对账。Claude Code / OpenCode / Qoder / ZCode 的正式 `usage.db` 均已按稳定 ID 逐条比对通过；WorkBuddy 本机无客户端目录，按计划 §3 记录真实样本缺口。**

**样本时间**：2026-11-23。源端与 Jolu 快照在同一会话内连续采集（两侧都只读打开，未触发扫描）。本次修复后正式扫描结束，Qoder 999 条与源端全量匹配；ZCode 目前源端 18,872 条均已在 Jolu 中找到，早先快照中的暂时性扫描滞后已消失。
配套计划：[产品对齐执行计划 §P0-3](../plans/2026-11-23-product-alignment-execution-plan.md)
配套基线：[能力矩阵](../specs/2026-11-23-capability-matrix.md)

---

## 0. 方法与口径

**全部会话只读**：源端库与 Jolu `usage.db` 都用 `sqlite3.connect("file:...?mode=ro", uri=True)` 或只读文件句柄打开，无任何写操作、无扫描触发。

**对账单元是「稳定 ID」，不是总数**。每家的 ID 都取自该平台自己的请求标识，不跨平台拼接：

| 平台 | 源端位置 | 源端 ID 字段 | Jolu `dedup_key` 构造 |
|---|---|---|---|
| Claude Code | `~/.claude/projects/**/*.jsonl` | `message.id` | 原样 `message.id`（`collector.rs:160`） |
| OpenCode | `~/.local/share/opencode/opencode.db` 的 `message` 表 | `id` + `session_id` | `{session}:{id}`（`opencode_usage.rs:31`） |
| Qoder | `~/.qoder/shared_client/cache/db/local.db` 的 `chat_message` | `session_id` + `id` | `{session_id}:{message_id}`（`qoder_usage.rs`；`request_id` 只是一轮对话标识） |
| WorkBuddy | `~/.workbuddy/projects/**/*.jsonl` | `id` / `messageId` | `{sessionId}:{id}`（`native_parse.rs:28`） |
| ZCode | `~/.zcode/cli/db/db.sqlite` 的 `model_usage` | `request_id` | 见 2026-11-21 计划 §10.3 |

**字段比对复刻生产代码的合并语义**，不是原始行比对。`db::insert_records_into`（`db.rs:687`）的规则是：

1. 首次见到该 ID：插入该行
2. 再次见到：token 各字段取 `max(旧, 新)`，**且只有值真的变了才写**；只有写了才 `ts = MAX(ts, 新 ts)`
3. `None` 不覆盖已有值（未知不会抹掉已知）

所以源端同一条消息出现多次（流式更新）时，Jolu 保存的是「首次值 + 后续最大值」，`ts` 是「首次插入时间 + 有更新时的最大时间」。直接拿源端最后一行比会得到大量假差异——第一轮比对就是因此报了 2,730 条时间不符，复刻合并语义后降到 0。

**「只在 Jolu」不等于错误**，要看源端保留策略：

| 平台 | 源端保留策略 | 只在 Jolu 的解释 |
|---|---|---|
| Claude Code | 会话文件按项目目录滚动清理 | 早期会话文件已被删除 |
| OpenCode | 不清理（本机 168 行从未缩水） | 应恒为 0，非 0 即缺陷 |
| WorkBuddy | 不清理 | 应恒为 0，非 0 即缺陷 |
| Qoder | `chat_message` 不清理 | 应恒为 0，非 0 即缺陷 |
| ZCode | `model_usage` 删除超过 30 天 | 保留期之外的历史 |

---

## 1. Claude Code

**源端**：`~/.claude/projects/` 下 62 个 `.jsonl`，`type=assistant` 且带 `message.usage` 的行，按 `message.id` 去重后 **3,295 个 ID**。
**Jolu**：`requests WHERE platform='claude'` 共 **3,987 行**（`total_known=1` 3,987，即全部有 Token）。

### 1.1 数量对账

| 项 | 条数 |
|---|---:|
| 源端唯一 ID | 3,295 |
| Jolu 行数 | 3,987 |
| **交集** | **3,295（覆盖源端 100%）** |
| **只在源端** | **0** |
| 只在 Jolu | 692 |

### 1.2 只在 Jolu 的 692 条

| 项 | 值 |
|---|---|
| 日期分布 | 10-12 (14) / 10-13 (108) / 10-15 (292) / 10-16 (117) / 10-17 (120) / 10-20 (41) / 10-28 (6) |
| 范围 | 2026-10-12 00:13 .. 2026-10-28 |
| 源端最早 | 2026-10-28 16:57:37 |

**全部落在源端最早记录之前**。Claude Code 的会话文件已被它自己清理，Jolu 保留了历史账本——与 ZCode 的 30 天清理是同一类现象，属于设计行为，不是虚构数据。

注：692 与「早于源端最早」的 698 相差 6 条，是同一毫秒边界附近的 10-28 记录，不影响结论。

### 1.3 字段比对（交集 3,295 条，全量）

| 字段 | 不一致 |
|---|---:|
| `ts` | **0** |
| `model` | **0** |
| `input_tokens` | **0** |
| `output_tokens` | **0** |
| `cache_read_tokens` | **0** |
| `cache_write_tokens` | **0** |

Token 合计：源端 801,570,164；Jolu 已知合计 948,488,830。差额来自仅在 Jolu 的 692 条历史，与交集无关。

---

## 2. OpenCode

**源端**：`~/.local/share/opencode/opencode.db`。`detect_layout` 选中 `SqliteV1`（存在 `message` 表；`session_message` 为 0 行）。按 `opencode_usage::parse` 的过滤条件（`role/type=assistant`、`tokens` 是对象、至少一个 Token 字段非 None）从 168 行中解析出 **132 条用量**。
**Jolu**：`requests WHERE platform='opencode'` 共 **132 行**（分属 6 个 provider source）。

### 2.1 数量对账

| 项 | 条数 |
|---|---:|
| 源端解析出的 ID | 132 |
| Jolu 行数 | 132 |
| **交集** | **132（100%）** |
| **只在源端** | **0** |
| **只在 Jolu** | **0** |

时间范围两侧完全一致：`2026-04-19 19:39:12 .. 2026-10-28 21:18:07`。

### 2.2 字段比对（交集 132 条，全量）

| 字段 | 不一致 |
|---|---:|
| `ts` | **0** |
| `source`（provider 后缀） | **0** |
| `input_tokens` / `output_tokens` | **0** / **0** |
| `cache_read_tokens` / `cache_write_tokens` | **0** / **0** |
| `total_tokens`（源端有值时必比） | **0** |
| 「源端 None 但 Jolu 标 known」 | **0** |

Token 合计：源端 2,521,590；Jolu 已知合计 2,521,590，**完全相同**。

源端 119 / 132 条带显式 `total`，其余 13 条走实现的 fallback（分项全可得时求和）。第一轮比对把这 13 条报成差异，复刻 fallback 后归零——是核对脚本的问题，不是生产代码的问题。

---

## 3. Qoder（本机发现真实缺口）

**源端**：`~/.qoder/shared_client/cache/db/local.db`（40.8 MB），`chat_message` 表 2,480 行，其中 `role='assistant'` 且 `token_info` 非空 **999 行**，去重后 **315 个 `request_id`**。
`token_info` 形状统一为 `{prompt_tokens, completion_tokens, cached_tokens, max_input_tokens}`；`model_info` 为 `{model_key}`（ultimate 668 / auto 311 / efficient 11 / performance 6 / lite 3）。
时间范围：2025-12-02 14:52:51 .. 2026-01-24 00:26:28。合计 prompt 46,823,437 / completion 619,650 / cached 38,745,227。
`cached_tokens` 是 `prompt_tokens` 的子集（999/999 满足 `cached <= prompt`），字段映射见 §3.3。

**修复前 Jolu 快照**：`requests WHERE platform='qoder'` 仅 **2 行**，且都是网关转发行（`source='gateway-subscription'`，`session_id=NULL`，时间 2026-11-14 16:05）。修复后正式扫描新增了 999 条本地账本记录，这 2 条网关记录仍保留。

### 3.1 修复前数量对账

| 口径 | 源端 | 交集 | 只在 Jolu | 只在源端 |
|---|---:|---:|---:|---:|
| 按每条 assistant 消息 `{session}:{message_id}`（= 一次 LLM 调用） | 999 | **0** | 2 | **999** |
| 按每轮对话 `{session}:{request_id}` | 315 | **0** | 2 | **315** |

**两种口径都是零交集。**取更细的 999 更合适——`request_id` 是一轮 agent 对话，一轮里可能有几十次调用（见 §3.3），按它合并会低估用量。

### 3.2 修复前结论：Qoder 的源端采集器在这台机器上完全没工作

| 检查项 | 值 |
|---|---|
| 采集器读取的路径 | `~/.qoder/projects`（`native_sources.rs:58`） |
| 该路径是否存在 | **否**（`~/.qoder` 下只有 `session-env` / `shared_client` / `skills`） |
| Qoder 桌面库路径 | `%APPDATA%\com.qoder.app.stable\main.sqlite`（`native_sources.rs:903`） |
| 该路径是否存在 | **否** |
| 真实有账本的位置 | `~/.qoder/shared_client/cache/db/local.db` 的 `chat_message` |

所以 Jolu 那 2 行与源端 999 行毫无关系：前者来自 Jolu 自己的网关转发（`browser:` 前缀的连接），后者是 Qoder 客户端自己的历史对话。

**这是 P0-3 这一轮查出的最实质问题**：能力矩阵 §1 把 qoder 的用量采集标成 ✅（依据 `native_sources.rs:54`），但那是「代码路径存在」，在**本机真实布局下取不到任何数据**。矩阵 §2 也把 Qoder 列入「账号 / 额度 无验证记录」，两处结论一致地指向同一个缺口。

同时这也解释了矩阵 §4.3「Qoder / WorkBuddy 会话验收表与代码矛盾」的一半原因：验收表写 `none` 是对的（本机确实没采到），代码里那段采集逻辑没有可用的输入。

### 3.2b 修复与临时库复验（2026-11-23）

新增 `backend/src/qoder_usage.rs` 读取 `chat_message`，字段映射按 §3.3 的实测结论。同时修了来源可用性判定（原本只按 `~/.qoder/projects` 是否存在）。

**真机只读复验**（`real_qoder_ledger_scans_into_a_temp_db`，写临时库、不碰用户 `usage.db`）：

| 项 | 结果 |
|---|---|
| 扫出条数 | **999** |
| 新鲜输入（prompt − cached） | 8,078,210 |
| 缓存读 | 38,745,227 |
| 输出 | 619,650 |
| 合计 | 47,443,087 |
| 重复 dedup_key | **0** |
| 错误 | **0** |

与本节用独立 Python 算出的 999 条 / 46.8M prompt / 0.62M completion 逐项一致。

**修复过程中被测试抓到的真实 bug**：`qoder_source_is_available_from_the_ledger_alone` 第一次跑出 4 行而非 2 行——`native:qoder:global` 与 `native:qoder:cn` 两个实例都判定可用，**共用同一份账本、各写一遍**（去重键含 `source`，真的变成两行）。已限定只有 `global` 实例读主账本。

### 3.2c 正式库扫描与逐条复验（2026-11-23）

应用运行修复版本完成正式扫描后，重新运行只读对账脚本，结果如下：

| 项 | 结果 |
|---|---:|
| Qoder 源端稳定 ID | **999** |
| Jolu Qoder 行数 | **1,001**（999 条本地记录 + 2 条网关记录） |
| **交集** | **999 / 999** |
| 只在源端 | **0** |
| 字段差异 | **0** |
| 重复扫描后的本地账本行 | **999**，无重复键 |

因此修复前的零交集已由正式库复验关闭。`local.db` 的布局目前只在本机确认，跨机器稳定性仍待有 Qoder 的另一环境验证。

### 3.3 源端字段语义（为将来接入采集器准备）

**`request_id` 不是请求标识，是「一轮用户对话」的标识。** 实测最长的一个 `request_id` 含 57 行：

| 观察 | 值 |
|---|---|
| 该 `request_id` 的 assistant 行数 | 57 |
| 57 行的 `chat_message.id` | **全部不同** |
| 同 session 的角色分布 | `user` 1 / `assistant` 57 / `tool` 97 |
| 同 session 的 `request_id` 数 | 1 |
| `prompt_tokens` 序列 | 17,128 → 19,588 → 21,451 → … → 158,579（**单调递增**） |

这是典型的 agent 循环：一条用户提问 → 模型调用 → 工具执行 → 再调用，上下文逐步累积。**每一行都是一次真实的 LLM 调用、都有自己的 Token 消耗**，把它们当「同一次请求的重试」合并会丢掉 56/57 的用量。

全局统计也支持这个判定：

| 项 | 值 |
|---|---|
| assistant 带 Token 的消息数 | 999 |
| 去重 `request_id` | 315 |
| 去重 `(session_id, id)` | **999（id 全局唯一）** |
| 每个 `request_id` 的 distinct session 数 | **全部为 1** |
| session 数 | 60（每 session 的 request 数 1–23） |

所以：**`request_id` ≈ 一轮对话，`chat_message.id` = 一次 LLM 调用**。Jolu 的 `requests` 表一行对应一次调用，因此 dedup_key 应取 `{session_id}:{chat_message.id}`。

**Token 语义**：`cached_tokens` 是 `prompt_tokens` 的子集（999/999 满足 `cached <= prompt`，无一例外），与 Codex 的 `includes_cache` 同型：

| 源端字段 | 映射 | 说明 |
|---|---|---|
| `prompt_tokens` | `input_tokens + cache_read_tokens` 的总量 | 含缓存 |
| `cached_tokens` | `cache_read_tokens` | prompt 的子集 |
| `completion_tokens` | `output_tokens` | |
| （无） | `total_tokens` = prompt + completion | 源端不提供显式 total |
| `max_input_tokens` | — | 上下文窗口，不是用量 |

`input_tokens` 保存源端原始 `prompt_tokens`，`input_semantics = "includes_cache"`，`cached_tokens` 单独映射到 `cache_read_tokens`。消费侧按该语义扣一次缓存，得到新鲜输入；采集时提前扣除会导致再次扣减。

**`model_key` 不是模型名**（`ultimate` / `auto` / `efficient` / `performance` / `lite`），是档位。真实模型名不在 `chat_message` 里，需查 `chat_session.preferred_model_info` 或不填（保持未知）。

---

## 4. WorkBuddy

### 4.1 本机状态：没有客户端，无法对账

| 检查项 | 结果 |
|---|---|
| `~/.workbuddy` | 不存在 |
| `~/.workbuddy-ai` | 不存在 |
| `%APPDATA%` / `%LOCALAPPDATA%` 下含 workbuddy 的目录 | 无 |
| `~/.codebuddy` | 不存在 |
| Jolu 中的 workbuddy 行 | 10 行，**全部** `source='gateway-subscription'` |

全盘扫描了 `~/.workbuddy*`、`~/.codebuddy*`、`*WorkBuddy*`、`*workbuddy*`（home / `%APPDATA%` / `%LOCALAPPDATA%` 三处），**一个都没命中**。

Jolu 的 10 行是网关转发记录（`browser:` 连接），能自证「Jolu 转发了 10 次请求」，但**不能**验证「WorkBuddy 本地用量是否被完整采集」——因为本机根本没有可采的东西。

**结论：本机无法执行 WorkBuddy 按 ID 对账。这是环境缺样本，不是代码缺口。** 按计划 §3「真实账号调用…无样本时记录缺口并推进其他任务」处理。

### 4.2 为「有客户端的机器」预先做好的准备

不能在这台机器上跑对账，但把**跑对账所需的一切**都准备好了，换台机器只需一条命令：

**对账脚本已支持 WorkBuddy**：`scripts/acceptance/reconcile-usage-by-id.py` 新增 `read_workbuddy()`，读 `~/.workbuddy/projects/**/*.jsonl`，字段映射严格复刻 `native_parse::workbuddy`：

| 源端 | 映射 |
|---|---|
| `id` 或 `messageId` | `dedup_key` 的一部分（`{sessionId}:{id}`） |
| `providerData.usage.inputTokens` | `input_tokens`（回退 `rawUsage.prompt_tokens`） |
| `providerData.usage.outputTokens` | `output_tokens`（回退 `rawUsage.completion_tokens`） |
| `providerData.usage.totalTokens` | `total_tokens`（回退 `rawUsage.total_tokens`） |
| `rawUsage.prompt_tokens_details.cached_tokens` | `cache_read_tokens`（回退 `prompt_cache_hit_tokens` → `usage.inputTokensDetails[]`） |
| `rawUsage.prompt_cache_write_tokens` | `cache_write_tokens`（回退 `cache_creation_input_tokens`） |
| `rawUsage.prompt_tokens` 存在 | `input_semantics = includes_cache` |

**与 Rust 解析器的交叉验证**（本机能做的最强验证）：
- `backend/src/native_parse.rs` 新增 `workbuddy_shapes_that_the_reconcile_script_must_match`：四种真实形状（rawUsage 带缓存 / 只有 usage / `messageId` + `inputTokensDetails` 数组 / 缓存写）逐字段断言 Rust 解析器的输出。
- 同一组样本喂给 Python `read_workbuddy()`，与 Rust 基准**逐字段一致**（4/4 通过）。
- 这条测试的存在意义：**对账报告的「字段差异」完全取决于两边读法一致**。读法不一致时，报告会显示一堆假差异（Claude 首轮就因此报了 2,730 条假时间差异）。现在至少保证「两边读法相同」这一半是对的。

**在有客户端的机器上要做的三件事**（各约 5 分钟）：
1. `python -I scripts/acceptance/reconcile-usage-by-id.py --platform workbuddy`
2. 把输出贴进本节 §4.3
3. 若交集不为 100% 或字段有差异，按 §0 的判据逐项归因

### 4.3 对账结果

**待补**：需在装有 WorkBuddy 客户端的机器上执行（见 §4.2）。

### 4.4 一个在准备过程中查出的真实缺陷（已修）

`qoder_usage.rs` 最初把 `prompt_tokens - cached_tokens` 存进 `input_tokens` 并标 `includes_cache`。但 `source_store::write` 的约定是「`input_tokens` 存来源给的**原始**输入」，新鲜 Token 在消费侧按语义扣：

```sql
CASE input_semantics WHEN 'includes_cache' THEN max(input_tokens-cache_read_tokens,0)+output_tokens END
```

两处都扣 = 扣两次，缓存大的请求新鲜输入会算成 0（本机 Qoder 缓存占比 83%，影响极大）。已改为存原始 `prompt_tokens`。**这条是写 WorkBuddy 读取器时对照 `native_parse::workbuddy`（同样存原始 `inputTokens`）才发现的**——`qoder_usage` 的测试当时锁的是我写错的期望值，所以没抓到。

对应改动：`qoder_usage.rs` 的 `input = prompt`，测试改名 `raw_prompt_is_stored_and_cache_stays_separate`；`qoder_source_is_available_from_the_ledger_alone` 的断言从 700 改为 3000（原始 prompt 之和）。

---

## 5. ZCode（沿用既有结论，格式统一）

来自 [用量准确性计划 §10.3](../plans/2026-11-21-usage-data-accuracy-plan.md) 与本次计划 §P0-3：

用本轮固化的脚本重跑（`scripts/acceptance/reconcile-usage-by-id.py --platform zcode`）：

| 项 | 条数 |
|---|---:|
| ZCode `model_usage` | 18,872 |
| Jolu `native:zcode:main` | 24,744 |
| **交集** | **18,872** |
| 只在 Jolu | **5,872**（源端 30 天清理的历史记录） |
| 只在源端 | **0** |

字段差异：**零**。本次源端全部 18,872 个 ID 均已在 Jolu 找到，逐条字段比对零差异。

**关键校验**：只在 Jolu 的 5,872 与 2026-11-21 计划的调查结果**完全相同**，说明脚本复刻的合并语义与生产代码一致——如果口径有偏差，这个数字不可能连续三轮对得上。

`dedup_key` 是 `model_usage.id`（`native_sources.rs:779`），不是 `logical_request_id`：后者是一次逻辑请求，可能对应多个 attempt 行。按 `id` 对账与 Jolu「一行一次调用」的口径一致。

---

## 6. 汇总

| 平台 | 交集 | 只在源端 | 只在 Jolu | 字段差异 | 判定 |
|---|---:|---:|---:|---|---|
| **Claude Code** | 3,295 / 3,295 | **0** | 692（源端已清理） | **零** | ✅ 通过 |
| **OpenCode** | 132 / 132 | **0** | **0** | **零** | ✅ 通过（最强证据） |
| **Qoder** | 999 / 999 | **0** | 2（网关行） | **零** | ✅ 正式库逐条对账通过（见 §3.2c） |
| **WorkBuddy** | 待补 | 待补 | 待补 | 待补 | ⬜ 本机无客户端；对账工具已就绪（§4.2），换机一条命令 |
| **ZCode** | 18,872 / 18,872 | **0** | 5,872（源端清理） | **零** | ✅ 通过 |

**判定标准**：交集覆盖源端 100%、只在源端 0、字段零差异 → 通过。只在 Jolu 的条数只有在能被源端保留策略解释时才算通过。

---

## 7. 本轮新增的待办

| 编号 | 问题 | 影响 | 建议 |
|---|---|---|---|
| ~~P0-3-a~~ | ~~Qoder 采集器读错路径~~ | ~~999 条调用 / 46.8M prompt Token 未入库~~ | ✅ **本机问题已修复并关闭**：临时库验证、正式扫描和 999 条逐 ID 对账均通过。另一台机器的布局确认作为跨环境兼容性后续项 |
| ~~P0-3-b~~ | ~~能力矩阵把 qoder / workbuddy 用量标 ✅ 依据的是「代码路径存在」~~ | ~~与 §2 验证强度矩阵的结论矛盾~~ | ✅ 已修正：能力矩阵已把「实现覆盖」与「本机真实取数 / 验证强度」分列，并记录 Qoder 正式库复验通过、WorkBuddy 本机缺少客户端样本 |
| **P0-3-c** | WorkBuddy 无客户端目录 | 无法对账 | 保持未验证；需真实安装的机器 |
| **P0-3-d** | Qoder `model_key`（ultimate / auto / efficient …）是档位不是模型名 | 即便接入采集器，模型分布也只能显示档位或留空 | 真实模型名需另找来源（`chat_session.preferred_model_info`），找不到就保持未知，不拿档位冒充模型名 |
| ~~P0-3-e~~ | ~~对账脚本未入库~~ | — | ✅ 已固化：`scripts/acceptance/reconcile-usage-by-id.py`，五个平台可一次跑完，`--json` 供机器读 |

---

## 8. 复现步骤

```bash
# 全部五个平台，表格输出
python -I scripts/acceptance/reconcile-usage-by-id.py

# 单个平台
python -I scripts/acceptance/reconcile-usage-by-id.py --platform qoder

# 机器可读，便于跨机器比对
python -I scripts/acceptance/reconcile-usage-by-id.py --json > output/acceptance/reconcile.json
```

脚本本身只读（源端与 Jolu 库都 `mode=ro`），不读消息正文，不触发扫描。每个平台的源端读取器都复刻了对应生产代码的过滤与合并语义（见脚本内注释与 §0）。

**不要**用「两个总数相近」代替上表。Claude 两侧数量相差 692 行，是源端已清理的历史；Qoder 额外的 2 行是网关记录；逐条 ID 和字段比较才能区分这些情况。
