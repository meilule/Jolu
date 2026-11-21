# 下拉选择器前端规范

> **适用范围**：Jolu 前端所有「从一组选项里选一个」的控件。
> **强制要求**：新代码一律使用 `@/components/ui/SelectField`，禁止自建。
> **配套代码**：`components/ui/SelectField.tsx` + `src/index.css` 的 `.jolu-select-*` 段。
> **清单与迁移状态**：同目录 `DROPDOWN-INVENTORY.md`。
>
> 本文是**规范**（怎么做、为什么）；清单是**状态**（哪些已做、哪些待做）。

---

## 1. 一分钟上手

```tsx
import { SelectField } from "@/components/ui/SelectField"

const options = [
  { value: "png", label: "PNG" },
  { value: "webp", label: "WebP", description: "更小的体积" },   // 可选副标题
  { value: "svg", label: "SVG", disabled: true },
]

<SelectField
  label="导出格式"                        // 必填：同时作为 aria-label，无可见标签时也读得出
  value={format}
  options={options}
  onValueChange={setFormat}
  width="sm"                              // 宽度挡位，默认 sm
  disabled={busy}
/>
```

**默认行为就是对的**。除非有明确理由，不要额外传 `className`、不要关 `glide`。

---

## 2. 属性规范

### 2.1 必填

| 属性 | 类型 | 说明 |
|---|---|---|
| `label` | `string` | 无障碍名称。**同时是 `aria-label`**，屏幕阅读器读的就是它。写业务语义（"导出格式"），不要写"请选择" |
| `value` | `string` | 受控值。不在任何选项上时显示 `placeholder` |
| `options` | `SelectOption[]` | 见 §2.3 |
| `onValueChange` | `(v: string) => void` | 只在值真正变化时触发 |

### 2.2 可选

| 属性 | 默认 | 说明 |
|---|---|---|
| `width` | `"sm"` | 宽度挡位，见 §3。**不要用 `className` 写宽度** |
| `disabled` | `false` | 禁用态：45% 透明 + 禁用光标 |
| `placeholder` | `"请选择"` | 没有匹配值时的文案 |
| `triggerLabel` | — | 覆盖触发器显示的文案（值正确但想显示别的，如 `来源：Claude`） |
| `onOpenChange` | — | 菜单开合回调，用于联动外部状态 |
| `glide` | `true` | 共享滑块动效。**除特殊情况不要关** |
| `readOnlySingle` | `false` | 只有一个可选项时降级为纯文本（不是下拉） |
| `className` | — | **只放视觉微调与布局意图**，见 §2.4 |

### 2.3 SelectOption

```ts
interface SelectOption {
  value: string          // 必填，唯一标识，不要随显示文案变化
  label: string          // 必填，显示文案
  description?: string   // 副标题，第二行小字。行高会自适应，滑块按实测高度定位
  icon?: ReactNode       // 行首图标（16×16 容器）
  disabled?: boolean     // 单项禁用，40% 透明
}
```

**约定**
- `value` 用**稳定标识**（`"png"` / 连接 id），不要用 label 当 value——文案改了会导致选中丢失。
- `description` 是补充说明，不是第二标题。超过 290px 自动换行。
- `icon` 容器固定 `size-4`，传入的图标自己控制尺寸。

### 2.4 `className` 的三条禁令

`className` 会被拼到**触发器**上（不是外层容器）。以下三类**禁止**出现在这里：

| 禁止 | 原因 | 正确做法 |
|---|---|---|
| **宽度** `w-[142px]` / `w-full` | 与 `width` 挡位冲突，出现两套宽度来源 | 用 `width` 挡位 |
| **边框/背景** `border` / `bg-*` | 触发器自带边框与底色，重复声明会出现"双层框" | 不传 |
| **内边距/高度** `px-*` / `h-8` | 破坏统一尺寸（32px 高 / 10px 横向内边距） | 不传 |

**允许**：对齐（`self-end`）、外边距（`mt-1.5`）、圆角微调（`rounded-[6px]`，仅表格内）。
**统一外观类**：`SELECT`（定义在 `IslandView.tsx`）这类项目级外观常量可以传，它不含上述三类。

---

## 3. 宽度挡位

宽度只有四档，**不要写像素**。

| 挡位 | 实际宽度 | 使用场景 |
|---|---|---|
| `xs` | 120px | 行内短值：分钟数、百分比、小数量 |
| `sm` | **142px（默认）** | 设置页右侧的操作控件，绝大多数场景 |
| `md` | 170px | 需要读完整选项名的行内选择 |
| `lg` | 280px | 长文案（带副标题的策略名、模型 id） |
| `full` | 100% | 表单里独占一列，或表格单元格内 |

**父容器比这四档都宽时**（如 420px 的表单区块）：让父容器撑开，下拉传 `width="full"` 跟随。
**表格里 `full` 过宽时**：给单元格加 `max-w-[260px]` 限制上限，不要新增挡位。

---

## 3.5 触发器变体（variant）

两套外壳，**菜单、滑块、键盘、无障碍完全共用**，只是触发器的外观不同。

| 变体 | 外观 | 用在哪 |
|---|---|---|
| `default` | 32px 高、8px 圆角、带边框、`--surface` 底 | 绝大多数场景（默认） |
| `filter` | 36px 高、9px 圆角、**无边框无底色**（由外层 `label` 画框） | 工具栏里的筛选控件，与搜索框并列 |
| `form` | 38px 高、10px 圆角、全宽、带边框 | 表单里的下拉，与同表单的 `input` 同高 |
| `toolbar` | 34px 高、8px 圆角、带边框、可带前置图标 | 页面顶部的一排筛选（用量统计、Skills） |
| `picker` | 38px 高、**176px 宽**、`font-medium`、带边框 | 带品牌图标的资源选择器（画板 76 的供应商） |

**`filter` 变体的用法**：外框自己画，组件只负责内部：

```tsx
<label className="flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border pl-2.5 pr-2">
  <SlidersHorizontal className="size-3.5 text-jolu-muted" />
  <SelectField
    variant="filter"
    width="xs"
    className="w-auto"        // 让触发器按内容收缩，不占固定宽度
    label="按状态筛选"
    value={status}
    options={...}
    onValueChange={setStatus}
  />
</label>
```

**不要**用 `filter` 变体去改高度/内边距做别的形状——它就是"去壳"这一种语义。
新的工具条形状需要新变体（在 `SelectVariant` 里加，并同步本文档）。

#### ⚠️ 悬停驱动必须用 `pointerover`，不能用 `pointerenter`

`pointerenter` **不冒泡**，绑在菜单容器上收不到子行的事件（只在容器自身触发一次）。
滑块会完全不动。用 `pointerover`（冒泡），在 handler 里 `closest("[data-glide-value]")`
找到行。键盘导航另走 `MutationObserver` 盯 `data-highlighted`。

#### ⚠️ `filter` 变体的去壳必须写在 CSS 里，不能只靠 Tailwind 类

项目在 `index.css` 有全局规则，权重比 Tailwind 工具类高得多：

```css
html:not(.island-window) .jolu-select-trigger {
  border-color: var(--jolu-border);
  background: var(--jolu-cc-surface);
}
html:not(.island-window) .jolu-select-trigger:is(:focus-visible, [data-state="open"]) {
  border-color: var(--jolu-accent);
  box-shadow: 0 0 0 2px var(--jolu-selected);
}
```

所以只写 `border-0 bg-transparent` 是**压不住**的，表现是"外层 label 一圈框 + 触发器自己再一圈
（打开时还是绿的）"——框里套框。

正确做法：在 `index.css` 的全局规则**之后**加一组同权重的豁免规则（`.jolu-select-trigger--filter`），
逐条压回 `border` / `background` / `box-shadow` / `padding`，并去掉 `outline`。
组件侧的 `filter` 分支只保留尺寸类（`h-9 rounded-[9px] text-meta`）。

⚠️ **实现约束**：变体与宽度的类名必须**完整写在 JSX 文本里**（`variant === "filter" && "..."` 这种写法），
不能只在常量表里拼——Tailwind 静态扫描看不到运行时变量值，会漏生成 CSS。

## 4. 视觉规格（不满足就走查，不要新造）

### 4.1 触发器（`default` 变体）

| 项 | 值 |
|---|---|
| 高度 | 32px（`h-8`） |
| 圆角 | 8px |
| 横向内边距 | 左右各 10px（`px-2.5`） |
| 字号 | 12px（`text-xs`） |
| 边框 | 1px `--border` |
| 底色 | `--surface` |
| 文字 | `--text-primary` |
| 右侧箭头 | `ChevronDown`，12px，`--text-secondary` |
| 悬停 | 底色 → `--hover` |
| 展开 | 边框 → `--border-strong`，箭头旋转 180°（160ms） |
| 聚焦（键盘） | 蓝色边框 + 2px `--accent-blue-soft` 光晕 |
| 禁用 | 45% 透明 + `cursor-not-allowed` |

### 4.1b 触发器（`filter` 变体）

| 项 | 值 |
|---|---|
| 高度 | 36px（与同行搜索框对齐） |
| 圆角 | 9px |
| 边框 | **无**（由外层 `label` 画） |
| 底色 | transparent（悬停也不变） |
| 内边距 | 左 0（图标在外层）/ 右 4px |
| 字号 | `text-meta`（与工具栏一致） |
| 右侧箭头 | 12px，`--text-secondary` |

### 4.2 菜单

| 项 | 值 |
|---|---|
| 定位 | Radix Portal，自动避让视口边缘（`collisionPadding: 12`） |
| 与触发器间距 | 6px |
| 最小宽度 | 160px |
| 默认宽度 | 跟随触发器宽度（`w-[var(--radix-dropdown-menu-trigger-width)]`） |
| 最大宽度 | `min(360px, 100vw - 24px)` |
| 最大高度 | `min(248px, 可用高度)`，超出滚动 |
| 圆角 | 8px |
| 内边距 | 4px（`p-1`） |
| 边框 | 1px `--border` |
| 底色 | `--surface` |
| 阴影 | `shadow-popover` |
| 出现/消失 | 160ms opacity + translateY，关闭 120ms（快 1/3） |

### 4.3 菜单行

| 项 | 值 |
|---|---|
| 最小高度 | 34px（单选）/ 28px（行本身 `min-height`） |
| 内边距 | 上 4 / 右 28（给勾留位）/ 下 4 / 左 12 |
| 圆角 | 6px（`glide` 滑块用 7px） |
| 字号 | 12px |
| 行间距 | 1px |
| 选中行 | 底色 `--jolu-selected`（绿），文字 `--jolu-on`，右侧勾 |
| 悬停行 | 由滑块表达，**不额外涂底色** |
| 禁用行 | 40% 透明 |

---

## 5. 动效规范：共享滑块

### 5.1 是什么

菜单里只有**一个**绝对定位的滑块元素（`.jolu-select-glide__pill`），
它不改 DOM 顺序，只改 `transform: translateY()` 在行之间平移。

**为什么用它**：如果让每行自己切换底色，切换瞬间会有**两块高亮同时存在**（旧的淡出、新的淡入），
读起来像两个物体。共享滑块是**一个连续运动的物体**——这就是 iOS 分段控件的物理感。

### 5.2 两种运动模式

| 输入 | 模式 | 时长 | 为什么 |
|---|---|---|---|
| 鼠标悬停 | **滑动** | 220ms | 连续感，体现"一个物体在移动" |
| 键盘导航 | **瞬移** | 0ms | 连续按方向键时，动画会排队堆积，越按越慢 |
| 打开菜单 | **瞬移** | 0ms | 滑块直接出现在选中行，不要从第一行滑下来 |

实现：`wrap.dataset.jump` 存在时，CSS 把 `transform` 过渡时长设为 0。

### 5.3 视觉规则

| 状态 | 滑块 | 说明 |
|---|---|---|
| 指针在**非选中行** | **显示**，灰色 `--surface-3` | 正常滑动 |
| 指针在**选中行** | **隐藏**（opacity 0） | 选中行已有绿色底，叠灰块会脏 |
| 指针**离开菜单** | **停在最后停留的行，不回弹** | 对应参考实现的 `rememberPosition: true` |
| 重新打开菜单 | 回到选中行并隐藏 | 每次打开都从"当前值"开始 |

### 5.4 几何精度（踩过的坑，不要改回去）

滑块的位置/尺寸**必须按目标行的实际矩形量取**，写进 CSS 变量：

```
--gs-pill-y  → t.top    - wrap.top     垂直位置
--gs-pill-h  → t.height                高度（支持带 description 的双行项）
--gs-pill-x  → t.left   - wrap.left    水平位置
--gs-pill-w  → t.width                 宽度
--gs-pill-o  → 0 | 1                   可见性
```

**为什么不用 `left: 0; right: 0`**：容器有 4px 内边距，那样写滑块会比绿色选中底两边各窄 4px，
视觉上"长短不一"。按实测矩形量取后两者严格等宽（差值 0）。

**为什么写在 wrap 的 CSS 变量上，而不是滑块的内联 style**：
滑块是 React 渲染的节点，每次重渲染（切值、Radix 更新 `aria-checked`）都会把手写的内联
style 清回 CSS 默认值。wrap 的 style 由我们自己管，React 不碰。

### 5.5 驱动方式

| 输入源 | 驱动 | 说明 |
|---|---|---|
| 鼠标 | 自己的 `pointerenter`（捕获阶段） | 不依赖 Radix 的 `data-highlighted`——它在菜单刚开、焦点不在列表时可能不写 |
| 键盘 | `MutationObserver` 盯 `data-highlighted` | 300ms 内按过键才认，避免误判 |

**不要用 `pointerover`**：Radix 内部处理指针后改属性，属性变化不触发事件。

### 5.6 减少动效

`prefers-reduced-motion: reduce` 或 `html.reduce-motion` 时：
滑块只做 opacity 淡入淡出（150ms），**不做位移**。

---

## 6. 无障碍

| 项 | 要求 |
|---|---|
| 角色 | 触发器 `role="combobox"`，菜单 `role="listbox"`，行 `role="option"`（Radix 自动） |
| 名称 | `label` → `aria-label`。**必填，不可省略** |
| 状态 | `aria-expanded` / `aria-selected` / `aria-disabled`（Radix 自动） |
| 活动项 | `aria-activedescendant` 指向当前高亮行（Radix 自动） |
| 键盘 | ↑↓ 移动、Home/End 首尾、Enter/Space 选中、Esc 关闭、Tab 离开（Radix 自动） |
| 首字母跳转 | 打字自动跳到下一个匹配项（Radix 自动） |
| 焦点返回 | 选中后焦点回到触发器（Radix 自动） |

**因此不要自己实现键盘逻辑**——复用 Radix 就自动获得以上全部。

---

## 7. 新增下拉的检查清单

写完后逐条确认：

- [ ] 用的是 `SelectField`，不是 `Menu.RadioGroup`，不是原生 `<select>`
- [ ] `label` 写了业务语义
- [ ] `value` 是稳定标识，不是 label
- [ ] `width` 用了四档之一，**没有** `w-[...]` / `w-full` 出现在 `className` 里
- [ ] `className` 里**没有** `border` / `bg-*` / `px-*` / `h-*`
- [ ] 没有关掉 `glide`（除非有明确理由并在代码里注释）
- [ ] 没有自己写键盘/焦点逻辑
- [ ] `pnpm typecheck` 通过

### 可自动化检查的命令

```bash
cd frontend/src

# 1) 没有自建单选（应为空）
grep -rn "Menu.RadioItem" --include="*.tsx" . | grep -v "SelectField.tsx"

# 2) 没有原生 select（应为空）
grep -rn "<select" --include="*.tsx" .

# 3) 没有手写宽度（应为空）
grep -rn "<SelectField" --include="*.tsx" . | grep -E 'w-\[|w-full'

# 4) 宽度挡位只在四档内
grep -rn "<SelectField" --include="*.tsx" . | grep -oE 'width="[a-z]+"' | sort -u
```

---

## 8. 反例（这些写法会被 review 打回）

```tsx
// ❌ 自建单选：键盘、焦点、无障碍全要自己写，且样式会漂移
<Menu.Root><Menu.RadioGroup>…</Menu.RadioGroup></Menu.Root>

// ❌ 原生 select：系统绘制，无法做滑块动效，样式不可控
<select><option>…</option></select>

// ❌ 手写宽度：与 width 挡位冲突
<SelectField className="w-[142px]" />

// ❌ 把整套外观类串进来：套出"外层一层框 + 触发器一层框"
<SelectField className={cn(SELECT, "border bg-surface px-2.5 h-8 w-[142px]")} />

// ❌ 用 label 当 value：改文案就丢选中
options={["PNG", "JPG"].map(s => ({ value: s, label: s }))}
// ✅
options={[{ value: "png", label: "PNG" }, { value: "jpg", label: "JPG" }]}

// ❌ 关掉动效（除非有注释说明理由）
<SelectField glide={false} />

// ❌ 自己实现键盘导航
<Menu.Root onKeyDown={handleArrows}>…
```

---

## 9. 已废弃写法对照

| 旧写法 | 新写法 | 备注 |
|---|---|---|
| `className="w-[142px]"` | `width="sm"` | 宽度挡位 |
| `className={cn(SELECT, "w-[170px]")}` | `className={SELECT} width="md"` | 外观类 + 挡位分离 |
| `Menu.RadioGroup` 自建 | `SelectField` | 4 处待迁移，见清单 §3 |
| 原生 `<select>` | `SelectField` | 1 处待迁移 |
| 每行各自 `bg-*` 高亮 | 共享滑块 | `glide` 默认开启 |

---

## 10. 文件位置

| 文件 | 作用 |
|---|---|
| `components/ui/SelectField.tsx` | 组件实现（唯一入口） |
| `src/index.css` → `.jolu-select-*` | 样式与动效 |
| `src/index.css` → `.jolu-select-glide*` | 共享滑块 |
| `components/ui/DROPDOWN-INVENTORY.md` | 全量清单与迁移状态 |
| `components/ui/DROPDOWN-SPEC.md` | **本文** |
