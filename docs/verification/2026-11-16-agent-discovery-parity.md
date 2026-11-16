# Agent 探测修复验收记录

对应执行文档：`docs/plans/2026-11-16-agent-discovery-fix.md`。参考 magpie `40f7de24`（调查时为 `e9dd8e8`，差异见执行文档 T1 / T2 的「执行前核对」）。

| 任务 | 状态 |
|---|---|
| T1 桌面版 / 官方安装器落点 | 已完成 |
| T2 ZCode 数据目录跟随 | 已完成，已扩展到额度 / 账号导入 / 归因 / 本机来源 |
| T3 目录内容甄别 | 已完成 |
| T4 例外规则钩子 | 已完成（Gemini `CliOnly`） |
| T5 命令查找缓存 | 已完成 |
| T6 注册表 PATH | 已完成（并入 T5：每次取戳只读一次，不单独缓存） |
| T7 页面不重复等待 | 已完成（方案 B：先显示上次结果、后台刷新、不节流） |
| T8 同名程序甄别 | 已完成（Goose `bin_not: go_program`；顺带改正 Goose 的 Windows 目录） |
| T9 路径占用 | 已完成 |

---

## T1 · 桌面版 / 非标准安装位置的 CLI 识别

### 改动

- `backend/src/agent_discovery.rs`
  - 新增 `vendor_bin_dirs(name, home, local_app_data)`：按工具列出官方安装器与桌面版的固定落点。Claude `~/.claude/local`；OpenCode `~/.opencode/bin`；Codex 依次为 `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin`（`install.ps1`）、`~/.codex/packages/standalone/current/bin`、`%LOCALAPPDATA%\OpenAI\Codex\bin\<哈希>`（桌面版）。
  - 新增 `hashed_bin_dirs(root, name)`：只看一层子目录，只保留含该命令的子目录，按命令文件的修改时间新到旧排。
  - `find_bin` 的顺序：进程 PATH → 通用兜底目录 → 注册表 PATH → 固定落点。固定落点只是兜底，PATH 上有的优先。
  - 注释和 `trim_matches` 字符字面量里原本有两个 NUL 字节，导致 Git 把文件当二进制。已换成 `\0` 转义，语义不变。
- 三态语义、`discover_with` 的注入形态、只读约束都不变。

### 验证

- 单元测试（Windows）：
  - `codex_desktop_cli_in_hashed_dir_is_found`：两个哈希目录，一个只有 `rg.exe`、一个有 `codex.exe`，只靠固定落点查找 → `installed` / `config_file`，`cli_path` 指向哈希目录里的 exe；别家不受影响。
  - `hashed_bin_dirs_prefer_newest_and_skip_missing_root`：多个哈希目录取最新，根目录不存在返回空。
  - `vendor_bin_dirs_cover_official_installers`：落点清单覆盖 Claude / OpenCode / Codex，其他工具为空。
- 真机只读核对 `vendor_codex_real_machine_readonly_smoke`（`#[ignore]`，2026-11-16）：
  - 不经 PATH 和 npm，只靠固定落点：找到 `%LOCALAPPDATA%\OpenAI\Codex\bin\5449c1f0f26f1f98\codex.exe`，`--version` 为 `codex-cli 0.167.0-alpha.3.2`。
  - 完整查找：本机 11-14 另装了 npm 版（`%APPDATA%\npm\codex.cmd`，`codex-cli 0.169.0`），优先命中它，符合「兜底放最后」。
  - 本机已有 npm 版，执行文档原来的验收「本机从 `cli_missing` 变成 `installed`」复现不出来，改用上面的只靠固定落点查找来验证。
- 全量 `cargo test --manifest-path backend/Cargo.toml`：1398 passed / 0 failed / 49 ignored，没有编译警告。

## T2 · ZCode 数据目录跟随

### 改动

- `backend/src/creds.rs` 新增公共函数，探测与接线共用：
  - `zcode_data_dir(home)`：先认 `ZCODE_HOME`（值就是 `.zcode` 目录，空值视为未设）；否则取 `zcode_data_base(zcode_home(home))`，再拼 `.zcode`。
  - `zcode_home(home)`：Windows 上 `HOME` 是根路径、且默认 home 就是 `USERPROFILE` 时用 `HOME`，原样从 `library/mod.rs` 移过来，对应 magpie `ZCodeHomeOn`。
  - `zcode_data_base(home)`：用 `steady::read_to_string` 读原位置的 `.zcode/v2/setting.json`，取 `dataBaseDir`。字段不是字符串、去空白后为空、不是根路径，或者文件缺失、JSON 不合法，一律回退 `home`，对应 magpie `ZCodeDataIn`。
  - `rooted`：绝对路径，或 Windows 上以 `\`、`/` 开头的路径，对应 magpie `appdir/env.go::rooted`。
- `agent_discovery::dir_zcode` 与 `gateway/wiring.rs::zcode_dir` 都改成调 `creds::zcode_data_dir`。
- `wiring::zcode_clean_legacy` 还是找原位置的 `provider.json`：那是旧版 Jolu 写在原位置的文件，不跟 `dataBaseDir` 走。
- `library/mod.rs` 的 MCP 配置路径改用 `creds::zcode_home`。`cli/config.json` 留在 ZCode 主目录下，不跟 `dataBaseDir` 搬，行为不变。

### 验证

- 单元测试：
  - `creds::zcode_dir_tests`：没有 setting / 没有字段时回退 home；绝对路径跟过去（含首尾空白）；相对路径、空串、数字、null、坏 JSON 都回退 home；临时 home 不受 `HOME` 影响；`rooted` 判定。
  - `zcode_follows_data_base_dir`：`dataBaseDir` 指向另一个临时目录，`provider_config.json` 只放在新位置 → 探测得到 `config_file`，且 `creds::zcode_data_dir` 与探测目录相同，即接线写的就是探测看的目录。
  - `zcode_without_data_base_dir_stays_at_home`：没有 `dataBaseDir` 时与改动前一致。
- 真机：本机 ZCode 的 `setting.json` 里没有 `dataBaseDir`，搬家场景只有单测覆盖，**未做真机验证**。

### 扩展：其余读 `v2` 的地方（鼠鼠 2026-11-16 决定一并改）

- `zcode_quota.rs`：本机额度入口读 `provider_config.json`；`coding_plan_key_at` / `coding_plan_jwt_at` / `read_account` / 团队套餐读 `credentials.json`；Windows 凭据库条目名（`sha256(credentials.json 路径)`）。以上都改走 `creds::zcode_data_dir`。
- 团队套餐的 `setting.json` 改从 `creds::zcode_home` 读，和 magpie `zcode_team.go` 一致，不跟 `dataBaseDir` 搬。
- `credential_secret` 不变：口令按系统用户目录派生，和 magpie `zcodeSecret` 一致。
- `gateway/adapter/zcode.rs` 的模型清单、`attribution.rs` 的来源归因读 `provider_config.json`；`native_sources.rs` 的「ZCode 账号」来源读 `credentials.json`。这三处都改走 `creds::zcode_data_dir`。会话库 `cli/db/db.sqlite` 保持在原位。
- 验证：
  - 新增 `zcode_quota::tests::credentials_follow_data_base_dir`：凭据只放在 `dataBaseDir` 下，用原 home 派生的口令能解出 Key。
  - 相关模块测试 155 passed。
  - 真机只读 `local_v2_credentials_decrypt_readonly_smoke`：本机没搬家，5 个条目全部解密，coding-plan Key 和 JWT 都能读到，行为不变。

---

## T3 · 目录内容甄别

### 改动

- `agent_dir_has_content(path, sign_in)`，对应 magpie `agentDir`：
  - 目录不存在 → 没有内容；存在但读不了 → 算有内容；空目录 → 算有内容。
  - 只剩 `skills`、`.DS_Store`、残留目录 `logs` / `log` / `debug` / `diagnostics` / `cache`（只认目录，大小写不敏感），或者只剩 `sign_in` 登录文件 → 不算有内容。
- `Spec.sign_in`：Codex 为 `auth.json`，Claude Code 为 `.credentials.json`。Jolu 的「应用到原工具」会替没装过的工具写这两个文件，magpie `signIn` 也是这么处理的。
- `discover_with` 的目录证据改用这个判定。配置文件证据不变：文件在就算。

### 验证

- 单元测试 `agent_dir_content_ignores_leftovers_skills_and_sign_in`，覆盖：不存在、空目录、只有 skills、只有残留目录与 `.DS_Store`、同名文件、只有登录文件（有 / 没有 sign_in 两种）、有配置文件、有未知文件。
- 单元测试 `residue_only_dirs_are_not_found`：Qwen 只剩 logs + cache、Codex 只有 `auth.json`、Droid 只有 skills，都是 `not_found`；Kimi 有历史文件时照旧 `cli_missing`。
- 真机：本机各工具的配置目录里都有真实配置，**没有卸载残留样本，未做真机验证**。真机只读探测结果见文末。

## T4 · 例外规则钩子

### 改动

- `Spec.detect: Detect`，枚举 `Standard` / `CliOnly`。API 形态由鼠鼠 2026-11-16 选定，不提前加 `Custom`。
- Gemini 设为 `CliOnly`：目录和文件都不算证据；命令不在 → `not_found`，命令在 → `installed`，证据记 `bin`。依据是 magpie `gemini.go` 的 `detect`：`~/.gemini` 卸载后会留下，Antigravity 也用这个目录。其他 12 家为 `Standard`，行为不变。

### 验证

- 单元测试 `gemini_is_detected_by_cli_only`：`~/.gemini` 里有 antigravity、settings.json、`.env`，但没有命令 → `not_found`；有命令 → `installed` / `bin`。
- 单元测试 `only_gemini_uses_cli_only`：13 行规格里只有 Gemini 是 `CliOnly`；Qwen 配置残留照旧 `cli_missing`。
- 真机：本机 `~/.gemini` 里有 Antigravity 的目录，同时装了 npm 版 Gemini CLI，所以结果仍是 `installed`（证据从 `config_file` 变为 `bin`）。「没装 CLI」的分支只有单测覆盖。

## T5 / T6 · 命令查找缓存

### 改动

- `agent_discovery.rs`：
  - 新增 `LookCache` 与纯逻辑 `looked`，照 magpie `proc/look.go`。答案按「目录戳」缓存，「找不到」也缓存。
  - 每 2 秒（`LOOK_FRESH`）最多重取一次戳；戳没变答案照用。目录 100ms 内刚改过（`LOOK_SETTLE`）时戳不稳，这时什么都不留。
  - 环境（PATH、PATHEXT、APPDATA、LOCALAPPDATA、home）一变就清空答案。
  - 取戳和查找都不在锁里做；跨过 forget、戳变了时问到的答案不留。
- `take_stamp`：
  - 戳覆盖进程 PATH、通用兜底目录、注册表 PATH、各家固定落点，以及 Codex 桌面版哈希目录的根。
  - 注册表只在这里读一次，算好的兜底目录存进缓存，未命中的查找直接复用（T6）。
- `forget_bin_lookups()`；`agent_discover` 命令新增可选参数 `force`，为 true 时先清空缓存。
- `agent_version` 也走 `find_bin`，同样受益。

### 验证

- 单元测试：
  - `lookups_share_one_stamp_and_cache_misses`：一轮 13 家只取 1 次戳（注册表只读 1 次），各找 1 次；窗口内再来一轮，不取戳也不查找；过了窗口重取 1 次戳，戳没变不重找。
  - `stamp_env_change_and_forget_invalidate_answers`：戳变、PATH 等环境变、forget 都会重找；forget 之后的答案照常缓存。
  - `unsettled_stamp_keeps_nothing`：戳不稳时不缓存。
  - `stamp_of_marks_recent_changes_unsettled`：刚改过的目录不稳；目录出现后戳会变。
- 真机 release 实测（`discover_real_machine_readonly_smoke`）：

  | 场景 | 改动前 | 改动后 |
  |---|---|---|
  | 首次 `discover()` | 约 100–115ms | 约 120ms（要先取戳） |
  | 2 秒窗口内再次 | 约 100ms | 约 0.6–0.8ms |
  | 过了窗口（重取戳、戳没变） | 约 100ms | 约 3.7ms |
  | 「重新检测」（forget 后） | — | 约 115ms，全量重找 |
  | 单个命令查找（命中缓存） | 4–12ms | 约 7µs |
  | `registry_path_dirs()` | 约 0.09ms | 约 0.1ms（不是瓶颈） |

## T7 · Agent 页不重复等待

### 改动

- `views/AgentsView.tsx`：
  - 用模块级变量 `lastDiscovery` / `lastVersions` 跨挂载记住上次结果，组件挂载时用它们初始化，再在后台刷新，不节流。
  - 探测和版本查询分开落地，探测结果不再等 `--version`。查询失败时清空，不拿旧结果冒充。
  - 「重新检测」调 `detect(true)`。
- `lib/gatewayApi.ts`：`agentDiscovery(force = false)` 把 `force` 传给后端。`GatewayView` 的调用不变。

### 验证

- `pnpm --dir frontend typecheck` 通过；`node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs` 445 passed / 0 failed。
- **未做真实桌面验证**：没有在 `pnpm tauri dev` 里实际来回切页面看效果；后端调用耗时以上表的真机实测为准。

## T8 · 同名程序甄别

### 改动

- `Spec.bin_not: Option<fn(&Path) -> bool>`（鼠鼠 2026-11-16 选定）。`discover_with` 里，命中 `bin_not` 的命令当作没找到。13 行里只有 Goose 设为 `Some(go_program)`。
- `go_program` / `has_go_magic`：用 256 KB 缓冲顺序找 Go 构建信息魔数，跨两次读取的魔数也能接上；结果按「路径 + 大小 + 修改时间」缓存。只认魔数，不像 magpie 那样再用 `debug/buildinfo` 确认。
- `dir_goose`：Windows 上改为 `%APPDATA%\Block\goose\config`，依据 magpie `gooseIn`，原来误用了 Linux 布局。
- Continue（`cn`）没做甄别：magpie 里没有这家，没有参考（见执行文档 T8 核对第 2 条）。

### 验证

- 单元测试：
  - `go_magic_is_found_across_buffer_boundary`：魔数在开头、横跨 256 KB 缓冲边界、600 KB 文件里没有魔数、文件不存在。
  - `go_goose_on_path_is_not_block_goose`：Go 版 goose 判为没找到命令；同样内容的 qwen 照常找到；换成非 Go 文件后 goose 能找到。
  - `only_goose_has_bin_not`、`goose_dir_on_windows_is_under_appdata_block`。
- 真机只读 `go_magic_real_binaries_readonly_smoke`（release）：
  - GitHub CLI 的 `gh.exe`（Go，40 MB）判为 Go，用时 23–29ms。
  - Codex 自带的 `rg.exe`（Rust，4 MB）判为非 Go：冷读 211ms，文件缓存热后约 3ms。
  - 两者命中缓存后都在 0.1ms 以内。
  - 按热读速度估算，260 MB 的 Block goose 首次约 200ms，和 magpie 注释的 150ms+ 同一量级；只有 PATH 上真有 goose 时才会扫。
  - 本机没装任何 goose，没做「PATH 上放一个 Go 版 goose」的真机验证。

## T9 · 路径占用

### 改动

- `path_taken(path)`，对应 magpie `Taken`：从配置目录本身往上找第一个存在的路径，是文件就算被占。
- `discover_with` 里，配置目录被占时直接 `not_found`，`Standard` 和 `CliOnly` 都适用；`cli_found` 仍然如实报告。

### 验证

- 单元测试 `path_taken_checks_self_and_ancestors`：目录位置是文件、上级是文件、目录存在、一路都不存在。
- 单元测试 `taken_config_dir_is_not_found`：`.qwen`、`.gemini` 是文件，即使命令在也判 `not_found`。
- 真机：本机没有这种情况，真机探测结果与 T7 时一致。

## 全量与真机只读探测（2026-11-16）

- T8 / T9 完成后全量 `cargo test --manifest-path backend/Cargo.toml`：1413 passed / 0 failed / 51 ignored，没有编译警告；真机只读探测结果与下表一致。
- T5–T7 完成后全量 `cargo test --manifest-path backend/Cargo.toml`：1407 passed / 0 failed / 50 ignored，没有编译警告。

- 全量 `cargo test --manifest-path backend/Cargo.toml`：1403 passed / 0 failed / 49 ignored，没有编译警告。
- 真机只读探测 `discover_real_machine_readonly_smoke`（`#[ignore]`）的结果：

  | agent | 状态 | 证据 |
  |---|---|---|
  | claude-code / codex / opencode | installed | config_file |
  | gemini | installed | bin |
  | zcode | cli_missing | config_file |
  | 其余 8 家 | not_found | — |

  ZCode 是桌面应用，没有 `zcode` 命令。它显示为 `cli_missing` 是原有行为：前端 `agentPage.ts` 的 `DESKTOP_CLIENTS` 已把 ZCode / Zed 的 `cli_missing` 当成可接入，本次未改。
