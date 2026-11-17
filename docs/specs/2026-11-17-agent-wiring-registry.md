# Agent 接线总台账方案研究（探测即可接线）

日期：2026-11-17
状态：**讨论中**——由鼠鼠发起，本文整理讨论点，交同事共同研究最优方案；决议写进第 8 节
关联：`docs/specs/2026-11-17-magpie-agent-wiring.md`（magpie 接线机制拆解，本文的背景读物）

---

## 1. 需求目标（鼠鼠原话与拆解）

> 「20 家的接线是否可以有一张表去记录，如果用户本机有，会对应相应的表去扫出 agent，随时可以接线。」

拆成可验收的目标：

1. **消除死区**：本机探测到的工具，只要接线键名有出处（官方文档 / magpie 源码背书），就应该能接——不再出现「暂无专用接入 · 等待接入适配」卡着不动（Continue / Qwen / Cline / Goose / Zed / VS Code / Droid / Crush / Pencil / DSH / Copilot / Qoder / WorkBuddy 等 14 家目前全部卡在这里）；
2. **降成本**：新增一家「简单工具」的接线 = 加一行声明数据，不再写一版 Rust 写入器 + 还原器 + 测试；
3. **不失守**：现有 6 家专用接线（claude-code / codex / zcode / gemini / opencode / kimi）的行为与还原语义零回归；
4. **守住项目规矩**（AGENTS.md）：键名没有出处不许自动接线（不猜）；还原要完整；不能根据 UI / 图标 / 套餐名推断能力。

---

## 2. 现状盘点：Jolu 已经有三大块资产

方案不是从零开始，下面三块都已在仓库里，先弄清它们各自能干什么。

### 2.1 探测表 `SPECS`（`backend/src/agent_discovery.rs`）

「本机有什么」这半边已经是表驱动。20 行常量表，每行：

```rust
Spec { agent: "kimi", name: "Kimi CLI", bin: "kimi", dir: dir_kimi, file: "config.toml", sign_in: "", detect: Detect::Standard, bin_not: None }
```

- `dir` / `file`：配置目录与文件（有 `*_HOME` 类环境变量覆盖的写在 dir 函数里）；
- `detect`：`Standard`（目录有真内容算已装）/ `CliOnly`（还要命令行在 PATH，如 Gemini）；
- `bin_not`：同名程序甄别钩子（Goose 用 `go_program` 排除 Go 自带的 goose）；
- 命令查找带缓存（T5/T6，见 `docs/plans/2026-11-16-agent-discovery-fix.md`），一次探测约 100ms、过窗口约 4ms。

**结论：需求里「扫出 agent」这半边已达成，不用动。**

### 2.2 通用接线引擎 `custom_agents.rs`（`backend/src/custom_agents.rs`）

**这是本次讨论最重要的发现：表驱动的通用接线器已经存在且相当完整**——用户在「添加自定义工具」里手动接的工具走的就是它：

- 数据形态（`CustomAgent`）：`format`（json / toml / env）+ `config_path` + `base_url_key` / `auth_key` / `model_key` + `base_suffix`（`/v1` 这类）——**这就是鼠鼠想要的「一张表」**，只是现在挂在「用户手动添加」的入口上，没和探测出的 20 家对上；
- `plan_values`：从表声明算出要写的三个键值（base = `http://127.0.0.1:{port}{suffix}`，auth = `jolu-<slug>.<实例8位>` 令牌，model 可选）；
- `plan_wire`（约 470 行）：**键级快照**——写前取每个键的原值；区分「上次 Jolu 写的」（applied 记账比对）与「用户自己改的」，用户改过的保留；上次写过、这次没写的键还原回原值。这已经是 magpie `kimiOwn` 的语义；
- `plan_unwire`：**只还原仍等于 Jolu 写下值的键**（值对不上 = 用户或别的工具改过，不动）——外部意图优先，magpie 同款；
- 快照落在 `custom_agents` 表的 `original_json` / `applied_json`，接线状态走统一的 `agent_wiring` 表（mark_pending / mark_wired / mark_idle），文件先备份再原子写（复用 `steady` + `commit_plan` 同款设施）。

前端对应 `frontend/src/lib/agentPage.ts` 的 `CUSTOM_AGENT_PRESETS`（7 行预设，qwen / kimi / droid / zed / crush / goose / continue），**带 `verified` 标记**——按 AGENTS.md「不猜」的规矩，键名没有出处的预设 `verified: false`，界面提示用户核对后再接。

**结论：需求里「一张表随时可接」的引擎已经存在，缺口只在两点：① 预设表没和探测结果联动（探测到 qwen 不会自动指向 qwen 预设）；② verified 键名没有逐家核实。**

### 2.3 专用适配器 `gateway/wiring.rs`（6 家）

复杂工具的手写写入器，每家一节：claude-code（env 逐键 + 档位 + 子代理模型 + keyHolds）、codex（OAuth 保留）、zcode（两份文件 + 每模型规则）、gemini（两文件 + 登录切换）、opencode、kimi（2026-11-17 新增：整批 `models."jolu/…"` 表 + 旧版少写键 + default_model 快照还原）。公共设施：`plan_wire` → `commit_plan`（备份 → 先记 pending → 多文件原子写 → 失败回滚）、`foreign_marker`（另一实例接管检测）、`token_for`（`jolu-<agent>.<实例8位>`）。

**已知坑（2026-11-17 Kimi 踩过，已修 + 回归测试锁死）**：`agent_wiring.agent_id` 外键指向 `agents` 种子表，每加一个可接线 agent 必须同步补 `db.rs` 的种子行——测试 `every_wirable_agent_has_a_seed_row_for_the_wiring_foreign_key`。

---

## 3. magpie 的做法（为什么它 30 家全可接）

详见拆解报告，这里只给决策相关的结论：

- magpie **没有总表**，是每工具一个声明式写入器文件（30 家），靠公共设施（`atomic` 原子写、`stash.json` 还原记账、`applied.json` 漂移记账、`Wired/Drift/Check/Notice/Sync` 状态机）把每家的代码压到很薄；
- 它不用表的原因：复杂户的接线是**操作题**不是填空题——「如果登录了就只改地址」「按目录批量重建模型表」「旧版少写键」，静态的键值行表达不了控制流；
- 它的简单户写起来确实就是几十行声明——**说明简单户本来就该数据化**，magpie 只是让数据住在 Go 代码里。

---

## 4. 提案（鼠鼠思路 + 现状对齐后的形状）：总台账 + 两类执行器

一张**接线总台账**，20 家每家一行（含探测信息的引用），行里的「接线形态」决定走哪条路：

```
WiringEntry {
  agent: "goose",            // 对齐 SPECS 的 id
  entry: Adapter("goose")    // 形态一：指到专用适配器（6 家复杂户）
       | Generic {           // 形态二：表驱动，直接走 custom_agents 引擎
           format: Env,      // json | toml | env
           paths: [...],     // 文件路径（复用 SPECS 的 dir/file）
           keys: { base: "OPENAI_BASE_URL", auth: "OPENAI_API_KEY", model: "OPENAI_MODEL" },
           base_suffix: "/v1",
           verified: true,   // 键名有出处才能 true
           source: "magpie goose.go",   // 出处
         }
       | Unverified { ... }  // 形态三：键名没背书 → 不出现在「可接入」，只给手动通用接入
}
```

- **Adapter 行**（claude-code / codex / zcode / gemini / opencode / kimi）：表只做索引，行为零变化；
- **Generic 行**：探测命中即「可接入」，接线直接复用 `custom_agents.rs` 的 `plan_wire` / `plan_unwire` 引擎——**不用新写引擎**；
- **Unverified 行**：如实显示「键名未核实」，引导用户走通用接入核对，不猜。

**预期覆盖**（待逐家核实后定稿）：20 家里约 8–10 家可进 Generic（goose / continue / droid / qwen / cline / vscode / zed / crush 这类写两三个键的），6 家 Adapter，剩几家 Unverified。

---

## 5. 疑难点（请同事重点研究，逐条给结论）

1. **引擎现成性验证**：`custom_agents.rs` 引擎按「一个文件、三个键」设计。逐家核对 8–10 家简单户的真实配置结构，确认没有「要写进数组元素 / 嵌套重建 / 多文件」的漏网（如 continue 的 `models[0].apiBase` 是数组——`set_key` 对数组路径的支持现状如何？zed 的键在深层嵌套对象里？）；
2. **键级还原 vs 文件级快照**：custom_agents 引擎是键级快照（好，magpie 同款），但它**没有整文件快照**——如果写入后文件被写坏/用户大改，还原是否仍可靠？要不要给 Generic 行也加整文件备份（`commit_plan` 的 backup）；
3. **多文件工具**：gemini / zcode 现在是 Adapter，但如果未来某家简单工具也要写两个文件，Generic 的表行要不要支持 `paths` 复数；不支持的话在文档里明确「多文件 = 必须 Adapter」；
4. **wired / drift 判定统一**：Generic 行的「已接入」判定现在是 `agent_wiring` 表 + applied 比对，与 Adapter 的 `ours_with`（看文件现值）口径不同。总台账方案要统一口径，否则列表态的状态灯会两套标准；
5. **verified 的治理**：出处（官方文档 / magpie 源码）记在哪、谁背书、工具改版键名漂移怎么发现（magpie 的伤疤全是这个）。建议：Generic 行必填 `source` 字段 + drift 检测天然会暴露键失效；
6. **两处同步收敛**：现在加一家要动 `wiring.rs::AGENTS`（前端 `WIRABLE_AGENTS`）+ `db.rs` 种子表。总台账方案下能否让 AGENTS 清单从台账派生（宏或 build 时检查），种子表从 AGENTS 派生，把「三处同步」收敛成「一处声明」；
7. **用户自定义工具与总台账的关系**：`custom_agents`（用户手加的）和总台账（官方预设）共用一个引擎后，UI 上两条入口（「添加自定义工具」弹窗 / Agent 卡开关）怎么摆，预设行要不要允许用户覆盖键名；
8. **改造面评估**：方案 B 的实际代码改动量（台账表 + AgentsView 路由到引擎 + 状态口径统一），对比方案 C（维持现状按家写适配器）逐家半天到一天的成本，给出切换阈值。

---

## 6. 三个候选方案（研究的起点，不是终点）

| | A · 纯总表 | B · 总台账 + 双执行器（提案） | C · 维持现状 |
|---|---|---|---|
| 简单户接入成本 | 一行数据 | 一行数据 | 每家半天+ |
| 复杂户（kimi / claude / codex…） | **表达不了，无法接入** | 走 Adapter，行为不变 | 各自手写 |
| 死区消除 | 部分能 | 有出处的全覆盖 | 靠逐家手写，慢 |
| 还原安全性 | 靠通用引擎（需验证） | 键级快照 + 外部意图优先（已实现） | 已实现 |
| 改造量 | 最小但天花板低 | 台账重构 + 状态口径统一（中等） | 零 |
| 风险 | 复杂户永远进不来 | 口径不统一会出现两套状态灯 | 每家都是新代码新坑 |

---

## 7. 验收标准（无论选哪个方案）

1. magpie 三关全过：接入前快照、断开完整还原（只还原仍是 Jolu 写的键）、外部改动可被发现；
2. 现有 6 家真机往返零回归（接入 → 验文件 → 断开 → 验还原）；
3. 键名有出处的工具不再出现「暂无专用接入」死区；没出处的如实标「键名未核实」并引导手动核对；
4. 新增一家简单工具 ≤ 一行数据 + verified 出处引用；
5. 「每加一家的同步点」有测试锁死（现有 FK 回归测试保留）；
6. UI 状态灯单一口径：可接入 / 已接入 / 暂不能接入的判定来源唯一。

---

## 8. 决议记录（研究后填）

| 日期 | 决定 | 参与人 | 备注 |
|---|---|---|---|
| 2026-11-17 | 暂不建「简单户」总台账；先按专用适配器补接 Pencil、Qoder、Qoder CN、Crush（`backend/src/gateway/wiring_extra.rs`） | 鼠鼠授权「全部完成」，助手按证据定（待确认） | 逐家核对 magpie `internal/agent/*.go`：探测表里 14 家未接线的工具**没有一家**能用现有字符串键引擎表达——要写 yaml（goose、dsh）、往数组插项（qwen、droid、copilot、vscode、workbuddy）、写多个文件（cline、copilot、vscode、goose、dsh）、整段写含 models 数组的 provider 对象（crush、qoder、pencil、zed），或密钥在系统凭据库（zed）；Continue 在 magpie 里没有实现。离简单最近的四家都只改一个 JSON 文件、provider 在固定的 `providers.jolu` 下，于是照 Kimi 的方式做成专用适配（共用网关模型目录、快照只记选择键与原有同名段、断开时选择键仍指向 Jolu 才还原）。台账（方案 B）等出现第一家真正的简单户再建。自定义工具预设里 Droid（数组）与 Zed（凭据库）按核对结果删掉，Kimi / Crush 有专用接线后也不再给预设。未做真实工具验收 |
| 2026-11-18 | dsh 破例按专用适配器补接（`backend/src/gateway/wiring_dsh.rs`），从「暂不接」名单移除 | 鼠鼠直接要求「dsh 本机有但是没适配你进行适配一下」 | 多文件 + 行级 YAML 的第一家：每个 profile 的 cordis.patch.yml（行级拼接保注释，用户条目与 hualong 块零改动）+ `.env` + credentials refs。生成形状逐字段对齐 magpie dshKindConfig（baseURL 带 /v1、maxTokens 走压缩公式 window/2−65536、input 用 flow 数组、无档位数据不写 reasoningEfforts）；v1 只写 openai-completions 一个路由（magpie 按协议分三个，Jolu 目录暂无 API 信息）。真机以副本 + DSH_HOME 隔离验证接入/断开往返（写 2 profile + .env + credentials，断开 0 残留、用户 MCP 与 hualong 原样） || 2026-11-17 | 暂不建「简单户」总台账；先按专用适配器补接 Pencil、Qoder、Qoder CN、Crush（`backend/src/gateway/wiring_extra.rs`） | 鼠鼠授权「全部完成」，助手按证据定（待确认） | 逐家核对 magpie `internal/agent/*.go`：探测表里 14 家未接线的工具**没有一家**能用现有字符串键引擎表达——要写 yaml（goose、dsh）、往数组插项（qwen、droid、copilot、vscode、workbuddy）、写多个文件（cline、copilot、vscode、goose、dsh）、整段写含 models 数组的 provider 对象（crush、qoder、pencil、zed），或密钥在系统凭据库（zed）；Continue 在 magpie 里没有实现。离简单最近的四家都只改一个 JSON 文件、provider 在固定的 `providers.jolu` 下，于是照 Kimi 的方式做成专用适配（共用网关模型目录、快照只记选择键与原有同名段、断开时选择键仍指向 Jolu 才还原）。台账（方案 B）等出现第一家真正的简单户再建。自定义工具预设里 Droid（数组）与 Zed（凭据库）按核对结果删掉，Kimi / Crush 有专用接线后也不再给预设。未做真实工具验收 |
