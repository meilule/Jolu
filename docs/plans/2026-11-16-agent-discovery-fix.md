# Agent 探测修改执行文档

日期：2026-11-16
状态：执行中（T1–T4 已完成，T2 已扩展到额度 / 账号导入 / 归因 / 本机来源，见 `docs/verification/2026-11-16-agent-discovery-parity.md`；T5–T9 未开始）
范围：只动 `backend/src/agent_discovery.rs`（及必要的 `agent_version.rs`、前端调用节奏），不动接线（`gateway/wiring.rs`）、不动模型路由。
读者：接手执行的同事。

---

## 0. 执行纪律（先读这条，优先级高于本文所有内容）

**本文是调查记录，不是规范。执行前必须到两个项目里自己核对一遍，禁止照抄本文的结论或代码片段。**

具体要求：

1. **每个任务动手前，先在 magpie 里读对应的实现**，确认本文描述的行为仍然成立。magpie 是活跃仓库，行号会漂移；本文给的行号只用于快速定位，**以函数名为准**。
2. **同时在 Jolu 里读对应的现有实现**，确认本文写的「现状」还准确。如果 Jolu 侧已经改过，以代码为准，并把差异记回本文档。
3. **任何一处与本文不符，先停下来更新文档，再动手改代码**。不允许「文档说要这样，代码不是这样，那我按文档改」。
4. **不确定的行为不要猜**。magpie 里有大量带注释的边界处理（例如为什么某个判定要排除某类目录），注释通常写明了踩坑历史。读注释，别只看代码。
5. 本文出现的 magpie 路径均相对 `D:\develop\_research\magpie`，参考 commit `e9dd8e8`（2026-11-13 调查时）。执行前先 `git log --oneline -1` 确认当前 commit，如果已更新，重点复核本文引用的函数是否还在、行为是否变化。

---

## 1. 背景：现在有两个独立问题

### 问题 A：探测不准（用户实际踩到）

1. **Codex 探测不到**：本机 Codex 是 MSIX 桌面版，CLI 在
   `AppData\Local\OpenAI\Codex\bin\<hash>\codex.exe`，既不在 PATH，也不在现有兜底目录里。
   探测结果是 `cli_missing`（有配置、无 CLI），前端归到「暂不能接入」折叠组，看起来像没装。
2. **ZCode 可能探测不到**：ZCode 允许用户在设置里改「数据存储路径」，改过之后
   `.zcode/v2` 整体搬家，真路径记在 `~/.zcode/v2/setting.json` 的 `dataBaseDir` 里。
   Jolu 只认 `~/.zcode`，搬过家的用户探测不到。
3. **卸载残留会误报**：目录存在就算「已装」，不管里面只剩 `logs/`、`cache/` 这类运行时残留。
4. **同名程序会误报**：PATH 里有同名但不相干的程序（例：pressly 的 Go 写的 `goose` 不是 Block 的 Goose），会被当成 agent 的 CLI。

### 问题 B：探测慢（用户反馈「每次进页面都探测一次，很麻烦」）

1. `find_bin` 没有任何缓存：每探测一家工具就扫一遍 PATH + 6 类兜底目录 + **读两次注册表**（HKCU / HKLM 的 PATH）。
2. 前端 `AgentsView` 组件挂载即调 `detect()`；切走页面会卸载组件，切回来重新挂载再探测一次。
3. 结果：来回切几次 Agent 页 = 跑几次全量探测，每次都包含 26 次注册表读取。

### 一个反直觉的事实

magpie **也是每次调用都全量探测**。`internal/gui/api.go` 的 `state()` 注释原话：

> `state() is asked for after nearly every click`

它不缓存探测结果，而是让**每次探测足够便宜**——三层小缓存（见 T5–T7）。所以性能问题的正解是「让单次探测便宜」，不是「少探测」。

---

## 2. 任务清单

按建议顺序执行。每项都写了「先调查什么」，请照做。

### T1 · 桌面版 / 非标准安装位置的 CLI 识别（P0，解决 Codex）

**先调查**

- magpie：`internal/agent/desktopapp.go` 的 `HasApp()` / `desktopApp.installed()`（Windows 上查安装目录文件与注册表卸载项）；`internal/agent/gone.go` 的 `cliHeld()` 里 `a.ID` 的 switch 分支（`claude` / `opencode` / `codex` 各自的官方安装器落点）。
- Jolu：`backend/src/agent_discovery.rs` 的 `find_bin` / `user_bin_dirs` / `registry_path_dirs`。
- 本机实测：`ls "$LOCALAPPDATA/OpenAI/Codex/bin"` 看目录结构，确认 `codex.exe` 在哈希子目录里。

**要做什么**

在 `find_bin` 的查找目录清单里补 Windows 桌面应用的常见落点，至少覆盖：

- `%LOCALAPPDATA%\<Vendor>\<App>\bin\*\*.exe`（哈希子目录形式，Codex 就是这样）
- `%LOCALAPPDATA%\Programs\<App>\`
- MSIX 包目录 `%LOCALAPPDATA%\Packages\<PackageFamilyName>\LocalCache\...`（按需，先确认目标工具是否真在这里）

**注意**

- 这是**兜底清单**，不是「凡是找到就算」。找到后仍要走 T3 的同名程序甄别（如果做了的话）。
- 不要把整个 `%LOCALAPPDATA%` 递归扫，会慢。按已知厂商目录逐个拼。

**验收**

- 本机 Codex 的探测结果从 `cli_missing` 变成 `installed`，`cli_path` 指向真实 exe。
- 单元测试：用 `discover_with` 注入临时目录，断言「哈希子目录里的 exe 能被找到」。

**执行前核对（2026-11-16，magpie `40f7de24`，比调查时的 `e9dd8e8` 新 3572 个提交）**

与上文不符的地方，按 §0 第 3 条先记在这里：

1. **本机现状变了**：`%APPDATA%\npm` 下 11-14 10:58 装上了完整的 npm 版 Codex（`codex` / `codex.cmd` / `codex.ps1` 加 `node_modules/@openai/codex`），而 npm 目录本来就在 `user_bin_dirs` 里，所以本机 Codex 现在已经能探测到，「从 `cli_missing` 变成 `installed`」在本机复现不出来。改为验证「去掉 PATH 与 npm 后，只靠桌面版目录也能找到 `codex.exe`」。
2. **本机桌面版落点核实与上文一致**：`%LOCALAPPDATA%\OpenAI\Codex\bin\5449c1f0f26f1f98\codex.exe`；同级另一个哈希目录 `a7d4c4900eafc659` 里只有 `rg.exe`，所以要逐个子目录找，有多个时取 `codex.exe` 最新的那份。MSIX 虚拟化目录 `%LOCALAPPDATA%\Packages\OpenAI.Codex_2p2nqsd0c76g0\LocalCache` 里没有 `codex.exe`，这条不加。
3. **magpie 不靠找这个 exe 识别桌面版**：`desktopapp.go` 的 `desktopApps["codex"]` 用 AppX 注册表里的 Store 包（`OpenAI.Codex`，发布者 `2p2nqsd0c76g0`）判断桌面应用在不在（`HasApp`），只把它当成「目录不是残留」的证据，不当 CLI。Jolu 的三态里没有「桌面应用」这一档，所以按本文的做法走 CLI 查找，把桌面版自带的 `codex.exe` 当 CLI（`agent_version` 用它跑 `--version` 也能拿到版本）。
4. **magpie 记的官方安装器落点**（`gone.go` 的 `cliHeld()` 与 `cliupdate.go` 的注释）：Claude `~\.claude\local\claude.exe`；OpenCode `~\.opencode\bin\opencode.exe`；Codex 官方 `install.ps1` 的 `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin`（目录联接到 `~\.codex\packages\standalone\current\bin`）。这些都不在 PATH 也不在现有兜底目录里，一并加进按工具的兜底清单。
5. `%LOCALAPPDATA%\Programs\<App>\`：magpie 只给 Jolu 没有的那几家桌面应用（DeepSeek Harness 等）配了这类路径，Jolu 的 13 家里没有能从源码确认的，不加。

---

### T2 · ZCode 数据目录跟随（P0）

**先调查**

- magpie：`internal/appdir/zcode.go` 的 `ZCodeHomeOn()` 与 `ZCodeDataIn()`（读 `setting.json` 的 `dataBaseDir`，非绝对路径则忽略）；`internal/agent/zcode.go` 的 `dir` 计算。
- Jolu：`backend/src/agent_discovery.rs` 的 `dir_zcode`；另外**必须同步检查** `gateway/wiring.rs` 里 ZCode 接线用的目录（两处必须同源，否则探测到了却写错位置）。

**要做什么**

1. `dir_zcode` 增加 `dataBaseDir` 读取：读 `~/.zcode/v2/setting.json`，取 `dataBaseDir`，是绝对路径就用它拼 `.zcode`，否则回退 `~/.zcode`。
2. 抽出公共函数（例如 `zcode_data_dir(home) -> PathBuf`），让 `agent_discovery` 和 `wiring` 都用它。

**注意**

- `setting.json` 可能不存在、可能不是合法 JSON、字段可能是字符串或数字——照 magpie 的做法：任何异常都回退默认值，不报错。
- 改完必须验证「探测到的目录」和「接线写的文件」是同一个。

**验收**

- 造一个临时 home：`setting.json` 里 `dataBaseDir` 指向临时目录，`.zcode/v2/provider_config.json` 放在那个目录 → 探测为 `installed`。
- 不带 `dataBaseDir` 时行为与现在一致。

**执行前核对（2026-11-16，magpie `40f7de24`）**

1. `dataBaseDir` 是 **`.zcode` 的上一级**：magpie `appdir/zcode.go` 的 `ZCodeDataIn` 返回的是要再拼 `.zcode` 的目录；`setting.json` 从**原位置**的 `.zcode/v2/setting.json` 读（ZCode 主进程启动时先读它）。字段不是字符串、去掉空白后为空、不是绝对路径、文件缺失或 JSON 不合法，一律回退原 home。
2. **上文漏了 HOME 这一步**：magpie `agent/zcode.go` 用的是 `ZCodeDataIn(ZCodeHomeOn(home))`。Windows 上用户自己设了绝对路径的 `HOME`、且默认 home 就是 `USERPROFILE` 时，ZCode（Node 程序）读的是 `HOME`。Jolu 的 `library/mod.rs` 已有同款 `zcode_home`，公共函数一并沿用。
3. **不是所有 ZCode 文件都跟着搬**（magpie 注释）：搬走的只有 `.zcode/v2`（provider、config、登录凭据、tasks-index）；Skills、`AGENTS.md`、CLI 的 `cli/config.json` 留在 HOME 下，`cli/db/db.sqlite` 会话库留在 `USERPROFILE` 下（`sessions/zcode.go` 的 `ZCodeDir`）。所以公共函数只给 `v2` 数据用，`skills.rs`、`library`、`session_threads`、`native_sources` 的会话库不改。
4. Jolu 现有 `ZCODE_HOME` 覆盖（值就是 `.zcode` 目录本身）magpie 没有，保留为最高优先级；`wiring::zcode_dir` 原先不过滤空值，统一后空值视为未设置。
5. **范围之外、同样读 `v2` 的地方**：`zcode_quota.rs`（`credentials.json` / `setting.json` / `config.json`）、`gateway/adapter/zcode.rs`（`provider_config.json`）、`attribution.rs`（`provider_config.json`）、`native_sources.rs`（账号来源 `credentials.json`）原先写死 `~/.zcode/v2`。**鼠鼠 2026-11-16 决定一并改**：凭据、provider 规则、config 都走 `creds::zcode_data_dir`；团队套餐读的 `setting.json` 照 magpie `zcode_team.go` 从 ZCode 主目录读（`creds::zcode_home`）；凭据解密口令仍按系统用户目录派生（magpie `zcodeSecret` 用 `os.UserHomeDir`，不随搬家）；Windows 凭据库条目名按 `credentials.json` 的实际路径算，跟着搬家。

---

### T3 · 目录内容甄别：只有残留不算已装（P1）

**先调查**

- magpie：`internal/agent/agent.go` 的 `agentDir()` 与 `leftover` 变量。注意注释里解释了为什么排除 `skills`（`npx skills` 会给每家都建）和 `logs/log/debug/diagnostics/cache`（卸载后残留）。
- Jolu：`classify()` 现在只看 `dir_found = path.is_dir()`。

**要做什么**

把「目录存在」换成「目录里至少有一个不是残留的条目」：

- 残留名单照抄 magpie：`logs` / `log` / `debug` / `diagnostics` / `cache`
- `skills` 单独处理：只有 `skills` 也不算（见 magpie 注释）
- 目录为空时算已装（agent 可能刚建目录还没写文件）——这一点按 magpie 的 `agentDir` 逻辑来，别自己改

**注意**

- 这是纯函数，必须写成可测试的：`fn agent_dir_has_content(path: &Path) -> bool`。
- 读目录失败（权限）时按 magpie 的处理：当作「有内容」，不要误判成未装。

**验收**

- 单元测试覆盖：空目录 / 只有 `skills` / 只有 `logs`+`cache` / 有 `settings.json` / 有未知文件。
- 真机：找一个卸载过的工具，确认它从「已接入」或「暂不能接入」变成不出现或「未安装」。

**执行前核对（2026-11-16，magpie `40f7de24`）**

1. `agentDir(p, own...)` 与上文一致：读不了目录时返回 `isDir(p)`（存在但读不了算有内容，不存在算没有）；空目录算有内容；忽略 `skills`（不论文件还是目录）、`.DS_Store`、`leftover` 里的**目录**（大小写不敏感），以及 `own` 里的**文件**。
2. **上文漏了 `own` 参数**：它是 `Agent.signIn`，即 magpie 替没装过的 agent 写进 Dir 的登录文件（Codex `auth.json`，Claude Code 在非 Mac 上的 `.credentials.json`）。Jolu 的 `cli_apply.rs`（应用到原工具）同样会写这两个文件，所以一并加进 `Spec.sign_in`，其余 11 家为空。
3. 同名的**文件**（例如叫 `logs` 的文件）仍算内容，和 magpie 一致。

---

### T4 · 例外规则钩子（P2，为以后新增铺路）

**先调查**

- magpie：`internal/agent/agent.go` 的 `Agent` 结构体里 `detect` / `sign` / `pathOwn` / `dirShared` 四个可选字段，以及 `Detected()` 里它们各自在哪一步生效。再看 `internal/agent/gemini.go` 的 `detect` 实现，理解「Gemini CLI 只认命令行，因为 `~/.gemini` 也被 Antigravity 用」这个真实案例。
- Jolu：`struct Spec` 现在只有 `agent/name/bin/dir/file` 五个字段。

**要做什么**

1. `Spec` 增加可选字段（建议用 `Option<fn(&Path) -> bool>` 或枚举，具体形态先和鼠鼠确认，别自己定 API）：
   - `detect`：完全接管这家工具的判定
   - `cli_only`：只要命令行在就算装，目录不算证据
2. `discover_with` 里：有 `detect` 就只调它；否则走现有 `classify`。

**注意**

- 这个字段一旦加上，就是给未来所有新 agent 用的扩展点，**API 形态要稳**。先看 magpie 怎么用（`detect` 是 `func() bool`，无参，闭包捕获路径），再决定 Jolu 用函数指针还是别的。
- 加字段后要检查 `SPECS` 的 13 行是否需要逐个填值（默认 `None` / `false`，先保持现状）。
- **首个使用方建议是 Gemini**（照 magpie 的做法设 `cli_only`），作为这个机制的真实用例；不要只加字段不接用例。

**验收**

- Gemini 在「`~/.gemini` 存在但 `gemini` 命令不在」时判为未装（现状会判 `cli_missing`）。
- 其他 12 家行为不变。

**执行前核对与决定（2026-11-16，magpie `40f7de24`）**

1. magpie 的 `detect` 仍是 `func() bool`，在 `Detected()` 第一行生效、完全接管判定。Gemini 的实现是「`~/.gemini` 没被同名文件占用，且 `proc.Look("gemini")` 找得到」，注释写明 `~/.gemini` 卸载后会留下（#230），而且 Antigravity 也用这个目录（#330）。
2. 在 Jolu 的 13 家里，magpie 只给 Gemini 配了 `detect`。`dirShared` / `sign` / `pathOwn` / `binNot` 在这 13 家里都没有使用者。
3. **API 形态（鼠鼠 2026-11-16 选定）**：枚举 `Detect { Standard, CliOnly }`，作为 `Spec.detect` 字段，13 行逐个填值，只有 Gemini 是 `CliOnly`。不提前加 `Custom` 变体：以后真有工具需要完全接管时，再加 `Custom(fn(&Facts) -> Verdict)`，钩子只看探测已收集的事实、不自己读盘，保持可注入测试。
4. `CliOnly` 的语义：目录和文件都不算证据。命令不在 → `not_found`；命令在 → `installed`，证据记 `bin`。路径被同名文件占用（magpie 的 `Taken`）属于 T9，本次不做。

---

### T5 · 命令查找加缓存（性能）

**先调查**

- magpie：`internal/proc/look.go` 的 `lookFresh = 2 * time.Second`、`looked()`、`lookStamp()`。**重点看 `lookStamp()` 的设计**：缓存 key 不是只有命令名，而是「命令名 + PATH + PATHEXT + 每个查找目录的 mtime」——目录变了缓存自动失效，这是它能放心用 2 秒窗口的原因。
- Jolu：`find_bin` 现在每次全量扫。

**要做什么**

在 `agent_discovery.rs` 加进程级缓存（`LazyLock<Mutex<HashMap<..>>>`）：

- key：命令名 + `lookStamp`（PATH、PATHEXT、各查找目录 mtime 的拼接）
- value：找到的路径（或 None）
- 时间窗 2 秒

**注意**

- 缓存的是「找不到」也要缓存，否则最慢的「全扫一遍没找到」每次都要重来。
- 不要缓存成「全局一个 map 只按命令名」——PATH 变了会拿到过期结果。
- 注册表读取（T6）不在这个缓存里，单独处理。

**验收**

- 连续调用 `discover()` 两次，第二次不再读盘（可加日志计数或测试断言调用次数）。
- 修改 PATH 环境变量后，缓存立即失效（`lookStamp` 包含 PATH）。

---

### T6 · 注册表 PATH 单独缓存（性能，最贵的一步）

**先调查**

- Jolu：`registry_path_dirs()` 现在每次探测读 HKCU + HKLM 两次 `RegGetValueW`。
- magpie：`internal/proc/path_windows.go` 的 `LoginPath()` 是否有缓存、缓存多久。

**要做什么**

给注册表 PATH 加缓存，时间窗比 T5 长（注册表 PATH 极少变，建议 30–60 秒，具体值先看 magpie 的取值再定）。

**注意**

- 缓存的是「展开后的目录清单」，不是原始字符串。
- 与 T5 的缓存分开，不要混在一个 map 里。

**验收**

- 探测 13 家工具，注册表读取次数从 26 次降到最多 2 次（加计数器断言）。

---

### T7 · 整体探测短缓存 / 前端节流（性能）

**先调查**

- Jolu 前端：`frontend/src/views/AgentsView.tsx` 的 `detect()` 在哪个 `useEffect` 里、依赖是什么；`frontend/src/views/MainPanelWindow.tsx` 里 `{page === "agents" && <AgentsView/>}` 的挂载行为。
- magpie 前端：`internal/gui/assets/app.js` 里 `state()` 的调用时机（几乎每次点击后），确认它没有「进页面才探测」的特殊处理。

**要做什么**（二选一或都做，先和鼠鼠确认取舍）

- **方案 A（后端）**：`discover()` 加 1–2 秒结果缓存，返回 clone。
- **方案 B（前端）**：`AgentsView` 记住上次探测结果（模块级变量或 sessionStorage），挂载时先显示缓存、后台再刷新；或者给 `detect()` 加最小间隔节流。

**注意**

- 方案 A 会让「点重新检测」在 1–2 秒内拿到旧结果——需要给手动刷新留一个「强制绕过缓存」的参数，否则用户点了没反应。
- 方案 B 要注意「窗口切回来」时是否要刷新（用户可能刚在别的软件里装了工具）。

**验收**

- 快速来回切 Agent 页 3 次，后端探测只跑 1 次（加日志确认）。
- 点「重新检测」始终能拿到最新结果。

---

### T8 · 同名程序甄别（P3，可选）

**先调查**

- magpie：`internal/agent/agent.go` 的 `goProgram()`（读 Go build-info 魔数 `\xff Go buildinf:`，小缓冲扫描 + 按路径/大小/mtime 缓存）与 `internal/agent/gone.go` 的 `cliHeld()` 里对 goose 的特殊处理。
- 判断依据：哪些家可能撞名（目前 Jolu 的 13 家里，`cn`（Continue）和 `goose` 风险最高）。

**要做什么**

只给已知会撞名的工具启用，不做全量二进制扫描（magpie 注释明确说扫描大二进制要 150ms+）。

**验收**

- 造一个 Go 写的假 `goose` 在 PATH 里，探测结果不受影响。

---

### T9 · 路径占用反查（P3，可选）

**先调查**：magpie `internal/agent/agent.go` 的 `Taken()`。

**要做什么**：配置路径（或其祖先）被同名**文件**占着 → 判未装。

**验收**：造一个同名文件挡住配置目录，探测判未装。

---

## 3. 通用要求

### 不能破坏的东西

1. **三态语义**（`installed` / `cli_missing` / `not_found`）和前端对它的依赖：
   - `frontend/src/lib/agentPage.ts` 的 `agentRow()` 按这三态分组
   - `AgentRow.supported`（V8「暂不支持」卡）依赖 `WIRABLE_AGENTS`，与探测无关，别动
2. **`discover_with(home, bin_lookup)` 的可注入测试形态**：所有新逻辑都要能注入假 home 和假命令查找，不允许只能靠真机验证。
3. **只读**：探测不得创建目录、写文件、运行 agent 的命令（版本探测在 `agent_version.rs`，那里已有 24 小时缓存与 3 秒总超时，本次不动）。

### 验证要求

- 每项任务都要有单元测试（`#[cfg(test)]`，用 `discover_with` 注入）。
- 改完跑：`cargo test --manifest-path backend/Cargo.toml`（全量）。
- 涉及前端改动时跑：`pnpm --dir frontend typecheck` + `node --test frontend/src/lib/*.test.mjs`。
- 真机验证：至少验证 Codex（本机 MSIX 版）与一个卸载残留案例。
- 验证结果写进 `docs/verification/2026-11-16-agent-discovery-parity.md`。

### 提交要求

- 按阶段提交（例如：T1+T2 一个、T3+T4 一个、T5–T7 一个），不要把所有任务混成一个提交。
- 提交信息加 `[skip ci]`（未要求发版）。
- 提交前更新 `AGENTS.md` 模块 J 的「现有实现」列，写明探测能力现状与剩余差距。

---

## 4. 明确不做

- 不重写 Jolu 的探测为 magpie 的「每家一个文件」形态——13 家规模不值得，用 T4 的钩子字段覆盖例外即可。
- 不做 WSL 内 agent 探测（magpie 有，Jolu 现在没有 WSL 需求）。
- 不做插件式 agent 注册（magpie 的 `Plugin` 字段）。
- 不改 `agent_version.rs` 的版本探测逻辑。
- 不动接线（`gateway/wiring.rs`）的行为，除了 T2 要求的数据目录同源。
