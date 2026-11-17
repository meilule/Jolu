# V10 Agent 上下文观察 · 第 1 期解析层验证

日期：2026-11-17
执行文档：`docs/plans/2026-11-16-agent-context-observation-backend.md`（第 8 节第 1 期）
代码：`backend/src/gateway/context/`
范围：只验证解析层。采集接入、落库、查询命令与界面属于第 2–4 期，本次没有做。

## 1. 样本

每种协议至少 3 个真实请求，共 14 个，放在 `backend/src/gateway/context/testdata/`。来源与生成方法见该目录的 `README.md`。

| 协议 | 样本 |
|---|---|
| Anthropic Messages | Claude Code 2.1.326 × 3，magpie `claude-code.json` × 1 |
| OpenAI Responses | Codex CLI 0.169.0 × 3，magpie `codex.json` × 1 |
| OpenAI Chat | OpenCode 1.19.5 × 3（含 1 个起标题的小请求） |
| Gemini 原生 | Gemini CLI 0.68.0 × 3 |

本机样本的抓取方式：

- 每个 CLI 在隔离主目录里运行，连本机回环的假上游。
- 每个 CLI 都走两轮工具调用：先用读文件工具读 `README.md`，再用 shell 执行 `cat src/cart.ts`，最后回一句文字。
- 抓到的请求体经 `context-sample-sanitize.py` 等长填充、替换路径后才进入仓库。脚本最后检查本机用户名、Git 身份和仓库目录名，都没有残留。

ZCode 没有单独抓：它是桌面应用，要抓就得改它的配置。它走 Anthropic Messages 协议，由 Claude Code 的样本覆盖。

## 2. 自动化测试

命令：`cargo test --manifest-path backend/Cargo.toml --lib gateway::context`

结果：25 项通过。

| 测试 | 核对内容 |
|---|---|
| `samples_add_up_and_unknown_stays_small` | 14 个样本都能解析：各类之和等于总量，每类各项之和等于该类，unknown 占比低于 20%，每类不超过 24 项，类别顺序固定 |
| `claude_code_sample` | 用户与项目的 CLAUDE.md 分别记为 user / project；其余提醒记为 reminders；技能单独一类；Read 读的记为 read，Bash `cat` 读的记为 shell；billing 与 prompt 记为 system |
| `codex_sample` | AGENTS.md 按 `--- project-doc ---` 拆成用户的与项目的；`<skills_instructions>` 是技能；exec_command 的 `cat` 记为文件；prompt、permissions instructions、environment_context 是 system |
| `opencode_sample` | `Instructions from:` 的文件是 memory（`.config` 下的是用户的）；`<available_skills>` 是技能；read 与 bash `cat` 读的都是文件 |
| `gemini_sample` | `--- Context from: … ---` 的 GEMINI.md 是 memory；`<available_skills>` 是技能；`<session_context>` 是 system，不算用户说话 |
| `magpie_claude_code` / `magpie_codex` / `split_skills_ends_at_blank_line` / `codex_mcp_namespace` / `chat_completions` / `gemini_matches_response_by_name` / `responses_held_and_not_a_body` / `calibrated_adds_up_and_keeps_estimate` / `files_of_command_cases` | 移植自 magpie `prompt_test.go`，断言与原测试一致 |
| `gemini_thought_counts_as_chat` | Gemini 的思考段计入对话（与 magpie 一致，见执行文档核对第 3 条） |
| `unrecognized_goes_to_unknown` | 四种协议里认不出的块、条目、角色都记到 unknown，并带上类型名或角色名 |
| `overflow_is_summed` | 超过 24 项时：非对话类把最小的汇总成 `more`，对话类把最早的汇总成 `earlier` |
| `codex_agents_md_split` / `memory_tags` | Jolu 补充的规则 |
| `odd_texts_never_panic` | 多字节字符紧贴标记、标签不闭合、只有标记等文字不 panic |
| `mutated_bodies_never_panic` | 每个样本做 120 次变异：随机把一个节点换成 null、数字、字符串、空数组、空对象等，解析都不 panic；截断后的请求体返回 `None` |
| `guarded_turns_panic_into_none` | 调试构建下 panic 被兜住，返回 `None` |
| `parse_large_body_fast` | 见第 4 节 |

全量后端测试：`cargo test --manifest-path backend/Cargo.toml`，1450 项通过，51 项忽略，0 项失败。

## 3. 各样本八类占比（估算，未校准）

| 样本 | 总量 | system | tools | skills | memory | files | results | chat | unknown |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude-code-2.1.326-3 | 22542 | 30.3% | 56.3% | 7.2% | 5.5% | 0.4% | 0 | 0.2% | 0 |
| magpie-claude-code | 13886 | 45.3% | 49.3% | 0 | 4.3% | 0.5% | 0.1% | 0.6% | 0 |
| codex-0.169.0-3 | 10767 | 42.1% | 42.6% | 13.5% | 0.6% | 0.9% | 0 | 0.4% | 0 |
| magpie-codex | 19294 | 35.1% | 50.4% | 13.5% | 0.2% | 0.6% | 0 | 0.3% | 0 |
| opencode-1.19.5-3 | 16023 | 13.8% | 32.5% | 0.9% | 51.9% | 0.7% | 0 | 0.3% | 0 |
| gemini-cli-0.68.0-3 | 12819 | 51.9% | 44.0% | 2.4% | 0.7% | 0.6% | 0 | 0.4% | 0 |

其余 8 个样本是同一工具较早的请求，占比相近；完整输出用 `-- --nocapture` 运行 `samples_add_up_and_unknown_stays_small` 查看。

OpenCode 的 memory 占一半以上，是因为它读入的项目指令是本仓库根目录的 AGENTS.md，不是演示项目自己那份。演示项目放在本仓库的 `output/` 下，虽然单独 `git init` 过，结果也一样；原因没有深究。这个样本正好覆盖「项目指令文件很大」的情形。

## 4. 性能

请求体：把 Claude Code 样本的对话重复到约 56 万字节，估算约 12.8 万 token。每次解析耗时：

| 构建 | 单次解析 |
|---|---|
| 调试构建（`cargo test`） | 约 10.5ms |
| 发布优化（`opt-level = "s"`，关闭 LTO、`codegen-units = 16`，`cargo test --release`） | 约 2.9ms（20 次平均），低于 5ms 门槛 |

门槛「低于 5ms」只在发布构建里断言；调试构建只打印耗时。

## 5. 与执行文档的出入

见执行文档第 8 节「执行前核对」第 1–10 条，其中第 9、10 条是本次实现时新发现的：

- 第 9 条：发布版是 `panic = "abort"`，`catch_unwind` 兜不住，解析层本身必须不 panic。
- 第 10 条：Gemini CLI、OpenCode、Codex 新版格式需要 Jolu 补规则。

## 6. 未验证

- 校准只做了单元测试，没有拿上游的真实 `input_tokens` 对照；这一项属于第 2 期。
- ZCode 的真实请求没有抓，原因见第 1 节。
- 真机验收（V10-01 / V10-03 展示）属于第 3、4 期。
