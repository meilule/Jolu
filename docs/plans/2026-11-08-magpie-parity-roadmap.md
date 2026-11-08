# magpie 功能对齐 · 逆向改进路线图（2026-11-08）

**交给：鼠鼠**
**状态：计划稿，按阶段实施；每阶段完成并通过验证后单独提交**
**参照物：** [yetone/magpie](https://github.com/yetone/magpie) @ `627fd1a`（2026-11-08 拉取，本地克隆 `/tmp/magpie`；Go，网关 298 文件、provider 274 文件、agent 适配 148 文件）
**对比全文：** 见本文件附录 A 的差异总表；本文只写「怎么做、风险是什么、怎么规避」。

---

## 0. 为什么要做、总原则

magpie 与 Jolu 核心思路相同（本地网关 + 改写各工具配置 + 统一模型列表），但 magpie 在三处关键设计上更成熟，直接对应 Jolu 目前反复出现的故障：

1. **Claude Code 的模型键体系**：magpie 删 `ANTHROPIC_MODEL`、写顶层 `model` 字段、档位跟随主模型——换模型不用重配 Jolu；Jolu 只写两个键，换模型必须重新接线。
2. **模型 id 解析**：magpie 剥 `[1m]` 后缀、裸模型名按 provider 顺序兜底；Jolu 只认 `<slug>/<model>` 精确匹配，`GLM-5.3-Flash[1M]` 这类名字处处碰壁。
3. **订阅账号的合规处理**：magpie 把 Qoder / WorkBuddy / ZCode / Trae / Grok 等第三方订阅登录**退役出主程序**，移交给社区插件，理由是「直接调用可能违反厂商条款」；Jolu 全部内置，主程序承担全部封号风险。

**总原则（每阶段都适用）：**

- 以 magpie 的**行为**为验收标准，不照抄实现语言；magpie 的 Go 文件是实现参照，不是移植对象。
- 每个阶段独立可发布、可回退；阶段完成跑全量测试后按规范提交（默认 `[skip ci]`）。
- UI 一律先补 `Jolu.pen` 画板或标「待确认」，不凭空造界面（AGENTS.md §1）。
- 涉及认证协议 / 第三方 SDK 的条目，动代码前用 Context7 核对文档（AGENTS.md §4）。
- 工作区另有并行会话在改代码，动手前先 `git status` 核对，只动本阶段文件。

---

## 1. 阶段总览

| 阶段 | 主题 | 条目 | 风险等级 | 依赖 |
|---|---|---|---|---|
| **P1** | 故障根治：模型 id 与 Claude Code 档位 | 4 项 | 中（破坏性变更） | 无 |
| **P2** | ZCode 转发链路简化 + 网关鉴权放宽 | 2 项 | 中（合规敏感） | 无 |
| **P3** | 路由分组与多账号故障转移 | 3 项 | 中 | P1 的 id 解析 |
| **P4** | Provider 预设库 + models.dev 元数据 + 导入 + Profiles | 4 项 | 低 | 无 |
| **P5** | 隐私脱敏 | 1 项 | 中（改动请求体） | 无 |
| **P6** | 网关中间件 + 插件系统 | 2 项 | 高（架构扩展） | P3 |
| **P7** | 工具生态（搜索 / 图像 / Library） | 3 项 | 低 | P6 可选 |
| **P8** | 分发形态（CLI / 共享 / Docker / 同步 / WSL） | 5 项 | 低–中 | P6 |

P1、P2 解决当下「一直不行」的问题，先行；P3 起按鼠鼠排期推进。

---

## 2. P1 · 故障根治：模型 id 与 Claude Code 档位（最高优先）

### 2.1 模型 id 解析对齐（gateway）

**做什么**

- `route::split_model_id` 之上新增统一入口 `resolve_model_id`：
  - 剥离尾部 `[1m]` / `[1M]` 标记并记住（magpie：`provider/models.go` `Resolve()` 第一步 `TrimSuffix(id,"[1m]")`，注释明说这是 Claude Code 的 1M 窗口标记）；
  - 带前缀 `<slug>/<model>` → 按前缀路由（现状保留）；
  - 裸模型名 → 先查模型目录（catalog），命中唯一来源即路由；未命中再落「默认来源」兜底（P1 保留我已加的默认来源，两者并存）；
  - 前缀一半为空（`/model` 或 `slug/`）→ 明确 400，沿用 magpie 文案「expected provider/model with both parts nonempty」。
- 转发层（各 adapter）**移除对模型名必须带 `/` 的二次校验**（zcode 适配器今天已修，其余 adapter 排查一遍）；前缀只用于选路，到上游前剥掉。
- 剥掉的 `[1m]` 在响应侧不需要回填（Claude Code 自己处理），但路由流水、用量统计要按剥后的名字记账，避免同一模型出现两行。

**风险与规避**

| 风险 | 规避 |
|---|---|
| `[1m]` 剥离后上游按 200K 截断输出 | magpie 在**写入工具配置**时补回 `[1m]`（`mark1M`）；网关侧只负责接受两种写法。写入侧在 2.2 一起做 |
| 裸名在多个来源都存在时路由歧义 | 按 provider 顺序取第一个，并在路由流水里记 `resolved_from=bare`；文档说明优先级 |
| 目录未拉取时裸名无解 | 保留默认来源兜底为第二层；「拉取」按钮已存在 |
| 历史流水/用量按旧名字记账，统计分裂 | 记账统一用剥后的规范名；历史行不回改（AGENTS.md：不改历史数据） |

**验证**：`route::tests` 补四类用例（[1m] 剥离、裸名目录命中、裸名默认来源兜底、半截前缀 400）；实测 `GLM-5.3-Flash[1M]`、`GLM-5.3-Flash`、`zcode/GLM-5.3-Flash[1M]` 三种写法都能路由。

### 2.2 Claude Code 档位接线体系（wiring，破坏性变更）

**做什么**（参照 magpie `internal/agent/claude.go` `writeTiers`，约 1749–1810 行）

接线 Claude Code 时按此**完整清单**写 `~/.claude/settings.json`：

1. **删除** `env.ANTHROPIC_MODEL`——magpie 注释原话：它压过 `/model` 的选择，写了模型切换只在本会话生效。（我上一轮加的「接线写 ANTHROPIC_MODEL」就此撤销。）
2. 主模型写**顶层 `"model"` 字段**（不是 env）。
3. 写 `env.ANTHROPIC_SMALL_FAST_MODEL` = haiku 档；`env.ANTHROPIC_DEFAULT_OPUS/SONNET/HAIKU/FABLE_MODEL` = 各档；某档未单独设置时**删键**（跟随主模型）。
4. 每档追加 `ANTHROPIC_DEFAULT_<TIER>_MODEL_NAME` / `_DESCRIPTION` / `_SUPPORTED_CAPABILITIES`（写前先 `dropAbout` 清旧值），让 `/model` 菜单显示正确名称。
5. 1M 模型自动补 `[1m]` 标记。
6. 检测 `CLAUDE_CODE_USE_BEDROCK`、`ANTHROPIC_VERTEX_PROJECT_ID` 等云开关：暂存到 Jolu 快照并**移除**（否则绕过网关），断开时还原；云专属变量（AWS_* 等）不动。
7. 快照/还原清单扩到上述全部键（现有 `claude_snapshot`/`claude_restore` 的键表翻倍），沿用「只清理自己写的键 + 原子写 + 备份」。
8. 若现有 token 是网关会接受的调用方 Key，保留不覆盖（magpie `keyHolds` 行为）。

**界面（需补画板或标待确认）**：接线面板 Claude Code 行改为「主模型」一个下拉；档位默认跟随，展开可单独覆盖每档。

**风险与规避**

| 风险 | 规避 |
|---|---|
| **破坏性**：已接线用户升级后，旧版写的 `ANTHROPIC_MODEL`、我的 `zcode/…` 映射残留在 settings.json | 升级迁移：`wire_agents` 前检测「值是旧版 Jolu 写的」（`jolu-` token + 无实例后缀 / applied 记录在案），按快照清理；清理动作在启动自愈里已有同类先例 |
| 写坏 settings.json 让用户工具全挂 | 已有原子写 + 备份 + rollback；补「写后校验」：写完立即 JSON 解析，失败即回滚 |
| `model` 顶层字段与用户已有值冲突 | 快照保存原值，断开还原；接线前若用户顶层 model 与 env 同设，Toast 提示 |
| 档位键表写错（漏 FABLE / SUBAGENT） | 键表做成常量数组 + 测试锁定（magpie 同款 `claudeEnv` 列表） |
| `/model` 菜单显示错乱（#1227 同款） | `_NAME/_DESCRIPTION` 与模型绑定，换档位时同步重写 |
| 云开关误删用户手动配置 | 只处理已知白名单键，其余不动；暂存值断开时原样还原 |

**验证**：wiring 纯函数测试扩到 15+ 用例（往返、drift、云开关、1M、FABLE、SUBAGENT、调用方 Key 保留）；真机验收「接线 → Claude Code 内 `/model` 换模型 → 无需重接」。

---

## 3. P2 · ZCode 转发链路简化 + 网关鉴权放宽

### 3.1 ZCode 改读 coding-plan Key（放弃网关转发走 OAuth 交换链）

**做什么**

- 网关适配器（`gateway/adapter/zcode.rs`）的取凭据路径改为：读 `~/.zcode/v2/credentials.json`，找 `*:coding-plan:*:api-key` 条目，AES-256-GCM 解密（**`zcode_quota.rs` 已有全套解密实现，直接复用**），按键名判家族（含 `:bigmodel-` → open.bigmodel.cn，否则 api.z.ai）。
- 保留现有 OAuth 设备码登录**只用于「供应商页登录 ZCode」**这一产品功能（写入 `connections.secret`，签到、额度继续用），网关转发不再走「列项目 → 取项目 Key → 换短期 token」三步链。
- 兜底顺序照 magpie：coding-plan Key → `config.json` 的 `options.apiKey`（ZCode 3.15+）→ `zcodejwttoken`（Start Plan）→ 团队计划 Key（`zcode_team.go` 语义）。
- `-Trial` 模型的两种上游拒绝话术（400 试用结束 / 429 额度耗尽）只休置该模型条目不连坐账号（magpie `zcode_trial.go`）。

**风险与规避**

| 风险 | 规避 |
|---|---|
| **合规敏感**：读用户在另一工具的凭据文件 | magpie 同样读且只读不改（Jolu 的 `zcode_quota.rs` 先例相同）；只读、不写、不落日志，密钥不出后端（AGENTS.md §4 已有此约束）；README 里向用户说明读取范围与关闭方式 |
| 解密失败的报错要可懂 | 复用 quota 模块文案「请在原工具检查登录」；不把解密异常当 401 透传 |
| coding-plan Key 可能失效（2026-11-05 验证记录里那把 Key 业务码 1000） | 兜底链多级降级 + 失败原因分档展示；验证记录注明那次是额度端点，模型端点需实测 |
| 两套凭据路径并存（secret vs credentials.json）造成状态分裂 | credentials.json 读取作为**转发期**实时路径；`connections.secret` 仍是登录态的持久化，二者职责分离并写进模块注释 |
| ZCode 3.15+ 凭据位置变化 | 兜底链含 config.json 分支；family 判定写测试锁定 |

**验证**：解密单元测试（构造 enc:v1 密文）；家族判定测试；真机用 z.ai 登录后网关直发一条请求。

### 3.2 网关鉴权放宽（消灭 Key 刷新 401）

**做什么**

- 回环监听不变；token 校验改为 magpie 模式：**接受任意 token**，仅用 token 前缀（`jolu-<agent>`）与 User-Agent 识别调用方；手动 Key 保留为「局域网共享 / 未来多 Key」场景的可选强校验。
- 保留现有 `authorize` 代码路径作为共享模式开关，默认关。

**风险与规避**

| 风险 | 规避 |
|---|---|
| 本机恶意程序借网关消耗订阅 | 网关仅绑定 `127.0.0.1`（现状）；共享模式（局域网）必须显式开且走强校验 Key |
| 旧接线用户升级后 token 不匹配 | 放宽后任意 token 都收，401 自然消失；这正是目的 |
| 路由流水按 agent 归因弱化 | token 后缀 + User-Agent 双通道识别（现状已有），补测试 |

---

## 4. P3 · 路由分组与多账号故障转移

**做什么**

1. **分组模型**：`group/<name>` 作为一个可选模型 id 成员进入模型目录；成员支持嵌套分组。语义参照 magpie：成员可带档位（`provider/model:low`）。
2. **五种路由策略**：`smart`（配额剩余先到先用）/ `order`（依次到不能答）/ `rotate`（轮转）/ `usage`（最少先用）/ `pace`（按周剩余配速）。Jolu 现有 Affinity（会话粘账号）+ Cooldowns（429 冷却）直接复用为组内约束。
3. **多账号故障转移**：同一订阅多账号、多 Key 轮转；失败后 `rest`（休置）与恢复；用量上限保持（cap hold）；每账号权重与轮位。路由流水记录每次 try（magpie `Try` 结构：ID/Model/Effort/Picked/Start/Status）。

**风险与规避**

| 风险 | 规避 |
|---|---|
| 分组嵌套与亲和、冷却交织，状态机复杂 | 先做 `order` 单策略打通端到端，再逐个加策略；每种策略独立测试文件 |
| 会话亲和与分组切换冲突（切换=缓存失效） | 沿用现有语义：亲和优先，组切换仅在当前成员整体不可用时发生，流水记录原因 |
| 意图路由（小模型分流）依赖额外模型调用，产生成本 | 放到最后做，且默认关闭 |
| UI 复杂度（画板缺口） | 管理界面先做在「管理来源」弹窗扩展页，`Jolu.pen` 补画板后再上主界面 |

**验证**：每种策略一个端到端测试（合成上游已具备：`scripts/acceptance/model-upstream.py`）；配额耗尽切换实测。

---

## 5. P4 · Provider 预设库 + models.dev 元数据 + 导入 + Profiles

**做什么**

1. **预设库**：把自定义供应商连接流程升级为预设选择（DeepSeek、Kimi、MiniMax、Qwen、OpenRouter、SiliconFlow、Ollama、LM Studio 等 magpie 同款清单），预设含默认 Base URL 与协议；自定义 OpenAI/Anthropic 兼容地址保留。
2. **models.dev 元数据**：模型清单拉取后补全上下文窗口、输出上限、模态、推理档位（models.dev 是 magpie 的数据源；离线缓存 + 手动刷新，不自动联网写库）。
3. **导入**：一键读取 **CC Switch**、Claude Code、Codex 现有配置生成连接（Alma 可选）；导入前 diff 预览、用户确认才落库。
4. **Profiles（配置一键切换）**：把「每个 agent 当前选的模型 + 档位覆盖 + 默认来源」整体存成一个命名快照（如 `Budget` / `Focus`），切换时一次还原全部 agent 的接线；每个 agent 自己的常用模型短列表随 Profile 一起保存。
   - 存储：SQLite 新表 `profiles(name, snapshot_json, created_ms)`，快照只存 **Jolu 自己的接线决策**（agent → 模型 id 映射），不存用户原配置；
   - 切换 = 按 P1.2 的档位接线流程逐 agent 重放（复用备份/原子写/写后校验），不引入新的写文件路径；
   - 切换前 drift 检测：某个 agent 的配置被外部改过时，先提示确认再覆盖，不静默重写；
   - CLI 同步提供 `jolu save <名>` / `jolu use <名>`（P8 CLI 的两个子命令先行）。

**风险与规避**

| 风险 | 规避 |
|---|---|
| 预设地址过期/变更 | 预设表集中一个文件 + 测试锁定；版本更新可修 |
| models.dev 拉取失败/数据错误 | 元数据只作「建议值」，可编辑；拉取失败用内置兜底表 |
| 导入误改用户现有配置 | 只读导入；生成连接草稿，用户确认后入库；不写回原文件 |
| 读 CC Switch 配置的格式风险 | 参照 magpie 的 import 实现（`internal/provider` 对应文件）逐字段核对 |
| Profile 切换覆盖用户手动改动 | 切换前 drift 检测 + 逐 agent 确认；快照只含 Jolu 决策，断开还原路径不变 |
| Profile 与并行手动接线冲突 | 切换即一次完整接线重放，走同一把 `cli_switch` 锁；流水记录「来自 Profile ×××」 |

---

## 6. P5 · 隐私脱敏

**做什么**（magpie 的方案是标杆：规则 + 可选本地模型 + 占位符回填）

1. **规则层**：API Key、私钥（PEM 段）、token、邮箱、校验位合法的卡号/身份证号；`Jolu.pen`/AGENTS.md 无相关画板 → 界面标「待确认」。
2. **占位符协议**：`{{API_KEY_<hash8>}}`，同值同占位符（保 prompt cache 与思考签名），**响应侧流式 + 工具参数都回填**。
3. **内存态**：值不落盘，记录只留类型/长度/首字符；脱敏开启但引擎不可用时请求 **503 拒发**，绝不原样放行。
4. **可核查**：Insights 页列每次改写对照；豁免清单（Never mask）。
5. 会话交接脱敏（现有 `handoff_facts.rs` 的正则）**复用同一规则引擎**，两处一套实现。

**风险与规避**

| 风险 | 规避 |
|---|---|
| 改写请求体破坏流式/工具调用 | magpie 证明了可行（占位符稳定 + 全链路回填）；测试覆盖 SSE 分块切割场景 |
| 规则误报（把代码当密钥） | 默认只开确定规则；本地模型层默认关（1.6 GB 下载）；Warn/Block/Mask 三档 |
| 性能（大 prompt 全量正则） | 增量扫描 + 长度上限；基准测试锁定 |
| 与「验证模型」「观测统计」冲突 | 脱敏后请求不再含真值，观测层记账按脱敏前长度估算，文档说明 |

---

## 7. P6 · 网关中间件 + 插件系统（架构扩展，最后做）

### 7.1 网关中间件

- `onRequest`（改写/拒发）、`onEvent`（逐流事件）、`onResponse`（整响应）三个钩子；抛错或超时则请求原样放行。
- 首发内置：`model-map`（模型改名映射）、`param-override`、`system-prompt`、`word-guard`、`think-tags`（New API 同款语义）。
- **风险**：钩子链执行顺序与失败语义复杂。**规避**：钩子相互独立、失败隔离（单个钩子异常=该钩子跳过）、执行顺序固定文档化；每钩子一个独立测试。

### 7.2 插件系统 + 订阅外置（决策点）

- magpie 用 Bun 跑 npm 插件，把 Qoder/WorkBuddy/ZCode/Trae/Grok/Cursor/Kiro/Devin/Factory/Zed/MiMo 等订阅登录全部外置，主程序规避厂商条款风险。
- **对 Jolu 的含义**：这是**产品路线决策**而非纯技术——内置适配（现状）体验一体化但有合规暴露；插件化安全但要引入插件运行时（Rust 侧需要 JS 运行时或子进程方案，工作量大）。
- **风险与规避**：
  - 引入 Bun/Node 子进程 = 新增攻击面与分发体积 → 分期：先做「子进程插件协议」只跑登录取凭据，再做全功能插件；
  - 插件生态不存在（magpie 的插件是社区仓库）→ Jolu 可先内置但**模块隔离**（feature flag 编译开关），为未来外置留缝；
  - 本阶段开工前单独与鼠鼠确认路线，不默认启动。

---

## 8. P7 · 工具生态 / P8 · 分发形态（低风险，按需排期）

| 条目 | 做法 | 风险/规避 |
|---|---|---|
| Web 搜索 MCP 工具 | Tavily/Brave/Exa 等多 Key 轮转；挂成 MCP server 供任意 agent 用 | Key 管理复用现有加密存储；限流轮转写测试 |
| 图像 / 视频工具 | 选中模型经网关转发生成 | 纯转发，低风险 |
| Library | 指令/MCP/skills 按各 agent 自己的格式写入，移除只删自己写的 | 与 Skills 管理模块整合，避免两套写入器 |
| CLI | `jolu <agent> <model>`、`jolu provider add`、`jolu quota`、`jolu settings set` | 复用 Tauri 后端命令层抽出的服务函数；先列命令清单再实现 |
| TUI / Web UI | 远期 | 依赖 CLI 层 |
| 局域网共享 + 每客户端 Key 限额 | 现有 `gateway_access` 扩展：命名 Key、日/周/月 token 与费用限额 | **必须**强校验（P2.2 已预留开关）；限额计数落库 |
| 远程网关 / Docker | 文档 + 容器化打包 | 网关默认只听回环；容器模式文档明示风险 |
| 同步备份 | WebDAV / S3 / 加密 Git 仓库 | 凭据文件**不进同步**或加密后同步；冲突策略 last-write-wins + 提示 |
| agent 版本管理 + WSL 接线 | 检测各 CLI 版本、按安装方式更新；Windows 下 WSL 内配置写入 | WSL 路径解析按 magpie `appdir` 语义；只读版本检测先行 |
| 每日 warm-up | 定时激活订阅 5 小时窗口 | 复用自动化调度器；默认关 |
| OTLP 导出 / 账单 | 用量数据 OTLP 推送、按账号/Key 出账 | 导出内容脱敏核查（AGENTS.md §4） |

---

## 9. 明确不抄的

| magpie 的做法 | 不抄理由 |
|---|---|
| 文件式存储（放弃 SQLite） | Jolu 的 SQLite + WAL 运行良好；迁移无收益 |
| 每日匿名遥测 | Jolu 无此需求，AGENTS.md 无授权 |
| 菜单栏为主的应用形态 | Jolu 有灵动岛承担常驻职责 |
| 退役第三方订阅出主程序的**当前执行** | 路线正确但 Jolu 用户基数不同；P6.2 决策点另行确认，短期以模块隔离过渡 |
| `usemagpie.ai` 导流链接 / 赞助体系 | 非功能 |

---

## 10. 里程碑与验收口径

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M1（P1+P2） | id 解析 + 档位接线 + ZCode 简化 + 鉴权放宽 | 你的 z.ai 账号：供应商登录成功 → 网关转发成功 → Claude Code 里 `/model` 换模型无需重接；`GLM-5.3-Flash[1M]` 三种写法全通 |
| M2（P3） | 分组 + 多账号转移 | 配额耗尽自动切换、流水可解释 |
| M3（P4+P5） | 预设 + 导入 + 脱敏 | 从 CC Switch 一键迁入；开脱敏后真 Key 不出本机 |
| M4（P6） | 中间件 + 插件路线确认 | model-map 可用；插件路线有结论 |
| M5（P7+P8） | 生态与分发 | CLI 可用、共享可开 |

**每阶段的通用验收**：全量测试（当前基线 977 后端 / 403 前端）只增不减、typecheck 过、`Jolu.pen` 有对应画板或「待确认」标注、真机验收记录写入 `docs/verification/`。

---

## 11. 全局风险清单（跨阶段）

| 风险 | 等级 | 规避 |
|---|---|---|
| **合规**：内置第三方订阅登录违反厂商条款（magpie 退役它们的理由） | 高 | 短期：适配器模块隔离 + 文档声明；中期：P6.2 插件化决策；永不把凭据写入日志/导出 |
| **写坏用户工具配置**（接线/导入/Library 都在改用户文件） | 高 | 统一写文件组件：备份 → 原子写 → 写后校验 → 回滚；只清理自己写的键；drift 检测（已有） |
| **破坏性升级**（旧版接线残留、模型键变化） | 中 | 每次接线键表变化配迁移清理；启动自愈路径已有先例（pending 修复） |
| **并行会话冲突**（工作区常有另一会话在改） | 中 | 动手前 `git status`；按文件划界；提交前核对 staged 清单 |
| **测试基线回退**（977/403） | 中 | 每条目「测试先行」；CI（发布工作流）跑通才算完成 |
| **UI 无画板依据** | 中 | 先画 `Jolu.pen`（或标「待确认」）再写界面；冲突以画板为准 |
| **凭据读取的边界**（credentials.json 解密） | 中 | 只读、加密落库、不进日志；README 向用户披露读取范围与关闭方式 |
| **范围失控**（magpie 68+ agent、298 网关文件的体量） | 高 | 本计划只对齐「功能能力」，不对齐 agent 数量；agent 扩容按需单独排期 |

---

## 附录 A · 差异总表（对照研究结论，2026-11-08）

| # | 能力 | magpie | Jolu | 结论 |
|---|---|---|---|---|
| 1 | Claude Code 模型键 | 删 ANTHROPIC_MODEL、写顶层 model、4 档 env + SMALL_FAST + SUBAGENT、_NAME/_DESCRIPTION、[1m]、云开关暂存 | 只写 BASE_URL + AUTH_TOKEN | **P1.2** |
| 2 | 模型 id 解析 | [1m] 剥离、裸名兜底、目录索引 | 精确 slug 匹配（默认来源为新增补丁） | **P1.1** |
| 3 | 转发层模型名 | 不二次校验 | zcode 曾要求必带 `/`（已修，其余待排查） | **P1.1** |
| 4 | 网关鉴权 | 任意 token（回环） | 手动 Key + 接线 Key 严格校验 | **P2.2** |
| 5 | ZCode 凭据 | 直读 credentials.json coding-plan Key（键名判家族） | OAuth→项目→Key→短期 token 多步链 | **P2.1** |
| 6 | ZCode 双家族 | zai / bigmodel 键名即家族 | 登录写死 bigmodel（z.ai 支持本轮刚加） | **P2.1** |
| 7 | ZCode 试用条目 | -Trial 两种拒绝话术分档休置 | 无 | P2.1 |
| 8 | 路由分组 | 5 策略 + 嵌套 + 意图路由 | 无（有亲和/冷却） | P3 |
| 9 | 多账号转移 | 权重/轮位/上限保持/rest 休置 | 有池与冷却，无转移策略 | P3 |
| 10 | Provider 预设 | ~30 家 + models.dev 元数据 | 自定义地址为主 | P4 |
| 11 | 导入 | CC Switch / Claude Code / Codex / Alma | 无 | P4 |
| 12 | Profiles | 一套配置一键切换 | 无 | P4 |
| 13 | 隐私脱敏 | 规则+本地模型+流式回填+Insights | 会话交接脱敏（局部） | P5 |
| 14 | 网关中间件 | onRequest/onEvent/onResponse + New API 同款内置 | 无 | P6 |
| 15 | 插件系统 | Bun + npm，订阅全部外置 | 无（全内置） | P6（决策点） |
| 16 | 搜索/图像/视频工具 | 多引擎轮转 MCP | 无 | P7 |
| 17 | Library | 指令/MCP/skills 写入各 agent 格式 | Skills 管理只管 skills 文件 | P7 |
| 18 | CLI / TUI / Web UI | 全量 | 无（桌面 GUI） | P8 |
| 19 | 局域网共享/远程/Docker | 有 | 无 | P8 |
| 20 | 同步备份 | WebDAV/S3/GitHub | 无 | P8 |
| 21 | agent 版本管理/WSL | 有 | 无 | P8 |
| 22 | warm-up | 定时激活 5h 窗口 | 无 | P8 |
| 23 | 账单/OTLP | 有 | 无（有用量统计） | P8 |
| 24 | 签到 | WorkBuddy、Trae CN、Qoder 活动、ZCode 礼包 | WorkBuddy 国内、Qoder、Trae 国内 | 基本对齐 |
| 25 | 灵动岛 | 无（菜单栏面板） | 有（独有） | 保持 |
| 26 | 自动化任务 | 无 | 有（独有） | 保持 |
| 27 | 价目估算体系 | models.dev 价 + 自定义价 | 61 条价目 + 长上下文 + 峰谷价（独有深化） | 保持 |
| 28 | agent 覆盖数 | 68+ | 5+ | 按需扩，不在本计划 |
| 29 | 观测模式 | 无 | 有（独有） | 保持 |
| 30 | 隐私遥测 | 有（DO_NOT_TRACK） | 无 | 不抄 |
