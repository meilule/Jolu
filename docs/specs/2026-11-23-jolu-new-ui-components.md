# Jolu 新版界面：外壳与基础组件

日期：2026-11-23

状态：设计中（R1）。鼠鼠拍板整体方案后，再统一替换现有前端。

- 设计依据：`docs/ui/jolu-new.pen`，画板 `R1-C · 组件库`、`R1-00 · 设计语言`、`R1-01～03` 页面稿
- 产品依据：[Jolu 产品定位与功能边界](2026-11-23-jolu-product-positioning.md)
- 现状问题：[前端界面开发说明](../frontend-ui-guide.md)（7 种「当前项」写法、5 个 INPUT 常量、两套弹层动效）

## 1. 规则

1. **组件只在 `R1-C · 组件库` 定义。** 页面画板都是「窗口外壳」的实例，页面插槽里只放组件实例和布局行。页面需要新控件时，先在组件库加，再在页面引用。
2. **代码与画板一一对应。** 每个画板组件对应 `frontend/src/components/ui/` 下的一个同名组件，变体轴就是 props。页面代码不写交互态颜色、圆角、阴影。
3. **颜色只用 `jn-*` 变量**，浅 / 深色由 `mode` 主题切换，深色逐项取值，不用滤镜推导。
4. **数字一律经过 DataValue**，来源一律用 SourceNote，平台能力一律用 CapabilityTag。这是产品定位 §5 的数据可信度原则在界面上的落点。

## 2. 组件清单

| 分区 | 画板组件 | 变体轴 | 代码组件（替换时） | 取代的现有写法 |
|---|---|---|---|---|
| 外壳 | 窗口外壳 | — | `AppShell` | `MainPanelWindow` 外层 + `PanelPage` |
| | 侧栏 | — | `Sidebar` | `panel/SideNav.tsx` |
| | 导航项 | state: default / hover / active；collapsed: no / yes | `NavItem` | `SideNav` 内的 `NavItem` |
| | 页头 | — | `PageHeader` | `panel/PageHeader.tsx` |
| 按钮与操作 | 按钮 | kind: primary / soft / ghost / danger；size: md / sm | `Button` | `JoluButton.tsx`、各处手写按钮 |
| | 图标按钮 | kind: surface / ghost；size: md / sm | `IconButton` | 页头与卡片里的散装图标按钮 |
| | 页签项 | style: tab / segment；active: on / off | `TabItem` | `.jolu-tab`、`.jolu-segment-item`、`RubberSegment`、三处下划线页签 |
| | 页签栏 / 分段栏 | 插槽 | `TabBar` / `SegmentBar` | `.jolu-tablist`、`.jolu-segment`、`primitives.Segmented` |
| | 选项块 | active: on / off | `OptionTile` | 策略选择等自拼卡片 |
| 选择与输入 | 下拉触发器 | variant: field / filter；state: default / open / disabled | `Select` | `SelectField.tsx`（保留其宽度挡位 sm / md / lg / full） |
| | 菜单面板 + 菜单行 | state: default / hover / selected / disabled | `Menu` / `MenuItem` | `.jolu-menu`、`.jolu-menu-item`、右键菜单 |
| | 输入框 | state: default / focus / error / disabled；size: md 36 / lg 40 | `Input` | `.jolu-input`、`.jolu-input-lg`、5 个 INPUT 常量 |
| | 表单字段 | — | `FormField` | 弹窗与设置里各自拼的标签 + 输入 + 说明 |
| | 开关 | on；disabled | `Switch` | `JoluSwitch.tsx` |
| | 勾选 | state: on / off / mixed | `Checkbox` | `primitives.Checkbox` 与批量选择里的写法 |
| | 设置行 | 右侧控件插槽 | `SettingRow` | `settings/settingsUi` |
| 状态与数据 | 状态胶囊 | tone: success / active / warning / danger / info / neutral / muted | `StatusPill` | `Chip`、`StatusDot`、`.jolu-badge-*` |
| | 计数 | tone: neutral / primary / danger | `Count` | 标题后的各种数字徽章 |
| | 标签 | — | `Tag` | 描述性小标签 |
| | 能力标签 | level: verified / synthetic / pending / unsupported | `CapabilityTag` | 新增 |
| | 数据值 | state: value / zero / estimated / stale / unknown / failed / unsupported | `DataValue` | 新增；各处直接写的数字 |
| | 来源脚注 | source: official / native / routed / mixed | `SourceNote` | 新增；各处自写的「来自…」文案 |
| | 额度环 | level: normal / high / unknown；size: md / sm | `QuotaRing` | 额度展示的多种写法 |
| | 进度条 | level: normal / high / unknown | `Progress` | 上下文、额度、存储进度条 |
| | 平台标识 | platform；size: lg 40 / md 30 / sm 22 | `PlatformLogo` | `brand/PlatformLogo.tsx`（矢量保留，底板统一） |
| | 指标卡 | — | `MetricCard` | `MetricGrid.tsx` 与各页自写指标 |
| 容器与反馈 | 卡片 | — | `Card` | `.jolu-card` + 手写圆角边框 |
| | 列表行 | state: default / hover / selected | `ListRow` | `.jolu-row-*` 与各列表行 |
| | 表头 / 表格行 | stripe: on / off | `Table` / `TableRow` | `.jolu-table-head` 与两处表格 |
| | 弹窗 | — | `Dialog` | `ConfirmDialog` 与 27 个文件里的 `Dialog.Root` |
| | 通知 | tone: success / danger / info | `Toast` | `Toast.tsx` |
| | 提示 | — | `Tooltip` | `Tip.tsx` |
| | 横幅 | tone: warning / info | `Banner` | 新增 |
| | 空状态 / 骨架 | — | `EmptyState` / `Skeleton` | 15 处各写的「暂无」、`LoadingSurface` |
| 外壳 | 侧栏 · 收起 | — | `Sidebar collapsed` | 收起 72 宽，底部迷你状态换成额度环 |
| 图表 | 系列色 `jn-viz-1…4` + `jn-viz-other` | — | 令牌 | `TrendChart.tsx` 等处自定的系列色 |
| | 图例项 | — | `LegendItem` | 各图表自写图例 |
| | 横条行 | — | `BarRow` | 模型分布、供应商费用的进度条 |
| | 图表提示 | — | `ChartTooltip` | 新增；悬停时其余柱降到 55% 不透明 |

系列色顺序固定为绿 → 蓝 → 橙 → 紫，按实体分配，超过 4 个合并为「其他」。这组颜色用 dataviz 校验脚本验证过（浅色以 `#FFFFFF`、深色以 `#151917` 为底，色觉差异、亮度带、对比度全部通过）。深色的绿用 `#1FAE6B`，不用 `jn-primary` 的 `#3DD68C`，因为后者在深色底上太亮，超出图表亮度带。

## 3. 数据状态（产品定位 §5）

DataValue 的七种状态互不混用：

| 状态 | 显示 | 什么时候用 |
|---|---|---|
| value | 数字 + 单位 | 有确定值 |
| zero | `0` + 单位，正常字色 | 采集成功且真实为零 |
| estimated | `≈` 前缀 + 「估算」 | 由公式或部分数据推算 |
| stale | 数字变浅 + 「已过期 · 多久前」 | 有旧值但超过刷新周期 |
| unknown | `—` + 「未知」 | 没有数据来源能回答 |
| failed | 红色 `—` + 「采集失败」 | 采集出错；指标卡的对比行写失败原因和最后成功时间 |
| unsupported | 「不支持」 | 该平台没有这项能力 |

额度环、进度条在未知时只画轨道，不画 0%。

## 4. 动效（界面过渡）

画板只能表达静态，下面是实现约定，数值以 `R1-00 · 设计语言` 为准：

- 悬停、按下、颜色切换：CSS transition 120ms。
- 页签 / 分段滑块：选中块在项之间滑动（共享布局动画），弹簧。
- 弹出层：菜单、下拉 160ms 进 / 120ms 退；弹窗 180ms；通知从右下滑入。
- 切页：卡片自上而下错开 30ms 淡入 + 上移 6px，总时长不超过 300ms。
- 数字滚动、折线与环首屏描线、花瓣按钮运行中慢速自转。
- 减少动效：系统设置与 Jolu 设置任一开启，保留颜色变化，去掉位移和循环；由一个全局开关统一处理。

## 5. 替换时的技术选型（待拍板后执行）

- 保持 React 19 + Vite + Tailwind 4 + Radix，不换框架。
- `jn-*` 变量放进一个令牌 CSS 文件，用 Tailwind 4 的 `@theme` 暴露。
- 动效统一用 Motion（现有依赖 `framer-motion` 14.14.0 改为同版本的 `motion` 包，从 `motion/react` 导入），外层 `MotionConfig` 接管减少动效。执行前按 AGENTS.md 用 Context7 核对文档。
- 图表继续手写 SVG，需要平滑曲线时加 `d3-shape`。
- 替换顺序：先实现 `components/ui/` 全部组件和外壳，再逐页按画板重做，最后删除 `index.css` 里被取代的 `.jolu-*` 组件类和旧组件。

## 6. 待确认

- 当前页签用绿色实心（tab）还是白色滑块（segment）：画板两种都保留了，按「切换页面内容 / 筛选视图」区分，需鼠鼠确认。
- 侧栏底部的灵动岛迷你状态卡、供应商页的「最近动态」「连接安全」卡、概览页的响应耗时分布：现有功能里没有，是 R1 新增的提议。
- 收起侧栏与空 / 加载 / 失败状态的通用示范在 R1-06；其余页面各自的特殊状态尚未逐页画。
- 已画页面：R1-01/02 模型路由概览（浅 / 深）、R1-03 供应商、R1-04 用量统计概览（浅 / 深）、R1-05 会话线程（浅 / 深）、R1-06 页面状态、R1-07 一键签到、R1-08 Agent、R1-09 Agent 详情、R1-10 自动化 · 任务、R1-11 Skills 管理、R1-12～17 设置六个分区、R1-18～22 模型路由其余五个页签（路由流水、路由分组、模型池、对外访问、中间件）、R1-23 用量分析、R1-24 调用明细（含选中调用的详情与费用拆解）、R1-25 自动化 · 运行记录、R1-26 灵动岛页。R1-07 起只画了浅色，深色由实例切 `mode` 得到，未逐页截图核对。
- 画布布局（2026-11-23 整理）：左列从上到下是 R1 标题、R1-00 设计语言、R1-C 组件库；右侧从 x=8480 起按模块分行，每行有「行标题」文字，行距 1140、列距 1360，同一模块的页面与弹窗排在同一行。
- 第二批：R1-27 新建任务、R1-28 选择日期范围、R1-29 自动化运行详情、R1-30 Skill 详情、R1-31 Skill 编辑（含外部修改提示）、R1-32 / 33 Agent 详情的「高级」「写入改动」页签、R1-34 签发客户端 Key（含签发结果态）、R1-35 编辑中间件规则（含试跑结果）、R1-36 添加分组成员、R1-37 新建 API Key 连接、R1-38 通用危险操作确认。弹窗画板是 1280×900 的舞台，里面放「弹窗」组件实例，内容区由页面替换。
- 第二批里的「新会话强度」仍标「待确认」（后端没有）。危险确认按钮沿用组件库的 danger 样式（浅红底），是否改成实心红待确认。
- 供应商连接页（R1-03，2026-11-23 扩充）：主体结构不变，只把「可连接」从一行 5 个小入口扩成完整的卡片目录，按 `lib/providerCatalog.ts` 的条目分三组、每行 4 张：编码工具账号 9 张（Grok、OpenCode、Qoder / Qoder CN、WorkBuddy / WorkBuddy AI、Trae CN / Trae、ZCode 国际版）、模型服务 API 10 张（DeepSeek、Kimi、MiniMax、StepFun、Qwen、OpenRouter、SiliconFlow、Groq、Together、Fireworks）、本机与自定义 3 张（Ollama、LM Studio、自定义供应商）。卡片显示标识、名称与地区、接入方式标签、「可签到」标记（按一键签到已接入的版本），底栏写一句限制说明并给「连接」；本机已检测到登录的卡片描绿边、按钮变为「一键接入」。分组标题旁有筛选（全部 / 订阅登录 / API Key / 本机）与搜索。画板加高到 2020，下方各行整体下移。没有官方矢量标识的供应商（Trae、DeepSeek 等）暂用首字母方块，待补品牌素材。R1-48 / R1-49 两版已删除。前端架构与代码组织见 `docs/specs/2026-11-23-frontend-architecture.md`。
- 第三批：供应商行加 R1-39 账号管理（订阅看额度窗口、API Key 看余额与 Jolu 记录的消费，含认证失效、暂停查询、未提供三种状态）、R1-40 新建订阅连接（尚未获取 / 等待浏览器授权 / 已检测到账号三态并排）、R1-41 从本机导入、R1-42 批量删除账号（含部分失败与重试）；会话线程行加 R1-43 压缩会话配置、R1-44 压缩修订（对话式改草稿、选版本生成）、R1-45 交接文档预览、R1-46 在原工具继续、R1-47 创建自动化确认。R1-43 的「生成方式」在现有代码里标「待确认」，画板照搬。
- 未画：一键签到账号详情抽屉、自动化任务详情、模型路由会话观察、Skills 导入与冲突检查、新建分组、Key 限额调整、其余小确认弹窗（取消 / 删除任务、移除签到账号等，可直接套 R1-38）、各页特有的空 / 加载 / 失败状态、所有深色稿。
- R1-26 灵动岛页沿用现有功能（显示开关、样式切换、连接与分身、窗口行为、显示外观、刷新与位置）；现有代码里标「待确认」的「联动」分组没有画进来。
- 灵动岛窗口本身：AGENTS.md 规定经典 / 胶囊两种样式保持现状。2026-11-23 应鼠鼠要求出了一版联动方案 R2（A 行 R1-26 右侧，R2-I-00～04：方案思路与联动关系表、收缩态六种状态、展开卡片、请求过程 / 贴边 / 分身、联动设置），在胶囊样式基础上加「请求流向、按来源额度、谁在等、跟随主窗口、同分组岛之间的流量去向」。这只是提案：采用前要鼠鼠确认，并同步修改 AGENTS.md §1 的灵动岛例外。R2 里「预警时给出接替来源 / 改用」需要新的写操作（调整分组顺序），标「待确认」；其余用的都是已有数据（`gateway-route-start` / `gateway-route`、额度查询、`automation_waiting_connections`、`main-context`、会话归因）。
- R2 趣味部分（R2-I-05～07，联动设置 R2-I-04 加了「趣味」分组）：bot 道具与小剧场（签到礼物、额度重置充能、破百万彩带、缓存省钱、上下文吃撑、长任务完成、深夜咖啡、岛间接力传球）、戳一戳与长按快捷动作圆盘（全部签到 / 刷新额度 / 停止中转 / 打开会话 / 建自动化，不放危险操作）、看向主窗口、傍晚回顾卡与本地成就徽章。表情只用眼睛表达，沿用 `Bot.tsx` 的 `EYES` 形状（happy、star、round、gt、line、lid、pill、单眼眨）；心形眼是新增表情，待确认。待确认项：破百万阈值、长任务时长、缓存省钱的计算、回顾卡与成就（需新增本机存储）。R2 怎么实现才不像「窗口里装一个网页」见 `docs/specs/2026-11-24-island-rendering-tech.md`。2026-11-24 鼠鼠已定：采用舞台窗口（固定尺寸 + 透明区域鼠标穿透）；贴边改为「从屏幕边缘长出来」，取代 32px 迷你胶囊，R2-I-03 的「贴边迷你胶囊」一栏以该文档第 5 节为准；bot 改用 Rive，额度环仍由前端 SVG 画。
