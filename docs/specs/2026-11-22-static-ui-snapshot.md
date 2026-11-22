# 静态界面快照

给前端样式重构做对照基线用的单文件 HTML。它**复用真实组件**渲染，不是重画的稿子，所以像素、间距、字体、深浅色、动效都和当前实现一致。

- 产物：`output/jolu-ui-snapshot.html`（约 2.5 MB，字体与图标已内联，可直接双击打开）
- 生成：`pnpm snapshot`
- 自检：`node scripts/dev/check-snapshot.mjs`（无头 Chromium 逐页截图 + 控制台错误检查）

## 为什么这么做

浏览器预览（`pnpm dev`）里，Agent 与模型路由两页直接显示「需要在桌面端使用」——它们的所有数据都来自后端命令，没有 mock 分支。要在样式重构时对照这两页，就必须让组件相信自己跑在桌面端。

做法是在导入业务模块**之前**装一层假 IPC：

```
installFakeTauri(...)   // 先装 __TAURI_INTERNALS__
  ↓ 动态 import
lib/api.ts 的 isTauri = true
  ↓
MainPanelWindow / IslandWindow 走桌面分支
  ↓
每个 invoke(cmd) 命中 snapshotCommands.ts 的路由表 → 返回示例数据
```

关键顺序在 `src/snapshot/entry.tsx`：`isTauri` 是 `lib/api.ts` 的模块级常量，必须在导入时就为真，所以业务模块只能用动态 `import()` 加载。

## 目录

| 文件 | 作用 |
|---|---|
| `frontend/snapshot.html` | 快照入口 HTML |
| `frontend/vite.snapshot.config.ts` | 快照构建配置（单入口、不拆包、资源内联） |
| `frontend/src/snapshot/entry.tsx` | 先装假 IPC，再挂载 |
| `frontend/src/snapshot/fakeTauri.ts` | IPC 替身：事件总线、窗口 / 应用 / opener 插件桩 |
| `frontend/src/snapshot/snapshotCommands.ts` | 命令路由表：每个 invoke 命令 → 示例数据 |
| `frontend/src/snapshot/snapshotData.ts` | 设置、连接、额度、余额、灵动岛实时用量 |
| `frontend/src/snapshot/snapshotUsage.ts` | 用量统计三页、会话线程消息 |
| `frontend/src/snapshot/snapshotModules.ts` | 一键签到、Skills、自动化 |
| `frontend/src/snapshot/snapshotGateway.ts` | Agent 探测与接线、模型路由各页签、设置杂项 |
| `frontend/src/snapshot/mount.tsx` | 快照外壳：主面板 / 灵动岛、浅深色、样式与停靠切换 |
| `scripts/dev/build-snapshot.mjs` | 把 JS / CSS / 字体 / 图片合成单文件 |
| `scripts/dev/check-snapshot.mjs` | 无头浏览器逐屏截图与错误检查 |

## 覆盖面

主面板 10 个模块，以及样式重构最容易漏的二级界面：

- 灵动岛：完整设置页、经典样式窗口、胶囊样式窗口（双击可展开）
- 供应商：供应商连接 + 账号管理（配额条、余额、ZCode 日额度桶）
- 一键签到：卡片栅格 + 批量管理
- 用量统计：概览 / 用量分析 / 调用明细
- 会话线程：项目树 + 消息记录
- 自动化：任务列表 + 运行记录
- Skills 管理：列表（全局 / 项目 / 停用 / 同冲突 / 异常各一行）
- Agent：V8 三列工具卡（已接入 / 可接入 / 暂不支持）
- 模型路由：概览 / 路由流水 / 路由分组 / 模型池 / 对外访问 / 中间件
- 设置：常规 / 外观 / 网络与代理 / 数据与存储 / 通知 / 关于与更新

每屏都能切浅色 / 深色。

## 假数据约定

- 全是设计示例，**不对应任何真实账号、Key 或本机状态**；Key 只出现掩码。
- 时间以「快照打开的时刻」为基准推算，因此「3 分钟前」这类相对时间永远自然。
- 数据刻意覆盖多状态：已用满 / 未知 / 未提供 / 读取失败、连接过期、账号已暂停、Skill 同名冲突、路由接线漂移等，方便重构时确认每种状态都有样式。
- 交互是「就地改状态」的最小实现：开关、页签、签到按钮点了会有反馈，但不写任何持久化。

## 重构时的用法

1. 打开快照，切到要改的模块和主题，截图存档。
2. 改真实组件。
3. 重新执行 `pnpm snapshot`，用同一屏对比。
4. 需要重新对齐基线时，先 `git stash` 改动再生成一次快照。

## 注意事项

- **不要**把 `output/jolu-ui-snapshot.html` 提交进仓库，它已经在 `.gitignore` 里（`/output/`）。需要分享时直接发文件。
- 快照里未登记的命令会记进 `window.__SNAPSHOT_UNKNOWN__`，自检脚本会打印；新增页面或模块后先看这个清单，补上路由表，否则页面可能显示空态。
- 快照构建会读取 `frontend/public/` 下的字体与图标并内联。若新增了 public 资源且是绝对路径引用（`/xxx.png`），需要确认 `build-snapshot.mjs` 的 MIME 表覆盖了对应扩展名。
- `frontend/snapshot-dist/` 是中间产物，也已忽略。
