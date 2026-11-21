# 用量采集准确性：ZCode 对账与改进计划

日期：2026-11-21  
状态：**已实施完成（2026-11-21），见 §10 执行记录；§10.5 列出待鼠鼠确认项**  
范围：用量统计页中 ZCode 的原生请求采集、统计口径与展示准确性。

## 1. 结论先行

针对 2026-11-21 的本机只读对账，ZCode 的 `model_usage` 有 1599 条请求，Jolu 对应的 `native:zcode:main` 来源也有 1599 条。按来源记录 ID 对齐后，1599 条全部匹配，没有发现当天漏采或重复采集。

用户看到的“ZCode 国际版 1599 次、Token 为 —”主要暴露了统计展示和口径问题，不足以证明采集器少读了记录：

- 1594 条有可汇总的 Token，总计 754,323,790 Token；另外 5 条（2 条 `error`、3 条 `cancelled`）没有可用总量。
- 供应商汇总行有任意一条 Token 未知时会把整组 `tokens` 设为 `null`，因此已知小计也显示为“—”。总览指标已有 `token_coverage`，供应商行没有对应覆盖信息。
- Jolu 将 ZCode 的 `cancelled` 和 `error` 都存成 `native_outcome='failed'`，所以 UI 显示 5 次失败，而 ZCode 原始状态中只有 2 次错误、3 次取消。
- ZCode 按 `started_at` 归日，Jolu 当前优先使用 `completed_at`。本机库有 6 条请求跨过本地日期边界，按日筛选时两边可能出现不同分布。
- 用量汇总按 `platform='zcode'` 聚合，但前端把平台 ID `zcode` 解析为“ZCode 国际版”。ZCode 国内版和国际版共用平台 ID，因此需要核对 1599 条的 `provider_id` / 账号归因后才能确认这个标签是否属实。

历史全量数量不同有已知原因：本机当前 ZCode 库有 16,674 条，Jolu 有 22,546 条，多出的 5,872 条集中在 2026-10-02 至 2026-10-11。ZCode 源码会清理超过 30 天的 `model_usage`；Jolu 保留已经采集的历史记录。这里是源端保留期与 Jolu 历史账本不同，不是已证实的 Jolu 虚构数据，也不应通过清空 Jolu 记录来“对齐”。

## 2. 本次证据

### 2.1 当天采集对账

| 检查项 | 2026-11-21 只读结果 |
|---|---:|
| ZCode `model_usage` 请求行 | 1599 |
| Jolu `native:zcode:main` 请求行 | 1599 |
| 两侧按请求 ID 匹配 | 1599 / 1599 |
| 可汇总 Token 的请求 | 1594 |
| 已知 Token 小计 | 754,323,790 |
| 无可用总量的请求 | 5（`error` 2，`cancelled` 3）|
| 跨本地日期边界的请求 | 6 |

这些是调查当天本机数据库的快照，不是未来运行时保证值。实现验收应以固定测试夹具验证行为，并用只读查询重新核对当时数据。

### 2.2 历史行数差异

| 数据源 | 当前行数 |
|---|---:|
| ZCode `model_usage` | 16,674 |
| Jolu 对应来源记录 | 22,546 |
| 只在 Jolu 留存的旧记录 | 5,872，日期为 2026-10-02 至 2026-10-11 |

ZCode 的 `apps/zcode-cli/packages/adapters/src/storage/session-store/repositories/usage.ts` 定义 30 天保留期，并按 `started_at` 删除较早的 `model_usage`。Jolu 扫描到的旧请求保留在自身数据库中。不要为了让行数相等而删除 Jolu 历史记录。

## 3. 代码路径与根因

### 3.1 采集入口

- `backend/src/native_sources.rs` 的 ZCode 扫描读取 `model_usage` 的 ID、状态、`started_at`、`completed_at`、Token 字段和 `provider_id`。
- 请求去重键使用 ZCode 请求 ID，平台存为 `zcode`，来源为本机 ZCode 监控来源。
- 当前请求时间戳取 `completed_at`，缺失时才回退到 `started_at`。
- 原始状态当前被折叠成 `success`、`failed` 或 `unknown`；`error` 和 `cancelled` 都折叠为 `failed`。
- ZCode `provider_id` 会进入 Jolu 的 `requests.provider_key`，平台仍为 `zcode`。这能支持后续归因，但供应商用量汇总目前没有按版本拆分。

### 3.2 Token 汇总与展示

- `backend/src/usage_stats.rs` 的 `ProviderRow` 只有平台、请求数、Token 值和费用，没有 Token 覆盖数。
- 供应商分组调用 `sum_known`。对同组里部分 Token 未知的原生请求，组总 Token 会是 `None`，已知的小计无法通过该行返回。
- 同文件的 `Overview` 已返回 `token_coverage.known_total` 与 `known_requests`，说明后端已经有可复用的覆盖口径。
- `frontend/src/lib/api.ts` 的 `StatsProviderRowDto` 和 `frontend/src/components/usage/OverviewPanel.tsx`、`AnalysisPanel.tsx` 需要同步供应商行 DTO 与展示逻辑。
- `OverviewPanel.tsx` 的总览指标已能显示部分已知 Token 和覆盖请求数；供应商用量行仍直接格式化 `p.tokens`，遇到部分未知时显示“—”。

### 3.3 版本标签

- `frontend/src/lib/providerCatalog.ts` 中 `zcode` 对应“ZCode 国际版”，`zcode-bigmodel` 对应“ZCode 国内版”，但二者的 `platformId` 都是 `zcode`。
- `frontend/src/components/usage/usageControls.tsx` 的 `providerName("zcode")` 按 `id === platform` 查找，结果会显示“ZCode 国际版”。
- 后端 `providers()` 只按 `platform` 分组，所以该行可能是 ZCode 平台总量，却显示成国际版。需要先用 `provider_key`、连接归因和本机 provider 配置只读核对版本组成；不能由模型名或行标题猜版本。

### 3.4 Magpie 参考边界

Magpie 的 ZCode 会话调用实现位于本机源码 `D:/develop/_research/magpie/internal/sessions/calls_zcode.go`。它从会话消息中按每条 assistant message 计一次调用，并读取消息 Token；这和 ZCode `model_usage` 的每条原生请求记录不是同一口径。

Magpie 网关账本 `usage.jsonl` 记录的是经过 Magpie 网关的请求，也不是 ZCode 本机所有直连调用的替代数据源。因此：

- 对账 ZCode 自己的请求数和 Token，应以 ZCode `model_usage` 为参照。
- 可借鉴 Magpie 明确说明“会话消息调用”和“网关账本调用”的来源边界，不要把两种计数合并成一个数字。
- 不要用 Magpie 的 assistant message 数直接替换 Jolu 的原生请求数。

## 4. 实施目标

| 编号 | 目标 | 验收标准 |
|---|---|---|
| G1 | 部分 Token 未知时仍能看见已知小计和覆盖率 | 1594 / 1599 有 Token 时，供应商行呈现已知小计并明确标出 1594 / 1599；全未知仍显示“—”，不能显示成 0 |
| G2 | 状态不丢信息 | ZCode 的 `error` 与 `cancelled` 能区分；成功率、失败数与状态筛选有明确一致的定义 |
| G3 | 日期筛选可与 ZCode 对账 | 文档和 UI 对请求归日使用哪个时间有明确约定；跨本地午夜用测试锁定 |
| G4 | ZCode 版本名称准确 | 汇总行不把多个版本混合总量标成单一版本；版本拆分只能使用可验证的归因事实 |
| G5 | 历史数据不被误清 | ZCode 30 天清理不会触发 Jolu 删除其已采集的旧历史 |
| G6 | Magpie 参考不混淆计数口径 | 网关日志、Agent 会话消息和原生 usage 行的含义分别说明 |

## 5. 建议实施顺序

### P0：先修 Token 覆盖展示

1. 为供应商统计行增加已知 Token 小计与有 Token 的请求数，沿用 `Overview.token_coverage` 的“未知不是零”原则。
2. 更新 Rust DTO、`frontend/src/lib/api.ts` 和用量概览 / 分析页供应商行，避免一个页面修好、另一个页面仍显示“—”。
3. 部分覆盖时显示已知 Token 和 `known_requests / requests`；全覆盖时保留现有紧凑展示；全部未知时显示“—”并说明覆盖数为 `0 / N`。
4. 增加后端单测覆盖全量已知、部分未知、全部未知三种情况，并覆盖多个平台混合分组。

### P1：保留准确状态

1. 检查 `requests.native_outcome` 的现有约定与所有调用方，再决定是否扩展它，或单独保存原始原生状态。不要为了区分取消而破坏其他原生来源的既有含义。
2. 定义“失败数”和成功率是否排除取消。至少保证 `error`、`cancelled` 不会在明细中被伪装成同一种原始状态。
3. 变更后处理已存在的本机行。当前扫描有源数据库戳缓存，ZCode 数据库不变时可能直接跳过；需设计受控的重扫 / 回填方式，只更新对应原生记录，不删除历史账本。
4. 加入状态夹具：completed 计成功，error 计错误，cancelled 计取消，缺失或未知状态仍保持未知。

### P1：统一请求归日时间

1. 明确用量页面的日统计与 ZCode 原生统计是否都按 `started_at` 归日。建议按请求开始时间归日，因为这是 ZCode 原生保留和查询使用的时间字段。
2. 检查 `ts` 同时参与趋势、调用明细排序、价格时段估算和 CSV 导出。若请求归日改用开始时间会改变价格时段含义，应把归日时间与计价时间分开保存或明确选择，不要无意间改变费用结果。
3. 增加跨午夜测试，确认本地时区日期、趋势桶、日期筛选和导出一致。

### P1：修正平台与版本标签

1. 只读统计当前 1599 条按 `provider_key`、连接归因结果和可用 provider 配置的分布。不得输出 API Key 或完整地址。
2. 若统计行按 `platform='zcode'` 聚合了国内版和国际版，则名称应为“ZCode”平台总量；只有在真实归因可信且实现支持分组时，才可拆成国内版 / 国际版。
3. 增加映射单测，避免 `platform='zcode'` 因 provider catalog 的默认首个匹配而被隐式标成国际版。

## 6. UI 与设计稿约束

用量页对应 `docs/ui/Jolu.pen` 模块 C，画板 30–39。实现者开始 UI 修改前，按 AGENTS.md 通过 Pencil MCP 查看相关浅色 / 深色画板，确认供应商行是否已有覆盖数的布局。

若画板没有“已知 Token / 覆盖请求数”状态，不要自行扩展视觉设计；在实现记录中标记“待确认”，并先完成不改变布局的数据层测试与方案说明。保留现有主窗口设计变量与密度。

## 7. 工作区注意事项

本计划形成时工作区已有未提交修改，包括 `frontend/src/components/usage/OverviewPanel.tsx`、`AnalysisPanel.tsx`、`CallsPanel.tsx`、`TrendChart.tsx` 等文件。接手者必须先查看这些文件的 `git diff`，在现有改动基础上叠加任务，不得回退或覆盖用户的修改。

## 8. 完成验收

逐条对照（证据见 §10）：

| # | 验收项 | 状态 | 证据 |
|---|---|---|---|
| 1 | 后端用量统计测试、前端纯逻辑测试、类型检查通过 | ✅ | §10.2：后端 1582 通过 / 0 失败；前端 504 通过 / 0 失败；`tsc` 通过；0 warning |
| 2 | 供应商行部分未知显示已知小计与覆盖数，全未知仍显示「—」 | ✅（单测） | §10.2 的 `providerTokenCell` 三态测试。原始快照为 1594 / 1599 条有已知总量，见 §10.3.2 |
| 3 | 与 ZCode 快照按 ID 一对一匹配 | ✅ | §10.3.1：两侧交集 **1,599**，**只在 Jolu 的 0 条**（无虚构/重复）。383 条「只在 ZCode」是扫描位点之后的新数据，非漏采 |
| 4 | 已知 Token 为 1594 条、合计 754,323,790 | ✅ | 合计 **754,323,790** 一致。必须按 `total_known` 判断，不能仅检查 `total_tokens` 是否为空。见 §10.3.2 |
| 5 | 状态差异不再被隐藏 | ✅ | 四值映射 + 受控回填已实现并有单测；**真机回填已于 2026-11-22 执行**：50 条 `failed → cancelled`、160 条不动、总行数 22,548 不变、标记已写。见 §10.4 |
| 6 | 跨本地午夜的固定测试数据验证归日规则 | ✅ | `zcode_attributes_a_cross_midnight_request_to_its_start_day`，固定 2026-03-10 23:59:30 开始 / 00:00:30 完成 |
| 7 | 「ZCode 国际版」标签核实 | ✅ | §10.3.5：`bigmodel*` 前缀全库 **0 条**，`connection_id` 全 NULL → 无法证明版本，显示平台名「ZCode」 |
| 8 | Jolu 保留 ZCode 已清理的旧记录 | ✅ | §10.3.3：**只在 Jolu 5,872 条**，与计划 §2.2 完全一致；回填只 UPDATE 不删行 |
| 9 | 执行记录写清修复内容 / 测试 / 真机范围 / 待确认 | ✅ | §10.1–§10.5 |

上述记录是实施时的验收结果。后续复核发现筛选、未知状态和历史归日仍有缺口，修复与验证范围见 §11；桌面界面与真实扫描后的对账仍待验证。

## 9. 参考文件

- Jolu 采集：`backend/src/native_sources.rs`
- Jolu 聚合：`backend/src/usage_stats.rs`
- ZCode provider 归因：`backend/src/attribution.rs`
- 用量 DTO：`frontend/src/lib/api.ts`
- 用量展示：`frontend/src/components/usage/OverviewPanel.tsx`、`AnalysisPanel.tsx`、`usageControls.tsx`
- ZCode 30 天清理：`D:/develop/ZCode-src/apps/zcode-cli/packages/adapters/src/storage/session-store/repositories/usage.ts`
- Magpie ZCode 会话调用：`D:/develop/_research/magpie/internal/sessions/calls_zcode.go`

---

## 10. 执行记录（2026-11-21 实施）

### 10.1 实际修复内容

**P0 · Token 覆盖展示**

| 改动 | 文件 |
|---|---|
| `ProviderRow` 新增 `known_tokens: i64` / `known_requests: i64`；`providers()` 改用已有的 `known_subtotal()` 计算，`tokens` 语义不变（任一未知则 None） | `backend/src/usage_stats.rs` |
| `StatsProviderRowDto` 同步两个字段 | `frontend/src/lib/api.ts` |
| 三态判断抽成纯函数 `providerTokenCell(row)`：全覆盖 → 原紧凑展示；部分覆盖 → 已知小计 + `1594 / 1599 条有 Token`；全未知 → 「—」+ `0 / N 条有 Token`。复用已有 `formatTokens` / `DASH` | `frontend/src/lib/usageStats.ts` |
| 供应商用量行调用该纯函数（列宽 / 卡片 / 行高未动） | `frontend/src/components/usage/OverviewPanel.tsx` |
| 示例数据补字段 | `frontend/src/mock/usageStats.ts` |

`AnalysisPanel.tsx` 的「供应商费用」卡只渲染费用不渲染 Token，按 §6「不自行扩展视觉设计」未改。

**P1 · 状态不丢信息**

`native_outcome` 值域由三值扩成四值（`success` / `failed` / `cancelled` / `unknown`）：

| 改动 | 文件 |
|---|---|
| 新增 `zcode_native_outcome()`：`completed→success`、`error→failed`、`cancelled→cancelled`、其余→`unknown` | `backend/src/native_sources.rs` |
| **失败口径不含 cancelled**：取消是用户主动中止，不是上游错误。并进了会让「失败数」和「成功率」同时失真（本机 1599 条只有 2 条 error，却因 3 条 cancelled 显示 5 次失败）。`FAILED_SQL` 显式排除 `cancelled`，三档严格互斥 | `backend/src/usage_stats.rs` |
| 值域常量 `NATIVE_OUTCOMES` + `native_outcome_valid()`，避免校验点漂移 | `backend/src/db.rs` |
| **受控回填** `backfill_zcode_cancelled`：只在源库 `model_usage` 仍可读时按 `dedup_key` 把 `failed` 且源状态确为 `cancelled` 的行改成 `cancelled`；**只 UPDATE 状态，不删任何行**；用 `migration_log` 记一次标记，幂等 | `backend/src/native_sources.rs` |
| 前端 `callStatus` 加 `cancelled` → 「已取消」（中性色，不冒充成功绿也不冒充失败红）；状态筛选加「已取消」 | `frontend/src/...` |

顺带修掉一处边界缺陷：取消时上游可能留非 2xx，原来会同时被判失败与取消。现在 `cancelled` 优先，`success + error + cancelled == requests`。

**P1 · 请求归日**

`ts` 从「优先 completed_at」改为「优先 started_at」（缺失才回退 completed_at），与 ZCode 自身归日口径一致。**关键修正**：`ts` 同时是趋势桶 / 明细排序 / 价格时段 / CSV 的时间，所以会话活跃判定单独改用 `completed_at`（`rows` 元组加第 7 个元素），两个时间各有用途不能合并——否则长请求或跨午夜请求会被误判成空闲。

**P1 · 版本标签**

`providerName(platform)` 从「按 `id === platform` 查找」改为「按 `platformId` 统计条目数」：

- 多条目平台（`zcode` / `qoder` / `workbuddy` / `trae` / `custom`）→ 返回平台名（`ZCode`），不标任何版本
- 单条目平台（`claude` / `codex` / …）→ 行为不变
- 查不到 → 返回裸 platformId，不猜

11 个调用方改为直接从 `@/lib/providerCatalog` 导入。两个条目的 `platformId` 未改动，连接页供应商选择不受影响。

### 10.2 运行的测试

| 项 | 结果 |
|---|---|
| `cargo test --manifest-path backend/Cargo.toml --lib` | **1582 通过 / 0 失败 / 52 忽略** |
| 编译警告 | **0** |
| 前端 `node --test` | **504 通过 / 0 失败** |
| 前端 `tsc --noEmit` | 通过 |

新增测试：
- `zcode_native_outcome_keeps_cancelled_apart_from_error`（四值映射 + 会话状态仍折 failed）
- `backfill_turns_legacy_failed_cancellations_into_cancelled_without_deleting_rows`（只改一行、不删行、幂等、源库已清理行保留）
- `backfill_marks_done_even_when_nothing_to_fix`
- `cancelled_is_neither_failure_nor_success`（三档互斥、502+cancelled 优先、CSV、筛选项校验）
- `backup_metrics_accept_cancelled_and_reject_unknown_values` + 往返
- `zcode_attributes_a_cross_midnight_request_to_its_start_day`（**固定**本地时刻 2026-03-10 23:59:30 开始、00:00:30 完成，断言归 3 月 10 日）
- 前端 `providerTokenCell` 三态 + `providerName` 多版本平台

### 10.3 本机只读核对（2026-11-21 当天）

#### 10.3.1 与 ZCode `model_usage` 的按 ID 交叉匹配

**核对时间**：2026-11-21 深夜（ZCode 源库继续增长，行数比 §2.1 调查时刻多）

| 检查项 | 结果 |
|---|---:|
| ZCode `model_usage` 当天（`started_at` 归日） | 1,982 |
| Jolu `native:zcode:main` 当天（`ts` 归日） | 1,599 |
| **两侧按请求 ID 交集** | **1,599** |
| 只在 Jolu（虚构/重复） | **0** |
| 只在 ZCode（未采集） | 383 |

**「只在 Jolu = 0」是最关键的结论**：Jolu 当天每一条记录都能在 ZCode 源库里按请求 ID 找到，没有虚构、没有重复。

**那 383 条差异与扫描位点时间相符**：

| 分组 | 时间范围 |
|---|---|
| 已采集（1,599 条） | 11-21 00:00:03 .. 21:16:48 |
| 未采集（383 条） | 11-21 22:39:27 .. 23:59:52 |

未采集记录的开始时间晚于当时已采集记录的最新时间。`scan_state` 的 mtime 是 22:53、最新入库记录是 21:17，这一快照不能单独证明 383 条已被后续扫描补齐，需重新扫描后按 ID 对账。计划文档 §2.1 记录的 1,599 是当天较早时刻的快照值。

#### 10.3.2 原始快照的 Token 覆盖率（2026-11-22 更正）

原始快照覆盖 2026-11-21 本地时间 00:00 至约 21:16 的 1,599 条请求。请求表将未知数值保存为 0，并用 `total_known` 标记是否已知。此前仅检查 `total_tokens IS NOT NULL` 的结论有误。

| 口径 | 数值 |
|---|---:|
| 全部请求 | 1,599 |
| `total_known=1` | 1,594 |
| 已知总量小计 | 754,323,790 Token |
| `total_known=0` | 5（2 条 error，3 条 cancelled） |

供应商行应展示已知小计和 1,594 / 1,599 的覆盖率。后续采集增加了当天的记录，整天的数量会随扫描时间变化。

#### 10.3.3 Jolu 保留了 ZCode 已清理的历史（全库核对）

| 检查项 | 本次实测 | 计划 §2.2 |
|---|---:|---:|
| ZCode `model_usage` 当前行数 | 17,060 | 16,674 |
| Jolu `native:zcode:main` 行数 | **22,546** | 22,546 |
| **只在 Jolu（ZCode 已按 30 天清理）** | **5,872** | 5,872 |
| 两侧共有 | 16,674 | 16,674 |
| 只在 ZCode（尚未采集） | 386 | — |

**「只在 Jolu 5,872 条」与计划调查完全一致**，证明：
1. Jolu 确实保留了 ZCode 已按 30 天期限删除的历史记录
2. 受控回填（`backfill_zcode_cancelled`）没有删除任何行——它只 UPDATE 状态列

ZCode 侧从 16,674 涨到 17,060 是源库继续新增的正常增长。

#### 10.3.4 状态差异在真实数据上的体现

**当天（2026-11-21，1,599 条）**

| 值 | 条数 | 说明 |
|---|---:|---|
| `success` | 1,594 | |
| `failed` | 5 | 其中 2 条原始状态 `error`、3 条 `cancelled`（回填前的状态） |

**全库（22,548 条，回填后）**

| 值 | 条数 |
|---|---:|
| `success` | 22,336 |
| `failed` | 160 |
| `cancelled` | **50** |
| NULL | 2 |

回填已执行（见 §10.4）：原来 210 条 `failed` 里混着 50 条实为取消的请求，现在拆成 `failed 160 + cancelled 50`，**`error` 与 `cancelled` 不再被隐藏成同一个状态**。总行数保持 22,548。

#### 10.3.5 版本标签核对

当天 1,599 条的 `provider_key` 分布：

| provider_key | 条数 | 性质 |
|---|---:|---|
| `new-provider-5` | 1,493 | 用户自建第三方中转（`ollama拼车`，openai-responses） |
| `new-provider-2` | 104 | 用户自建第三方中转（`华龙ds`） |
| `jolu` | 2 | 指向 Jolu 网关 |

`connection_id` **全部为 NULL**（1,599 / 1,599），归因没有给出任何连接级或版本级事实。

与 `~/.zcode/v2/provider_config.json` 交叉核对（只读 `baseUrl` / `providerId` / `providerName`，未读任何密钥字段）：上述三条都是 `templateId: null` 的自建条目，`baseUrl` 指向第三方。

**全库层面**：`zai*` 前缀 7,810 条、`bigmodel*` 前缀 **0 条**、第三方 10,410 条、`provider_key` 为 NULL 的 4,328 条。

**结论：BigModel 国内版在全库没有任何一条可验证记录**，这 1,599 行只能表示「ZCode 平台总量（含国际版与第三方中转）」。§3.3 怀疑的「汇总行被隐式标成国际版」得到证实——那是 `providerName` 按目录 id 首个匹配的巧合，修正为「ZCode」是正确的。

### 10.4 真机验证范围

**已做**（只读，无副作用）：
- §10.3.1 与 ZCode `model_usage` 按请求 ID 交叉匹配（1,599 交集 / 只在 Jolu 0 条）
- §10.3.3 全库行数对比（只在 Jolu 5,872 条历史完整保留）
- §10.3.5 `provider_key` / `connection_id` 版本归因核对

**未做**：
- **未做真机桌面 UI 对照**：Pencil MCP 在本次会话不可用，无法截取画板 30–39 核对供应商行的覆盖数布局

**真机回填已执行（2026-11-22 00:08，经鼠鼠授权）**

新增 `#[ignore]` 测试 `real_zcode_cancelled_backfill`（`backend/src/native_sources.rs`）对真实 `usage.db` 执行一次回填。执行前已备份数据库到 `usage.db.backfill-bak-20261122-000720`。

| 项 | 预演 | 实跑 |
|---|---:|---:|
| zcode 总行数 | 22,548 | **22,548（不变）** |
| `failed` | 210 → 160 | **210 → 160** ✓ |
| `cancelled` | 0 → 50 | **0 → 50** ✓ |
| 改动行数 | 50 | **50** ✓ |
| 二次调用 | 返回 0 | **返回 0**（幂等）✓ |

独立只读复核（回填后）：

```
success 22,336 | failed 160 | cancelled 50 | null 2  →  合计 22,548
migration_log: zcode_cancelled_backfill ✓
```

测试自己断言了三条硬约束：**行数不变**（绝不删行）、**`failed + cancelled == 回填前的 failed`**（改动量全部来自那一池）、**幂等**（标记已写后再跑返回 0）。

所以 §8 第 5 项「状态差异不再被隐藏」已在本机真实数据上达成。

### 10.5 未解决 / 待确认

| # | 项 | 说明 |
|---|---|---|
| 1 | **供应商行覆盖数的视觉布局** | 画板 30–39 没有「已知 Token / 覆盖请求数」状态。当前实现用既有次要文字样式（`text-note text-jolu-muted`）做副标注，未新增卡片 / 边框 / 列宽。**待鼠鼠确认这个呈现方式** |
| 2 | **「失败数」口径变化** | 从「failed + cancelled」改成只算 `failed`，用户看到的失败数会变小（本机 1599 条：5 → 2）。这是有意的，但属于用户可感知的变化 |
| 3 | **`AnalysisPanel` 未加 Token 覆盖** | 该卡只渲染费用，按「不自行扩展视觉」未动 |
| 4 | **供应商筛选值** | 已改为使用平台 ID 并对同平台选项去重，复核与测试见 §11。 |
| 5 | **归日口径变化的影响面** | `ts` 改用 started_at 后，趋势桶、明细排序、价格时段估算、CSV 导出的时间都会跟着变。这是有意的口径统一，但价格时段估算的结果可能与改前不同 |
| 6 | **回填只覆盖 `native:zcode:main`** | 历史行里若存在其他 ZCode 来源前缀不会被回填；实际查证 ZCode 请求只写过 `main` |

## 11. 后续复核与修复（2026-11-22）

- 供应商和会话筛选改用平台 ID，避免国内版等目录条目 ID 被后端当成未知平台。同一平台的多个版本合并为一个筛选项。
- 明细中的未知状态单独显示和筛选，成功率扣除未知与取消，CSV 与筛选保持一致。旧记录没有原生状态且没有状态码时沿用原有成功口径。
- ZCode 历史请求改为按开始时间归日。即使源库文件未变化，也会触发一次重扫。扫描位点与数据更新在同一个事务中，失败后可以重试。源库已清理的旧记录无法从源库重新取得开始时间，保留现有记录。
- 取消状态回填遇到写入错误时回滚并保留重试机会。源库已经清理的旧记录无法确认原始状态，继续保留现有状态。
- 归日采用开始时间也影响趋势、明细排序、CSV 和按时间查找价格，这是本次采用的统一口径。
- 2026-11-22 只读对账快照：ZCode 源库有 17,127 条请求，Jolu 的 `native:zcode:main` 有 22,999 条；源库中的 17,127 条都能按 ID 在 Jolu 找到，共有记录的开始时间和终态差异均为 0。Jolu 还保留了源库已清理的历史记录，数量会随采集继续变化。
- 前端相关测试 36 项及 TypeScript 类型检查通过；后端用量统计测试 13 项通过，本机来源相关测试 23 项通过、2 项默认忽略。跨午夜归日、写入失败后重试和历史记录保留均由测试覆盖。
- 前端覆盖率文字与状态样式的画板和真实桌面验收尚未完成，当前覆盖率文字沿用现有次要文字样式，新增状态沿用中性色样式，布局待确认。

