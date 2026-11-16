# Agent 上下文观察后端实现执行文档

日期：2026-11-16
状态：待执行
范围：后端新增「请求上下文快照」采集与查询；前端 V10 画板已由鼠鼠完成，本文只讲后端。
读者：接手执行的同事。

---

## 0. 执行纪律（先读这条，优先级高于本文所有内容）

**本文是调查记录与设计契约，不是规范。执行前必须到两个项目里自己核对，禁止照抄本文的结论或代码片段。**

1. **先读画板**：前端设计在 `Jolu.pen`，按名称搜 `V10-`。本文第 2 节列了画板清单与各自要的数据，但**画板本身才是唯一设计依据**，本文与画板冲突时以画板为准。
2. **先读 magpie 对应实现**：本文引用的 magpie 路径相对 `D:\develop\_research\magpie`，参考 commit `e9dd8e8`。行号只用于快速定位，**以函数名为准**；执行前先 `git log --oneline -1` 确认 commit 是否变化。
3. **先读 Jolu 现有实现**：本文第 7 节列的「可复用」每一项都要自己确认函数还在、行为没变。
4. **任何一处与本文不符，先更新本文再改代码**。不允许「文档说要这样，代码不是这样，那我按文档改」。
5. **不确定的行为不要猜**。magpie 的 `prompt.go` 里有大量注释写明踩坑历史（例如为什么某段文本要从系统提示词里剥出来算「记忆」），读注释再动手。

---

## 1. 目标

在模型路由页实现「看见每一次请求的上下文」：以会话为入口，展示最近一次请求的上下文占用、缓存命中、八类组成与逐项明细。

**这不是 magpie 那张图的照搬**，差异见第 4.2 节末尾。Jolu 的设计多了：未分类兜底、来源证据、采集状态机、脱敏摘要、历史请求选择。

---

## 2. 画板清单（15 张，7 组）

| 组 | 画板 | 需要的数据 |
|---|---|---|
| 索引 | V10-00 | — |
| 运行概览与活跃会话 | V10-01 / 02 | 会话列表：agent/项目、实际模型/来源、最近上下文（已用/窗口）、缓存读取/占输入、状态、最近请求时间 |
| 会话观察与上下文组成 | V10-03 / 04 | 单次快照：链路、状态行、上游实测、八类组成、明细表格（名称/类别/次数/token/占输入） |
| 工具结果原位展开与脱敏摘要 | V10-05 / 06 | 按工具聚合 + 逐条：命令、时间、token、**脱敏摘要**、正文是否采集 |
| 历史请求选择与跟随暂停 | V10-07 / 08 | 会话的历史快照列表 |
| 矩形图与文件筛选联动 | V10-09 / 10 | 同 V10-03 数据，前端换可视化；文件筛选需要 `items` 的类别与路径 |
| Skills 来源证据与未识别边界 | V10-11 / 12 | 每个 skill / 文件的来源证据（路径、tag），以及「未分类」占比 |
| 采集状态与交互规范 | V10-13 / 14 | 六种状态的如实区分（见 3.3） |

画板 id（便于截图对照）：`AyNtk`（索引）、`BRrfK`/`HcN8u`（01/02）、`bip7H`/`LbqBW`（03/04）、`JhF7m`/`tYEOL`（05/06）、`yRpgu`/`P19cF9`（07/08）、`bo3VW`/`YB1Tq`（09/10）、`fBPX5`/`FmLIZ`（11/12）、`dpu3z`/`x7LqM`（13/14）。

---

## 3. 数据契约

### 3.1 表

新增 `gateway_context_snapshot`（**不要塞进 `gateway_route_log`**，后者只保留 1000 行且只存元数据）：

```sql
CREATE TABLE IF NOT EXISTS gateway_context_snapshot (
  id            INTEGER PRIMARY KEY,
  session_key   TEXT    NOT NULL,   -- 平台 + 会话 id，与 gateway/sessions.rs 同一套
  request_id    INTEGER,            -- 关联 gateway_route_log.request_id（同一次网关请求）
  connection_id TEXT,
  ts            INTEGER NOT NULL,
  state         TEXT    NOT NULL,   -- analyzing / ready / failed / unsupported / no_body
  input_tokens  INTEGER,            -- 上游实测，未知 NULL（不存 0）
  output_tokens INTEGER,
  cache_read    INTEGER,
  window_tokens INTEGER,            -- 模型窗口，来自 models.dev 元数据；未知 NULL
  estimated     INTEGER,            -- 本地估算总量
  counted       INTEGER NOT NULL DEFAULT 0,  -- 1 = 已被上游实测校准
  parts_json    TEXT,               -- 八类组成
  error         TEXT
);
CREATE INDEX IF NOT EXISTS idx_ctx_snap_session ON gateway_context_snapshot(session_key, ts DESC);
CREATE INDEX IF NOT EXISTS idx_ctx_snap_request ON gateway_context_snapshot(request_id);
```

**清理**：跟 `gateway_route_log` 一起按条数清理（保留量与流水一致或更少），清理逻辑写在同一处，避免两套。

### 3.2 parts_json

八类，顺序固定（前端配色与顺序依赖它，见 V10-13「类别颜色、顺序固定」）：

```json
[
  {"kind":"system",  "tokens":35300,  "items":[{"name":"prompt","tag":"","tokens":20000}]},
  {"kind":"tools",   "tokens":27100,  "items":[{"name":"Bash","tag":"tool","n":42,"tokens":800}]},
  {"kind":"skills",  "tokens":16600,  "items":[{"name":"xxx","tag":"skills","tokens":1200}]},
  {"kind":"memory",  "tokens":14200,  "items":[{"name":"reminders","tag":"reminder","tokens":900}]},
  {"kind":"files",   "tokens":112400, "items":[{"name":"D:\\develop\\Jolu\\AGENTS.md","tag":"project","tokens":21600}]},
  {"kind":"results", "tokens":206800, "items":[{"name":"Bash","tag":"shell","n":299,"tokens":88800}]},
  {"kind":"chat",    "tokens":180000, "items":[{"name":"turns","tag":"turns","n":62,"tokens":180000}]},
  {"kind":"unknown", "tokens":38600,  "items":[{"name":"other","tag":"","tokens":38600}]}
]
```

规则：
- 每类最多 **24 项**（magpie 的 `promptItems`），其余汇总成一项 `{"name":"more","n":N}`；
- **只存名字、tag、次数、token**，不存正文（AGENTS.md 要求）；
- `unknown` 是 Jolu 相对 magpie 新增的兜底类：认不出的内容一律归这里，**不推断**为技能或文件（V10-11 的「未识别边界」）。

### 3.3 状态机（V10-13 六种状态）

| state | 何时 | 前端表现 |
|---|---|---|
| `analyzing` | 已收到请求，解析未完成 | 总量可先展示，组成显示骨架，**不以 0 占位** |
| `no_body` | 该来源拿不到请求体（部分订阅适配器） | 「尚未采集上下文」，保留模型/来源/时间 |
| `failed` | 解析或落库失败 | 「采集失败」，可重试；有旧快照时标「上次成功 HH:MM」 |
| `unsupported` | 请求体格式不认识（非三种协议） | 「此来源暂不支持内容分析」 |
| `ready` | 有完整快照 | 正常展示 |
| 无行 | 会话还没走过路由 | 「暂无请求」，**不拿历史累计用量冒充上下文** |

补充：
- `window_tokens` 为 NULL 时前端隐藏百分比与剩余空间（V10-13 第 5 种状态）；
- `cache_read` 为 NULL 时写「未提供」，来源无缓存能力时写「不支持」——两者必须分开。

### 3.4 查询命令（5 个）

| 命令 | 服务画板 | 关键返回 |
|---|---|---|
| `gateway_context_sessions` | V10-01 | 每个会话：agent、项目、实际模型、来源账号、最近快照（已用/窗口/占比）、cache_read/input、状态、最近请求时间 |
| `gateway_context_snapshot(session_key, request_id?)` | V10-03 | 链路（请求模型 → 分组 → 实际模型/来源）、状态行（完成时间/用时/响应头）、上游实测、八类组成 |
| `gateway_context_items(snapshot_id, kind?, search?)` | V10-03 表格 | 明细：名称、类别、次数、token、占输入；支持按类别筛选与名称/路径搜索 |
| `gateway_context_tool_items(snapshot_id, tool_name)` | V10-05 | 逐条：命令、时间、token、**脱敏摘要**、正文是否采集 |
| `gateway_context_history(session_key)` | V10-07 | 历史快照列表（id、时间、模型、token、状态） |

所有命令返回「未知」用 `null`，不要用 0 或空字符串（前端要分别表达）。

---

## 4. 解析层（核心，约 1000 行）

新增 `backend/src/gateway/context/`（或单文件 `gateway/prompt.rs`，命名先与鼠鼠确认）：

```
context/
  mod.rs        // 对外：parse(protocol, body) -> Prompt；calibrate
  parts.rs      // 八类常量与 PromptPart / PromptItem 结构
  estimate.rs   // tokensOf / rawTokens
  anthropic.rs  // Anthropic Messages 解析
  chat.rs       // OpenAI Chat 解析
  responses.rs  // Responses 解析
  summary.rs    // 工具结果摘要 + 脱敏
```

### 4.1 四协议入口

先读 magpie：`promptOf`（`prompt.go:282`）、`anthropic`（`prompt.go:662`）、`chatCompletions`（`prompt.go:1028`）、`responses`（`prompt.go:824`）、`gemini`（`prompt.go:1102`）。

Jolu 要覆盖的入口（鼠鼠 2026-11-16 确认，Gemini 原生要做）：

| 入口 | 触发场景 | magpie 参考 |
|---|---|---|
| Anthropic Messages | Claude Code 等调用方 | `anthropic` @662 |
| OpenAI Chat | Codex / 第三方工具 | `chatCompletions` @1028 |
| Responses | Codex 新版调用方 | `responses` @824 |
| **Gemini 原生 `/v1beta`** | Gemini CLI 与 Gemini 订阅直连 | `gemini` @1102 |

Gemini 原生请求的 body 结构与前三者差异最大，已核对 magpie `gemini` @1102 的实际字段：

- `systemInstruction.parts[].text` → 系统提示词；
- `contents[].role`（`user` / `model`）+ `parts[]` → 会话轮次；
- `tools[].functionDeclarations[]` → 工具定义（每项取 `name`）；
- `parts[]` 里的 `functionCall` / `functionResponse` → 工具调用与结果；`inlineData` / `fileData` → 文件与图片；`thought` 标记 → 思考段（不计入会话正文）。

实现前再读一遍 `gemini` 与 `gemini_thought_tag_test.go` 确认行为，不要只按本文的字段清单写。

### 4.2 八类识别规则

先读 magpie 的对应函数，再按 Jolu 的字段名实现：

| 类 | 识别依据 | magpie 参考 |
|---|---|---|
| `system` | system/developer 文本里**剥掉**记忆与技能列表后剩下的部分，按开头标签命名（`# Environment` → environment） | `systemText` @450、`harnessName` @438 |
| `tools` | 请求里的 `tools` 数组，每个工具一项；同名合并计次 | `tool` @622 |
| `skills` | 匹配固定句 `The following skills are available for use with the Skill tool:`，其后到空行为列表；`<skills_instructions>` 块整块算技能 | `splitSkills` @420、`skillsApart` @248 |
| `memory` | `Contents of <path> (project instructions)` 类标题、`<system-reminder>` 块、`AGENTS.md` / `CLAUDE.md` 引用 | `splitMemory` @379、`memoryTag` @359、`userText` @464 |
| `files` | 工具调用参数里带文件路径的读取（Read 类），以及图片块 | `filesOfCommand` @502 |
| `results` | 工具返回，按工具名聚合，带次数 | `result` @595 |
| `chat` | 用户与模型的对话轮次；被压缩过的会话按「更早的 N 轮」聚合 | `prompt` @190、`newTurn` @188 |
| `unknown` | 以上都匹配不上的一切 | **magpie 没有，Jolu 新增** |

**互斥去重**（V10-13 规范）：识别为文件或 Skill 后，不再计入工具结果。

**与 magpie 的差异**（实现时别照抄）：
1. magpie 只有七类，Jolu 多 `unknown`；
2. magpie 的 skills 是从 memory 里再拆出来的（`skillsApart`），Jolu 要一开始就分开；
3. magpie 有 `prompt_text.go`（271 行）按需重读正文——**Jolu 第一版不做正文预览**（画板写「正文未采集时不补齐预览」），省掉这部分；
4. Jolu 要记录「来源证据」：每个 file/skill 条目带 `tag`（`project` / `user` / `skills`），供 V10-11 展示。

### 4.3 估算

先读 magpie `tokensOf`（`prompt.go:78`）与 `rawTokens`（@93）：

- ASCII 约 4 字符/token；
- 中文（非 ASCII）约 1 字符/token；
- 图片按固定估算值（magpie 的 `imgPiece` @142），具体值以 magpie 为准。

### 4.4 校准

先读 magpie `calibrate`（`prompt.go:308`）与 `calibrated`（@1189）：

1. 上游返回真实 `input_tokens` 后，`scale = 真实 / 估算`；
2. 所有 part 与 item 等比缩放；
3. 四舍五入的差额补给最大的一块，保证总和等于上游计数；
4. `counted = 1`；
5. **估算与实测的偏差要保留**（Jolu 新增字段或在 UI 提示），不凑平——V10-13 明确要求「偏差明确提示，不凑平」。

---

## 5. 摘要与脱敏（约 200 行）

V10-05 要求每条工具结果有「已脱敏摘要」：

- 提取：命令名（如 `pnpm --dir frontend typecheck`）、退出码、输出行数；
- 脱敏：本机用户名、私有路径替换——**先读 `backend/src/gateway/mask.rs`**，能复用就复用，不要另写一套规则；
- **默认只存摘要**，不存正文；正文预览如果以后要做，必须先脱敏（V10-13「默认保存摘要，正文预览先脱敏；不采集或展示凭证」）。

摘要示例（画板原样）：`检查完成，退出码 0。输出包含 12 行诊断信息；本机用户名与私有路径已替换。`

---

## 6. 采集接入点

先读 `backend/src/gateway/wire.rs`：

- `handle`（约 457 行）是入口，`forward_one`（约 892 行）是单次转发；
- **解析必须在「最终发上游的 body」上做**——订阅适配器会改写请求体（ZCode 着装、Codex tools 提升），拿原始 body 拆会与上游实际不符。具体挂在哪一步，先读代码确认，再和鼠鼠定。

纪律：
- **解析失败绝不能影响转发**：任何 panic / 错误都降级为 `unsupported`，请求照常发；
- **异步落库**：不要阻塞请求路径，用 `spawn_blocking` 或独立队列；
- **只在有会话标识时采集**：拿不到 session_key 的请求不建快照（避免大量无主行）；
- **保留策略（不做随机采样）**：每个请求都解析，但**每个会话只保留最近 20 条快照**，旧的按会话滚动清理。理由：随机采样会让 UI 大量显示「尚未采集」，与 V10-13「如实区分状态」冲突；磁盘不是瓶颈（1000 条 × 几 KB ≈ 几 MB），真正要控的是热路径 CPU，而单次解析 < 5ms 已在验收标准内。
  - 清理与 `gateway_route_log` 的清理写在同一处，避免两套；
  - 若后续出现极端并发（单机 10+ agent 同时跑）再评估「每小时每会话只留一条」的降级策略，作为可选项，不是第一版必需。

---

## 7. 现有可复用（别重写）

| 需求 | 现成的 | 位置 |
|---|---|---|
| 会话聚合 | session_key、links、运行状态 | `gateway/sessions.rs` |
| 请求关联 | request_id、session_platform/id、connection_id | `gateway/route_log.rs` |
| 上游用量 | input/output/cache_read/cache_write + `*_known` | `db.rs` 的 `requests` 表 |
| 口径归一化 | includes_cache / excludes_cache | `usage_stats.rs`（`fresh_input` / `hit_denominator`） |
| 模型窗口 | `context_window` | models.dev 缓存 |
| 脱敏 | 隐私规则 | `gateway/mask.rs` |
| 在途状态（调用工具/正在生成） | 在途快照 | `gateway/activity.rs` |
| 运行状态聚合 | 今日计数 | `gateway_day_overview` / `gateway_today_counts` |

---

## 8. 分期任务

### 第 1 期：解析 + 单测（先不接网关）

- 建 `context/` 模块，实现**四协议**解析（Anthropic / Chat / Responses / Gemini 原生）、八类归类、估算；
- 用**真实请求体样本**做单测（Claude Code / Codex / ZCode / Gemini CLI 各抓一份，脱敏后放测试 fixture）；
- 验收：对样本跑解析，打印八类 token 与占比；**`unknown` 占比超过 20% 说明规则没覆盖，要先补规则**。

### 第 2 期：采集接入 + 校准 + 摘要

- 表与迁移（`db.rs`）；
- `wire.rs` 挂采集点（最终 body）、异步落库、状态机；
- 校准 + 偏差记录；
- 工具结果摘要 + 脱敏。

### 第 3 期：查询命令 + 会话列表

- 5 个 Tauri 命令；
- 与前端 V10-01 对接。

### 第 4 期：快照详情 + 明细 + 工具展开

- 与前端 V10-03 / 05 / 07 对接；
- 矩形图（V10-09）复用同一份 `parts_json` + `items`，前端换可视化即可，后端不用改。

---

## 9. 风险与边界

**必须注意**
1. **未知不能存 0**：`input_tokens` / `cache_read` / `window_tokens` 拿不到存 NULL；0 与未知在前端是两种显示；
2. **格式会漂移**：magpie 注释里全是「Claude Code 3.x 起改成这样」的记录。识别规则是文本匹配，**agent 升级一次就可能失效**——所以 `unknown` 兜底是必需的，且要能观察到它占比上升；
3. **性能**：解析在热路径上，只做 JSON 结构遍历 + 字符串扫描，不做全文正则回溯；
4. **隐私**：不存正文、不存凭证；摘要先脱敏；路径可以存（画板要求显示全路径）。

**明确不做**
- 不做正文预览（`prompt_text.go` 那部分）——第一版不做，画板也写「正文未采集」；
- 不做 WSL 内会话的上下文采集；
- 不把上下文数据写进统计导出或诊断报告；
- 不改 `agent_version` / 探测相关逻辑；
- 不动 `usage_stats` 的既有口径，只复用它的函数。

---

## 10. 验收标准

1. **解析正确性**：**四种协议**各至少 3 个真实样本（Anthropic / Chat / Responses / Gemini 原生），八类合计 = 上游 `input_tokens`（校准后）；`unknown` 占比 < 20%；
2. **状态机**：六种状态各有测试（含 `no_body`、`failed`、`unsupported`）；
3. **未知表达**：`window_tokens` / `cache_read` 为 NULL 时，查询返回 `null` 而不是 0；
4. **不影响转发**：解析层注入 panic 时请求仍正常返回（用测试断言）；
5. **性能**：单次解析 < 5ms（10 万 token 级别的请求体）；
6. **真机**：Claude Code / Codex / ZCode 各跑一次真实请求，V10-01 会话列表能显示「最近上下文 + 缓存占比」，V10-03 能展开八类与明细。

验证记录写到 `docs/verification/2026-11-1X-agent-context-observation.md`。

---

## 11. 提交要求

- 按分期提交（解析 / 采集 / 查询 / 前端对接各一条），不混在一起；
- 提交信息加 `[skip ci]`（未要求发版）；
- 提交前更新 `AGENTS.md` 模块 J 的「现有实现」列，写明上下文观察已实现到哪一期、剩余差距。
