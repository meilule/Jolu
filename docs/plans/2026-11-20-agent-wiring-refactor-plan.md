# Agent 接线分层重构计划

日期：2026-11-20
状态：**第 1、2、3 步均已完成并验证**（2026-11-20）；第 2 步按实际调研结果调整为「只读侧注册表」，详见 §7
背景调查：`docs/plans/2026-11-19-remaining-agent-wiring-research.md`（剩余工具调研）

---

## 0. 为什么要做

**问题：加一家工具要改 11 处。** 2026-11-18 加 dsh 改了 7 处，2026-11-20 加 Pi 改了 8 处（`wiring.rs` 内）+ db 种子 + escape 模型 + 前端 2 处 = 11 处。每处都可能漏，漏了不报错、只表现为「功能莫名不工作」。

真实踩过的坑（Kimi，2026-11-17）：`db.rs` 的 agents 种子表忘了加 kimi，点接入报 `FOREIGN KEY constraint failed`——文件都写好了，记录存不进去。

**当前规模**（2026-11-20）：`wiring.rs` 2924 行，里面有 **6 个独立的 `match agent`**，一家工具的行为被切成 6 片撒在不同位置。

**目标**：加一家工具从「改 11 处」降到「新建 1 个 adapter 文件 + 注册 1 行」。

---

## 1. 三步方案

### 第 1 步 · `WireContext` 收参数（本轮执行）

**做什么**：把 `plan_wire` 的 16 个位置参数收进一个具名结构体。

**为什么**：
- 16 个参数里，**每家工具只用到 3-4 个**——Claude Code 用 `tiers` / `subagent_model`，Kimi 完全不看；dsh 只看 `catalog_models`
- 位置参数里第 9、第 12 格都是 `Option<&str>`，**填错位置编译器不报错**，运行时才发现「Claude 的档位写到 Kimi 里去了」
- 加一家要改调用处的参数组装逻辑

**改完效果**：字段具名、编译器检查、调用处一眼看清传了什么。

**风险**：低。纯重构，不改行为。

### 第 2 步 · `AgentAdapter` trait（待执行）

**做什么**：把「按动作分文件」改成「按工具分文件」。

```
现在                          改后
wiring.rs                     wiring/
  config_path()  ← 所有家       mod.rs        编排（读文件/备份/原子写/记账）
  ours_with()    ← 所有家       traits.rs     AgentAdapter trait
  plan_wire()    ← 所有家       adapter/
  plan_unwire()  ← 所有家         claude.rs   一家一个文件
                                 kimi.rs
                                 dsh.rs
                                 pi.rs
                                 extra.rs    简单 JSON 户共用
```

**trait 契约**（草案）：

```rust
pub(crate) trait AgentAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn config_path(&self, home: &Path) -> Option<PathBuf>;
    fn extra_files(&self, home: &Path) -> Vec<PathBuf> { Vec::new() }
    fn plan_wire(&self, ctx: &WireContext) -> Result<Vec<(PathBuf, String)>, String>;
    fn plan_restore(&self, ctx: &RestoreContext, snapshot: &Value) -> Result<Vec<(PathBuf, String)>, String>;
    fn snapshot(&self, ctx: &WireContext) -> Option<Value>;
    fn is_wired(&self, ctx: &StatusContext) -> bool;
    fn foreign_instance(&self, home: &Path, instance: &str) -> Option<String> { None }
    fn restart_notice(&self) -> &'static str { "需重启后生效" }
}
```

**能防住的 bug**（真实场景）：
- 忘了改 `ours_with` → 文件写进去了但界面显示「未接入」，用户再点一次又写一遍，断开时找不到记录 → 配置残留
- 忘了改 `Applied.token` 白名单 → 多实例检测失效，两个 Jolu 实例互相覆盖
- 忘了改 `db.rs` 种子 → FK 报错（Kimi 已踩）

**风险**：中。建议先迁 3 家（kimi + dsh + extra 一家）验证，每迁一家跑一次完整测试 + 真机往返。

### 第 3 步 · 抽公共件（待执行）

#### 3.1 · YAML 行级引擎 → `yaml_lines.rs`

`wiring_dsh.rs`（945 行）里约 600 行是「按行解析 YAML、找块范围、拼接、空清理」，与 dsh 无关。抽出来后：
- `parse_items(text) -> (Vec<String>, Vec<Item>)`
- `find_block(lines, indent, key) -> Option<(usize, usize)>`
- `join(head, items) -> String`

后续 qwen / droid / workbuddy 都是数组插项，可复用。

#### 3.2 · 清单由注册表派生

```rust
pub fn agents() -> Vec<&'static str> { ADAPTERS.iter().map(|a| a.id()).collect() }
// db.rs
for id in wiring::agents() { conn.execute("INSERT OR IGNORE INTO agents(id) VALUES(?)", [id]) }
```

物理上不可能漏。

#### 3.3 · 前端清单改由后端提供（已决议不做，见 §7.5）

现在 `frontend/src/lib/agentPage.ts` 的 `WIRABLE_AGENTS` 与后端 `AGENTS` 重复，`wireExplanation` 的文案也硬编码在前端。改由后端返回 `[{ id, name, wirable, explanation }]`。

**2026-11-20 定案：不纳入本次**，用守卫测试代替（前端测试直接读
`wiring_specs.rs` 源码比对 id 集合）。原因与效果见 §7.5。

---

## 2. 验收标准

1. **现有 17 家全部真机往返仍通过**（尤其 Kimi / dsh / Pi 三家有独立实现的）
2. **新增一家的 diff ≤ 2 个文件**（新 adapter + 注册一行）
3. `cargo test` 全绿、无新增 warning
4. 前端 `typecheck` + 测试通过

如果改完还是改 5 处以上，说明解耦没做到位。

---

## 3. 第 1 步详细设计（本轮执行）

### 3.1 · 现有签名（16 参数）

```rust
pub(crate) fn plan_wire(
    agent: &str,
    port: u16,
    instance_short: &str,
    prior_snapshot: Option<&str>,
    model: Option<&str>,
    label: Option<&str>,
    description: Option<&str>,
    prior_model: Option<&str>,
    prior_label: Option<&str>,
    subagent_model: Option<&str>,
    prior_values: &[String],
    tiers: Option<&TierModels>,
    tier_about: &dyn Fn(&str) -> (Option<String>, Option<String>),
    extras: &ClaudeExtras,
    holds: &dyn Fn(&str) -> bool,
    catalog_models: &[ZcodeModel],
) -> Result<WirePlan, String>
```

### 3.2 · 目标结构

```rust
/// 一次接线请求的全部输入。字段按用途分组，每家工具只读自己需要的。
pub(crate) struct WireContext<'a> {
    // ── 通用（所有工具都用）──
    pub agent: &'a str,
    pub base: &'a str,           // 由 port 算好，不再传 port
    pub token: &'a str,          // 由 agent + instance_short 算好
    pub instance_short: &'a str,
    pub catalog: &'a [ZcodeModel],

    // ── 快照：重新接线时沿用上次的原值 ──
    pub prior_snapshot: Option<&'a str>,

    // ── 模型选择（Claude Code 用 label/description，其它家多数只用 model）──
    pub chosen: ModelChoice<'a>,

    // ── Claude Code 专属（其它家 None）──
    pub claude: ClaudeInput<'a>,
}

pub(crate) struct ModelChoice<'a> {
    pub model: Option<&'a str>,
    pub label: Option<&'a str>,
    pub description: Option<&'a str>,
    pub prior_model: Option<&'a str>,
    pub prior_label: Option<&'a str>,
    pub prior_values: &'a [String],
    pub subagent_model: Option<&'a str>,
    pub tiers: Option<&'a TierModels>,
}

pub(crate) struct ClaudeInput<'a> {
    pub tier_about: &'a dyn Fn(&str) -> (Option<String>, Option<String>),
    pub extras: &'a ClaudeExtras,
    /// keyHolds：Claude Code 里已是网关认的命名客户端 Key 时保留它
    pub holds: &'a dyn Fn(&str) -> bool,
}
```

### 3.3 · 为什么拆成三个结构而不是一个

一个结构装 16 个字段会变成「上帝对象」，读起来和 16 个位置参数一样累。按「谁用」分组：
- `WireContext` 本身：所有工具都用的 6 个
- `ModelChoice`：与「选哪个模型」相关的 8 个（Claude Code 全用，其它家用 1-2 个）
- `ClaudeInput`：只有 Claude Code 用

**好处**：`wiring_pi::wire()` 拿到的 `ctx.chosen.model` 一眼知道是「用户选的模型」，不用数第几个参数。

### 3.4 · 改造范围

1. 新增三个结构体（`wiring.rs` 里，`plan_wire` 之前）
2. `plan_wire` 签名改成 `plan_wire(ctx: &WireContext)`
3. 函数体内逐处替换 `agent` → `ctx.agent`、`model` → `ctx.chosen.model` 等
4. `escape.rs` 的 `plan_agent` 改成先构造 `WireContext` 再调用
5. 检查是否有测试直接调 `plan_wire`（有则同步改）

**不改的部分**：`plan_unwire`（参数少，5 个）、`status_with`、`commit_plan`。

### 3.5 · 验证

```bash
cargo test --manifest-path backend/Cargo.toml   # 期望 1538 全过
pnpm --dir frontend typecheck
```

真机抽查 3 家：Claude Code（用满 16 个参数的那家）、Pi、pencil（走 extra 的）。

---

## 4. 后续步骤的执行前提

- **第 2 步开始前**：确认第 1 步已合入且真机通过
- **第 3 步开始前**：确认第 2 步迁完至少 3 家
- **任何一步做完都要**：更新本文件的「状态」，并记录实际改动点数量（对比「加一家改 11 处」的基线）

---

## 5. 风险与边界

**不做的事**：
- 不改 `wiring_extra.rs` / `wiring_dsh.rs` / `wiring_pi.rs` 的内部实现（第 1 步只动签名与调用）
- 不改 `plan_unwire`（下一轮再说）
- 不动前端（第 3.3 步才做）

**如果中途发现**：某个工具的行为依赖参数顺序（不应存在，但若发现），停下来记录，不要硬改。

---

## 6. 第 1 步执行记录（2026-11-20）

### 实际改动

| 文件 | 改动 |
|---|---|
| `gateway/wiring.rs` | 新增 `WireContext` / `ModelChoice` / `ClaudeInput` 三个结构体；`plan_wire` 签名从 16 个位置参数改成 `plan_wire(ctx: &WireContext)`；体内逐处替换为具名访问 |
| `gateway/escape.rs` | `plan_agent` 改为先构造 `WireContext` 再调用；模型目录与 `holds` 闭包提到构造前 |
| `gateway/wiring_pi.rs` | 删未使用的 `is_pi` |
| `agent_discovery.rs` | `go_program` 加 `#[allow(dead_code)]`（删 Goose 后无调用方，但 `Spec.bin_not` 机制与测试要留） |

### 与原参数的对应关系（16 → 8 + 8 + 3）

| 原参数 | 新位置 |
|---|---|
| agent / instance_short / prior_snapshot / catalog_models | `WireContext` |
| port | 调用方算成 `base: String` 传入 |
| model / label / description / prior_model / prior_label / subagent_model / prior_values / tiers | `ModelChoice` |
| tier_about / extras / holds | `ClaudeInput`（非 Claude 传 `None`，内部用空桩） |
| （新增）token | 从 `plan_wire` 内部提到外部，`token_for()` 调用方式不变 |

### 执行中修掉的两个问题

1. **临时值生命周期**：初版 `base: &format!(...)` 靠临时值生命周期延长才能编译，一旦 ctx 跨语句存活就失效。改成 `String` 拥有所有权。
2. **`claude` 为 None 的类型体操**：初版用元组 + `as` 转换，可读性差；改成三个具名空桩变量。

### 验证

- 后端 `cargo test`：**1538 全过**（零回归，编译警告清零）
- 前端 `typecheck` + 493 测试全过
- 真机 11 家全部走一遍写入预览（覆盖所有 `plan_wire` 分支）：

| 工具 | 文件数 | 键数 |
|---|---|---|
| claude-code | 1 | 2 |
| codex | 1 | 7 |
| zcode | 2 | 276 |
| gemini | 2 | 2 |
| opencode | 1 | 5 |
| kimi | 1 | 428 |
| dsh | 4 | 481 |
| pencil | 1 | 5 |
| qoder | 1 | 7 |
| qoder-cn | 1 | 7 |
| pi | 2 | 7 |

- Pi 真机往返：接入 → 断开，两个文件与备份**语义完全一致**

### 第 1 步的收益与局限

**收益**：`plan_wire` 调用处一眼看清传了什么，加字段不会错位。

**局限**：加一家工具仍要改 8-11 处——**第 1 步只解决了「传参安全」，没解决「改多处」**。那个要靠第 2 步的 `AgentAdapter` trait。

---

## 7. 第 2、3 步执行记录（2026-11-20）

### 7.1 第 2 步实际做了什么（与计划的偏差必须记录）

**计划是**：引入 `AgentAdapter` trait，把 `wiring.rs` 拆成 `adapter/<tool>.rs`，一家一文件。

**实际调研后改成**：**只读侧注册表**（`gateway/wiring_specs.rs`），写入侧保留现状。

**为什么改**（调研发现，两条子代理分析 + 自查）：

1. **`Applied` 组装留在编排层是伪解耦点**：`base` 后缀（codex）、`token` 白名单、`provider`/`mode`（codex）、`model`/`label`/`subagent_model`/`tiers`/`extra_env`（claude）都写在共享结构里。adapter 只返回 `files + snapshot` 的话，这里仍是 6 个 `agent == ...` 三元表达式——加一家照样要改，漏写 token 白名单会让多实例检测静默失效。
2. **dispatch 前有两处专属变换**：claude 的 keyHolds **改写 token**、codex 的 oauth 检测**决定 mode**。它们发生在 `match` 之前，adapter 无法"自己拥有"。
3. **`wiring_wsl.rs` 直接依赖** claude/codex/opencode 的 8 个私有函数（`claude_snapshot` / `codex_wire` / `opencode_restore` …）。搬进子模块后 `pub(super)` 只到 `wiring`，WSL 会编译失败——必须改可见性或让 WSL 也走 adapter。
4. **测试里约 30 处直接调** `claude_wire` / `codex_restore` 等私有函数。
5. **四家快照形状不同**：claude/codex = `Option<Value>`、gemini = 恒 `Some`、opencode = `None` 且 restore 忽略。统一成 trait 的 `snapshot()` 会改变 opencode 的既有行为。
6. **令牌判定宽严不同**：exact（extra/kimi）/ 允许任意 `jolu-`（pi/dsh）/ 完全不看（codex/gemini/opencode/zcode）。

**结论**：真正的结构障碍不是 `match` 本身，而是「共享前导 + `Applied` 组装 + `status_with` 特判」。只做只读侧能拿到 80% 收益（4 处分发收敛成一张表），风险却低一个数量级。

### 7.2 只读侧注册表（新增 `gateway/wiring_specs.rs`，330 行）

一张表，一家一行，声明 8 个字段：

```rust
pub(crate) struct AgentSpec {
    pub id: &'static str,
    pub path: fn(&Path) -> Option<PathBuf>,   // 配置路径
    pub base: fn(&str) -> Option<String>,     // 从文件读 base URL
    pub token: fn(&str) -> Option<String>,    // 从文件读令牌
    pub token_match: TokenMatch,              // Exact / Prefixed / LegacyLiteral / Ignored
    pub extra: WiredExtra,                    // None / KimiDefaultModel / PiDefaultProvider / SelectionOurs / Adapter
    pub notice: &'static str,                 // 重启提示
    pub uses_catalog: bool,                   // 是否要网关模型清单
}
```

**收敛了 4 处分发**：
- `config_path()` → `(spec.path)(&home)`
- `restart_notice()` → `spec.notice`
- `ours_with()` → `spec.is_wired(...)`
- `escape.rs` 的模型目录白名单 → `spec.uses_catalog`

**保留了 2 处特判**（有明确理由，注释里写了）：
- **gemini 的 settings.json 判定**：`WiredExtra` 只读主文件，这条要读第二个文件，留在 `ours_with` 开头
- **dsh 的状态判定**：要读 `.env` 与多 profile patch，`WiredExtra::Adapter` 转调 `wiring_dsh::wired`

### 7.3 第 2 步顺带完成（原计划第 3 步的 3.2）

**清单从注册表派生**，三处不再手写：
- `wiring::agents()` 从 `SPECS` 派生
- `db.rs` 的 `sync_agent_seeds()` 在 `init` 末尾按注册表 `INSERT OR IGNORE`——加一家自动补种子，Kimi 那类 FK 事故物理上不可能再发生
- 前端 `WIRABLE_AGENTS` 由**守卫测试**锁定（下节）

### 7.4 第 3 步·3.1（抽 `yaml_lines.rs`）

`wiring_dsh.rs` 里 6 个纯通用函数（`id_of` / `parse` / `join` / `find` / `block_range` + `Item` 结构）抽到 `gateway/yaml_lines.rs`（260 行，含 6 条测试）。

`wiring_dsh.rs` 里保留 4 个薄转发（`dsh_parse` / `dsh_join` / `dsh_find` / `dsh_block_range`），30+ 调用点不用改。

**收益**：后续 qwen / droid / workbuddy 这类「数组插项」工具可直接复用，不必重写。

### 7.5 第 3 步·3.3（前端清单一致性——改为守卫测试）

**计划是**让后端提供清单命令、前端纯渲染。**实际改成**守卫测试：

- 前端测试直接读 `backend/src/gateway/wiring_specs.rs` 源码，正则提取 id，与 `WIRABLE_AGENTS` 比对
- 零新增产物、零运行时契约变更
- **已验证守卫有效**：故意从前端删掉 `qoder-cn`，测试立刻红并给出可读原因（"能接但没卡片 / 有卡片但开关灰"）；恢复后绿
- 正则同时支持单行与多行 `AgentSpec` 排版（2026-11-20 修：上一版只认 `AgentSpec {\n`，单行加一家会被静默漏过）；另加「解析条数 == 源码里 `AgentSpec {` 实例数」断言，排除 `struct` / `impl` 声明

**3.3 的最终决议（2026-11-20 定案）：不纳入本次。** 理由是运行时契约改造要动
`agentRow` 签名、`AgentsView` 传参、`agentPage.test.mjs` 三处，而 `WIRABLE_AGENTS`
只有 11 个字符串；守卫测试达到同样的「不可能漏同步」效果，且零运行时风险。
若将来工具数量涨到 20+ 或需要按能力过滤卡片，再回来做命令化。

### 7.6 验证

**最终一轮（2026-11-20，含 11 家往返测试）**

| 项 | 结果 |
|---|---|
| 后端 `cargo test --lib` | **1554 通过 / 0 失败 / 52 忽略** |
| 编译警告（`cargo check` 与 `cargo test --no-run`） | **0** |
| 前端 `typecheck` | 通过 |
| 前端测试 | **500 通过 / 0 失败** |
| **11 家全量往返** | **全部通过**：`every_agent_round_trips_through_production_plan`，每家走 `plan_wire` → `commit_plan` → `status_with`（判已接线）→ `plan_unwire` → 语义还原断言 → `status_with`（判未接线） |

**11 家往返测试怎么做的（为什么不是点桌面）**

`plan_wire` / `plan_unwire` 走 `creds::home()`（读 `USERPROFILE` / `HOME`）。真机桌面实例
没有重定向 home，点「接入」会写用户的真实配置——这正是现有
`agent-icons-kimi-preview.e2e.mjs` 只做「写入预览」不做往返的原因。

所以这条测试用**子进程 + 临时 home**：父测试为每家起一个子进程，把 `USERPROFILE` /
`HOME` 以及所有能改配置落点的变量（`CLAUDE_CONFIG_DIR` / `CODEX_HOME` /
`XDG_CONFIG_HOME` / `KIMI_CODE_HOME` / `ZCODE_HOME` / `DSH_HOME` /
`PI_CODING_AGENT_DIR` / `QODER_CONFIG_DIR` / `QODERCN_CONFIG_DIR`）一起指进临时目录，
子进程里跑的是**与用户点接入完全一致的生产代码**。

判据是语义等价（生产实现走 `serde_json::Value` / `toml_edit` 解析后重新序列化，
键序与缩进会规范化，这是既有行为），加两条硬断言：网关地址不残留、`jolu-` 令牌不残留。

**这条测试抓出的 3 个既有缺陷（非本次重构引入，已修）**

| 工具 | 缺陷 | 修法 |
|---|---|---|
| codex | `codex_wire` 在文件原本没有 `model_providers` 时新建空表，`codex_restore` 只删子项 → 断开后残留 `[model_providers]` 空表头 | 新增 `prune_empty_table`，restore 后清理空表 |
| kimi | 同上：残留 `[models]` / `[providers]` 空表头 | 同上 |
| zcode | `zcode_rules_wire` 新建 `schemaVersion` / `providerOrder` / `modelConfigRules`，restore 只 `retain` 不删壳 → 断开后多出 4 个空键 | 新增 `zcode_rules_snapshot` 记下「这几处接线前是否存在」，restore 只清我们新建的；用户本来就有的空数组/空对象原样保留 |

**此前一轮（注册表 + yaml_lines 完成时）**

| 项 | 结果 |
|---|---|
| 后端 `cargo test` | 1549 全过（第 1 步 1538 → +5 注册表 + 6 yaml_lines） |
| 真机 `gateway_status` | 返回 11 家，id 顺序与注册表一致 |
| 真机 11 家写入预览 | 全部通过，文件数与键数与重构前一致 |
| 真机 Pi 往返 | 接入 `wired: true` → 断开 `idle`，两个文件与备份完全一致 |

### 7.7 加一家的成本变化

| | 重构前 | 重构后 |
|---|---|---|
| `AGENTS` 数组 | 改 | **0**（注册表派生） |
| `config_path` | 改 | **0**（注册表一行） |
| `restart_notice` | 改 | **0**（注册表一行） |
| `ours_with` | 改 | **0**（注册表 token_match + extra） |
| `escape.rs` 模型目录 | 改 | **0**（注册表 uses_catalog） |
| `db.rs` 种子 | 改（漏了 FK 报错） | **0**（init 自动同步） |
| 前端 `WIRABLE_AGENTS` | 改（漏了开关变灰） | 改 1 处，**漏了测试红** |
| `plan_wire` / `plan_unwire` | 改 2 处 | 改 2 处（写入逻辑本质不同，保留） |
| **总改动点（易漏的）** | **7 处** | **1 处（有测试保护）** |

写入侧的 2 处仍在，但它们是「实现体」不是「登记点」——`wiring.rs` 里只有 1-2 行转发，实现已隔离在独立文件。
