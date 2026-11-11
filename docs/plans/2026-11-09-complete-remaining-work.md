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

## 交接说明（2026-11-09 收尾，给接手的同事）

**进度：** 阶段 0 完成（`fe29dc1`）；阶段 1 完成（1.1–1.8 `0a8c042`，1.9–1.10 于 2026-11-11）；阶段 1B（同事批次遗留）进行中；阶段 2–5 未开始。以下勾选框为准。

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
- [ ] 2.2 普通来源失败换同模型其他连接重试（最多 3 个）；402 `exhausted` 次日零点自动恢复
- [ ] 2.3 Grok / WorkBuddy / ZCode 订阅转发的用量入库
- [ ] 2.4 C3 团队席位状态校验；C4 每账号并发队列；P2-8 `-trial` 模型按模型粒度休息
- [ ] 2.5 Claude Code：D2 effort 联动、D4 `_SUPPORTED_CAPABILITIES`、单档覆盖、keyHolds（保留调用方已有 Key）、顶层 model 冲突提示；流水记 `resolved_from=bare`
- [ ] 2.6 命名 Key：`accounts` 账号范围、`cost` 每日金额上限；周 / 月 Token 与费用限额；按账号 / Key 出账
- [ ] 2.7 接线漂移增加「网关不可达」；改来源 id 联动已写接线；分组成员档位 `provider/model:effort`、权重与轮位、rest 休置；Profile 存档位 / 默认来源 / 常用模型，流水记「来自 Profile」；Profile 切换遇漂移先确认
- [ ] 2.8 models.dev 模态与推理档位、手动刷新命令、内置兜底表
- [ ] 2.9 ZCode 接线写 rules 文件（防 32000 截断）；P2-9 IP 白名单（`JOLU_ALLOW_IPS` 同款）
- [ ] 2.10 中转模式端口被占拒绝启动的单测；写后校验（写完重新解析一遍）

## 阶段 3 · 协议能力

- [ ] 3.1 Claude 订阅：工具调用闭环、流式、图片（真实账号验收归鼠鼠）
- [ ] 3.2 Gemini 订阅：流式与 Chat / Responses 转换
- [ ] 3.3 Codex 的 Responses → Chat 翻译（给只支持 Chat 的第三方来源）
- [ ] 3.4 ZCode 协议转换（非 Anthropic 入口）
- [ ] 3.5 Trae：Max 派生模型、GLM / DSML 文本工具模板、选路评分（名称多数票、misnamed）
- [ ] 3.6 WorkBuddy：systemAsUser 重试、非流式适配
- [ ] 3.7 中间件 `onEvent` 流式钩子；隐私脱敏 Insights 对照页、豁免清单、警告 / 拦截 / 脱敏三档、首次脱敏通知、与交接共用引擎
- [ ] 3.8 E1 JS 异步钩子（`onRequest` / `onResponse`，失败回滚）
- [ ] 3.9 E3 意图路由（小模型判意图选强 / 便宜模型）

## 阶段 4 · 其他模块与工程质量

- [ ] 4.1 字号放大 10→11、11→12，同步 `Jolu.pen`
- [ ] 4.2 灵动岛：「等额度」表情数据源；请求进入时额度环切到被调用来源；联动调研 P0–P2（会话行显示来源、点岛跳会话 / 用量页、今日按来源分解、「正在写」三档、`island-focus` 等事件）；月窗口环
- [ ] 4.3 额度描述符：后端收敛到注册表、运行时置信度、读转发响应的限流头、`PlatformId` 放宽、虚构供应商渲染快照测试
- [ ] 4.4 价目：模型行悬停显示价目来源与核对日期
- [ ] 4.5 Grok / Trae 会话与用量采集
- [ ] 4.6 系统通知冷启动应用
- [ ] 4.7 自动化区分额度耗尽 / 短时限流 / 余额不足；无模型时的本地交接导出；平台能力「未验证」三态与验证日期；夏令时调度测试
- [ ] 4.8 ZCode 余额遗留：账号卡解释免费档、经典灵动岛日额度、「BigModel 套餐剩余」文案
- [ ] 4.9 P2-1 `stall` 下载超时；P2-2 `netguard` 下载地址校验；P2-3 `proc` 统一子进程封装；N10 `filelock` 跨进程文件锁；N13 `beat` 时钟对齐
- [ ] 4.10 `lib.rs` 拆分：命令按模块迁到 `service/` 与各 `commands` 文件
- [ ] 4.11 早期路线图遗留：P0 基线文档、P1 契约文档

## 阶段 5 · 大模块

- [ ] 5.1 Library（G4 / N3）：指令、MCP server、skills、subagents 按各 agent 格式写入，移除只删自己写的
- [ ] 5.2 远程 MCP OAuth 集中登录（N1）
- [ ] 5.3 Web 搜索 MCP（G1：九个后端、多 Key 轮转、`web_search` + `web_fetch`）
- [ ] 5.4 图像 / 视频生成 MCP（G2 / G3 / N11）
- [ ] 5.5 CLI 写命令（G5：`provider add` / `claude <model>` / `group add` / `save` / `use` / `settings set` / `serve`，走跨进程锁）
- [ ] 5.6 WSL 接线写入（G10 剩余 / P2-6）
- [ ] 5.7 TUI 与 Web 管理界面（G6）
- [ ] 5.8 远程网关（G7）、Docker 镜像（G8）、系统服务（N12）
- [ ] 5.9 同步备份（G9：WebDAV / S3 / 加密 GitHub）
- [ ] 5.10 `tune` 成本调优建议（N9）、`codexsummary` 替 Codex 压缩（N15）、OIDC 换网关 Key（N8）、浏览器扩展（N6）
- [ ] 5.11 更多 agent 接线（权威版 §1.1：Antigravity、Zed、Factory、MiMo、Copilot、Hermes 等）

---

每完成一项在此勾选，并在对应阶段的提交信息与 `docs/verification/` 记录里写明验证结果。
