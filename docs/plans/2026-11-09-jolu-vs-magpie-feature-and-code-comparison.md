# Jolu vs magpie · 全功能与代码质量对照（2026-11-09）

**交给：鼠鼠**
**参照物：** [yetone/magpie](https://github.com/yetone/magpie) @ `627fd1a`（2026-11-08），本地克隆 `/tmp/magpie`
**Jolu 基线：** `dc79c08`（后端 1121 测试全绿 / 前端 429 全绿 / tsc 通过）
**与另一份文档的分工：** [`2026-11-09-magpie-parity-full-audit-authoritative.md`](./2026-11-09-magpie-parity-full-audit-authoritative.md) 回答「**还差什么、按什么顺序补**」；**本文回答「两边各有什么、谁做得更好、代码层面谁更优」**。

---

## 0. 一句话结论

| 维度 | 谁更强 | 说明 |
|---|---|---|
| **功能广度** | magpie | 支持 60+ agent 适配、9 个搜索后端、插件市场、TUI、Web UI、Docker |
| **功能深度（核心链路）** | 平手 | 网关选路、中间件、Key 限额、额度、脱敏两边都有；Jolu 的脱敏更保守 |
| **桌面 UI 完成度** | **Jolu** | magpie 的 GUI 是单页 + JS 补丁；Jolu 是完整设计稿驱动的多页应用 |
| **代码可测性** | **Jolu** | 1121 个 Rust 测试 vs magpie 大量靠 cgo/真实进程；Jolu 纯逻辑可测比例更高 |
| **代码组织** | magpie（局部） | magpie 一个包一件事，注释详尽到「谁要求的、为什么」；Jolu 的 `lib.rs` 仍有 4,446 行、几百个 tauri 命令堆在一起 |
| **数据安全** | magpie | `lastgood` 回滚、`steady` 竞态处理、`filelock` 跨进程锁，Jolu 全缺 |
| **可维护性（长期）** | **Jolu** | Rust 类型系统 + 编译期保证；magpie 的 `map[string]any` 满天飞 |

---

## 1. 功能正面对照

### 1.1 支持的 Agent / 平台

| | Jolu | magpie |
|---|---|---|
| 数量 | **10 个**：Claude Code、Codex、Gemini、Grok、OpenCode、ZCode、Qoder、WorkBuddy、Trae、自定义 | **148 个非测试适配文件**（`internal/agent/`，含同一 agent 的多平台分支，实际 agent 数十个）：含 Antigravity、Zed、Factory、Cursor、Copilot、Kimi、MiniMax、Cline、Goose、AionUi、Devin 等 |
| 形式 | 内置 Rust 适配器 | 内置 + 插件（`agentplug`）双轨 |
| **优劣** | Jolu 胜在「每个都做深」（登录、额度、Skills、签到都接）；magpie 胜在「广度」但很多 agent 只做最小接线 | |

### 1.2 网关核心

| 能力 | Jolu | magpie | 优劣 |
|---|---|---|---|
| 模型 id 解析（`[1m]` 剥离、裸名兜底） | ✅ | ✅ | 平 |
| 路由分组（order/rotate/usage/smart/pace） | ✅ 5 策略 | ✅ | 平 |
| 分组成员暂停 / 额度用满降权 | ✅ 本轮补齐，含重置过期失效语义 | ✅ | **Jolu 略优**：magpie 只有「休息」，Jolu 多了「快照过期即失效」 |
| 中间件链 | ✅ 5 个内置（model-map/param-override/system-prompt/word-guard/think-tags） | ✅ 同 5 个 + JS 异步钩子 | magpie 多 JS 钩子；Jolu 缺 |
| 命名 Key 约束 | ✅ ttl / 模型白名单 / 只读 / 请求数上限 | ✅ 还多 `accounts`（限定账号）、`cost`（金额上限） | magpie 更全 |
| `tokens_per_day` **强制** | ❌ 只存不拦 | ✅ `internal/budget` 预扣回冲 | **magpie 胜，且这是 Jolu 的「说了不生效」** |
| 隐私脱敏 | ✅ 稳定占位符 + SSE 逐块回填 + **引擎不可用 503 拒发** | ✅ `internal/redact` + `maskrec`（只通知一次） | **Jolu 更保守**（宁可拒发）；magpie 多了「首次脱敏通知」 |
| 局域网共享 | ✅ 开关 + 严格校验 + Key 管理 UI | ✅ `MAGPIE_ALLOW_IPS` IP 白名单 | 平（形态不同） |
| ZCode Start Plan 通道 | ✅ 完整着装协议（三块系统提示逐字节一致） | ✅ 同 | 平 |
| ZCode trial 按模型粒度休息 | ❌ | ✅ `zcode_trial.go` | magpie 胜 |
| 渠道记忆（C2） | ❌ | 未在本版本核实到 | — |

### 1.3 Claude Code 接线深度

| 项 | Jolu | magpie |
|---|---|---|
| 删 `ANTHROPIC_MODEL`、写顶层 `model` | ✅ | ✅ |
| 4 档 env 跟随（opus/sonnet/haiku/fable） | ✅ | ✅ |
| 每档 `_NAME` / `_DESCRIPTION` | ✅ | ✅ |
| `_SUPPORTED_CAPABILITIES` | ❌ 不写（不猜能力） | ✅ 写 |
| subagent 独立模型 | ✅ | ✅ |
| 云端开关 | ✅ 6 项 | ✅ 6 项 |
| 档位 effort 联动 | ❌ | ✅ |
| 断线还原快照 | ✅ 逐 agent 快照 | ✅ |

**结论**：Claude Code 这条线 Jolu 已追到 85%，缺 effort 联动与能力声明两项。

### 1.4 用量与成本

| 能力 | Jolu | magpie | 优劣 |
|---|---|---|---|
| 用量统计（平台/模型/会话） | ✅ | ✅ | 平 |
| 模型计价 + 币种换算 | ✅ 含当日汇率 | ✅ | **Jolu 略优**（币种切换） |
| 调用明细分页 | ✅ | ✅ | 平 |
| 预算上限 | ❌ 只有不生效的 tokens_per_day | ✅ `budget` + `keyspend`（Key 花费上限） | magpie 胜 |
| 手工录入余额递减 | ❌ | ✅ `keybalance.js` | magpie 胜 |
| 成本调优建议 | ❌ | ✅ `tune`（从真实行为反推最省 token 的设置） | **magpie 独有的思路** |

### 1.5 工具生态

| 能力 | Jolu | magpie |
|---|---|---|
| Web 搜索 MCP | ❌（只是透传 `<model>/web_search` 翻译） | ✅ **9 后端**：Tavily / Brave / Exa / Firecrawl / You.com / DeepSeek / 豆包 / 元宝 / SearXNG，多 Key 轮转 |
| 图像生成 MCP | ❌ | ✅ `imagemcp` |
| 视频生成 | ❌ | ✅（Grok 订阅 / MiniMax） |
| 远程 MCP OAuth 集中登录 | ❌（MCP 只有 stdio） | ✅ `mcpauth`（Notion/Neon/Supabase 登录一次共用） |
| Library（统一写 MCP/subagents/指令） | ⚠️ 只有 Skills | ✅ `library` 59k 行 |
| 浏览器扩展（用用户已登录的标签页） | ❌ | ✅ `browserext` + `browsermcp` |
| 插件市场 | ❌ | ✅ `plugins.js`（npm registry 发现/安装） |

### 1.6 分发与运维

| 能力 | Jolu | magpie |
|---|---|---|
| 自更新 | ✅ 签名 + latest.json + 被动安装重启 | ✅ `update` |
| CLI | ⚠️ 5 个只读子命令 | ✅ 全功能（含 `serve` / `tui` / `claude <model>`） |
| TUI | ❌ | ✅ `internal/tui` |
| Web UI | ❌ | ✅ `internal/gui` 的 `app.js`（50k 行 JS） |
| Docker | ❌ | ✅ `-tags nogui` 镜像 |
| 装成系统服务 | ❌（桌面 + 托盘） | ✅ `daemon`（systemd / launchd） |
| Keep awake | ✅ 本轮 | ✅ 三平台 |
| OTLP 导出 | ✅ 本轮 | ✅ |
| 同步备份 | ❌ | ✅ `davsync`（WebDAV）+ `backup`（自动备份 + 版本回退） |
| WSL | ⚠️ 只读探测 | ✅ 真接线 |
| OIDC 换 Key | ❌ | ✅ `oidc`（CI 不用存 Key 到 secrets） |

### 1.7 Jolu 独有（magpie 没有）

这是容易被忽略的一半——**Jolu 在桌面产品层面其实是领先的**：

| 能力 | 说明 |
|---|---|
| **灵动岛**（浮动 + 胶囊两套样式） | 实时 token 轮播、视线跟随、贴边迷你胶囊、额度环、表情。magpie 无对应 |
| **一键签到** | 6 个品牌卡、批量签到、官方每日奖励接入、统计按来源区分。magpie 只有 `checkinnow.js` 单个手动签到 |
| **会话线程** | 会话列表、搜索、在原工具继续、压缩会话（MD 交接 + 对话式修订）。magpie 有 `sessions` 但与 agent 集成方式不同 |
| **自动化（定时任务）** | 任务列表、等待额度续跑、运行记录、Skills 选择、执行账号校验。magpie 无独立自动化模块 |
| **Skills 管理** | 8 个 agent 的 skills 目录，含启用/停用（Claude Code 移到停用区）、Git 仓库导入、草稿与磁盘差异 |
| **完整设计稿驱动 UI** | `Jolu.pen` 179 个画板，浅深色成对，颜色走变量。magpie 是单页 + 独立 JS 补丁逐个加功能 |
| **诊断与反馈** | 网络诊断、数据导出/导入预览、清理预览、状态与反馈页 |

---

## 2. 代码层面的对照

### 2.1 体量与结构

| | Jolu | magpie |
|---|---|---|
| 语言 | Rust（后端）+ TypeScript（前端） | Go（后端 + GUI）+ JavaScript（插件/前端） |
| 后端非测试行数 | 80,599 | **485,714** |
| 网关包规模 | `gateway/` 约 25 文件 | `internal/gateway/` **119,143 行** |
| 最大单文件 | **`lib.rs` 4,446 行**（含全部 tauri 命令） | 最散：`internal/gateway` 298 个文件，平均 400 行 |
| 前端 | React 19 + TS，按功能分 13 个组件区 | 39 个独立 `.js` 文件按需 import，50,944 行 |

**优劣**：

- **Jolu 的问题**：`lib.rs` 是个巨石。本轮抽了 5 个高频命令到 `service/`，但还有几百个命令在里面。改一处要重新读上下文。
- **magpie 的优势**：一个包一件事，`internal/steady`（225 行）就只解决 rename 竞态。找东西靠包名就行。
- **magpie 的代价**：298 个文件全在 `gateway` 包里，同样是巨石，只是拆成了文件而不是目录。

### 2.2 类型安全

| Jolu | magpie |
|---|---|
| Rust 枚举 + 结构体，非法状态编译期拦 | `map[string]any` 满天飞；`provider/search_api.go` 里 500 行全是 map 查找 |
| 例：`QuotaRoute` 枚举 —— 查询链的 9 种情况由类型穷举，漏一个分支编译报错 | 例：magpie 靠字符串比较 `status == "EFFECTIVE"`，拼错不报错，运行时静默走错分支 |

**Jolu 明显更优。** 这是本轮抽 `service::gateway::quota` 时最有价值的收获：原来埋在 100 行命令体里靠 `return` 顺序表达的 9 条分支，现在是一个枚举 + 一个 `match`。

### 2.3 测试

| | Jolu | magpie |
|---|---|---|
| 后端测试数 | **1121**（1,098 个 `#[test]` 函数，测试框架展开后 1121 条） | 大量 `_test.go`，测试行数 **≈ 实现行数**（`budget` 651/259、`steady` 225/260、`stall` 66/69） |
| 特点 | 纯逻辑可测比例高；真实进程类测试标 `#[ignore]` | 测试同样详尽，但很多要起真实 HTTP server / 真实进程 |

**平手**，但取向不同：
- Jolu 的测试偏「纯函数 + 内存库」，跑得快（10 秒跑完 1121 个）
- magpie 的测试偏「真实场景」，`steady` 的 260 行测试全是为了覆盖 Windows 那 2ms 窗口

### 2.4 注释质量

| Jolu | magpie |
|---|---|
| 中文注释，说明「为什么这么写」「边界在哪」 | **英文注释，几乎每个非平凡函数都写「谁提的需求、为什么这么改、出过什么事故」** |

magpie 的注释是本轮最让我意外的发现。举例：

```go
// zcodeEndedWords and zcodeSpentWords are the ZCode plugin's own answers
// for a Start Plan trial entry (<model>-Trial) it can't serve, given before
// anything is sent: a 400 "ZCode has no active limited quota for
// GLM-5.3-Trial" when no live plan holds the model any more ...
// (#3661, orion-nebula-dev: two expired trials rested "zcode@zcode")
```

**带 issue 号 + 带用户 ID + 带事故描述。** 这解释了「为什么需要这个判断」，而不只是「这段代码做什么」。

Jolu 的注释质量不差（也写边界），但缺少「**这条规则是谁踩坑踩出来的**」这层历史。

### 2.5 错误处理

| Jolu | magpie |
|---|---|
| `Result<T, String>` 为主；近期新模块改用具体枚举 | `error` + 大量 `if err != nil { return }` |
| 例：`ensure!` 式的「未知值 / 真实零 / 不支持 / 查询失败」四态分离 | 例：多处 `return nil` 把「没查到」和「出错」混为一谈 |

**Jolu 更好**，但 magpie 有个 Jolu 缺的东西：**`lastgood` 兜底**——文件读不出时回读上一版好的，而不是给默认值。

### 2.6 并发与资源

| Jolu | magpie |
|---|---|
| Tokio 异步 + `spawn_blocking` 分流同步 IO | 原生 goroutine + channel |
| 本轮新增 `keep_awake` 用常驻线程持有线程局部状态 | 同类问题用 `beat` 统一时钟对齐 |
| **缺**：统一的跨进程文件锁（只有 `cli_switch` 一处命名互斥量） | `filelock` 139 行，覆盖所有状态文件 |

### 2.7 一个具体的代码质量对照

同样是「下载超时」，两边思路差一个维度：

**Jolu 现状**（`updater.rs` / `models_dev`）：
```rust
.timeout(Duration::from_secs(20))   // 整体 wall-clock
```
慢速大文件（140 KB/s 下 83MB 需要 10 分钟）会在第 20 秒被掐断，**尽管字节还在持续到达**。

**magpie**（`internal/stall`，66 行）：
```go
// 只判「上次收到字节距今多久」，不判总时长
```
注释里直接写了踩坑记录：`#5572: the 83 MB Windows update under ~140 KB/s against 10 minutes`。

**66 行解决了 Jolu 一个真实的用户可见故障。** 这就是为什么我说 P0 那几项性价比极高。

---

## 3. 结论表：谁该向谁学

### Jolu 该向 magpie 学的（按性价比排序）

| # | 项 | Go 行数 | 为什么值得 |
|---|---|---|---|
| 1 | `stall` 下载超时 | 66 | 66 行解决真故障 |
| 2 | `netguard` 地址校验 | 97 | 安全，且逻辑简单 |
| 3 | `lastgood` 配置回滚 | 276 | 防丢配置 |
| 4 | `steady` rename 竞态 | 225 | 防静默丢数据 |
| 5 | `budget` 预扣回冲 | 651 | 让 `tokens_per_day` 真生效 |
| 6 | `library`（只翻 Jolu 已有 8 agent） | 8–10k | 补齐 MCP/subagents 写入 |
| 7 | Web 搜索 MCP | — | 最显眼能力缺口 |
| 8 | 注释风格 | — | 加「谁踩的坑、issue 号」 |

**前 5 项合计 1315 行 Go，翻译成 Rust 约 5–7 天**——这是最该先做的。

### magpie 该向 Jolu 学的

| # | 项 | 说明 |
|---|---|---|
| 1 | 类型驱动的分支穷举 | 把 `map[string]any` 换成枚举 + match |
| 2 | 桌面 UI 完整度 | Jolu 有设计稿驱动的多页应用，magpie 是单页 + JS 补丁叠加 |
| 3 | 灵动岛 / 签到 / 自动化 / Skills 管理 | magpie 完全没有对应模块 |
| 4 | 浏览器登录态复用 | Jolu 的 `*_browser_auth` 系列比 magpie 的 `browserext` 更贴合国内厂商 |

---

## 4. 复现方式

```bash
# magpie 侧
cd /tmp/magpie
for d in internal/*/; do n=$(basename $d); l=$(cat $d*.go 2>/dev/null | grep -v _test | wc -l); echo "$n|$l"; done | sort -t'|' -k2 -rn
head -12 internal/steady/*.go | grep '^//'

# Jolu 侧
cd /d/develop/Jolu
git log --oneline | wc -l
grep -rc "#\[test\]" backend/src --include=*.rs | awk -F: '{s+=$2} END {print s}'
wc -l backend/src/lib.rs
```
