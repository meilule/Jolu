# 通用 MCP 配置管理：现状复核、设计与汇聚评估

日期：2026-11-24
状态：**P3-3 / P3-4 的调研与设计稿，不包含实现。** 界面部分待确认：`Jolu.pen` 没有对应画板。具体支持范围待鼠鼠决策（§6）。
配套计划：[产品对齐执行计划 · P3](../plans/2026-11-23-product-alignment-execution-plan.md)
相关：[Agent 工具调用矩阵与工具链边界](2026-11-24-tool-call-matrix.md)、[接线注册表](2026-11-17-agent-wiring-registry.md)、[产品定位](2026-11-23-jolu-product-positioning.md)

后端路径相对 `backend/src/`。Magpie 的引用来自固定提交 `488e20ad921797d91b18812be30e0d2f407ea203`。本次 WebFetch 访问 GitHub 被拦截，因此用本机已有的 magpie 克隆（仓库外）执行 `git show 488e20a:<路径>` 只读查看，没有切换它的工作区。引用写作 `magpie@488e20a:<路径>`。

---

## 0. 结论

1. **现有 Library 只做了一件事**：把一个本地 stdio server（`jolu-search`，即 `jolu.exe --mcp-search`）写进 8 家 Agent 的**用户级** MCP 配置，并且只认自己登记过的名字。它没有导入、启停、项目作用域、远程（http / sse）、环境变量与凭据、写前备份，也不按内容判断条目是否被用户改过。
2. 仓库里有据可查的 MCP 配置格式只有 8 家（§2.1）。其余 Agent 要么只在 magpie 文档里有记载，要么完全没有记录，必须先用官方文档核实。
3. 设计建议：新建一套「MCP 条目库 + 按（Agent, 作用域, 文件, 名字）记账」的数据模型。写入复用接线的原子写、备份、多文件回滚和统一锁。**不复用 `agent_wiring` 表**，它的结构是一个 Agent 一行，存的是模型接线字段。
4. P3-4 建议分期推进：先做「按 Agent 分发」（第 1 期，就是通用配置管理本身）；「网关汇聚」（MCP 路由器，tools only）放第 2 期，前提是 §5.4 的兼容性和权限隔离验证通过；「按需发现」（find_tools / call_tool）放第 3 期。resources 和 prompts 不进前两期，magpie 在这个版本也只做了 tools（§5.2）。

---

## 1. 现有 Library 能力复核

| 能力 | 现状 | 出处 |
|---|---|---|
| 支持的 Agent | `LIBRARY_AGENTS` = claude-code、codex、gemini、opencode、zcode、qoder、qoder-cn、workbuddy，以及 trae。trae 只列出、明确不支持：「MCP 配置位置与格式没有可靠来源」 | `library/mod.rs:90,104-111` |
| 条目模型 | `LibraryItem::McpServer{name, command, args}`，只有本地 stdio 一种。**没有** env、cwd、type、url、headers 字段 | `library/mod.rs:38-52` |
| 内置条目 | 只有 `jolu-search`：当前 exe 加 `--mcp-search` | `library/mod.rs:314-343` |
| 安装 | 先 upsert `library_applied(agent, name, kind, payload_json, installed_ms)`，再写文件；写文件失败就删掉记录（半事务）。同名条目不是自己登记的就返回 `ForeignEntry` 拒绝 | `library/mod.rs:215-244`；表结构 `db.rs:304-314` |
| 卸载 / 还原 | 只删 `library_applied` 里登记过的名字；卸载钩子 `--restore-agents` 调用 `restore_all_library` | `library/mod.rs:248-308`；`lib.rs:1854-1856` |
| 格式读写 | JSON 宽松解析（含 JSONC 注释）；TOML 用 `toml_edit`，保留注释；点分嵌套键（ZCode 的 `mcp.servers`）；键路径上某段不是对象时拒绝写；删除后逐层清理空壳 | `library/agents.rs:63-329` |
| 原子写 | `wiring::write_json_atomic_public`：临时文件 + rename，写后读回校验；读取走 `steady` | `library/agents.rs:303-306`；`gateway/wiring.rs:1412,1422-1475` |
| 写前备份 | **没有**。接线用的 `wiring::backup()` 没有被 Library 调用 | `gateway/wiring.rs:1477-1489`（`pub(super)`） |
| 归属判断 | MCP 只按名字判断（`library_applied` 主键是 (agent, name)），用户手改过的同名条目在重装或卸载时会被覆盖或删除。共享指令和子代理（`shared.rs`）已经用 SHA-256 哈希判断「被改过就跳过」 | `library/mod.rs:223-226`；`library/shared.rs:58-61,295-301,333-336` |
| 并发保护 | 桌面命令走 `cli_switch::with_switch`（进程内 Mutex 加 Windows 命名互斥量，与接线同一把锁）；CLI 的 `jolu library` 没有加锁 ?（待确认：`bin/jolu_cli.rs:241-280` 未见 `with_switch`） | `commands/capabilities.rs:115-130`；`cli_switch.rs:1-40` |
| 对外命令 | `library_install` / `library_remove` / `library_list`，以及共享指令和子代理的 4 个命令；事件 `library-changed` | `commands/capabilities.rs:115-229`；`lib.rs:1918-1924` |
| 前端 | 设置页的「搜索插件（MCP）」分组：逐个 Agent 安装或移除；「共享指令 / 子代理（待确认）」 | `frontend/src/components/settings/SearchSection.tsx:192-237`、`LibrarySharedSection.tsx` |
| mcp_search 本身 | 独立的 stdio 进程，不是网关端点；协议版本 `2024-11-05`；只声明 tools 能力；提供 `web_search{query,count}` 和 `web_fetch{url,max_chars}`；工具失败用 `isError` 返回 | `library/mcp_search.rs:4-20,25-105`；`main.rs:13-18`；`lib.rs:1867-1884` |
| 网关自己的 MCP 端点 | 只有 Claude 订阅桥用：`POST /__jolu/claude-mcp/<令牌>`，临时 `mcp.json` 只交给隔离的 claude 进程，**不写进任何 Agent 的配置** | `gateway/adapter/claude_bridge.rs:403-457,848-867`；`gateway/server.rs:65-75` |

两个已知的文档过期问题（不影响行为）：

- `library/agents.rs:5-10` 的模块头还写着「claude-code 写 `~/.claude/settings.json`、只有 4 个 agent」，与 `mod.rs:126` 的实现不一致。
- `docs/specs/2026-11-10-capability-batches-handoff.md:191-199` 写「Jolu 只做 4 种格式」，实际已经是 8 种。

---

## 2. 各 Agent 的 MCP 配置格式与位置

### 2.1 仓库代码里已经实现的（用户级）

| Agent | 文件 | 键与条目形状 | 出处 | 备注 |
|---|---|---|---|---|
| Claude Code | `%CLAUDE_CONFIG_DIR%` 或 `~` 下的 `.claude.json` | `mcpServers.<名> = {type:"stdio", command, args}` | `library/mod.rs:126`；`library/agents.rs:112-118` | 旧位置 `~/.claude/settings.json` 的 `mcpServers` 不生效，只在卸载时清理（`mod.rs:142-147`） |
| Codex | `creds::codex_dir()/config.toml` | `[mcp_servers.<名>]`，含 `command`、`args` | `library/mod.rs:127`；`library/agents.rs:310-329` | 同一文件也被接线改写（`wiring_specs.rs` 的 codex 行）；session_handoff 用 `-c mcp_servers.<名>.enabled=false` 关闭服务（`session_handoff.rs:1566-1576`），说明存在 `enabled` 字段；测试样本里有 `url = …` 的远程条目（`session_handoff.rs:3056`） |
| Gemini CLI | `~/.gemini/settings.json` | `mcpServers.<名> = {command, args}` | `library/mod.rs:128`；`library/agents.rs:117` | 接线写的是另一个文件 `~/.gemini/.env`（`wiring_specs.rs:155-157`） |
| OpenCode | `%XDG_CONFIG_HOME%` 或 `~/.config` 下的 `opencode/opencode.json` | `mcp.<名> = {type:"local", command:[cmd, ...args], enabled:true}` | `library/mod.rs:129`；`library/agents.rs:134-144` | 与接线写同一个文件 |
| ZCode | `zcode_home/.zcode/cli/config.json` | 嵌套键 `mcp.servers.<名> = {type:"stdio", command, args}` | `library/mod.rs:130`；`library/agents.rs:427-445` | 接线写的是 `v2/provider_config.json` |
| Qoder CLI | `%QODER_CONFIG_DIR%` 或 `~/.qoder` 下的 `settings.json` | `mcpServers.<名> = {command, args}` | `library/mod.rs:131` | 与接线写同一个文件（`gateway/wiring_extra.rs:29-30`）；条目形状注明参照 magpie，?（待确认：仓库内没有真机样本） |
| Qoder CN | `%QODERCN_CONFIG_DIR%` 或 `~/.qoder-cn` 下的 `settings.json` | 同 Qoder | `library/mod.rs:132` | 同上 |
| WorkBuddy | `%WORKBUDDY_CONFIG_DIR%` 或 `~/.workbuddy` 下的 `mcp.json` | `mcpServers.<名> = {type:"stdio", command, args}` | `library/mod.rs:133`；`library/agents.rs:114` | 写入后需要用户在 WorkBuddy 的 MCP 设置里批准一次（`mod.rs:116`） |

### 2.2 只在 magpie 文档里有记载的（待用官方文档核实）

下面内容来自 `magpie@488e20a:docs/subsystems/library.md`，Jolu 代码里没有实现，**写入前必须逐家用官方文档或源码核实**。

| 内容 | magpie 记载 | 出处 |
|---|---|---|
| 项目级 MCP 文件 | Claude Code `.mcp.json`；Codex `.codex/config.toml`；Gemini `.gemini/settings.json`；OpenCode `opencode.json`；ZCode `.zcode/config.json`；Qoder / Qoder CN `.qoder/settings.json` 的 `mcpServers`；Pi `.pi/mcp.json`；Droid `.factory/mcp.json`。Claude Code、Qoder、CodeBuddy 使用项目级 server 前会要求用户批准 | `library.md:26`（Projects 行） |
| 远程 server | `Server` 可以是命令或 URL（`http`、`sse`）；Codex、Goose、dsh、Grok、Command Code、Zed 不接受 SSE；Claude Desktop 不接受远程 server | `library.md:28,76` |
| 环境变量引用 | 多数 Agent 用 `${NAME}`；OpenCode 用 `{env:NAME}`；Codex 有专用键 `bearer_token_env_var`、`env_http_headers`、`env_vars`；dsh 用 `!!js process.env.NAME`；Kimi、Cline、ZCode、Pi 等没有引用语法，magpie 拒绝把带变量的 server 写给它们 | `library.md:77` |
| 单个工具停用 | Codex `disabled_tools`；Gemini `excludeTools`；Crush `disabled_tools`；Claude Code 在 `settings.json` 的 `permissions.deny` 里写 `mcp__<server>__<tool>`；OpenCode 在顶层 `tools` 里写 `"<server>_<tool>": false`；其余 Agent 只能整台 server 启停 | `library.md:80-86` |
| WorkBuddy 国际版 | 目录是 `~/.workbuddy-ai`；批准记录在 `mcp-approvals.json`，magpie 不代写 | `library.md:112` |
| Zed | `settings.json` 的 `context_servers` | `library.md:115` |

### 2.3 仓库内无记录

| Agent | 已知信息 | 结论 |
|---|---|---|
| Kimi | 接线文件是 `~/.kimi-code/config.toml` 或 `~/.kimi`（`gateway/wiring.rs:1117-1130`）；magpie 说它没有环境变量引用语法 | MCP 字段：待用官方文档核实 |
| Qwen Code | 接线文件是 `qwen_dir/settings.json`（`gateway/wiring_qwen.rs:37-39`）；Library 只给它写 `QWEN.md` 和子代理（`library/shared.rs:75,86`） | 待用官方文档核实 |
| dsh | 用户自己的 MCP 条目在 YAML profile patch 里，接线时原样保留（`gateway/wiring_dsh.rs:847-858` 测试样本）；magpie 说它用 `!!js` 写环境变量 | 字段语义待核实 |
| Pi | 接线写 `openai-completions` provider（`gateway/wiring_pi.rs:139`）；magpie 记载项目级 `.pi/mcp.json` | 用户级待核实 |
| Pencil | 接线文件 `~/.pencil/models.json`（`gateway/wiring_extra.rs:28`） | 待核实 |
| Crush、Goose、Cline、Continue、Droid | 只有探测目录或子代理路径（`agent_discovery.rs:227-260`；`library/shared.rs:76`） | 待核实 |
| Trae | `library/mod.rs:107` 明确写了「没有可靠来源」 | 维持不支持 |

---

## 3. 通用 MCP 配置管理设计（P3-3）

### 3.1 目标与非目标

- **目标**：用户在 Jolu 里登记一份 MCP server，选择给哪些 Agent、在哪个作用域生效，Jolu 把它写进各家配置，并且能安全地撤回。也能把各 Agent 里已有的 server 导入进来。
- **非目标（本期）**：MCP 市场、OAuth 远程登录（magpie 的 `mcpauth`；`docs/specs/2026-11-10-capability-batches-handoff.md:203-207` 已列为依赖）、跨机同步、插件化（magpie 拆解文档已注明「插件化 / 插件市场不做」，`docs/specs/2026-11-12-magpie-claude-agent-detail-parity.md:560-564`）。

### 3.2 数据模型（建议）

| 表 | 字段 | 说明 |
|---|---|---|
| `mcp_servers` | `name`（主键）、`transport`（stdio / http / sse）、`command`、`args_json`、`cwd`、`env_json`、`url`、`headers_json`、`enabled`、`disabled_tools_json`、`note`、`source`（jolu / imported:<agent> / builtin）、`updated_ms` | 条目库。`env_json` 和 `headers_json` 只存**变量名或密文引用**，不存明文（见 3.6） |
| `mcp_targets` | `server`、`agent`、`scope`（global / project）、`project_dir`（global 时为空） | 「给谁、在哪生效」，对应勾选 |
| `mcp_applied` | `agent`、`scope`、`path`、`name`、`entry_hash`、`written_ms`；主键 (agent, path, name) | 写入记账，复用 `library_files` 的哈希归属思路（`library/shared.rs:46-54`）。**取代**现在只按名字记账的 `library_applied` 中 MCP 那部分，需要可恢复的迁移：旧行搬过来，`entry_hash` 在首次同步时从文件里读出补上 |

现有 `jolu-search` 作为 `source = builtin` 的一行迁入，保证现有安装不丢。

### 3.3 导入

1. 按 §2.1 的路径（用户级）和用户添加的项目目录（项目级，路径待核实后再开放），只读扫描各 Agent 的 MCP 段。
2. 每个条目归一化为 `mcp_servers` 的形状（OpenCode 的 command 数组拆成 command + args；Codex 的 TOML 表转成字段），并算出 `entry_hash`。
3. 导入预览列出：新条目；与库里同名且内容一致的（合并为同一条，只补 `mcp_targets`）；同名但内容不同的（冲突，见 3.4）；含明文密钥的（提示改为变量引用，见 3.6）。
4. 导入**只写 Jolu 数据库，不改 Agent 文件**。导入后的条目，在用户勾选之前不由 Jolu 接管（`mcp_applied` 不记），所以仍然算用户自己的。

### 3.4 冲突

| 冲突 | 处理 |
|---|---|
| Agent 文件里已有同名条目，且 `mcp_applied` 里没有 | 不覆盖，标为「外部同名」。沿用现在的 `ForeignEntry` 语义（`library/mod.rs:223-226`）。用户可以选「导入后接管」 |
| `mcp_applied` 有记录，但文件里条目的哈希已变 | 视为用户改过：同步时跳过，卸载时也不删，并提示。沿用 `shared.rs:295-301` 的做法 |
| 两个库条目在同一 Agent 里同名 | 名字是 `mcp_servers` 的主键，不会发生；从不同 Agent 导入的同名异内容条目，要求用户改名其中一个 |
| 同一文件也被接线写入（Codex `config.toml`、OpenCode `opencode.json`、Qoder `settings.json`） | 写入必须在 `cli_switch::with_switch` 里进行，与接线串行。CLI 入口也要补上这把锁 |
| 文件在 Jolu 读写之间被 Agent 自己改了 | 写之前重新读取并比对哈希，变了就放弃本次写入并提示重试（接线的 `reconcile_pending` 思路，`gateway/wiring.rs:1878-1896`） |
| 名字含不合规字符，或 Claude Code 工具名超长 | 按目标 Agent 规则校验。Claude Code 的 `mcp__<server>__<tool>` 有字符与长度限制，可参考 `claude_bridge::bridge_name`（`gateway/adapter/claude_bridge.rs:80`） |

### 3.5 启停与作用域

- **整台启停**：`mcp_servers.enabled = false` 时，从所有 Agent 的文件里撤掉 Jolu 写入的条目，保留勾选，重新启用时恢复（对应 magpie 的 `ServerOn` / `Paused`，`library.md:29`）。不使用 Codex 的 `enabled = false`：各家语义不一致，统一按「撤掉条目」处理更可控。
- **按 Agent 勾选**：`mcp_targets` 增删即可。
- **单个工具停用**：只对 §2.2 列出的、有对应字段的 Agent 生效，其余 Agent 标「不支持单工具停用」。**这一项需要先核实各家字段（待确认）**。
- **作用域**：global 写用户级文件；project 写项目目录里的文件。项目路径从哪里来，可以复用 Skills 的「会话记录自动带出 + 手动添加」（AGENTS.md 模块 F）。Claude Code 和 Qoder 的项目级 server 需要用户在 Agent 里批准，Jolu 只能提示，不能代批。

### 3.6 凭据

- `env` 和 `headers` 里的值分两种：**变量引用**（`${NAME}`），按目标 Agent 的语法写出（§2.2）；**Jolu 托管的密钥**，用 DPAPI 加密后存库（复用 `auth_snapshot::protect`，`auth_snapshot.rs:148`），只在后端解密。
- 目标 Agent 支持变量引用时，只写引用，真实值不落盘到 Agent 文件。这要求 Agent 进程能读到这个变量，**Jolu 怎么把变量交给 Agent 进程待确认**：写用户环境变量会影响所有程序，有副作用。
- 目标 Agent 不支持引用时（magpie 列出 Kimi、Cline、ZCode、Pi 等），只能写明文。默认**拒绝写入**，并告诉用户原因，与 magpie 的 `refsProblem` 一致；是否提供「明知风险仍写明文」的开关，需要鼠鼠决策。
- 前端只拿掩码，掩码规则复用 `gateway/search/mod.rs:88` 的 `mask`（前 4 后 4）。诊断导出与日志不含值（AGENTS.md §4）。

### 3.7 路径

- 用户级路径集中在 `library::mcp_config_path`（`library/mod.rs:124-139`），已支持 `CLAUDE_CONFIG_DIR`、`XDG_CONFIG_HOME`、`QODER_CONFIG_DIR`、`QODERCN_CONFIG_DIR`、`WORKBUDDY_CONFIG_DIR` 等覆盖。新增 Agent 时同步扩展，并与探测（`agent_discovery.rs`）用同一个目录函数，避免像 Gemini 首版那样写错文件（`library/mod.rs:146`）。
- stdio server 的 `command`：Windows 下要处理含空格的路径和 `.cmd` / `.ps1` 启动器 ?（待确认：各 Agent 在 Windows 上怎么拉起 `npx` 这类 `.cmd`）。
- WSL 中的 Agent：接线已经有 `gateway/wiring_wsl.rs`，MCP 是否跟进待确认。magpie 有专门处理（`library.md:79`）。

### 3.8 回滚

| 机制 | 复用方式 |
|---|---|
| 写前备份 | 把 `wiring::backup()`（`gateway/wiring.rs:1477-1489`，目前是 `pub(super)`）提升可见性，每次改 Agent 文件前都备份到 `gateway-backups/` |
| 多文件写入失败回滚 | 复用 `write_all_with_rollback`（`gateway/wiring.rs:1795-1813`，目前私有）。一次同步可能涉及多个 Agent 文件 |
| 原子写与写后校验 | 继续用 `write_json_atomic_public`；TOML 写入走同样的临时文件 + rename |
| 半事务记账 | 沿用 Library 的「先记库、写失败删记录」（`library/mod.rs:230-243`），或接线的 pending → wired 两段式（`gateway/wiring_store.rs:249-284`），启动时修复中间态 |
| 改动预览 | 复用 `wiring_diff::diff_file`（`gateway/wiring.rs:1853-1875` 的 `preview_plan`） |
| 卸载还原 | 扩展 `restore_all_library`：按 `mcp_applied` 撤掉哈希仍一致的条目。卸载钩子目前在接线锁释放之后才执行，这一步应该补锁 ?（待确认：`service/gateway/wiring_restore.rs:34`） |
| **不复用** `agent_wiring` | 这张表一个 Agent 只有一行，`Applied` 里全是模型接线字段（`gateway/wiring_store.rs:29-60`），装不下按 server 区分的条目 |

### 3.9 界面

待确认：`Jolu.pen` 没有对应画板。本文不设计界面。现有入口是设置页「搜索插件（MCP）」分组（`SearchSection.tsx:192-237`）。通用管理需要的列表、导入预览、冲突提示、凭据输入等界面，都要先补画板。

---

## 4. 网关已有的三种「工具」能力与 MCP 管理的关系

计划文档已经强调三者不能合并看待（`docs/plans/2026-11-23-product-alignment-execution-plan.md:34`）：

| 能力 | 运行位置 | 与通用 MCP 管理的关系 |
|---|---|---|
| `library/mcp_search.rs` | Agent 拉起的 stdio 子进程 | 是通用管理里的一个内置条目（`source = builtin`） |
| `gateway/tool_loop.rs` | 网关内部，代执行服务端工具 | 与 MCP 配置无关。但如果将来网关能代调用户的 MCP server，tool_loop 的「替身工具」可以扩展成「替身 MCP 工具」，这属于 P3-4 汇聚的延伸 |
| `gateway/adapter/claude_bridge.rs` | 网关自己的 HTTP MCP 端点，只对隔离的 claude 进程开放 | 已有 Streamable HTTP 端点、令牌鉴权和回环限制（`gateway/server.rs:65-75`），可以作为汇聚端点的实现参考，但不能直接复用：它的令牌是按进程发的，用完即弃 |

---

## 5. P3-4 评估：汇聚、按 Agent 分发、按需发现

### 5.1 magpie 的做法（固定版本）

来源：`magpie@488e20a:docs/subsystems/mcp-router.md`、`internal/gateway/hub_mcp.go`。

- **汇聚（MCP router）**：开关打开后，每个 Agent 只拿到一个名为 `magpie` 的 server，地址是 `/mcp/magpie/hub/<agent>`（Streamable HTTP）。网关把每次调用转给 Library 里对应的后端 server；同一个命令型 server 只启动一次，所有 Agent 共用。开关默认关闭（`mcp-router.md:3`）。
- **按 Agent 分发**：`tools/list` 只列这个 Agent 勾选的 server 的工具，命名为 `<server>__<tool>`，最长 64 字符（`mcp-router.md` 的 Serve 行）。不能接受远程 server，或自己已有名为 magpie 的条目的 Agent，保留逐条写入（Who keeps their entries 行）。
- **按需发现**：Agent 自己不支持工具搜索、且工具超过 10 个（`hubOnDemandOver`，`internal/gateway/hub_ondemand.go:44`）时，改为只提供 `find_tools {query, server?}` 和 `call_tool` 两个工具。Claude Code（`ENABLE_TOOL_SEARCH`）、Codex（`tool_search`）、Pi、Droid 等自己会搜索工具，直接拿到完整列表（Serve 行、Tools on demand 行）。
- **项目上下文**：项目覆盖了 server 时，Claude Code 和 Codex 拿到 `/mcp/magpie/hub/<agent>~<项目键>`（A project's set 行）。图像 server 需要知道项目目录，路由器向 Agent 发 `roots/list` 询问，等待 3 秒；拿不到时退回读 Codex 的 `_meta` 或会话目录，需要用户同意（Magpie Image's project 行）。
- **权限隔离**：只允许本地请求，或带网关 Key 的请求；按 Origin 拒绝网页请求；请求体上限 4 MiB；每次调用都受调用方 Key 的额度约束。来自其他电脑的调用方拿不到「以本机用户身份运行」的 server（浏览器、图像），受限 Key 只能访问远程 HTTP server，不能访问本机程序（Guard 行，Constraints 部分）。
- **生命周期与超时**：列工具等待 40 秒，单次调用 50 秒，闲置 10 分钟结束进程；调用记入请求日志，但不记录参数（Runtime path 部分；`hub_mcp.go:301-302`；`internal/library/mcp_hub.go:62`）。
- **resources / prompts**：`hub_mcp.go:28-30` 原文为「Tools only, for now: no resources or prompts.」，`initialize` 只声明 `tools` 能力（`hub_mcp.go:649`）。

### 5.2 tools、resources、prompts 分开评估

| 能力 | 汇聚的可行性 | 价值 | 风险 | 建议 |
|---|---|---|---|---|
| tools | 高：协议简单（list / call），magpie 已验证过；Jolu 的 `claude_bridge` 和 `mcp_search` 已实现 tools 端 | 一处配置、所有 Agent 共用；命令型 server 只起一个进程 | 每个 Agent 的工具名长度和字符限制不同；超过 10 个工具会挤占上下文（magpie 的用户反馈） | 第 2 期做 |
| resources | 中：需要 `resources/list`、`resources/read`，可能还有订阅（`resources/subscribe`）和 `list_changed` 通知 | 各 Agent 对 resources 的支持参差不齐 ?（待确认：哪些 Agent 会在界面上展示 MCP resources） | 订阅类通知需要长连接或 SSE，汇聚后要维护每个 Agent 的订阅状态；资源内容可能很大 | 不进前两期。先做调研：列出各 Agent 对 resources 的实际使用 |
| prompts | 中：`prompts/list`、`prompts/get` | 在支持的 Agent 里表现为斜杠命令 ?（待确认） | 名字冲突（多个 server 的 prompt 同名）；参数补全 | 不进前两期。Jolu 的共享指令和子代理（`library/shared.rs`）已经覆盖一部分需求 |

### 5.3 三种模式比较

| 模式 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| 按 Agent 分发（§3） | 把条目写进每个 Agent 自己的配置 | 不依赖网关常驻；兼容所有支持 MCP 的 Agent；断开 Jolu 后 Agent 仍能用 | 同一个命令型 server 在每个 Agent 里各起一个进程；各家格式差异都要维护 |
| 网关汇聚 | 每个 Agent 只写一个指向网关的 HTTP 条目 | 一个 server 只起一次；集中做权限、日志和超时控制；新增 server 不用改 Agent 文件 | 网关停了所有 MCP 工具都不可用；要求 Agent 支持 Streamable HTTP；项目上下文要靠 `roots/list` 或路径编码 |
| 按需发现 | 汇聚基础上，只提供 find_tools / call_tool | 工具多时节省上下文 | 模型多一步搜索；对已有工具搜索的 Agent 反而有害（magpie 对它们不启用） |

### 5.4 实现前必须验证的点

1. **兼容性**：逐家确认是否支持远程 Streamable HTTP server，以及是否支持 `Authorization` 头。magpie 记载 Claude Desktop 不接受远程 server，多家不接受 SSE（`library.md:76`），Jolu 自己要实测。
2. **权限隔离**：汇聚端点要和网关 Key 体系打通。局域网共享打开时，外来调用方不能调用本机命令型 server；这需要沿用 `gateway/access.rs` 的命名 Key 和 `server.rs` 的 `local_only`。
3. **项目上下文**：命令型 server 只起一个进程，`cwd` 只能有一个。需要依赖项目目录的 server（例如文件系统类）不适合汇聚，应保留「直连」选项（magpie 的 `Server.Hub = direct`）。
4. **网关不可用时的退化**：汇聚开着而网关没运行时，Agent 里所有 MCP 工具都会失效。要么启动时提示，要么像 `service/gateway/status.rs` 的 `wired_tools_with_a_stopped_gateway_are_flagged_unreachable` 那样标出来。
5. **与 tool_loop 的关系**：汇聚后网关理论上可以代执行用户的 MCP 工具，但只有 Anthropic 服务端工具这一种触发条件（见工具矩阵 §5.1）。不建议把两者混在一起。

### 5.5 建议的实现范围与分期

| 期 | 内容 | 前置条件 | 验收 |
|---|---|---|---|
| 1 | 通用 MCP 配置管理（§3）：条目库、导入、冲突、整台启停、全局作用域、stdio + 支持的 Agent 的 http、变量引用式凭据、备份与回滚；把 `jolu-search` 迁入 | 补 `Jolu.pen` 画板；核实 §2.2 中准备开放的 Agent 字段 | 8 家 Agent 的写入 / 撤回往返测试（仿照 `every_agent_round_trips_through_production_plan`）；手改条目后不被覆盖；与接线同时写同一文件不丢数据 |
| 1.5 | 项目作用域、单工具停用 | 各家项目级文件和停用字段的官方核实 | 每家一条往返测试，加一次真实 Agent 加载验证 |
| 2 | 网关汇聚（tools only）、按 Agent 分发、直连例外 | §5.4 第 1–4 项验证通过 | 假 stdio server 与假 HTTP server 的端到端测试（参照 magpie `TestFakeMCPServer`）；至少一家真实 Agent 通过汇聚端点调用工具 |
| 3 | 按需发现（find_tools / call_tool） | 第 2 期上线并收集到工具数量的实际分布 | 超过阈值的 Agent 的上下文 token 下降；能自己搜索工具的 Agent 不受影响 |
| 待定 | resources / prompts、远程 OAuth 登录、WSL | 需求确认 | — |

---

## 6. 需要鼠鼠决策的点

1. **范围**：第 1 期只做「按 Agent 分发」，还是同时做汇聚？（建议先做分发。）
2. **支持哪些 Agent**：是否只开放 §2.1 的 8 家，其余等官方文档核实后再逐家加入？
3. **明文凭据**：目标 Agent 不支持变量引用时，一律拒绝写入，还是提供明确提示后允许写明文？
4. **变量怎么交给 Agent**：Jolu 托管的密钥，用什么方式让 Agent 进程读到？写用户环境变量会影响所有程序，写 Agent 自己的 env 文件则要逐家核实。
5. **导入后的归属**：导入的条目是否自动由 Jolu 接管（以后同步会覆盖文件里的改动），还是默认仍算用户自己的？
6. **项目作用域**：项目目录是否复用 Skills 的「会话自动带出 + 手动添加」？
7. **汇聚的默认值和直连例外**：汇聚开关默认关闭吗？依赖项目目录的 server 是否默认直连？
8. **resources / prompts**：是否接受「本期不做，先调研各 Agent 的实际使用情况」？
9. **界面**：需要先在 `Jolu.pen` 补画板（列表、导入预览、冲突、凭据），这一步阻塞第 1 期的前端，但不阻塞后端数据模型与测试。
10. **旧数据迁移**：现有 `library_applied` 的 MCP 行迁到新表，旧表保留只读还是删除？AGENTS.md §4 要求迁移可恢复。
