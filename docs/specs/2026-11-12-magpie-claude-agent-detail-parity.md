# magpie Agent 详情面板拆解与 Jolu 对齐清单

**日期：** 2026-11-12
**来源：** 本地 `D:\develop\_research\magpie`（commit `e9dd8e8`），主要文件 `internal/agent/claude.go`、`internal/gui/assets/app.js`、`internal/gui/assets/index.html`
**目的：** 说明 magpie「接入 agent → 展开详情」这块界面的设计与实现，并逐项对比 Jolu 现状，标出可直接照做与需要决策的部分。§8 覆盖 **agent 自动发现**，§9 是**定位与功能取舍**（Jolu 是 AI 工具管家）。
**前提：** 本文只做拆解与差距记录，不预设 Jolu 必须全部照搬。magpie 的界面语言、布局密度、交互习惯与新稿 `Jolu.pen` 不一致，**界面部分必须在画板确认后再做**（见 §5「需要鼠鼠决策」）。

**章节索引：**

| 章节 | 内容 |
|---|---|
| §1–2 | 详情面板是什么、逐块拆解（档位 / 模型列表 / 新会话 / 长对话 / diff） |
| §3 | Claude Code 的字段与写入（magpie 实现细节） |
| §4 | **B1–B8 后端缺口、F1–F6 前端缺口**（逐项核对过，均未实现）；§4.2.1 细化 B8「怎么知道你在用什么模型」 |
| §5 | 需要鼠鼠决策的 7 点（D1–D7） |
| §6 | 建议执行顺序 |
| §7 | 参考位置速查 |
| §8 | **A 段：agent 自动发现**（A1 / A2 要做，A3 与插件化不做） |
| §9 | **定位与功能取舍**：Jolu 是 AI 工具管家；agent 广度与协议互转要做，插件市场先不做 |
| §10 | **完成状态总览**（接手先读）：已实现带验证的项、未完成清单与依赖、遗留问题、决策状态 |

---

## 1. 这块界面是什么

magpie 的 Agents 页是一张列表，每行一个检测到的 agent（Claude Code、Codex、OpenCode…），右侧一个接入开关。点开某一行（或它已接入时点「展开」）后，在原位展开一块详情面板。

面板不是一张表单，而是**三类性质不同的东西拼在一起**：

| 区域 | 性质 | 存放位置 |
|---|---|---|
| 档位区（Opus / Sonnet / Haiku / Fable / 子代理 / 自动模式） | agent 自己的设置 | 直接写进该 agent 的配置文件 |
| 模型列表（「路由组 1」「ZCode 3」「挑选」） | magpie 要给这个 agent 看的模型菜单 | magpie 自己的库，删选/排序 |
| 新会话默认、长对话 | 全局偏好 | magpie 设置，影响网关声明的窗口与起点模型 |

设计意图：把「这个 agent 能调的东西」和「影响这个 agent 的全局设置」放进同一个抽屉，用户不用在两个页面之间跳。

---

## 2. 逐块拆解

### 2.1 档位区（`app.js` 的 `connectPanel`，约 2028–2078 行）

六行，每行三段：

```
档位        [模型下拉 ▾]      [effort 下拉 ▾]   /model opus
Opus       同主模型           默认              /model opus
Sonnet     同主模型           默认              /model sonnet · 默认档位
Haiku      同主模型           默认              /model haiku · 也用于后台小任务
Fable      同主模型           默认              /model fable
子代理      由 Claude Code 自己挑选  默认           —
自动模式    默认              —                自动模式下检查每个操作的模型
```

**第一段：这一档用哪个模型。** 选项第一项恒为「同主模型」，即该档不写、跟着主模型走。magpie 在 stash 里记一个 `claude.tier_own.<tier>` 标记表示「这一档是用户自选的」，换主模型时不重算；只有仍为「同主模型」的档位才跟随。

**第二段：这一档的推理强度（effort）。** 只列该模型在 Claude Code 里真收的档位；默认空 = 交给 Claude Code。子代理那一行的 effort **在没设子代理模型时是禁用的**（`app.js` 的 `effortBtn(e, !sub.value)`），因为写了 effort 不写模型会产出无效组合。

**第三段：灰字提示「在 Claude Code 里怎么做」。** 这是全篇最值得抄的一处：

- `/model opus` 之类的命令映射，让用户知道在 agent 内部怎么复现同样操作
- `Sonnet · 默认档位`：告诉用户这是没设时的默认档
- `Haiku · 也用于后台小任务`：**重要警告**。Claude Code 把会话标题、Explore 子代理、WebFetch 阅读这类高频小请求走 haiku 档；这一档被设成贵模型会显著加快额度消耗（magpie 代码注释引用了一位用户的实测，走网关时额度掉得比原生快）

**自动模式行**：auto 模式下每个操作前先跑一个分类器。这里可指定分类器用哪个模型；不设 = Claude Code 自己挑。magpie 会过滤掉不支持分类器的模型（`claudeNoClassifier`，如 `claude-sonnet-5-5`、`claude-opus-5-5` 等），因为给它们设了分类器 Claude Code 会退回自己的选择。

**「Claude 模型」那一行**：检测主模型选项里有没有 `claude/*` 来源。没有时显示「claude-* 模型需要先在 magpie 里添加 Claude 订阅」+ 一个「添加 Claude 订阅」按钮。这是把「缺依赖」变成可点击的引导，不是一句干巴巴的报错。

### 2.2 模型列表（`app.js` 的 `openAgentModels`，约 3216 行起）

```
模型列表   [☑ 路由组 1]  [☑ ZCode 3]  [挑选 ›]
```

- 分组芯片按来源统计（路由组几个、某个来源几个），显示图标 + 名字 + 数量
- 超过一定数量显示「N 个已隐藏」
- 点「挑选」弹出清单，两件事：
  1. **打勾决定这个 agent 的菜单里出现哪些模型**。语义是「默认新模型是关的，要手动开」（`only` 模式），不是「默认全开、删掉不要的」
  2. **拖动排序**，顺序就是 agent 菜单里的顺序（`ordered`）
- 弹窗里明确写：`Models turned off don't show in {agent}'s model picker; other agents can still use them`——裁剪是按 agent 的，不影响其他 agent

解决的实际问题：接入模型多了以后 Claude Code 的 `/model` 列表会膨胀到上百项，没法找。这是按 agent 单独裁剪。

### 2.3 新会话默认（`app.js` 的 `connectPanel` 末段）

`推理强度 [默认 ▾] [▷] [👤]`

- 一个模型 + 一个 effort，决定「打开 agent 后新会话从哪开始」
- **不填 = 用 agent 自己的上次选择**（尊重用户在 agent 内部的 `/model` 操作）
- 落点是各 agent 的「新会话默认」字段，Claude Code 走的是 `model` + effort 的一套写入

### 2.4 长对话（`index.html` 的 `fullContextRow` + `app.js` 的 `renderAgentPrefs`）

```
长对话   Codex 和 Claude Code 在 272K(非 Claude 的 [1m] 模型也是)或这里填的大小压缩，完整窗口每轮更慢更贵
         [272K 时压缩]  [完整窗口]  [272k]
```

三态：

- **默认（`compactAt` = 272000）**：上下文到 272K 时压缩
- **完整窗口（`fullContext`）**：不压缩，用满模型声明的最大窗口——UI 上直接写「更慢、更贵」
- **自定义数值**：`parseContexts` 解析 `272k` / `1M` / `400000` 这类写法

为什么默认 272K 而不是模型上限：超长上下文有首字延迟和阶梯定价两个代价。magpie 代码注释引用的实测是 550K 时首 token 要 70–90 秒；OpenAI 一类厂商超过 272K 就切更高价档。所以默认在性价比拐点压缩。

关键实现：这个设置**通过 `settings/full-context` 写入，影响的是网关对模型声明的窗口**（`CLAUDE_CODE_AUTO_COMPACT_WINDOW` 与窗口声明），不是直接改 agent 的配置。所以在另一台机器上运行的 agent 也能读到。

### 2.5 底部：改动可见与还原（`app.js` 的 `connectPanel` 末段）

```
改动了 ~/.claude/settings.json  查看 ›                   断开并还原
```

- **「改动了 X」列出 magpie 实际写过的文件**（不是猜的，是写入时记录的 `previews`）
- **「查看」展开逐行 diff**：`- 旧值` / `+ 新值`，能看清改了哪几个键
- **「断开并还原」**：只撤掉 magpie 自己写的东西，用户原有设置原样留下

`agents/preview/{id}` 是后端接口，返回 `changes`（文件 + 行级 diff）与 `revision`。断开前还会先拉一次「断开会发生什么」的预览。

建立信任的核心：用户能看清一个工具到底动了自己哪个文件的哪几个键，并且有一键撤回。

---

## 3. Claude Code 的字段与写入（`internal/agent/claude.go`）

### 3.1 七个字段

| 字段 | 存储位置 | 要点 |
|---|---|---|
| `model`（主模型） | `settings.json` 顶层 `model` | **刻意不写 `ANTHROPIC_MODEL`**——写了会压过 `/model`，用户自己的切换失效 |
| `effort` | `modelSettings.<model>.effortLevel`、顶层 `effortLevel`、`env.CLAUDE_CODE_EFFORT_LEVEL` | 三处分层：`max` 只进 env（只持续一个会话）；Opus 5.5+ 只读 modelSettings；顶层同时写是为了用户在 agent 里换到别家模型时也生效 |
| `ultracode` | 顶层 `ultracode` | 只对收 `xhigh` 的模型显示 |
| `opus` / `sonnet` / `haiku` / `fable` | `env.ANTHROPIC_DEFAULT_<TIER>_MODEL` | 空 = 跟随主模型；有值 = 自选，换主模型不重算 |
| `subagent` | `env.CLAUDE_CODE_SUBAGENT_MODEL` | 同样的自选语义；空值时 effort 单独暂存不写 |
| `auto_mode` | `env.CLAUDE_CODE_AUTO_MODE_MODEL` | 过滤掉不支持分类器的模型 |
| `login` | `env.ANTHROPIC_AUTH_TOKEN` | 可选「保留 claude.ai 登录」（写空串）——写 key 会把它登出 |

### 3.2 `writeTiers`（`claude.go` 约 1534–1645 行）一次写入维护的派生项

接入时除主模型与四档外，还会连带算并写：

1. **`ANTHROPIC_SMALL_FAST_MODEL`** ← haiku 档的值
2. **空档显式 `DelJSON`**（不留脏值）
3. **`CLAUDE_CODE_MAX_CONTEXT_TOKENS`** ← 按主模型真实窗口（`claudeWindow`），1M 模型自动加 `[1m]` 标记，否则 Claude Code 按 200K 处理
4. **`CLAUDE_CODE_MAX_OUTPUT_TOKENS`** ← 按模型真实最大输出（`claudeOutput`），绕开 Claude Code 默认的 32000
5. **`CLAUDE_CODE_AUTO_COMPACT_WINDOW`** ← 长对话设置
6. **`CLAUDE_CODE_MODEL_CAPABILITIES`** ← 全局能力声明，格式 `模型id=effort,xhigh_effort,max_effort;下一个=...`，上限 8000 字符，当前在用的模型排前面（`claudeCapabilities`，约 393–466 行）
7. **`CLAUDE_CODE_AUTO_MODE_SERVER`** ← 上游不支持时关掉 auto review
8. **`CLAUDE_CODE_DISABLE_EXPLORE_INHERIT_CAP`** ← Explore 子代理的能力继承

### 3.3 两个防坑特判（`follow`，约 467–497 行）

- **haiku 档跟随主模型时**：若主模型是 Claude 系，改成跟该 provider 的 Haiku。否则会话标题、Explore 子代理、WebFetch 阅读这些高频小请求全走贵模型，额度消耗远快于原生。
- **fable 档跟随主模型时故意留空**：若把主模型写进 `ANTHROPIC_DEFAULT_FABLE_MODEL`，Claude Code 会把它当成 Fable 模型；Pro 账号没有 Fable 会先要求 usage credits，Opus 掉回 Haiku，别家模型（如 GPT 系）直接不跑。这是 magpie issue #2090 的真实事故。

### 3.4 `Follow` hook（约 2125–2146 行）

用户在 Claude Code 里自己 `/model` 换了模型后，magpie 下次读取时发现主模型变了，会把**仍处于「跟随」状态的档位**一起更新，但**不动用户自选的那些档**。

### 3.5 `Check` hook（约 2147–2185 行）

只检测不静默处理，逐条给人话：

- 管理员策略（`managed-settings.json`）覆盖了 `ANTHROPIC_BASE_URL`
- `ANTHROPIC_AUTH_TOKEN` 被清空导致走了 claude.ai 登录
- 保留了登录却写了 key

### 3.6 `Unwire`（约 2037–2067 行）

靠 stash 记录判定「这是 magpie 写的」才删；用户的 `effortLevel` 原样保留；主模型原本是用户的写回去，原本没有就删键。

---

## 4. Jolu 现状对比

> **核对时间 2026-11-12，基线 `9386a07`。** 当时的两个远端新提交（`b7c9ae7` 意图路由、`9386a07` 字号放大）与本主题无关；下表 14 项缺口**逐项 grep 核对过，无一实现**。同事接手时若代码已变，先按本表逐项重新确认。

**核对命令（可直接重跑）：**

```bash
# B1 / B2 / B5 B6 B7：全部无输出 = 未实现
grep -n "MAX_OUTPUT_TOKENS" backend/src/gateway/wiring.rs
grep -rn "MAX_CONTEXT_TOKENS" backend/src/
grep -rn "AUTO_COMPACT_WINDOW" backend/src/
grep -rn "CLAUDE_CODE_MODEL_CAPABILITIES" backend/src/
grep -rn "AUTO_MODE_SERVER" backend/src/
# B3 / B4：wiring.rs 里 fable / haiku 只有常量与测试，没有留空或降级逻辑
grep -n "fable\|haiku" backend/src/gateway/wiring.rs
# F1 / F4 / F5 / F6：无输出
grep -rn "/model opus" frontend/src/components/gateway/TierPanel.tsx
grep -rn "新会话\|new_session" frontend/src/components/gateway/*.tsx
grep -rn "compactAt\|compact_at" frontend/src backend/src
grep -rn "preview\|diff" backend/src/gateway/wiring.rs
```

### 4.1 已对齐的部分（不需要改）

| 能力 | Jolu 位置 |
|---|---|
| 主模型 + 四档 + small-fast + 子代理 + Auto 判别 | `backend/src/gateway/wiring.rs` 的 `claude_wire`（148 行起） |
| 档位自选（`tier_own` 语义，换主模型不重算） | `wiring.rs` 约 177 行 |
| 每档 effort 控件 | `frontend/src/components/gateway/TierPanel.tsx`（已有四档 + effort 单选 + Auto 判别） |
| effort 可用值校验 | `wiring.rs` 的 `fit_effort`（122 行） |
| 各档 `_SUPPORTED_CAPABILITIES` | `wiring.rs` 的 `claude_capabilities`（105 行） |
| 删 `ANTHROPIC_MODEL` | `wiring.rs`（已做） |
| 快照还原 | `wiring_store.rs` 的 `Applied` |
| 主模型写顶层 `model` | `wiring.rs` 第 229 行 |

### 4.2 后端缺失（按价值排序）

| # | 缺项 | 影响 | 位置 | 工作量 |
|---|---|---|---|---|
| B1 | `CLAUDE_CODE_MAX_OUTPUT_TOKENS` 在**接线时**写 | 现在只在 `claude_bridge.rs` / `claude.rs` 给内部子进程设，用户直接跑 Claude Code 时拿不到；第三方模型（GLM、DSD）回复被 32000 截断 | `wiring.rs` `claude_wire` 的 env 写入段（约 227 行附近） | 小（0.5 天） |
| B2 | `CLAUDE_CODE_MAX_CONTEXT_TOKENS` | 不写则大窗口模型（1M）被当 200K，长上下文能力浪费 | 同上 | 小（0.5 天，需先确认窗口数据来源，见 §5 决策 D2） |
| B3 | fable 档跟随主模型时**留空** | Pro 账号可能被要求 usage credits、Opus 掉 Haiku、别家模型不跑（magpie #2090） | `wiring.rs` 档位写入段 | 小（0.5 天） |
| B4 | haiku 档跟随主模型时**降级到该 provider 的 Haiku** | 主模型是 Claude 时，高频小请求走贵模型，额度消耗过快 | 同上 | 中（1 天，需确认降级目标模型从哪来） |
| B5 | `CLAUDE_CODE_AUTO_COMPACT_WINDOW`（长对话） | 压缩阈值用 Claude 默认，别的模型上下文不同会有偏差 | 同上 + 设置项 | 中（1 天，含设置存储） |
| B6 | `CLAUDE_CODE_MODEL_CAPABILITIES` 全局声明 | `/model` 里不显示 effort 选项。Jolu 只写了每档的 `_SUPPORTED_CAPABILITIES`，是简版；magpie 的全局声明还包含**所有**可路由模型的能力 | 同上 | 中（1 天，格式见 §3.2 第 6 条） |
| B7 | `CLAUDE_CODE_AUTO_MODE_SERVER` | 上游不支持时没关掉，会发无效请求 | 同上 | 小（0.5 天） |
| B8 | `Follow` hook（跟 `/model` 同步） | 用户在 Claude Code 里换模型后，跟随档不会更新，显示与实际不符，详见 §4.2.1 | 需要在读配置时做比对，可能放进 `escape.rs` 或独立模块 | 中（1.5 天，拆成 B8a/B8b/B8c） |

### 4.2.1 B8 细化：magpie 怎么「知道你在用什么模型」

**先纠正一个说法：** magpie 不是实时监控正在运行的会话。它是**每次界面刷新时现场回读配置文件并解析**。用户视角的「实时」来自：Claude Code 的 `/model` 会把选择写回文件，magpie 下次读同一文件就跟着变了。

#### magpie 的三层链路

**第一层：读配置，按优先级回退**（`claudeMain`，`claude.go` 2694–2709 行）

```go
1. settings.json 顶层 model                    ← /model 存的就是这里
2. 若顶层是 tier 别名（opus/sonnet/haiku/fable）→ 去 env.ANTHROPIC_DEFAULT_<TIER>_MODEL 找
3. env.ANTHROPIC_MODEL
4. env.ANTHROPIC_DEFAULT_SONNET_MODEL          ← Claude Code 的默认档
5. tierAt() 剥掉 ":effort" 后缀
```

这条优先级链的存在，正是 magpie 接线时**刻意不写 `ANTHROPIC_MODEL`** 的原因——那个变量优先级高于 `settings.json` 的 `model`，写了就会盖掉用户在 `/model` 里的选择。

**第二层：别名解析成真实模型**（`claudeAliasOption`，2329–2378 行）

用户在文件里写的是 `opus` 这种别名，magpie 在自己 catalog 里找 `claude-opus-*` 的最新版，拼出截图里的两段式文案：

```
opus · claude-opus-5-5
└┬─┘   └──────┬──────┘
用户写的      真实模型
```

特殊情况都处理了：
- `best` → 按 `opus` 解析
- `opusplan` → `opusplan · claude-opus-x / claude-sonnet-y`（两个模型）
- `[1m]` 后缀 → 解析出的模型是大窗口时补标记
- 解析结果不是 Claude 官方模型（走网关时是别家的）→ 图标跟着换

**第三层：`[1m]` 窗口标记**（`claudeViaMagpie`，2385–2404 行）

magpie 查自己 catalog 的 `Context` 字段，≥1M 的模型加 `[1m]` 后缀。原因：Claude Code 对**没带这个标记的模型一律按 200K 处理**，长对话会提前压缩。

#### 刷新机制

| 触发 | 行为 |
|---|---|
| 每次点击 / 切页面 | 重新 `GET /api/state`，`state()` 对每个 agent 调 `a.Values()` **重读配置** |
| 配置被外部改 | `GET /api/drift` 单独轮询，只返回被改过的项 |

#### Jolu 现状与差距

Jolu 现在是**只写不读**：接线时写配置、把「我写过什么」存快照，之后从不回读文件里的现值。

| 能力 | magpie | Jolu |
|---|---|---|
| 读 `settings.json` 顶层 `model` | ✅ 每次刷新 | ❌ 只读自己的快照 |
| 别名解析（`opus` → 真实模型） | ✅ | ❌ 界面只显示下拉选项，不显示真实模型 |
| 用户 `/model` 换模型后能感知 | ✅ 下次刷新 | ❌ 不知道，仍显示旧值 |
| `[1m]` 按真实窗口自动标记 | ✅ | ⚠️ 有 `[1m]` 处理，但是否按 `models_dev` 的窗口自动判断需确认 |

**最关键的一条**：用户装了 Jolu、接线选好模型，之后在 Claude Code 里 `/model` 换成别的，Jolu 界面会一直显示旧状态。magpie 靠「每次回读」避免了这个问题。

#### 拆成三个可独立交付的子项

| # | 内容 | 工作量 |
|---|---|---|
| **B8a** | 接线状态查询时回读 `settings.json` 的 `model` 与各档 env，而不是只读快照。用户 `/model` 换模型后界面能跟上 | 0.5 天 |
| **B8b** | 别名解析：读到的 `opus` 解析成真实模型，界面显示 `别名 · 真实模型` 两段式 | 0.5 天 |
| **B8c** | `[1m]` 按 `models_dev` 的窗口数据自动标记（≥1M 才加） | 0.5 天 |

**B8a 是核心**，单独做就能消除「显示与实际不符」；B8b/B8c 是呈现与标记精度的提升。


### 4.3 前端缺失

| # | 缺项 | 说明 | 工作量 |
|---|---|---|---|
| F1 | **「在 Claude Code 里怎么做」提示** | 每档后面加灰字 `/model opus`、`/model sonnet · 默认档位`、`/model haiku · 也用于后台小任务`。纯展示，数据可静态映射 | 小（0.5 天） |
| F2 | 子代理 effort 与模型的联动禁用 | 没设子代理模型时禁用 effort（magpie 同款） | 小 |
| F3 | **模型列表按 agent 裁剪 + 排序** | 打勾决定该 agent 菜单里出现哪些模型，拖动排序。需要后端存 per-agent 的模型开关表和顺序 | 大（2–3 天，含后端表与接口） |
| F4 | 新会话默认（模型 + effort，可留空跟随） | 独立入口；留空 = 用 agent 上次选择 | 中（1–2 天） |
| F5 | 长对话压缩阈值 | 与 B5 配套的界面 | 小（0.5 天，界面）+ B5 的后端 |
| F6 | **改动文件列表 + diff 预览** | 「改动了 X」+ 逐行 diff + 断开前预览。需要后端记录写入的文件与键，并提供 `preview` 接口 | 中（2 天） |

### 4.4 与 magpie 语义一致但 Jolu 表达不同的部分（不需要改）

- magpie 的「同主模型」= Jolu 的「跟随主模型」，语义相同
- magpie 的 `tier_own` 标记 = Jolu 快照里存绝对值，效果相同
- magpie 的 stash（进程内键值记录）≈ Jolu 的 `wiring_store` 快照表

---

## 5. 需要鼠鼠决策

| # | 决策点 | 选项 |
|---|---|---|
| D1 | **界面要不要按 magpie 的样子做** | magpie 面板信息密度高（一屏六行档位 + 三段提示 + 模型芯片 + 长对话 + diff）。Jolu.pen 没有对应画板。可选：(a) 等鼠鼠出新画板；(b) 先按 magpie 样式做「待确认」版；(c) 只做后端，界面等画板 |
| D2 | **模型窗口与最大输出的数据来源** | B1/B2 需要「这个模型支持多大上下文、多大输出」。Jolu 已有 `models_dev.rs`（models.dev 元数据）与 `models_dev_builtin.json`。是直接用 models.dev 的字段，还是另建表？需要确认 models.dev 里这两个字段的覆盖率 |
| D3 | **haiku 降级的目标怎么定** | B4 需要知道「某 provider 的 Haiku 是哪个模型」。magpie 是从自己的 catalog 里找同家族最轻的；Jolu 的模型池结构不同，需要定规则（比如同来源里 effort 最低/价格最低的，或明确映射表） |
| D4 | **要不要做 F3（按 agent 裁剪模型）** | 这是 magpie 里解决「模型太多找不到」的核心手段，但 Jolu 的模型池规模目前不大。可以先不做，等模型数上来了再说 |
| D5 | **F6（diff 预览）的必要性** | Jolu 现在有「断开并还原」但没有「改了什么」。如果鼠鼠更看重透明度就做；如果觉得够了可以跳过 |
| D6 | **agent 自动发现做到哪一层**（见 §8） | 鼠鼠已定：**做 A1 + A2，不做 A3 表驱动重构，不做插件化** |
| D7 | **Windows PATH 兜底是否照抄** | 见 §8.3。这是 magpie 用两个 issue（#839 / #1482）换来的，建议照做 |

---

## 6. 建议的执行顺序

如果只看后端正确性，**B1 → B3 → B2 → B7** 是最小闭环（约 2 天），能立刻解决「第三方模型回复被截断」「Pro 账号掉 Haiku」两个真实问题。

如果要做界面，F1 是投入产出比最高的（半天，纯展示），其次 F5 + B5（长对话，一天半）。

**A1（1 天）+ A2（1 天）独立于上面两档**，不依赖任何界面决策，可以并行做，见 §8.4。

F3、F6 工作量大且依赖产品决策，建议等 D4 / D5 定了再做。

---

## 7. 参考位置速查

| 内容 | 文件 | 行号（约） |
|---|---|---|
| Claude Code 字段声明 | `internal/agent/claude.go` | 598–2208 |
| 档位字段（四档循环） | `internal/agent/claude.go` | 1789–1828 |
| 子代理字段 | `internal/agent/claude.go` | 1857–1893 |
| Auto 判别字段 | `internal/agent/claude.go` | 1952–1989 |
| 登录字段 | `internal/agent/claude.go` | 1993–2030 |
| `writeTiers`（派生项写入） | `internal/agent/claude.go` | 1534–1645 |
| `claudeCapabilities`（能力声明） | `internal/agent/claude.go` | 393–466 |
| `follow`（haiku / fable 特判） | `internal/agent/claude.go` | 467–497 |
| `Follow` hook | `internal/agent/claude.go` | 2125–2146 |
| `claudeMain`（回读主模型的优先级链） | `internal/agent/claude.go` | 2694–2709 |
| `claudeAliasOption`（别名 → 真实模型） | `internal/agent/claude.go` | 2329–2378 |
| `claudeViaMagpie`（`[1m]` 窗口标记） | `internal/agent/claude.go` | 2385–2404 |
| `Check` hook | `internal/agent/claude.go` | 2147–2185 |
| `Unwire`（断开还原） | `internal/agent/claude.go` | 2037–2067 |
| 详情面板渲染 | `internal/gui/assets/app.js` | `connectPanel` 1956–2153 |
| 档位行渲染 | `internal/gui/assets/app.js` | 2028–2078 |
| 模型列表弹窗 | `internal/gui/assets/app.js` | `openAgentModels` 3216 起 |
| 长对话控件 | `internal/gui/assets/app.js` | `renderAgentPrefs` 20866–20885 |
| 长对话 HTML | `internal/gui/assets/index.html` | 465–468 |

**Jolu 侧对应位置：**

| 内容 | 文件 | 行号 |
|---|---|---|
| Claude Code 接线 | `backend/src/gateway/wiring.rs` | `claude_wire` 148 起 |
| 主模型与 env 写入 | `backend/src/gateway/wiring.rs` | 227–250 |
| 能力声明 | `backend/src/gateway/wiring.rs` | `claude_capabilities` 105 |
| effort 校验 | `backend/src/gateway/wiring.rs` | `fit_effort` 122 |
| 快照 | `backend/src/gateway/wiring_store.rs` | — |
| 档位面板 | `frontend/src/components/gateway/TierPanel.tsx` | — |
| 档位纯逻辑 | `frontend/src/lib/tierModels.ts` | — |

---

## 8. Agent 自动发现（A 段）

> **状态（2026-11-13）：A1 / A2 已实现并真机验证。** 实现 `backend/src/agent_discovery.rs`（探测顺序 + Windows PATH 兜底 + `classify` 纯决策函数）、`agent_version.rs` 升级（并行 + 3 秒总超时 + `pending` + 24 小时版本缓存 + 兜底目录查找）、命令 `agent_discover`、接线弹窗对未安装工具置灰标「未检测到」。真机结果：claude-code `installed`（CLI 经兜底找到、版本 2.1.114），codex / zcode / gemini / opencode `cli_missing`（配置残留、CLI 不在——ZCode 是 GUI 客户端本无 `zcode` 命令，状态如实且不阻碍接线）。下文保留当时的规划说明作为实现依据。

**背景：** magpie 的 Agents 页会自动列出「这台电脑上装了哪些 agent」，用户不用手动添加。Jolu 现在是**内置式**——固定 5 个 agent，从不探测。

**鼠鼠决策（2026-11-12）：** 做 A1 + A2，**不做** A3 表驱动重构，**不做**插件化。

### 8.1 Jolu 现状：内置式

`backend/src/gateway/wiring.rs:19`：

```rust
pub const AGENTS: [&str; 5] = ["claude-code", "codex", "zcode", "gemini", "opencode"];

pub fn is_agent(agent: &str) -> bool {
    AGENTS.contains(&agent)
}
```

每个 agent 的配置路径、当前令牌读取、接线写法都是 `match` 分支硬编码（`config_path`，968 行起）：

```rust
match agent {
    "claude-code" => crate::creds::claude_dir()?.join("settings.json"),
    "codex"       => crate::creds::codex_dir()?.join("config.toml"),
    "zcode"       => zcode_dir(&home).join("v2").join("provider_config.json"),
    "gemini"      => home.join(".gemini").join(".env"),
    "opencode"    => XDG_CONFIG_HOME/opencode/opencode.json,
}
```

**接线的 5 个 agent 从来没做过「有没有装」的探测**——`wiring.rs` 里没有任何 `Command::new` / `which` / `LookPath`。后果：本机没装 Codex 也会显示 Codex 卡片；装完 agent 之后才启动 Jolu 可能读不到。

**已有的部分基础**（可复用，但都不是「发现」）：

| 模块 | 现有能力 | 与「发现」的差别 |
|---|---|---|
| `backend/src/agent_version.rs` | 跑 `<cli> --version` 读版本，硬编码 5 个；无缓存、无并行、无超时 | 只探 CLI 是否存在，不看配置文件/目录 |
| `backend/src/login_detect.rs` | 读各家登录文件判断**登录状态** | 是「登没登录」，不是「装没装」 |
| `backend/src/native_sources.rs` | 扫 ZCode/Qoder/Trae/WorkBuddy 的会话与账号 | 是「本机登录态镜像」，不覆盖 claude/codex/gemini/opencode |

### 8.2 magpie 的做法

**核心只有五行**（`internal/agent/agent.go` 的 `Detected()`，370–394 行），命中任一即算「在」：

```go
1. a.detect()                      // 少数特例的自定义探测
2. 配置文件存在 && 确实是它写的       // 最强证据（pathOwn）
3. 配置目录存在（排除共用目录）       // dirShared 排除
4. 自定义标记文件存在                // sign
5. 命令在 PATH 里能找到              // binAt
```

`Detected()`（588 行）遍历 `All()`（注册表 60+ 个 agent，`agents.go` 148–215 行的 `onMachine`），逐个调 `a.Detected()`。

**三种排除法（防误判，见 `agent.go` 280–326 行的字段注释）：**

| 机制 | 防的是什么 | 真实案例 |
|---|---|---|
| `Taken(dir)` | 目录被别的工具占了 | `~/.dsh` 是某个 shell 的目录，不是 dsh 这个 agent |
| `dirShared` + `sign` | 目录和自家别的产品共用，光有目录不算数，必须找到只有它会写的文件 | Cursor CLI 的 `~/.cursor` 也是 Cursor 编辑器的 |
| `pathOwn(path)` | 配置文件其实是别人写的 | Qoder CN 目录里的 `settings.json` 是 JetBrains 插件写的（#1495） |

**版本号**（`internal/agent/cliupdate.go` 590–607、852–884 行）：

1. 缓存键 = 文件路径 + 大小 + 修改时间，24 小时有效；失败 10 分钟后重试
2. 没缓存就跑 `<cli> --version`，`parseVersion` 解析
3. **并行**探测所有 agent，超时 3 秒返回已拿到的，剩下的标 `pending` 下次再问

**Windows 的 PATH 兜底**（`binAt`，396–410 行）—— 这段注释值得完整读一遍：

> Windows 没有 login shell 可以借 PATH 给 magpie。一个在 Claude Code 安装器把 `claude.exe` 放进 `~/.local/bin` **之前**就启动的 magpie，永远列不出它，而登录检查（走 `FindTool`）却能找到它（#839, #1482）。

所以在 Windows 上不能只查当前进程的 PATH，还要查：注册表里的最新 PATH、`~/.local/bin`、npm / bun / Volta / pnpm 的目录。

### 8.3 A 段任务清单

**A1 — 加「是否安装」探测（1 天）**

| 步骤 | 内容 |
|---|---|
| A1.1 | 新建 `backend/src/agent_discovery.rs`，定义 `AgentSpec { id, name, bin, dir, path, sign }` 与 `detected() -> Vec<&AgentSpec>` |
| A1.2 | 探测顺序照 magpie：配置文件在（且 `pathOwn` 通过）→ 配置目录在（且非共用）→ `sign` → PATH 命令 |
| A1.3 | **Windows PATH 兜底**：注册表 `HKCU\Environment\Path` + `HKLM\...\Session Manager\Environment\Path`，再查 `~/.local/bin`、`%APPDATA%\npm`、`~/.bun/bin`、`~/.volta/bin`、pnpm 目录 |
| A1.4 | 门槛：任一证据成立即算已装，返回 `installed: bool` 与 `evidence: &'static str`（用于诊断和界面说明） |
| A1.5 | 前端：供应商/接线列表只列 `installed: true` 的 agent（或未装的置灰并标「未检测到」） |

**A2 — 状态分层（1 天）**

在 A1 基础上补「检测到了但不可用」的中间状态：

| 状态 | 判据 | 界面表达 |
|---|---|---|
| `installed` | A1 任一证据 | 正常显示，可接线 |
| `cli_missing` | 配置文件/目录在，但 PATH 里没命令 | 显示但标「CLI 未找到」，接线按钮禁用，说明「装回 CLI 后可接入」 |
| `unsupported` | 检测到了但 Jolu 没有适配器 | 显示但标「暂不支持接入」（对应 magpie 的 `NATIVE_ONLY`） |
| `not_found` | 全部证据都没有 | A1 的选择：不显示 / 置灰显示 |

版本探测顺带升级（`agent_version.rs`）：
- 加 24 小时缓存（键含路径 + 大小 + mtime）
- 改成并行 + 3 秒超时，未完成的标 `pending`

### 8.4 明确不做的部分

| 不做 | 原因 |
|---|---|
| **A3 表驱动重构**（把接线逻辑也抽成表） | 5 个 agent 的接线写法差异很大（Claude 写 JSON env、Codex 写 TOML、Gemini 写 .env、ZCode 写两份 JSON），表只能统一「发现」这一层，接线仍要各自实现。为「少改 5 行 match」重构不划算 |
| **插件化**（第三方写插件加 agent） | 需要插件协议、加载、沙箱隔离，与 Jolu「面向普通用户的桌面应用」定位不符。鼠鼠已定不做 |
| **扩到 60+ agent** | Jolu 的定位是主流 AI 工具 + 各家订阅，不是「接管你电脑上所有 AI 工具」。先把现有 5 个做扎实 |

### 8.5 验收要点

1. **装完 agent 才启动 Jolu**：`claude` 在 `~/.local/bin`、但当前进程 PATH 里没有 → 必须能检测到（这是 Windows 兜底要解决的）
2. **目录被占不误判**：手工造一个 `~/.dsh` 普通文件 → 不应把 dsh 判为已装
3. **共用目录要 sign**：只有 `~/.cursor` 目录、没有 Cursor CLI 的独有文件 → 不判为已装
4. **卸载后状态**：删掉 CLI 但保留配置目录 → 归为 `cli_missing`，接线按钮禁用并说明原因
5. **版本探测不卡界面**：某个 CLI 的 `--version` 挂住 → 3 秒后页面正常渲染，该 agent 标 `pending`

### 8.6 参考位置

| 内容 | 文件 | 行号 |
|---|---|---|
| `Detected()` 核心判定 | `internal/agent/agent.go` | 370–394 |
| Agent 注册表（60+） | `internal/agent/agents.go` | 148–215 |
| `detect` / `pathOwn` / `sign` / `dirShared` 语义 | `internal/agent/agent.go` | 280–326 |
| `binAt`（Windows PATH 兜底） | `internal/agent/agent.go` | 396–410 |
| 版本探测与缓存 | `internal/agent/cliupdate.go` | 590–607、852–884 |
| Jolu 现有接线白名单 | `backend/src/gateway/wiring.rs` | 19 |
| Jolu 现有配置路径 | `backend/src/gateway/wiring.rs` | `config_path` 968 起 |
| Jolu 现有版本探测 | `backend/src/agent_version.rs` | `detect` 162 |

---

## 9. 定位与功能取舍（2026-11-12 定）

**鼠鼠的定位：** Jolu 是 **AI 工具的管家** —— 把用户已有的订阅账号登进来、管好额度、替你签到、排队执行任务，并把做过的事留成可查的会话。

**与 magpie 的分界线是「是否持有账号生命周期」**：Jolu 持有一条账号的完整生命周期（登录 → 保存 → 额度 → 续期 → 随时处置），magpie 只借凭据做转发。

### 9.1 为什么这四条曾被我判为「不该追」

我最初的判断按**投入产出比**：magpie 在单项深度上领先太多（127k Go vs Jolu 96k Rust），按单项追永远追不完。那四条看起来都是「基础设施的活」。

**这个判断错在维度上。** 用户需求与投入产出比不是同一个维度。鼠鼠的修正：**这四条都是用户真会提的需求，不能划到「不追」**。

### 9.2 修正后的判断：按「用户能不能用起来」排序

| 项 | 用户真实场景 | 需求强度 | 结论 |
|---|---|---|---|
| **agent 广度** | 「我用的不是你支持的那 5 个工具」→ 根本装不上 | **高（准入门槛）** | **现在做**（有节奏，见 9.3） |
| **全协议互转** | 「我的 key 是 Chat 接口，但我要用 Claude Code」 | 中（有明确付费群） | **接着做**（先打透三条主链路） |
| **多机配置漫游** | 「公司电脑和家里电脑要同步」 | 中（有需求不高频） | **缓一缓**（先做备份，不做实时同步） |
| **插件市场** | 「我想让 Jolu 支持我自己写的工具」 | 低（普通用户不写插件） | **先不做**（鼠鼠 2026-11-12 定） |

关键区分：**agent 广度和协议互转是「准入类」**——没有它，用户连门都进不来，后面的额度、签到、自动化全都没机会用。**插件市场和多机漫游是「体验类」**——用户已经用起来了才会提。

### 9.3 agent 广度：做，但不照抄 magpie 的 60 个

**不能只扩白名单。** 直接复制 magpie 的 60+ agent 探针等于把 127k 行代码的积累摊薄到 Jolu 的维护预算里。做法是三条：

1. **先扩到「主流 + 国内常用」15–20 个**（覆盖约 90% 用户），不追 magpie 全量
2. **提供「通用接入」兜底**：对没有专门适配的工具，允许用户手动填配置路径与写入方式——把「不支持」变成「你自己填」，比直接拒绝体验好得多
3. **A1/A2 自动发现是前提**（§8）：列表再长，没有探测也白搭

**优先级：A1（探测）→ 通用接入 → 逐家适配。**

### 9.4 全协议互转：先打透三条主链路

> **状态（2026-11-13）：三条主链路已全部打通并端到端验证。** 3×3 协议矩阵（调用方 Anthropic / Chat / Responses × API 上游 anthropic / openai）全连通：新增 `translate.rs` 的 Chat→Anthropic 请求转换与 Anthropic→Chat 响应回程（非流式 + 流式 SSE），`wire.rs` 补 `(Chat/Responses, anthropic)` 两格（Responses 经 Chat 中转）。真机验证用 mock 上游 + 网关 relay：Chat 与 Responses 两条对角线及流式均通过，mock 确认收到标准 Anthropic 请求形状（`/v1/messages`、`x-api-key`、`anthropic-version`、`max_tokens` 兜底）。订阅侧此前已由 `claude_bridge` 等适配器覆盖。

Jolu 已有 `responses_chat.rs`（Chat ↔ Responses）与 Anthropic 通道。目标不是「支持所有协议」，是：

- **Chat ↔ Anthropic**：第三方 key（DeepSeek / Kimi / GLM）接入 Claude Code 的必经之路
- **Chat ↔ Responses**：接入 Codex 的必经之路
- **Anthropic ↔ Chat**：反向，让 Claude 订阅服务 Chat 调用方

Gemini 原生协议排最后（用户基数小）。

### 9.5 多机配置漫游：先做备份，不做实时同步

- **该做**：账号 + 额度历史的**备份与恢复**（管家怕丢数据）
- **缓做**：配置实时同步（基础设施怕不一致；实时同步还要处理冲突，复杂度陡增）

### 9.6 插件市场：先不做

**鼠鼠 2026-11-12 决定：插件市场先不做。**

替代方案：轻量扩展能力——「自定义供应商」（已有 `providerCatalog.ts` 的预设库）与「自定义 agent 配置」（9.3 的通用接入）。这两项覆盖了插件市场 80% 的实际用途，成本却低一个数量级。等用户量到「有人愿意开发插件」时再评估。

### 9.7 该做但不在这四条里的（管家独有）

这四项 magpie 结构上没有，是管家定位的护城河：

| 项 | 为什么是管家独有 |
|---|---|
| **额度异常 / 将尽的主动通知** | 管家应该在你察觉之前提醒，而不是等你来问 |
| **账号 + 额度历史的备份恢复** | 见 9.5；管家怕丢数据 |
| **跨模块联动** | 额度事件驱动自动化续跑、会话与额度双向跳转、签到失败在账号卡上标出 |
| **F6 diff 预览**（§4.3） | 管家动过主人的东西必须交代清楚，这是信任基础 |

**「全能」的护城河应该长在横向串联上，不在单项深度。** magpie 的每块能力是孤立的（路由归路由、library 归 library、checkin 归 checkin），Jolu 的每块能力应该是一条链：**账号 → 额度 → 自动化 → 会话 → 留档**。这类联动不需要新协议、新 agent，只要把已有模块连起来，因此也是 Jolu 相对 magpie 更容易出彩的地方。

### 9.8 全能的隐含前提：每项都必须可靠

基础设施出 bug 只是少一个功能；**管家出 bug，用户可能丢账号、少领额度、白花 token**，伤害不可逆。

所以节奏是：**每补一项，先做到「不出错」，再补下一项。** `AGENTS.md` 的「未知值、真实的零、能力不支持、查询失败、数据过期要分别表达」与「不得删除旧账号、历史统计和原始会话文件」本质上都是这条原则。

### 9.9 品类对照

| | magpie | Jolu |
|---|---|---|
| 品类 | 模型路由层（基础设施） | AI 工具管家 |
| 解决的问题 | 把散落各处的模型来源统一成一个入口 | 把 AI 工具管好、用好 |
| 核心资产 | 协议适配广度、agent 覆盖广度 | 账号生命周期、额度可见性、任务与留档 |
| 成功标准 | 支持的 agent 与协议越多越好 | 每项照看都不出错 |
| 关系 | 可以并存：magpie 是「管道」，Jolu 是「管家」 | |

---

## 10. 完成状态总览（2026-11-13 更新，接手先读这里）

**接手与待办详情：** `docs/plans/2026-11-13-butler-parity-remaining-handoff.md`（逐项改动位置：改哪个文件、哪个函数、加什么字段，接手同事从那里开始）。

**已实现（带真机/测试验证）：**

| 项 | 状态 | 验证方式 |
|---|---|---|
| A1 agent 安装探测 | ✅ 已实现 | 真机：claude-code `installed`（CLI 经 WinGet 路径找到）；codex / zcode / gemini / opencode `cli_missing` 与实际吻合；单测覆盖 §8.5 验收场景 |
| A2 状态分层与版本探测升级 | ✅ 已实现 | 单测：缓存命中、文件变化重探、超时 pending 不冒充结果；真机：claude 2.1.114 读出，其余如实 `found:false` |
| 9.4 全协议互转（三条主链路） | ✅ 已实现 | 单测 8 个（含流式事件序列与截断不冒充）；真机：mock anthropic 上游 + 网关 relay，Chat 与 Responses 两条对角线 + 流式全通，mock 确认收到标准 Anthropic 请求形状 |
| 9.6 插件市场 | 按鼠鼠决定**不做** | 替代：自定义供应商（已有预设库）+ 通用接入（见 9.3） |

**未完成（接手从这里开始，按优先级）：**

| 项 | 要做什么 | 依赖 / 备注 |
|---|---|---|
| B1 `CLAUDE_CODE_MAX_OUTPUT_TOKENS` 接线时写 | 解决第三方模型回复被 32000 截断 | **先定 D2**（窗口/输出数据来源，建议 models.dev） |
| B3 fable 档跟随主模型时留空 | 防 Pro 账号掉档（magpie #2090） | 无依赖，`wiring.rs` 档位写入段 |
| B2 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` | 1M 模型不再被当 200K | **先定 D2** |
| B7 `CLAUDE_CODE_AUTO_MODE_SERVER` | 上游不支持时关 auto review | 无依赖 |
| B8a 回读配置 | 用户 `/model` 换模型后界面能跟上 | 无依赖（见 §4.2.1） |
| A 段前端完整化 | 接线列表只列已安装 / 置灰说明，等鼠鼠画板定稿后按稿补 | 当前已有最小版（弹窗置灰），标「待画板确认」 |
| B4 haiku 降级、B5 长对话、B6 全局能力声明 | 见 §4.2 对应行 | B4 先定 D3（降级目标），B5/B6 无依赖 |
| 9.3 agent 广度（15–20 个 + 通用接入兜底） | 准入门槛；先做「通用接入」（用户手填配置路径与写入方式） | 依赖 A1 ✅ 已具备；通用接入约 3–5 天 |
| 9.5 账号 + 额度历史备份恢复 | 管家怕丢数据；实时同步缓做 | 无依赖，1–2 天 |
| 9.7 管家独有四项（额度主动通知 / 备份 / 跨模块联动 / F6 diff） | 横向串联是护城河 | 额度通知与 F6 优先 |
| F1–F6 详情面板前端 | **鼠鼠已在 Jolu.pen 设计，按稿实现**（本表的 B1–B8 是其对应后端） | 等画板导出核对 |

**已知遗留（2026-11-13 已查明）：** 真机验证时「你好」触发「请求体缺少 model 字段」并非脱敏改坏 JSON——脱敏对中文请求体往返无损（`mask.rs` 测试覆盖）；原因是调用方（Windows PowerShell 5）把中文按本地代码页发送，请求体不是 UTF-8。网关现在对这种请求明确报「请求体不是 UTF-8 编码」。

**2026-11-13 接手进度：** P0-1（B1/B2/B3/B7，连同 B5/B6 的后端）、P0-2（B8a/B8b）、F6 的后端预览接口已完成，详见交接清单 §一。界面按 Jolu.pen「统一网关 · R2 重设计」的 V5 定稿方向实现。

**决策状态：** D1 界面按稿（pen 已设计）；D2 / D3 待定（阻塞 B1/B2/B4）；D4 / D5 待定；D6 已定（A1+A2，不做 A3 与插件化）；D7 建议照做（已随 A1 实现）。
