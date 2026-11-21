# 下拉 / 选择器统一清单（Dropdown inventory）

> **规范正文见 [DROPDOWN-SPEC.md](./DROPDOWN-SPEC.md)**（样式、动效、属性、反例）。
> 本文是**清单**：哪些已统一、哪些待迁移。
>
> 目的：全前端的「选一个值」类控件统一走同一套样式与动效，避免各页面各写一套。
> 口径：**单选下拉一律用 `@/components/ui/SelectField`**（已默认带共享滑块动效）；
> 多选菜单、命令菜单、筛选标签不属于下拉，不在此列。
>
> 最后盘点：2026-11-21 · 共 41 处 `SelectField` 调用 + 1 处原生 `<select>` + 3 处自建单选。

---

## 1. 标准实现（复用这套）

| 文件 | 说明 |
|---|---|
| `components/ui/SelectField.tsx` | **唯一入口**。Radix DropdownMenu + `glide` 共享滑块动效（默认开启） |
| `index.css` → `.jolu-select-glide*` | 滑块样式（`--gs-pill-y/h/o` 变量驱动） |
| `index.css` → `.jolu-select-trigger` / `.jolu-select-menu` | 触发器与菜单的项目主题（`html:not(.island-window)` 作用域） |

### 用法

```tsx
<SelectField
  label="可见的名称（也是 aria-label）"
  value={value}
  options={[{ value: "a", label: "A", description?: "副标题", disabled?: false }]}
  onValueChange={(v) => setValue(v)}
  disabled={busy}
  width="sm"              // 宽度挡位，见下表
  className={SELECT}      // 只传视觉/布局微调，不要再写宽度
/>
```

### 宽度挡位（统一口径，别写像素）

| 挡位 | 宽度 | 用在哪 | 现状 |
|---|---|---|---|
| `xs` | 120px | 行内短值（分钟数、百分比） | 1 处 |
| `sm` | 142px | 设置页右侧操作控件（**默认**） | 3 处显式 + 28 处默认 |
| `md` | 170px | 需要读完整选项名的行内选择 | 1 处 |
| `full` | 100% | 表单里独占一列（配合父容器） | 9 处 |

**为什么统一**：之前同一界面里混着 `w-[142px]` / `w-[120px]` / `w-[170px]` / `w-full`
和 30 处靠父容器撑开的收缩宽度，宽窄不一致。现在全部收敛成四个语义值。

**约定**
- `className` **只承载视觉微调**（`SELECT` 那种统一外观类）和布局意图（对齐、外边距）。
  **不要写宽度** `w-[...]` / `w-full`，用 `width` 挡位。
  也不要写 `border` / `bg` / `px` / `h` —— 那会套出「外层一层框 + 触发器一层框」。
- 需要旧的无滑块行为时传 `glide={false}`（目前无人使用）。
- 选项文案带副标题用 `description`；行高会自适应，滑块按实测高度定位。
- 父容器需要更宽时（如 `AgentDetail` 那个 `w-[420px]` 的 span），让它撑开即可，
  下拉传 `width="full"` 跟随父容器。

---

## 2. 已统一（41 处，无需再改）

| 文件 | 处数 | 场景 |
|---|---|---|
| `components/agents/AgentDetail.tsx` | 7 | 模型 / 思考强度 / 子代理 / 自动模式 |
| `components/gateway/GroupsPanel.tsx` | 7 | 分组、策略、成员档位、意图分类模型 |
| `views/IslandView.tsx` | 6 | 透明度、大小、缩小后大小、连接来源、显示位置、刷新间隔 |
| `components/gateway/AccessPanel.tsx` | 3 | 有效期、示例模型、并发数 |
| `components/gateway/MiddlewarePanel.tsx` | 2 | 规则类型、思考段处理 |
| `components/agents/CustomAgentPanel.tsx` | 2 | 预设工具、写入模型 |
| `components/providers/ConnectDialog.tsx` | 2 | 模型服务、服务区域 |
| `components/sessions/CompressDialog.tsx` | 3 | 执行压缩的 AI、压缩模型、生成方式、交接给 |
| `components/gateway/SourceManageDialog.tsx` | 1 | 来源所在分组 |
| `components/island/IslandConnectionSwitcher.tsx` | 1 | 切换连接 |
| `components/providers/AccountsPanel.tsx` | 1 | 账号相关选择 |
| `components/settings/AboutSection.tsx` | 1 | 检查间隔 |
| `components/settings/SearchSection.tsx` | 1 | 搜索范围 |
| `components/settings/settingsUi.tsx` | 1 | 通用设置项 |

---

## 3. 待统一（4 处，各写了一套）

### ~~3.1 原生 `<select>`~~ ✅ 已完成（2026-11-21）
`components/gateway/OverviewPanel.tsx` 的会话状态筛选已换成 `SelectField variant="filter"`：
外层 `label` 保留图标与圆角边框，触发器去壳（`variant="filter"`），菜单与滑块与其它下拉一致。
`SelectField` 因此新增了 `SelectVariant`（`default` / `filter`）与 `lg` 宽度挡位（280px）。

### 3.2 `components/automation/TaskDialog.tsx:82` — `FieldSelect`
自建的单选下拉，`RadioGroup + RadioItem`，**有 tag / note 两行布局**。
→ 可直接换成 `SelectField`：`tag` 映射到 `description`，其余一致。

### ~~3.3 `components/skills/skillParts.tsx` — `FieldSelect`~~ ✅ 已完成（2026-11-21）
已改为 `SelectField` 的薄封装：外观走新增的 `variant="form"`（38px 高、10px 圆角、全宽，
与画板 80/82 的 input 同高），菜单 / 滑块 / 键盘 / 无障碍全部来自 SelectField。
`FieldOption.note` 映射为 `description`（显示成第二行小字）。
skills 的三处调用（供应商 / 作用范围 / 安装范围）无需改动。

### 3.4 `components/usage/usageControls.tsx:106` — `FilterSelect`
自建单选，**带 icon**，且用的是 `FilterTrigger`（筛选条触发器，不是标准 input 外观）。
→ 需要保留筛选条外观：`SelectField` 支持 `triggerLabel`，但触发器样式是固定的。
   建议先保留，或给 `SelectField` 加一个 `triggerVariant` 道具。

---

## 4. 不属于下拉（列出来避免误改）

这些用了 `@radix-ui/react-dropdown-menu` 但语义是**命令菜单 / 多选 / 操作列表**，
不应套用共享滑块：

| 文件 | 用途 |
|---|---|
| `components/automation/automationParts.tsx` | 9 个普通菜单项 |
| `components/gateway/SourceManageDialog.tsx` | 6 个操作项 |
| `components/skills/SkillsTable.tsx` | 4 个批量操作 |
| `components/skills/skillDialogs.tsx` | 3 个操作 |
| `components/providers/ConnectDialog.tsx` | 2 个操作 |
| `components/providers/AccountsPanel.tsx` | 1 个操作 |
| `components/sessions/SessionDetail.tsx` | 2 个操作 |
| `components/skills/SkillEditor.tsx` | 2 个操作 |
| `views/IslandView.tsx` | 1 个操作菜单 |
| `components/skills/skillParts.tsx` | 2 个普通菜单项 |

---

## 5. 统一检查方法

改了任何下拉后，跑这三条：

```bash
# 1) 确认没有新增自建单选（应只剩 3 处待清理）
cd frontend/src && grep -rn "Menu.RadioItem" --include="*.tsx" .

# 2) 确认没有新增原生 select
grep -rn "<select" --include="*.tsx" .

# 3) 确认没有 SelectField 还手写宽度（应该都用 width 挡位）
grep -rn "<SelectField" --include="*.tsx" . | grep -E 'w-\[|w-full'

# 4) 看宽度挡位分布（应只有 xs/sm/md/full 四种）
grep -rn "<SelectField" --include="*.tsx" . | grep -oE 'width="[a-z]+"' | sort | uniq -c
```

第 3 条应该**没有输出**（父容器的宽度不算，那是 `<span>` / `<div>` 上的）。
第 4 条出现 `full/sm/md/xs` 以外的值就是错的。

`className` 的人工判断标准：出现 `border` / `bg-` / `px-` / `w-` 就该质疑。
