# magpie Agent 模块全链路调查（后端探测 → 写入 → 前端 UI 与动效）

日期：2026-11-13
调查对象：`D:\develop\_research\magpie`（commit `e9dd8e8`）
用途：给 Jolu 的 Agent 模块做实现参照。本文只描述 magpie 怎么做的，第 6 节才是 Jolu 的差距表。

---

## 1. 一句话结论

magpie 的 Agent 模块不是「一张配置表 + 几个开关」，而是三层：

1. **每个 agent 一个 Go 文件**（`internal/agent/*.go`，70 多个），各自声明它的配置路径、可调字段、每个字段怎么读写、菜单选项怎么来；
2. **统一的运行时判定**（`Detected` / `Wired` / `Drift`），把「装没装」「接没接」「接了但现在不对」分成三个正交概念，而不是一个状态枚举；
3. **前端只负责画**（`app.js` 一处 `renderAgents`），所有能点、能选、能改的东西都由后端 `state()` 一次性下发。

Jolu 目前是「5 个硬编码工具 + 一个 discovery 探测结果」，字段与菜单在前端拼装，缺的正是第 1、2 层。

---

## 2. 后端：Agent 描述符

### 2.1 一个 agent 就是一个结构体字面量

`internal/agent/agent.go:118` 起：

```go
type Agent struct {
	ID      string
	Name    string
	Icon    string        // 内置图标名，internal/gui/assets/icons
	Aliases []string
	Bin     string        // 可执行名，探测用
	Dir     string        // 配置目录，探测用
	Path    string        // magpie 实际改写的配置文件
	Plugin  string        // 由插件添加时为插件名，"" 表示内置
	Fields  []Field
	// ... 约 40 个可选 hook
}
```

`internal/agent/agents.go:149` 的 `onMachine()` 就是 70 多个构造函数的列表（`claude(home)`、`codex(home)`、`zed(home, cfg)` …）。**每个 agent 一个文件、一个构造函数**，不做表驱动 —— 因为它们的配置格式（JSON / TOML / YAML / env / CLI 命令）和写入约束差异太大，表驱动会退化成 `if agent == "xxx"` 的泥潭。

### 2.2 字段是「活的」，不是静态配置项

`agent.go:82` 的 `Field`：

```go
type Field struct {
	Key     string
	Label   string
	Get     func() string                          // 现读现值
	Set     func(string) error                     // 写进去（空值 = 恢复默认）
	Options func(cur map[string]string) []Option   // 此刻能选什么
	Quiet   bool   // 空值时不列出（跟随别的字段，如 Claude Code 的分档模型）
	Live    bool   // magpie 自己的设置，改了不用重启 agent
	Follows string // Quiet 字段跟随哪个字段（"model"）
}
```

关键点：**`Get` 和 `Options` 是函数，每次读都重新算**。所以：

- 模型菜单里的候选取决于「当前有没有接网关」「用的是哪个账号」「models.dev 有没有这个模型的上下文」；
- 字段值永远反映磁盘上的真实状态，包括用户在 CLI 里自己改的。

### 2.3 选项携带的信息量

`agent.go:30` 的 `Option` 有 25 个字段，前端菜单几乎全靠它渲染：

| 字段 | 含义 | 前端表现 |
|---|---|---|
| `Value` / `Label` | 值 / 显示名 | 菜单行文字 |
| `Group` / `GroupIcon` | 分组标题（provider 名） | picker 左侧 rail |
| `Icon` / `Icons` | 单 logo / 叠加 logo 栈 | 行首图标 |
| `Ref` | 目录里的模型 id（各 agent 通用） | 判断「是不是 magpie 的模型」 |
| `Note` | 说明 | tooltip |
| `Free` | 这个账号不花钱 | 绿色小徽标 |
| `Rate` / `RateWas` | 计费倍数 / 折扣前 | 角标（Qoder 0.5×、WorkBuddy ×0.03） |
| `Context` | 上下文长度 | 大窗口标记 |
| `Direct` | 走它自己的登录，不经网关 | tooltip「Anthropic」 |
| `Via` | 是 agent 自己的模型，但经网关 | 菜单里标「via magpie」 |
| `Same` | 与用户自己登录的那个是同一账号 | 折叠成一行（Claude Code #496） |
| `Served` | magpie 列表也展示 agent 自己的模型 | 不当作「外部模型」隐藏 |
| `Alias` | 日期版 id 的别名（`claude-opus-4-5-20251101` → `claude-opus-4-5`） | 两行合并 |
| `FastFor` / `Fast` / `UltraOK` / `Ultra` | 快速模式 / Ultrafast | 闪电图标（单击切换） |
| `Takes` | 该模型不带档位时的默认档位 | 「default」旁的小字 |

Jolu 的 `AgentDetail` 目前只有「模型 + 档位」两列，magpie 是这一整套。

---

## 3. 后端：三个正交判定

magpie 不维护 `state: "installed" | "wired" | ...` 这种单枚举，而是三个独立函数：

### 3.1 `Detected()` —— 装没装（`agent.go:371`）

按顺序，任一命中即为真：

```go
if a.detect != nil { return a.detect() }                        // 自定义探测（WSL 里探到的）
if a.Dir != "" && Taken(a.Dir) { return false }                 // 目录被别的工具占了（同名文件），排除
if os.Stat(a.Path) 成功 && (a.pathOwn == nil || a.pathOwn())    // 配置文件在，且确实是它的
if a.Dir != "" && !a.dirShared && agentDir(a.Dir)               // 配置目录在，且是 agent 自己建的
if a.sign != nil && a.sign()                                    // 自定义特征（如某文件存在）
if a.Bin != "" && binAt(a.Bin) != ""                            // PATH 里有命令
if a.Bin == "" && a.WSL == "" && brewBin(cliSpecs[a.ID]) != ""  // 没有唯一命令名的，看 Homebrew
```

两个细节值得抄：

- **`Taken(dir)`**：`~/.dsh` 如果是某个 shell 的普通文件而不是目录，就说明这是别的工具的地盘，agent 不可能在这儿（`agent.go:373` 附近的注释）。
- **`agentDir(dir)`**（`agent.go:412` 起）：判断一个目录是不是 agent 自己建的，避免「用户有个同名空目录」被误判成已安装。

`binAt()`（`agent.go:400`）在 Windows 上特别处理：

```go
func binAt(name string) string {
	if p, err := proc.LookPath(name); err == nil { return p }
	if runtime.GOOS == "windows" { return proc.FindTool(name) }
	return ""
}
```

注释写得很实在：Windows 没有登录 shell 可以借 PATH，所以 magpie 启动早于 Claude Code 安装时会永远找不到 `claude.exe`（#839、#1482）。`proc.FindTool` 会查注册表的 PATH、`~/.local/bin`、npm 全局、bun、Volta、pnpm。

**这条 Jolu 已经做了**（`agent_discovery.rs` 的 `registry_path_dirs` / `user_bin_dirs`）。

### 3.2 `Wired()` —— 接没接（`applied.go:415`）

```go
func (a *Agent) Wired() bool {
	if a.Native != nil { return a.Native.Read().Provider == "connected" }
	if a.Joined != nil && a.Joined() || a.Beside != nil && a.Beside() { return true }
	vals := a.Values()
	for _, f := range a.Fields {
		if v := vals[f.Key]; v == magpieID || magpieValue(a, f, v, vals) || ... {
			return true
		}
	}
	return false
}
```

判定依据是**配置里的值**，不是 magpie 自己的记录。所以用户手工把模型改回 magpie 的某个模型，`Wired()` 立刻变真（`Apply` 里也会 `forget(disconnectedKey)`）。

### 3.3 `Drift()` —— 接了但现在不对（`applied.go:162`）

这是 magpie 最值得抄的一块。`Drift` 有 5 种 `Kind`：

| Kind | 含义 | 判据 |
|---|---|---|
| `unwired` | 模型是 magpie 的，但配置不再指向网关 | `Check()` hook 说某处接线不见了 |
| `replaced` | magpie 设的模型被换成了 agent 自己的 | 与 `applied.json` 记录比对，且新值不是 magpie 的模型 |
| `bypassed` | 配置是对的，但它启动早于 magpie 的设置，跑的是旧配置 | `LastUsed()` 时间 > applied 时间，但网关没收到对应请求 |
| `unreachable` | 配置对，但非 loopback 地址的网关不通 | 实际发请求探测 |
| `proxied` | 配置对，但请求被系统代理截走 | 检查代理设置 |

`bypassed` 的判据（`applied.go:275`）：

```go
func bypassed(used, applied, seen time.Time) bool {
	const grace = 30 * time.Second
	return !used.IsZero() && used.After(started) && used.After(applied) &&
		time.Since(used) > grace && seen.Before(used.Add(-2*time.Second))
}
```

翻译：agent 在 magpie 启动之后用过（`used > started`）、在 magpie 写配置之后用过（`used > applied`）、距今超过 30 秒、**但网关在那一前一后根本没收到请求**（`seen < used - 2s`）。结论就是「它跑的是旧配置，重启一下」。

`LastUsed` 怎么来？`lastJSONLTime()`（`applied.go:867`）读 agent 自己的 prompt 日志（JSONL），取最后一条**不是斜杠命令**的时间戳。这就是「知道你在用 Claude Code、什么时候用的」的实现方式之一。

### 3.4 `applied.json` —— 记账

`applied.go:33`：

```go
type applied struct {
	Fields map[string]string   // 每个字段 magpie 写进去的值
	At     time.Time           // 写入时间
}
```

存在 `applied.json`（与 provider 配置同目录）。

作用有三个：① drift 的 `replaced` 判据；② 区分「用户自己改的」和「magpie 改的」；③ `Reapply()` 时知道该写回什么。

**重点**：magpie 不靠「记录说已接入」，而是「记录 + 现场读配置」两边比。这是它比 Jolu 现状可靠的根本原因。

---

## 4. 后端：连接、写入、断开

### 4.1 `Pick` vs `Connect` —— 两个入口，一个效果

`Pick(key, v)`（`applied.go:436`）：从页面菜单选一个模型。如果还没接入且选的是 magpie 的模型，**先自动 `connect()` 再 `Apply`**。

`Connect()`（`applied.go:485`）→ `ConnectHow()` 返回一个 `Connection` 说明「这次是怎么选的模型」：

| `How` | 场景 |
|---|---|
| `kept` | 本来就接着，不动 |
| `again` | 把断开前的模型放回去 |
| `joined` | 保持它自己的模型，magpie 的模型并入它的列表（Codex 登录 ChatGPT 时） |
| `magpie` | 应用只有一个设置就是「magpie 当它 provider」 |
| `same` | 它现在用的模型，经网关（同一个账号） |
| `alike` | 别名指向的模型（`opus`） |
| `default` | 它自己没设模型，取列表里第一个 |
| `first` | magpie 没有它正在用的模型，取该账号第一个 |

这个 `How` 会下发给前端，用于告诉用户「为什么给你选了这个」（#726）。

### 4.2 写入是幂等的，因为写入前会备份

`Unwire` hook：把 magpie 写进去的东西撤掉，**并把用户原来的 endpoint / provider / model 还回去**（存在 stash 里）。

`Restore` hook：magpie 第一次写之前会把整个文件拷到一边（`beforeMagpie`），断开时如果 stash 说一致，就整体还原。

`stash` 是个 `map[string]string`，键形如 `{agentID}.connect.was`、`{agentID}.claude.login`、`{agentID}.tier_own.opus` —— 记录「这个字段当初是什么」「是不是 magpie 写的」「用户有没有覆盖过」。

### 4.3 三个特殊语义

从 `Field` 的定义直接读出来：

- **`Set("")` = 恢复 agent 自己的默认**：magpie 写进去的 provider、catalog 全部撤掉，键删掉。不是「设成空字符串」。
- **`Quiet`**：空值时不列出。Claude Code 的分档模型（opus/sonnet/haiku）默认跟随主模型，用户没单独设就不该占一行。
- **`Follows`**：`Quiet` 字段跟随哪个字段。`Drift` 判 `replaced` 时会把这个字段解析成它跟随的那一个（避免「主模型动了、档位跟着动」被误判成被篡改，#1050）。

---

## 5. 前端

### 5.1 技术栈

纯手写，没有框架、没有构建：

- `internal/gui/assets/index.html`（522 行，骨架）
- `assets/app.js`（1.3 MB，全部逻辑）
- `assets/app.css`（360 KB，全部样式）
- 通过 `go:embed` 打进二进制

好处是零构建、`index.html` 里 `data-t` 属性挂 i18n 键。Jolu 用 React 不需要照抄，但**「所有 UI 状态由后端一次下发」**这一点该抄。

### 5.2 列表行的 DOM

`app.js:433` 的 `agentRow`：

```html
<div class="row agent" data-id="codex">
  <div class="who">
    <div class="name">Codex</div>   <!-- + drift 药丸、CLI 版本药丸 -->
  </div>
  <div class="fields">              <!-- 两列网格，宽 350px -->
    <button class="field main">…</button>   <!-- 模型 -->
    <button class="field side">…</button>   <!-- 档位 -->
  </div>
  <div class="ag-open">…</div>      <!-- 展开的详情，grid-template-rows 0fr→1fr -->
</div>
```

CSS（`app.css:783`）：

```css
.fields {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 124px;
  gap: 5px;
  width: 350px;
  flex: none;
  margin-left: auto;
}
```

**两列固定宽度**是为了「每一行的 chevron 竖向对齐」。面板态（`body.panel`）缩成 272px / 118px。

窄窗（`max-width: 600px`）时整行改成两行网格：

```css
@media (max-width: 600px) {
  body.window .row.agent { display: grid; grid-template-columns: 28px minmax(0, 1fr); row-gap: 3px; }
  body.window .row.agent > .fields { grid-column: 2; grid-row: 2; width: 100%; }
}
```

### 5.3 展开行（详情）—— 就是原地展开

不是抽屉、不是弹窗，是**同一个 row 内部的高度展开**：

```css
.ag-open { display: grid; grid-template-rows: 0fr; visibility: hidden;
           transition: grid-template-rows .4s var(--row-spring), visibility 0s .4s; }
.row.agent.open .ag-open { grid-template-rows: 1fr; visibility: visible;
           transition: grid-template-rows .48s var(--row-spring); }
.ag-body { opacity: 0; transform: translateY(-6px);
           transition: opacity .16s ease, transform .4s var(--row-spring); }
.row.agent.open .ag-body { opacity: 1; transform: none;
           transition: opacity .3s ease .06s, transform .48s var(--row-spring); }
```

`--row-spring: cubic-bezier(.25, .3, .1, 1)`。

关键手法：**`grid-template-rows: 0fr → 1fr`**。这比 `max-height` 好，因为不需要猜高度，浏览器自己算；而且对这个属性的 transition 是逐帧插值的。

关闭时反过来 `.4s`，且 `visibility` 延迟 `.4s` 才变 hidden（让内容先消失再收高度）。内容体 `translateY(-6px)` 下移进位的细节：开的时候「先透明后归位」，关的时候「先上移再消失」。

### 5.4 折叠组（没有设置过的 agent）—— 逐行错峰

最后一行「Show N more」展开后：

```css
.agent-fold { display: grid; grid-template-rows: 0fr;
              transition: grid-template-rows .42s cubic-bezier(.4, 0, .2, 1); }
.agent-fold.open { grid-template-rows: 1fr;
              transition-duration: .62s; transition-timing-function: cubic-bezier(.22, 1, .36, 1); }

/* 关闭：从下往上收，每行间隔 22ms */
.agent-fold .row { opacity: 0; transform: translateY(-10px) scale(.985);
                   transform-origin: 50% 0;
                   transition: opacity .22s ease, transform .32s cubic-bezier(.4, 0, .2, 1);
                   transition-delay: calc((var(--n) - 1 - var(--i)) * 22ms); }

/* 打开：从上往下落，先等 70ms，每行间隔 48ms */
.agent-fold.open .row { opacity: 1; transform: none;
                   transition: opacity .42s ease, transform .56s cubic-bezier(.22, 1, .36, 1);
                   transition-delay: calc(70ms + var(--i) * 48ms); }
```

**打开慢（62s 容器 + 48ms 错峰）、关闭快（42s 容器 + 22ms 反向错峰）** —— 这是「展开要仪式感、收起要利落」的通用做法。

`--n` 是总行数，`--i` 是当前行索引，所以关闭时 `(n-1-i)` 让**最后一行先收**。

展开过程中边缘做软化：

```css
.agent-fold.moving .agent-fold-inner {
  mask-image: linear-gradient(#000 calc(100% - 32px), transparent);
}
```

### 5.5 模型 picker（弹出层）

`app.css:2097`：

```css
.pop {
  position: fixed; z-index: 30;
  width: 320px; max-height: 340px;
  background: var(--pop-bg);
  backdrop-filter: blur(24px);
  border-radius: 11px;
  transform-origin: var(--ox, 24px) top;
  animation: pop-fade .18s ease-out, pop-in .42s cubic-bezier(.16, 1, .3, 1);
}
@keyframes pop-in { from { transform: translateY(var(--pop-dy, -6px)) scale(.94); } }

/* 关闭：沉回按钮方向，先淡后缩 */
.pop.leaving { animation: pop-out .22s cubic-bezier(.4, 0, .7, .2) forwards; }
@keyframes pop-out { to { opacity: 0; transform: translateY(calc(var(--pop-dy, -6px) * .6)) scale(.96); } }
```

要点：

- `transform-origin: var(--ox) top` —— **从按下的那个按钮中间长出来**（`--ox` 由 JS 按按钮位置算）；
- `blur(24px)` 半透明背景；
- 打开 `.42s cubic-bezier(.16, 1, .3, 1)`（长距离软着陆，iOS 菜单手感），关闭 `.22s cubic-bezier(.4, 0, .7, .2)`；
- 关闭时**只移动到 `--pop-dy * 0.6`**、缩到 `.96`（不是缩回原点）—— 因为已经淡出了，缩太狠会「啪」一下。

左侧 provider rail：

```css
.rail-thumb { width: 31px; height: 31px; border-radius: 8px;
              transition: transform 260ms cubic-bezier(.22, 1, .36, 1), opacity 100ms ease-out; }
```

**滑块是一个绝对定位的 div，滚动时靠 `transform` 跟随选中项**（不是给每个 item 加背景）。这样才有「滑块滑过去」的效果。

### 5.6 行内 hover 才出现的 logo

`app.css:1747`：

```css
.ag-sum .mi { width: 0; overflow: hidden;
              transition: width .36s var(--row-spring), margin-right .36s var(--row-spring); }
.row.agent:hover .ag-sum .mi { width: calc(var(--w, 16px) + 4px); margin-right: 4px;
              transition: width .46s var(--row-spring), margin-right .46s var(--row-spring); }
.ag-sum .mi-in { opacity: 0; transform: scale(.6); transform-origin: 100% 50%;
              transition: opacity .14s ease, transform .3s var(--row-spring); }
.row.agent:hover .ag-sum .mi-in { opacity: 1; transform: none;
              transition: opacity .28s ease .08s, transform .52s var(--row-spring) .04s; }
```

`--w` 是 JS 在 `pointerenter` 时读 `offsetWidth` 写进去的 —— 因为进入 hover 时 `width: 0` 量不到宽度。注释直接说明了原因。

### 5.7 状态药丸

| 药丸 | CSS | 用途 |
|---|---|---|
| `.ag-fix` | `background: var(--drift-soft); color: var(--drift)`，coral 色 | drift 时「重新设置」 |
| `.ag-fix.ag-restart` | 改成 `--amber` 弱势 | 需要重启桌面应用 |
| `.ag-up` | 卡片色 + 边框 | CLI 有新版本 |
| `.ag-show` | 同上 | 被手动隐藏的行「显示回来」 |

`.ag-fix:active { transform: scale(.96); }` —— 50ms 的按下反馈。

### 5.8 拖拽排序

```css
#agents.sorting > .row { transition: transform .22s cubic-bezier(.22, 1, .36, 1); }
#agents > .row.dragging {
  z-index: 2; background: var(--card);
  box-shadow: 0 6px 18px rgba(20,22,30,.12), 0 0 0 1px var(--line);
  border-radius: 8px; transition: none;
}
```

拖动时其他行 `transform` 让位（`.22s`），被拖的行 `transition: none` 跟手。

手柄是 logo 本身（`.ag-handle`），左侧 margin 里画一个 6×11px 的点阵 grip，**只在 hover / dragging / focus 时出现**（注释：「一直出现太丑了」）。

### 5.9 排序与分组逻辑

`app.js:2713`：

```js
function arrangeAgents() {
  const order = s.agentOrder || [], hidden = new Set(hiddenIds());
  const rank = (a) => { const i = order.indexOf(a.id); return i < 0 ? order.length : i; };
  const all = state.agents
    .map((a, i) => [a, i])
    .sort(([x, i], [y, j]) => rank(x) - rank(y) || i - j)   // 手动顺序优先，其余保持后端顺序
    .map(([a]) => a);
  const counts = all.some(connectable) ? onMagpie : agentUsed;
  const anyUsed = all.some((a) => !hidden.has(a.id) && counts(a));
  const inView = (a) => !hidden.has(a.id) && (!anyUsed || agentUsed(a));
  return { all, shown: all.filter(inView), folded: all.filter((a) => !inView(a)) };
}
```

**分组不是按「已接入/可接入/不能接入」，而是按「有没有在用」**：只要有一个 agent 在 magpie 上，其余没设置的就被折进「Show N more」。Jolu 现在按三种状态分组，是另一套语义（画板 V5-1 要求），不冲突。

### 5.10 无障碍与降级

```css
@media (prefers-reduced-motion: reduce) {
  .pop { animation: pop-fade .12s ease-out; }
  .ag-open, .ag-body, .eslide .knob, .eslide .fill { transition: none; }
}
```

每个动效块都配了 reduced-motion 覆盖。Jolu 已在设置里有「减少动效」开关，可对应。

---

## 6. Jolu 现状与差距

### 6.1 逐项对照

| 能力 | magpie 实现 | Jolu 现状 | 差距 |
|---|---|---|---|
| **Agent 广度** | 70+ 个文件，每个独立声明 | `SPECS: [Spec; 13]`（`agent_discovery.rs`） | 少 57 个，但按既定取舍不追平 |
| **探测信号** | 配置文件 → 配置目录 → 特征 → PATH → Homebrew，五级 | `classify(file_found, dir_found, bin_found)` 三级 | 缺 `Taken(dir)` 排除与 `agentDir()` 自建判定 |
| **Windows PATH** | `proc.FindTool`：注册表 + npm + bun + Volta + pnpm + `~/.local/bin` | `registry_path_dirs` + `user_bin_dirs` 已实现 | ✅ 基本对齐 |
| **字段模型** | `Field{Get/Set/Options}` 函数式，7 种语义标记 | 前端 `agentRow` 拼状态，字段无后端声明 | ❌ 最大缺口 |
| **选项元数据** | `Option` 25 个字段 | `AgentDetail` 只有 model / effort 两列 | ❌ |
| **装没装** | `Detected()` | `discovery.state` | 概念等价，粒度较粗 |
| **接没接** | `Wired()` 现场读配置 | `wiring.wired` | 概念等价 |
| **接了但不对** | `Drift()` 5 种 Kind | `driftText()` 6 种（changed / removed / foreign / pending / unreachable / model_changed） | 缺 `bypassed`（最实用的一种） |
| **记账** | `applied.json`：每字段写入值 + 时间 | `wiring_store::Applied`（`wiring_store.rs:31`）：base / token / provider / mode / model / label / subagent_model / tiers / extra_env 等 | ✅ **已有**，且字段语义比 magpie 更细（区分「我们写的」与「用户原值」） |
| **写入前预览** | `GET /api/agents/preview/{id}` + revision 校验 | `gateway_wiring_preview` + `wiring_diff.rs` | ✅ 有 |
| **断开还原** | `Unwire` + `Restore` + stash | `wiring::unwire`（`wiring.rs:1559`）+ 快照备份（`commit_plan`）+ `escape.rs` 的外部接管检测 | ✅ **已有**，且多了「外部值接管就跳过」的保护 |
| **模型菜单来源** | `models.dev` 目录 + 各账号可用 + agent 自有模型，按 provider 分组 | `gateway` 路由对象 + 硬编码 | 缺 provider 分组与元数据 |
| **「知道用什么模型」** | 三路：① 读配置现值 ② 读 prompt 日志 `LastUsed` ③ 网关请求 `Reached` | 只读配置 | 缺 ②③ |
| **列表分组** | 按「有没有在用」，其余折叠 | 按「已接入/可接入/不能接入」 | 语义不同，按画板保留 |
| **行内字段** | 两列固定宽 chip + 第三列小方块 | `AgentDetail` 面板里 | ❌ 形态差异 |
| **详情展开** | 原地 `grid-template-rows 0fr→1fr`，.48s `.25,.3,.1,1` | 右侧独立面板（画板要求） | 按画板保留 |
| **折叠错峰** | 开 70ms + 48ms×i，关反向 22ms×i | 无 | ❌ |
| **picker 动效** | 从按钮长出，`.42s cubic-bezier(.16,1,.3,1)` | 无确认 | ❌ |
| **rail 滑块** | 绝对定位 div + `transform` 260ms | 无 | ❌ |
| **hover 出 logo** | `width 0 → var(--w)`，JS 量宽 | 无 | ❌ |
| **拖拽排序** | logo 当手柄 + margin 点阵 grip | 无（但 `agentOrder` 概念 Jolu 有 hidden 列表） | ❌ |
| **reduced-motion** | 每处都配 | 设置里有全局开关 | 需逐处接 |

### 6.2 结论：不是「全部误判」，是分工不同

先分清三件事：

| 判断 | 是不是误判 |
|---|---|
| 「列表只有 5 个 agent」 | **是实现缺陷**，已修（`AgentsView` 改为遍历探测结果，13 家全显示） |
| 「Agent 功能和 magpie 差得远」 | **不是误判**，是真实缺口，见下表 |
| 「第三轮改坏了 ZCode」 | **是误判**：ZCode 问题是官方 3.18 升级换了凭据封装，与本轮无关 |

已核实 Jolu 在**接线正确性**上并不落后 —— `Applied` 记账、`unwire` 还原、快照备份、外部接管检测、写入预览全都有，某些地方（外部值接管保护）比 magpie 更谨慎。

真正缺的是**表达层**：

- **字段模型没有后端声明**（magpie 的 `Field{Get/Set/Options}`）。Jolu 的字段、档位、菜单项都在前端拼，所以：
  - 加一个工具要改前端代码，magpie 只加一个 Go 文件；
  - 菜单里的模型没有元数据（归属 provider、是否免费、上下文窗口、是否走自己的登录）；
- **没有 `bypassed` 检测**：配置写对了，但 agent 跑的是启动时的旧配置 —— 这是用户最常踩、最难自诊断的坑。
- **动效缺失**（纯视觉，不影响正确性）。

### 6.3 建议的落地顺序

| 阶段 | 内容 | 价值 |
|---|---|---|
| 1 | 后端引入 `Field` 抽象（先覆盖 5 个已支持工具的二三十个字段） | 解开「加工具要改前端」的死结 |
| 2 | `Option` 元数据（`Ref` / `Group` / `Icon` / `Free` / `Context` / `Direct`） | 模型菜单立刻有分组和图标 |
| 3 | `bypassed` 检测（配置对但请求没到网关） | 「接了但没生效」能诊断 |
| 4 | 前端动效（折叠错峰、picker 生长、rail 滑块） | 观感；不影响正确性 |
| 5 | 拖拽排序 + 隐藏组 | 列表可用性 |

第 1、2、3 阶段是功能性的，第 4 阶段是纯视觉 —— 按 AGENTS.md「界面按 Jolu.pen 重做」，第 4 阶段必须以画板为准，magpie 只作手法参考。

---

## 7. 参考位置速查

| 想查什么 | 位置 |
|---|---|
| Agent 结构体与 Field / Option | `internal/agent/agent.go:30`（Option）、`:82`（Field）、`:118`（Agent） |
| 70 个 agent 的注册表 | `internal/agent/agents.go:149` `onMachine()` |
| 装没装的判定 | `internal/agent/agent.go:371` `Detected()`、`:400` `binAt()`、`:412` `agentDir()` |
| 接没接 | `internal/agent/applied.go:415` `Wired()` |
| 接了但不对（5 种 drift） | `internal/agent/applied.go:129` `Drift` 结构、`:162` `Drift()` |
| 记账 | `internal/agent/applied.go:33` `applied`、`:38` `appliedPath()` |
| 选模型 / 连接 | `internal/agent/applied.go:436` `Pick`、`:485` `Connect`、`:516` `ConnectHow`、`:509` `Connection` |
| 断开与还原 | `internal/agent/applied.go:724` `Disconnect`、`:794` `DisconnectOffline` |
| 「知道你在用什么模型」 | `internal/agent/applied.go:867` `lastJSONLTime()`、`:275` `bypassed()` |
| Claude Code 全字段 | `internal/agent/claude.go:1647` 起（model / effort / ultracode / 三档位） |
| 模型菜单构造 | `internal/agent/claude.go:2385` `claudeViaMagpie`、`:2406` `claude1M` |
| 前端行渲染 | `internal/gui/assets/app.js:433` `agentRow`（在 `renderAgents:414` 内） |
| 排序分组 | `internal/gui/assets/app.js:2713` `arrangeAgents` |
| 详情展开动效 | `internal/gui/assets/app.css:1771` `.ag-open` / `.ag-body` |
| 折叠错峰 | `internal/gui/assets/app.css:596` `.agent-fold` |
| picker 动效 | `internal/gui/assets/app.css:2097` `.pop` |
| rail 滑块 | `internal/gui/assets/app.css:2127` `.rail-thumb` |
| 行内字段布局 | `internal/gui/assets/app.css:783` `.fields` |
| drift 药丸 | `internal/gui/assets/app.css:641` `.ag-fix` |
| 拖拽排序 | `internal/gui/assets/app.css:748` `#agents.sorting` |
| 后端下发结构 | `internal/gui/api.go:184` `agentJSON` |
| 模型清单接口 | `internal/gui/agent_models.go` |
| 预览 / 连接 / 断开接口 | `internal/gui/api.go:1045`（preview）、`:1106`（`{action}/{id}`） |
