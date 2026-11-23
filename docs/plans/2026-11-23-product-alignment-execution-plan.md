# Jolu 产品对齐执行计划

日期：2026-11-23

状态：**🟡 P0 基本完成（P0-1 能力矩阵覆盖不足），🟡 P1 部分完成**（2026-11-23 检验，同日经 [P0 / P1 复查](../verification/2026-11-23-p0-p1-review.md) 修订）。P0 的固定样本、代码检查和可用平台正式库逐条对账已通过，WorkBuddy 缺少客户端样本已按 §3 明确记录。Qoder 正式库现已完成修复后扫描并与 999 条源端记录逐条匹配。P0-1 的能力矩阵只覆盖要求的 11 项能力中的 7 项，验证分档也少两档。P1 的稳定去重、扫描恢复和账号切换等已验证，完整的来源、采集时间、覆盖范围和单位共享契约尚未落地，因此 P1 不能标为完成。

状态标记：✅ 实现或资料已完成；🟡 部分完成或等待验收；⬜ 未完成。带有“验收中”的项目不能作为整体完成依据。

产品基线：[Jolu 产品定位与功能边界](../specs/2026-11-23-jolu-product-positioning.md)

## 1. 目标和完成标准

围绕账号池 → 模型路由 → Agent → 用量、会话和灵动岛的路径，让用户能知道模型从哪里来、请求去了哪里、记录是否完整，以及 Agent 的工具调用能否完成。

计划完成时应满足以下条件。

1. 有一份逐平台、逐 Agent、逐功能的能力矩阵，包含数据来源、验证环境、证据和未验证项。真实账号不可用的项目明确列出缺口，其余项目继续推进。
2. 固定样本和可获得的真实样本按稳定 ID 对账，覆盖计数、状态、时间、Token、账号归属和去重。差异均有可复现原因或明确标为待调查，不能将未知值计算为零。
3. 支持的账号来源可接入路由，调度决策可追踪到来源、所选账号、策略和结果。过期额度、限流、失败、重试和并发不会被当作确定的可用额度。
4. 已声称支持的 Agent、上游模型、协议和工具组合有完整链路验证，至少检查工具请求、执行、结果回传和最终回答，并记录流式及失败行为。
5. 灵动岛、用量、会话和路由对同一可关联事实使用一致的身份、时间和来源语义，保留各自覆盖范围。无法可靠关联时显示未知或未关联。
6. 每个交付阶段有可运行的针对性验证、结果和回归记录。真实环境、模拟与单元测试分开记录。

这些标准约束每个被声明为支持的组合。没有账号、平台权限或真实样本的组合记为未验证，不能写成通过。

## 2. 现状与继承关系

- 多账号连接、账号池、路由分组、失败重试和会话亲和已有基础。本计划以复核和完善为主。
- [用量准确性计划](2026-11-21-usage-data-accuracy-plan.md) 第 11 节记录了较新修复和 2026-11-22 ZCode 真实只读对账。先复核最新结果，不重复实施历史章节中已完成的修复。
- [活跃会话观察计划](2026-11-21-active-session-observation-plan.md) 已记录阶段 1–3 的实施，先确认代码及后续验收，再补缺口。
- `backend/src/gateway/routing_groups.rs` 的 `pace` 策略目前以重置时间近似排序，需要核对是否足以表达各平台额度和新鲜度。
- `backend/src/gateway/route_log.rs` 的流水保留上限约 1000 行，且重试可能形成多行。统计是否受截断或重复影响需要按实际查询路径复核。`day_overview` 通过进程内请求 ID 和倒退幅度推断重启的潜在误合并是代码推断，先写回归复现后再决定修复。
- `backend/src/usage_stats.rs` 的账号归属依赖精确连接 ID，缺失 ID 的原生记录不应猜测账号。
- `backend/src/library/mcp_search.rs`、`backend/src/gateway/tool_loop.rs`、`backend/src/gateway/adapter/claude_bridge.rs` 对应不同的搜索 MCP、服务端工具代执行和订阅工具桥，不能合并视为同一种完整能力。

## 3. 范围和约束

本计划覆盖能力盘点、数据准确性、账号池与路由、工具调用与 MCP、跨组件联动及验收。现有 SQLite、历史数据、用户账号与 Agent 配置应得到兼容保护，具体实现根据测试和现有架构选择。

主窗口唯一设计依据为 `docs/ui/Jolu.pen`。仅能使用 Pencil MCP 读取。缺少的界面或状态标「待确认」，未经用户明确要求不修改画板。代码与测试先行的项目可以独立推进。

真实账号调用、真实工具配置写入及可能产生费用的测试应使用明确授权的账号和环境，优先隔离副本。无样本时记录缺口并推进其他任务。

## 4. 实施阶段

### P0 盘点与可复现基线

**当前状态：🟡 P0 基本完成（2026-11-23 检验，复查后修订）。** P0-2、P0-4 已验证；P0-1 能力矩阵覆盖不足，见下。P0-3 的 8 个固定场景齐备，Claude Code、OpenCode、Qoder、ZCode 均已按稳定 ID 完成正式库逐条对账。WorkBuddy 本机没有客户端，缺少真实样本，已按 §3 记录为环境缺口，不伪报通过。

**需求与任务**

- P0-1 对每个平台、地区、连接类型和 Agent 列出账号识别、额度、签到、模型发现、路由、接线、用量、会话、续跑、Skills、工具调用的能力。区分真实通过、部分通过、模拟通过、无验证记录、不适用及失败。
  - **状态：🟡 部分完成（2026-11-23，复查后由 ✅ 改判）。** 产出 [能力矩阵](../specs/2026-11-23-capability-matrix.md)：13 家 × 7 项能力的实现列（带文件:行号）+ 验证强度列（真实端到端 / 真实只读 / 合成 / 无记录 / 不适用，分开标注）。逐份定档了 68 份验收记录。真实账号不可用的 8 个平台单独列表。另发现 10 处「声明与实现不一致」（矩阵 §4.1–4.10），4.1 与 4.9 已修，其余列为待办。
  - **与本任务要求的差距**：
    - 能力只有 7 列（探测、接线、用量采集、会话读取、Skills、签到、订阅转发）。要求的 11 项里缺**额度、模型发现、续跑、工具调用**；「账号识别」「路由」只间接体现在探测与订阅转发里。
    - 只按 Agent 分，没有按平台、地区、连接类型分。
    - 验证分档是五档，缺本任务要求的「部分通过」和「失败」两档。
- P0-2 记录每项的代码入口、数据源、样本时间、操作步骤、证据链接和限制。核对现有计划、验收及最新代码，识别已完成和仍存在的差距。
  - **状态：✅ 完成（2026-11-23）。** 六项要求逐条对照：
    | 要求 | 落在哪里 |
    |---|---|
    | 代码入口 | 能力矩阵 §1 的 13 家 × 7 能力，每条带 `文件:行号`（22 处引用） |
    | 数据源 | 能力矩阵 §1 的采集路径列；对账报告 §0 的平台 → 源端位置 → ID 字段 → dedup_key 构造表 |
    | 样本时间 | 对账报告首页「样本时间」段：2026-11-23，两侧同会话内只读快照 |
    | 操作步骤 | 对账报告 §8「复现步骤」：固化为 `scripts/acceptance/reconcile-usage-by-id.py`，四平台一条命令 |
    | 证据链接 | 能力矩阵 §2 逐份定档 68 份验收记录（11 处文档引用）；对账报告四平台的交集/只在源端/只在 Jolu/字段差异 |
    | 限制 | 能力矩阵 §3 真实账号不可用的 8 个平台；§4 的 10 处声明-实现不一致（4.1–4.10）；对账报告 §7 待办表 |

    已关联的历史资料：用量准确性、活跃会话观察、Agent 探测、上下文观察、接线重构、统一网关。
- P0-3 建立脱敏固定样本，覆盖多账号、源端删除历史、Token 未提供、跨午夜、状态变化、重试、相同模型名和扫描延迟。记录只读对账方法与差异清单。
  - **状态：✅ 完成（2026-11-23，复查后补齐固定样本）。** 8 个必测场景固定样本全部齐备（复查前「源端删除历史」只有一次真实观察，已补可重复的固定样本）。Claude Code、OpenCode、Qoder、ZCode 均已按 ID 对账通过。WorkBuddy 本机无真实客户端样本，按 §3 留有明确缺口，不作为本阶段失败，也不标成真实验证通过。
    - **逐条对账**（本机真实数据，只读；完整报告见 [P0-3 多平台对账](../verification/2026-11-23-p0-3-multi-platform-reconciliation.md)）：

      | 平台 | 交集 | 只在源端 | 只在 Jolu | 字段差异 | 判定 |
      |---|---|---:|---:|---|---|
      | Claude Code | 3,295 / 3,295 | 0 | 692（源端已清理） | 零 | ✅ |
      | OpenCode | 132 / 132 | 0 | 0 | 零 | ✅ |
      | Qoder | 999 / 999 | 0 | 2（网关行） | 零 | ✅ 修复后正式库对账通过 |
      | WorkBuddy | — | — | 10（网关行） | 不适用 | ⬜ 本机无客户端（按 §3 记录缺口，工具已就绪） |
      | ZCode | 18,872 / 18,872 | 0 | 5,872（源端 30 天清理） | 零 | ✅ |

      对账已固化为 `scripts/acceptance/reconcile-usage-by-id.py`（五个平台一次跑完，`--json` 供机器读，全程只读）。

      对账口径写在对账报告 §0：字段比对复刻 `db::insert_records_into` 的合并语义（首次插入 + 后续取 max + 有变化才更新 ts），不是原始行比对；第一轮没复刻时 Claude 报了 2,730 条假时间差异，复刻后归零。
    - 8 个必测场景的覆盖情况（2026-11-23 复查后补齐 **8/8 有固定样本**；「真实观察」是一次性的本机对账，不计入固定样本）：

      | 场景 | 状态 | 测试 |
      |---|---|---|
      | 多账号 | ✅ | `concurrent_accounts_keep_their_own_rows`：2 条连接 × 4 条交错请求，各账号各 2 条、`unattributed_requests = 0`，验「不能错配、也不能丢进未归属」 |
      | 源端删除历史 | ✅ | 固定样本 `history_survives_the_source_deleting_its_files`（源端删掉旧文件后历史保留、新文件照常采集）；真实观察：只在 Jolu 的 5,872 条即源端已按 30 天清理的历史 |
      | Token 未提供 | ✅ | 固定样本：`providerTokenCell` 三态测试、`total_is_unknown_when_either_part_is_missing`；真实观察：真机 29 条取消/失败 |
      | 跨午夜 | ✅ | `zcode_attributes_a_cross_midnight_request_to_its_start_day` |
      | 状态变化 | ✅ | 四值映射 + 受控回填测试；缺「工具异常退出→状态停留」真机样本 |
      | 重试 | ✅ | `day_overview_counts_requests_not_rows`；缺「所有重试入口」清单式覆盖 |
      | 相同模型名 | ✅ | 两个中转商 `:1017` / `-1017` 不合并的测试 |
      | 扫描延迟 | ✅ | `a_delayed_scan_picks_up_new_rows_without_duplicating`：先扫一批、等模拟延迟后补扫，验新行入库且旧行不重复 |
    - **Qoder 采集缺口已修复（2026-11-23，P1-3「按 P0 差异修复真实问题」）**：新增 `backend/src/qoder_usage.rs` 读取 `~/.qoder/shared_client/cache/db/local.db` 的 `chat_message`，字段语义按对账报告 §3.3 实测结论实现：`dedup_key = {session_id}:{message_id}`（**不是** `request_id`——那是「一轮对话」，一轮里实测最多 57 次调用）；`input_tokens = prompt_tokens` 原始值，`cache_read_tokens = cached_tokens`，并标记 `includes_cache`；消费侧只扣一次缓存，避免缓存被重复扣除（999/999 均满足 `cached <= prompt`）；`model_key` 是档位不是模型名，不写 `model`。同时修了来源可用性判定（原本只按 `~/.qoder/projects` 是否存在，本机没有该目录导致整个来源永不启用），并限定只有 `global` 实例读主账本（`cn` 实例共用会让同一批记录被两个 source 各写一遍——这条由 `qoder_source_is_available_from_the_ledger_alone` 抓到，第一次跑是 4 行不是 2 行）。
    - **真实账本验证通过**：`real_qoder_ledger_scans_into_a_temp_db`（`#[ignore]`）把真实账本扫进临时库，读出 **999 条**、新鲜输入 8,078,210 + 缓存读 38,745,227 + 输出 619,650 = 47,443,087 Token，与独立 Python 逐项核对一致，零重复键、零错误。随后应用正式扫描写入 `usage.db`，只读对账确认 999 / 999 交集、零源端遗漏、零字段差异；另外 2 行是网关记录。重复扫描仍为 999 条且无重复键。
      - 单测 8 条覆盖：存原始 prompt 不提前扣缓存、`request_id` 不是去重键、非 assistant / 空 token 行跳过、`model_key` 不冒充模型名、时间过滤、缺表报错而非静默空、缺一项时总量未知、坏行逐行跳过并计入扫描错误（后两条为复查补充）。
      - 复查后改为按主库 / WAL 内容戳记判断：账本没变时不重读（`an_unchanged_qoder_ledger_is_not_reread`）。
    - **WorkBuddy**：本机全盘无客户端目录，无法对账——**按计划 §3「无样本时记录缺口并推进其他任务」处理，不视为 P0 阻塞**。对账工具已就绪：`reconcile-usage-by-id.py` 新增 `read_workbuddy()`（字段映射复刻 `native_parse::workbuddy`），并与 Rust 侧新增的 `workbuddy_shapes_that_the_reconcile_script_must_match`（四种形状逐字段断言；形状取自解析器已支持的字段组合，不是从真实客户端采到的样本）完成交叉验证，4/4 一致。换机执行 `--platform workbuddy` 一条命令即可补齐。详见 [对账报告 §4](../verification/2026-11-23-p0-3-multi-platform-reconciliation.md)。
    - **「状态变化」与「重试」**：固定样本已有；真机全链样本仍缺（需真实工具异常退出 / 全部重试入口的真实触发）。
    - **新暴露的产品问题（已修复并完成正式复验）**：Qoder 采集缺口来自读取位置错误。现改为读取 `local.db` 的 `chat_message`，临时库复验和应用正式扫描均完成，正式 `usage.db` 与源端 999 条逐条匹配。`model_key` 仍代表档位而非真实模型名，模型名缺失时继续保留未知。
- P0-4 检查请求 ID 重启碰撞和流水截断对统计的影响，先建立可复现测试。无法复现时记录测试覆盖与观察结果。
  - **状态：✅ 完成（2026-11-23）。** 核对结果：
    - **重启碰撞**：已有 `day_overview_counts_requests_not_rows`（`insert(301, ..., Some(2), Some(5))` 明确验「不能并进上一轮同号请求」）
    - **旧行缺身份**：已有 `request_counts_keep_restarts_and_legacy_rows_separate`（`None` 身份的行独立保留）
    - **按请求而非按行**：同上两条覆盖
    - **流水截断**：本次补 `a_truncated_log_is_reported_as_partial`，并修了一处真缺陷 —— `partial` 判定硬编码 `1000` 改为 `KEEP_ROWS`，且 `>` 改 `>=`（行数**正好**到上限时也无法排除刚删过一批）。已验证：改回 `>` 测试立刻失败
    - **旧数据兼容**：本次补 `request_uid_column_migrates_and_old_rows_stay_unknown`（模拟旧库无该列，验迁移补列、旧行留 NULL、新行可写、重复 init 幂等）

**交付与验收**

能力矩阵和基线报告可追溯到具体版本、来源和验证记录；固定样本重复运行结果一致；每个已知差异有证据、影响范围和后续任务。缺真实账号标记未验证。

**当前验收结论：🟡 P0 基本完成（2026-11-23，复查后修订）。** 逐项对照：
- P0-1 能力矩阵 🟡（13 家 × 7 能力 + 68 份验收记录定档；缺额度、模型发现、续跑、工具调用 4 项能力，缺「部分通过」「失败」两档）
- P0-2 代码导航 ✅
- P0-3 固定样本 ✅（8/8 场景，「源端删除历史」复查后补齐）+ 可用平台正式库逐条对账 ✅（Claude Code 3,295 / OpenCode 132 / Qoder 999 / ZCode 18,872，字段差异均为零）
- P0-3 WorkBuddy 🟡 → **按 §3 记录真实样本缺口**：本机全盘无客户端目录（`~/.workbuddy*` / `~/.codebuddy*` / `*WorkBuddy*` 三处 AppData + home 均无），不伪报通过。对账脚本已支持 WorkBuddy 并与 Rust 解析器完成四种形状（非真实客户端样本）的逐字段交叉验证，换机后一条命令即可补验
- P0-4 请求 ID 与流水截断 ✅

按计划 §1 的完成标准第 1 条「真实账号不可用的项目明确列出缺口，其余项目继续推进」——WorkBuddy 属于「真实账号不可用」，缺口已明确列出并给出可执行的补法。

P0-3 的 8 个场景（多账号、源端删除历史、Token 未提供、跨午夜、状态变化、重试、相同模型名、扫描延迟）均有固定样本。逐条对账工具已覆盖 Claude Code / OpenCode / Qoder / ZCode，当前有源样本的四家正式库对账均通过；WorkBuddy 因本机没有客户端，仍缺真实源端样本，已明确记录为环境缺口。

### P1 统一数据语义与准确性

**当前状态：🟡 P1 部分完成（2026-11-23）。** P1-2 的去重与幂等、P1-3 的数据问题修复及正式复验已完成。P1-1、P1-4 已共享 Token 覆盖状态，但完整的来源、采集时间、数据时间、覆盖范围和单位契约尚未实现，因此 P1 整体仍有实质任务未完成。

**需求与任务**

- P1-1 梳理连接、平台、账号、Agent、会话、用户请求与路由尝试之间的身份关系。为来源、采集时间、数据时间、覆盖范围、单位、已知程度定义共享契约，复用现有存储与类型，按需迁移。
  - **状态：🟡 部分完成（2026-11-23）。**
    - **身份关系**：持久化 `request_uid` 贯穿路由流水 / 上下文快照 / 请求统计，同一用户请求的多次路由尝试可关联（重启后由 `gateway_request_sequence` 独立分配，见 P0-4）。
    - **共享契约**：`backend/src/data_contract.rs` 的 `Coverage`（复查后字段私有化，只能经 `of` / `from_counts` 构造，保证「零条已知时总量未知」）定义「未知不是零、负数当未知、已知小计保留」与 known / partial / missing 三态，已接入三处 —— 用量概览 `token_coverage`、会话卡片 `SessionTokens`、灵动岛 `LiveUsage.today_token_coverage` → 前端 `todayTokenView()`。路由侧只有会话卡片 `GatewaySession.tokens`（复用 `SessionTokens`）接了覆盖状态；路由流水与 `day_overview` 没有 Token 覆盖。
    - **一处有意的语义差异**（已加注释与测试）：`Coverage.total` 是「已知部分之和」，而会话卡片的总 Token 要求四个分项全部可信（缺一项就返回 `None`），两者不能混用。
    - **未完成的部分**：原设计的 `DataOrigin` / `Measured`（来源 + 采集时间 + 覆盖范围 + 单位四元组）尚未保留。当前共享契约只统一了 Token 覆盖状态，不能声明 P1-1 的完整元数据契约已完成。P4 跨组件联动需要这组字段时再按实际消费方引入。
- P1-2 对可证明相同的记录制定稳定去重规则。区分一次请求与多次尝试，保留未知归属及冲突证据；重复扫描、失败重试和进程重启均应幂等。
  - **状态：✅ 完成（2026-11-23，复查后补齐「缺 ID」并修正取消判定与锁等待）。** 已处理一次请求与多次尝试的区分、进程重启后的错误合并、并发请求区分、数据库锁忙（写入排队、请求不等锁）、部分拒绝和失败记录、源端记录缺稳定 ID（`records_without_a_stable_id_are_not_given_a_made_up_key`）；取消入口、重试幂等、进程中断、扫描期间并发改写均有固定样本并通过有效性验证。
    - **本轮补齐「取消」入口**：此前流式响应的流水在响应头到手时就写成 `ok`，客户端中途断开（Ctrl-C、关终端、读超时）时那行不会被改判——「读到 40% 就断」被记成完全成功。新增 `route_log::CancelWatch`：包住响应 body，客户端中途断开（流没读完就被 drop）时把该行 `result` 从 `ok` 改判为 `cancelled`；上游在流中途报错时改判为 `failed`（复查发现原实现会把上游错误也记成取消，已修）。**只改 `ok` 行**，响应头阶段已定的 `failed` / `rejected` 不覆盖。改判靠流水行 id 定位而不是猜 `(ts, request_uid)`。
    - **为什么改原行而不是新写一行**：同一请求已有 `ok` 行，再写 `cancelled` 会让「一次请求」变成两行，正是 P1 要消除的「把尝试当请求」。
    - **测试**（8 条）：`a_stream_read_to_the_end_is_not_a_cancellation`（读完不算取消——最容易写错的一侧）、`a_stream_dropped_before_the_end_is_a_cancellation`（真流 drop）、`an_upstream_error_mid_stream_is_a_failure_not_a_cancellation`、`a_queued_row_is_flipped_to_failed_when_the_upstream_errors`（生产路径：排队写入 + 改判）、`a_cancelled_stream_flips_its_ok_row_to_cancelled`、`a_failed_row_is_not_overwritten_by_cancellation`、`cancelling_a_missing_row_reports_no_change`、`record_returns_the_row_id_it_wrote`。已验证：把 `poll_next` 里「放弃回调」那一支改成 `if false`，第一条测试立刻失败；去掉 `Err` 分支，第三、四条失败。
    - **流水写入不再卡请求线程**（复查 C-1）：流水与改判排队交给单独的写入线程（`route_log::submit`），按提交顺序执行；`request_uid` 改为一次预留 1,000 个号段、请求在内存取号（`activity::UidBlock`），号段用尽且库忙时本次记为未知，不等锁。
    - **待确认**：「已取消」是路由流水页的新状态，路由流水页签没有 V6 画板，界面展示待鼠鼠确认。
    - **待真机核实**：客户端收到结束事件（`message_stop` / `[DONE]`）后不等流读完就断开时，会被记成取消。需用真实 Claude Code / Codex 各抓一次确认。
    - 前端 `RouteLogPanel` 新增「已取消」档并带说明；`counts_since` 的第三项（非 ok 非 rejected）现在包含 cancelled，灵动岛不会少报。
    - **其它重试入口幂等性 + 进程中断**（已补齐并验证）：四个写入入口在同一记录上叠加只留 1 行（`every_retry_entry_point_is_idempotent_on_the_same_record`）、`(source, dedup_key)` 而非仅 `dedup_key` 作用域（`dedup_key_is_scoped_to_source_not_global`）、kill -9 事务中途（`a_scan_killed_mid_transaction_is_recovered_without_loss_or_duplication`，子进程 `abort`）、半行尾行（`a_half_written_tail_is_picked_up_after_the_source_finishes_it`）。
    - **扫描期间源端被并发改写（2026-11-23 补齐）**：两条走 `scan_file` 真实事务路径的固定样本。
      - `a_file_growing_during_the_scan_is_picked_up_next_round_without_duplicates`（边扫边追加）：`scan_file` 先 `metadata()` 拿 `size`、再 `file.take(size-offset)` 只读那个长度，扫描期间新增的字节不在本次范围内。断言：上一轮只采到当时看见的 2 行；下一轮补上第 3 行且只新增 1 行；三行都在、无重复键；第三轮 `records_inserted == 0`。
      - `a_truncated_or_rewritten_file_is_rescanned_from_the_start`（同名截断/重写）：A **文件变短**（`saved.offset > size`）——截断后只剩一行新 ID，断言读到新行、旧的 3 行保留、新行 Token 正确；B **等长内容替换**（`saved.offset == size && mtime 变了`）——构造字节数完全相同的替换内容，断言仍要重扫。
      - **两个判定分别验证有效性**：去掉 `saved.offset > size` 分支 → A 失败（`截断后要能读到新行`，左 0 右 1）；去掉 mtime 分支 → B 失败（`等长替换也要重扫`，左 0 右 1）。
    - **不重复的根本保证**是数据库层 `(source, dedup_key)` 唯一约束：即使两个扫描器同时读同一段字节，`insert_records_into` 也只写一行。上面的样本锁的是「不丢」（位点判定）与「不重复」（去重兜底）两侧。
- P1-3 按 P0 差异修复真实问题，涵盖统计、会话、扫描位点与账号切换。迁移兼容旧记录和已清理的源端历史，更新数据与扫描位点保证一致，可失败重试和回滚。
  - **状态：✅ 完成（2026-11-23）。** 已修复已发现的统计重复、静默失败和概览重复计数问题；扫描位点、中断恢复、并发采集、账号切换、旧数据库升级均有固定样本并通过有效性验证。Qoder 读取器已在真实账本临时库复验，并在正式 `usage.db` 扫描后完成逐条对账。
    - **已覆盖**：进程中断（kill -9 中途、半行尾行两种形态）、多入口重试幂等、源端删除历史（真实对账 5,872 条）、并发采集（`concurrent_accounts_keep_their_own_rows`）、扫描延迟（`a_delayed_scan_picks_up_new_rows_without_duplicating`）、扫描位点恢复（`resume_offset` 在位点缺失 / 越界 / 不在行边界时从头读，已由中断测试从外部锁住）。
    - **账号切换（本轮补齐，2026-11-23）**：两条固定样本，覆盖两种真实形态。
      - `switching_accounts_rescans_without_duplication_or_reattribution`：A、B 两个账号并存。先扫出 A 名下的记录并归因，插入 B 后再扫——断言记录数不变（`records_inserted == 0`）、`connection_id` 一条都没变、B 不凭空得到历史记录；归因视图在两条 auth 连接且无端点事实时落「平台」档而不是猜。**已验证有效性**：在扫描入口注入「每次扫描把旧记录改归到最新账号」，测试立刻失败（「旧记录仍归 A，不能被改判成 B」，左 0 右 2）。
      - `deleting_the_old_account_keeps_history_and_does_not_hand_it_to_the_new_one`：旧账号被删、新账号接管。断言历史记录不丢（AGENTS.md 要求）、`connection_id` 仍是产生它的旧 id 不被改写、悬空 id 的归因落「平台」档且不返回不存在的连接 id。
    - **旧数据库升级（本轮补齐，2026-11-23）**：两条固定样本，一条测迁移逻辑、一条测真实文件路径。
      - `an_old_schema_upgrades_without_losing_rows_or_changing_their_meaning`：从**最早的 schema**（`requests` 只有 12 列、无 `*_known` / `effort` / `connection_id` / `provider_key`；`connections` 无 `base_url` / `slug` 等；`sessions` 无 `activity_state`；`gateway_route_log` 无 `request_uid`；`scan_state` 无 `parser_*`）开始跑真实 `init`。断言：**44 个新增列全部存在**、旧记录一条不少且值不变、`*_known` 默认是 1（已知）而不是 0、无归因证据的 `connection_id` / `provider_key` 留 NULL、旧流水 `request_uid` 留 NULL 不编造、`serve_enabled` 默认 0、`activity_state` 默认 `done`、`parser_initialized` 默认 0（首次扫描重建一次）、新列可写可读、**连跑三次 `init` 行数与旧值都不变**。
      - `an_old_database_file_opens_reads_and_reopens_cleanly`：用**真实文件**（WAL + `foreign_keys=ON` + `PRAGMA optimize`）走 `db::open`，而不是内存连接。断言迁移后 `open_reader` 只读池能看到新列、关掉再开一次仍幂等。
      - **已验证测试有效性**：把 `input_known` 的 `DEFAULT 1` 改成 `DEFAULT 0`（最容易犯的错——会让历史 Token 全部变「未知」、统计从彩色变灰），第一条测试立刻失败（`左 (0,2) 右 (2,2)`）。
    - **正式复验结果**：Qoder 源端 999 条与正式 `usage.db` 逐条匹配，源端遗漏 0、字段差异 0；额外 2 行均为网关转发行。WorkBuddy 因本机无客户端仍缺真实样本，另有完整数据元信息契约未落地，这些不属于 P1-3 的代码修复缺陷。
- P1-4 用同一数据契约向用量、会话、路由和灵动岛提供查询。官方额度、本地记录和网关记录各自保留来源及覆盖范围。
  - **状态：🟡 部分完成（2026-11-23）。** 已建立共享契约 `backend/src/data_contract.rs`：`Coverage::of()` 定义「未知不是零、负数当未知、已知小计保留」三条规则，`state()` 统一 known / partial / missing 三态。接入点：
    - **用量概览** `token_coverage`：`usage_stats.rs` 用 `Coverage::of()` 算，`ProviderRow` / `ModelRow` 用 `known_subtotal()`，模型分布按 `known_tokens` 排序。
    - **会话卡片** `SessionTokens`：含一处语义修正——会话的 `total` 要求四分项全齐，而 `Coverage.total` 是已知部分之和，两者不能混用，已加注释与测试 `token_total_requires_all_parts`。
    - **灵动岛**（本轮新增）：后端 `LiveUsage.today_token_coverage` 由 `db::today_token_coverage()` 算（复用 `EXCLUDE_DEGENERATE_SQL`，与 `today_fresh_tokens` 同口径）；前端纯函数 `todayTokenView()`（`lib/islandPresentation.ts`）把 coverage 翻成三态展示，经典样式走 `MetricRow` 的 `hint`、胶囊样式走 footnote，两边共用同一个值。关键点：`today_tokens` 为 null 时靠 coverage 区分「今天没有记录」与「有记录但都没给 Token」；**没有记录时仍沿用后端的 `today_tokens` 判断**（原生来源可能是还没扫到，不能冒充真零）。测试 `todayTokenView` 6 条（含总量兜底优先级）+ 后端 `today_token_coverage_separates_empty_from_all_unknown` 与 `live_usage_carries_today_token_coverage`（锁住「灵动岛入口确实把覆盖带出去」）。
    - **未完成部分**：路由侧只有会话卡片 `GatewaySession.tokens` 间接复用 `SessionTokens`，路由流水与 `day_overview` 没有 Token 覆盖状态。完整的「来源 + 采集时间 + 覆盖范围 + 单位」四元组（原设计的 `DataOrigin` / `Measured`）尚未保留，因此 P1-4 目前只能判为部分完成。

**交付与验收**

固定样本逐 ID 对账通过，重复导入与中断重试不增加重复记录；跨午夜、账号切换、缺 ID、历史兼容和并发有针对性测试。真实样本提供源与 Jolu 的数量、交集、仅一侧记录及字段差异，解释扫描时间、清理历史和未知值。不能以两个总数相近代替逐条对账。

**当前验收结论：🟡 P1 部分完成（2026-11-23）。** 逐项对照 §1 的完成标准：
1. 能力矩阵 🟡（13 家 × 7 能力 + 68 份验收记录定档；能力项与分档不全，见 P0-1）
2. 按稳定 ID 对账 ✅（Claude Code 3,295 / OpenCode 132 / Qoder 999 / ZCode 18,872 正式库字段零差异；WorkBuddy 缺少客户端样本，已按 §3 记录环境缺口，与 P0 结论一致）
3. 账号来源可接入路由、决策可追踪 —— **不在 P1 范围**（P2-4；P1 只提供 `request_uid` 关联与流水截断标记，不等于选路可解释）
4. Agent / 上游 / 协议 / 工具组合链路 —— **不在 P1 范围**（P3）
5. 四处共用一致语义 🟡（`data_contract::Coverage` 已接入用量 / 会话 / 灵动岛，路由复用 `SessionTokens`；完整来源、采集时间、覆盖范围和单位四元组尚未统一）
6. 每阶段有可运行针对性验证 ✅（已完成部分均有固定样本，本轮新增测试已验证有效性）

已知遗留：WorkBuddy 无客户端样本；Qoder `local.db` 布局目前只在单台机器确认；完整 `DataOrigin` / `Measured` 四元组尚未落地；旧库升级只覆盖「最早 schema → 当前」，中间版本未逐一构造。

复查（[P0 / P1 复查](../verification/2026-11-23-p0-p1-review.md)）新增的遗留：
- **Qoder 新旧去重键不一致**：旧的 projects 日志解析器优先用 `request_id`，账本读取器用消息 id，两者都写进 `native:qoder:global`。目前账本存在时只扫账本，两套不会同时写；但如有旧 projects 记录，再扫账本会让同一次调用变成两行。旧日志里 `request_id` 是否也代表「一轮对话」本机没有样本，无法确认。需要旧日志样本后再决定统一键与迁移方案。
- **流式结束事件后的提前断开**：可能被误记为取消，待真机核实（见 P1-2）。
- **「已取消」界面状态**：路由流水页签无 V6 画板，待确认。
- **Claude Code 只在 Jolu 的 692 条**与「早于源端最早记录」的 698 条差 6 条，对账报告解释为毫秒边界附近，未复现，待调查。

### P2 账号池与可解释路由

**需求与任务**

- P2-1 核实每类连接是否同时具备额度查询、模型发现和实际转发能力，识别授权过期、地区、协议和模型限制。
- P2-2 评估并完善分组策略，明确余额、单位、重置时间、快照过期、账号健康、限流及并发的含义。数据不足时采用明确的回退规则，并提供原因。
- P2-3 复核会话亲和、失败转移、重试和流式中断。记录原始请求及各次尝试的关联，避免将一次请求当作多次成功或重复消耗。
- P2-4 让已有路由查询能解释选中的来源、账号、模型、策略及结果。缺数据时保留未知。涉及新界面遵循画板约束。

**交付与验收**

对相同条件使用固定场景验证选路、额度过期、限流、失败转移、并发与账号切换；重试记录和统计能正确对应；至少对有授权样本的来源执行端到端请求并记录实际模型、账号、计费及结果。未提供用量时明确标记。

### P3 Agent 工具调用和 MCP

**需求与任务**

- P3-1 建立矩阵，逐项记录 Agent、模型上游、协议、认证方式、工具类别、是否流式、调用结果、回传、用量和证据。分别测试 Agent 自带工具、MCP 搜索 / 抓取、上游服务端工具，以及多种工具混用。
- P3-2 核实现有工具链的边界，包括服务端工具代执行轮数上限、转非流式行为、混合调用、失败、超时和取消。对可复现的缺陷做针对性修复，并保证结果进入后续模型请求。
- P3-3 调研并设计通用 MCP 配置管理，覆盖导入、冲突、启停、作用域、凭据、路径与回滚。先复核现有 Library 能力与各 Agent 的配置格式；界面和具体支持范围依据产品设计确认。
- P3-4 依据矩阵和实际需求评估 MCP 汇聚、按 Agent 分发和按需发现。Magpie 调查版本的路由器文档描述的是 tools；resources 和 prompts 应单独评估。先验证兼容性、权限隔离及项目上下文，再确定实现范围。

**交付与验收**

对每个声明支持的组合，能复现模型发起调用、工具执行、结果回传和最终回答。分别记录成功、工具失败、超时、取消、混合工具和流式行为，核对敏感信息与调用用量。P3-3、P3-4 如需新增界面，先取得对应画板或标记待确认，不阻塞无界面的验证和修复。

### P4 联动与用户验收

**需求与任务**

- P4-1 对照画板和统一数据契约核实灵动岛、用量、会话和路由的连接身份、来源、时间范围及数据状态。
- P4-2 验证查看会话、原工具继续、压缩和 Markdown 交接的工具范围。跨 Agent 交接基于现有能力提出方案，涉及产品体验时待设计确认。
- P4-3 用真实可用的账号和 Agent 走通连接 → 路由 → 工具调用 → 用量查看 → 会话查看 → 灵动岛联动。记录账单或额度与本地记录的口径差异和采集延迟。

**交付与验收**

完成端到端验收记录、画板与运行界面对照、数据差异解释及未验证矩阵。一个来源的未验证项不掩盖已验证组合的结果。

## 5. 依赖和可并行关系

P0 为后续基线。P1 的数据契约应先于 P4 的跨组件联动；P2 可以在 P0 明确路由问题后与 P1 并行，但共享身份和统计字段须遵循 P1 契约；P3 的现状盘点和工具链测试可以与 P1、P2 并行。P4 在相关链路可用后逐项验收。无需等待某个缺失的真实账号才处理其他平台。

## 6. 验证方法与通过口径

| 类型 | 方法 | 可说明的范围 |
| --- | --- | --- |
| 单元 / 固定样本 | 固定时间、ID 和输入，覆盖正常及边界 | 可重复的代码行为 |
| 集成 / 模拟 | 模拟上游、Agent、流式、故障和并发 | 协议与失败处理，记录模拟条件 |
| 真实只读对账 | 记录源端与 Jolu 快照时间，按 ID 比较 | 当次样本与字段的采集准确性 |
| 真实端到端 | 已授权账号和 Agent，记录实际调用与结果 | 指定版本、账号、平台及场景的真实行为 |
| 界面对照 | Pencil 读取画板并与运行页面比对 | 有对应画板的可视状态与交互 |

必测边界包括本地午夜与时区、增量扫描延迟、旧记录、源端清理、账号切换、缺失 ID、同名模型、进程重启、并发、部分失败、重复重试、流式断开、配额快照过期、缓存 Token 和上游未返回用量。配置写入测试覆盖原文件备份、部分失败恢复、重复接入、断开还原与用户并发修改。测试凭据和个人内容须脱敏。

## 7. 风险与应对

| 风险 | 应对 |
| --- | --- |
| 第三方认证、接口和条款变化 | 记录工具版本与授权方式，按平台独立适配和验证，失败保留清晰状态 |
| 官方额度、Token 和费用口径不同 | 保留来源、单位、时间窗口、缓存语义和覆盖范围，未知或估算明确标记 |
| 错误归属、重复采集或尝试被重复统计 | 仅使用可证明的身份关联，区分请求与尝试，覆盖扫描重试和重启 |
| 历史迁移或配置写入造成数据损失 | 兼容旧数据，事务化与幂等，验证备份、回滚、失败恢复和用户改动 |
| 自动切换影响会话或实际消耗 | 保留亲和和尝试链路，验证限流、并发及流式中断，说明可能产生的计费 |
| 额度快照陈旧导致误选 | 记录时间、过期状态和单位，策略有可验证的回退规则 |
| 密钥、会话内容泄露 | 使用现有安全存储，日志与样本脱敏，限制工具权限和项目目录上下文 |
| MCP 工具不兼容或执行越权 | 逐组合测试权限、工作目录、工具列表和结果回传，失败时给出可见状态 |
| 高采集频率导致卡顿或资源占用 | 记录现状并对比扫描耗时、内存、数据库增长和 UI 响应，按测量结果优化 |
| Windows 路径、文件锁和安装环境差异 | 覆盖用户目录、空间及非 ASCII 路径、文件占用、权限和进程重启 |

## 8. 交付记录模板

每个任务完成后记录：任务编号、代码版本、修改范围、数据来源及样本时间、测试命令与结果、真实环境和授权范围、验证证据、遗留差异、风险及下一步。真实验证、模拟验证和未验证分别列出。不得将缺少样本写成通过。

## 9. 代码和参考导航

- 账号、采集及统计：`backend/src/native_sources.rs`、`backend/src/usage_stats.rs`、`backend/src/session_threads.rs`
- 路由及分组：`backend/src/gateway/route_log.rs`、`backend/src/gateway/routing_groups.rs`、`backend/src/gateway/wire.rs`
- MCP 和工具：`backend/src/library/mcp_search.rs`、`backend/src/library/mod.rs`、`backend/src/gateway/tool_loop.rs`、`backend/src/gateway/adapter/claude_bridge.rs`
- 界面：`frontend/src/views/UsageView.tsx`、`frontend/src/views/SessionsView.tsx`、`frontend/src/views/GatewayView.tsx`、`frontend/src/components/island/`
- 参考项目：[Magpie 固定版本](https://github.com/yetone/magpie/tree/488e20ad921797d91b18812be30e0d2f407ea203)、[CC Switch](https://github.com/farion1231/cc-switch)、[CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI)、[Sub2API](https://github.com/Wei-Shaw/sub2api)。外部项目能力以核查版本和原始文档为准。
