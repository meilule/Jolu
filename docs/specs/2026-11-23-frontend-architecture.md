# 前端架构与代码组织（新版界面替换用，2026-11-23）

**适用范围**：按 `docs/ui/jolu-new.pen`（R1 / R2）统一替换主窗口前端时遵守。现在的代码在替换完成前保持原样。
**配套文档**：组件清单 `docs/specs/2026-11-23-jolu-new-ui-components.md`（画板组件 ↔ 代码组件对照）；旧代码的交互类与动效规则 `docs/frontend-ui-guide.md`（替换完成后以本文为准）。

## 0. 结论

不换框架。React 19 + Vite + TypeScript + Tailwind CSS 4 + Radix 继续用，只补三处并立三条规矩：

| 补什么 | 选型 | 原因 |
|---|---|---|
| 动效 | `motion`（14.14.0，从 `motion/react` 导入，替掉同版本的 `framer-motion` 包名） | 只有它能做 `layoutId` 滑块、`AnimatePresence` 退场和弹簧，且能用 `MotionConfig` 一处接管「减少动效」 |
| 平台 / 品牌图标 | `vite-plugin-svgr`（5.2.0）把 `.svg` 编成 React 组件 | 品牌图标放文件就能用，不用再把路径写进 TSX |
| 页签、弹出层 | 补 `@radix-ui/react-tabs`、`@radix-ui/react-popover` | 键盘与无障碍交给 Radix；现在 6 种页签写法各管各的焦点 |

三条规矩：
1. **页面只组合，不定义样式。** 页面文件里不写颜色、圆角、阴影、动效；需要新样子就在 `ui/` 加一个变体。
2. **每样东西只有一个入口。** 图标从 `@/icons` 拿，动效从 `@/motion` 拿，颜色从 `jn-*` 令牌拿，下拉用 `SelectField`。ESLint 规则拦住绕过入口的写法（见 §6）。
3. **数据和界面分开。** `lib/` 放纯逻辑与 API，`features/<模块>/` 放这个模块的 hooks 与组件，`ui/` 不认识任何业务数据。

不建议的方向：
- 不上 Recharts / ECharts 这类整套图表库。默认样式和动画很难调成画板的样子，继续手写 SVG，需要平滑曲线时只加 `d3-shape` 算路径。
- 不上 CSS-in-JS（styled-components、Emotion）。Tailwind 4 的 `@theme` 已经能把令牌变成类名，再加一层只会多一套写法。
- 不上全局状态库。现在的跨窗口同步走 Tauri 事件 + hooks，问题不在状态管理，在组件重复。真有需要再看 Zustand。
- 灵动岛的 bot 已定用 Rive（2026-11-24），只在浮动灵动岛窗口里加载，和界面过渡的 Motion 分开。输入契约、资源位置与回退方式见 `docs/specs/2026-11-24-island-rendering-tech.md` 第 6 节。

## 1. 目录结构

```
frontend/src/
  app/                     窗口入口、路由（页面切换）、全局 Provider
    MainWindow.tsx
    providers.tsx          MotionConfig、Tooltip.Provider、主题与密度
  theme/
    tokens.css             jn-* 颜色、圆角、间距、阴影、字号（浅 / 深两套，对应 Jolu.pen 变量）
    motion.css             CSS 层的动效变量与少量 @keyframes（见 §3）
    base.css               重置与全局元素样式，不放组件类
  icons/
    index.ts               唯一出口：export { Icon, PlatformIcon, AgentIcon }
    ui.ts                  界面图标注册表（lucide 名 → 语义名）
    platform/*.svg         平台标识母版（claude.svg、codex.svg …）
    agent/*.svg            Agent 工具标识
    platform.ts            平台 id → 组件、品牌色、是否单色
  motion/
    index.ts               唯一出口
    tokens.ts              时长、曲线、弹簧参数（与 motion.css 同值）
    presets.ts             fade、pop、slideUp、listItem 等 variants
    components.tsx         <Reveal> <Pop> <Stagger> <Collapse> <NumberRoll> <SlideIndicator>
  ui/                      组件库（= R1-C）。不认识任何业务数据
    shell/                 WindowShell、Sidebar、NavItem、PageHeader、PanelPage
    actions/               Button、IconButton、Tabs（tab / segment）、OptionTile
    inputs/                SelectField、Input、FormField、Switch、Checkbox、SettingRow
    data/                  StatusPill、Count、Tag、CapabilityTag、DataValue、SourceNote、QuotaRing、Progress、MetricCard
    containers/            Card、ListRow、Table、Dialog、ConfirmDialog、Toast、Tooltip、Banner、EmptyState、Skeleton、BatchBar、Pagination
    charts/                Legend、Bar、BarRow、Heatmap、ChartTooltip
    index.ts               统一导出
  features/                按模块分，一个模块一个目录
    providers/
      ProvidersPage.tsx
      connect/             连接页：目录、检测、接入面板
      accounts/            账号管理页
      hooks/               useConnections、useLocalDetection …
    usage/  sessions/  automation/  skills/  agents/  gateway/  checkin/  settings/  island-page/
  island/                  浮动灵动岛窗口（独立窗口，自带 capsule 主题）
  lib/                     纯逻辑、API 桥接、平台目录（现在的 lib/ 基本不动）
  mock/                    浏览器预览示例数据
```

和现在的对应关系：
- `components/ui/`、`components/brand/`、`components/panel/` → 拆进 `ui/` 和 `icons/`。
- `components/<模块>/` + `views/<模块>View.tsx` → 合并到 `features/<模块>/`，页面和它专用的组件放在一起，找一个功能只看一个目录。
- `hooks/` 里只被一个模块用的 hook 搬进那个模块的 `features/<模块>/hooks/`；跨模块的留在 `hooks/`。
- `index.css`（1485 行）拆成 `theme/` 三个文件，页面专用的 `.jolu-*` 类随组件迁走后删除。

路径别名：`@/ui`、`@/icons`、`@/motion`、`@/features/*`、`@/lib/*`（`tsconfig.json` 与 `vite.config.ts` 同时配）。

## 2. 图标

现状：80 个文件直接从 `lucide-react` 导入，共 141 个不同图标；平台标识一部分是 `PlatformLogo` 里手写的路径，一部分是 `assets/` 里的图片；同一个意思在不同页面用了不同图标（例如「刷新」有 `RefreshCw` 和 `RotateCw`）。

### 2.1 三类图标，三个组件

| 组件 | 内容 | 来源 |
|---|---|---|
| `<Icon name="refresh" />` | 界面图标 | lucide，经 `icons/ui.ts` 注册表映射 |
| `<PlatformIcon id="claude" size="md" />` | 供应商 / 平台标识 | `icons/platform/*.svg`，svgr 编译 |
| `<AgentIcon id="cline" />` | Agent 工具标识 | `icons/agent/*.svg` 或图片 |

### 2.2 界面图标注册表

```ts
// icons/ui.ts —— 页面只能用这里的语义名
import { RefreshCw, Plus, Trash2, ChevronRight, CircleCheck, /* … */ } from "lucide-react"

export const UI_ICONS = {
  refresh: RefreshCw,
  add: Plus,
  delete: Trash2,
  next: ChevronRight,
  success: CircleCheck,
  // …
} as const

export type IconName = keyof typeof UI_ICONS
```

```tsx
// icons/index.ts
const SIZE = { xs: 12, sm: 14, md: 16, lg: 20 } as const

export function Icon({ name, size = "md", className, label }: {
  name: IconName; size?: keyof typeof SIZE; className?: string; label?: string
}) {
  const C = UI_ICONS[name]
  return <C size={SIZE[size]} strokeWidth={1.75} className={className}
    aria-hidden={label ? undefined : true} aria-label={label} />
}
```

好处：
- 换图标只改注册表一行，全站跟着变。
- 语义名统一后，「刷新」不会再有两种画法。
- 尺寸与线宽只有四档，和画板的 12 / 14 / 16 / 20 一致。
- 只导入注册表里的图标，打包体积不会随着到处 `import` 而失控。

### 2.3 平台标识

- 母版放 `icons/platform/claude.svg` 这样的文件，`fill="currentColor"` 的单色图跟随文字颜色，多色图保留原色。
- `icons/platform.ts` 用 `import.meta.glob("./platform/*.svg", { query: "?react", eager: true })` 一次性收集，按文件名生成 id → 组件的表，新增平台只需要放一个文件。
- 品牌底色（`jn-logo-*`）和「是否单色」写在同一张表里，`PlatformIcon` 根据它决定画底块还是直接画图。
- `Jolu.pen` 引用的图片路径改了，要同时用 Pencil 更新设计稿（AGENTS.md §1 的规定不变）。

## 3. 动效

现状：只有 `JoluSwitch.tsx` 用了 `framer-motion`；`index.css` 里 14 个 `@keyframes`、`motion-dialog` 两处定义不一致、`gw-pop` 与 `motion-popover` 两套弹层动效并存，「减少动效」的媒体查询散在 9 个文件。

### 3.1 分两层

| 层 | 负责 | 写在哪 |
|---|---|---|
| CSS | 悬停、按下、颜色切换这类即时反馈 | `theme/motion.css` 的变量 + 组件里的 `transition-*` 类 |
| Motion | 进出场、布局变化、滑块、弹簧、数字滚动、列表依次入场 | `motion/` 里封装好的组件 |

规则：**页面里不直接写 `<motion.div>`，也不写 `transition={{ … }}` 字面量。** 页面只用 `motion/` 导出的组件或 `ui/` 里已经带动效的组件。

### 3.2 令牌（一处定义，CSS 与 JS 同值）

```ts
// motion/tokens.ts
export const DURATION = { feedback: 0.12, pop: 0.16, popOut: 0.12, dialog: 0.18, slide: 0.22 } as const
export const EASE = { standard: [0.2, 0.8, 0.2, 1], exit: [0.4, 0, 1, 1] } as const
export const SPRING = {
  snappy: { type: "spring", stiffness: 520, damping: 38 },   // 页签滑块、开关
  soft:   { type: "spring", stiffness: 260, damping: 30 },   // 卡片展开
  island: { type: "spring", stiffness: 380, damping: 26 },   // 灵动岛形变（对应 cubic-bezier(.32,1.28,.48,1)）
} as const
```

`theme/motion.css` 里的 `--motion-feedback: 120ms` 等变量与上表同值，改一处时两处一起改（加一条测试比对两边数值，见 §6）。

### 3.3 封装好的动效组件

| 组件 | 用在哪 | 替掉的旧写法 |
|---|---|---|
| `<Pop>` | 菜单、下拉、Tooltip、通知 | `motion-popover`、`gw-pop` / `gw-pop-out` |
| `<DialogMotion>` | `ui/containers/Dialog` 内部 | 两处不一致的 `motion-dialog` |
| `<SlideIndicator layoutId>` | `ui/actions/Tabs` 的滑块（tab 与 segment 共用） | `RubberSegment`、`.jolu-segment`、三处下划线页签 |
| `<Reveal>` / `<Stagger>` | 页面切换后卡片依次入场 | 各页自写的入场动画 |
| `<Collapse>` | 手风琴、展开表单、高级设置 | 各处 `max-height` 过渡 |
| `<NumberRoll>` | 指标卡数字、Token 增量 | 灵动岛与概览页各写的一套 |
| `<Presence>` | 列表增删、状态切换（`AnimatePresence` 的薄封装，固定 `mode` 与退场参数） | 直接用 `AnimatePresence` |

组件内部用 `LazyMotion` + `m.*` 按需加载动画特性，减小首屏体积。

### 3.4 减少动效：一处接管

```tsx
// app/providers.tsx
<MotionConfig reducedMotion={reduceMotionSetting ? "always" : "user"} transition={{ duration: DURATION.pop, ease: EASE.standard }}>
```

- `"user"` 跟随系统；Jolu 设置里打开「减少动效」时传 `"always"`。
- CSS 层只保留一处全局兜底（`theme/motion.css`），删掉散落的 9 处媒体查询。
- 例外照旧：启动页、灵动岛在减少动效下直接呈现终态，只换颜色和文字。

### 3.5 动效性能规则

- 只动 `transform` 和 `opacity`；宽高变化用 `layout` 属性交给 Motion 算。
- 形变期间关掉 `backdrop-filter`（胶囊灵动岛踩过这个坑）。
- 列表超过 30 项不做依次入场，直接整体淡入。

## 4. 组件库约定

- **一个组件一份变体表。** 用 `cva` 风格的对象（或 `tailwind-variants`）列出 `kind` / `size` / `state`，和画板组件的变体名一一对应；页面传变体名，不传类名。
- **`className` 只用于布局**（宽度、外边距、网格位置），不用于改颜色或圆角。要改就加变体。
- **数据状态统一走 `DataValue`。** 七种状态：有值、真实的零、估算、已过期、未知、采集失败、不支持（产品定位文档 §3）。额度环、进度条、指标卡内部都用它，页面不自己判断「显示 0 还是 —」。
- **来源说明统一走 `SourceNote`**，能力状态统一走 `CapabilityTag`。
- **弹窗统一走 `Dialog`**：标题、说明、内容、底部按钮四个槽，危险确认用 `ConfirmDialog`。新弹窗不再自写外壳（R1-27～47 全部是这一个组件的实例）。
- 每个 `ui/` 组件在 `ui/<分类>/<组件>.stories.tsx` 或一张预览页里列出所有变体，对照 R1-C 截图检查。不引入 Storybook 的话，用一个只在 dev 打开的 `/__ui` 预览路由即可。

## 5. 页面（feature）怎么写

一个模块的目录长这样：

```
features/providers/
  ProvidersPage.tsx        只负责页签切换和布局
  connect/
    ConnectPage.tsx        组合：ProviderSearch + LocalDetections + ProviderCatalog + ConnectPanel
    ProviderCatalog.tsx
    ConnectPanel.tsx
  accounts/
    AccountsPage.tsx
    AccountCard.tsx
  hooks/
    useConnections.ts
    useLocalDetection.ts
  providers.test.mjs       纯逻辑测试（沿用现在的 node --test）
```

- 页面文件控制在 300 行以内。现在超过 700 行的 `AccountsPanel.tsx`（988 行）、`CapsuleIsland.tsx`（749 行）、`SessionObservation.tsx`（732 行）在替换时按区块拆开。
- 数据获取写在 hook 里，组件只拿结果和动作函数；加载、空、失败三种状态用 `ui/` 的 `Skeleton`、`EmptyState`、`Banner`，不各写一套。
- 不允许 feature 之间互相导入组件。要共用就提到 `ui/`（纯展示）或 `lib/`（逻辑）。

## 6. 防止再乱：自动检查

| 检查 | 拦什么 | 实现 |
|---|---|---|
| `no-restricted-imports` | 页面直接 `import … from "lucide-react"`、`"motion/react"`、`"framer-motion"` | ESLint（项目目前没有 ESLint，替换时加，只开这几条和 TS 推荐规则） |
| 禁止颜色字面量 | `className` 里出现 `#xxxxxx`、`text-[#…]`、`bg-[#…]` | ESLint 自定义规则或一段 `node --test` 扫描 |
| 禁止 `text-[10px]` 这类任意字号 | 字号绕过档位 | 同上 |
| 令牌同步 | `motion/tokens.ts` 与 `theme/motion.css` 数值不一致；`theme/tokens.css` 少了 `Jolu.pen` 的某个 `jn-*` 变量 | `node --test` 读两边比对（和现在 `agentPage.test.mjs` 比对接线表的做法一样） |
| 跨 feature 导入 | `features/a` 导入 `features/b` | `no-restricted-imports` 的 patterns |

这些检查进 `pnpm --dir frontend typecheck` 之后的同一条命令，提交前跑。

## 7. 替换顺序

1. **地基**：`theme/`（令牌）、`icons/`、`motion/`、ESLint 规则。旧页面不动，先让新入口存在。
2. **组件库**：按 R1-C 的六个分区实现 `ui/`，配 `/__ui` 预览页逐个对照截图。
3. **外壳**：`WindowShell`、`Sidebar`、`PageHeader`，主窗口换成新外壳，旧页面先放在新外壳里跑。
4. **逐页替换**：模型路由概览（试点）→ 供应商 → 用量统计 → 会话线程 → 一键签到 → 自动化 → Skills → Agent → 模型路由其余页签 → 设置 → 灵动岛页。每页替换完删掉它的旧组件和旧 CSS 类。
5. **收尾**：删除 `index.css` 里剩下的 `.jolu-*` 组件类、`RubberSegment`、`gw-pop`；更新 AGENTS.md 的设计依据与模块表。

每一步都能单独提交、单独验证；中途停下来，已替换的页面和没替换的页面也能同时运行。

## 8. 待确认

- 是否引入 ESLint（目前项目只有 `tsc` 检查）。
- 组件预览用 `/__ui` 路由还是 Storybook。
- 浮动灵动岛窗口是否也迁到 `motion/` 令牌：轮廓弹簧用 `SPRING.island`（见灵动岛技术方案第 4 节），其余随灵动岛改造一起定。
- `vite-plugin-svgr`、`@radix-ui/react-tabs`、`@radix-ui/react-popover` 与 `motion` 的用法，动手前按 AGENTS.md 用 Context7 核对文档（本次 Context7 请求失败，版本号来自 `npm view`）。
