# P4 跨组件一致性审计（P4-1 / P4-2）与 P4-3 验收清单

- 日期：2026-11-24
- 状态：只读审计完成，未改代码；P4-3 只写清单、未执行
- 配套计划：[产品对齐执行计划](../plans/2026-11-23-product-alignment-execution-plan.md)（P4-1、P4-2、P4-3，§1 完成标准第 5 条）
- 共享契约：`backend/src/data_contract.rs`（`Coverage`）
- 审计基线：提交 `c8bf1f8`。审计期间工作区有其他任务的未提交改动，涉及 `backend/src/data_contract.rs`、`db.rs`、`usage_stats.rs`、`gateway/route_log.rs`、`gateway/wire.rs`、`gateway/route.rs` 和 `frontend/src/lib/api.ts`、`gatewayApi.ts`。其中已看过的部分是 P1-4 `Provenance`：`LiveUsage.today_provenance`、`Overview.provenance`、`DayOverview.provenance`，只增加了来源元数据，没有改变下文缺陷涉及的计算逻辑。本文不把这些改动当作已交付能力，只在相关处注明；`wire.rs` 的改动没有审。
- 引用格式：`文件 › 函数`。行号会随并行改动漂移，所以只写函数名。

结论先行：

1. 四个入口的「今天」都用本机时区零点，这一点一致，没有发现 UTC 与本地时区混用。
2. 不一致主要在三处：Token 口径（新鲜 Token 与含缓存总量）、计数单位（按尝试还是按请求）、`requests.platform` 的含义（调用方工具还是上游账号平台）。另外，同一请求在不同表里取的是不同时刻的时间戳。
3. 确定缺陷 14 条（D1–D14），疑似 4 条（S1–S4）。会话工具范围的后端分支和 AGENTS.md 基本一致，但前端「压缩会话」入口和 ZCode 的不可用说明对不上，Kimi / Pi / dsh 会话的平台筛选也不可达（见 §3）。

## 修复跟踪（2026-11-24，主线程逐条核实后处理）

| 编号 | 核实 | 处理 |
|---|---|---|
| D1 今日经网关按尝试计 | ✅ 属实 | 已修：`gatewayActivity.ts` `onEnd` 按 `request_id` 只计最后一次尝试；快照补加用 `countByRequest` 折叠；测试 2 条（去掉撤销上一次计数即失败） |
| D3 今日经网关不标截断 | ✅ 属实 | 已修：`route_log::truncated_since`（与概览 `partial` 同一判定），`GatewayTodayCounts.partial`；胶囊「今日经网关」截断时显示「≥N 次」（`todayTotalText`，无画板，最小版，待确认）；测试 `today_counts_report_truncation_like_the_overview` 与前端 1 条 |
| D4 趋势与 P50 最多 100 行 | ✅ 属实 | 已修：`route_log::page` 上限改为 `MAX_PAGE_SIZE = 500`；测试 `a_page_can_hold_the_overview_sample_of_240_rows` |
| D8 「较昨天同时段」比的是昨天最后 N 小时 | ✅ 属实 | 已修：`usage_stats::previous_window` 按整天平移、长度不变；测试 `todays_comparison_uses_the_same_hours_of_yesterday` |
| D5 原生平台今日 Token 含缓存 | ✅ 属实 | 已修（鼠鼠 2026-11-24 同意按推荐）：`today_fresh_tokens` 原生分支按 `input_semantics` 取新鲜值，与增量同口径；语义未知或分项缺失的行沿用总量（上界）；测试 `native_today_tokens_use_the_fresh_measure`。ZCode / Qoder 等平台灵动岛今日数字会明显变小，属预期 |
| D9 两个页面「上期」不同 | ✅ 属实 | 已修：`route_log::day_overview_until` + `Range::prior_end`，截止到此刻的范围上期只算到同一时段；前端文案改「昨天同时段」「前 7 天同时段」；测试 `the_prior_period_stops_at_the_same_time_of_day` |
| D10 同一次调用记两遍 | 待复现 | 依赖调用方工具是否必写本机日志，放 P4-3 真实环境核对 |
| D2、D6、D7、D11–D14 | 未处理 | 见 §2 各条；涉及界面口径或展示的需对应画板 |

---

## 1. P4-1 对照表

行是语义维度，列是四个入口。每格先写实际口径，再写代码出处。

| 维度 | 灵动岛 | 用量 | 会话 | 路由 |
|---|---|---|---|---|
| **连接身份** | 绑定连接：`useIslandWindowModel.ts`（`islandActiveConnection`：明确选择优先，否则取本平台第一个 connected）。正在运行的连接：`db.rs › live_usage` 调 `attribution::session_attribution`，分 `exact` / `mapped` / `platform` 三档，在 `SESSION_ATTRIBUTION_WINDOW_MS` 窗口内推断。「今日按来源」只认 `requests.connection_id`：`usage_stats.rs › source_breakdown` | 账号筛选只认 `requests.connection_id` 的精确值，NULL 归入 `__unattributed__`：`usage_stats.rs › where_clause`。调用明细另附 `attribution` 视图，只用于展示：`usage_stats.rs › calls` | 只有全部请求都属于同一连接时才有值，否则为 None：`session_threads.rs › list`（`MIN/MAX(connection_id)` 与 `SUM(connection_id IS NULL)`） | 选路时选中的连接写入 `gateway_route_log.connection_id`；被拒时为 None：`gateway/wire.rs › record_route` / `linked` |
| **平台 id** | 平台目录 id（claude / codex / zcode…）：`live_usage(platform)` 直接按 `requests.platform` 过滤 | 同上，必须属于 `platforms::known` 的 10 家：`usage_stats.rs › StatsFilter::validate` | `requests.platform` 加外部来源 kimi / pi / dsh：`session_threads.rs › list`。命令层只放行 `platforms::known`：`commands/usage_sessions.rs › sessions_list` | 两套命名并存：`agent` 用接线 id（claude-code…），`session_platform` 用会话平台（claude…）。映射写在 `gateway/sessions.rs › agent_of_platform` |
| **`requests.platform` 的实际含义** | 本机采集行表示调用方工具。`gateway-subscription` 行表示上游账号平台：`gateway/subscription.rs › record_usage` 写 `platform: entry.platform` | 同左，两种含义混在一起统计 | 同左 | 不使用 `requests` |
| **会话身份** | `requests.session_id`，ZCode 带 `native:zcode:main:` 前缀：`native_sources.rs`（`session_id: format!("{}:{session}")`） | `requests.session_id`，用于高消耗会话：`usage_stats.rs › top_sessions` | `(platform, session_id)`；ZCode 子会话被过滤：`session_threads.rs › list`（`zcode_child_session`） | 只取工具原生会话头：Claude / Codex（根会话）/ OpenCode（父会话，加 `opencode:` 前缀），其他工具为 None：`gateway/wire.rs › session_identity` |
| **请求身份** | 实时事件用进程内的 `request_id`，重启后从头计。事件里没有 `request_uid`：`gateway/activity.rs › RouteStart` / `RouteEnd`。今日计数按 `request_uid` 归并：`route_log.rs › counts_since` | `(source, dedup_key)` 唯一。Claude 用 `message.id`，Codex 用 `response_id`：`collector.rs`；网关订阅用 `连接id长度:连接id:响应id`：`subscription.rs › record_usage` | 不单列请求身份；`requests` 计数取 `COUNT(*)` | `request_uid` 跨进程持久，同一请求的多次尝试共用：`activity.rs › reserve_request_uids` / `UidBlock`。旧行按 id 单独成组：`route_log.rs › day_overview` |
| **时区** | 本机时区：`db.rs › period_range` → `start_of_today_at`（`chrono::Local`）。前端跨零点清空用 JS 本地时间：`useGatewayActivity.ts` | 前端用 JS 本地日期：`usageStats.ts › rangeToMillis`。后端分桶用 `chrono::Local`：`usage_stats.rs › buckets` | 前端 JS 本地零点：`sessionThreads.ts › sinceFor` | `chrono::Local`：`route_log.rs › midnight_ms`、`commands/routing.rs › gateway_today_counts` |
| **自然日边界** | `[今天 00:00, 现在)`：`period_range_at("today")` | `[开始日 00:00, min(结束日次日 00:00, 现在))`：`rangeToMillis` | 「今天」表示 `last_ts ≥ 今天 00:00` 的会话，按会话过滤，不按请求过滤：`session_threads.rs › list`（`f.since`） | 本期 `[今天 00:00, 明天 00:00)`，上期是昨天全天：`Range::bounds` / `end_exclusive`。「今日经网关」只有下界 `first_ts ≥ 今天 00:00`：`counts_since` |
| **用哪个时间戳** | `requests.ts`，取值见下一行。增量事件用 `usage_events.ts`，等于记录时间：`db.rs › insert_records_into` | `requests.ts` | `first_ts` / `last_ts` 分别是 `MIN(ts)` / `MAX(ts)` | 流水时间取 `record_route` 被调用的时刻：流式请求是拿到响应头时，非流式是拿到完整响应后（`wire.rs › record_route`）。多次尝试按首次尝试的时间归日（`day_overview` / `counts_since` 的 `MIN(ts)`），趋势图按最终尝试的时间归入小时（`gatewayOverview.ts › hourlyTrend`）。上下文快照用请求开始时间（`context/capture.rs › run`） |
| **`requests.ts` 的来源** | ZCode：`started_at`（`native_sources.rs`，注释写明按开始时间归日）。Claude / Codex：JSONL 记录时间，更新时 `ts = MAX(ts, 新值)`（`db.rs › insert_records_into`）。网关订阅：流结束时的 `Utc::now()`（`subscription.rs › record_usage`）。Qoder：`gmt_create`（`qoder_usage.rs`）。OpenCode：`time_created`（`opencode_usage.rs`） | 同左 | 同左 | — |
| **来源** | 本机记录（`requests`）+ 网关流水（今日经网关）+ 官方额度（`useQuota`）+ 官方组织用量（`quota.rs › anthropic_api_usage` / `openai_api_usage`） | 本机记录 + 网关订阅记账，混在同一张表。工作区的 `Overview.provenance` 已经区分 `local_record` / `gateway_log` / `mixed`，尚未提交，前端也还没用 | 本机记录（`requests` + 源文件 / ZCode sqlite） | 网关流水 + 上下文快照。会话 Token 借用会话线程的数据，文件头注明「可能含经其他入口的消耗」：`gateway/sessions.rs` |
| **Token 口径** | 「本机今日 Token」：Claude 取输入 + 输出，Codex 先扣缓存，**原生平台直接用 `total_tokens`**：`db.rs › today_fresh_tokens`。本轮增量用 `delta_fresh`：`live_usage`。今日按来源用 `total_tokens`（含缓存）：`source_breakdown` | 「总 Token」是 `total_tokens` 之和（含缓存）：`usage_stats.rs › split` / `overview` | 四个分项：新鲜输入、缓存读取、缓存写入、输出。按 `input_semantics` 换算：`session_threads.rs › list` | 不统计 Token。上下文快照只记单次请求的输入 / 输出 / 缓存，不是累计值：`context/query.rs` |
| **未知与零** | `today_tokens` 加 `today_token_coverage`（`Coverage::from_counts`）：没有记录时，原生平台给 None，其他平台给 0；全部未知时给 None：`db.rs › today_fresh_tokens` / `today_token_coverage`。前端分 known / partial / missing / empty 四种显示：`islandPresentation.ts › todayTokenView` | 任一请求缺某个分项，该分项就整体未知，另给已知小计：`sum_known` / `known_subtotal`；原生平台空范围给 None：`empty_value`。**高消耗会话把未知当 0 累加，并隐藏总量为 0 的会话**：`top_sessions` | 四个分项任缺一项就不给总数：`gateway/sessions.rs › tokens`；顶部统计排除未知会话并给出计数：`sessionThreads.ts › sessionTotals` | 成功率在不确定时返回 null：`gatewayOverview.ts › successRate`；流水里查不到的链路字段为 null：`context/query.rs` |
| **失败 / 取消** | 实时事件结果只有 ok / rejected / failed。流式请求事后被改判为 cancelled 时，不会补发事件：`route_log.rs › CancelWatch::watch` | `failed` / `cancelled` / `unknown` 三者互斥：`usage_stats.rs › FAILED_SQL` 等 | 会话状态 running / done / failed / recent / usage_only：`session_threads.rs › list` | 按最后一次尝试判定结果，`cancelled` 归入错误：`day_overview` / `counts_since` |
| **过期 / 采集失败** | 额度有 `stale` 状态，提示「额度可能已过期」；采集源失败时会话标「未知」：`useIslandWindowModel.ts`（`sourceFailed`） | **没有读取采集状态**（`useCollectionStatus` 只被灵动岛使用） | **没有读取采集状态** | 显示「每 5 秒更新」和更新时刻：`OverviewPanel.tsx` |
| **覆盖范围 / 截断** | 今日经网关：流水只保留 1000 行，但**不标是否截断**：`commands/routing.rs › GatewayTodayCounts` | 只代表已采集的记录（工作区里是 `as_recorded`，未提交） | 只要会话的最后活动在范围内，就统计它的全部历史 | 用 `partial` / `identity_incomplete` 标注：`day_overview`。趋势和 P50 **最多只有 100 行**，见 D4 |

展示差异（不算缺陷，记录在此）：

- 未命名会话的回退文案不统一：灵动岛写「未命名会话」（`db.rs › live_usage`），会话页和用量页写「会话 xxxxxxxx」（`sessionThreads.ts › sessionTitle`、`usage/OverviewPanel.tsx`）。
- 路由「活跃会话」取最近 24 小时滚动窗口（`gateway/sessions.rs › RECENT_WINDOW_MS`），不按自然日。

---

## 2. P4-1 不一致项

### 2.1 确定缺陷

#### D1 今日经网关：前端按尝试计数，后端按请求计数

- **代码出处**：
  - 前端：`frontend/src/lib/gatewayActivity.ts › onEnd` 每收到一个结束事件就调一次 `countResult`；`hooks/useGatewayActivity.ts` 在快照之后继续累加。
  - 后端：`gateway/wire.rs › record_route` 每次尝试都会 `emit_end` 并写一行流水，测试 `a_failing_member_hands_over_to_the_next_and_the_log_shows_both_attempts` 证实一次请求写两行。`gateway/route_log.rs › counts_since` 按 `request_uid` 归并，并取最后一次尝试的结果。
- **触发场景**：分组使用 order 策略，成员 A 返回 503，转移到 B 后返回 200。
- **影响**：胶囊显示「成功 1、失败 1」，路由概览显示今日 1 次请求、0 次错误。重开灵动岛后数字又对上，所以用户看到的读数会跳变。
- **修法**：`RouteEnd` 增加 `request_uid` 字段；或者让 `gatewayActivity.ts` 的 `ActivityState` 维护 `counted: Map<request_id, result>`，同一个 `request_id` 再次结束时先撤销旧结果，再计入新结果，与「最后一次尝试」的口径一致。
- **单测思路**：在 `gatewayActivity.test.mjs` 中，对同一个 `request_id: 7` 先调 `onEnd(...result:'failed')`，再调 `onEnd(...result:'ok')`，期望 `today = {ok:1, failed:0}`。修复前实际结果是 `{ok:1, failed:1}`。

#### D2 被取消或被改判的请求：实时事件与流水结果不一致

- **代码出处**：
  - 情形 A：客户端在拿到响应头之前断开。`gateway/activity.rs › EndGuard::drop` 只补发一个 `failed` 结束事件，不写流水。之后的 `record_route` 不会执行，因为整个 future 已被丢弃。
  - 情形 B：流式响应中途断开，或上游中途报错。`route_log.rs › CancelWatch::watch` 和 `mark_unfinished` 把已写好的 `ok` 行改判为 `cancelled` / `failed`，但不发任何事件，而前端在拿到响应头时已经按 `ok` 计数（`gatewayActivity.ts › onEnd`）。
- **影响**：
  - 情形 A：灵动岛失败数加 1，路由概览和 `counts_since` 完全看不到这次请求。路由请求总数偏少。
  - 情形 B：灵动岛的成功数偏多，直到重开灵动岛。
- **修法**：
  - `EndGuard` 持有 `Db`，在 `drop` 里调 `route_log::queue_record` 写一行 `cancelled`，让流水与事件对齐。
  - `CancelWatch::watch` 改判成功后，补发一个带同一 `request_id` 的结束事件（新事件名或 `amended: true`），由 D1 的去重逻辑替换旧结果。
- **单测思路**：
  - 在 `activity.rs` 的测试里用 `test_support::collector()` 构造 `EndGuard` 后直接 drop，断言收到一个 `failed` 结束事件，同时 `gateway_route_log` 的 `COUNT(*) = 0`。
  - `route_log.rs` 已有测试 `a_queued_row_is_flipped_to_failed_when_the_upstream_errors`，在它基础上断言改判时没有发出事件。

#### D3 今日经网关不标截断

- **代码出处**：
  - `route_log.rs › counts_since` 的注释写着「当天超过 1000 次时这是下限，调用方据此标注」，但 `commands/routing.rs › GatewayTodayCounts` 没有 `partial` 字段，`components/island/CapsuleGateway.tsx` 直接显示「{total} 次」。
  - 对比：同一份流水，`day_overview` 有 `partial` 字段，路由概览会标「（记录不完整）」。
- **影响**：当天经网关超过约 1000 次时，灵动岛显示的是一个确定数字，路由页则标为不完整。
- **修法**：把 `day_overview` 里的截断判定（`total >= KEEP_ROWS && oldest > 起点`）抽成 `route_log::retention_partial(conn, since)`，`GatewayTodayCounts` 增加 `partial: bool`，胶囊在截断时显示「≥N 次」。
- **单测思路**：写入 `KEEP_ROWS` 行，时间都在今天，最老一行晚于今天零点。断言 `day_overview(...).partial == true`，再断言新函数对同一个 `since` 返回 true。

#### D4 路由概览的趋势和 P50 最多只用 100 行，前端以为是 240 行

- **代码出处**：`views/GatewayView.tsx` 设 `RECENT_ROWS = 240` 并调用 `routeLogPage(1, 240, null)`，但后端 `route_log.rs › page` 把 `page_size.clamp(1, 100)` 截到 100。趋势图、成功 / 失败计数和响应头分位都由这批行推导：`gatewayOverview.ts › hourlyTrend` / `headerLatency`、`OverviewPanel.tsx`。AGENTS.md 模块 J 写的「240 行推导趋势」与代码不符。
- **影响**：最近 12 小时请求超过 100 次时，「请求趋势」的成功 / 失败之和明显小于同屏「今日请求」（后者来自 `day_overview` 全量统计），而且没有任何标注。P50 / P95 只反映最近 100 行。
- **修法**：后端新增 `route_log::hourly_counts(conn, since, until)`，按 `request_uid` 归并并取最后一次尝试，用 SQL 按小时分组；另加 `header_latency(conn, since, p)`。前端不再从分页数据推导统计。如果暂时保留现状，至少把 `RECENT_ROWS` 改为 100，并在卡片上写「最近 100 条流水」。
- **单测思路**：写入 300 行，断言 `page(&conn, 1, 240, None).rows.len() == 100`。

#### D5 灵动岛：原生平台「本机今日 Token」含缓存，本轮增量不含缓存

- **代码出处**：`db.rs › today_fresh_tokens` 的原生平台分支（`platforms::native`）直接 `SUM(total_tokens)`。`source_store.rs › write` 算 `delta_fresh` 时按 `input_semantics` 扣掉缓存，`db.rs › live_usage` 的增量取 `COALESCE(delta_fresh, delta_total)`。函数注释自称「新鲜口径」，原生平台分支并不满足。
- **触发场景**：Qoder 的 `input_semantics = includes_cache`，`total = prompt + completion`（`qoder_usage.rs`）。能力矩阵记录的真实账本里，缓存约占提示的 83%（`docs/specs/2026-11-23-capability-matrix.md`）。
- **影响**：
  - 对 Qoder / ZCode 这类原生平台，「本机今日 Token」和「今日累计写入」兜底值（`CapsuleIsland.tsx`）比 Claude / Codex 的同名指标多算了缓存读取，同一个灵动岛上的本轮增量与今日累计口径不同。
  - 第 2 节的 D6 会因此在同一张卡片上出现两套口径。
- **修法**：原生平台分支改用 `session_threads.rs › list` 里那段 `CASE input_semantics` 公式（`includes_cache` 时减去 `cache_read`，`excludes_cache` 时直接取输入），`unknown` 语义的行按未知计入覆盖范围。或者把 `fresh` 抽成 SQL 片段常量，三处共用。
- **单测思路**：用 `source_store::write` 写一条 qoder 记录，`input=1000, cache_read=900, output=10, total=1010, includes_cache`，ts 为今天。断言 `today_fresh_tokens(conn, "qoder") == Some(110)`，修复前返回 `Some(1010)`。

#### D6 胶囊卡片上「今日按来源」与「本地今日 Token」口径不同

- **代码出处**：`usage_stats.rs › source_breakdown` 用 `total_tokens`（含缓存），而且没有加 `EXCLUDE_DEGENERATE_SQL`。`db.rs › today_fresh_tokens` 用新鲜口径，并排除退化行。两者同时显示在 `CapsuleIsland.tsx` 的展开卡片里（「今日按来源」区块和「本地今日 Token」行）。
- **影响**：Claude 的缓存读取通常远大于输入加输出，各来源行相加会远大于底部的「本地今日 Token」。用户无法对账。
- **修法**：`source_breakdown` 增加新鲜 Token 列，公式与 `today_fresh_tokens` 相同，并加上退化行排除；灵动岛改用新列。如果要保留含缓存的数字，就把行标签写成「含缓存」。
- **单测思路**：写入一条今天的 claude 记录，`connection_id='c1'`，`input=10, output=5, cache_read=1000, total=1015`。断言 `source_breakdown(...).rows[0].tokens == Some(1015)`，而 `today_fresh_tokens(conn,"claude") == Some(15)`。

#### D7 从灵动岛跳到「今日用量明细」后，数字对不上

- **代码出处**：
  - 灵动岛的本地今日 Token 是整个平台的数据，不按连接筛选：`db.rs › today_fresh_tokens`。
  - 跳转时会带上连接：`CapsuleIsland.tsx` 的 `api.openMainUsage(data.platform, data.connectionId)`，其中 `data.connectionId = active?.id`（`useIslandWindowModel.ts`）。`views/MainPanelWindow.tsx` 把它转成 `accounts: [connection_id]`。
  - 用量页的账号筛选只认 `requests.connection_id` 的精确值：`usage_stats.rs › where_clause`。
  - 用量页的「总 Token」含缓存，见对照表。
- **触发场景**：直接使用 Claude Code、不经网关时，记录的 `connection_id` 为 NULL。灵动岛绑定了某个 Claude 连接，点「在主窗口查看」。
- **影响**：灵动岛显示今日 X Token，用量页显示 0 次调用（或明显更少），而且 Token 口径也不同。
- **修法**：`openMainUsage` 不再带 `connectionId`，只预筛平台；或者只在 `live.attribution.confidence === "exact"` 时带上归属到的连接。用量页的「总 Token」旁注明「含缓存」，或增加新鲜 Token 指标。
- **单测思路**（后端部分）：写入一条今天的 claude 记录，`connection_id = NULL`。断言 `today_fresh_tokens(conn,"claude") == Some(15)`，同时 `usage_stats::overview(StatsFilter{connections: vec!["c1"], ..今天}).requests == 0`。

#### D8 用量页「较昨天同时段」实际比的是昨天的最后 N 小时

- **代码出处**：`usage_stats.rs › overview` 用 `f.with_range(f.start - span, f.start)` 算上期，其中 `span = 现在 - 今天零点`。`components/usage/OverviewPanel.tsx` 在今天范围内显示「较昨天同时段」。
- **影响**：上午 10 点时，比较对象是昨天 14:00–24:00，而不是昨天 00:00–10:00，变化率失真。
- **修法**：上期改为把本期起止各平移「本期跨越的自然日数」。今天的上期是 `[昨天零点, 现在 − 1 天)`，任意范围都按自然日整体前移。时间边界在后端用 `local_ms` 计算。
- **单测思路**：今天 10:00 查询。昨天 05:00 写入 100 Token，昨天 20:00 写入 7 Token。断言 `previous_total == Some(100)`，修复前为 `Some(7)`。

#### D9 用量页和路由概览的「上期」定义不同

- **代码出处**：路由概览「今天」的上期是昨天全天：`route_log.rs › Range::bounds`，文案为「较昨天 ±N」（`OverviewPanel.tsx`）。用量页声称比较的是「同时段」，见 D8。
- **影响**：两个页面都写「较昨天」，但分母不同。路由概览在白天会长期显示负增长。
- **修法**：先统一定义，建议都用「昨天同时段」：`Range::Today` 的上期设为 `[昨天零点, 现在 − 1 天)`。如果保留全天口径，文案改为「较昨天全天」。由鼠鼠决定。
- **单测思路**：`day_overview` 在今天 10:00 统计。昨天 05:00 一次请求，昨天 20:00 一次请求。按「同时段」口径，期望 `yesterday == 1`，现在是 2。

#### D10 `requests.platform` 含义混用，跨入口重复计数

- **代码出处**：
  - 本机采集行的 `platform` 表示调用方工具。`docs/specs/2026-11-05-island-live-linkage.md` 写的是「只是"哪个 CLI"」。
  - `gateway/subscription.rs › record_usage` 写 `platform: entry.platform`，即上游账号平台，`source = gateway-subscription`。调用方：`adapter/codex.rs`、`claude.rs`（隔离模式）、`claude_bridge.rs`、`gemini_subscription.rs`、`qoder.rs`，以及 `wire.rs` 里 grok / workbuddy / zcode 走的 `record_usage_on_end`。
  - 调用方工具自己的本机日志（Claude Code 的 JSONL、Codex 的 rollout）仍会记下同一次调用，`platform` 为 claude / codex。
- **触发场景**：Claude Code 经网关路由到 Qoder、ZCode 或 Codex 订阅。
- **影响**：
  - 用量页「全部供应商」的调用次数和 Token 各算两次，一次记在 claude，一次记在上游平台。
  - Claude 灵动岛和上游平台的灵动岛都会显示这笔用量。
  - 路由概览只算 1 次。
  - 计价按 `platform` 查价（`pricing::estimate_usage_at`），同一笔调用可能按两家的价格各估一次。
- **修法**（方案有取舍，由鼠鼠决定）：
  - **甲**：网关订阅记账改写到独立表 `gateway_usage`，带 `request_uid` 和 `caller_agent`，作为「来源账号」视图，不进 `requests`。
  - **乙**：保留在 `requests`，但新增 `caller_platform` 和 `request_uid` 列；用量页按调用方统计时排除 `source LIKE 'gateway%'`，按账号统计时才计入。
  - 两种方案都需要提供可恢复的数据迁移（AGENTS.md §4）。
- **单测思路**：写入一条 claude `local-session` 记录（`total=100`），再写一条 qoder `gateway-subscription` 记录（`total=100, connection_id='c-q'`），时间都是现在。断言 `usage_stats::overview(platform=None).requests == 2`。流水里写同一请求的一行，断言 `day_overview.today == 1`。
- **说明**：代码路径已确定。调用方工具是否一定写本机记录属于外部行为，实际规模放到 P4-3 第 5 步核对。

#### D11 同一个会话：用量页「高消耗会话」与会话页的总量不同

- **代码出处**：
  - `usage_stats.rs › top_sessions` 用 `r.total.unwrap_or(0)` 把未知当 0 累加，并过滤掉合计为 0 的会话；前端直接显示这个数，不标注覆盖情况（`usage/OverviewPanel.tsx`「高消耗会话」）。
  - `gateway/sessions.rs › tokens` 和 `sessionThreads.ts › sessionTotals` 在任一分项缺失时不给总数。
  - 退化行：用量页统计加了 `EXCLUDE_DEGENERATE_SQL`（`usage_stats.rs › where_clause`），`session_threads.rs › list` 没加，一条退化行会让会话的 `cache_read` / `cache_write` 变成未知。
- **影响**：同一个会话，用量页显示 1.2M，会话页显示「—」或「部分」；Token 全部未知的会话在用量页直接消失，违反「未知不是零」。
- **修法**：`SessionRow` 增加 `known` / `unknown` 两个计数（用 `Coverage::of` 计算），部分覆盖时前端写「已知 X（N 条未提供）」。`session_threads::list` 的 WHERE 加上退化行排除（参数位置需要重排）。
- **单测思路**：会话 s 有两条记录，一条 `total=100` 已知，一条 `total_known=0`。断言 `top_sessions` 给 100 且没有覆盖信息，`session_threads::list` 里该会话 `fresh_input` 等分项为 None。

#### D12 运行状态：路由页写「空闲」，灵动岛和会话页写「运行中」

- **代码出处**：
  - 路由页：`lib/contextSessions.ts › statusCell` 只看在途请求，没有在途请求就写「空闲」（`activeSessionRows` 传入的是 `view.running`，即在途请求数大于 0）。
  - 灵动岛：Claude / Codex 用生命周期状态 `sessions.activity_state = 'running'`（`db.rs › live_usage`）。
  - 会话页：同样用 `activity_state`（`session_threads.rs › list`）。
  - `lib/gatewaySessions.ts › sessionStatus` 会把 `thread_status` 一起考虑，但概览表没有用它。
- **触发场景**：Claude Code 在思考、执行本地工具或等待用户确认，此时没有在途请求。ZCode 请求不带会话标识，永远不会有在途记录。
- **影响**：同一会话，灵动岛和会话页显示「运行中」，路由「活跃会话」显示「空闲」。
- **修法**：`statusCell(running, threadStatus)` 复用 `sessionStatus`；没有在途请求但 `thread_status = running` 时写「运行中（无在途请求）」；没有证据时写「最近活跃」，不写「空闲」。文案标「待确认」。
- **单测思路**：在 `contextSessions.test.mjs` 中，`activeSessionRows` 传入 `thread_status:'running'` 且 `running:false` 的会话，断言状态文本不是「空闲」。修复前是「空闲」。

#### D13 采集失败只在灵动岛上体现

- **代码出处**：`hooks/useCollectionStatus.ts` 只被 `hooks/useIslandWindowModel.ts` 引用。在 `views/`、`components/` 中搜不到用量页、会话页、路由页对 `collection_status` 或 `failed_sources` 的引用。
- **影响**：某个采集源出错时，灵动岛把会话标为「未知」，用量页和会话页仍显示确定数字，违反「失败与过期要单独表达」（AGENTS.md §4）。
- **修法**：用量页概览和会话页顶部读 `collection_status`；当前筛选平台在 `failed_sources` 里时，在指标旁注明「采集失败，数据可能不完整（上次成功 HH:MM）」。文案和位置标「待确认」。
- **复现**：前端没有现成的单测入口。手工复现：在隔离实例里把一个 Claude 会话文件改成不可读，触发扫描，然后对比灵动岛和用量页。

#### D14 同一请求在不同入口取不同时刻，跨午夜时归入不同的日

- **代码出处**：`requests.ts` 在不同来源取不同时刻（ZCode 取开始时间，Claude 记录与网关订阅取结束时间），流水取响应头时刻，`day_overview` 与 `hourlyTrend` 一个用首次尝试、一个用最终尝试。出处见对照表「用哪个时间戳」两行。
- **触发场景**：
  - ZCode 经网关使用 ZCode 订阅，23:59 开始、00:01 结束。原生行（开始时间）归昨天，网关订阅行（结束时间）归今天。
  - 一个请求的首次尝试在 23:59:59，转移后的最终尝试在 00:00:01：路由概览把它算作昨天，趋势图画进今天 00 点。
- **影响**：只影响跨午夜或跨整点的请求，量很小，但对账时会出现无法解释的差额。
- **修法**：在契约中写明「归日用请求开始时间」。网关订阅记账改用 `started` 对应的墙钟时间（`record_usage_on_end` 已经持有 `started`，需要同时记录开始时的 `Local::now()`）。`hourlyTrend` 改按组内最小 ts 归小时。
- **单测思路**：`hourlyTrend` 输入两行流水，`request_uid` 相同，ts 分别为 0:59:59 和 1:00:01，断言计入 0 点那一格。修复前计入 1 点那一格。

### 2.2 疑似（需复现）

| 编号 | 现象 | 代码出处 | 为什么还不能定为缺陷 | 复现方法 |
|---|---|---|---|---|
| S1 | 灵动岛今日计数在快照合并时漏计 | `useGatewayActivity.ts` 跳过 `event.ts <= counts.at` 的结束事件；流水经 `route_log::submit` 异步写入，事件先于写库发出（`wire.rs › record_route`） | 需要 ts ≤ at 的行在 `counts_since` 查询时还没入库，取决于写入线程是否拿得到锁 | 采集器持锁期间连续发请求，并在此时打开胶囊；对比胶囊计数与 `counts_since` |
| S2 | 「组织今日 Token」与本地今日的日界不同 | `quota.rs › today_bounds` 用本地零点作为 `starting_at`，`bucket_width=1d` | ?（待确认：用 Context7 查了 Anthropic SDK 文档，`/anthropics/anthropic-sdk-python` 没有说明 1d 桶是否按 UTC 零点对齐；OpenAI 未查） | UTC+8 下 07:00 调接口，看返回桶的起点 |
| S3 | 子会话的 Token 在路由会话里漏算 | 网关把 OpenCode 子会话请求归到父会话、把 Codex 子线程归到根会话（`wire.rs › session_identity`）；`requests` 按各自的会话 id 记录（`opencode_usage.rs` 写 `opencode:{session}`）；`gateway/sessions.rs › overview` 只按父会话 id 查 Token | 需要真实的 OpenCode 子代理和 Codex 子线程样本，确认子会话是否落成独立的 `session_id` | 在 OpenCode 中触发一次子代理，对比路由会话 Token 与会话页父、子两行 |
| S4 | 「今日按来源」把被拒请求以外的网关行也计入请求数 | `source_breakdown` 按所有带 `connection_id` 的行计数；`gateway-subscription` 行和本机行都可能带同一连接（`settle_gateway_timing` 会回填本机行的 `connection_id`） | 直连 API 路径只回填，不新增行（`adapter/mod.rs`），订阅路径只新增网关行，理论上不会对同一请求产生两行带同一连接的记录；混合路径没有样本 | P4-3 第 5 步按 `connection_id` 分组核对 |

### 2.3 已核对、一致的部分

- 四个入口的自然日边界都基于本机时区零点（`chrono::Local` 与 JS `new Date(y,m,d)`），没有发现 UTC 混用。
- 「今日经网关」与路由概览共用一套去重和结果规则：都按 `request_uid` 归并、取最后一次尝试、`cancelled` 计为错误（`counts_since` 与 `day_overview`）。
- 「未知不是零」：灵动岛（`todayTokenView`）、用量页总量（`split` / `TokenCoverage`）、会话页（`SessionTokens`）都已经走 `Coverage`。例外是 D11 的高消耗会话。
- 上下文快照与流水按 `request_uid` 关联，并要求快照唯一（`context/query.rs › SUMMARY_JOINS`）。AGENTS.md 模块 J 写的「按 request_id 取流水最后一条」已经过时。

---

## 3. P4-2 会话工具范围

### 3.1 以代码为准的支持矩阵

| 能力 | 后端实际支持 | 代码出处 | 前端如何处理不支持的工具 |
|---|---|---|---|
| 查看会话（列表） | `requests` 中带 `session_id` 的来源：Claude / Codex 采集器、ZCode（过滤子会话）、OpenCode、Qoder；另从各自目录读取 Kimi / Pi / dsh | `session_threads.rs › list`、`kimi_sessions.rs` / `pi_sessions.rs` / `dsh_sessions.rs` | 状态显示 `usage_only` / `recent`（`statusLabel`） |
| 查看会话（正文） | Claude、Codex 读源文件；ZCode 只读其 sqlite，单会话最多 2000 个 part；其他来源 `available=false` | `session_threads.rs › messages`、`parse_file`、`zcode_page` | 显示「这个来源未保留消息正文」（`SessionDetail.tsx`） |
| 在原工具继续 | Claude（`claude --resume`）、Codex（`codex resume`），还要求会话 id 安全、命令在 PATH 中、项目目录可访问 | `session_threads.rs › tool_command` / `continue_check` / `continue_session` | 按钮禁用并给出提示：`canContinue(platform) && has_source`（`sessionThreads.ts`、`SessionDetail.tsx`）。与后端一致 |
| 压缩会话 | 源会话只支持 Claude / Codex，因为 `load_session` → `source_path`（只认 claude / codex 源文件）→ `parse_detail`（只解析 claude / codex）。执行压缩的账号也只能是 Claude / Codex 当前登录账号（`executor_args`、`automation::tool_name`、`current_connection_ids`）。「在原工具里分叉」要求执行账号与源会话同平台 | `session_handoff.rs › handoff_start` / `load_session` / `fork_args` | **按钮对所有平台都可点**（`SessionDetail.tsx` 的压缩按钮没有 `disabled`）。对话框里 `readable=false` 时显示「仅用量记录 / 源文件不可读」（`CompressDialog.tsx`） |
| Markdown 交接 | 渲染已有压缩版本：`handoff_render`，需要已有版本。本地导出（不调用模型）：`handoff_local_export`，同样依赖 `load_session`，所以也只支持 Claude / Codex。保存：`handoff_save`。「交接给」目标（10 项，`HANDOFF_TARGETS`）只影响文档措辞（`target_text`），不会为目标工具调整格式，也不会打开目标工具 | `session_handoff.rs`、`lib/sessionHandoff.ts` | 源不可读时隐藏本地导出按钮（`CompressDialog.tsx`） |

### 3.2 与 AGENTS.md「模块 D」行及前端的不一致

1. **ZCode 压缩时给出的原因不对**：会话页能显示 ZCode 正文（`messages` 走 `zcode_page`），但点「压缩会话」后对话框说「仅用量记录 / 源文件不可读，无法仅凭 Token 数与会话标题生成可靠的开发交接」。真实原因是压缩的读取链路只支持 Claude / Codex。AGENTS.md 写的「压缩会话仍只支持 Claude Code / Codex」本身没错，但界面给出的原因与事实不符。
2. **压缩入口没有按平台禁用**：「在原工具继续」按平台禁用并有提示，「压缩会话」没有。对不支持的平台，建议参照继续按钮的做法：禁用按钮并提示「{平台} 暂不支持压缩（需要 Claude Code / Codex 会话）」。文案标「待确认」。
3. **AGENTS.md 没写本地导出的范围**：D 行只说「压缩会话」只支持 Claude / Codex，没有说明「本地交接导出」同样只支持这两家。
4. **AGENTS.md 没写列表范围**：OpenCode / Qoder 会话会出现在列表里但没有正文；Kimi / Pi / dsh 会话来自各自目录，`has_source=false`。D 行只提到了 ZCode 正文。
5. **Kimi / Pi / dsh 无法按平台筛选**：`commands/usage_sessions.rs › sessions_list` 拒绝 `platforms::known` 以外的平台（目录里只有 10 家，不含 kimi / pi / dsh），所以 `session_threads.rs › list` 里 `Some("kimi") / Some("pi") / Some("dsh")` 这几个分支实际上执行不到。这些会话只在「全部」视图中出现，选定平台后就看不到了。前端 `PlatformId` 也不含这三家。
6. **「交接给」的作用没有说明**：选择目标工具只改变文档里的措辞，界面没有说明这一点，用户可能以为 Jolu 会把交接内容送进目标工具。

### 3.3 跨 Agent 交接方案草案（例如 Claude Code 会话交给 Codex 继续）

目标：在不修改原会话的前提下，把 A 工具会话的进度交给 B 工具，开出一个新会话继续工作，并且让这两个会话在 Jolu 里能互相追溯。

| 步骤 | 复用的能力 | 新增内容 | 说明 |
|---|---|---|---|
| 1. 读取源会话 | `load_session` / `parse_detail`（Claude / Codex） | 把 ZCode 正文接入 `parse_detail`，复用 `zcode_page` 的 part 解析，ZCode 会话也能压缩和本地导出 | 只读，沿用 2000 个 part 的截断和标注 |
| 2. 生成交接文档 | `handoff_start`（Jolu 整理或分叉）、`handoff_local_export`（不调用模型）、`render_markdown` | `RenderMeta` 增加 `target_platform`。按目标工具调整首段写法（例如 Codex 读 `AGENTS.md` 习惯，Claude 读 `CLAUDE.md` 习惯），只改模板措辞 | 不额外调用模型 |
| 3. 落盘 | `handoff_save`（可写到项目目录） | 默认写到源会话的 `cwd`，文件名沿用 `suggested_file_name` | 已有覆盖确认（画板 54-09） |
| 4. 在目标工具中打开 | `continue_session` 里打开新控制台的做法 | 新增 `handoff_open(target, cwd, md_path)`：在 `cwd` 新开控制台，用目标 CLI 启动新会话，并把「请先阅读 <md 路径>」作为首条提示 | ?（待确认：Claude Code / Codex 以初始提示启动交互会话的参数，需要先用 Context7 核对，本次未查）。不自动发送模型请求的做法另议 |
| 5. 关联两个会话 | `session_handoffs` 表 | 增加 `target_platform` 和 `target_session_id`；目标工具的新会话出现后，在一段时间窗口内按 `cwd` 和启动时刻找唯一候选，**找不到唯一候选就标「未关联」，不猜** | 符合 §1 完成标准第 5 条 |
| 6. 界面 | 会话详情、压缩对话框 | 详情页显示「交接自 / 交接到」；对话框在「交接给」旁加「生成后在 {目标} 中打开」开关 | **待确认：需设计**（Jolu.pen 没有这组状态） |
| 7. 经网关时 | 路由分组、接线 | 可选：目标工具复用源会话最近一次所用的路由分组 | **待确认：需设计**，并涉及 Agent 页 |

---

## 4. P4-3 真实账号端到端验收清单（本次未执行）

### 4.1 需要的账号与费用

| 角色 | 建议 | 可能产生的费用 / 消耗 |
|---|---|---|
| 调用方 Agent 1 | Claude Code，已登录 Claude 订阅或配置了 API Key | 订阅：消耗 5 小时 / 7 天额度；API：按 token 计费 |
| 调用方 Agent 2 | Codex，已登录 ChatGPT 或配置了 API Key | 同上 |
| 上游 A：API Key | 一个 OpenAI 兼容的第三方 Key（例如 DeepSeek），用于直连路径 | 按 token 计费，建议在供应商后台设置预算上限 |
| 上游 B：订阅 | ZCode 或 Qoder 订阅连接（`gateway-subscription` 记账路径，用于复核 D10） | 消耗订阅额度或积分 |
| 失败转移成员 | 第二个 API Key，或故意填错一个 Key 让它稳定返回 401 / 5xx | 失败请求通常不计费；?（待确认：各家对失败请求是否计费） |

费用无法事先估算：Claude Code / Codex 每次请求都带系统提示和工具定义，实际 token 数取决于模型和缓存命中。执行时用最便宜的模型，每一步控制在 3 轮对话以内，并以官方账单的实际数字为准。

### 4.2 准备工作

1. 记录 Jolu 提交号、构建方式（`pnpm tauri dev` 或安装包）、系统时区，以及本机时钟与网络时间的偏差。
2. 备份：Jolu 应用数据目录下的 `usage.db`（路径以「设置 › 数据与存储」显示的为准），以及接线会写入的工具配置：`~/.claude/settings.json`、`~/.codex/config.toml`，还有上游工具自己的配置目录。对账一律在 `usage.db` 的**只读副本**上执行 SQL。
3. 记录起始状态：各连接在官方后台的额度百分比或余额，以及账单页当前的累计数。截图存到 `output/acceptance/p4-3/`（该目录已被 Git 忽略）。
4. 记录基线：
   - `SELECT COUNT(*), MAX(id) FROM gateway_route_log;`
   - `SELECT MAX(id) FROM requests;`
   - `SELECT value FROM gateway_request_sequence;`

### 4.3 步骤与每一步要记录的字段

| 步 | 操作 | 要记录的字段 | 对账方法 |
|---|---|---|---|
| 1 连接 | 在供应商页添加上游 A、B 和失败成员，启用转发 | `connections.id / platform / kind / slug / status`（只记掩码）；网关端口 | 账号卡状态与 `gateway_source_capabilities` 一致 |
| 2 路由 | 新建分组 `group/p43`，策略选 order，成员依次为失败成员、A；接线 Claude Code 与 Codex | 分组成员顺序、接线写入预览、`agent_wiring` 行 | 写入预览与配置文件改动逐项一致 |
| 3 工具调用 | 在 Claude Code 里用 `group/p43` 发一个固定提示，内容需要调一次本地读文件工具（例如「读取 README.md 并告诉我第一行」），Codex 做同样的事；再用上游 B 发一轮 | 本地开始与结束时间（精确到秒）、Claude 会话 id（JSONL 文件名）、Codex 会话 id（rollout 文件中的 `session_meta`）、终端显示的 token / 费用、工具是否真的执行并回传了结果 | 最终回答包含文件第一行内容 |
| 4 路由流水 | 打开「模型路由 › 路由流水」与概览 | 新增流水行的 `ts / agent / requested_model / source / connection_id / session_platform / session_id / request_uid / result / status_code / duration_ms / decision` | 同一 `request_uid` 有 2 行（失败成员 + A）；`session_id` 与 JSONL / rollout 一致；概览今日请求增加量等于不同 `request_uid` 的个数 |
| 5 用量 | 用量页选今天，分别筛 Claude、Codex、上游 B 的平台 | 新增 `requests` 行的 `platform / source / dedup_key / session_id / ts / input / output / cache_read / cache_write / total / *_known / connection_id / duration_ms / status_code`；入库时刻（第一次在用量页看到这条记录的时间） | `dedup_key` 与 JSONL 的 `message.id`、rollout 的 `response_id` 逐条对应；直连路径的本机行带 `connection_id`；**复核 D10**：上游 B 那一轮是否同时出现 `platform='claude'` 的本机行和 `platform=<B>` 的 `gateway-subscription` 行；用量页「全部」的调用次数与流水请求数之差 |
| 6 会话 | 会话页找到第 3 步的会话；在路由「活跃会话」里找同一会话 | `first_ts / last_ts / requests / 四个分项 / status / connection_id`；路由行的状态和上下文快照 `state` | 会话页的 `requests` 等于 `requests` 表中该会话的行数；**复核 D12**：思考期间路由行的状态 |
| 7 灵动岛 | 经典样式与胶囊样式各看一次；展开卡片；点「在主窗口查看」 | 本机今日 Token、本轮增量、今日累计写入、今日经网关次数（成功 / 被拒 / 失败）、今日按来源各行、会话状态，以及看到每个数字的时刻 | **复核 D1**：失败转移后胶囊失败数是否加 1；**复核 D6**：按来源之和与本地今日 Token；**复核 D7**：跳转后用量页的数字 |
| 8 异常 | 在 Claude Code 流式输出途中按 Ctrl-C；再在响应头返回前断开（使用慢上游） | 流水 `result`、胶囊计数 | **复核 D2**：流水是 cancelled，胶囊是否仍按 ok 计；断开前的请求是否完全没有流水 |
| 9 账单 | 结束后 10 分钟和次日各看一次官方账单与额度 | 官方 token / 费用 / 额度变化、看到这些数据的时刻 | 本地 Token 与官方 token 的差额及原因（缓存计费、失败重试、官方延迟）；记录采集延迟：JSONL 写入 → `requests` 入库 → 界面出现 → 官方后台出现 |
| 10（可选）跨午夜 | 23:58 发起一个长请求 | 两侧归属的日期 | **复核 D14** |

### 4.4 对账 SQL（只读副本）

```sql
-- 本次新增的流水按请求归并（:base_id 为准备阶段记下的 MAX(id)）
SELECT request_uid, COUNT(*) AS attempts, MIN(ts), MAX(ts),
       GROUP_CONCAT(source || ':' || result, ' → ') AS chain
FROM gateway_route_log WHERE id > :base_id GROUP BY request_uid ORDER BY MIN(ts);

-- 本次新增的用量行，按平台和来源统计
SELECT platform, source, COUNT(*), SUM(CASE WHEN total_known=1 THEN total_tokens END),
       SUM(connection_id IS NULL)
FROM requests WHERE id > :base_req GROUP BY platform, source;

-- 某个会话两侧对照
SELECT 'requests' AS side, COUNT(*), MIN(ts), MAX(ts) FROM requests
 WHERE platform=:p AND session_id=:s
UNION ALL
SELECT 'route_log', COUNT(DISTINCT COALESCE(request_uid, -id)), MIN(ts), MAX(ts) FROM gateway_route_log
 WHERE session_platform=:p AND session_id=:s;
```

### 4.5 结果记录格式

- 每一项结论分「真实环境 / 模拟 / 单元测试」三栏填写（计划 §1 第 6 条）。
- 记录未验证矩阵：Agent（Claude Code / Codex / OpenCode / ZCode…）× 上游（API / 各订阅）× 协议（Anthropic / Chat / Responses）× 流式与否。没有账号的组合标「未验证」，不写成通过。
- 对照画板：用 Pencil MCP 截取模块 A / A3 / C / D / J 的对应画板，与运行截图并排放置（本次审计未打开 `.pen`）。
