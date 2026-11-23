# 前端界面开发说明

面向后续重构的样式与动效规范。设计依据仍是 `docs/ui/Jolu.pen`（**最高优先级**，冲突时以它为准）；本文解决的是「同一语义在代码里有几套写法」的问题。

- 基线对照画板：`docs/ui/jolu-new.pen`（当前实现 31 屏真实渲染截图）
- 重构目标画板：`docs/ui/Jolu.pen`
- 公共样式定义：`frontend/src/index.css` 的 `@layer components`

---

## 0. 一句话规则

**先找公共类，找不到再加，加在 `@layer components` 里。组件内不再手写交互态颜色。**

---

## 1. 交互态：只允许这几个类

以前「当前项」有 7 种写法，其中 4 种在浅色主题下与背景同为 `#ffffff`，等于没有选中反馈。现在统一如下。

| 语义 | 类名 | 说明 |
|---|---|---|
| 当前项（下拉选中行、列表选中行、树节点、分段当前项） | `.jolu-row-selected` | 绿底 + 绿字 |
| 悬停 | `.jolu-row-hover` | 挂在行上，`:hover` 与 `[data-highlighted]` 都生效 |
| 按下 | `.jolu-row-pressed` | `:active` |
| 禁用 | 行内用 `data-[disabled]`，面板内由 `.jolu-menu-item` 统一 `opacity .45` | |

**禁止再写**：`bg-jolu-selected`、`hover:bg-jolu-side`、`bg-jolu-bg` 作为选中底色、`bg-jolu-table-active`、`bg-jolu-selected/25`。

### 为什么不能用 `hover:bg-jolu-side`

浅色主题下 `--jolu-bg`、`--jolu-side`、`--jolu-cc-surface` **三者都是 `#ffffff`**。白底上 `hover:bg-jolu-side` 等于没有反馈。仓库曾有 59 处这种写法（`OverviewPanel.tsx`、`GatewayView.tsx` 的注释已记录该坑）。

悬停底色一律用 `--jolu-stage`（`#f4f4f5`）。

---

## 2. 组件类对照表

### 2.1 菜单 / 下拉

| 用途 | 类名 |
|---|---|
| 面板 | `.jolu-menu`（12px 圆角、边框、`--jolu-cc-surface`、统一阴影、4px 内边距） |
| 面板（大内边距）| `.jolu-menu-lg`（8px，用于带搜索框或图标栅格的菜单） |
| 行 | `.jolu-menu-item` + `.jolu-row-hover` |
| 行（带副标题）| `.jolu-menu-item-lg`（min-height 44px） |
| 菜单内搜索框 | `.jolu-menu-search` |

面板上**只加布局类**（宽度、`overflow-y-auto`、`z-index`、`motion-popover`），不再重复写圆角/边框/底色/阴影。

**下拉一律用 `components/ui/SelectField.tsx`**，禁止自建 `Menu.RadioGroup` 或原生 `<select>`。宽度用 `width` 挡位，`className` 只放视觉微调（详见 `DROPDOWN-SPEC.md`）。

### 2.2 页签 / 分段

| 用途 | 类名 |
|---|---|
| 轨道 | `.jolu-segment`（`--jolu-track` 底、3px 内边距、10px 圆角）|
| 项 | `.jolu-segment-item`，选中靠 `aria-selected` / `aria-checked` / `aria-pressed` |
| 页签（内容区顶部）| `.jolu-tablist` + `.jolu-tab`（已有，白滑块 + 阴影）|
| 需要橡皮滑块动效 | `components/ui/RubberSegment.tsx`（仅模型路由主页签在用）|

`RubberSegment` 与 `.jolu-tablist` 是**两种不同动效档位**，都是有意的：前者用于跨越多页签的长轨，后者用于普通切换。但选中态视觉必须一致（白底滑块 + 阴影 + 600 字重）。

**下划线式页签**（`AgentDetail` / `SkillDetail` / `SessionDetail`）目前三处字色与字重各不相同，重构时统一为：2px 绿下划线 + `font-semibold` + `--jolu-text`。

### 2.3 输入框

| 用途 | 类名 |
|---|---|
| 普通 | `.jolu-input`（34px / 8px 圆角）|
| 表单字段 | `.jolu-input-lg`（38px）|

以前有 5 个 `INPUT` 常量散在 `TaskDialog` / `GroupsPanel` / `NetworkSection` / `skillParts` / `handoffParts`，重构时收敛到这两档。

### 2.4 其它

| 用途 | 类名 |
|---|---|
| 表头行 | `.jolu-table-head` |
| 徽章 | `.jolu-badge` + `.jolu-badge-on` / `-muted` / `-danger` |
| 卡片 | `.jolu-card`（阴影）+ 自己写 `rounded-xl border border-jolu-border bg-jolu-cc-surface` |
| 控件阴影 | `.jolu-control`（含 `:disabled` / `:active`）|

---

## 3. 动效

### 3.1 时长与缓动

统一用 CSS 变量，不要写字面量：

| 变量 | 值 | 用途 |
|---|---|---|
| `--motion-ease` | `cubic-bezier(0.2, 0.8, 0.2, 1)` | 所有位移动画的默认缓动 |
| `--motion-feedback` | 颜色/底色反馈时长 | 悬停、选中切换 |

时长档位只保留四档：

| 档位 | 时长 | 用途 |
|---|---|---|
| 微反馈 | 120ms | 悬停、按下、颜色切换 |
| 弹出 | 160ms 进 / 120ms 退 | 菜单、通知、箭头旋转 |
| 对话框 | 180ms | 弹窗进出 |
| 滑块 | 220ms | 分段滑块位移（`RubberSegment` 例外：用弹簧）|

### 3.2 动效类

| 类名 | 用途 |
|---|---|
| `.motion-button` | 按钮悬停亮度 + 按压缩放 |
| `.motion-popover` | 菜单/浮层进（160ms）退（120ms）|
| `.motion-dialog` | 对话框本体 |
| `.motion-overlay` | 遮罩层 |

**`gw-pop` / `gw-pop-out` 是历史遗留的第二套弹层动效**，与新代码应统一到 `motion-popover`。`index.css` 里 `motion-dialog` 有两处定义（`671` 与 `1081`），时长与关闭位移不一致，重构时删掉后者。

### 3.3 减少动效

`prefers-reduced-motion: reduce` **与** `html.reduce-motion` 都要覆盖。目前 `index.css` 里有 12+ 处独立的媒体查询块，重构时收敛为一处全局兜底：

```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

例外：启动页在减少动效下应**直接呈现终态**，而不是把 1.85s 的动效压成 0.01ms（见 `startup.css`）。

---

## 4. 颜色令牌

只允许两套：主窗口 `--jolu-*`，灵动岛 `--pn-island-*` / `--pn-capsule-*`。深色逐项取值，**不要用滤镜或反色推导**。

### 4.1 容易用错的几个

| 令牌 | 浅色 | 深色 | 正确用途 |
|---|---|---|---|
| `--jolu-bg` | `#ffffff` | `#0d0d0d` | 页面底 |
| `--jolu-side` | `#ffffff` | `#141414` | 侧栏底、表头底。**不能当悬停色**（浅色下与 bg 同色）|
| `--jolu-stage` | `#f4f4f5` | `#181818` | 悬停底、浅色分区 |
| `--jolu-selected` | `#e7f7ee` | `#183126` | 当前项 |
| `--jolu-track` | `#eaeaec` | `#323235` | 轨道、进度条底 |
| `--jolu-cc-track` | `#f4f4f5` | `#242424` | 与 `--jolu-stage` 同值，重构时合并 |

---

## 5. 尺寸档位

审计发现高度出现了 22/26/28/30/32/34/36/38/42/44/52px 十一种。重构时收敛：

| 元素 | 高度 |
|---|---|
| 徽章 | 20 |
| 紧凑行 / 分段项 | 28 |
| 菜单行 | 32 |
| 输入框、筛选触发器 | 34 |
| 表单字段、工具条按钮 | 38 |
| 主操作按钮 | 40 |

字号同理：11（徽章）/ 12（辅助）/ 13（正文）/ 15（卡片标题）/ 20（区块标题）/ 24（页面标题）。

---

## 6. 基线对照

`docs/ui/jolu-new.pen` 是**当前实现的真实渲染截图**，不是重画的设计稿。用途：

1. 重构某页前，先在 `.pen` 里看这一屏现在长什么样
2. 改完后用 `node scripts/dev/check-snapshot.mjs` 重新截图，与基线对比
3. 设计稿 `Jolu.pen` 与基线的差异，就是这次重构要改的东西

重新生成基线：

```bash
pnpm snapshot                                  # 生成单文件快照
node scripts/dev/check-snapshot.mjs            # 逐屏截图到 output/snapshot-shots/
py scripts/dev/build-jolu-new-pen.py           # 重排 jolu-new.pen
```

### 注意：不要用 Pencil MCP 改 jolu-new.pen

MCP 只操作「当前打开的文档」，而当前打开的必然是 `Jolu.pen`。2026-11-22 实测 `save(uri)` 另存会**连带写脏原稿**（已从 git 恢复）。所以 `jolu-new.pen` 由 `build-jolu-new-pen.py` 直接生成 JSON，可重复、无害。

`.pen` 结构要点（pen.dev schema 2.25）：

- 顶层必须是 `{"version": "2.25", "children": [...]}`
- 每个节点要 `type` 与 `name`；id 不写，由工具生成
- `image` 填充用相对 `.pen` 的路径，所以截图要放进 `docs/ui/jolu-new-assets/`
- `layout: "vertical" | "horizontal"` 的容器里才能用 `fit_content`；绝对定位用 `layout: "none"` + `x/y`

---

## 7. 重构时的检查清单

- [ ] 没有新增手写的 `bg-jolu-selected` / `hover:bg-jolu-side`
- [ ] 新菜单面板用了 `.jolu-menu`，行用了 `.jolu-menu-item`
- [ ] 新分段控件用了 `.jolu-segment`，没有自己拼 track + pill
- [ ] 下拉用了 `SelectField`，没有新增 `Menu.RadioGroup` 或 `<select>`
- [ ] 动效时长取自四档，缓动取自 `--motion-ease`
- [ ] 减少动效下可用（`prefers-reduced-motion` 与 `html.reduce-motion` 都试）
- [ ] 浅色下悬停有可见反馈（不能是 `--jolu-side`）
- [ ] 改了界面后跑过 `check-snapshot.mjs`，与 `jolu-new.pen` 基线对照
