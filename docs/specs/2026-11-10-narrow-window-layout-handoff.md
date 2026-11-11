# 窄窗口布局适配 · 现状与剩余工作（2026-11-10）

**交给：接手成员**
**状态：** 账号管理 / 供应商连接已修复并实机验证；统计卡 2026-11-11 已按 §2.1「正确做法」修复（`components/ui/MetricGrid.tsx`，骨架屏同断点，预览中 1000px 四列、窄时两列无裁切）；会话详情按钮行改了但未在桌面端验证
**背景：** 用户反馈「窗口缩小后卡片不适配、右侧被切」「会话详情底部按钮文字折成两行」

---

## 1. 已修复（实机验证通过）

### 1.1 账号管理卡片（`components/providers/AccountsPanel.tsx`）

**问题**：固定 `grid-cols-4` + `gridAutoRows: calc((100% - 12px) / 2)`。窗口宽 1210px 时内容区约 930px，四列把每张卡压到 300px，第三张卡（末痕）右侧被容器裁掉；行高锁死还让卡片内容挤不进去。

**改动**：

```diff
- <div className="absolute inset-0 grid grid-cols-4 content-start gap-3"
-      style={{gridAutoRows:"calc((100% - 12px) / 2)", scrollbarGutter:"stable"}}>
+ <div className="absolute inset-0 grid content-start gap-3 overflow-y-auto pr-1
+      grid-cols-1 @[620px]:grid-cols-2 @[920px]:grid-cols-3 @[1200px]:grid-cols-4"
+      style={{scrollbarGutter:"stable"}}>
```

外层 `<div className="@container flex min-h-0 flex-1 flex-col gap-3" data-account-panel>` 加了 `@container`——按**容器宽度**（不是视口宽度）决定列数。

**断点依据**：单卡内容要放得下名字 + 套餐 + 邮箱 + 到期 + 额度 + 四个操作按钮，最小约 300px；加 12px 间距后 2 列需 620px、3 列需 920px、4 列需 1200px。

**比签到页（560/780/1040）高的原因**：账号卡内容更密。签到卡只有品牌 + 名称 + 状态。

**实机结果**：930px 内容区 → 2 列，三张卡完整可见，卡片高度自然（不再被 `gridAutoRows` 压扁）。

### 1.2 供应商连接网格（`views/ProvidersView.tsx`）

同一个 `grid-cols-4` 问题，一并改成 `@container` + 同一套断点（620/920/1200）。

---

## 2. 未修复（**必须继续做**）

### 2.1 统计卡四列（5 处）★ 主要遗留

| 文件 | 行 |
|---|---|
| `components/automation/TaskList.tsx` | 55 |
| `components/automation/RunList.tsx` | 31 |
| `components/automation/RunDetail.tsx` | 80 |
| `components/usage/OverviewPanel.tsx` | 47 |
| `components/usage/AnalysisPanel.tsx` | 40 |

**我改错过一次，已回退**（未提交进仓库）：从 `grid grid-cols-4 gap-3` 改成

```jsx
<div className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3">
```

**为什么错**：`auto-fit + minmax(180px, 1fr)` 在 800px 容器里能算成 4 列（4×180 + 3×12 = 756px < 800px），所以它**不换行**，但每列只剩约 190px，比卡片实际需要的最小宽度窄，内容溢出被切——实测第四张卡「今日失败」仍在右边被裁。

**正确做法**（与账号卡一致）：

1. 给各组件的容器祖先加 `@container`（`views/AutomationView.tsx` **已加**，`views/UsageView.tsx` **没加**）
2. 网格改成显式断点：

```jsx
<div className="grid grid-cols-2 gap-3 @[720px]:grid-cols-4">
```

统计卡比账号卡窄（只有 label + 数字 + 一行说明），两列在窄屏可读；720px 是「四列每列 ≥ 170px」的估算值，实机可微调。

**现状**：这 5 处与 `AutomationView` 的 `@container` **都已回退到改动前**（`grid-cols-4`），仓库里没有留下半成品。开工时按上面「正确做法」两步一起做；`UsageView.tsx` 的根节点结构与 `AutomationView` 不同，`@container` 要手动找位置。

### 2.2 会话详情按钮行（`components/sessions/SessionDetail.tsx`）

**问题**：`flex h-[34px]` 一行塞了五个元素（眼睛图标、说明、弹性占位、三个按钮），按钮**没有 `shrink-0`**。窗口变窄时浏览器压缩按钮 → 文字折成两行 → 但高度锁死 34px，文字被挤出按钮边框。

「在 `<工具名>` 中继续」的文案长度随工具名变化（`ZCode 国际版` / `Claude Code`），比其它按钮长，**总是第一个被压坏**。

**已改**（未实机验证）：
- 外层改 `flex min-h-[34px] shrink-0 flex-wrap items-center gap-x-3 gap-y-2`（窄时整体折行）
- 三个按钮各加 `shrink-0 whitespace-nowrap`，图标加 `shrink-0`
- `continueButton` 加 `whitespace-nowrap`

**需要验证**：把窗口缩到 900px 左右，打开会话线程 → 任一有记录的会话，看底部按钮是否单行显示、不折字。

### 2.3 全局按钮不折行（`index.css`）

在 `@layer base` 末尾加了：

```css
button { white-space: nowrap; }
```

**理由**：按钮高度固定（`h-[34px]` 这类），文字折行必然溢出。所有「按钮文字变两行」的问题根源都是这个。

**需要验证**：
- 有没有对话框需要按钮折行？（那种应该显式加 `whitespace-normal`）
- 有没有按钮里塞了长文本（比如错误信息）被 nowrap 撑出容器？

**风险评估**：低。若某个按钮确实需要折行，给它自己加 `whitespace-normal` 即可，全局规则可被覆盖。

---

## 3. 未确认的设计问题

**`Jolu.pen` 里账号管理页的窄屏行为没有画板依据。**

我按 `docs/specs/2026-10-27-daily-checkin-design.md` 里对**签到页**的要求（「宽屏每行四张卡片，窄屏逐级减少列数」）推断账号管理页也应如此。但账号管理页本身没找到对应画板。

**需要鼠鼠确认**：
- 如果画板规定固定四列 → 把断点改回 `grid-cols-4`，另想办法处理溢出（比如横向滚动）
- 如果画板没规定窄屏 → 当前响应式方案保留，建议后续补画板

---

## 4. 验证方法

改完后按这个顺序验（三步都要看）：

1. **宽屏**：窗口拉到 1400px 以上 → 账号卡/统计卡都应该是四列
2. **中屏**：缩到 1000px 左右 → 账号卡 3 列、统计卡 4 列（若按 720 断点）
3. **窄屏**：缩到 900px 以下 → 账号卡 2 列、统计卡 2 列，**所有卡片完整可见、无横向裁切**

截图用 `PrintWindow` 方式（不受遮挡影响）：

```powershell
# 见会话中的 /tmp/cap_pw.ps1
powershell -File cap_pw.ps1 <HWND> output.png
```

---

## 5. 同类问题排查清单

已确认有固定 `grid-cols-4` 且**需要检查**的地方（除上面 5 处统计卡）：

| 文件 | 说明 |
|---|---|
| `components/ui/LoadingSurface.tsx:20,21,24,25` | 骨架屏，也写了 `grid-cols-4`。**加载态和加载完的列数必须一致**，否则内容出现时布局会跳。改统计卡时一并改 |
| `components/providers/AccountsPanel.tsx:350` | 供应商筛选弹层 `grid-cols-3`，宽度固定 528px，不受窗口影响，**不用改** |

**排查方法**：

```bash
grep -rn "grid-cols-4\b" frontend/src --include=*.tsx | grep -v "@\["
```

---

## 6. 窗口尺寸记忆（同批修复，已完成）

不是布局问题，但同一天发现并修好了：主面板「像被固定死、不记得调整过的尺寸」。

**根因**：`record_main_window_event` 只在 `CloseRequested` 落盘。而主面板点关闭是 **隐藏**（`api.prevent_close` + `hide`），进程不退出，那条分支之后不会再有事件；只有真正退出应用（`RunEvent::Exit`）才写一次。于是拖动/缩放后的值一直在内存里，重启读的是很久以前的值。

**改动**（`backend/src/lib.rs`）：

```rust
// 新增 800ms 防抖：拖动会连发事件，每个都写 JSON 会让磁盘和设置锁跟着抖
static PENDING_PLACEMENT_MS: AtomicI64 = ...;

if matches!(event, Moved(_) | Resized(_)) {
    schedule_persist_placement(window, placement.clone());
}
```

**踩过的坑**：最初在后台线程里用 `main_window::capture()` 重取尺寸，写入的是 `1357×865` 而实际窗口 `1100×700`——后台线程与主线程窗口状态不同步，读到缩放动画中间值。改为**只用事件里累积的值**。

**实机验证**：缩放到 1024×768 @ 320,180 → 落盘 `1262×950 @ 400,225`（物理像素，DPI 1.233 换算）→ 重启恢复 1024×798 @ 320,56 ✓

---

## 7. 提交前的验证门槛

```bash
pnpm --dir frontend typecheck        # 必须过
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs
cargo test --manifest-path backend/Cargo.toml   # 窗口修复改了 lib.rs
```

**注意**：当前工作区是**未完成状态**——统计卡改了但改错、会话按钮改了未验、`index.css` 全局规则未验。提交前要么补完，要么在提交信息里如实写明「统计卡待修」。
