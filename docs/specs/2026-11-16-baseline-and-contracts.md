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
| 冻结对象与接口字段；事件带实体标识、序号与错误码 | 对象定义在 `frontend/src/types.ts`、各 `*Api.ts` 的 DTO 与后端对应结构体；事件目录见本文 §4。**未统一**：事件没有统一的序号与错误码字段，只有网关在途事件带序号（`gateway/activity.rs` 的 `live_snapshot`） |
| 区分「能力支持」与「本机已配置」；未知平台不回落其他平台 | 后端 `platforms::supports` / `known`；额度路线集中在 `quota_registry.rs`（目录声明了订阅额度的平台都要有路线，有一致性测试）；前端 `PlatformId` 放宽为「已知 id 或任意字符串」，`fictionalProvider.test.mjs` 让未登记平台走一遍全部纯函数，不补假数据 |
| 可重复执行的迁移与备份恢复 | 数据库在 `db.rs` 用 `CREATE TABLE IF NOT EXISTS` 与可重复执行的 `ALTER TABLE … ADD COLUMN`（失败即已存在，忽略）；数据导出 / 导入带 `schema_version`（1 / 2），不认识的版本明确拒绝。AGENTS.md §4 要求「数据结构变更提供可恢复的迁移」 |
| 新 UI 查询加请求身份，防切换筛选后旧响应回填 | **分散实现，未统一**：`hooks/useQuota.ts` 用 `owner.current !== queryKey` 丢弃旧结果，另有约 15 处用 `cancelled` / `alive` 标志；计划中的 `lib/queryIdentity.ts` 没有建。`lib/queryCache.ts` 只做 30 秒展示缓存 |
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

## 4. 事件目录（后端 → 前端，共 30 个）

前端统一用 `lib/api.ts` 的 `listenEvent(name, handler)` 订阅（启动页 `startup.ts`、右键菜单 `ContextMenuWindow.tsx` 等少数地方直接 `listen`）。「载荷」列是后端 `emit` 的第二个参数。

| 事件 | 载荷 | 主要发送方 | 前端监听方 | 含义 |
|---|---|---|---|---|
| `settings-changed` | `Settings`（整份） | 各设置命令、`proxy.rs`、`net.rs`、`updater.rs`、`automation.rs` | 主窗口、灵动岛 | 设置变了，用新值替换本地副本 |
| `connections-changed` | `()` | `commands/providers.rs`、`commands/quotas.rs`、`watcher.rs`、浏览器授权 | 供应商、灵动岛 | 连接增删改，重拉列表 |
| `refresh-requested` | `()` 或连接 id（**两种都有**） | 设置、供应商命令、`watcher.rs`、`data_files.rs`、`lib.rs` | 灵动岛、用量 | 请求刷新；带 id 时只刷该连接 |
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
| `dnd-changed` | `bool` | `commands/preferences.rs`、`lib.rs` | **无** | 见 §5 |
| `island-platform-changed` | 平台 id | `commands/island_window.rs` | **无** | 见 §5 |
| `gateway-catalog-changed` | `()` | `commands/routing.rs`、`commands/providers.rs` | **无** | 见 §5 |
| `warmup-finished` | `WarmupOutcome` | `automation.rs` | **无** | 见 §5 |

## 5. 核对中发现的待查问题（本次不改代码）

1. **四个事件没有前端监听**：
   - `dnd-changed`、`island-platform-changed`：已核对全部 4 处发送点（`commands/preferences.rs`、`lib.rs`、`commands/island_window.rs` 两处），每处都同时发 `settings-changed`，前端靠后者已能拿到新值，是遗留的重复事件，可删；
   - `gateway-catalog-changed`：来源改名、刷新模型清单后发出。前端模型池 / 模型菜单是否因此没有即时刷新，需在 Agent 页与模型路由页实测；
   - `warmup-finished`：warm-up 结束后发出，界面上是否需要反馈待确认。
2. **`refresh-requested` 的载荷不统一**：有的发 `()`，有的发连接 id。监听方需要同时处理两种形状。
3. **事件没有统一的序号 / 错误码**（P1 原计划要求），只有网关在途快照带序号。
4. **防旧响应回填没有统一模块**（P1 原计划的 `queryIdentity`），各页面各自用 `owner` 判断或 `cancelled` 标志。
5. 全页面冒烟脚本里，会话线程、自动化、Skills 管理三页报「『正在读取』再次出现」（重构前同样失败），见 `docs/plans/2026-11-09-complete-remaining-work.md` 4.10。
