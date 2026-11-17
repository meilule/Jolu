# 待办 · 在原会话通道里直接压缩（D-34）

**来源：** `docs/plans/2026-10-22-jolu-remaining-work.md` 决定 D-34（这一版不采用，记入待办）。
**状态：** 2026-11-17 已实现（鼠鼠要求「现在需要完成的全部完成」；生成方式的界面没有画板，标「待确认」）。实现见文末「实现」一节。

## 是什么

压缩会话时不另起新会话，而是用 `claude --resume <id>` / Codex 的恢复参数在**原会话通道**里发压缩指令，让原工具自己生成摘要。

## 当初不做的原因（2026-11-17 核实后已不成立，见文末「实现」）

1. 会写入原会话：压缩指令和摘要会追加到用户的原始会话文件里，违反「不改原始会话文件」。
2. Codex 能否分叉（fork 出新会话再压缩）还没有核实；不能分叉就只能写原会话。
3. 压缩过的会话在原通道里只剩摘要，用户回到原工具继续时看不到完整上下文。

## 如果要做，先确认

- Claude Code 是否有只读分叉（`--fork-session` 一类参数）可以避免写原会话；用 Context7 查当前 CLI 文档。
- Codex 的 `resume` / `fork` 子命令在当前版本的行为。
- 现有「压缩会话」（模块 D，48/50/52/54/57A）的本地方案能否覆盖同样的需求——能覆盖就不做。

## 实现（2026-11-17）

先按本文「如果要做，先确认」的三条核实：

1. **Claude Code 能不能只读分叉**：能。
   - 本机 2.1.326 的 `--help` 有 `--fork-session`（When resuming, create a new session ID instead of reusing the original）。
   - Context7 的官方文档（code.claude.com how-claude-code-works）原话：「Forking with `--fork-session` … copies the history into a new session ID, leaving the original unchanged.」
   - 再加 `--no-session-persistence`（只在 `-p` 下生效），分叉出的会话也不写盘。
2. **Codex 能不能分叉**：能。本机 0.169.0 有 `codex exec fork <SESSION_ID> [PROMPT]`（「Fork a previous session by id into a new session」，PROMPT 为 `-` 时从标准输入读）。加 `--ephemeral` 不落盘；`fork` 没有 `--sandbox` 参数，改用 `-c sandbox_mode="read-only"`。
3. **现有「压缩会话」能否覆盖**：不能完全覆盖。现有方案是 Jolu 先把会话整理成纪要和最近原文，再交给所选 AI；原通道分叉让原工具带着完整原会话（含它自己的压缩摘要）来写，信息更全。所以做成「生成方式」二选一，默认仍是 Jolu 整理。

当初三条不做的理由，现在的情况：

- 会写入原会话：分叉加不落盘后不再写。
- Codex 能否分叉：已核实可以。
- 原通道只剩摘要：原会话不变，所以也不成立。

**代码**（`backend/src/session_handoff.rs`）：
- `fork_args`：分叉参数。会话 id 只允许字母、数字、`-`、`_`，并且不能以 `-` 开头。工具与 MCP 全部禁用。
- `FORK_PROMPT` / `fork_prompt`：同样的八节格式。引用来源只用 `<facts>` 里的 #n，不自己编序号。
- `generate_forked`：只发一次提示词、不写段落纪要；草稿检查与普通压缩相同。
- `handoff_start` 新增 `channel` 参数，值为 `fork` 时：
  - 只能用会话所属工具的账号；
  - Claude 在原会话的项目目录里运行，因为它按项目目录找会话；
  - 修改草稿仍走隔离执行器。
- `run_args` 在 Codex 分叉时，把关闭 MCP 的参数插在会话 id 之前。

**前端**：压缩弹窗新增「生成方式（待确认）」，只对 Claude Code / Codex 会话出现。选分叉时说明「不改原会话、不留新会话、会消耗原工具额度」；账号不是原工具的时候禁用开始按钮。

**验证**：
- 单元测试 `fork_args_resume_without_writing_sessions` 和 `forked_generation_sends_one_prompt_with_facts`，`session_handoff` 共 40 项通过；
- 前端类型检查通过；
- 没有用真实账号跑一次，因为会消耗真实额度，需要鼠鼠同意后再做。
