# 管家对齐 · 未完成工作接手清单

**日期：** 2026-11-13
**给谁：** 接手 magpie 对齐剩余工作的同事。**本文档就是要改的交接文档**：完成某项后，更新本文档对应条目的状态，并在 §10（spec 文档）同步勾选。
**前置阅读：** `docs/specs/2026-11-12-magpie-claude-agent-detail-parity.md`（背景拆解与实现依据，§10 是完成状态总览，§9 是定位与取舍——本文只列「还没做的」，不重复理由）

**定位口径（接手前先对齐）：** Jolu 是 AI 工具的**管家**——持有账号生命周期（登录 → 保存 → 额度 → 续期），magpie 只借凭据转发。每补一项，先做到「不出错」再补下一项；动用户配置必须可还原；未知值不冒充（AGENTS.md 硬约束）。

---

## 一、已完成（不要重做）

| 项 | 说明 |
|---|---|
| A1/A2 agent 安装探测与状态分层 | `backend/src/agent_discovery.rs`；接线弹窗已置灰未安装项 |
| 9.4 全协议互转（3×3 矩阵） | `translate.rs` / `wire.rs`；mock 上游端到端验证过 |
| 9.6 插件市场 | 鼠鼠决定**不做**（替代：预设库 + 通用接入） |
| P0-1 B1/B2/B3/B7 + B5/B6 后端 | 2026-11-13 完成：`wiring.rs` 的 `ClaudeExtras` / `claude_extra_env`（最大输出、上下文窗口、长对话压缩、全局能力声明、非 Claude 主模型写 `CLAUDE_CODE_AUTO_MODE_SERVER=0`）；数值取 models.dev 缓存 → 内置快照，缺失不写并记日志（按 D2 倾向执行）；`Applied.extra_env` 记账、按键排除自己写的值；Fable 档跟随别家主模型时留空；长对话设置 `claude_long_context` + 命令 `gateway_long_context_set`。往返测试覆盖写入 → 读回 → 断开还原 |
| P0-2 B8a + B8b | 2026-11-13 完成：`status_with` 回读 `settings.json`（`claude_read_model`，magpie `claudeMain` 优先级链 + 别名解析），与上次写的主模型不同时 `drift = "model_changed"`，返回 `current_model` / `current_resolved`；接线入口在主模型或派生值变化时不再被当成「无需重写」 |
| P2-3 F6 后端 | 2026-11-13 完成：接线拆成 `plan_wire` + `commit_plan`、断开拆成 `plan_unwire`；新模块 `wiring_diff.rs`（逐行 LCS、折叠未改动段、键数统计、密钥遮挡）；命令 `gateway_wiring_preview`（已接入 = Jolu 改了什么；接入前 = 将要写成什么）。前端按 R2 / V5 画板接入 |
| R2 / V5 界面（F1、F2、F5、F6 前端，B8a 界面，A 段前端） | 2026-11-13 完成：鼠鼠指定按 Jolu.pen「统一网关 · R2 重设计」的 V5 实现。导航拆成「Agent」「模型路由」两页；Agent 页含档位行 `/model` 提示（F1）、子代理没选模型时强度置灰（F2，档位行同理）、长对话三态（F5）、写入预览与查看差异（F6）、`/model` 被换过时「以配置为准」（B8a 界面）、未检测到 / CLI 未找到分组（A 段）；模型路由页含概览四项指标（`gateway_day_overview`，按请求计、改道算一次）、对外访问、只读模型池；供应商加「模型路由」区块（V5-5）。仍待确认：F3 按 Agent 裁剪菜单、F4 新会话强度（后端都还没有）、改端口命令、Key 表格样式、深色稿 |
| P2-2 额度主动通知 | 2026-11-13 完成：`notify.rs` 的 `quota_alert` / `quota_should_alert` / `check_quota`，在 `finish_quota_query` 收尾时判定：任一窗口用到 80%（将尽）、登录失效、连续 2 次查询失败；同一账号同一种 24 小时内不重复，恢复正常后再跌破立即提醒；网关观测值不触发。设置 `notify_quota_alerts`（默认开，通知设置页一行，无画板标「待确认」）。点击通知定位账号 |
| 遗留：脱敏改坏中文请求体 | 2026-11-13 查明**不是脱敏的问题**：脱敏对中文 JSON 往返无损（`mask.rs` 的 `non_ascii_tests`）；真机现象来自调用方发了非 UTF-8 字节（Windows PowerShell 5 直接发中文字符串）。网关现在对这种请求体明确报「请求体不是 UTF-8 编码」 |

---

## 二、待办清单（按优先级，P0 最先）

### P0-1 · Claude env 写入最小闭环（B1 + B3 + B2 + B7，约 2 天）

**要解决：** 第三方模型回复被 Claude Code 默认的 32000 截断；1M 窗口模型被当 200K；Pro 账号被 fable 档误标触发 usage credits。

**做什么（全在 `backend/src/gateway/wiring.rs` 的 `claude_wire`，约 227 行的 env 写入段）：**

| # | 键 | 写入值 |
|---|---|---|
| B1 | `env.CLAUDE_CODE_MAX_OUTPUT_TOKENS` | 主模型的最大输出 tokens |
| B2 | `env.CLAUDE_CODE_MAX_CONTEXT_TOKENS` | 主模型的上下文窗口 |
| B3 | fable 档跟随主模型时**不写** `ANTHROPIC_DEFAULT_FABLE_MODEL`（删除键） | — |
| B7 | `env.CLAUDE_CODE_AUTO_MODE_SERVER` = "0" | 上游模型不支持 auto review 时 |

**实现要点：**
- B1/B2 的数据从 `crate::gateway::models_dev`（models.dev 元数据）读；缺的模型回退到 `models_dev_builtin.json`；都没有就**不写并记日志**，不猜值（D2 决策：已倾向 models.dev）
- 写入的键记进快照（`wiring_store::Applied` 加字段），断开时还原成接线前的样子；用户自己写的值**不许动**（照 magpie 的 `ours` 判定：值等于 magpie 上次写的才算自己的，见 spec §3.4）
- B3 只影响「跟随主模型」状态的 fable 档；用户自选（tier_own）的照写

**验收：** 接 GLM/Kimi 后让 Claude Code 输出长内容不截断；1M 模型长对话不提前压缩；Pro 账号接线后不弹 usage credits。`cargo test` 往返测试（写入→读回→断开还原）覆盖新键。

### P0-2 · B8a 回读配置（0.5 天）

**要解决：** 用户在 Claude Code 里 `/model` 换了模型，Jolu 界面一直显示旧值。

**做什么：** 接线状态查询（`gateway_wiring_status` 链路）时，**回读** `settings.json` 顶层 `model` 与各档 env 的现值；`model` 与快照里的主模型不一致时，把漂移标出来（`drift: "model_changed"` 一类），跟随档跟着新主模型走，自选档（tier_own）不动。

**参考：** magpie 的 `claudeMain` 优先级链与 `Follow` hook（spec §4.2.1 有完整伪码与行号）。

**验收：** 在 Claude Code 里 `/model` 换模型 → Jolu 下次刷新显示新值；tier_own 档不受影响。

### P1-1 · 通用接入兜底（3–5 天，agent 广度的关键一步）

**要解决：** 用户用的工具不在 Jolu 的 5 个适配里时，现在完全接不了。通用接入让用户**手填配置方式**，把「不支持」变成「你自己填」。

**做什么：**

1. 新表 `custom_agents`（迁移要可恢复，遵守 AGENTS.md）：
   - `id`（`custom-agent:<slug>`）、`name`、`slug`
   - `config_path`（配置文件绝对路径，支持 `~`）
   - `format`：`json` / `toml` / `env`（写入引擎按格式分派；Jolu 已有 JSON/TOML 的编辑能力在 `wiring.rs`，env 是 `KEY=VALUE` 行）
   - 写入映射：`base_url_key`（如 `env.ANTHROPIC_BASE_URL`）、`auth_key`（如 `env.ANTHROPIC_AUTH_TOKEN`）、`model_key`（如 `model`）
2. 探测：接入 `agent_discovery`（`Spec` 表加一条 `detect: 自定义` 分支，按用户填的 `config_path` 判定）
3. 接线：复用 `wiring` 的快照/还原机制——写 `base_url` = 网关地址、`auth_key` = 网关令牌、`model_key` = 用户选的模型；快照记录原值
4. 网关转发：自定义 agent 走通用 token 识别（网关已按 `jolu-<agent>` 前缀识别），**不需要**专门适配器
5. 前端：接线弹窗加「添加自定义工具」入口（表单：名称 / 配置路径 / 格式 / 三个键名），**无画板标「待确认」**

**验收：** 拿一个 magpie 支持而 Jolu 不支持的 CLI（如 Crush），手填三键后能完成接线 → 该 CLI 的请求经网关转发 → 断开还原原配置。

### P1-2 · 逐家 agent 适配（通用接入落地后再排）

通用接入做完后，按用户反馈决定优先适配哪几家（每家 0.5–1 天：探针 + 接线逻辑 + 往返测试）。**不要在通用接入之前铺数量**——那是在沙地上盖楼。

### P2-1 · 账号 + 额度历史备份恢复（1–2 天）

**要解决：** 管家怕丢数据。导出账号凭据（加密）、额度历史、签到记录、自动化任务；导入时校验版本、同账号冲突让用户选保留哪份。**不做实时同步**（多机漫游缓做，见 spec §9.5）。

### P2-2 · 额度异常 / 将尽的主动通知（1–2 天）

**要解决：** 管家该在用户察觉之前提醒。额度环已经有数据（`quota_registry.rs`），缺的是**主动推**：阈值（将尽 20% / 异常 Failed / 失效 Unauthorized）触发系统通知（`notify.rs` 已有通知基建），自动化任务等额度的场景联动提示。

### P2-3 · F6 diff 预览（2 天，前端等画板）

接线弹窗里展示「改动了哪些文件 + 逐行 diff + 断开前预览」。后端在 `wiring` 写入时记录变更文件与键值对，提供 preview 接口；前端照 magpie 的 `ag-diff` 样式（spec §2.5）。

### P3 · 其余（无阻塞，按需排）

| 项 | 说明 |
|---|---|
| B4 haiku 档降级 | 先定 D3（降级目标：同来源最便宜？映射表？），1 天 |
| B5 `CLAUDE_CODE_AUTO_COMPACT_WINDOW` | 长对话压缩阈值，含设置存储，1 天 |
| B6 `CLAUDE_CODE_MODEL_CAPABILITIES` 全局声明 | `/model` 显示 effort 选项，1 天 |
| B8b 别名解析两段式 / B8c `[1m]` 自动标记 | 各 0.5 天，见 spec §4.2.1 |
| F1–F6 详情面板前端 | **鼠鼠已在 Jolu.pen 设计，按稿实现**；B1–B8 是对应后端 |

---

## 三、阻塞决策（找鼠鼠）

| # | 决策 | 阻塞的项 |
|---|---|---|
| D2 | 模型窗口/最大输出的数据来源（已倾向 models.dev，需确认字段覆盖率） | B1、B2 |
| D3 | haiku 降级目标怎么定 | B4 |
| 画板 | 详情面板与通用接入的界面稿 | F1–F6、P1-1 的表单 |

---

## 三点五、逐项改动位置（改哪个文件、改什么函数，接手直接照着定位）

> 行号是 2026-11-13 基线（`c2e1dc2`）的大致位置，函数名为准；动手前先 `grep` 确认。

### P0-1 Claude env 写入（B1/B2/B3/B7）

| 文件 | 位置 | 改什么 |
|---|---|---|
| `backend/src/gateway/wiring.rs` | `claude_wire`（148 行起）env 写入段（约 227 行附近） | 新增 B1/B2 两个 env 的写入：值来自 `models_dev::lookup_meta` 的窗口/输出字段；写不进快照就还原不了，见下一条 |
| `backend/src/gateway/wiring_store.rs` | `Applied`（31 行） | 加 `max_output_tokens: Option<i64>`、`max_context_tokens: Option<i64>` 字段（`#[serde(default)]`，旧快照兼容），断开时这两个键按「我们写的」删除 |
| `backend/src/gateway/wiring.rs` | 档位写入段（fable 分支） | B3：fable 档为「跟随主模型」且主模型非 Fable 家族时**跳过写入并删键**；用户自选（tier_own）照写 |
| `backend/src/gateway/wiring.rs` | `claude_wire` / 各档 `_SUPPORTED_CAPABILITIES` 写入处 | B7：上游模型不支持 auto review 时写 `env.CLAUDE_CODE_AUTO_MODE_SERVER = "0"`，同样进快照 |
| `backend/src/gateway/models_dev.rs` | `lookup_meta`（138 行）/ `builtin`（55 行） | 只读参考：确认窗口与输出字段的名字与覆盖率；缺字段的模型返回 None，调用方不写键 |
| 测试 | `wiring.rs` tests（1506–1760 行有一组往返测试可照抄模式） | 每个新键加「写入→读回→断开还原」三段断言；B3 加「跟随态不写、自选态照写」断言 |

### P0-2 B8a 回读配置

| 文件 | 位置 | 改什么 |
|---|---|---|
| `backend/src/gateway/wiring.rs` | `status_with`（1116 行） | 回读 `settings.json` 顶层 `model` 现值（`edit::GetJSON`），与快照主模型不一致时 `drift = Some("model_changed")`，`reason` 写明「配置里的主模型已被 Claude Code 的 /model 改为 X」 |
| `backend/src/gateway/wiring.rs` | 新增私有函数 `read_current_model(path)` | 优先级链照 magpie `claudeMain`（spec §4.2.1：顶层 model → 档位别名解 env → ANTHROPIC_MODEL → sonnet 档） |
| `frontend/src/components/gateway/GatewayDialogs.tsx` | 行渲染（206 行附近的状态文案） | `drift === "model_changed"` 时显示回读到的现值，并提供「以配置为准」按钮（调既有 acceptWiring） |

### P1-1 通用接入兜底

| 文件 | 位置 | 改什么 |
|---|---|---|
| `backend/src/db.rs` | 迁移列表末尾 | 新表 `custom_agents(id, name, slug, config_path, format, base_url_key, auth_key, model_key, created_ms)`；迁移可恢复（同款表重复执行跳过） |
| `backend/src/custom_agents.rs` | 新模块 | CRUD + 探测（`config_path` 存在即 installed）+ 接线（按 format 分派写入：JSON 走 `edit::SetJSON`、TOML 走 `toml_edit`、env 走行改写）；快照/还原复用 `wiring_store` 的机制 |
| `backend/src/agent_discovery.rs` | `SPECS` 表 | 加「自定义」分支：`discover_with` 里拼上 `custom_agents` 表的条目（dir/file 来自用户填的路径） |
| `backend/src/lib.rs` | 命令区 + invoke_handler | `custom_agents_list/add/remove/wire/unwire` 五个命令 |
| `frontend/src/components/gateway/GatewayDialogs.tsx` | 接线弹窗末尾 | 「添加自定义工具」表单（名称/路径/格式/三键名），无画板标「待确认」 |

### P2-2 额度主动通知

| 文件 | 位置 | 改什么 |
|---|---|---|
| `backend/src/quota_registry.rs` | `route`（31 行）后加判定函数 | 阈值判定：`used_percent ≥ 80` / `state = Failed / Unauthorized` 触发 |
| `backend/src/notify.rs` | `notify_task`（145 行）旁加 `notify_quota` | 复用既有通知基建发系统通知；同一连接 24 小时内不重复提醒（内存去重） |
| `backend/src/lib.rs` | 额度查询命令完成处 | 查询结果落库后调判定 + 通知 |

### P2-3 F6 diff 预览

| 文件 | 位置 | 改什么 |
|---|---|---|
| `backend/src/gateway/wiring_store.rs` | `Applied` 旁 | 新增 `changes` 记录：接线/断开时把改动的键值对（文件、键路径、旧值、新值）存进快照表 |
| `backend/src/lib.rs` | 新命令 `wiring_preview` | 返回「即将发生的变更」与「上次已发生的变更」两份 |
| `frontend/src/components/gateway/GatewayDialogs.tsx` | 接线弹窗底部 | 「改动了 X · 查看」+ diff 列表 + 「断开并还原」前置确认框（照 magpie `ag-foot`/`ag-diff` 样式，spec §2.5） |

## 四、纪律（每项的验收都过这几条）

1. 未知值、真实的零、能力不支持、查询失败、数据过期**分别表达**，不冒充
2. 动用户配置必须可还原；只还原 magpie/Jolu 自己写的键
3. 凭据只在后端；日志与错误文案不出原文
4. 验证强度跟风险匹配：动 `wiring` / 网关的，跑全量后端测试 + 往返测试 + 真机一次
5. 不确定的不编造：数据来源不够就标「未验证」，写明口径（参考自动化「未验证三态」的做法）
