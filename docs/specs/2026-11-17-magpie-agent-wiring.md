# magpie 的 Agent 接线机制拆解报告

日期：2026-11-17
对象：magpie（`D:\develop\_research\magpie`，commit `e9dd8e8`）里「把本机编码工具接上网关」的整块实现
用途：Jolu 对齐与后续按家补接线的参考。本文是调查记录，执行前仍要到源码核对（行号会漂，以函数名为准）。

---

## 0. 一句话总览

**magpie 的接线不是「一个写入函数」，而是三层结构：每工具一个声明式写入器 + 两份 JSON 记账文件（stash = 断开还原，applied = 漂移检测）+ 统一的状态机（Wired / Drift / Check）。** 用户在界面上做的每一个动作（接入开关、模型选择、断开）都归结为「设置某个字段」，由字段自己的 Set 闭包去写文件，写完记账，断开时按记账还原。

---

## 1. 分层结构（自上而下）

| 层 | 位置 | 职责 |
|---|---|---|
| GUI HTTP API | `internal/gui/api.go` | `POST /api/set`（设字段 = 接线）、`GET /api/drift`（逐工具漂移）、`/api/agents/rescan` 等 |
| Agent 抽象 | `internal/agent/agent.go` | `Agent{ID, Fields, Bin, Dir, Path, Check, Notice, Sync, …}`；`Pick / Connect / Wired / Drift / Values` 统一编排 |
| 每工具写入器 | `internal/agent/<id>.go`（30 家） | 只声明「写哪个文件的哪几个键、怎么还原」，如 `kimi.go` / `claude.go` / `codex.go` |
| 原子写 | `internal/edit/` | `edit.Atomically(fn, paths…)`、TOML/JSON 键级读写（`SetTOMLTable` / `GetTOMLTable` / `DelTOMLTable`），注释与格式保留 |
| 记账 | `internal/agent/stash.go` + `applied.go` | `stash.json`（断开还原）+ `applied.json`（漂移检测），都在 provider 配置旁，0600 |

**关键设计**：`atomic()`（`agent.go:733`）把每个 Agent 的 `Field.Set` / `Sync` / `Unwire` 全部包上 `edit.Atomically`——写入器本身不用考虑原子性，写一半崩了由这层兜住。

---

## 2. 「接入」在 magpie 里到底是什么

**没有独立的「接线按钮后端」——接线 = 把某个字段的值设成 magpie 的模型（或 magpie 本身）。** 三个入口殊途同归：

1. **接入开关** → `Connect()` → `ConnectHow()`：给主字段选一个 magpie 模型并 `Set`；
2. **模型选择器**（Agents 页）→ `POST /api/set` → `Pick(key, v)`：**未接入时选 magpie 模型会先自动接入再切模型**（`applied.go:436`）；
3. **断开** → 字段设回用户自己的模型（或 `""`）→ 各工具的还原逻辑删掉 magpie 写的键。

### 2.1 Connect 的完整流程（`applied.go` connect()）

```
1. forget(ID + ".disconnected")            ← 清「用户主动断开过」标记
2. stash(ID + ".connect.was", 全部字段现值) ← 断开时兜底还原用（Goose 这类没有快照就靠它）
3. reconnect(now)                           ← 「断开又重接」：按上次断开前的值原样恢复（How="again"）
4. Join()（有则试）                          ← 已登录自有账号的照旧保留自有模型（Codex ChatGPT 登录，How="joined"）
5. 选字段与模型（两轮扫描）：
   pass 0：字段 Options 里第一个带 Ref 的（= magpie 目录模型）
   pass 1：Options 里值 == magpieID 的（= 「以 magpie 为供应商」型 app）
6. 选不到 → noModelLeft()：
   · 目录全空 → NoModelsError（"先在 magpie 加供应商再接入 X"）
   · 有模型但被清单隐藏光 → AllHiddenError（提示去模型清单勾回来，#1989）
7. 设字段 → 各工具 Set 闭包写文件 → applied.json 记账
```

### 2.2 选模型的 8 种「怎么选的」（`Connection.How`，`applied.go:490`）

`kept / again / joined / magpie / same / alike / default / first`——接入后 UI 会说明「为什么给它选了这个模型」（#726：Codex 显示上次的选择、Claude Code 显示 "a Sonnet through magpie"，用户不该看不懂为什么）。

**对 Jolu 的启示**：Jolu v1 接入 Kimi 直接取目录第一个模型（`kimi_wire` 的 `models[0]`），magpie 是「正在用的模型 magpie 也有的 → 用它（same）；是别名 → 解析（alike）；都没有 → 第一个（first）」。这是可对齐的体验差距。

---

## 3. 两份记账文件，各管一件事（最容易混的部分）

### 3.1 `stash.json` —— 断开时「还你原样」

- 记的是**接入前的用户原值**，键名如 `kimi:<config路径>:default_model`；
- `stashMu` 互斥 + 整文件原子写（两个并发改动不会互相覆盖）；
- 文件被手改坏（解析不了）→ **先拷贝存档再覆写**（`lastgood.KeepUnparsed`），绝不悄悄丢；
- 权限 0600（里面是键值，可能含敏感信息）；
- kimi 还有一份**按表记账**：`kimi:<path>:wrote` = `{模型表名: {键: 上次写的值}}`（`kimiWrote`），供 `kimiOwn` 区分「用户手改的 display_name / max_output_size」与「magpie 上次写的」——用户手改的永远保留。

### 3.2 `applied.json` —— 漂移检测的基准

- 记的是 **magpie 上次写下的值**（agent id → `{At, Fields}`）；
- `Drift()`（`applied.go:162`）：把「applied 记的值」和「配置文件现值」对比——**别的工具/人改走了 magpie 的接线**就标出来，并给「重新应用」（`/api/reapply`）入口。注释说得很清楚为什么必须有它：*“配置文件谁都能写：另一个切换器、安装器、工具自己的 setup。有人把 magpie 改走了，行上还显示着 magpie 模型、请求却发给它自己的厂商，用户会怪 magpie。”*
- `record()`：每次 Set 后记录；字段设回默认值（`""`）= magpie 不再管它，从记录里删掉；
- `Keep()`：彻底忘掉一个 agent 的 applied。

**Jolu 对照**：Jolu 把这两份合在了 sqlite 的 `agent_wiring` 表里（`original_json` ≈ stash，`applied_json` ≈ applied，`token`/`instance_id` ≈ 多实例归属，`state` = wired/idle/pending）。语义对齐，载体不同——**而这次 Kimi 接入失败的根因恰好在 Jolu 这层**：`agent_wiring.agent_id` 外键引用 `agents` 种子表，种子清单漏了 kimi，导致「文件都写好了、记录存不进去」。magpie 用 JSON 文件记账，天然没有这类种子表同步问题；Jolu 以后每加一个可接线 agent，`db.rs` 的种子行必须同步补（已加回归测试 `every_wirable_agent_has_a_seed_row_for_the_wiring_foreign_key`）。

---

## 4. 状态机：Wired / Drift / Check / Notice / Sync

| 判定 | 实现 | 说明 |
|---|---|---|
| `Wired()` | `applied.go:415` | **以文件现值为准**：任一字段值 == magpieID，或是解析后指向 magpie 的值（`magpieValue`），或路由分组（`Routed`） |
| `Drift()` | `applied.go:162` | applied 记录 vs 文件现值逐字段对比，`checkDetailed` 说明哪里变了 |
| `Check()` | 每工具闭包 | 比 Wired 更细的健康检查：kimi 检查「default_model 指向的表还在吗」「providers.magpie 表还在吗」「base_url/api_key 还对吗」，坏了说清是哪一行丢了 |
| `Notice()` | 每工具闭包 | 该工具 CLI 正在运行时提示「要重启才生效」（`Running(kimicli)` 正则探测） |
| `Sync()` | 每工具闭包 | 配置还是 magpie 的但内容旧了（如目录更新）→ 整体重写一遍我们写的键 |
| `Disconnected()` | stash 标记 | 用户主动断开后不再自动重连 |

---

## 5. kimi 具体例（`internal/agent/kimi.go`，Jolu 本次实现的母本）

**文件**：`~/.kimi-code/config.toml`（新版 Kimi Code，`KIMI_CODE_HOME` 覆盖）；`~/.kimi`（旧版 Python kimi-cli，`KIMI_SHARE_DIR` 覆盖）。新版目录存在时旧的不再读（kimi-cli 1.50 起移交，#290）。

**写入面**（`writeMagpie` = `kimiProvider` + `kimiModelTables` + `setDefault`）：

1. `providers.magpie`：`type="kimi"`、`base_url=<网关>`、`api_key=<网关Key>`——**表已存在就只改这三个键**（`kimiProvider`），用户自加的 `custom_headers`、env、挂在 provider 上的 default_model 不动（#2030）；
2. 模型表：**先整批删掉所有 `models."magpie/…"` 表再重写**（目录改名/停用不留残表），每张表：
   - `provider = "magpie"`、`model = <目录模型id>`（Kimi 发请求用的是这个字段，网关按它路由）、`max_context_size`（缺省 128000，Kimi 靠它决定何时压缩会话）；
   - `display_name`（目录别称才写；**用户手改过的优先**，靠 kimiOwn + wrote 记账判断）；
   - `capabilities`：`thinking`（有推理档位时）/ `image_in`（收图）/ `tool_use`（**仅新版**，旧版写了拒绝启动）；
   - `max_output_size`（仅新版；用户设了非整数按没设处理，`kimiInt`）；
   - `support_efforts` / `default_effort`（仅新版推理模型；用户手设的 effort 在档位还在时保留，`keptEffort`）；
3. `default_model`：指向 `magpie/<模型id>`（= 表键）。

**还原**（`dropMagpie` + Set("") 分支）：删全部 magpie 模型表 + provider 表；default_model 还原 stash 里的原值——**前提是那张模型表还在**（`ownModel`，否则写回就是悬空引用）。

**kimiOwn 语义**（`kimi.go:136`）：`was`（表现值）≠ `wrote`（上次 magpie 写的）→ 是用户手改的，保留；否则用 magpie 的值。

---

## 6. 防呆设计清单（照抄即对齐）

| 防呆 | magpie 实现 | Jolu 现状 |
|---|---|---|
| 别人改走了接线 | applied.json + Drift + 重新应用 | `status_with` 的 drift（foreign/removed/changed/model_changed）✓ |
| 另一个实例接管 | 令牌带实例后缀，`Check` 认得出 | `foreign_marker`（尾号不符即 foreign）✓ |
| 用户手改保留 | stash 的 wrote 记账（kimiOwn） | 简化为「表里已有值一律保留」（差异已注释；目录改名后旧显示名留到断开重接） |
| 悬空引用 | ownModel：原模型表不在就不写回 | kimi_restore 同款判断 ✓ |
| 配置写坏 | lastgood.KeepUnparsed：坏文件先存档再覆写 | Jolu 在 `commit_plan` 先整文件备份，写坏可回滚 ✓ |
| 空目录拒绝 | NoModelsError / AllHiddenError 分两种说清 | 「网关模型清单为空；先在模型池启用…」✓（未分两种） |
| CLI 在跑要重启 | Notice() 探测进程 | Jolu 用静态文案「需重启 X 后生效」 |
| 写一半崩溃 | edit.Atomically 包住一切 | write_atomic 临时文件 + rename + 失败回滚 ✓ |
| 发请求的模型名 | 表内 `model` 字段（前缀不出本机） | 同款：`jolu/` 只做本地命名空间 ✓ |

---

## 7. Jolu 对齐状态与剩余差距

**已对齐**（`wiring.rs` 的 kimi 分支 + 真机往返验证通过，2026-11-17）：
providers.jolu 就地更新、模型表整批重建、旧版少写键、快照还原 + ownModel、外部意图优先、多实例令牌、写入预览（`gateway_wiring_preview`，magpie 没有对应物，是 Jolu 多出来的）。

**可继续对齐的差距**（按优先级）：
1. **选模型的 How**：接入时优先「正在用的模型 magpie 也有的」（same），其次解析别名（alike），最后第一个（first）——现在是直接第一个；
2. **kimiOwn 级别的保留**：用 applied 记录区分「上次写的」和「用户改的」，目录改名后能跟上；
3. **Notice() 进程探测**：接入时该工具 CLI 正在跑，明确提示要重启；
4. **其余工具的专用接线**：magpie 30 家写入器里，Jolu 20 家内还有 **qwen / cline / droid / dsh / zed / vscode / goose / qoder / workbuddy** 有现成参考可搬（continue / crush / pencil / copilot 在 magpie 也只有探测、无接线，Jolu 卡上显示「等待接入适配」是对的）。

---

## 8. 本次 Kimi 接入失败的根因记录（防再犯）

- 现象：真机点接入报「保存接线记录失败：FOREIGN KEY constraint failed」；
- 根因：`db.rs` 的 `agents` 种子表只插了老五家，`agent_wiring.agent_id REFERENCES agents(id)`，kimi 不在种子表 → 文件计划算好了、记录存不进去；
- 修复：种子行补 kimi（`INSERT OR IGNORE` 每次开库都会跑，老库自动补上）+ 回归测试锁死「每个可接线 agent 必须有种子行」；
- 教训：magpie 的记账是 JSON 文件，没有种子表这一层；Jolu 用 sqlite，**每加一个 agent 有两处要同步**——`wiring.rs::AGENTS`（开关清单）与 `db.rs` 的 agents 种子（外键），回归测试已把后者锁住。
