# Agent 接线解耦重构 · 交底报告

日期：2026-11-21
执行计划：`docs/plans/2026-11-20-agent-wiring-refactor-plan.md`
涉及模块：`backend/src/gateway/`、`backend/src/db.rs`、`frontend/src/lib/agentPage.ts`

---

## 2026-11-21 测试隔离事故与修复

上轮验收中的 `a_wired_sample_is_recognised_as_wired` 存在隔离漏洞。Claude Code 和 Codex 的路径解析会读取真实环境变量，测试虽然传入临时 home，仍可能写到真实配置。Claude Code 的真实 `settings.json` 被测试样本覆盖，令牌尾号 `abcd1234` 导致接入时触发其他实例保护。Codex 的真实 `config.toml` 随后发现为空，测试确曾写入该路径，清空发生的具体过程尚未确认。

已把该测试放到独立子进程，将各工具使用的目录环境变量指向临时目录，并在写主配置前检查路径范围。注册表的 6 条测试、11 家生产接线往返测试及其他实例保护测试均通过。上述测试前后，真实 Claude Code 和 Codex 配置文件的哈希未改变。上轮完整测试通过的数字仍是当时实际结果，但不能作为所有测试都已隔离的证据。

Claude Code 配置已从 Jolu 的 2026-11-15 备份恢复，受损样本另存于 `settings.json.jolu-test-damaged-20261121.bak`。该备份之后的更改无法确认。Codex 找到的最新备份为 2026-09-21，未找到近期备份，当前空文件尚未恢复。真实桌面接入尚未再次验证。

---

## 2026-11-21 后续优化与验收

本节记录原报告之后完成的两项优化。第 0 至 6 节保留当时的过程和结论，其中只读注册表、前端手写清单及测试结果的描述属于历史状态，以本节为准。

- 后端注册表增加各工具的接入和断开计划函数，公共层继续负责备份、写入和保存记录。状态规则也通过注册表登记，Gemini 与 dsh 的多文件判定仍有各自处理。
- 前端直接使用现有 `gateway_status.wiring` 返回的清单，删除独立维护的 `WIRABLE_AGENTS`。已探测但尚无专用接线的工具仍显示为暂不支持。状态请求成功后独立更新，因此模型目录请求失败不会挡住工具清单。
- 纯浏览器且没有模拟后端时，首次打开 Agent 页面无法获取后端清单，原先由硬编码清单生成的默认工具卡不会显示。模拟后端的页面入口验收通过，覆盖注册表新工具、已探测但不支持的工具以及模型目录请求失败。

| 验收项 | 本轮结果 |
|---|---|
| `cargo test --lib` | 1556 通过、0 失败、52 忽略；测试编译输出未显示 warning |
| 11 家隔离目录往返 | 通过，覆盖生产接线计划、写盘、数据库记录、状态及断开；真实桌面入口和真实工具完整往返尚未验证 |
| 前端 Agent 清单测试 | 10 通过 |
| 前端 `tsc --noEmit` | 通过 |
| 前端 `node --test src/lib/*.test.mjs` | 493 通过、1 失败；失败为现有灵动岛改动相关的 `botGaze.test.mjs` 尺寸补偿断言 |
| 新增一家工具的 diff 不超过 2 个文件 | 未证明；仍需实现适配器、登记注册表，可能还需探测登记和测试样本 |

前一轮完整 Rust 测试中的 `status_is_read_only_and_repeatable` 曾失败，单独重跑与本轮完整测试均通过，失败原因尚未确认。

---

## 历史记录

以下记录原阶段的实施过程和当时结论。

## 0. 一句话概括

把「往 11 家 AI 工具配置里写网关地址」这套接线代码，从**按动作分文件**改成**按工具登记**：
只读侧的路径、状态与提示收拢到注册表，并用测试检查前后端清单。新增工具仍需实现接入、断开与前端登记。

---

## 1. 为什么要做

### 1.1 业务背景

Jolu 的「模型路由」要接管本机各家 AI 工具的请求：往它们的配置文件里写网关地址和令牌（这一步叫**接线**），让工具的请求经过 Jolu 再转发出去；用户断开时要把配置**原样还回去**。

截至重构前已接 11 家：Claude Code、Codex、ZCode、Gemini、OpenCode、Kimi、dsh、Pi、Pencil、Qoder、Qoder CN。

### 1.2 当时的结构问题

代码是**按动作分文件**的：

```
wiring.rs（2900 行）
  ├── AGENTS 常量        ← 11 家清单，手写
  ├── is_agent()         ← 判断是否可接，查上面的常量
  ├── config_path()      ← 一个 match，11 个分支
  ├── ours_with()        ← 一个 match，判定「这文件是我们写的吗」
  ├── restart_notice()   ← 一个 match，11 条文案
  └── plan_wire()        ← 一个 match，11 个分支的写入逻辑
```

**每加一家工具，要改 6-7 个地方，而且漏了不报错。**

### 1.3 已经因为这个结构踩过的坑（不是假设）

| 时间 | 事故 | 漏在哪 |
|---|---|---|
| 2026-11-17 | 真机点 Kimi 接入，报 `保存接线记录失败：FOREIGN KEY constraint failed` | `db.rs` 的 agents 种子表漏了 kimi，而 `agent_wiring` 表的外键指向它 |
| 2026-11-20 | dsh 接上过之后永远显示「未接入」，断开和启动自愈都跳过它 | 注册表判定里令牌前置门写错，且**手写的 11 家自查清单没查出来** |

第二条尤其说明问题：我手工逐家核对了 11 家，还是漏了——因为「判定逻辑是否成立」这种事实，静态读代码看不出来，必须真的跑一遍。

### 1.4 目标（可验收）

1. 新增一家工具的 diff **≤ 2 个文件**
2. 现有 11 家真机往返（接入 → 状态 → 断开 → 配置复原）**全部通过**
3. `cargo test` 全绿、无新增 warning；前端 typecheck + 测试通过

> 判据：改完还是改 5 处以上，说明解耦没做到位。

---

## 2. 三步做了什么

### 第 1 步 · `WireContext` 收参数

#### 需求

`plan_wire` 有 **16 个位置参数**。问题是：

- 16 个里**每家工具只用到 3-4 个**——Claude Code 用 `tiers` / `subagent_model`，Kimi 完全不看；dsh 只看 `catalog_models`
- 第 9 格和第 12 格**都是 `Option<&str>`**——填错位置编译器不报错，运行时才发现「Claude 的档位写进了 Kimi」
- 调用处是一长串字面量，看不出传了什么

#### 做了什么

拆成三个按「谁用」分组的具名结构：

```rust
pub(crate) struct WireContext<'a> {
    // 通用字段
    pub agent: &'a str,
    pub base: String,        // 拥有所有权
    pub token: String,       // 拥有所有权
    pub instance_short: &'a str,
    pub catalog: &'a [ZcodeModel],
    pub prior_snapshot: Option<&'a str>,
    pub chosen: ModelChoice<'a>,      // 选模型相关
    pub claude: Option<ClaudeInput<'a>>,  // Claude 专属，其余传 None
}
```

`base` / `token` 特意改成**拥有 `String`** 而不是 `&str`：原调用方写的是 `base: &format!(...)`，靠 Rust 的临时值生命周期延长才能编译，一旦 ctx 要跨语句存活就失效。

#### 达到的效果

- 字段具名，编译器检查，调用处一眼看清传了什么
- 非 Claude 工具传 `None`，`plan_wire` 内部用一组廉价空桩统一按引用使用，不必每处再判 `agent == "claude-code"`

**风险**：低。纯重构，不改行为。

---

### 第 2 步 · 注册表（原计划是 `AgentAdapter` trait，实际改了方案）

#### 原计划

引入 `AgentAdapter` trait，把 `wiring.rs` 拆成 `adapter/<tool>.rs`，一家一个文件：

```
wiring/
  mod.rs        编排（读文件 / 备份 / 原子写 / 记账）
  traits.rs     AgentAdapter trait
  adapter/
    claude.rs   一家一个文件
    kimi.rs
    dsh.rs ...
```

#### 为什么改方案（调研结论，必须记录）

派了两个子代理分别查 Jolu 和 magpie，加上自查，发现 **5 个结构性障碍**：

1. **`Applied` 组装留在编排层是伪解耦点**——`base` 后缀（codex）、token 白名单、`provider`/`mode`（codex）、`model`/`label`/`tiers`/`extra_env`（claude）全写在共享结构里。adapter 只返回 `files + snapshot` 的话，这里仍是 6 个 `agent == ...` 三元表达式，加一家照样要改，**漏写 token 白名单会让多实例检测静默失效**。
2. **dispatch 前有两处专属变换**：claude 的 `keyHolds` **改写 token**、codex 的 oauth 检测**决定 mode**。它们发生在 `match` 之前，adapter 无法「自己拥有」。
3. **`wiring_wsl.rs` 直接依赖** claude/codex/opencode 的 8 个私有函数。搬进子模块后 `pub(super)` 只到 `wiring`，WSL 会编译失败。
4. **测试里约 30 处**直接调 `claude_wire` / `codex_restore` 等私有函数。
5. **四家快照形状不同**（`Option<Value>` / 恒 `Some` / `None` 且忽略），统一成 trait 的 `snapshot()` 会改变 opencode 的既有行为。

**结论**：真正的障碍不是 `match` 本身，而是「共享前导 + `Applied` 组装 + `status_with` 特判」。**只做只读侧能拿到 80% 收益，风险低一个数量级。**

#### 实际做了什么：只读侧注册表 `gateway/wiring_specs.rs`

一张表，一家一行，声明 8 个字段：

```rust
pub(crate) struct AgentSpec {
    pub id: &'static str,
    pub path: fn(&Path) -> Option<PathBuf>,   // 配置路径
    pub base: fn(&str) -> Option<String>,     // 从文件读 base URL
    pub token: fn(&Path, &str) -> Option<String>, // 从配置或用户目录读令牌
    pub token_match: TokenMatch,              // Exact / Prefixed / LegacyLiteral / Ignored
    pub extra: WiredExtra,                    // None / KimiDefaultModel / PiDefaultProvider / SelectionOurs / Adapter
    pub notice: &'static str,                 // 重启提示
    pub uses_catalog: bool,                   // 是否要网关模型清单
}
```

把多处只读分发收敛成一张表：

| 原来 | 现在 |
|---|---|
| `config_path()` 一个 11 分支 match | `(spec.path)(&home)` |
| `restart_notice()` 手写 11 条文案 | `spec.notice` |
| `ours_with()` 判定逻辑 | `spec.is_wired(...)` |
| `escape.rs` 的模型目录白名单 | `spec.uses_catalog` |
| `foreign_marker()` 的令牌读取分发 | `spec.token` |

**保留 2 处特判**（有明确理由，注释里写了）：

- **gemini**：`WiredExtra` 只读主文件，而 gemini 还要求第二个文件 `settings.json` 的 `selectedType == gemini-api-key`，这条留在 `ours_with` 开头
- **dsh**：状态判定要读 `.env` 与多 profile patch，`WiredExtra::Adapter` 转调 `wiring_dsh::wired`

#### 第 2 步顺带完成（原属第 3 步的 3.2）

**清单从注册表派生**，三处不再手写：

- `wiring::agents()` 从 `SPECS` 派生
- `db.rs` 的 `sync_agent_seeds()` 在 `init` 末尾按注册表 `INSERT OR IGNORE`——**Kimi 那类 FK 事故物理上不可能再发生**
- 前端 `WIRABLE_AGENTS` 由守卫测试锁定（见第 3 步 3.3）

#### 达到的效果

只读侧的登记集中到注册表，新增工具仍需完成写入与前端登记：

| | 重构前 | 重构后 |
|---|---|---|
| `AGENTS` 数组 | 改 | **0**（派生） |
| `config_path` | 改 | **0**（表里一行） |
| `restart_notice` | 改 | **0**（表里一行） |
| `ours_with` | 改 | **0**（表里 token_match + extra） |
| `escape.rs` 模型目录 | 改 | **0**（表里 uses_catalog） |
| `db.rs` 种子 | 改（漏了 FK 报错） | **0**（init 自动同步） |
| 前端 `WIRABLE_AGENTS` | 改（漏了开关变灰） | 改 1 处，**漏了测试红** |
| `plan_wire` / `plan_unwire` | 改 2 处 | 改 2 处（写入逻辑本质不同，保留） |
| **合计** | 多处手写登记 | 注册表、写入与还原分支、前端清单仍需修改 |

写入侧的实现分布因工具而异，不能仅凭登记表估算新增工具所需修改的文件数。

---

### 第 3 步 · 抽公共件

#### 3.1 · YAML 行级引擎 → `gateway/yaml_lines.rs`（200 行）

**需求**：`wiring_dsh.rs` 里约 600 行是「按行解析 YAML、找块范围、拼接」，与 dsh 无关。而且 dsh 的编辑必须**保住用户的注释和缩进**——不能引 YAML 库重新序列化，否则用户手写的注释全没了。

**做了什么**：抽出 4 个纯函数 + 1 个结构：

```rust
pub(crate) struct Item { pub id: String, pub lines: Vec<String> }
pub(crate) fn id_of(line: &str) -> Option<String>
pub(crate) fn parse(text: &str) -> (Vec<String>, Vec<Item>)
pub(crate) fn join(head: &[String], items: &[Item]) -> String
pub(crate) fn block_range(lines: &[String], indent: usize, key: &str) -> Option<(usize, usize)>
```

dsh 保留 4 个薄转发（`dsh_parse` / `dsh_join` / `dsh_find` / `dsh_block_range`），30+ 调用点不用改。

测试覆盖了 LF 换行样本的 `parse` → `join` 往返。引擎使用 `str::lines` 和 LF 拼接，不能保证所有换行格式及文件末尾状态逐字节不变。

**达到的效果**：后续 qwen / droid / workbuddy 这类「数组插项」工具可直接复用，不必重写。

#### 3.2 · 清单由注册表派生

并入第 2 步完成（见上）。

#### 3.3 · 前端清单一致性

**原计划**：让后端提供清单命令，前端纯渲染 `[{ id, name, wirable, explanation }]`。

**决议：不做，改为守卫测试。**

理由：`WIRABLE_AGENTS` 只有 11 个字符串，改运行时契约需要调整多个调用处。守卫测试在运行测试时检查清单是否同步，运行时仍使用前端清单。

**做法**：前端测试**直接读** `backend/src/gateway/wiring_specs.rs` 源码，正则提取 id，与 `WIRABLE_AGENTS` 比对：

```javascript
const backend = [...source.matchAll(/AgentSpec\s*\{([\s\S]*?)\n\s*\},/g)]
  .map((block) => block[1].match(/id:\s*"([^"]+)"/)?.[1])
  .filter((id) => typeof id === "string")
// 解析条数必须等于源码里的 AgentSpec 实例数，正则漏排版会被抓出来
const declared = (source.match(/(?<!struct )(?<!impl )AgentSpec\s*\{/g) ?? []).length
assert.equal(backend.length, declared, "正则漏掉了某些排版")
assert.deepEqual([...WIRABLE_AGENTS].sort(), [...backend].sort())
```

**已验证守卫有效**：故意从前端删掉 `qoder-cn`，测试立刻红并给出可读原因（「能接但没卡片 / 有卡片但开关灰」）；恢复后绿。

> 正则 2026-11-20 修过一次：上一版只认 `AgentSpec {\n`（多行排版），单行加一家会被静默漏过。

---

## 3. 结果

### 3.1 验收标准对照

| 验收标准 | 结果 | 证据 |
|---|---|---|
| 新增一家 diff ≤ 2 个文件 | **未证明** | 注册表、写入与还原分支及前端清单仍需登记，没有新增工具的实际 diff 验证 |
| 现有 11 家真机往返全部通过 | **未完成** | 自动测试在隔离目录运行生产函数，未执行真实桌面完整入口 |
| `cargo test` 全绿、无新增 warning | **未达到** | 本轮 `cargo test --lib` 有 1 项失败，52 项忽略；完整 `cargo test` 与 warning 对照未验证 |
| 前端 typecheck + 测试通过 | **未达到** | typecheck 和 Agent 清单测试通过，前端 lib 测试有 1 项失败 |

### 3.2 11 家往返测试是怎么做的

`plan_wire` / `plan_unwire` 走 `creds::home()`（读 `USERPROFILE` / `HOME`）。真机桌面实例没有重定向 home，**点「接入」会写用户的真实配置**——这正是现有 `agent-icons-kimi-preview.e2e.mjs` 只做「写入预览」不做往返的原因。

所以这条测试用**子进程 + 临时 home**：

```
父测试
  └── 为每家起一个子进程
        ├── USERPROFILE / HOME → 临时目录
        ├── CLAUDE_CONFIG_DIR / CODEX_HOME / XDG_CONFIG_HOME
        ├── KIMI_CODE_HOME / ZCODE_HOME / DSH_HOME
        └── PI_CODING_AGENT_DIR / QODER_CONFIG_DIR / QODERCN_CONFIG_DIR
              ↓
        子进程里跑生产代码：
        plan_wire → commit_plan（保存数据库快照）→ status_with（判已接线）
                  → 读取记录 → unwire → 检查各配置文件
                  → status_with（判未接线）
```

测试调用生产接线函数，并使用合成配置和隔离目录。真实桌面入口及真实工具的行为仍需另行验证。

测试对 JSON、TOML 和 `.env` 比较配置含义，对样本中的 YAML 文件逐字节比较，并检查原本不存在的文件没有遗留网关内容。样本还覆盖 dsh 接线后用户修改密钥的情况。

### 3.3 往返测试发现并修复的问题

| 工具 | 缺陷 | 修法 |
|---|---|---|
| codex | `codex_wire` 在文件原本没有 `model_providers` 时新建空表，`codex_restore` 只删子项 → 断开后残留 `[model_providers]` 空表头 | 新增 `prune_empty_table` |
| kimi | 同上：残留 `[models]` / `[providers]` 空表头 | 同上 |
| zcode | `zcode_rules_wire` 新建 `schemaVersion` / `providerOrder` / `modelConfigRules`，restore 只 `retain` 不删壳 → 断开后多出 4 个空键 | 新增 `zcode_rules_snapshot` 记录接线前结构是否存在，只清我们新建的 |
| zcode | 原配置没有 `provider` 时，断开后遗留空对象 | 快照记录原字段是否存在，仅删除本次新建的空对象 |
| dsh | 断开时原有 credentials 密钥未恢复，接线后用户修改的密钥可能被覆盖 | 保存原值，仅在当前仍为 Jolu 写入的值时恢复 |

> zcode 那条绕了一圈：第一版思路是「在接线**后**的文本上判断键本来是否存在」，那永远为真。正确做法必须靠**接线前的快照**。

### 3.4 本轮复核

| 项 | 结果 |
|---|---|
| `cargo test --lib` | **1555 通过 / 1 失败 / 52 忽略**；失败项为 `service::gateway::status::tests::status_is_read_only_and_repeatable`，两次读取到的 Codex 状态不同。单独重跑通过，原因未确认 |
| 11 家隔离目录往返 | 通过，覆盖生产计划、落盘、数据库记录、状态与断开 |
| 编译警告 | 本轮测试编译输出未显示 warning，未单独运行 `cargo check` 或对比修改前 warning |
| 前端 `tsc --noEmit` | 通过 |
| 前端 Agent 清单测试 | 10 通过 |
| 前端 `node --test src/lib/*.test.mjs` | 493 通过 / 1 失败，失败项是 `botGaze.test.mjs` 的尺寸补偿断言 |

---

## 4. 新增文件与改动量

| 文件 | 行数 | 说明 |
|---|---|---|
| `backend/src/gateway/wiring_specs.rs` | 新 | 注册表、11 条 `AgentSpec` 及测试 |
| `backend/src/gateway/yaml_lines.rs` | 新 | YAML 行级引擎及测试 |
| `backend/src/gateway/wiring.rs` | 改 | `WireContext` 重构、11 家往返测试及还原缺陷修复 |
| `backend/src/gateway/wiring_dsh.rs` | 改 | YAML 引擎复用及凭据还原修复 |
| `backend/src/db.rs` | 改 | `sync_agent_seeds` 从注册表派生 |
| `frontend/src/lib/agentPage.test.mjs` | +38 | 前后端清单守卫测试 |

---

## 5. 遗留与后续

### 5.1 本次没做

| 项 | 原因 |
|---|---|
| `AgentAdapter` trait（写入侧彻底拆文件） | 5 个结构性障碍，收益/风险比不划算（见 §2 第 2 步） |
| 3.3 前端清单命令化 | 当前由测试检查两端清单，运行时仍有两份清单 |
| 真机桌面点接入往返 | 生产函数已在隔离目录测试，桌面入口和真实工具仍待验证 |

### 5.2 什么时候该回头做 trait

- 工具数涨到 20+，单个 `match` 的分支多到难读
- 出现「某家的写入逻辑需要独立的 `Applied` 字段」这类需求
- 需要按工具做能力过滤（F3「按 Agent 裁剪模型菜单」）

### 5.3 仍然待确认的

以下为 `AGENTS.md` 已记录的、与本次重构无关的待确认项：高级 / 写入改动页签的展开表单、已接入行卡的档位数硬编码、概览「今天」范围选择无后端、按 Agent 裁剪模型菜单、新会话强度、改端口。

---

## 6. 一句话给后来的人

新增工具需要登记注册表、实现接入和还原、更新前端清单及相应样本。数据库种子从注册表派生，前后端清单一致性由测试检查。
