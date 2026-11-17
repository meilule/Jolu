# Jolu 开发规范

Windows 桌面 AI 工具管理器。前端 `frontend/`：React 19 + TypeScript + Vite + Tailwind CSS 4；后端 `backend/`：Tauri 2 + Rust + rusqlite（SQLite）。

## 1. UI 设计依据（最高优先级）

- **唯一设计依据是 `docs/ui/Jolu.pen`。**
- **主窗口全部界面和对应的前后端功能都严格按 `Jolu.pen` 重做，旧界面不是参考。** 唯一例外是浮动灵动岛窗口本身的展示（`components/island/`、`views/IslandWindow.tsx`）：经典样式保持现状，胶囊样式按 A3 实现，二者并存。旧组件只能复用其中的数据逻辑；页面按新稿重做完成后，删除被取代的旧组件。
- `.pen` 是加密文件，只能通过 Pencil MCP 读取（`get_app_state`，`execute` 里的 `Get` / `GetVariables` / `TakeScreenshot`）；禁止用 Read、Grep 或文本编辑器打开。未经鼠鼠明确要求，不修改 `Jolu.pen`。
- 按模块找画板。画板按序号成对出现，奇数是浅色、偶数是深色；先看画板本身，再看旁边的「画板序号与标题」和行标题里的说明：

  | 顶层画板 | 画板 | 内容 | 现有实现（截至 2026-10-22） |
  |---|---|---|---|
  | 00 · Jolu 模块目录 | — | 索引与阅读方式 | — |
  | 模块 A · 灵动岛 | 01–04 | 主窗口「灵动岛」页、侧栏收起形态 | 已按新稿实现：`views/MainPanelWindow.tsx`、`views/IslandView.tsx`、`components/panel/SideNav.tsx` / `PageHeader.tsx` |
  | 模块 B · 供应商 | 05–29 | 直接连接、账号管理、订阅连接、API Key 与模型获取、OpenCode、账号与连接的异常状态；29 是平台能力规则 | 已按新稿实现：`views/ProvidersView.tsx`、`components/providers/`、`lib/providerCatalog.ts`；自定义供应商、ZCode API Key、OpenCode 模型服务已接入；Trae 双版账号、ZCode BigModel 个人订阅、自定义余额已接入，真实与合成验证范围见未验证清单；Qoder / WorkBuddy 不代管 API Key |
  | 模块 I · 一键签到 | 133–160 | 供应商下方同级入口、六版本品牌卡片、批量与单账号签到、记录、供应商选择、连接表单、右侧通知、四列统一摘要、右侧详情、统一操作区与两页批量删除 | `views/CheckinView.tsx`、`components/checkin/`、`lib/dailyCheckin.ts` / `checkinApi.ts` / `checkinStats.ts`，后端 `daily_checkin.rs` / `checkin_*.rs`；WorkBuddy 国内、Qoder 双版、Trae 国内已接入官方每日奖励；WorkBuddy AI 为自动活跃奖励、Trae 国际未开放；顶部刷新显示右上角通知，添加账号先选供应商，选择项与卡片有品牌图标且无加号；共用 `AccountConnectDialog.tsx` 原地保存，支持浏览器多账号；订阅免填名称并显示官方账号，宽屏每行四卡；149–150 为最新统一摘要，151–152 右侧完整详情，底部详情/状态/删除并排；153–160 为签到及账号管理批量选择、共用确认和部分失败重试，`AccountBatchControls.tsx` / `useAccountSelection.ts` / `accountActions.ts` 按选中连接 ID 删除、筛选裁剪选择并保留历史；A 统计按来源显示官方与 Jolu 记录，WorkBuddy 接返回日期及本期累计，Trae 接汇总/逐包与标明计算的可用量，不限量及未知分别保留；Trae 国内真实网页授权至保存及只读签到状态已通过，第二个真实账号尚缺登录样本，见同日验收记录 |
  | 模块 C · 用量统计 | 30–39 | 概览、用量分析、调用明细、日期范围弹窗、交互状态 | 已按新稿实现：`views/UsageView.tsx`、`components/usage/`、`lib/usageStats.ts`，后端 `usage_stats.rs`；会话项目名、额度提醒与按账号同步失败状态待确认 |
  | 模块 D · 会话线程 | 40–57 | 会话列表与消息、搜索、在原工具继续、压缩会话与 Markdown 交接、从会话创建自动化 | 已按新稿实现：`views/SessionsView.tsx`、`components/sessions/`、`lib/sessionThreads.ts`，后端 `session_threads.rs`；压缩会话（48/50/52/54/57A）已接入配置、对话式修订、MD 预览与导出；创建自动化携带会话与项目；短会话真实探索评测完成，长会话正式评测缺少样本；ZCode 会话正文以只读方式解析其 sqlite（`message` / `part`，`native:zcode:main:` 前缀还原裸会话 id，单会话超出 2000 条 part 截断并标注），列表只列根会话（子代理会话 workflow_child / subagent_child / fork / selection_side_chat 不进列表），压缩会话与在原工具继续仍只支持 Claude Code / Codex |
  | 模块 E · 自动化 | 58–73 | 任务列表、新建任务、等待额度与续跑、运行记录与详情、执行会话选择 | 已按新稿实现：`views/AutomationView.tsx`、`components/automation/`、`lib/automation.ts` / `automationApi.ts`，后端 `automation.rs`（Claude Code / Codex 非交互执行与本机调度）；Skills 选择（D-08，`SkillPicker.tsx`，下拉面板待确认）只列目标工具已启用的全局 + 工作目录项目 Skills，执行前与续跑前复查，执行账号须为原工具当前登录的账号 |
  | 模块 F · Skills 管理 | 74–85 | Skills 列表、供应商选择、详情与文件、新建 / 编辑、导入与冲突检查 | 已按新稿实现：`views/SkillsView.tsx`、`components/skills/`、`lib/skillsManager.ts` / `skillsApi.ts`，后端 `skills.rs`；已接入 Claude Code、Codex、Gemini、OpenCode、Qoder、Trae、ZCode；WorkBuddy 仅只读扫描 `.codebuddy/skills`；自定义目录只读；Grok 未接入；HTTPS Git 仓库导入、草稿与磁盘差异已有实现；项目范围由会话记录自动带出 + 手动添加，Claude Code 停用 = 移到应用数据目录的停用区（不写 `skillOverrides`） |
  | 模块 G · 设置 | 86–103 | 常规、外观、网络与代理、数据与存储、通知、关于与更新、清理确认、状态与反馈、手动联网 | 已按新稿实现：`views/SettingsPage.tsx`、`components/settings/`（settingsUi、各分区与 DataDialogs）、`lib/settingsPage.ts`、`lib/settingsApi.ts`，后端 `storage_stats.rs`；默认页面、缩放、密度、减少动效、联网方式、诊断、任务提醒与导入预览已有实现；检查更新默认启用（启动及每 30 分钟检查、后台下载、页头绿色版本按钮、被动安装后自动重启，画板 100/101 的 06），首次安装与 0.1.0→0.1.1 更新已用改名测试包真机验证，见 `docs/verification/2026-10-27-update-install-e2e.md`；全局快捷键与套餐查询凭证已按鼠鼠要求删除，设计稿已同步；2026-11-09 新增隐私脱敏分区（`PrivacySection.tsx`，挂「网络与代理」页尾；2026-11-11 加每类处理方式、从不脱敏清单与脱敏记录弹窗 `PrivacyRecordsDialog.tsx`）与运维四项（`OpsSection.tsx`：保持唤醒、每日 warm-up、OTLP 导出、Agent 版本，挂「数据与存储」页尾；后端 `keep_awake.rs` / `warmup*.rs` / `otlp.rs` / `agent_version.rs` / `wsl_probe.rs`），均无画板、标「待确认」；2026-11-17 在「数据与存储」页尾加 WSL 里的工具接线（`WslWiringSection.tsx`，后端 `gateway/wiring_wsl.rs`：Claude Code / Codex / OpenCode，mirrored 走 127.0.0.1，NAT 只在用户已开局域网共享时经主机地址 + 专用客户端 Key，无画板，待确认）；`jolu` CLI（`bin/jolu_cli.rs`）2026-11-17 起有写命令 `provider add|rm`（只收 API Key，Key 从标准输入读，走桌面端同一套校验）、`save` / `use` / `serve`（经 `cli_requests.rs` 文件队列由运行中的桌面端代办）、`group add|set|rm`（路由分组，写库即生效）、`settings set`（白名单键，与桌面端共用 `filelock.rs` 文件锁，桌面端每 2 秒读回外部改动）与 `claude run`（`--settings` 临时走模型路由）；2026-11-17 在「数据与存储」页尾加 Library 共享指令与子代理（`LibrarySharedSection.tsx`，后端 `library/shared.rs`，区块 / 文件按哈希判归属，无画板，待确认） |
  | 模块 H · 全局规范与品牌 | 104–110 | 按钮与图标交互六态、Logo 与人物、桌面 / 任务栏 / 托盘图标、启动页 | `components/brand/`、`components/ui/`、`components/tray/`；启动页（109 / 110）已按新稿实现：`frontend/startup.html`、`src/startup.ts` / `startup.css`，窗口在 `lib.rs` 的 `show_startup` |
  | 模块 A2 · 浮动灵动岛 | 111–132 | 浮动灵动岛：Auth / API 收缩与展开、仅 Token、Token 动效、配置细节、平台选择、拖动贴边、右键菜单与分身 | `components/island/`、`views/IslandWindow.tsx`、`views/ContextMenuWindow.tsx` |
  | 模块 A3 · 胶囊灵动岛（可选样式） | A3-1…A3-12（按名称搜索 `A3-`） | 胶囊收缩 / 展开卡片、bot 表情库、模型入口适配、Token 轮播与视线跟随；设置 · 灵动岛页新增样式切换 | `components/island/CapsuleIsland.tsx` / `Bot.tsx` / `capsule.css`、`hooks/useGaze.ts` / `useBotShots.ts`、`lib/botExpression.ts` / `botGaze.ts` / `tokenTicker.ts`；后端 `island_style` 与 `set_island_style`；Token 轮播、视线跟随（含窗口外，后端 `island_pointer.rs`）、额度环交互、贴边迷你胶囊（同一个 `CapsuleIsland` 外壳缩到 32px）、模型入口适配（后端 `gateway/activity.rs` 的请求开始 / 结束事件与 `gateway_today_counts`，前端 `useGatewayActivity` / `CapsuleGateway.tsx`）已接入；自动化「等额度」表情已接 `automation_waiting_connections`（4.2）；4.2 另加额度环跟随被调用来源、会话行来源与点击跳转、今日按来源、「正在写」三档眼色、月窗口蓝色外圈（均无画板，标「待确认」）；真机已验证双击、Esc、点卡片外、贴边探出、窗口外视线与连接下拉；贴边动效 2026-11-05 返工为「预留盒子先长、外壳按停靠边绝对锚定、气泡三栏居中」，且只有左右贴边缩成迷你胶囊（上下贴边保持完整胶囊）、贴边不投影、展开卡片可拖动；2026-11-05 追加：胶囊态窗口贴合胶囊（不再留 20px 内边距与点击死区，只有展开卡片留投影空间）、展开态可直接拖动（拖动期间的 dock-changed 不改变形态，松手按最新停靠边收尾，拖离边缘回到自由胶囊）、形变期间关闭 `backdrop-filter` 消除贴边弹出卡帧；A3-11 / A3-12 为展开态拖动的浅色 / 深色示意，见 `docs/plans/2026-11-04-capsule-island-implementation.md` |
  | 模块 J · Agent + 模型路由（原统一网关 / 模型入口） | **现行：Agent 列表态为「统一网关 · R2 重设计」的 V8（按名称搜索 `V8-`，鼠鼠在 Pencil 手绘：三列工具卡片 + 卡上开关 + 详细入口）；其余页面沿用 V6（按名称搜索 `V6-`；预览导出在 `docs/ui/previews/agents-routing-v6/`；V7 为鼠鼠要求的过程稿；V5 / V4 / V1–V3 为过程稿）**；此前 161–166、`s1Bm7` / `Lz482` / `eMBlV` / `g1Uhz`、`J6-` | V8（Agent 列表态，2026-11-15 实现）：三列工具卡片栅格（卡 328×218：官方 logo + 名称/状态 + 接入开关；模型摘要三行；底栏状态点 + 「详细」进详情页）+ 接入状态筛选（全部/已接入/可接入，带计数）+ 搜索 + 暂不能接入折叠条（默认收起，展开三列原因卡）+ 底部「开启前预览配置改动，关闭时可还原 Jolu 管理的字段。」安全提示；副标题「接入模型路由，为每个工具配置顺手的模型。」；已检测但无专用接入的工具（非 WIRABLE 的 8 家）在栅格里显示「暂不支持」卡（开关禁用、详细看等待接入适配说明，`AgentRow.supported` 区分，不走折叠区）；V8 未画详情态，「详细」沿用 V6 详情页。V6：Agent 详情态（「← 全部工具」+ 配置一致 badge + 配置方案下拉 + 模型设置 / 高级 / 写入改动三页签）、模型路由状态条（本机路由 · 地址 · 正在中转 · 停止中转 · 更多）+ 六页签：概览（六指标 / 每小时趋势 / 来源分布 / 请求活动 / 需要关注）、路由流水（沿用 V5 面板）、路由分组（分组下拉 + 策略 + 成员表格 + 保存更改）、模型池（来源手风琴 + 模型行）、对外访问（局域网共享大卡 + 连接地址与示例 / 客户端 Key 两页签）、中间件（流程链 + 规则卡列表）；Cktyu 为概览深色稿（与浅色同构，颜色全走 `jolu-*` 变量） | 2026-11-15 按 V6 重做两页后，同日按 V8 重做列表态（`AgentList.tsx` 的 `AgentDirectory`：三列 `ToolCard` + 筛选 chips，筛选纯逻辑 `agentGridRows` 在 `lib/agentPage.ts` 有测试；开关 on=断开走确认弹窗、off=一键接入，均复用 `AgentsView` 的 wire/unwire）：「Agent」（`views/AgentsView.tsx` 列表 / 详情整页切换、`components/agents/AgentList.tsx` 为全宽 `AgentDirectory` + 官方 logo `AgentBadge` + 开关 `Switch`、`AgentDetail.tsx` 三页签：模型设置（新会话默认模型 + 档位表格 + `/model` 菜单卡 + 统计条）/ 高级 / 写入改动、配置方案移入右上下拉）与「模型路由」（`views/GatewayView.tsx` 状态条 + 页签容器；`OverviewPanel.tsx` 概览（后端 `gateway_day_overview` + `gateway_today_counts` + `routeLogPage` 240 行推导趋势 / 来源分布 / 响应头分位，纯逻辑 `lib/gatewayOverview.ts` 有测试）、`GroupsPanel.tsx` 分组、`PoolPanel.tsx` 模型池、`AccessPanel.tsx` 对外访问（Key 表格 + 签发 + 限额就地展开，沿用 `sharedAccess.ts`）、`MiddlewarePanel.tsx` 中间件（流程链 + 规则卡）；`RoutingObjects` / `RoutingOverview` / `ExternalAccess` / `ModelPoolPanel` / `RoutingGroupsSection` / `MiddlewareDialog` / `SharedAccessDialog` 已删除，`SourceManageDialog` 不再内嵌分组区块）。供应商账号卡「模型路由」区块（`providers/RoutingBlock.tsx`）不变。真机截图对照 V6 画板通过（2026-11-15 dev 实例）。Agent 探测（`agent_discovery.rs`，执行文档 `docs/plans/2026-11-16-agent-discovery-fix.md`，验收 `docs/verification/2026-11-16-agent-discovery-parity.md`）：2026-11-16 完成 T1（`vendor_bin_dirs` 在 PATH / 通用兜底 / 注册表之后补各家固定落点：Claude `~/.claude/local`、OpenCode `~/.opencode/bin`、Codex `install.ps1` 与 standalone 目录及桌面版 `%LOCALAPPDATA%\OpenAI\Codex\bin\<哈希>`）与 T2（`creds::zcode_data_dir`：`ZCODE_HOME` → Windows `HOME` → `setting.json` 的 `dataBaseDir`；探测、接线、额度、账号导入、归因、本机来源共用，团队套餐的 `setting.json` 读 `creds::zcode_home`，会话库 `cli/db` 不跟着搬）、T3（`agent_dir_has_content`：只剩 `logs` / `cache` 等残留、`skills`、`Spec.sign_in` 登录文件不算已装）、T4（`Spec.detect` 枚举 `Standard` / `CliOnly`，Gemini 只认命令行）、T5 / T6（`find_bin` 按目录戳缓存，含「找不到」，每 2 秒最多重取一次戳、注册表只在取戳时读一次；`agent_discover` 带 `force` 时 `forget_bin_lookups` 清缓存；本机一次探测约 100ms → 过窗口约 4ms）、T7（`AgentsView` 跨挂载记住上次探测与版本结果，先显示再后台刷新、不节流，探测不再等 `--version`）、T8（`Spec.bin_not`：Goose 用 `go_program` 排除 Go 写的同名 goose；Goose 的 Windows 目录改正为 `%APPDATA%\Block\goose\config`）、T9（`path_taken`：配置目录或上级被文件占着判未装）；T7 真实桌面验收 `scripts/acceptance/agents-discovery-cache.e2e.mjs` 通过（切回第一帧即为探测后内容、「重新检测」带 force）；Continue 按官方文档核实为 `cn` + `~/.continue/config.yaml`（magpie 里没有这家，未做撞名甄别）。V10 Agent 上下文观察（执行文档 `docs/plans/2026-11-16-agent-context-observation-backend.md`，画板按名称搜 `V10-`）：2026-11-17 完成第 1 期解析层 `gateway/context/`（Anthropic / Chat / Responses / Gemini 原生四协议请求体 → 八类组成 system / tools / skills / memory / files / results / chat / unknown 与逐项 token 估算、按上游实测校准并保留估算值；移植 magpie `prompt.go` 并补 Gemini CLI / OpenCode / Codex 新版格式规则；14 个脱敏真实样本在 `context/testdata/`，抓取与脱敏脚本 `scripts/acceptance/context-capture*.{mjs,py}` / `context-sample-sanitize.py`，验证见 `docs/verification/2026-11-17-agent-context-observation.md`）；同日完成第 2 期采集（`context/capture.rs` 挂在 `wire::handle` / `adapter::gemini::handle` 选路前，有会话标识的请求按调用方协议在后台解析一次，响应结束读上游用量（`budget::UsageMeter`，含缓存命中）后校准；表 `gateway_context_snapshot`（`context/store.rs`，状态 analyzing / ready / unsupported / no_body / failed，未知存 NULL，每会话留 20 条、随路由流水一起清理）；逐条工具结果明细 `calls_json`（`context/summary.rs`：命令或路径经 `handoff_facts::redact` 与本机目录 / 用户名替换，只存行数、退出码与一句话摘要，不存正文）；模型窗口查来源覆盖 → 同目录 models.dev 缓存 → 内置快照）；同日完成第 3 期：`context/query.rs` + `commands/routing.rs` 的 5 个只读命令（`gateway_context_sessions` / `_snapshot` / `_items` / `_tool_items` / `_history`，走只读连接池，链路按 request_id 取流水最后一条，未知为 null），「模型路由 › 概览」按 V10-01 / V10-02 重排（`OverviewPanel.tsx`：运行概览六指标、最近 12 小时趋势、需要关注含上下文接近窗口提醒、活跃会话表，V6 的来源分布 / 请求活动随之去掉；纯逻辑 `lib/contextSessions.ts` 有测试；预览验收 `scripts/acceptance/context-overview-preview.e2e.mjs`），阈值 80%、状态筛选项、「调用工具」细分、表格只显示 8 个标「待确认」，同日完成第 4 期会话观察（`components/gateway/SessionObservation.tsx` + `lib/contextObservation.ts`，V10-03～14：链路、上下文占用 / 提示词缓存卡、八类分类堆叠条与矩形图点选筛选、明细表按类别聚合展开（轮次、工具定义、单次工具返回的脱敏摘要「查看全部」）、Skills 加载证据（后端识别 `SKILL.md` 读取与 Skill 工具，归入 Skills 一次）、历史请求下拉与跟随暂停 / 新请求计数 / 恢复跟随、各采集状态与偏差提示；预览验收 `scripts/acceptance/context-observation-preview.e2e.mjs`），请求编号用快照 id、响应头时间与单次返回时间不显示（数据没有）等标「待确认」；Gemini CLI 无会话标识暂不建快照、V10-05 每条调用的时间、「来源不支持缓存」与「未提供」的区分标「待确认」；发布版 `panic = "abort"`，解析层须自身不 panic。Kimi 专用接线（2026-11-17，参照 magpie `internal/agent/kimi.go`）：`wiring.rs` 加 kimi——config.toml 写 `providers.jolu`（type/base_url/api_key 就地表内更新，保住用户自加的 custom_headers）+ 重建全部 `models."jolu/<网关模型 id>"` 表（model 字段发上游可路由 id、max_context_size 缺省 128000、display_name / max_output_size 保留旧值、capabilities 按新版 Kimi Code 写 tool_use 与 image_in；旧版 kimi-cli（`~/.kimi`，`KIMI_CODE_HOME` / `KIMI_SHARE_DIR` 覆盖在 `kimi_dir` 与探测 `dir_kimi` 同步对齐 magpie kimiDir）少写它不认识的键）+ `default_model`（接线前用户原值进快照，断开还原且要求那张表还在；用户改走的非 jolu 默认不动）；模型清单与 ZCode 共用网关目录（`escape.rs` 的 `plan_agent` 对 kimi 也传 `zcode_models`，目录暂无按工具过滤，全量给）；与 magpie 差异：不用 stash 区分「上次写的值」，表里已有 display_name / max_output_size 一律保留（目录改名后旧显示名留到断开重接）；真机只读验收 `scripts/acceptance/agent-icons-kimi-preview.e2e.mjs`（写入预览走生产路径，不真点接入）+ 单测 5 条；同日修复真机接入报「保存接线记录失败：FOREIGN KEY constraint failed」——`db.rs` 的 agents 种子表漏了 kimi（agent_wiring 外键指向它），补种子行并加回归测试锁「每个可接线 agent 必须有种子行」；修复后真机接入/断开往返通过（写入 28 张模型表 + default_model，断开 0 残留、default_model 还原 moonshot 原值）；magpie 接线机制拆解见 `docs/specs/2026-11-17-magpie-agent-wiring.md`。Pencil / Qoder / Qoder CN / Crush 专用接线（2026-11-17，5.11，参照 magpie pencil.go / qoder.go / agents.go 的 crush）：`gateway/wiring_extra.rs` 整段写 `providers.jolu`（含网关模型目录生成的 models 数组），Qoder 另写 `model.name = jolu/<模型>`、Crush 另写 `models.large` / `models.small`；断开删 provider 段，选择键仍指向 Jolu 才还原快照；其余 10 家按 magpie 核对都要 yaml / 数组插项 / 多文件 / 凭据库，暂不接，决议见 `docs/specs/2026-11-17-agent-wiring-registry.md` §8；未做真实工具验收。Agent 图标补齐（2026-11-17，需求「检测到的工具都要有官方 logo」）：`AgentBadge` 三条来源——单色矢量进 `PlatformLogo` 的 SHAPES（cline / copilot 取 simple-icons，goose 取 lobe-icons MIT，zed 取 zed.dev/brand 官方 90×90，均带 evenodd；kimi / qwen / dsh(DeepSeek) 复用厂商矢量；不设 COLOR_CLASS 跟随主题前景）；多色 / 位图走 img（`assets/agents/`：vscode 官方品牌包、droid 为 factory.com 站点图标（黑底白纹）、continue 为其 GitHub 组织头像、pencil 为 pen.dev 站点图标；qoder / zcode / workbuddy / opencode 沿用旧资产）；Crush 没有干净官方图形（Charm 家只有照片底 touch 素材），暂保留首字母方块标「待确认」。`AGENT_PLATFORM` 加 kimi（会话 / Skills / 自动化的平台图标）。待确认：高级 / 写入改动 / 路由流水页签与展开表单（新建分组、成员勾选、规则编辑、Key 限额）无 V6 画板沿用旧交互、已接入行卡的档位数硬编码 Claude Code 4 档、暂停刷新按钮置灰、概览「今天」范围选择无后端、按 Agent 裁剪模型菜单（F3）、新会话强度（F4）、改端口。以下为此前 V5 记录：2026-11-13 按 R2 / V5 拆成两页（详情链接计数、档位与 `/model` 提示、写入预览与改动差异（后端 `wiring_diff.rs` / `gateway_wiring_preview`）、回读 `/model` 后「以配置为准」；预览验收 `scripts/acceptance/agents-routing-r2.e2e.mjs`）。再此前：2026-11-06 按 J6 重做主图（开发说明 `docs/specs/2026-11-06-unified-gateway-development.md`；`SessionGraph.tsx` / `GatewayAccessBar.tsx` 随 V6 删除；会话关联只认 Claude Code / Codex 原生会话 ID（`wire::session_identity`），在途快照带序号（`activity::live_snapshot`），聚合在 `gateway/sessions.rs`；Gemini CLI 请求不带会话标识，只记流水与运行事件；真实桌面验收 `gateway-unified-desktop.e2e.mjs` 15 项通过，见 `docs/verification/2026-11-06-unified-gateway-desktop.md`）。V3：三栏来源→模型池→Agent、E1–E10 状态弹窗、`lib/gatewayPage.ts` / `gatewayApi.ts`，后端 `gateway/`（route、wiring、escape、server、adapter、groups、identity）；持久化 slug 与 `slug_history`、account_fp 去重、统一接线锁与 `agent_wiring`、Host/Origin 校验、Codex 保留 OAuth 的 `openai_base_url` / 未登录时的静态 token、Gemini `/v1beta` 原生端点与事务式登录切换、CLI 原生会话头、来源 / Agent 分组命令均已接入；Claude / Codex 订阅已改为按所选连接读取加密授权，Claude 隔离 CLI 桥接（文本走一次性进程；流式 / 图片 / 调用方工具走 `adapter/claude_bridge.rs`，工具经网关自己的 MCP 端点挂起续跑）、Codex Chat / Responses 转发及账号用量归账已接入；全部官方订阅开关可选择，同平台不同账号可同时启用，按平台/地区/账号身份去重；OpenCode 的 OpenAI 订阅、Grok、Gemini 原生请求（含流式）与 Chat / Responses / Anthropic 调用方（`adapter/gemini_chat.rs`）、WorkBuddy 双版和 ZCode 个人套餐已有隔离适配，Qoder 已按 magpie 参考接入账号目录与直接转发，Trae 已接账号目录 function / __dev 路由、`llm_utils_chat`、Chat / Messages 流式与工具转换、Max 派生模型与 GLM / DSML 文本工具模板（均未真实账号验收，分别见 `docs/verification/2026-11-04-qoder-subscription-adapter.md`、`docs/verification/2026-11-04-trae-subscription-adapter.md`）；路由流水已接入记录、查询与展示（不等于尚未实现的多次故障转移尝试记录），见 `docs/specs/2026-11-03-gateway-dedup-and-wiring.md`。2026-11-08/09 magpie 对齐（计划与剩余差距见 `docs/plans/2026-11-09-magpie-parity-batch3.md`）：模型 id 解析与 Claude Code 完整档位 / Subagent 模型 / 六个云端开关；路由分组 `group/<slug>`（`routing_groups.rs`，五策略 + 「按意图」（`intent.rs`，小模型判每个新提问分强 / 轻档成员，默认不启用）、额度用满降权、成员暂停）、Profiles、预设库、本机导入（`import_sources.rs`）、隐私脱敏（`mask.rs`）、models.dev 元数据；中间件链（`middleware.rs`，五个内置 + `js-hook` JS 钩子（`js_hooks.rs`，常驻 Node / Bun 宿主）；流式回复走 `EventChain` 逐事件处理；保存前试跑，`service/gateway/middleware.rs`）；局域网共享与命名客户端 Key（`access.rs`，过期 / 模型白名单 / 只读 / 每日请求数；每日 Token 由 `budget.rs` 预扣 + 回冲强制；拒绝按 401 / 403 / 429 区分）；ZCode Start Plan 通道（`zcode_start.rs`：着装、忙碌重试、余额礼包过滤、边缘防火墙保持；`zcode.rs` 双通道按连接记忆 + `start_channel` 手动锁通道）；`service/gateway/` 服务层与只读 CLI `bin/jolu_cli.rs`；外部配置读取走 `steady.rs`（跨过 Windows rename 窗口），`settings.json` 有 `.lastgood` 回读。剩余工作与交接说明见 `docs/plans/2026-11-09-complete-remaining-work.md`。上述无画板界面（导入弹窗、Profiles 等）标「待确认」 |

  - 浮动灵动岛窗口有两种样式，由设置 `island_style` 选择（默认 `classic`，在「灵动岛」页切换）：经典样式以 A2 为准，胶囊样式以 A3 为准。两种样式读同一份数据，经典样式的代码与行为保持现状。
  - 「现有实现」写的是代码当时的状态，不代表已经和画板一致。新建页面或模块时更新这一列。
  - 编号以画板名称为准。查找画板时用 Pencil 按名称前缀搜索（如 `"40 · "`），不要写死节点 id。
- 颜色使用 `Jolu.pen` 的变量：主窗口 `jolu-*`，灵动岛 `pn-island-*`，浅深色由 `mode` 主题区分，不要凭感觉推导深色。
- 冲突时以 `Jolu.pen` 为准，高于任何文档、旧代码注释和截图。
- `Jolu.pen` 里没有的界面或状态，标注「待确认」并询问鼠鼠；不得自行补全。
- `docs/ui/previews/` 是导出截图（已被 Git 忽略），不是设计依据。
- `Jolu.pen` 用相对路径引用 `docs/ui/brands/`、`docs/logo/`、`frontend/src/assets/platforms/zcode.png` 里的图片。移动或重命名这些文件，必须同时用 Pencil 更新设计稿里的路径。

## 2. 目录与命名

```
backend/                    Tauri + Rust；src/ 按功能平铺模块，capabilities/、icons/、windows/ 为打包资源
frontend/src/
  components/<区域>/        automation、brand、island、panel、providers、sessions、settings、skills、tray、ui、usage
  hooks/                    React hooks
  lib/                      纯逻辑、API 桥接、平台目录；测试与源文件同目录
  views/                    各窗口入口
  mock/                     仅供浏览器预览的示例数据
docs/
  plans/                    实施计划
  specs/                    设计说明
  todo/                     待办
  verification/             验收与执行记录（按需新建）
  ui/                       Jolu.pen 设计稿及其引用的素材
  logo/                     品牌人物与应用图标母版（scripts/icons 的输入）
  assets/                   README 截图与社交预览图
scripts/
  acceptance/  dev/  icons/  installer-art/  release/  social-preview/
```

- 目录名和文档文件名：小写 kebab-case，不使用中文或空格。`README`、`AGENTS`、`CLAUDE` 等约定俗成的文件除外。
- 按日期记录的文档：`YYYY-MM-DD-主题.md`，放进上面对应的目录；不要再新建 `docs/superpowers/`，Superpowers 的计划写到 `docs/plans/`，设计说明写到 `docs/specs/`。
- **自带滚动的整页（Agent / 模型路由 / 供应商）必须用 `components/panel/PanelPage.tsx`**：主面板内容区是 `overflow-hidden` 的 flex 项，页面里写 `h-full` 会按内容高解析，`absolute inset-0` 的抽屉会随之缩成内容高（表现为页面中部截断、下方大片空白）。PanelPage 用 `absolute inset-0` 钉住可视区；新增同类页面时在 `PANEL_PAGE_PAGES` 登记，MainPanelWindow 靠 `usesOwnScroll` 统一处理，不要再写散落的 `page === xx` 特例。
- 前端：组件 `PascalCase.tsx`（一个文件导出多个基础组件时用 camelCase，如 `primitives.tsx`）；hook `useXxx.ts` 放在 `hooks/`；纯逻辑 `camelCase.ts`；测试 `<源文件名>.test.mjs`，与源文件同名同目录。
- 后端 Rust：`snake_case.rs`；脚本：`kebab-case`。
- 发布脚本在 `scripts/release/`，发行工作流是 `.github/workflows/release.yml`：**每次推送 main 都会发一个新的公开版本**，不需要发版的提交在信息中加 `[skip ci]`。规则见 `docs/release-versioning.md`。
- 检查更新默认启用；构建时同时设置 `JOLU_UPDATES_ENABLED=0` / `VITE_JOLU_UPDATES_ENABLED=0` 可停用。Release 必须附正式私钥签名的安装包与 `latest.json`，否则检查会失败。测试公钥和本地端点仅放 output 的覆盖配置，不修改生产配置。
- 安装器使用自定义模板 `backend/windows/installer.nsi`（基于 tauri-cli 2.11.4，改动处标 `Jolu:`）：固定安装到 `%LOCALAPPDATA%\Jolu`，不显示选择目录页。升级 `@tauri-apps/cli` 时须与上游模板对照。卸载钩子会删除名为 `Jolu` 的开机自启项，本机测试安装包时先备份、测后恢复。

## 3. 常用命令（在仓库根目录执行）

```bash
pnpm install && pnpm --dir frontend install
pnpm dev                                    # 只启动前端预览，http://localhost:5173（被占用时顺延）
pnpm tauri dev                              # 启动桌面应用（自动拉起前端；5173 被占用时由 scripts/dev/tauri.mjs 顺延端口并同步 devUrl）
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs # 前端纯逻辑与共享状态测试
node --test scripts/release/*.test.mjs      # 发布脚本测试
cargo test --manifest-path backend/Cargo.toml
```

桌面验收流程见 `scripts/acceptance/README.md`，只能在隔离目录中运行。

## 4. 开发约束

- 所有代码、配置和文档使用 UTF-8。
- API Key 和认证信息只在后端处理，前端列表只返回掩码；不得写入普通日志、统计导出和诊断报告。
- 未知值、真实的零、能力不支持、查询失败、数据过期要分别表达；正式桌面运行出错时，不得悄悄回退到 `mock/data.ts` 的示例数据。
- 不能根据 UI、图标或套餐名称推断供应商支持登录、余额、Skills 或自动化。
- 不得删除旧账号、历史统计和原始会话文件；数据结构变更需要提供可恢复的迁移。
- 产品名：给用户看的地方写 `Jolu`；包名、标识符、文件前缀和 URL 写 `jolu`；环境变量用 `JOLU_` 前缀。本项目是新仓库，不保留任何旧项目的兼容或迁移逻辑。官网地址暂时置空。
- 涉及第三方库、SDK、CLI 或认证协议时，先用 Context7（`npx ctx7@latest`）核对文档。

## 5. 验证与 Git

- 验证强度跟改动风险匹配：局部修改只跑相关测试；涉及公共接口、核心逻辑或多个模块时，再跑前后端测试和类型检查。
- 改了界面，要用 Pencil MCP 截取对应 `Jolu.pen` 画板，和实际运行效果对照。
- 未经鼠鼠明确授权，不执行 `git add`、提交、推送、切换分支或重置。

### 提交与推送节奏

- 以完整的功能、问题修复或验收阶段为提交单位，不以一次对话、一处文案或几行代码为单位；相关的小改动先积累，阶段完成并通过必要验证后统一提交、推送。
- 阶段边界由助手根据改动关联性、完整性和验证结果判断；鼠鼠连续提出的同一轮界面微调通常属于同一阶段，不要每完成一项就提交。
- 不为凑提交混入无关改动，也不把未完成或未验证的工作打包提交。已有其他修改只纳入明确属于本阶段的部分。
- 鼠鼠已授权按阶段提交、推送时，在授权范围内自行把握时机，不反复请求确认；没有授权时仍遵守上面的 Git 授权规则。鼠鼠明确要求立即提交时按该要求执行。
- **默认不构建安装包，只有鼠鼠明确要求时才构建**：未明确要求构建时，提交信息必须加 `[skip ci]`，避免推送 main 触发发行；明确要求构建时才正常提交，由发行工作流构建并发布新版本。单次构建授权仅对当次有效，不延续到后续提交，也不必每次询问。
- 提交时使用 `git-commit-push-zh` 技能，核对本阶段范围、身份与远端状态，以中文说明改动目的和验证结果。
- 小修改完成后简短汇报即可；尚未达到阶段边界时保留工作区修改，不为汇报而专门提交。本规范的零碎更新同样遵守此规则。
