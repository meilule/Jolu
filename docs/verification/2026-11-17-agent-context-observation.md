# V10 Agent 上下文观察 · 第 1–4 期验证

日期：2026-11-17
执行文档：`docs/plans/2026-11-16-agent-context-observation-backend.md`（第 8 节第 1 期）
代码：`backend/src/gateway/context/`
范围：第 1 期（解析层，第 1–6 节）、第 2 期（采集、落库、校准、工具结果摘要，第 7 节）、第 3 期（查询命令与 V10-01 活跃会话，第 8 节）、第 4 期（V10-03～14 会话观察，第 9 节）。

## 1. 样本

每种协议至少 3 个真实请求，共 14 个，放在 `backend/src/gateway/context/testdata/`。来源与生成方法见该目录的 `README.md`。

| 协议 | 样本 |
|---|---|
| Anthropic Messages | Claude Code 2.1.326 × 3，magpie `claude-code.json` × 1 |
| OpenAI Responses | Codex CLI 0.169.0 × 3，magpie `codex.json` × 1 |
| OpenAI Chat | OpenCode 1.19.5 × 3（含 1 个起标题的小请求） |
| Gemini 原生 | Gemini CLI 0.68.0 × 3 |

本机样本的抓取方式：

- 每个 CLI 在隔离主目录里运行，连本机回环的假上游。
- 每个 CLI 都走两轮工具调用：先用读文件工具读 `README.md`，再用 shell 执行 `cat src/cart.ts`，最后回一句文字。
- 抓到的请求体经 `context-sample-sanitize.py` 等长填充、替换路径后才进入仓库。脚本最后检查本机用户名、Git 身份和仓库目录名，都没有残留。

ZCode 没有单独抓：它是桌面应用，要抓就得改它的配置。它走 Anthropic Messages 协议，由 Claude Code 的样本覆盖。

## 2. 自动化测试

命令：`cargo test --manifest-path backend/Cargo.toml --lib gateway::context`

结果：25 项通过。

| 测试 | 核对内容 |
|---|---|
| `samples_add_up_and_unknown_stays_small` | 14 个样本都能解析：各类之和等于总量，每类各项之和等于该类，unknown 占比低于 20%，每类不超过 24 项，类别顺序固定 |
| `claude_code_sample` | 用户与项目的 CLAUDE.md 分别记为 user / project；其余提醒记为 reminders；技能单独一类；Read 读的记为 read，Bash `cat` 读的记为 shell；billing 与 prompt 记为 system |
| `codex_sample` | AGENTS.md 按 `--- project-doc ---` 拆成用户的与项目的；`<skills_instructions>` 是技能；exec_command 的 `cat` 记为文件；prompt、permissions instructions、environment_context 是 system |
| `opencode_sample` | `Instructions from:` 的文件是 memory（`.config` 下的是用户的）；`<available_skills>` 是技能；read 与 bash `cat` 读的都是文件 |
| `gemini_sample` | `--- Context from: … ---` 的 GEMINI.md 是 memory；`<available_skills>` 是技能；`<session_context>` 是 system，不算用户说话 |
| `magpie_claude_code` / `magpie_codex` / `split_skills_ends_at_blank_line` / `codex_mcp_namespace` / `chat_completions` / `gemini_matches_response_by_name` / `responses_held_and_not_a_body` / `calibrated_adds_up_and_keeps_estimate` / `files_of_command_cases` | 移植自 magpie `prompt_test.go`，断言与原测试一致 |
| `gemini_thought_counts_as_chat` | Gemini 的思考段计入对话（与 magpie 一致，见执行文档核对第 3 条） |
| `unrecognized_goes_to_unknown` | 四种协议里认不出的块、条目、角色都记到 unknown，并带上类型名或角色名 |
| `overflow_is_summed` | 超过 24 项时：非对话类把最小的汇总成 `more`，对话类把最早的汇总成 `earlier` |
| `codex_agents_md_split` / `memory_tags` | Jolu 补充的规则 |
| `odd_texts_never_panic` | 多字节字符紧贴标记、标签不闭合、只有标记等文字不 panic |
| `mutated_bodies_never_panic` | 每个样本做 120 次变异：随机把一个节点换成 null、数字、字符串、空数组、空对象等，解析都不 panic；截断后的请求体返回 `None` |
| `guarded_turns_panic_into_none` | 调试构建下 panic 被兜住，返回 `None` |
| `parse_large_body_fast` | 见第 4 节 |

全量后端测试：`cargo test --manifest-path backend/Cargo.toml`，1450 项通过，51 项忽略，0 项失败。

## 3. 各样本八类占比（估算，未校准）

| 样本 | 总量 | system | tools | skills | memory | files | results | chat | unknown |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude-code-2.1.326-3 | 22542 | 30.3% | 56.3% | 7.2% | 5.5% | 0.4% | 0 | 0.2% | 0 |
| magpie-claude-code | 13886 | 45.3% | 49.3% | 0 | 4.3% | 0.5% | 0.1% | 0.6% | 0 |
| codex-0.169.0-3 | 10767 | 42.1% | 42.6% | 13.5% | 0.6% | 0.9% | 0 | 0.4% | 0 |
| magpie-codex | 19294 | 35.1% | 50.4% | 13.5% | 0.2% | 0.6% | 0 | 0.3% | 0 |
| opencode-1.19.5-3 | 16023 | 13.8% | 32.5% | 0.9% | 51.9% | 0.7% | 0 | 0.3% | 0 |
| gemini-cli-0.68.0-3 | 12819 | 51.9% | 44.0% | 2.4% | 0.7% | 0.6% | 0 | 0.4% | 0 |

其余 8 个样本是同一工具较早的请求，占比相近；完整输出用 `-- --nocapture` 运行 `samples_add_up_and_unknown_stays_small` 查看。

OpenCode 的 memory 占一半以上，是因为它读入的项目指令是本仓库根目录的 AGENTS.md，不是演示项目自己那份。演示项目放在本仓库的 `output/` 下，虽然单独 `git init` 过，结果也一样；原因没有深究。这个样本正好覆盖「项目指令文件很大」的情形。

## 4. 性能

请求体：把 Claude Code 样本的对话重复到约 56 万字节，估算约 12.8 万 token。每次解析耗时：

| 构建 | 单次解析 |
|---|---|
| 调试构建（`cargo test`） | 约 10.5ms |
| 发布优化（`opt-level = "s"`，关闭 LTO、`codegen-units = 16`，`cargo test --release`） | 约 2.9ms（20 次平均），低于 5ms 门槛 |

门槛「低于 5ms」只在发布构建里断言；调试构建只打印耗时。

## 5. 与执行文档的出入

见执行文档第 8 节「执行前核对」第 1–10 条，其中第 9、10 条是本次实现时新发现的：

- 第 9 条：发布版是 `panic = "abort"`，`catch_unwind` 兜不住，解析层本身必须不 panic。
- 第 10 条：Gemini CLI、OpenCode、Codex 新版格式需要 Jolu 补规则。

## 6. 未验证

- 校准只做了单元测试，没有拿上游的真实 `input_tokens` 对照；这一项属于第 2 期。
- ZCode 的真实请求没有抓，原因见第 1 节。
- 真机验收（V10-01 / V10-03 展示）属于第 3、4 期。

## 7. 第 2 期：采集、落库与校准（2026-11-17）

### 7.1 自动化测试

| 测试 | 核对内容 |
|---|---|
| `context::capture::tests::parse_then_usage_calibrates` | 解析先完成：先写估算的 `ready`，用量到了按实测校准。核对四点：各类之和等于实测量；`estimated` 保留原估算；缓存命中与连接写上；模型窗口与内置快照一致。逐条工具结果有 Read 和 Bash 两条，不采正文。重复回报用量只认第一次 |
| `context::capture::tests::usage_before_parse_still_ends_calibrated` | 用量先到、解析后完成，最后仍是校准后的 `ready` |
| `context::capture::tests::states_and_unknowns` | 不是 JSON 的请求体记 `unsupported`；空请求体记 `no_body`；上游失败时输入、缓存、窗口都是 NULL，`counted = 0`，不写 0 |
| `context::capture::tests::meter_reads_upstream_usage` | 计量层按原字节透传。从 Anthropic 流里读出输入 1000（含缓存 900）、输出 42；上游返回 429 时记为没有用量 |
| `context::capture::tests::prune_keeps_recent_per_session` | 每个会话留最近 20 条，删掉的是最早的 |
| `context::summary::tests::*` | 本机目录换成 `~`，用户名、凭证被遮掉，无关命令原样保留；能识别三种退出码写法；摘要文字正确 |
| `gateway::server::tests::context_snapshot_is_captured_and_calibrated_end_to_end` | 走真实网关链路：Claude Code 带会话头发请求，经假上游回用量。核对：快照校准到 40100（其中缓存 40000）；连接为 `c-kimi`；`request_id` 与路由流水一致。不带会话头、请求体也没有 metadata 的请求照常转发，不建快照 |

全量后端测试：见 7.3。

### 7.2 实现中发现的问题

- 端到端测试第一次没过，原因在测试本身：「不带会话」那次请求仍然建了快照。Claude Code 的 `metadata.user_id` 里本来就带 session_id，网关会从请求体认出会话，这是正确行为。测试改成去掉 metadata 再发。
- 本机可提交内存只剩 3–6GB（本机其他程序合计占约 71GB）。默认调试配置编译测试目标时 rustc 分配失败崩溃（`STATUS_STACK_BUFFER_OVERRUN`）。本阶段测试改用单独的 target 目录，并关闭调试信息和增量编译（`CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_INCREMENTAL=0`）才跑通。代码本身没有问题。

### 7.3 全量测试

- 全量测试：`cargo test --manifest-path backend/Cargo.toml`，按 7.2 的低内存配置、`-j 1` 运行。结果：1465 项通过，51 项忽略，0 项失败。
- 编译库目标：只有 `gateway/wiring.rs` 里 `saved_default` 一条警告，是同事已有的代码，与本期无关。

### 7.4 未验证

- 真机：还没有在桌面实例里让真实 CLI 经网关跑一次、查看快照。第 3 期加了查询命令后再做。
- 用 models.dev 缓存查模型窗口只在单元测试里走了内置快照这条路；读真实缓存文件的那条路没有测到。

## 8. 第 3 期：查询命令与 V10-01 活跃会话（2026-11-17）

### 8.1 后端

| 测试 | 核对内容 |
|---|---|
| `context::query::tests::sessions_take_latest_with_route_and_last_ready` | 会话列表：每个会话只取最新一条。核对：最近一次 ready 的时间与保留条数正确；失败的那次输入和缓存为 null；来源账号名取自连接表；流水里没有时链路为 null。换候选时，历史里的链路取这次请求最后一条流水 |
| `context::query::tests::snapshot_items_and_tools` | 快照详情：组成之和等于校准后的总量，按工具聚合出两条。其他会话拿不到这条快照。明细占比之和为 1，类别筛选、名称搜索正确。逐条工具结果能按工具过滤 |
| `context::query::tests::unsupported_snapshot_has_no_parts` | unsupported 的快照没有组成，明细为空表 |

后端全量测试：见 8.4。

### 8.2 前端纯逻辑

`node --test frontend/src/lib/contextSessions.test.mjs`：7 项通过，覆盖以下内容：

- 上下文单元格：实测 / 窗口，阈值标红；窗口未知；只有估算；暂无请求、尚未采集、分析中、失败（含上次成功时间）、不支持。
- 缓存列：未提供、0、占比。
- 状态、项目名、来源账号。
- 合并与排序：运行中在前，只有快照的会话也列出。
- 搜索、状态筛选与「需要关注」。

前端全部纯逻辑测试 `node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs`：463 项通过。`pnpm --dir frontend typecheck` 通过。

### 8.3 浏览器预览验收（对照 V10-01 / V10-02）

`scripts/acceptance/context-overview-preview.e2e.mjs`：挂载真实 `GatewayView`，接口全部用合成数据，不连桌面。

- 浅色、深色各跑一遍，核对以下内容：
  - 标题、最近 12 小时趋势、需要关注里的「Claude Code 上下文已用 84%」；
  - 「168K / 200K · 84.0% · 162K · 96.4% · 正在生成」那一行用危险色进度条；
  - 「74.2K / 400K · 18.6% · 60.3K · 81.3%」；
  - 「631K / 容量未知 · 未提供」那一行没有进度条；
  - 没有经过路由的会话写「暂无请求」；
  - 表尾口径说明。
- 搜索「文档」、状态筛「接近上限」和「尚未采集」，结果正确。
- 页面没有发出合成数据以外的原生调用，控制台无报错。
- 截图在 `output/acceptance/context-overview/`，已与画板截图对照：结构、列、文案与颜色变量一致。

两处说明：

- 画板示例写的是「60.3K · 81.2%」，按实际数字计算 60.3K ÷ 74.2K = 81.27%，四舍五入为 81.3%。
- 本机 Edge 无头启动会直接退出，脚本改为优先用 Chrome。

### 8.4 全量测试

后端全量测试按 7.2 的低内存配置运行：1468 项通过，51 项忽略，0 项失败。

### 8.5 未验证

- 真机：没有在桌面实例里让真实 CLI 经网关跑一次，再看活跃会话表。

## 9. 第 4 期：会话观察（V10-03～V10-14，2026-11-17）

### 9.1 自动化测试

- 后端 `context::tests::loaded_skills_have_evidence` 核对 Skill 加载证据：
  - 读 `SKILL.md`、调用 Skill 工具，都记为 Skills（`loaded`）；
  - 不再计入文件、工具结果；
  - 普通文件仍然记为文件；逐条明细里这几次调用的类别也正确。
- 前端 `node --test frontend/src/lib/contextObservation.test.mjs`：10 项通过，覆盖：
  - 八类顺序与颜色；
  - 明细表聚合与来源证据；
  - 筛选、搜索、排序、计数文字；
  - 单次返回的顺序与「先显示 3 条」；
  - 两张占用卡（实测、估算、容量未知、缓存未提供与 0）；
  - 偏差提示、运行摘要、链路；
  - 跟随与暂停（选旧请求暂停、新请求只计数、恢复才切换、已选的那条被清理时退回最新）；
  - 历史下拉文字；
  - 矩形图面积与不重叠。
- 前端全部纯逻辑测试 473 项通过，`pnpm --dir frontend typecheck` 通过。
- 后端全量测试：见 9.3。

### 9.2 浏览器预览验收

运行的脚本是 `scripts/acceptance/context-observation-preview.e2e.mjs`：挂载真实 `GatewayView`，接口全部用合成数据。浅色、深色各跑一遍，依次核对：

- 从概览「查看上下文」进入；链路、运行摘要、两张卡、八类图例、明细表各行与来源证据。
- 展开 Bash 工具结果：
  - 默认只有最近 3 条；
  - 「查看全部 5 条」后出现第一条；
  - 带路径的那条标「已脱敏摘要」；
  - 展开后自动暂停跟随。
- 「恢复跟随最新」后展开收起。
- 矩形图：点「文件」图块筛选，显示「已选：文件 · 112.4K（17.8%）」，列表只剩文件。
- Skills 筛选：出现加载证据那一项和技能列表。
- 选历史请求 #125（失败、只有估算）：
  - 显示「● 请求失败」「590K / 1M」「本地估算」；
  - 模拟新增一次请求后，显示「已有 1 次新请求，内容保持不变」，所选请求不变；
  - 恢复后切到 #127。
- 不支持分析的来源显示「此来源暂不支持内容分析」；没有快照的会话显示「暂无请求」的完整说明。
- 没有合成数据以外的原生调用，控制台无报错。

截图在 `output/acceptance/context-observation/`，与 V10-03 / 05 / 07 / 09 画板对照，结构、文案与颜色变量一致。另外重跑了第 3 期的 `context-overview-preview.e2e.mjs`，同样通过。

### 9.3 全量测试

后端全量测试按 7.2 的低内存配置运行：1468 项通过，1 项失败，51 项忽略。

失败的是 `handoff_facts::tests::redact_skips_values_it_does_not_mask`：

- 这个测试检查脱敏耗时是否线性增长。全量并行运行时机器负载高，它偶发超出耗时上限。
- 单独重跑 `handoff_facts` 的 26 项全部通过。
- 本期没有改动这个模块，失败与本期改动无关。
