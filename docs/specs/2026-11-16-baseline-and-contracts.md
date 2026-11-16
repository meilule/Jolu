# 开发基线与接口契约（4.11 收尾）

日期：2026-11-16
对应：`docs/plans/2026-11-09-complete-remaining-work.md` 4.11；原始计划在 `docs/plans/2026-10-21-jolu-development-roadmap.md` 第六节 P0 / P1。

## 结论

P0「开发基线」和 P1「统一契约」原本是开工前要先写的两份文档（计划中的文件名是 `docs/specs/2026-10-21-jolu-development-baseline.md` 与一份接口契约文档），但一直没有单独建。之后各模块已按 `Jolu.pen` 实现，它们的大部分检查项分散落在了别的文档和代码里；涉及「旧项目数据兼容」的几条，与 AGENTS.md「本项目是新仓库，不保留任何旧项目的兼容或迁移逻辑」冲突，已作废。

本文不补写过时的计划，只做三件事：

1. 逐条写清 P0 / P1 检查项现在的去向（§1、§2）；
2. 补上目前唯一没有文档的部分：**后端发给前端的事件目录**（§4）；
3. 给出命令的分布（§3），以及核对中发现的待查问题（§5）。

命令与事件随代码变化，本文是 2026-11-16 的快照（提交 `51c0285` 之后）。以代码为准。

## 1. P0 · 建立唯一开发基线

| 检查项 | 现在的去向 |
|---|---|
| 已有功能清单（可复用 / 需改造 / 新增 / 未验证，附证据文件） | AGENTS.md「UI 设计依据」表的「现有实现」列：按模块列出实现文件与验证状态，随功能更新 |
| 灵动岛 01–04 与 111–132 的唯一基线 | 已定：主窗口「灵动岛」页按 01–04；浮动灵动岛分两种样式并存，经典样式以 A2（111–132）为准、胶囊样式以 A3 为准，由设置 `island_style` 切换（AGENTS.md 表下说明） |
| 每个平台的登录、API Key、模型、余额、额度、会话、Skills、继续执行能力核对 | `docs/todo/2026-10-22-unverified-integrations.md`（「已做 / 缺少条件」逐平台维护，区分合成测试与真实服务）；画板 29「平台能力规则」；代码侧 `backend/src/platforms.rs`（`supports` / `limitation`）与 `frontend/src/lib/providerCatalog.ts` |
| 平台展示名、OpenCode 与自定义的独立标识 | 平台目录 `platforms.rs` / `providerCatalog.ts` 各有独立 id；「旧 Trae 数据保留方式」**作废**（新仓库不兼容旧项目数据） |
| 数据目录、应用标识、设置格式、窗口标签、更新渠道 | 应用标识与命名约定见 AGENTS.md §4（`Jolu` / `jolu` / `JOLU_`）；更新渠道见 `docs/release-versioning.md` 与 AGENTS.md §2；窗口标签是 `lib.rs` 的 `ISLAND` / `MAIN` / `STARTUP` 常量；「更名兼容方案」**作废** |
| 建立一次开发基线检查 | AGENTS.md §3 常用命令（前端类型检查、纯逻辑测试、后端全量测试）与 §5「验证强度跟改动风险匹配」 |

验收「每个按钮都能对应已有接口、拟新增能力或明确不可用状态」由 AGENTS.md §4（未知值、真实的零、不支持、失败、过期分别表达；不得按 UI 推断能力）持续约束，不再单独建表。

## 2. P1 · 统一契约和迁移基础

| 检查项 | 现在的去向 |
|---|---|
| 冻结对象与接口字段；事件带实体标识、序号与错误码 | 对象定义在 `frontend/src/types.ts`、各 `*Api.ts` 的 DTO 与后端对应结构体；事件目录见本文 §4。**2026-11-16 起**（鼠鼠选定「集中管理名字 + 快照修订号」）：事件名集中在后端 `events.rs` 与前端 `lib/events.ts`，前端 `listenEvent` 按名字推断载荷类型，后端测试核对两边名字集合一致；会乱序覆盖的整份快照 `settings-changed` 带修订号（见 §5）；网关在途事件仍按自己的序号（`gateway/activity.rs`）；错误码不加（只有 `startup-error` 带错误，本就是一段文字） |
| 区分「能力支持」与「本机已配置」；未知平台不回落其他平台 | 后端 `platforms::supports` / `known`；额度路线集中在 `quota_registry.rs`（目录声明了订阅额度的平台都要有路线，有一致性测试）；前端 `PlatformId` 放宽为「已知 id 或任意字符串」，`fictionalProvider.test.mjs` 让未登记平台走一遍全部纯函数，不补假数据 |
| 可重复执行的迁移与备份恢复 | 数据库在 `db.rs` 用 `CREATE TABLE IF NOT EXISTS` 与可重复执行的 `ALTER TABLE … ADD COLUMN`（失败即已存在，忽略）；数据导出 / 导入带 `schema_version`（1 / 2），不认识的版本明确拒绝。AGENTS.md §4 要求「数据结构变更提供可恢复的迁移」 |
| 新 UI 查询加请求身份，防切换筛选后旧响应回填 | **2026-11-16 起**（鼠鼠选定「建工具并迁移会切换的查询」）：`lib/latestGate.ts`（`hooks/useLatestGate.ts`）每次请求领号、只认最新一号，见 §5 第 4 条的迁移范围。effect 内靠清理函数置 `cancelled` / `alive` 的 16 处本就能防切换后的旧响应，保持原写法 |
| 凭证存储后端隔离；迁移、日志、导出不泄露原值 | AGENTS.md §4「API Key 和认证信息只在后端处理，前端只返回掩码；不得写入普通日志、统计导出和诊断报告」；各凭据模块按此实现 |
| 用旧版数据副本验证升级、注入迁移失败 | 旧版数据部分**作废**（新仓库）；本仓库自身的加列迁移由各模块测试覆盖 |

计划里点名的三个测试 `migration_preserves_legacy_platform_ids`、`query_identity_ignores_stale_response`、`capability_unknown_is_not_supported` 都没有建。第一个随「旧数据兼容」作废；后两个的意图分别由 `useQuota` 的 `owner` 判断和 `fictionalProvider.test.mjs` 部分覆盖。

## 3. 命令分布（共 231 个 `#[tauri::command]`）

命令名就是函数名，前端 `invoke("<函数名>")` 调用；注册在 `lib.rs` 的 `generate_handler!`。4.10 之后 `lib.rs` 不再定义命令。

| 位置 | 个数 | 内容 |
|---|---|---|
| `commands/routing.rs` | 47 | 模型路由：网关状态、对外访问与客户端 Key、中间件、分组、预设、流水 |
| `commands/preferences.rs` | 22 | 设置、数据目录、清理、开机自启 |
| `commands/providers.rs` | 18 | 供应商连接：增删改、凭据、模型清单、本机导入 |
| `skills.rs` | 17 | Skills 管理 |
| `commands/agents.rs` | 16 | Agent 探测、版本、接线、自定义工具 |
| `commands/usage_sessions.rs` | 15 | 用量统计、本机会话、导出 |
| `automation.rs` | 15 | 自动化任务与运行 |
| `commands/panel.rs` | 12 | 主窗口、启动、页面跳转 |
| `commands/island_window.rs` | 12 | 灵动岛窗口 |
| `session_handoff.rs` | 10 | 压缩会话与交接 |
| `commands/capabilities.rs` | 10 | 隐私脱敏、联网搜索、Library |
| `updater.rs` | 6 | 检查与安装更新 |
| `commands/quotas.rs` | 6 | 额度、余额、API 用量 |
| `data_files.rs` | 5 | 数据文件导入导出 |
| `context_menu.rs` | 5 | 右键菜单窗口 |
| `provider_browser_auth.rs` | 4 | 浏览器授权 |
| `daily_checkin.rs` | 3 | 一键签到 |
| `tray_summary.rs` / `service/mod.rs` / `net.rs` | 各 2 | 托盘摘要 / 服务层 / 联网方式 |
| `storage_stats.rs` / `notify.rs` | 各 1 | 存储统计 / 通知 |

## 4. 事件目录（后端 → 前端，共 28 个）

名字与载荷类型的唯一来源：后端 `backend/src/events.rs`（常量与 `settings_changed` / `refresh_requested` / `catalog_changed` 三个带类型的发送函数），前端 `frontend/src/lib/events.ts`（`EVENTS` 与 `EventPayloads`）。前端统一用 `listenEvent(EVENTS.X, handler)` 订阅，载荷类型按名字推断；启动页 `startup.ts` 与压缩对话 `CompressDialog.tsx` 直接 `listen`，名字同样取自 `EVENTS`。「载荷」列是后端发送的内容，没有载荷的前端收到 `null`。

| 事件 | 载荷 | 主要发送方 | 前端监听方 | 含义 |
|---|---|---|---|---|
| `settings-changed` | 整份设置 + `revision`（发送时现读快照） | `events::settings_changed`：各设置命令、`proxy.rs`、`net.rs`、`updater.rs`、`automation.rs`、`gateway/escape.rs` | `useSettings`（丢弃修订更小的）、`useGatewayActivity` | 设置变了，用新值替换本地副本 |
| `connections-changed` | `null` | `commands/providers.rs`、`commands/quotas.rs`、`watcher.rs`、浏览器授权 | 供应商、签到、灵动岛、Agent 页、模型路由页、账号卡模型路由区块 | 连接增删改，重拉列表（模型目录也跟着重读） |
| `refresh-requested` | `null`（全部）或连接 id（只刷该连接） | `events::refresh_requested`：设置、供应商命令、`watcher.rs`、`data_files.rs`、`lib.rs` | 灵动岛、`useQuota` | 请求刷新 |
| `collection-status` | `CollectionStatus` | `lib.rs::emit_collection_status` | 主窗口 | 本机日志采集是否连续失败 |
| `live-usage` | `LiveUsageDto` | 实时用量监听（`EVENT_LIVE`） | 灵动岛、`usageChanges.ts` | jsonl 有新写入，推增量 |
| `gateway-route-start` / `gateway-route` | 在途开始 / 结束记录 | `gateway/activity.rs`（`START_EVENT` / `END_EVENT`） | `useGatewayActivity`、`useGatewaySessions`、路由流水 | 网关请求开始 / 结束 |
| `gateway-wiring-changed` | `()` | `commands/agents.rs`、`commands/routing.rs` | Agent 页 | 接线写入 / 断开 / 接受 |
| `automation-changed` | — | `automation.rs`（`CHANGED`） | 自动化页、`useAutomationWaiting` | 任务或运行状态变化 |
| `daily-checkin-changed` | 连接 id | `daily_checkin.rs` | 一键签到 | 某账号签到结果变化 |
| `library-changed` | `()` | `commands/capabilities.rs` | Library / 搜索插件 | MCP 安装或移除 |
| `privacy-first-mask` | `()` | `commands/capabilities.rs` | 主窗口 | 第一次发生脱敏，弹一次说明 |
| `handoff-chat` | 流式片段 | `session_handoff.rs`（`CHAT_EVENT`） | `CompressDialog.tsx` | 压缩会话的对话式修订 |
| `dock-changed` | `{ island, dock }` | `commands/island_window.rs` | 灵动岛 | 停靠状态变化（带岛 id，各岛只处理自己的） |
| `dock-hint` | `{ island, hint }` | `commands/island_window.rs`、`lib.rs` | 灵动岛 | 拖动时的吸附提示 |
| `island-pointer` | 指针位置 | `island_pointer.rs`（`EVENT`） | `useGaze.ts` | 窗口外视线跟随 |
| `island-blur` | 岛 id | `lib.rs`（窗口失焦） | 灵动岛 | 收起展开卡片 |
| `island-refresh` | `()` | `lib.rs`（托盘 / 菜单） | 灵动岛 | 立即刷新 |
| `main-intent` | `()` | `lib.rs` | 主窗口 | 有待处理的跳转意图，调 `take_main_intent` 取 |
| `app-notice` | `()` | `notify.rs`（只发给主窗口） | `AppNotices.tsx` | 有应用内通知，调 `take_app_notices` 取 |
| `update-download-progress` | `{ downloaded, total }` | `updater.rs` | `UpdateContext.tsx` | 更新下载进度 |
| `menu-reopen` / `menu-state-changed` | `MenuSource` / `()` | `context_menu.rs`、`lib.rs` | `ContextMenuWindow.tsx` | 右键菜单重开 / 状态变化 |
| `tray-summary-open` / `tray-summary-close` | `()` | `tray_summary.rs` | 托盘摘要窗口 | 托盘摘要显隐 |
| `startup-error` | 错误文本 | `lib.rs::report_startup_error` | `startup.ts` | 启动失败 |
| `gateway-catalog-changed` | `null` | `events::catalog_changed`：路由分组增删改、刷新模型清单、改来源名、开关供网关使用（含批量） | Agent 页、模型路由页、账号卡模型路由区块 | 模型目录可能变了，重读 `gateway_catalog` |
| `warmup-finished` | 结果 + `finished_ms` | `automation.rs`（`warmup_run::remember`） | 设置页运维区 | warm-up 结束（成功 / 跳过 / 失败） |

## 5. 核对发现的问题与处理（2026-11-16，鼠鼠要求全部改代码）

1. **四个事件没有前端监听**：
   - `dnd-changed`、`island-platform-changed`：全部 4 处发送点都同时发 `settings-changed`，是遗留的重复事件，**已删除**（不再登记在事件表里）。
   - `gateway-catalog-changed`：**已补监听，并补齐发送方**。原来只有路由分组增删改与刷新模型清单发；改来源名（目录里 `来源/模型` 全变）只发了接线变化，开关「供网关使用」什么都不发，现在都发。账号卡上的模型路由区块原来只在挂载时读一次，Agent 页要等 15 秒轮询，模型路由页要等 5 秒；现在三处都在收到它或 `connections-changed` 时立即重读。
   - `warmup-finished`：**已接到设置页**。失败不记日期、会在下一次检查重试，原来界面上完全看不到。后端 `warmup_run::remember` 记住最近一次结果与完成时间，新增命令 `warmup_last_outcome`；设置页运维区挂载时读一次，之后跟事件更新，在「当前状态」里显示「今天 HH:MM 尝试失败：原因，下一次检查时会重试」或「今天 HH:MM 跳过：原因」（无画板，**待确认**）。
2. **`refresh-requested` 的载荷**：JSON 层面本来就是 `null` 或字符串（Rust 的 `()` 序列化为 `null`），只是 Rust 侧写法两种。**已统一**为 `events::refresh_requested(app, Option<&str>)`。
3. **事件序号 / 错误码**：**已按「集中管理名字 + 快照修订号」做**：
   - 名字集中在 `events.rs` / `lib/events.ts`，前端按名字推断载荷类型，写错名字或载荷编译失败；后端测试 `names_match_frontend_table` 核对两边名字集合一致。迁移中顺带发现右键菜单把没有载荷的 `menu-state-changed` 标成了 `MenuSourceEvent`，已拆成两个监听；
   - `settings::Store` 每次保存成功修订号加一（在写锁内），`snapshot()` 在同一把锁里取设置与修订号；`settings-changed` 一律发送时现读快照，`get_settings` 也返回快照；`useSettings` 丢弃修订号更小的推送与读取结果（`lib/settingsRevision.ts`）；
   - 错误码不加。
4. **防旧响应回填**：**已建工具并迁移**。`lib/latestGate.ts` 与 `hooks/useLatestGate.ts`（卸载时作废）。补上保护：账号卡 `useRoutingIndex.reload`、Agent 页 `loadCore` / `detect`（过期结果也不写跨挂载缓存）、模型路由页 `refresh`、对外访问与中间件面板的 `load`、搜索设置 `load`。手写计数改用它：`useQuota` 三个 hook 的 `requestId`、`useConnections` 的 `loadRevision`。不改的：effect 内靠清理置 `cancelled` / `alive` 的 16 处（本就正确）、`useLiveQuery`（已有 `revision` 计数）、单飞写法的 `useGatewaySessions.load` / 签到页 `refresh` / 路由流水 `load`、只调一次的导入对话框与数据分区。
5. **全页面冒烟脚本里会话线程、自动化、Skills 管理三页报「『正在读取』再次出现」：已查明是脚本判定缺陷，页面本身没有问题，已修。**
   - **根因**：第一次进入某页时，加载提示分两段：先是懒加载占位「正在加载<页名>」（下载页面代码），页面挂载后才是页面自己的骨架「正在读取…」。旧判定只认「正在读取」，于是在「正在加载」阶段就判定加载已结束（失败行里「读取 62–84ms」其实是加载还没开始）。之后只要这一页第一次加载超过约 1.5 秒，骨架还挂着，就被误报成「再次出现」。之前失败的那几次，都是刚启动的实例上第一次进入这三页时加载偏慢；这三页恰好排在第 5–7 个，访问时正处在启动后的忙碌期。
   - **证据**：
     - 页面内挂 DOM 观察器记录加载提示的出现与消失：「正在加载会话线程」在冷启动的 Vite 上持续 300–400ms。
     - 用请求拦截把页面代码延迟 1 秒（新增的慢环境开关 `JOLU_SMOKE_LATENCY_MS`）：「正在加载」实际持续约 1.1 秒，旧判定却报「读取 57ms」，差了一个数量级。
     - 正常条件下累计 15 轮以上都复现不出失败，说明失败只在加载偏慢时出现。
     - 排查中排除的方向：同步命令卡住主线程（每 100ms 调同步命令实测最大 18ms）、刚编译的新二进制首次启动、残留的 WebView2 进程占用调试端口。
   - **修法**（`scripts/acceptance/main-window-smoke.e2e.mjs`）：
     - 「正在加载」与「正在读取」都算加载提示；「加载结束」要求连续 400ms 看不到任何加载提示。
     - 结束后若再出现，或超过 10 秒未消失，失败说明里写出是哪一行、是否骨架屏的无障碍标签，以及本页每个 IPC 的耗时。
     - 加载超过 3 秒标「偏慢」并附 IPC 耗时，不判失败。
     - 新增慢环境开关 `JOLU_SMOKE_LATENCY_MS`。
   - **验证**：
     - 慢环境（页面代码延迟 1000 / 3500ms）全部通过，报出真实耗时约 1.0–1.1 / 3.5–3.6 秒，后者标「偏慢」。
     - 自检：每页加载结束后人为插入一段「正在读取」，8 页全部报出「加载提示在结束后再次出现」并附 IPC 耗时。
     - 正常条件下，全新实例连跑 3 轮全部通过，加载耗时报为约 270ms（旧判定报 40–80ms）。

验证：
- 后端全量 1425 passed / 0 failed / 无警告；前端类型检查通过、456 passed / 0 failed。新增的测试有：修订号递增与失败不加（含掩码）、前后端事件名一致、`settingsRevision` / `latestGate` 单测、`useSettings` 推送乱序与读取晚到；`opsSettings` 的 warm-up 文案覆盖今天失败 / 跳过、昨天不提、完成优先。
- 隔离桌面跑 `scripts/acceptance/events-contract.e2e.mjs` 通过：`get_settings` 修订号 3 → 保存一次 → 4；停在 Agent 页建一个路由分组后约 86ms 内重读 `gateway_catalog`；`warmup_last_outcome` 返回 null。全页面冒烟与改动前一致（5 页通过，上面第 5 条的 3 页仍是原有问题），`agents-discovery-cache.e2e.mjs` 通过。
