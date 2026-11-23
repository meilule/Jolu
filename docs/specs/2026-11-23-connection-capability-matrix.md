# 连接能力矩阵（P2-1）

日期：2026-11-23
状态：**代码事实核查。额度查询 / 模型发现 / 实际转发三列按代码路径核实；「真机」列区分已验证与未验证。**
配套计划：[产品对齐执行计划 §P2-1](../plans/2026-11-23-product-alignment-execution-plan.md)

---

## 0. 三列各测什么

| 列 | 含义 | 判定依据 |
|---|---|---|
| **额度查询** | 能拉到官方额度窗口 | `QuotaWindow` 列表有实际来源（`platformCatalog.json` 的 `subscription_quota` + 适配器实现） |
| **模型发现** | 能从上游实时拉模型清单 | `commands/providers.rs` 的 `match platform` 有分支；无分支的走内置清单或本地配置 |
| **实际转发** | 本连接能被选中并发出请求 | `gateway/wire.rs` 的订阅分发 `match platform` 与各适配器的 `bound()` 校验 |

**三列互相独立**：有额度不等于能转发（Grok 只有设备授权、WorkBuddy AI 首轮 401）；能转发不等于能发现模型（Claude Code / Codex 用固定清单）。

---

## 1. 连接类型 × 三能力

`api` = 用户填的 API Key；`auth` = 官方订阅 / 本机账号。

| 平台 | 额度查询 | 模型发现 | 实际转发 | 备注 |
|---|---|---|---|---|
| **claude** | ✅ 订阅额度（5h / 7d） | — 固定清单 | ⚠️ 仅 Anthropic 协议 | `forward_one` 订阅分支的协议检查对它直接 400；订阅适配器 `adapter/claude.rs` |
| **codex** | ✅ 订阅额度 | — 固定清单 | ⚠️ 仅 OpenAI 协议 | `forward_one` 订阅分支的协议检查对 Anthropic 请求直接 400 |
| **gemini** | ✅ `adapter/gemini_subscription` | ✅ `gemini_subscription::models` | ✅ | 原生 `/v1beta` + Chat/Responses/Anthropic 三协议 |
| **opencode** | ✅ | — 固定清单 | ⚠️ 仅 OpenAI 协议 | 复用 codex 适配器链 |
| **zcode** | ✅ | ✅ `adapter/zcode::models` | ✅ | 双通道（个人套餐 / Start Plan） |
| **grok** | ✅ | ✅ `adapter/grok::models` | ✅ | 设备授权初始化成功过，真实生成未验 |
| **qoder** | ✅ 双版 | ✅ `adapter/qoder::models`（富缓存 + display_name） | ✅ | 未真实账号验收 |
| **qoder-cn** | ✅ 同 qoder | ✅ 同上 | ✅ 同一适配器 | 独立入口缺失（能力矩阵 §4.5） |
| **trae** | ✅ 双版 | ✅ `trae_models::models` | ✅ | Max 派生模型 |
| **workbuddy** | ✅ 双版 | ✅ `workbuddy::models`（富缓存） | ✅ | CN 真实 200 通过；AI 国际首轮 401 |
| **custom**（API Key） | — 无官方额度 | ✅ 上游 `/models` | ✅ `adapter/direct` | 自带 Key，无额度概念 |

### 1.1 三个不对称点

1. **claude / codex / opencode 的协议限制是有意的**：它们各自的适配器只能翻译一种协议，网关在 `wire.rs` 里直接拒掉不匹配的调用方，而不是悄悄转错。这是「明确失败」而非「静默降级」。
2. **模型发现只有 6 家有实时拉取**：其余走 `platformCatalog.json` 的内置清单或本地配置（Claude Code / Codex 的模型由 CLI 决定，不是账号决定的）。
3. **额度快照的字段是「任一有值就记」**（F-2）：重置时间与剩余量任一存在就写，两者都缺才不留。只给剩余量不给重置时间的来源，`pace` 能用、`smart` 用不了——`smart` 需要重置时间才能排。
4. **qoder / trae / workbuddy 的模型清单是「富缓存」**：适配器自己写 `models_json`（含 `display_name` 与凭证指纹），`commands/providers.rs` 里富缓存不覆盖的判断（`matches!(platform, "qoder" \| "trae" \| "workbuddy")`）特意不为它们覆盖——否则会砸掉显示名并让指纹校验失败。

---

## 2. 按代码核实的限制

| 平台 | 限制 | 出处 |
|---|---|---|
| claude | 仅 Anthropic Messages 协议 | `forward_one` 订阅分支的协议检查 |
| codex | 仅 OpenAI 协议（Chat / Responses） | `forward_one` 订阅分支的协议检查 |
| opencode | 仅 OpenAI 协议 | 同上 |
| gemini | 检测只认命令行（`Detect::CliOnly`），因 `~/.gemini` 也被 Antigravity / Code Assist 使用 | `agent_discovery.rs:337` |
| qoder-cn | 会话 / Skills 查询挂在 `platform="qoder"`，`qoder-cn` 无独立入口 | 能力矩阵 §4.5 |
| workbuddy | Skills 只读 | `skills.rs:135` |

---

## 3. 验证强度（与代码分开）

| 平台 | 额度查询 | 模型发现 | 转发 |
|---|---|---|---|
| zcode | ✅ 真实（本机 18,872 条对账） | 未验 | 未验 |
| workbuddy CN | ✅ 真实 | ✅ 真实 | ✅ 真实 200（窄场景） |
| qoder CN | ✅ 真实只读 | ⚠️ 本地只读 | 未验 |
| trae CN | ✅ 真实 | 未验 | 未验 |
| claude | ✅ 真实读取 | — | 未验（缺 OAuth 样本） |
| codex | ✅ 真实读取 | — | 未验 |
| opencode | ⚠️ 本机 0 凭证 | 未验 | 未验 |
| gemini | ⚠️ 本机未登录 | 未验 | 未验 |
| grok | ⚠️ 仅设备授权初始化 | 未验 | 未验 |

**真实转发的整条链路（连接 → 选路 → 工具调用 → 用量）只有 WorkBuddy CN 窄场景一份记录。** 这是 P2-1 的核心缺口，且需要真实账号才能补——按计划 §3 记为样本缺口。

---

## 4. 本轮（P2）查出的问题

### 4.1 `pace` 策略名不副实（已修）

- **问题**：`quota_snapshot::Snapshot` 只存 `resets_at_ms` + `exhausted` 布尔，没有剩余量。于是 `pace`（语义是「剩余最多先用」）只能退化成「重置时间最晚先用」，代码注释自己写着「magpie pace 的近似」。
- **后果**：A 刚重置但已烧掉 90%、B 两天前重置但只用 30% 时，按重置时间会先选 A——正好选错。
- **修复**：`Snapshot` 增加 `remaining_percent`（取**所有窗口里最紧的那个**的 `100 - used_percent`，木桶效应），并增加 15 分钟有效期；`ordered_members` 新增 `remaining_of` 参数，`pace` 按真实剩余量降序排，缺数据的成员如实垫底并说明。
- **测试**：`pace_orders_by_remaining_headroom_not_reset_time`、`pace_does_not_fall_back_to_reset_order_when_headroom_disagrees`、`pace_puts_members_without_headroom_last_and_says_so`、`remaining_takes_the_tightest_window_not_the_average`、`a_stale_snapshot_stops_reporting_remaining` 等 6 条。已验证：把 `pace` 改回按重置时间排序，后两条立刻失败。

### 4.2 `trae` 不是会话来源，而是登录凭据来源（澄清，非缺陷）

初看 `native_sources.rs:50-51` 建了 Trae 的 `Source`，会误以为是会话采集源。实际：

- `collects_sessions()`（`:1036`）严格要求 `local_sessions` 能力，而 `platformCatalog.json` 里 trae 没有这一项——所以它确实**不参与会话采集**，这是正确的。
- 那两个 `Source` 的用途是 `validate_source`（`:232`）→ `trae_quota::validate_source`（`trae_quota.rs:440`），只读本地 `auth` 判断登录是否有效。
- 也就是说：**Trae 在本机的数据是「登录凭据 + 额度」，没有会话记录**。这与能力矩阵 §1 把 trae 的会话列标为「—」一致。

不需要改动。这条写下来是为了避免下次再把「凭据来源」误读成「会话来源」。

---

## 5. 结论

- **三能力的代码覆盖：额度 11/12（custom 无额度概念）、模型发现 7（6 家实时 + custom 上游）、转发 10/11（custom 属 api 分支）。**
- **真实链路验证只覆盖 WorkBuddy CN 窄场景**，其余平台的转发均无真实生成记录。这是 P2-1 的验收缺口，不是实现缺口。
- 本轮修掉了 `pace` 的语义错误；`trae` 的声明不一致需要先确认产品意图。
