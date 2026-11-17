# 剩余工作总执行清单（2026-11-09 起）

**来源：** 鼠鼠 2026-11-09 要求「不管哪些，现在没完成的都需要完成」。条目合并自：
`2026-11-09-magpie-parity-batch3.md`、`2026-11-09-magpie-parity-full-audit-authoritative.md`（权威版）、
`2026-11-09-jolu-vs-magpie-feature-and-code-comparison.md`、`2026-11-08-magpie-parity-roadmap.md`，
以及当日对全部计划 / 设计 / 待办文档的四组核查。

**鼠鼠已定：**
- 无画板的新界面：按现有组件做最小版，代码标「待确认」，最后集中过一遍。
- 每个阶段全量测试通过后提交并推送 main（`[skip ci]`）；远端领先时 rebase 本地未推送提交后再推。
- 字号放大 10→11、11→12，并同步 `Jolu.pen`。
- 原计划「暂不做」的高成本项也全部做。
- 订阅适配不做成插件（P6.2 已否决）。

**不归小的（需鼠鼠本人）：** 真实账号 / 真机验收、GitHub 社交预览图上传、Windows Sandbox（本机为家庭版，无 Sandbox）、发版构建。代码会写完并补测试，「真实可用」的结论以鼠鼠验收为准。

**与权威版的一处出入：** 权威版据 magpie 主仓库称 C1 礼包过滤、C5 边缘保持「作废」。ZCode 适配已从主仓库退役到社区插件 `magpie-community/plugins` 的 `packages/zcode`，该插件仍有 `startPlan` 与 `edge`（3 分钟），`fe29dc1` 按插件实现，保留。

---

## 交接说明（2026-11-12 更新，给接手的同事）

**进度：** 阶段 0、1、2、3 完成；阶段 1B 只剩 B5（桌面端验证）与 B13（依赖 5.2）；阶段 4 完成 4.1–4.4、4.6，4.7 完成三项（4.2 的 P1 / P2 联动另列 4.2b、4.7 的能力三态待鼠鼠拍板；4.5 本机无法取样，受阻），其余未开始；阶段 5 未开始。以下勾选框为准。

**接手从这里开始：** 4.8 → 阶段 4 其余 → 阶段 5。每项做完勾选并写明验证结果。新代码的小字用 `text-note`（11px）/ `text-meta`（12px），不要再写 `text-[10px]` / `text-[11px]`。

**2026-11-12 新增：** `gateway/intent.rs`（意图路由，3.9）。路由分组新增 `intent` 策略、成员 `tier`（`light` = 轻档）与分组的 `intent_json` 列（判定模型与超时，可空，旧分组不受影响）；判定调用经 `wire::handle` 自调网关的 Chat 入口（显式装箱打断递归 future）。顺带修了分组 id 只用毫秒时间戳、同一毫秒建两个分组会被误报「id 已被占用」的问题。

**2026-11-11 新增的公共件：**
- `gateway/adapter/claude_bridge.rs`：Claude 订阅的流式 / 图片 / 工具桥。网关自己是 MCP 服务器（`/__jolu/claude-mcp/<令牌>`，在 `server.rs::local_only` 里跳过 Key 校验、令牌由处理函数核对）；工具调用挂在 `RUNS` 里等调用方下一次请求带结果来。旧的 stdio 桥（`bridge.rs`、`bin/gateway_mcp_bridge.rs`、`/__jolu/bridge/callback`）已删除。
- `gateway/adapter/gemini_chat.rs`：Chat ↔ Code Assist 翻译与 SSE 解码（`SseDecoder`），Gemini 订阅的 Chat / Responses / Anthropic 调用方都经它。
- `gateway/mask.rs`：`Policy`（每类遮住 / 只记录 / 拦下 + 豁免清单）、`records()`（内存里的命中记录，不含原值）、`sensitive_spans()`（交接摘要共用的识别规则）、`on_first_mask()`（首次遮住的钩子）。
- `gateway/middleware.rs`：`EventChain` + `transform_sse`，响应侧中间件的流式逐事件钩子（onEvent）；新增流式能力的中间件在这里加分支。
- `gateway/js_hooks.rs` + `js_hooks_host.mjs`：JS 钩子宿主（常驻 Node / Bun，一行一个 JSON），`server.rs::serve` 是三个协议入口共用的流程：钩子 onRequest → 脱敏 → 转发 → 回填 → 响应中间件 → 钩子 onResponse。
- 测试里起子进程宿主时注意：Windows 上 tokio 读子进程输出走阻塞线程池，测试结束前要关掉宿主（见 `js_hooks::shutdown`），否则测试运行时收尾会卡住两分钟。

**2026-11-11 新增的未验证项：** Claude 订阅桥（真实 `claude` 客户端的 stream-json 输入、HTTP MCP 与工具挂起续跑）、Gemini 订阅的 Chat / 流式、JS 钩子在桌面端的接入、隐私设置新界面与脱敏记录弹窗——都只有单元 / 端到端模拟测试，没有真实账号或桌面端走过。

**本次新增的公共件（后续条目可复用）：**
- `backend/src/steady.rs`：读外部或状态文件一律用 `crate::steady::read / read_to_string / exists / read_optional`，不要直接 `std::fs::read`——否则 agent 用 rename 写配置时可能把「有配置」读成「没有」，进而覆盖用户配置。
- `backend/src/gateway/budget.rs`：`UsageMeter` 能从任意协议的响应流里读出 Token 用量，阶段 2.3（订阅用量入库）、2.6（周 / 月 Token 与费用限额）可以直接用。
- `backend/src/service/gateway/middleware.rs`：服务层「校验后保存」的写法可参照。
- 界面：无画板的新界面一律沿用 `GatewayDialogs.tsx` 的 `Modal` / `Button` 与 `jolu-*` 变量，代码里标「待确认」（鼠鼠已同意这样做，最后集中过稿）。

**未验证（需要实机或真实账号）：**
- 中间件弹窗、成员暂停、ZCode 通道开关、签发表单三项约束：只过了类型检查和纯逻辑测试，没在桌面端点过。
- Token 预算：只有单测与流包装测试，没有真实客户端走一遍。
- C1 / C2 / C5：ZCode 当前没有额度（余额 `plans=0`），端到端要等额度恢复。

**已修的不稳定测试：** `handoff_facts::tests::redact_is_linear_on_pathological_input` 原为 1 秒绝对上限，全量并行时常超时；2026-11-11 改为「输入放大 4 倍、耗时放大不超过 10 倍」的线性判定。

**提交节奏：** 每个阶段全量测试通过后提交推送 main，信息带 `[skip ci]`（不触发发版）；远端领先时把本地未推送提交 rebase 到 origin/main 之后再推。

---

## 阶段 0 · ZCode 通道收尾与命名 Key 修复

- [x] C1 / C5 / C2、版本号统一、白名单与状态码修复、签发表单（`fe29dc1`）

## 阶段 1 · 交付了但用不上 / 数据风险（P0）

- [x] 1.1 中间件配置入口：`gateway_middleware_list / save / delete` 命令（保存前在样例请求上试跑，跑不通拒存）+ 网关页头「中间件」弹窗；顺带实现了原先无效的 think-tags `wrap` 模式
- [x] 1.2 路由分组成员暂停入口（30 分钟 / 1 / 4 / 24 小时、立即恢复）；前端保存分组时保留 `paused_until_ms`；smart / pace 过期文案已改
- [x] 1.3 `zcode_free_first` 开关入口：「管理来源」弹窗底部，池里有 ZCode 来源时显示
- [x] 1.4 费用按请求时间计价：调用明细、费用汇总、请求日志、会话费用、趋势图五处（汇总与趋势按「一周中的第几个小时」分桶）
- [x] 1.5 P0-2 `lastgood`：`settings.json` 每次保存留 `.lastgood`；读不出时坏文件另存 `.corrupt` 并回读上一版
- [x] 1.6 P0-3 `steady.rs`：接线、配置接管、凭据检测、CLI 切换、设置加载的读取跨过 rename 窗口（不存在 / 被占用时 10 × 3ms 重试）
- [x] 1.7 P0-1 / F3 `gateway/budget.rs`：命名 Key 的 `tokens_per_day` 预扣 + 回冲强制（响应流里读 usage；读不到按预扣额记，上游失败按 0 记）
- [x] 1.8 P0-4 / C6 `zcode_version.rs`：读卸载注册表的 ZCode 版本，比适配版本新时启动日志 + 「管理来源」弹窗警告（`zcode_version_check` 命令）
- [x] 1.9 零碎：删除孤立的 `providerLinks.ts`；验收 README 补 `x-jolu-proxy` 说明；D-34 记入 `docs/todo/2026-11-11-resume-channel-compaction.md`
- [x] 1.10 更新状态已过期的文档（2026-11-11 逐份核对，改为「2026-11-11 核对」说明；观测响应头、未验证清单 Qoder / Trae 行、八平台现状表、连接归因命令名、10-29 网关计划、去重说明阶段 4、订阅绑定 Qoder、11-08 路线图附录 A、联动调研 §1.1、额度描述符抬头、平台补全 V-2）

## 阶段 1B · 同事 11-10 批次的遗留（2026-11-11 并入）

来源：`docs/specs/2026-11-10-capability-batches-handoff.md`、`docs/specs/2026-11-10-narrow-window-layout-handoff.md`、`docs/plans/2026-11-09-magpie-remote-sync-149.md` §6，以及 11-11 核对时发现交接文档漏写的项。

- [x] B1 搜索 Key 明文经 `set_*` 命令与 `settings-changed` 事件到达前端：改为序列化默认掩码、只有设置落盘时放行原文（`gateway::search::secret_keys`）
- [x] B2 Claude 拒绝 thinking 签名（400）后按点名位置剔块重发一次，并按来源记住（`thinking_guard` 400 节、`wire::with_thinking_repair`）
- [x] B3 OTLP 导出带命名 Key 名（`jolu.gateway.key.request.count`，属性 `jolu.gateway_key.name`）
- [x] B4 统计卡 5 处与骨架屏按容器宽度减列（`components/ui/MetricGrid.tsx`）
- [ ] B5 会话详情按钮行与全局 `button { white-space: nowrap }` 的桌面端验证；账号管理窄屏无画板依据（待鼠鼠确认）
- [x] B6 搜索后端补 Brave / Exa / Firecrawl / You.com / DeepSeek / 元宝 6 个；豆包改按火山文档的 `Query` / `Result.WebResults` 形状（首版字段不符）；跨主机重定向不跟随（Key 在自定义头里）；MCP 依次尝试所有启用的后端
- [x] B7 档位 effort 可用值校验（按 Claude Code 对各模型实际发送的范围，别家模型为全集；models.dev 没有 efforts 数据，未用）；顺带修 `autoMode` 字段名不一致导致 Auto 判别档被丢弃
- [x] B8 `web_fetch`（每跳过 netguard 并把连接钉在检查过的地址上；MCP 工具 + `jolu_cli fetch`，真实网页已试读）
- [x] B9 网关替身工具 + 内部多轮（`gateway/tool_loop.rs`、`wire::with_tool_loop`）：Claude Code 声明的 web_search / web_fetch 服务端工具在上游不是真 Claude 时由网关执行，最多 5 轮，客户端要流式时整条回放成 SSE；首字延迟未优化（多轮期间不发进度事件），实机未验证
- [x] B10 搜索插件安装扩到 ZCode（`~/.zcode/cli/config.json` 的 `mcp.servers`）、Qoder CLI 两版、WorkBuddy（`mcp.json`，需在 WorkBuddy 里批准）；Trae 无可靠来源，列为不支持。顺带修首版路径错误：Claude Code 应写 `~/.claude.json`（原写 `settings.json` 不生效）、Gemini 应写 `settings.json`（原写进 `.env`），卸载时清理旧位置
- [x] B11 界面（无画板，标「待确认」）：接线弹窗内的档位面板（四档 + effort + Auto 判别，接线与「应用档位」都带上档位，回填实际写入值）；设置 ·「网络与代理」页尾的联网搜索（掩码即「保留原 Key」，试搜）与搜索插件安装列表；预览验收 `scripts/acceptance/gateway-tiers-search.e2e.mjs`
- [x] B12 magpie 行动清单：裸模型名优先走 agent 自己的订阅（`route::pick_bare`）；命名 Key 同时请求数上限（`key_slots.rs`，超出排队，局域网共享弹窗可设）；Smart 等按额度排的策略在界面上说明列表顺序只决定平局、无额度数据的成员排最后；ZCode 草稿目录归为一组（Jolu 不把 ZCode 请求关联到会话，不存在重复计数）；URL 安全 / 无填充 base64 统一成标准（`base64_input.rs`，两个入口）；顺序策略恢复后回切（默认行为，同 magpie）。Codex 大 `config.toml`：实测 10,933 个 `[projects]` 表、0.9MB 时 `toml_edit` / `toml` 解析各约 20–25ms（release），没有 magpie 的 go-toml 平方级问题，不改
- [ ] B13 远程 MCP（http / sse）——依赖 5.2 OAuth

## 阶段 2 · 网关正确性

- [x] 2.1 路由分组进 `/v1/models`；R1 成员去重（保存与嵌套展开时）；R2 保存时查环、查引用存在、深度 ≤ 8；R3 一次请求同一 account_fp 只试一次；R6 流水行带 `request_id`，界面标「第 k/n 次」
- [x] 2.2 普通来源失败换同模型其他连接重试（同平台、同协议、不同账号、目录里有该模型，连主选最多 3 个；指定来源已标用尽 / 失效时直接用替补）；402 `exhausted` 记录当天日期，次日零点后刷新模型池时自动恢复（旧数据无日期不动）
- [x] 2.3 Grok / WorkBuddy / ZCode 订阅转发的用量入库（`subscription::record_usage_on_end`：响应流结束或客户端断开时用 `UsageMeter` 读上游给的用量写 `requests`，读不到不记）
- [x] 2.4 C3 团队席位状态校验（未分配席位 / 已过期 / 未订阅直接说明，查询失败不拦）；C4 Start Plan 每账号 3 个在途、其余排队、等 2 分钟本地回 429；P2-8 `-trial` 模型被拒只让该账号的该模型休息 60 秒，Start Plan 的 402 不再把整条 ZCode 连接标成用尽（真实账号未验收：当前无额度）
- [x] 2.5 Claude Code：D2 用户的 `effortLevel` 新主模型不收时调到它收的档（断开还原）；D4 各档写 `_SUPPORTED_CAPABILITIES`；单档覆盖（B11 档位面板）；keyHolds 保留已是命名客户端 Key 的令牌；顶层 model 与 `env.ANTHROPIC_MODEL` 同设时接线确认框提醒；流水记 `resolved_from=bare` 并标「裸名」。顺带修：Auto 判别键断开时不还原、重接线把自选档位 / Auto 值当用户原值收进快照、只有顶层 model 的快照被当旧格式不还原
- [x] 2.6 命名 Key：`accounts` 账号范围（范围外候选去掉，一个不剩 403）；日 / 周 / 月 Token 上限（预扣 + 回冲）与费用上限（按实际路由的平台与模型用价目表估算，已花达到后拒绝；未收录价目的模型不计）；命名 Key 一律记账，局域网共享弹窗按 Key 显示今日 / 本周 / 本月 Token 与费用，「限额」可编辑上限与账号范围（待确认）
- [x] 2.7 接线漂移增加「网关不可达」（已接入但网关没在中转模式运行）；改来源 id 时 Claude Code 接线里的 `旧id/模型` 同步改写；分组成员固定档位 `模型:effort`（按协议写进请求，请求本来没要推理不加）、轮转权重、失败后休置 60 秒；Profile 另存档位 / 子代理 / 默认来源 / 常用模型并在应用时恢复（修复切换会删掉子代理），流水标「来自 Profile」；应用 Profile 前在确认框列出会被跳过的漂移工具
- [x] 2.8 models.dev 读出名称、描述、推理与档位、输入模态、工具调用（修复名称从没读、显示名一律退回 id）；档位校验优先用 models.dev 的档位；手动刷新命令与接线弹窗里的「立即刷新」；内置兜底表为主要厂商的 models.dev 快照（`scripts/dev/models-dev-builtin.mjs` 生成，241 个模型），只在从没拉到过时使用
- [x] 2.9 ZCode 接线改写 `v2/provider_config.json`（provider 规则 + 每个模型的上下文与 `maxOutputTokens`，防 32000 截断）与 `v2/config.json`，断开时两份都还原并清掉旧版写在 `provider.json` 的键——此前写的 `~/.zcode/provider.json` ZCode 根本不读，ZCode 接线一直无效；P2-9 来源地址白名单（IP / CIDR，`JOLU_ALLOW_IPS` 优先，本机始终放行，只认 TCP 对端不信转发头），局域网共享弹窗可编辑（真机未验：需要 ZCode 重启后看选择器、需要另一台设备验白名单）
- [x] 2.10 中转模式端口被占拒绝启动的单测（绑定逻辑抽成 `proxy::bind_listener`）；接线写配置后读回比对并按扩展名重新解析 JSON / TOML，不对就放回原内容并报错

## 阶段 3 · 协议能力

- [x] 3.1 Claude 订阅：工具调用闭环、流式、图片（`adapter/claude_bridge.rs`）。仍驱动官方客户端：stream-json 输入带图片 / 文档块，`--include-partial-messages` 的流事件原样转给调用方（非流式由同一串事件拼成）；调用方工具由网关自己的 MCP 端点 `/__jolu/claude-mcp/<令牌>`（HTTP 传输，令牌写在临时配置文件里）提供，工具调用挂在网关里、本段以 `tool_use` 结束，下一次请求带结果回来交给同一个进程接着生成；工具名按客户端规则截短加哈希、联合 schema 折成 object 根。旧的 stdio 桥与 `/__jolu/bridge/callback`（无鉴权放行、从未接通）删除。假客户端端到端测试覆盖挂起与续跑；真实账号验收归鼠鼠
- [x] 3.2 Gemini 订阅：原生流式（`streamGenerateContent?alt=sse`，拆掉 Code Assist 外壳）；Chat 调用方经 `adapter/gemini_chat.rs` 翻译（历史工具调用 / 结果、内嵌图片、`parametersJsonSchema`、tool_choice、流式片段与用量），Responses / Anthropic 调用方在 wire 里先转 Chat；Gemini 3 的 thoughtSignature 按工具调用 id 记在内存里回填，查不到用占位签名。模拟上游测试覆盖；真实账号未验收
- [x] 3.3 Codex 的 Responses → Chat 翻译（`gateway/responses_chat.rs`：请求含历史调用、图片、自由格式工具包装与拆回；回复含非流式与流式事件、推理摘要与用量）；OpenAI 官方 / Azure 以外的 OpenAI 协议来源自动走翻译；顺带让 Claude Code 也能用只有 Chat 的第三方来源（复用 Anthropic ↔ Chat 翻译）。真实 Codex 客户端未走过
- [x] 3.4 ZCode 协议转换（非 Anthropic 入口）：Chat / Responses 调用方走 Coding Plan 的 OpenAI 兼容端点（`/api/coding/paas/v4`，同一把 Key），Responses 经 `responses_chat` 翻译；Start Plan 仍只服务 Anthropic。真实账号未验收（当前无额度）
- [x] 3.5 Trae：Max 派生模型（`<id>-max`，配置名用基础模型、一定带输出上限并把窗口余量给提示词）；文本工具补 GLM 模板与 DeepSeek DSML（DSML 归一成 `<invoke>` 后复用已有解析，跨块拆开的标签会扣住）；选路评分加「错名」与名称多数票（参照 magpie trae 插件 `worth`）。真实账号未验收
- [x] 3.6 WorkBuddy：被拒「未授权渠道」时先给开场白加逗号、再把系统提示挪进用户消息，各重发一次并按系统提示记住；非流式请求收到事件流时拼成完整回复。真实账号未验收
- [x] 3.7 中间件 `onEvent` 流式钩子（`middleware::EventChain` + `transform_sse`：think-tags 在流式回复里按结构化的思考字段 / 块逐事件剥除或包成 `<think>` 文本，Anthropic 块 index 往前补齐）；隐私脱敏每类三档（遮住 / 只记录 / 拦下，拦下时整条请求 400 不发）、「从不脱敏」清单（整段、不分大小写）、内存里的脱敏记录与发出前后对照弹窗（只存种类、长度、开头几个字与前后文，不存原值）、第一次真的遮住时发一次系统通知 + 主窗口提示；交接摘要的脱敏改用网关引擎识别卡号 / 身份证号并遵守同一份豁免清单（邮箱按交接的约定不遮）。新界面无画板，标「待确认」
- [x] 3.8 E1 JS 异步钩子（`gateway/js_hooks.rs`）：中间件类型 `js-hook`（`{path, timeoutMs, options}`），常驻 Node / Bun 宿主执行；onRequest 在脱敏之前改请求，抛错 / 超时 / 返回非对象 / 改了 model、stream、工具声明时回滚且不再收到 onResponse；onResponse 拿到整理好的完整回复副本（流式边转发边留，最多 8 MiB），发完即走；ctx 有协议、模型、agent、会话、子代理、origin（local / gateway-key / remote）、options 与每钩子 state，不给请求头与凭据。中间件弹窗可添加（无画板，标「待确认」）。真实 Node 宿主测试覆盖改写、三种回滚与 onResponse；桌面端未走
- [x] 3.9 E3 意图路由（`gateway/intent.rs`）：路由分组新策略「按意图」，默认不启用，要专门建这种分组并用 `group/<slug>` 调用。成员分强档 / 轻档，分组指定判意图用的模型（`来源/模型`，不能是分组）与超时（500–15000 毫秒，默认 4000）；每个新的人工提问把最后一条输入（截到 4000 字，保留头尾）交给判定模型答 STRONG / LIGHT，判定档位的成员排前、另一档兜底，会话亲和在档内仍生效；工具结果回合沿用该会话上次判定（内存记 6 小时），判定超时 / 报错 / 答非所问都按强档并写日志。判定调用走网关自己的 Chat 入口，用量记进路由流水，不计入发起请求的命名 Key 预算。分组弹窗加了策略、判定模型、超时与成员档位（无画板，标「待确认」）。模拟上游端到端测试覆盖轻档、强档、工具回合不重判与判定失效；真实模型的判定准确度没有评测集，未验证

## 阶段 4 · 其他模块与工程质量

- [x] 4.1 字号放大（2026-11-12）：主窗口代码 72 个文件 536 处按 10→11、10.5→11.5、11→12、11.5→12 一次映射（半号跟着挪，避免层级倒挂），`index.css` 的 `text-note` / `text-meta` 改为 11 / 12px；灵动岛（经典 / 胶囊 / 右键菜单）、托盘摘要、启动页不动，8.5–9.5px 的角标与次要说明未纳入。`Jolu.pen` 同步同一映射（A2、A3 与 107 桌面图标尺寸标注除外，各模块改动数与改前统计逐一对上），因放大导致的裁切已修：会话线程统计卡标题行改为随内容高度、自动化「额度未知与检查失败」卡改为随内容高度、用量统计延迟列标签改为自适应宽度。调用明细延迟列在 4 位数 TPS 时会挤进下一列，代码与画板同步把延迟列 124→132、模型列 196→188（总宽不变）。浏览器预览逐页扫描小字裁切并截图核对用量统计、会话线程、供应商；桌面端未走
- [x] 4.2 灵动岛（2026-11-12，只动胶囊样式，经典样式保持现状）：
  - 「等额度」表情：后端 `automation_waiting_connections`（未暂停任务的等待运行所在账号），岛随 `automation-changed` 重查，本岛连接在列表里就显示 `wait` 表情；
  - 额度环跟随被调用来源：网关在途 / 结果停留期间调用的是另一条未暂停的订阅连接时，环临时换成它的额度（分身不跟随，被拒不算调用），只换环、表情与芯片仍是本岛连接；
  - 联动 P0：会话行显示归到的连接名（只认 exact / mapped，平台档不显示）；点会话 → 主窗口会话线程定位、卡片底部「在主窗口查看」→ 用量统计按今天 + 本岛平台 / 连接预筛（新 `MainIntent::Sessions` / `Usage`）；展开卡片「今日按来源」（`island_source_breakdown`，只认 `requests.connection_id`，未归属单独计数写「n/m 条已归属」，最多 5 行）；「正在写」三档（运行中 / 网关在途 = 确证绿眼，只靠最近写入 = 推测青眼，采集源出错且无证据 = 空闲灰眼）；
  - 月窗口环：按额度描述符说明 §459 改蓝色，并按窗口长度排在最外圈（月 > 周 > 5 小时 > 日额度）。
  - 以上均无画板，标「待确认」；没有读 ZCode `provider_config.json`（明文 Key）、没有给 `requests` 加列、没有读进程信息——这些是调研 §6 的待拍板问题。浏览器预览看不到这些（都要桌面端数据），桌面端未走
- [x] 4.2b 灵动岛 P1 / P2 联动（调研 `docs/specs/2026-11-05-island-live-linkage.md` §4 #5–#13）：岛内切换关注的 ZCode provider、多家并列与 `island-focus`、阈值 / 事件短时动效（`island-signal`）、跟随主窗口（`main-context`）、严格模式。依赖 §6 的拍板（读明文 Key 的配置、`requests` 加列、读进程信息、动效默认值、多家并列方式），未拍板前不做
  - 2026-11-17 完成。鼠鼠要求「现在需要完成的全部完成」，§6 待拍板的几项按调研推荐先定，界面都没有画板，标「待确认」：
    - 不读带明文 Key 的 `provider_config.json`；
    - `requests` 不加列；
    - 严格模式不读进程信息；
    - 动效默认开，免打扰或减少动效时只亮颜色；
    - 跟随主窗口默认关；
    - 多家并列按「本体只显示主关注 + 分身显示其他」。
  - #5 只看这一家 ZCode provider：`island_linkage.zcode_provider`。本体岛在 ZCode 时按会话行的来源名（已存连接名）筛会话，只作显示筛选。选项取「今日按来源」里出现过的名字，入口是灵动岛设置页「联动」组。
  - #6 主关注：右键菜单「以此分身为主」（`clone_promote` → `island_clones::promote`）。分身与本体互换所看的平台与连接，窗口位置与停靠不动。「按最近活跃自动选主」（`island_linkage.auto_focus`，默认关）：后端每 30 秒看各平台最近一次请求时刻，某个分身所在平台 2 分钟内有请求、且比本体平台领先至少 1 分钟时，自动与本体互换；同平台的分身不比较，因为分不清是哪条连接在跑。判定是 `island_clones::auto_focus_target`，有测试。
  - #7 阈值 / 事件短时信号：`lib/islandSignals.ts`，在前端用已有数据合成，不加后端事件。
    - 四种信号：额度跨过 80%、额度用尽、网关回 429 / 402、出现新会话。
    - 第一次只记基线；同一种信号 5 秒内合并；每次显示 1.5 秒，沿岛内侧描边（`IslandLinkageBits.tsx` / `index.css`）。
    - 网关拒绝只在胶囊样式有，经典样式没有订阅网关事件。
  - #9 跟随主窗口：`set_main_context` / `main_context` 命令与 `main-context` 事件，前后端事件表同步。
    - 用量页只筛了一个账号时、供应商页定位到某个账号时，报告这个连接（`useMainContextReport`）。
    - 本体岛开了开关后临时显示该连接，显示「跟随中：用量 · 连接名」与「解除」；分身不跟随。
  - 严格模式：`islandWriting` 的 `strict`。只认确证（运行中会话或网关在途）；只有最近写过时不算在写，显示「不知道」。
  - 验证：`islandSignals` 4 项、`islandWriting` 严格模式 1 项、后端 `promote_swaps_connection_but_keeps_windows`、`auto_focus_picks_clearly_more_active_other_platform` 都通过；前端全部纯逻辑测试与类型检查通过。没有在真机上看动效与跟随。
- [x] 4.3 额度描述符（2026-11-12）：
  - 后端注册表 `quota_registry.rs`：（平台, 接入方式）→ 查询路线集中一处，`connection_quota` 只按路线分派；一致性测试保证目录声明了订阅额度的平台都有路线；
  - 限流头 `gateway/ratelimit.rs`：所有调用 `record_upstream_signal` 的转发都记下 Anthropic / OpenAI 式 / IETF `RateLimit` 头的最近一次观测（相对时长不换算成绝对重置时间，裸数字的 `x-ratelimit-reset` 不读）；
  - 运行时置信度：没有官方额度（不支持 / 查询失败）且 24 小时内有观测时返回新状态 `QuotaState::Observed`，前端 `normalizeQuota` 换成 `ok + confidence: "derived"`，窗口名写「上次请求观测 HH:MM」；授权失效 / 权限不足 / 限流仍原样返回，自动化把观测按「未知」处理；
  - `PlatformId` 放宽为「已知 id | 任意字符串」，保留 `KNOWN_PLATFORM_IDS`（测试与平台目录一致）；
  - 虚构供应商快照：`fictionalProvider.test.mjs` 让未登记的 `acme-gateway` 走一遍岛与卡片的全部纯函数；顺带修正未登记平台被补假 5h / 7d 占位行的问题（只有描述符声明 `placeholderWindows: true` 的平台补占位）。
  - 观测值只在网关转发过该连接后才有；真实上游的限流头未实测
- [x] 4.4 价目（2026-11-12）：用量统计「模型」行悬停显示模型全名、官方定价页与核对日期（`pricing::price_source` + 表级 `VERIFIED_ON`，与价目表文件头的核验日期同值，改价时一起改）；未收录的模型写「未收录价目：费用显示 —」，不编出处
- [ ] 4.5 Grok / Trae 会话与用量采集——**受阻（2026-11-12 调研）**：本机没有 Grok 的本地数据（无 `~/.grok`），格式无从核对；Trae 国内版与国际版的智能体库 `%APPDATA%\Trae*\ModularData\ai-agent\database.db` 都不是明文 SQLite（加密），编辑器的 `state.vscdb` 只有设置与模型列表、没有会话或用量。不破解加密、不猜格式；需要鼠鼠提供 Grok 本地样本，或确认 Trae 有官方的会话 / 用量接口后再做
- [x] 4.6 系统通知冷启动（2026-11-12，`toast_link.rs`）：插件在 Windows 上只在提示框还在屏幕上时回报点击；安装版改发协议激活的提示框（`launch="jolu://open?task=…|connection=…"`），启动时把 `jolu:` 协议登记到 HKCU（卸载时 `installer.nsi` 的 `Jolu:` 段删除）。Jolu 已运行时由单实例插件把链接交给已运行实例定位；未运行时新进程从启动参数取链接，主窗口挂载后定位。开发版没有 AUMID，仍走插件。链接解析与 XML 转义有单测；安装版真机未走
- [x] 4.7 自动化区分额度耗尽 / 短时限流 / 余额不足；无模型时的本地交接导出；平台能力「未验证」三态与验证日期；夏令时调度测试（2026-11-12 完成三项）：
  - [x] 三分：`Outcome::RateLimited`（429 / too many requests 且没有额度用完的说法 → 等约 1 分钟或执行器给的时间后检查一次就续跑，`block_kind = rate_limit`）与 `Outcome::Balance`（余额不足 / 402 / credit balance too low → 直接「需要处理」，`block_kind = balance`，等待无用）；额度用完的说法优先于 429。前端提示条与等待文案对应；会话交接的执行同样区分
  - [x] 本地交接导出 `handoff_local_export`：不调用模型，最初的需求 + 最近 6 条原文摘录（引用块、截断、脱敏）+ 结构化事实区，开头写明「这不是模型压缩」；不写交接版本、不推进检查点。压缩弹窗加「本地交接导出（不调用模型）」按钮（无画板，标「待确认」），预览侧栏注明未调用模型
  - [x] 夏令时：测试用自定义时区覆盖「跳过的时刻不补跑（每日 / 每周顺延）、重复的时刻只跑一次」；单次任务定在不存在的时刻时保存提示「因夏令时在本地不存在」，不再误报「已经过去」
  - [x] 平台能力「未验证」三态与验证日期（2026-11-17，按小的推荐方案做，待鼠鼠过稿）：五态（已验证 / 部分验证 / 未验证 / 无记录 / 不适用），只认真实账号或真机；数据 `lib/capabilityVerification.ts`，界面挂在「设置 › 关于与更新」（`CapabilitySection.tsx`，无画板）；方案、依据与 7 处记录矛盾见 `docs/verification/2026-11-17-capability-verification.md`。原文：**待鼠鼠拍板**。多数能力是「部分验证」（如 Trae 免费档通过、付费 / 企业缺样本），哪些算已验证、日期取哪份记录、在哪里显示（无画板，界面目前不单独展示能力）都需要定，不能由小的推断
- [x] 4.8 ZCode 余额遗留：账号卡解释免费档、经典灵动岛日额度、「BigModel 套餐剩余」文案（2026-11-17，按小的推荐做，文案无画板，待确认）：
  - 解释档位：不按套餐名推断「免费档」，改为照数据说明重置周期。余额接口每个 entitlement 带 `period`（实测 Start Plan 是 daily），后端 `coding_plan::parse_zcode_plan_balance` 把它写进窗口名，例如「GLM-5.3 · 每日额度」；日额度、月额度的 key 分别加 `1d:` / `30d:` 前缀，周额度只写进名称。账号卡与剩余额度的行标签改用这个窗口名，只对 ZCode 生效（描述符 `labelFromWindowName`），所以能看出是哪个模型、按日重置。
  - 经典灵动岛：原来没有 5h / 7d 窗口就画「不限量」彩虹条，ZCode 日额度用尽时也照样显示不限量。现在 `dockQuotaView` 在没有 5h / 7d、但有 `1d:` 额度桶时按日额度画，用得最多的在前。其他平台规则不变。
  - 文案：ZCode 本机这条路有两个来源，找到 BigModel 编码套餐 Key 时查 5h / 周窗口，否则用官方账号令牌查日额度桶。标题因此改为「ZCode 套餐剩余」，来源「本机 ZCode 登录账号 / BigModel 编码套餐 Key」，加载文案「正在查询 ZCode 套餐额度…」。API 连接的文案仍是 BigModel 编码套餐。
- [ ] 4.9 P2-1 `stall` 下载超时；P2-2 `netguard` 下载地址校验；P2-3 `proc` 统一子进程封装；N10 `filelock` 跨进程文件锁；N13 `beat` 时钟对齐
  - [x] P2-1（2026-11-16）：新模块 `stall.rs`（照 magpie `internal/stall`），下载只在连续 60 秒没收到数据时才掐断，不设总时长。接在更新安装包下载上：原来是插件 `timeout` 设成的 reqwest 总时长 600 秒，慢链路会半路被截断，断了的连接又要白等 10 分钟。其余流式读取都是网关转发，各自已有空闲超时；`web_fetch` 读小网页、有正文上限，不需要
  - [x] P2-2（2026-11-16 核对，无需改代码）：Jolu 去下载「自己没选过的地址」的只有 `web_fetch` 与搜索 API 解析出的地址，两处都已过 `gateway/net_guard.rs`；对照文档提到的「图标代理」在 Jolu 里不存在；后端不替调用方下载远程图片（Gemini 明确不下载，Qoder 原样转给上游）；其余请求是用户自己配的上游地址或固定厂商端点，按 `net_guard` 的设计不归它管
  - [x] P2-3（2026-11-16）：新模块 `proc.rs`（`kill_tree` / `kill_child_tree` / `wait_or_kill_tree`），Windows 用 `taskkill /T /F` 结束整棵树。替换了 3 份重复的 `kill_tree`（自动化、会话交接、Skills）和 5 处各写各的「超时就杀」循环：版本探测、warm-up、WSL 探测、Trae 签到、交接事实里的 git。实际修掉的问题有三个：warm-up 在本机找到的是 npm 的 `codex.cmd` 垫片，超时只杀 `cmd.exe`，node / codex 会把请求发完；WSL 探测每次超时多留一个 `wsl.exe`（magpie #2341）；git 超时后子进程拿着管道，读线程一直挂着。不改的两处：官方登录进程（它打开的浏览器归用户，取消时不能关）；网关的 Claude 一次性进程与桥接（只认原生 `claude.exe`、工具与 MCP 关闭，没有子进程）。单测用「外层 cmd 起内层 cmd」三条互相印证：不打断时孙进程会写标记文件；只杀外层时照样写出来；`wait_or_kill_tree` 超时后不会写
  - [x] N10（2026-11-17 随 5.5 完成，`filelock.rs`）：magpie 的 `filelock` 防的是同一台电脑上多个 magpie 进程同时改同一文件。Jolu 是单实例应用，`jolu_cli` 只读，现在没有第二个写入方，先做出来也没有调用方。第一个用户是 5.5 的 CLI 写命令，和它一起做
  - [ ] N13（跳过）：对照文档原话是「Jolu 的自动化是单线程 `wait_timeout`，无此问题」，属于可选的小优化
- [x] 4.10 `lib.rs` 拆分：命令按模块迁到 `service/` 与各 `commands` 文件——2026-11-16 完成「按功能区归类」这一层：
  - **搬了什么**：`lib.rs` 里全部 158 个 `#[tauri::command]` 及只服务于它们的辅助条目，搬到 `backend/src/commands/` 下 9 个文件：`panel`（主窗口与启动）、`island_window`（灵动岛窗口）、`usage_sessions`、`quotas`、`preferences`、`capabilities`（隐私 / 搜索 / Library）、`agents`、`routing`（模型路由）、`providers`。`lib.rs` 从 5281 行降到 2283 行，剩下共享状态、缓存、窗口 / 托盘 / 启动与 `run()`。
  - **怎么搬的**：每个子模块 `use crate::*;`，根模块 `use commands::*;` 导回，`generate_handler!` 列表与其他调用点一行不改；搬出的条目一律 `pub(crate)`。函数体不动：用行多重集合比对过，新旧两边只差模块头注释、`mod` / `use` 声明、删掉的 7 个空分区标题。另外把一段放错位置的文档注释从 `validate_restored_connection` 挪回 `enable_connection`，给前者补了一句注释。
  - **验证**：
    - 后端全量 1422 passed / 0 failed，无警告。
    - 隔离桌面跑 `main-window-smoke.e2e.mjs`：8 页里 5 页通过。会话线程、自动化、Skills 管理 3 页报「『正在读取』再次出现」。切回重构前的 HEAD 编译后对照，同样这 3 页失败，HEAD 那次用量统计还另外超时，所以这 3 页的失败是原有问题，与本次搬家无关。**2026-11-16 查明并修复**：这是冒烟脚本的判定缺陷，不是页面问题。旧判定不认第一次进入时的懒加载占位「正在加载<页名>」，在这个阶段就判定加载已结束，之后第一次加载稍慢就被误报成「再次出现」。当初写的「第二层加载区一直停在加载态」是推测，经核实不对。根因、证据与修法见 `docs/specs/2026-11-16-baseline-and-contracts.md` §5 第 5 条。
  - **没做的一层**：把业务逻辑从命令抽到 `service/`（不接 `AppHandle`、可给 CLI 复用）。这需要给发事件、重启监听等副作用逐个设计回调，等 5.5 CLI 写命令真正要复用哪个命令时再抽。
- [x] 4.11 早期路线图遗留：P0 基线文档、P1 契约文档——2026-11-16 以 `docs/specs/2026-11-16-baseline-and-contracts.md` 收尾。没有照 10-21 的计划补写两份过时文档，改为：
  - 逐条写明 P0 / P1 检查项的去向：已由 AGENTS.md「现有实现」列、`docs/todo/2026-10-22-unverified-integrations.md`、`platforms.rs` / `quota_registry.rs` 等承担；涉及旧项目数据兼容的几条按 AGENTS.md 作废；
  - 补上此前唯一没有文档的**事件目录**（30 个事件的载荷、发送方、监听方）与命令分布（231 个）；
  - 记下核对中发现的待查问题：`dnd-changed` / `island-platform-changed` / `gateway-catalog-changed` / `warmup-finished` 四个事件没有前端监听，`refresh-requested` 载荷有 `()` 与连接 id 两种，事件无统一序号 / 错误码，防旧响应回填没有统一模块。均未改代码

## 阶段 5 · 大模块

- [ ] 5.1 Library（G4 / N3）：指令、MCP server、skills、subagents 按各 agent 格式写入，移除只删自己写的
  - **部分完成（2026-11-16 核对）**：MCP stdio server 的安装 / 卸载 / 清单已有（`backend/src/library/`：按各 agent 自己的文件与格式写，Jolu 写的记在 `library_applied` 表，只删自己写的，不覆盖别人的同名条目，首批只有「Jolu 搜索」）；skills 由模块 F（`skills.rs`）按工具管理。**未做**：指令（AGENTS.md / CLAUDE.md 一类）与 subagents 的写入、远程（http / sse）MCP（依赖 5.2）
  - [x] 2026-11-17 共享指令与子代理（`backend/src/library/shared.rs`，参照 magpie `library/instructions.go` / `subagents.go` / `targets.go`）：一段共享指令写进 Claude Code / Codex / Gemini / OpenCode / Qwen / Droid 的用户级指令文件里 `<!-- jolu:begin … -->` / `<!-- jolu:end -->` 区块（区块外不动、保持换行风格）；子代理按各家格式写文件（md + frontmatter，OpenCode 加 `mode: subagent`，Codex 是 `agents/<名>.toml` role 文件、只有它写 model）。每处写入记哈希（表 `library_files`）：内容还等于写的才重写或删除，用户改过的跳过并说明，同名文件不是 Jolu 写的永不覆盖；卸载钩子一并移除。命令 `library_shared_state` / `library_instructions_save` / `library_subagent_save` / `library_subagent_remove`（走 `cli_switch` 跨进程开关）；界面 `components/settings/LibrarySharedSection.tsx` 挂「数据与存储」页尾（无画板，待确认），纯逻辑 `lib/librarySharedView.ts` 有测试；单测 8 条。未做：magpie 的多套指令集与按工具附加指令、子代理的 skills / MCP 引用、项目级子代理与从仓库导入、WSL
- [ ] 5.2 远程 MCP OAuth 集中登录（N1）
- [x] 5.3 Web 搜索 MCP（G1：九个后端、多 Key 轮转、`web_search` + `web_fetch`）——2026-11-16 核对已完成，此前漏勾：九个后端 tavily / brave / exa / firecrawl / you / deepseek / doubao / wsa / searxng（`gateway/search/backends.rs`）；多 Key 依次使用、被拒的排到最后（`search/mod.rs`）；`web_fetch` 逐跳过 netguard 并把连接钉在检查过的地址上（`search/fetch.rs`）；两个工具经 `library/mcp_search.rs` 以 stdio MCP 暴露给 agent；设置页「联网搜索 / 搜索插件」入口见 `2b2807c`（提交 `db2f13c`、`2b2807c`）
- [ ] 5.4 图像 / 视频生成 MCP（G2 / G3 / N11）
- [ ] 5.5 CLI 写命令（G5：`provider add` / `claude <model>` / `group add` / `save` / `use` / `settings set` / `serve`，走跨进程锁）——跨进程锁（4.9 的 N10 `filelock`）随本项一起做
  - [x] 2026-11-17 第一批：
    - `filelock.rs`（N10）：目标旁 `create_new` 建 `<文件>.jolu-lock`，50ms 重试到超时，超过 30 秒的遗留锁接管；8 线程各 10 次读改写不丢计数的单测。
    - 桌面端设置 `Store` 写入前拿同一把锁，磁盘被别的进程改过时以磁盘为底；后台线程每 2 秒读回外部改动并广播 `settings-changed`。
    - `jolu settings list|get|set`（`service/settings_write.rs`）只开放 13 个「用到时才读」的键（免打扰、通知、脱敏、网关熔断阈值与默认连接、warm-up 小时、默认页面等），只改这一个键、整份按 `Settings` 校验后原子写回，别的内容（含搜索 Key 原文）原样保留；端口、开机自启、保持唤醒等要做副作用的键仍在桌面端改。
    - `jolu claude run [--model <模型>] [参数…]`（`service/launch.rs`，参照 magpie `claude_launch_cli.go`）：写数据目录里的 `claude-launch.json`（env 指向本机网关 + `jolu-claude-code.<实例>` 令牌），用 `claude --settings` 启动；`--settings` 优先级高于用户 / 项目设置（Context7 核对 Claude Code `SettingsSource`），不改 `~/.claude/settings.json`。启动前检查模型路由开着且端口在听。
    - 隔离目录冒烟：`settings set` 改值、越界与非白名单键拒绝、`claude run` 在路由未开时说明原因；单测 9 条。
  - [x] 2026-11-17 第二批：`jolu group add <分组> <策略> <来源/模型>…` / `set` / `rm`（`service/gateway/route_groups.rs`），沿用 `routing_groups` 的校验（重名、嵌套成环、成员非空）；网关每次请求从库里读分组，写入即生效；「按意图」要判定模型，只在桌面端建。隔离目录冒烟通过，单测 2 条
  - [x] 2026-11-17 第三批（鼠鼠：「要做，按推荐」）：
    - `jolu provider list|add|rm`（`service/providers.rs`）：只收 API Key 连接，与桌面端「添加连接」同一条路（`prepare_new` → `quota::validate_connection` 真连一次上游 → 事务 `connections::add`）；Key 只从标准输入读，命令行参数里写 `--key` 直接拒绝；删除保留历史统计。官方订阅的登录凭据仍只在桌面端获取。
    - `jolu save <名称>` / `use <名称>` / `serve` 由运行中的桌面端代办（`cli_requests.rs`）：CLI 在数据目录 `cli-requests/` 写请求文件，桌面端后台线程每秒认领（改名）、执行（`gateway_profiles_save` / `gateway_profiles_apply` / `gateway_set_mode(relay)`），写回结果；5 秒没人认领 = 桌面端没在运行，CLI 撤回请求并说明。只接受这三个动作，手写的其他动作拒绝。
    - 桌面端每 2 秒看连接表与路由分组表的签名，命令行写库后广播 `connections-changed` / `gateway-catalog-changed`，界面随之刷新。
    - 验证：单测 5 条（请求往返、桌面端不在时撤回、拒绝伪造动作、参数归一、空库）；隔离目录冒烟：Key 写在参数里拒绝、空 Key 拒绝、对本地模拟上游 `provider add` 验证并保存、`list` 只显示掩码、`rm`、桌面端不在时 `use` 5 秒后说明原因。隔离桌面实例（`start.ps1 -Executable`，无前端页面，只验后台线程）上：`save` / `use` 把桌面端的报错原样带回（「Profile 至少要包含一个 agent 的模型」「没有名为…的 Profile」），`serve` 开启模型路由且 12731 由该实例监听、再次 `serve` 报「已在运行」，`claude run --model … --version` 用启动设置拉起 Claude Code 2.1.326 并正常退出，`settings set dnd on` 之后桌面端写设置未覆盖它；结束时只停本次两个 PID。没有在接过线的真实环境里切换 Profile
  - [ ] 不做（待确认）：真正无界面的 `serve`（网关依赖 Tauri 的 AppHandle，要先把网关与桌面端拆开；现在的 `serve` 是让运行中的桌面端开模型路由）
- [x] 5.6 WSL 接线写入（G10 剩余 / P2-6）——2026-11-17（无画板，待确认）：
  - `gateway/wiring_wsl.rs`（参照 magpie `agent/wsl.go`）：Claude Code / Codex / OpenCode 三家，复用 `wiring.rs` 的接线纯函数，文件经 `\\wsl.localhost\<发行版>\<家目录>\…` 读写，写前备份、快照记在 `wsl_wiring` 表，断开时文件里仍是 Jolu 写的地址才还原，卸载钩子一并还原。
  - 网络：`wslinfo --networking-mode`（没有时看 `.wslconfig` 的 `[wsl2] networkingMode=mirrored`）。mirrored / virtioproxy 走 127.0.0.1 与本机令牌；NAT 只在**用户自己开了局域网共享**时接，经发行版默认路由里的 Windows 地址，并给这个工具签一把专用客户端 Key（`wsl-<发行版>-<工具>`，断开撤销，重接换新）；否则拒绝并说明「改 mirrored」或「自己开局域网共享」两条路。Jolu 不替用户开局域网共享。
  - 坑：探测脚本里不能用 awk 的 `$3`——`wsl.exe` 转交参数时会再展开一次，改用 `cut`。
  - 命令 `wsl_wiring_state` / `wsl_wire` / `wsl_unwire`；界面 `components/settings/WslWiringSection.tsx` 挂「数据与存储」页尾，纯逻辑 `lib/wslWiring.ts` 有测试。
  - 验证：单测 4 条（网络解析与 `.wslconfig` 兜底、路由判定、三家接入 / 重接 / 还原往返、用户改走后不动）；本机真 WSL（Ubuntu，NAT，主机地址 172.30.224.1）经 UNC 在 `/tmp` 下往返写入与还原通过（`real_wsl_round_trip_under_tmp`，`--ignored`）。本机是 NAT 且未开局域网共享，没有真的把 WSL 里的工具接到网关发请求；mirrored 模式没有真机样本
- [ ] 5.7 TUI 与 Web 管理界面（G6）
- [ ] 5.8 远程网关（G7）、Docker 镜像（G8）、系统服务（N12）
- [ ] 5.9 同步备份（G9：WebDAV / S3 / 加密 GitHub）
- [ ] 5.10 `tune` 成本调优建议（N9）、`codexsummary` 替 Codex 压缩（N15）、OIDC 换网关 Key（N8）、浏览器扩展（N6）
  - [x] 2026-11-17 `codexsummary`（`gateway/responses_chat.rs`，参照 magpie `internal/codexsummary`）：Codex 以为对面是 OpenAI 时（保留 ChatGPT 登录的接线）压缩走 V2 协议——Responses 请求里带 `{"type":"compaction_trigger"}`，要求回复恰好一项 `{"type":"compaction","encrypted_content":…}`，之后每轮原样放回（Context7 核对 openai/codex `compact_remote_v2.rs` / `model-provider-info/capabilities.rs`）。以前第三方模型的转换层把这两种项静默丢掉，压缩失败。现在：触发项换成 Codex 自己的压缩提示词（Apache-2.0）且不带工具，回复文字包成 `jolu1:`+base64 的压缩项（非流式与流式都只给这一项），后续轮次还原成「摘要前缀 + 摘要」的用户消息；OpenAI 封装的压缩项第三方读不了，跳过。所有经 `responses_chat` 的路径（API 来源、Gemini / WorkBuddy / Qoder / Trae / ZCode 订阅、Anthropic 协议来源）都覆盖；Grok 直接转发到 xAI 的 Responses 接口，xAI 认不认压缩触发项未验证（待确认）。走 OpenAI 自家模型（Codex 订阅）时请求体与回包原样透传，不经这层。验证：单测 2 条（非流式往返：触发项变提示词且不带工具、OpenAI 封的项跳过、回复恰好一项、下一轮还原；流式只发一项压缩项、不发文字增量）；**真实 Codex 0.169 端到端通过**——隔离桌面实例 + 记录消息的合成 Chat 上游 + 隔离 `CODEX_HOME`（自建 provider 声明 `capabilities.remote_compaction = "v2"`，与保留登录时内置 OpenAI provider 走同一协议；`model_auto_compact_token_limit` 设低）：第一轮正常，`codex exec resume` 的下一轮先发压缩请求（上游收到 Codex 压缩提示词、0 个工具），Codex 显示 `context compacted` 并在会话文件里存下 `jolu1:` 压缩项，随后那一轮上游收到「摘要前缀 + 摘要」再接新问题。没有用真实 ChatGPT 登录跑（隔离环境不读本机凭据）
  - 同时修正 5.5：`jolu settings set` 白名单去掉 `gateway_default_connection` / `gateway_circuit_threshold` / `gateway_auto_escape`——它们在模型路由启动时取快照（`proxy::start` 建 `ProxyDeps`），命令行改了要等下次启动才生效；桌面端读回外部改动时同步隐私脱敏与 ZCode 通道两个进程级开关；新增 `jolu provider serve <id> on|off`（网关每次请求从库读模型池，写入即生效）。**已修（2026-11-17）**：桌面端自己的「设为默认来源」原先也只改设置、运行中网关用的是启动时的快照，要重启模型路由才生效。`ProxyDeps` 的默认来源与熔断阈值改成每次读当前设置（`LiveSetting`），自动逃生在熔断那一刻读当前设置；命令行重新开放熔断阈值与自动逃生，并新增 `jolu provider default <id>|none`（同桌面端的校验：必须在模型池、有前缀）。隔离桌面实例验证：目录外的裸模型名先报「未带来源前缀」→ 设默认来源 2 秒后同一个监听进程（PID 不变）转发成功 → 清掉后又报错
  - [x] 2026-11-17 `tune`（`backend/src/tune.rs`，移植 magpie `internal/tune/tune.go`）：从 `requests` 表取最近 N 天成功、用量全部已知、有会话标识的调用，按会话逐条回放，给 Claude Code（按模型）与 Codex 的压缩窗口建议、Claude Code 提示词缓存 5 分钟 / 1 小时建议；按厂商收费比例加权，省不到 3% 不建议改；窗口取 models.dev 元数据，不知道时提示「模型窗口未知」。与 magpie 的差异：Jolu 记录里没有 1 小时缓存写入的拆分、子代理标记和压缩耗时，所以当前缓存时长按 5 分钟推断并写明、不算等待时长、不分主会话与子代理。只给建议、不改配置（「一键应用」待确认）。入口：`jolu tune [--days N]` 与设置「数据与存储」页尾（`TuneSection.tsx`，无画板，待确认）；单测 10 条 + 前端 3 条；本机真实记录只读跑通（30 天 Claude Code 3.4 万次、Codex 6 千次调用，约 1 秒）
- [ ] 5.11 更多 agent 接线（权威版 §1.1：Antigravity、Zed、Factory、MiMo、Copilot、Hermes 等）
  - [x] 2026-11-17 Pencil、Qoder、Qoder CN、Crush 专用接线（`gateway/wiring_extra.rs`，参照 magpie pencil.go / qoder.go / agents.go 的 crush；单测 6 条；`agents` 种子表与 `wiring::AGENTS` 同步；Crush 在 Windows 的目录改为 `%LOCALAPPDATA%\crush`，探测与接线共用）。未做真实工具验收；不写推理强度、不保留用户在 `providers.jolu` 里手改的字段（magpie theirsKept）标「待确认」
  - [x] dsh（2026-11-18）：多 profile cordis.patch.yml 行级拼接 + .env + credentials refs，见 `backend/src/gateway/wiring_dsh.rs`；真机隔离往返通过
- [ ] 其余 9 家（qwen、cline、droid、zed、goose、continue、copilot、vscode、workbuddy）需要 yaml / 数组插项 / 多文件 / 凭据库写入，见 `docs/specs/2026-11-17-agent-wiring-registry.md` §8

---

每完成一项在此勾选，并在对应阶段的提交信息与 `docs/verification/` 记录里写明验证结果。
