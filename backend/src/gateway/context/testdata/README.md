# 上下文解析测试样本

`gateway/context/tests.rs` 用 `include_str!` 读这里的请求体。文件名格式为 `<来源>-<序号>.<协议>.json`，协议决定按哪种格式解析。

## 本机抓取（2026-11-17）

| 文件 | 工具与版本 | 协议 |
|---|---|---|
| `claude-code-2.1.326-{1,2,3}.anthropic.json` | Claude Code 2.1.326 | Anthropic Messages |
| `codex-0.169.0-{1,2,3}.responses.json` | Codex CLI 0.169.0 | OpenAI Responses |
| `opencode-1.19.5-{1,2,3}.chat.json` | OpenCode 1.19.5 | OpenAI Chat Completions |
| `gemini-cli-0.68.0-{1,2,3}.gemini.json` | Gemini CLI 0.68.0 | Gemini 原生 |

生成过程：

1. `scripts/acceptance/context-capture.mjs` 在隔离主目录里运行四个 CLI，全部指向本机回环的假上游 `context-capture-upstream.py`。
2. 假上游先让 CLI 用读文件工具读 `README.md`，再用 shell 执行 `cat src/cart.ts`，最后回一句文字。
3. 每个工具都抓到三个请求体，依次是：只有提问；读完一个文件；读完两个文件。OpenCode 的第 1 个是它起标题时发的小请求，第 3 个是读完两个文件后的请求。
4. `scripts/acceptance/context-sample-sanitize.py` 脱敏后写入本目录：
   - JSON 结构、键名、工具名、命令与路径参数原样保留。
   - 其余文字按字符类做等长填充：字母换成 lorem，非 ASCII 字符换成 UTF-8 字节数相同的字符，数字、标点与空白不动。
   - 解析器依赖的标记原样保留，包括：标签、`Contents of …`、`Instructions from:`、`--- Context from: … ---`、`# AGENTS.md instructions for …`、技能列表开头等。
   - 本机路径换成 `C:\Users\dev`、`D:\work\demo-shop`、`D:\work\jolu`。

演示项目和指令文件都是自编内容。Codex 会额外扫描真实用户目录下的 `.agents/skills`，所以样本里有这些技能的名字。名字已经填充，原文没有保留。

## magpie 样本

`magpie-claude-code.anthropic.json`、`magpie-codex.responses.json` 原样复制自 [magpie](https://github.com/yetone/magpie) 的 `internal/gateway/testdata/context/claude-code.json` 与 `codex.json`（commit `40f7de24`）。它们是真实请求结构，正文已做等长填充。按 MIT 许可使用，许可原文如下：

```
MIT License

Copyright (c) 2026 yetone

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
