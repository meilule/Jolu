# Agent 探测修复验收记录

对应执行文档：`docs/plans/2026-11-16-agent-discovery-fix.md`。参考 magpie `40f7de24`（调查时为 `e9dd8e8`，差异见执行文档 T1 / T2 的「执行前核对」）。

| 任务 | 状态 |
|---|---|
| T1 桌面版 / 官方安装器落点 | 已完成 |
| T2 ZCode 数据目录跟随 | 已完成，已扩展到额度 / 账号导入 / 归因 / 本机来源 |
| T3 目录内容甄别 | 已完成 |
| T4 例外规则钩子 | 已完成（Gemini `CliOnly`） |
| T5–T9 | 未开始 |

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

## 全量与真机只读探测（2026-11-16）

- 全量 `cargo test --manifest-path backend/Cargo.toml`：1403 passed / 0 failed / 49 ignored，没有编译警告。
- 真机只读探测 `discover_real_machine_readonly_smoke`（`#[ignore]`）的结果：

  | agent | 状态 | 证据 |
  |---|---|---|
  | claude-code / codex / opencode | installed | config_file |
  | gemini | installed | bin |
  | zcode | cli_missing | config_file |
  | 其余 8 家 | not_found | — |

  ZCode 是桌面应用，没有 `zcode` 命令。它显示为 `cli_missing` 是原有行为：前端 `agentPage.ts` 的 `DESKTOP_CLIENTS` 已把 ZCode / Zed 的 `cli_missing` 当成可接入，本次未改。
