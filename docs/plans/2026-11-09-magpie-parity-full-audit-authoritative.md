# magpie 功能对标 · 全量盘点（权威版 · 2026-11-09）

> **本文是全仓库 magpie 差距的唯一权威依据。**
> 与本文冲突时，以本文为准；`2026-11-08-magpie-parity-roadmap.md` 与 `2026-11-09-magpie-parity-batch3.md` 的差距表**已过期**（保留作为历史记录，不再据此排期）。

| 项 | 内容 |
|---|---|
| **文档类型** | 差距盘点（gap audit），不是执行计划 |
| **覆盖范围** | magpie 全部 86 个 `internal/` 包，按代码量排序逐个核实 |
| **结论** | 原差距表有 **5 处与 magpie 实际代码不符**；另有 **13 个原表完全没有列出的能力** |
| **待办规模** | P0 约 5 天 / P1 约 14 天 / P2 约 12 天 |
| **核实基线** | magpie `627fd1a`（2026-11-08），Jolu 后端 1121 测试全绿时点 |

**交给：鼠鼠**
**参照物：** [yetone/magpie](https://github.com/yetone/magpie) @ `627fd1a`（2026-11-08），本地克隆 `/tmp/magpie`
**方法：** 遍历 magpie 全部 86 个 `internal/` 包，按代码量排序后逐个读包级 doc comment，再对 Jolu 侧做 grep 验证「有 / 没有」

---

## 0. 先纠正原表的错误

| 原表条目 | 实际情况 |
|---|---|
| C1「Start Plan 礼包按余额过滤」（`startPlan()` / `giftNames`） | **两个符号都不存在**。真实机制是 `internal/gateway/zcode_trial.go::zcodeTrialRefused`：按上游返回的两种措辞识别「这个 `-trial` 模型不能用了」，只让**该模型**休息，账号其余模型继续服务 |
| C4「每账号并发队列」（`DEFAULT_CONCURRENCY`） | 本版本搜不到，**此项作废** |
| C5「边缘防火墙保持」（`edgeHeld`） | 本版本搜不到，**此项作废** |
| G8「Docker 镜像 + Web UI 管理」 | `Dockerfile` 用 `-tags nogui` 构建，`CMD ["serve"]` —— **镜像里没有 Web UI** |
| G7「远程网关 = 笔记本用桌面的 providers/groups」 | 实际是 `internal/gateway/allow.go::AllowIPs`（`MAGPIE_ALLOW_IPS`）+ `MAGPIE_ADDR=0.0.0.0:3425`，即「监听全网 + IP 白名单」 |

---

## 1. 按 magpie 包体量排的全量对照

代码量取「非测试 Go 文件行数」，用来判断 magpie 在这个方向投入了多少。Jolu 状态列：✅ 有 / ⚠️ 部分 / ❌ 没有。

### 1.1 大块头（> 4000 行）

| magpie 包 | 行数 | 是什么 | Jolu | 说明 |
|---|---|---|---|---|
| `gateway` | 340 624 | 网关核心：选路、适配、转发、休息、trial 识别、IP 白名单 | ⚠️ | 主链路已对齐（选路、分组、中间件、额度快照、cap hold）；缺 `zcode_trial.go` 的按模型粒度休息、`allow.go` 的 IP 白名单 |
| `provider` | 191 068 | 全部厂商账号模型：席位、额度、搜索 API、账户排序与权重 | ⚠️ | 账号/额度/席位大部分有；缺 `search_api.go`（9 个搜索后端）、`account_weight.go`（账号权重） |
| `agent` | 131 016 | 各 agent 的接线与配置写入 | ⚠️ | 已接 Claude Code / Codex / Gemini / OpenCode / Qoder / Trae / ZCode / WorkBuddy；magpie 还有 Antigravity、Zed、Factory、MiMo、Copilot、Hermes 等 |
| `gui` | 74 437 | Go 编译进主程序的 GUI | ❌ | Jolu 用 Tauri + React，**不需要对齐**（形态不同） |
| `sessions` | 62 860 | 会话解析、压缩、交接 | ⚠️ | Jolu 有 `session_threads.rs` / `session_handoff.rs`；magpie 还多 `tune`（见 1.3） |
| `library` | 59 273 | 把指令 / MCP server / skills / subagents 按**各 agent 自己的格式**写入 | ❌ | **Jolu 完全没有**。Jolu 的 Skills 只管 skills 目录，不写 MCP server、subagents、指令文件 |
| `cli` | 35 451 | 全功能 CLI | ⚠️ | Jolu 现只 `status` / `catalog` / `group list` / `keys` / `quota` 五个**只读**子命令 |
| `usage` | 32 331 | 用量记录与聚合 | ✅ | 已对齐（Jolu `usage_stats.rs` + 模型计价） |
| `plugin` | 28 989 | 插件系统（清单、安装、生命周期） | ❌ | magpie 把订阅适配外置成插件；Jolu 内置。**这是产品路线选择，不是技术欠账** |
| `redact` | 15 557 | 隐私脱敏引擎 | ✅ | Jolu 的 `gateway/mask.rs` 同类，且更保守（引擎不可用 503 拒发） |
| `davsync` | 14 568 | WebDAV 同步 | ❌ | 见 1.3 |
| `update` | 10 014 | 自更新 + agent 版本管理 | ✅ | Jolu 有 `updater.rs` + 本轮新增的 `agent_version.rs` |
| `proc` | 8 694 | 子进程管理（取消、树杀、超时） | ⚠️ | Jolu 各处直接 `Command`，无统一封装；本轮 `warmup_run` / `agent_version` 也是各写各的 `wait_with_timeout` |
| `edit` | 8 544 | dotenv / JSON / TOML 的安全编辑 | ⚠️ | Jolu 有 `wiring.rs` 的写入逻辑，但无通用「保留注释与格式的编辑」层 |
| `catalog` | 8 026 | 模型目录 | ✅ | 已对齐 |
| `tui` | 7 795 | 终端 UI | ❌ | 不需要对齐（Jolu 是桌面应用） |

### 1.2 中等块（1000–4000 行）

| magpie 包 | 行数 | 是什么 | Jolu |
|---|---|---|---|
| `pii` | 6 089 | 个人信息识别（脱敏的前置分类） | ⚠️ 脱敏有，分类层简化 |
| `settings` / `settingsapi` | 6 046 + 697 | 设置 + 「设置页保存」的服务层 | ✅ |
| `claudetui` | 5 249 | Claude Code 的 TUI 集成（仅 darwin/linux） | ❌ 平台不符，不需要 |
| `netproxy` | 4 709 | 在回环上架 http:// 代理转发到 SOCKS5（因为 Bun 插件只认 http/https 代理） | ❌ Jolu 无 JS 插件运行时，不需要 |
| `backup` | 4 412 | 备份（含自动备份、BOM 处理） | ❌ 见 1.3 |
| `access` | 4 371 | 命名 Key（ttl / models / accounts / cost / readOnly） | ⚠️ 本轮已补 ttl / models / readOnly；缺 `accounts`（限定只能用哪些账号）与 `cost`（金额上限） |
| `codexcat` | 2 904 | 按 Codex 的格式渲染 `models.json` | ⚠️ Jolu 写 `config.toml`，未做 `model_catalog_json` |
| `middleware` | 2 788 | 中间件（含 JS 钩子运行时） | ⚠️ 三个内置已对齐；缺 JS 异步钩子 |
| `appdir` | 2 477 | 各 agent 配置目录定位（含 WSL） | ⚠️ 有定位，WSL 只探测不接线 |
| `otlp` | 2 468 | OTLP 导出 | ✅ 本轮已做 |
| `imagemcp` | 2 264 | 图像 / 视频生成 MCP | ❌ 见 1.3 |
| `autostart` | 1 925 | 开机自启 | ✅ |
| `routes` | 1 908 | 路由表 | ✅ |
| `mcpauth` | 1 863 | **给远程 MCP server 做 OAuth 登录**，token 集中存 `mcp-signins.json`，每个 agent 共用一份 | ❌ **完全没有** |
| `omarchy` | 1 636 | Omarchy（Arch 桌面发行版）集成 | ❌ 平台不符 |
| `profile` | 1 524 | 配置档案 | ✅ Jolu 有 `gateway/profiles.rs` |
| `pluginrt` | 1 500 | 插件运行时 | ❌ 见 `plugin` |
| `daemon` | 1 359 | `magpie serve` 装成 systemd unit / launch agent | ❌ 见 1.3 |
| `maskrec` | 1 330 | 记录「哪些值被脱敏过」，只通知一次 | ⚠️ Jolu 无「首次脱敏通知」 |
| `wslrun` | 1 060 | WSL 内执行与配置写入 | ⚠️ 本轮只做探测 |
| `masking` | 1 055 | 隐私设置的网关侧实现 | ✅ |
| `source` | 983 | 来源抽象 | ✅ |
| `budget` | 910 | 精确预算记账（预扣 + 回冲） | ❌ **`tokens_per_day` 强制依赖它** |
| `uninstall` | 891 | 卸载清理 | ⚠️ Jolu 用 NSIS 卸载钩子 |
| `tune` | 876 | **从用户实际行为反推「最省 token 的设置」**（何时压缩对话、Claude Code 保留多久上下文） | ❌ **完全没有** |

### 1.3 magpie 有、原差距表完全没列出的能力（13 项）

按「对 Jolu 的实际价值」排序：

| # | 能力 | magpie 包 | 价值判断 |
|---|---|---|---|
| **N1** | **远程 MCP server 的 OAuth 登录** | `mcpauth` | **高**。Notion / Neon / Supabase 这类 MCP server 要 OAuth，现在每个 agent 各自登录各自存 token；magpie 登录一次、集中存、所有 agent 共用。Jolu 的 MCP 配置目前只有 stdio |
| **N2** | **`budget` 精确记账** | `budget` | **高**。这是 `tokens_per_day` 能真正强制的唯一路径（预扣 → 用后回冲）。Jolu 现在「写了限额但不拦」，比不写更糟 |
| **N3** | **`library` 统一写入 MCP / subagents / 指令** | `library` | **高**。Jolu Skills 只管 skills 目录；MCP server 与 subagents 仍要用户手改各 agent 的配置文件 |
| **N4** | **`lastgood` 状态文件回滚** | `lastgood` | **高**。保留上一版好的 accounts / providers / settings，文件读不出时回读。Jolu 的 `settings.json` / `wiring_store` 坏了就直接回落默认值，等于丢配置 |
| **N5** | **`steady` Windows rename 竞态** | `steady` | **高**。Windows 上文件被 rename 覆盖时有约 2ms「不存在」窗口，`Stat`/`ReadFile` 读成空 → 「有账号读成没账号，下次写入就丢了」。Jolu 的 JSON 配置读写是同类路径 |
| **N6** | **浏览器扩展（Magpie Browser）** | `browserext` + `browsermcp` | **中**。让 agent 用用户自己的浏览器（已登录的标签页）。Jolu 无对应 |
| **N7** | **`stall` 只掐真正停住的下载** | `stall` | **中**。magpie 曾因整体超时把 140 KB/s 下 83MB 的更新掐断。Jolu 的 `updater.rs` 与 models.dev 拉取是简单超时，慢速大文件会被误杀 |
| **N8** | **OIDC 换网关 Key** | `oidc` | **中**。CI 里用 GitHub Actions 的 OIDC token 换短期网关 Key，不用把 Key 存进仓库 secrets。Jolu 无 |
| **N9** | **`tune` 成本调优建议** | `tune` | **中**。从真实行为反推「最省 token 的设置」。Jolu 用量统计只有「花了多少」，没有「怎么花得更少」 |
| **N10** | **`filelock` 跨进程文件锁** | `filelock` | **中**。Jolu 有 `cli_switch` 命名互斥量（针对 CLI 配置切换），但没有对 `settings.json` 这类状态文件的统一跨进程锁 |
| **N11** | **`image` / 视频生成 MCP** | `imagemcp` | **中**。依赖用户自选模型 |
| **N12** | **`daemon` 装成系统服务** | `daemon` | **低**。Jolu 是桌面应用 + 托盘，形态不同 |
| **N13** | **`beat` 时钟对齐的定时器** | `beat` | **低**。让 5s / 15s / 30s 的循环在同一时刻唤醒。Jolu 的自动化是单线程 `wait_timeout`，无此问题但也没这个优化 |
| **N14** | **`netguard` 下载地址安全校验** | `netguard` | **中**。所有「magpie 自己没选的 URL」下载（用户给的图片、`web_fetch`、厂商图片）统一判地址。Jolu 的 `web_fetch` 与图标代理无统一校验 |
| **N15** | **`codexsummary` Codex 压缩** | `codexsummary` | **中**。Jolu 的压缩会话只支持 Claude Code / Codex，但 magpie 能做「用 Jolu 自己的模型替 Codex 压缩」 |

### 1.4 magpie 有、但 Jolu 不需要对齐（形态不同）

| 包 | 不做的理由 |
|---|---|
| `gui` / `tui` / `claudetui` / `omarchy` | Jolu 是 Tauri 桌面应用，UI 形态完全不同 |
| `plugin` / `pluginrt` / `jsmod` / `agentplug` / `netproxy` | magpie 把订阅适配外置成 JS 插件并自带 JS 引擎；Jolu 内置适配。**这是产品路线选择**（AGENTS.md 已定：不为插件引入 JS 运行时） |
| `daemon` | 桌面应用 + 托盘，不需要 systemd / launchd |
| `codexrelay` | `//go:build !windows`，平台不符 |

---

## 2. 需要追平的清单（合并去重）

### 2.1 P0：说了但没做到 / 有明确数据风险

| # | 项 | 来源 | 工作量 |
|---|---|---|---|
| P0-1 | **`tokens_per_day` 真正强制**（需先做预扣 + 回冲记账） | N2 | 2 天 |
| P0-2 | **`lastgood` 状态回滚**：配置读不出时回读上一版，而不是回落默认值 | N4 | 1 天 |
| P0-3 | **`steady` 文件竞态**：审一遍 Jolu 所有 JSON 配置读写点，补 rename 重试 | N5 | 1.5 天 |
| P0-4 | **C6 ZCode UA / App 版本跟随**：把版本号做成常量 + 启动校验，防 ZCode 升级后风控拦 | 原 C6 | 0.5 天 |

### 2.2 P1：明显能力缺口

| # | 项 | 来源 | 工作量 |
|---|---|---|---|
| P1-1 | **`library`：MCP server / subagents / 指令文件按各 agent 格式写入** | N3 | 3 天 |
| P1-2 | **`mcpauth`：远程 MCP server 的 OAuth 集中登录** | N1 | 2.5 天 |
| P1-3 | **Web 搜索 MCP（9 后端 + 多 Key 轮转）** | 原 G1 | 4 天 |
| P1-4 | **C3 团队席位状态校验**（`EFFECTIVE` + `VALID`） | 原 C3 | 0.5 天 |
| P1-5 | **`access` 补 `accounts` 与 `cost`**（限定账号 / 金额上限） | 原 F1 剩余 | 1 天 |
| P1-6 | **CLI 写操作**（`provider add` / `claude <model>` / `serve`） | 原 G5 剩余 | 3 天 |

### 2.3 P2：增强

| # | 项 | 来源 | 工作量 |
|---|---|---|---|
| P2-1 | `stall` 式下载超时（区分「真停住」与「慢但活着」） | N7 | 0.5 天 |
| P2-2 | `netguard` 统一下载地址校验 | N14 | 1 天 |
| P2-3 | `proc` 统一子进程封装（取消 / 树杀 / 超时） | `proc` | 1.5 天 |
| P2-4 | `image` / 视频生成 MCP | N11 | 1.5 天 |
| P2-5 | `tune` 成本调优建议 | N9 | 2 天 |
| P2-6 | WSL 真接线（现在只有探测） | 原 G10 剩余 | 1.5 天 |
| P2-7 | `codexsummary`：用 Jolu 模型替 Codex 压缩 | N15 | 2 天 |
| P2-8 | `zcode_trial` 按模型粒度休息 | 原 C1（修正后） | 0.5 天 |
| P2-9 | ZCode IP 白名单（`MAGPIE_ALLOW_IPS` 同款） | 原 G7（修正后） | 0.5 天 |

### 2.4 明确不做

见 `2026-11-09-magpie-parity-batch3.md` §4，外加本次新确认的：`gui` / `tui` / `claudetui` / `omarchy` / `daemon` / `codexrelay` / `netproxy` / `plugin` 系列（形态或平台不符）。

---

## 3. 建议顺序

1. **P0 四项一起做**（约 5 天）：都是「数据可能丢」或「说了不生效」，风险最高、改动局部。
2. **P1-4 C3 席位校验**（0.5 天）：最小代价改善报错质量，可穿插。
3. **P1-1 `library`**（3 天）：这是 magpie 最被低估的能力——Jolu 现在 Skills 只覆盖了四分之一。
4. **P1-3 搜索 MCP**（4 天）：magpie 最显眼的能力缺口，但依赖 P1-2 的 MCP 基础设施。
5. 其余按需。

---

## 4. 核实方法与可复现命令

```bash
cd /tmp/magpie
# 按代码量排序全部包
for d in internal/*/; do n=$(basename $d); l=$(cat $d*.go 2>/dev/null | grep -v _test | wc -l); echo "$n|$l"; done | sort -t'|' -k2 -rn
# 读某个包的用途
head -12 internal/<包名>/*.go | grep '^//'
# 搜索后端清单
sed -n '91,106p' internal/provider/search_api.go
```

Jolu 侧核实：
```bash
cd /d/develop/Jolu
grep -rl "mcpauth\|lastgood\|steady\|filelock\|netguard" backend/src/ --include=*.rs   # 全部 0 命中
```
注：`budget` / `tune` 在 Jolu 里有命中，但分别是交接文档的 token 预算（`handoff_facts.rs`）与 SQLite PRAGMA 调优（`db.rs::tune`），与 magpie 的预算记账、成本调优无关。
