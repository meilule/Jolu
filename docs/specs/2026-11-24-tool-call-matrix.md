# Agent 工具调用矩阵与工具链边界

日期：2026-11-24
状态：**P3-1 / P3-2 的现状盘点（只读调研，未改代码）。** 本文列出的「支持」都指代码路径和合成测试，**没有一条组合有真实账号的工具调用端到端记录**（见 §8）。
配套计划：[产品对齐执行计划 · P3](../plans/2026-11-23-product-alignment-execution-plan.md)
相关：[能力矩阵与验证基线](2026-11-23-capability-matrix.md)、[MCP 配置管理调研与设计](2026-11-24-mcp-config-management.md)

文件路径相对 `backend/src/`，`gateway/` 前缀在表格里省略时会注明。行号以 2026-11-24 的 `main`（`c8bf1f8`）为准。

## 修复跟踪（2026-11-24，主线程核实后修复）

| 编号 | 状态 | 改动与测试 |
|---|---|---|
| D1 强制 `tool_choice` 让循环跑满 | ✅ 已修 | `tool_loop.rs` `append_round` → `relax_tool_choice`：`any` / 指定工具在第一轮后改为 `auto`；测试 `a_forced_tool_choice_is_relaxed_after_the_first_round`（去掉放开调用即失败） |
| D3 多轮缓存用量只剩最后一轮、流式回放丢缓存字段 | ✅ 已修 | `tool_loop::Usage` 累加 `cache_read` / `cache_creation`（从没报过保持缺省，不写 0）；`message_to_sse` 的 `message_start` 带缓存字段；测试 `cache_usage_of_every_round_is_summed_and_unknown_stays_absent` |
| D5 Responses 流把上游错误块当成功 | ✅ 已修 | `responses_chat.rs` `StreamTranslator::push` 遇 `{"error":…}` 发 `response.failed` 并停止，收尾不再补 `completed`；测试 `an_upstream_error_chunk_fails_the_stream_instead_of_completing` |
| S1 / S7 / S10 / B1 静默丢弃 | ⏸ 待鼠鼠决策 | 改成显式拒绝还是降级并提示（§4；T14、T21 等决策后改断言） |
| D2 中间轮结果不回传、无整体时限与进度 | ⏸ 待鼠鼠决策 | 是否回传中间轮结果、是否加整体时限与流式进度（§5.6、§5.8；T11 依赖此决策） |
| D4、D6–D10 及其余待补测试 | ⬜ 未做 | 按 §7 / §8 清单继续 |

---

## 0. 结论

1. **工具链目前只有一条「代执行」路径：Anthropic 调用方 + 非 Claude 上游 + `web_search_*` / `web_fetch_*` 服务端工具**（`gateway/wire.rs:905-917`）。Chat、Responses 和 Gemini 原生调用方声明的服务端工具没有替身：Responses 的 `web_search` 等在转 Chat 时被**静默丢弃**（S7），Gemini 的 `googleSearch` 被**显式拒绝**。
2. 静默丢弃共 16 处（§4），影响最大的是 S1（Anthropic→Chat 丢掉全部其他服务端工具）、S7 / S10（Responses→Chat 丢掉内置工具及其历史）、B1（Claude 订阅桥不桥接服务端工具）。
3. tool_loop 的边界：最多问上游 5 次、执行 4 轮；流式调用方会被改成「多轮结束后一次性回放 SSE」，客户端在此期间收不到任何字节；混合调用时丢弃网关那部分调用；工具失败会作为 `is_error` 结果续进下一轮；没有独立超时，也没有显式取消处理。**中间轮的搜索结果不回传给客户端，因此不会进入后续用户回合的模型请求**（D2）。
4. 可复现或可推导的缺陷 10 条（§7），其中 D1（强制 `tool_choice` 让循环跑满 5 轮）、D3（缓存用量丢失）和 D5（Responses 流把上游错误当成功）可以直接写失败测试复现。
5. 待补测试 21 条（§8），按文件列出了测试名和断言；其中 T11、T14 依赖鼠鼠决策，暂不写。

---

## 1. Agent → 调用方协议

以 `gateway/wiring_specs.rs` 的 `SPECS`（13 家，`:202-405`）为准。网关按调用方协议分到不同入口：Anthropic Messages、OpenAI Chat、OpenAI Responses 走 `wire::handle`（`gateway/wire.rs:473`），Gemini 原生走 `adapter/gemini.rs`。

| Agent（SPECS id） | 接线写入的协议 | 出处 | 进入的网关入口 |
|---|---|---|---|
| claude-code | Anthropic Messages（`ANTHROPIC_BASE_URL`） | `wiring_specs.rs:204-216` | `wire::handle(Anthropic)` |
| codex | Responses（`wire_api = "responses"`） | `gateway/wiring.rs:527,556` | `wire::handle(Responses)` |
| gemini | Gemini 原生 `/v1beta` | `wiring_specs.rs:238-250`；`adapter/gemini.rs` | `adapter/gemini.rs`（不经 `forward_one`） |
| opencode | OpenAI Chat（`@ai-sdk/openai-compatible`） | `gateway/wiring.rs:678` | `wire::handle(Chat)` |
| zcode | Anthropic Messages（`api.type = anthropic-messages`） | `gateway/wiring.rs:919` | `wire::handle(Anthropic)` |
| kimi | provider `type = "kimi"` | `gateway/wiring.rs:1190` | ?（待确认：Kimi CLI 的 `kimi` 类型实际发 Chat 还是自有格式，仓库内无抓包） |
| dsh | OpenAI Chat（`openai-completions`，v1 只写这一种路由） | `gateway/wiring_dsh.rs:141-147` | `wire::handle(Chat)` |
| pi | OpenAI Chat（`openai-completions`） | `gateway/wiring_pi.rs:139` | `wire::handle(Chat)` |
| pencil | OpenAI Chat（`openai-completions`） | `gateway/wiring_extra.rs:88-104` | `wire::handle(Chat)` |
| qoder / qoder-cn | OpenAI Chat（`protocol: "openai"`） | `gateway/wiring_extra.rs:121` | `wire::handle(Chat)` |
| qwen | OpenAI Chat（`modelProviders.openai`） | `gateway/wiring_qwen.rs:7,135-138` | `wire::handle(Chat)` |
| workbuddy | OpenAI Chat（`url = <base>/v1/chat/completions`） | `gateway/wiring_workbuddy.rs:153` | `wire::handle(Chat)` |

推论：只有 claude-code 和 zcode 两家会触发 tool_loop；codex 的服务端工具走 S7；其余 9 家都是 Chat 调用方，只用函数工具（包括它们自己挂的 MCP 工具）。

---

## 2. 按协议转换路径归并的矩阵

### 2.1 状态分类

- **A：代码支持，有测试**（测试名可在代码里搜到）
- **B：代码支持，无测试**
- **R：不支持，显式拒绝**（返回 4xx 或报错文字）
- **S：不支持，静默丢弃**（不报错，内容直接消失或被改写）。编号见 §4

### 2.2 API Key 来源（`kind = api`，`adapter/direct.rs`）

认证：`direct.rs` 只改写 `model` 和鉴权头（`:56-63,79`），tools、tool_choice 和历史原样交给 `wire.rs` 选定的转换（`gateway/wire.rs:1016-1036`）。direct 本身不记用量，只回填计时（`direct.rs:183-191`）；用量由 `budget::meter`（命名 Key）和 `context::capture` 从最终响应读取。

| 路径（调用方 → 来源协议） | 转换 | 调用方 tools | tool_choice | 历史 tool_result | 回传（非流式 / 流式） | 并行调用 | 证据 |
|---|---|---|---|---|---|---|---|
| Anthropic → anthropic | 无 | 原样（A：直通） | 原样 | 原样 | 原样 | 原样 | `wire.rs:1017`；真 Claude 上游另走 `with_thinking_repair`（`wire.rs:1360`） |
| Chat → openai | 无 | 原样 | 原样 | 原样 | 原样 | 原样 | `wire.rs:1017` |
| Responses → openai（上游支持 Responses） | 无 | 原样 | 原样 | 原样 | 原样 | 原样 | `wire.rs:1018`、`responses_chat::serves_responses` |
| Anthropic → openai | `translate::anthropic_request` / `anthropic_response` | 函数工具 A；服务端工具 **S1** | auto/none/any/tool A；其他 **S3** | tool_use / tool_result A；其他块 **S2** | 非流式 A；流式 A（参数交错有 D7 风险） | A | `translate.rs:102-221,437-497`；测试 `anthropic_request_becomes_chat`、`chat_json_renders_anthropic_message`、`chat_stream_encodes_anthropic_events`、`truncated_chat_stream_never_becomes_successful_messages_stop`、`call_ids_round_trip` |
| Responses → openai（只有 Chat） | `responses_chat::request` / `response_body` | function、custom A；内置工具 **S7** | 字符串、带 name A；其他 **S9** | function / custom 的 call 与 output A；其他项 **S10**；非字符串 output **S12** | 非流式 A；流式 A（错误块见 D5） | A | `responses_chat.rs:115-216,248-461`；测试 `a_codex_request_with_history_and_tools_becomes_chat`、`a_chat_reply_becomes_a_responses_object_with_custom_calls_unwrapped`、`streamed_chat_becomes_responses_events_ending_in_completed` |
| Chat → anthropic | `translate::chat_to_anthropic_request` / `chat_response` | function A；其他类型 **S4** | auto/none/required/{function} A；其他 **S5**；`parallel_tool_calls` 不映射 | tool → tool_result A | 非流式 A；流式 A；上游 `server_tool_use` 等块 **S6** | A | `translate.rs:554-683,867-930`；测试 `chat_request_becomes_anthropic_messages`、`anthropic_message_renders_chat_completion`、`anthropic_stream_decodes_chat_chunks`、`truncated_anthropic_stream_never_fakes_chat_completion` |
| Responses → anthropic | 先 Responses→Chat 再 Chat→Anthropic | S7 + S4 叠加 | S9 + S5 叠加 | S10 + Chat→Anthropic | 回程 `chat_response` 后再 `response_body` | A | `wire.rs:1054-1066,1094-1097`；只有分段测试，**无整条路径测试（B）** |
| 协议不匹配且不能转换 | — | — | — | — | — | — | R：`wire.rs:1024-1034` 返回 400 |

### 2.3 订阅来源（`kind != api`，`gateway/wire.rs:1118-1211`）

| 来源 | 认证 | 接收协议 | 调用方 tools / tool_choice | 历史工具结果 | 回传与并行 | stream:false | 用量 | 测试证据 |
|---|---|---|---|---|---|---|---|---|
| codex / opencode | 所选连接的 ChatGPT OAuth | Responses、Chat；Anthropic **R**（`wire.rs:1121`） | Responses 原样 A（`codex.rs:61-64,178-190`）；Chat 带 tools / tool_calls **R**（`codex.rs:199-200`）；Chat 无工具时 tool_choice 等字段 S（`codex.rs:218-222`，影响小） | Responses 原样；Chat role=tool **R** | Responses SSE 原样；Chat 流只出文本 | 上游强制流式，非流式在适配器内取 `response.completed`（`codex.rs:390-430`）；?（待确认：`completed.output` 是否总含 function_call） | `codex.rs:377,415` | `chat_to_responses_maps_messages_and_rejects_tools`、`forward_chat_nonstream_aggregates_sse`、`sanitize_drops_rejected_params_and_forces_stream` |
| claude | 隔离 CLI 的 OAuth（一次性配置） | Anthropic；其他 **R**（`wire.rs:1124`） | 函数工具经网关 MCP 桥接 A；服务端工具 **B1**；tool_choice 只认 auto/none，其他 **R**（`claude_bridge.rs:64-67`） | 最后一条 user 的 tool_result 交回挂起调用；更早历史渲染成文字（`claude_bridge.rs:158-240`） | 流事件原样，非流式由 `assemble` 拼装 | 支持（同一串事件拼装） | 只在 isolated 时记（`claude_bridge.rs:753-756`） | `tool_calls_park_in_the_gateway_and_resume_the_same_client`、`tool_results_become_mcp_results_with_trailing_notes`、`sampling_options_the_client_cannot_honour_are_refused`、`stream_events_assemble_into_a_whole_message` |
| grok | 所选连接授权（刷新串行） | Chat、Responses；Anthropic **R**（`grok.rs:140-145`） | 原样直通（`grok.rs:167-181`） | 原样 | 原样 | 跟随调用方；?（待确认：上游是否支持非流式） | `wire.rs:1205-1207` `record_usage_on_end` | 无工具相关测试（B） |
| zcode（Coding Plan / Start Plan） | 账号凭据 | 三种都收（`zcode.rs:581-592,711-735`） | Anthropic 原样（Start Plan 只剥 cache_control）；Chat → Start Plan 走 S4/S5；Responses 走 S7–S12 | 同左 | 原样或经转换 | 原样发上游，不聚合；?（待确认：两条通道上游是否接受 stream:false，tool_loop 依赖这一点） | `record_usage_on_end` | `start_anthropic_body_converts_every_caller_protocol`；无工具测试 |
| gemini（Code Assist） | Google OAuth | Chat；Anthropic / Responses 先转 Chat（`wire.rs:1146-1171`） | function → functionDeclarations A；其他 **S13** | functionResponse，按 id 查名，查不到 **R**（`gemini_chat.rs:136-148`） | 非流式、流式均支持并行 | 支持（`gemini_subscription.rs:233,368-381`） | `gemini_subscription.rs:256-267` | `chat_history_with_tools_and_images_becomes_gemini_contents`、`gemini_reply_becomes_chat_and_signatures_are_remembered`、`stream_chunks_carry_role_once_and_end_with_the_right_reason` |
| workbuddy | 所选账号签名 | Chat；其他 **R**（`workbuddy.rs:384-389`），wire 先转换 | 原样（`workbuddy_request.rs:59-109`） | 原样；reasoning 字段改名 | 原样 | 上游回 SSE 时拼成 JSON（`workbuddy.rs:464-474`，`workbuddy_retry.rs:205-260`），错误块见 D6 | `record_usage_on_end` | `content_blocks_preserve_images_metadata_and_tool_arguments`、`a_streamed_reply_is_assembled_for_a_non_streaming_client` |
| qoder | 设备登录 | Chat；其他 **R**（`qoder.rs:656-658`） | 原生 function A；对象型 tool_choice 和非 function 工具 **S14**；`required` 只是提示词软约束 | 只留文本，图片挪下一条，孤儿结果并入 user 文本（`qoder_chat.rs:223-260`） | 原生 tool_calls + 文本调用解析（`text_calls.rs`），支持并行 | 上游恒 SSE，适配器聚合（`qoder.rs:799`） | `qoder.rs:530,576,630` | `body_carries_task_session_model_and_typed_tools`、`sse_events_lift_text_reasoning_tools_and_usage`；`text_calls.rs` 6 条 |
| trae | 账号 token | Chat；其他 **R**（`trae.rs:120-125`） | 严格白名单，不认识的字段一律 **R**（`trae_chat.rs:32-47,83-92,151-172`），**无静默丢弃** | 不完整、重复、孤儿结果 **R**（`trae_chat.rs:260-312`） | 完整调用才释放；并行上限 128 | 上游恒流式，适配器聚合（`trae_stream.rs:96-98`） | `trae_stream.rs:74,115,183` | `request_routes_tools_history_and_budget_without_accepting_internal_fields`、`unsupported_images_effort_and_tool_history_fail_instead_of_losing_information`、`incomplete_stream_and_invalid_tools_never_become_success` 等 10 条 |
| 未接入的平台 | — | — | — | — | — | — | — | R：`wire.rs:1128` 返回 501 |

### 2.4 Gemini 原生入口（`adapter/gemini.rs`，Gemini CLI）

| 来源 | 工具 | tool_choice | 历史 | 回传 | 证据 |
|---|---|---|---|---|---|
| openai 协议 API 来源 | functionDeclarations → Chat tools；`googleSearch` 等内置工具 **R**（`gemini.rs:391-393`）；`parametersJsonSchema` **S15** | `mode` 映射 ANY/NONE/auto；`allowedFunctionNames` **S15** | functionCall / functionResponse 的 id 由名字合成 `call_{name}`（`gemini.rs:307,318`），同名并行调用会撞 id ?（待确认上游表现） | 非流式、流式（到 finish 一次性发出） | `translates_function_declarations_and_calls`、`rejects_internal_tools_and_multiple_candidates`、`stream_translates_text_tool_calls_and_finish` |
| anthropic 协议 API 来源 | — | — | — | — | R：`gemini.rs:165-173` |
| gemini 订阅 | 原生透传（`gemini_subscription::forward`） | 原样 | 原样 | 原样 | `chat_and_native_streams_relay_code_assist_events` |
| 其他订阅 | — | — | — | — | R：`gemini.rs` 中 `RouteDecision::Subscription` 分支返回 400 |

---

## 3. 工具类别 × 路径

| 工具类别 | 谁执行 | 网关看到什么 | 覆盖路径 | 状态 |
|---|---|---|---|---|
| **Agent 自带工具**（调用方 tools，如 Read / Bash） | Agent 本地 | 普通函数工具 + 历史 tool_result | §2 全部路径 | 见 §2 各格：直通 A；转换路径 A，叠加 S1–S14 |
| **MCP 搜索 / 抓取**（`library/mcp_search.rs`，stdio，`jolu.exe --mcp-search`） | Agent 拉起的 Jolu 子进程 | 对网关来说就是 Agent 自带的函数工具（如 Claude Code 里叫 `mcp__jolu-search__web_search`） | 与 Agent 自带工具相同 | 协议层 A：`initialize_and_tools_list_shape`、`notifications_are_silent_and_unknown_tools_error`、`web_search_without_config_reports_actionable_error`；**Agent 真实挂载与调用无记录** |
| **上游服务端工具代执行**（`gateway/tool_loop.rs`） | Jolu 网关 | Anthropic `web_search_*` / `web_fetch_*` 服务端工具 | 只限 Anthropic 调用方 + 非 Claude 上游（`wire.rs:905-917`） | A：`tool_loop.rs` 6 条 + `wire.rs` 4 条（§5.8）；实机未验证（`docs/plans/2026-11-09-complete-remaining-work.md:88`） |
| 上游服务端工具（原生） | 上游 | 服务端工具原样 | 真 Claude（API 与订阅）、Codex 订阅的 Responses 内置工具、Gemini 订阅原生 | 直通 B（无工具测试）；Claude 订阅见 B1 |
| 上游服务端工具（无替身） | 无人 | Responses `web_search` / `file_search` 等、Anthropic 其他服务端工具、Gemini `googleSearch` | 转换路径 | Responses / Anthropic：**S**（S1、S7）；Gemini：**R** |
| **订阅工具桥**（`adapter/claude_bridge.rs`） | Agent 本地（经网关 MCP 端点挂起续跑） | 调用方 tools 改名 `mcp__jolu__<名>` 交给 claude CLI | 只限 Claude 订阅 | A（假客户端）：`tool_calls_park_in_the_gateway_and_resume_the_same_client`；真实账号验收归鼠鼠（`docs/plans/2026-11-09-complete-remaining-work.md:109`） |
| **多种工具混用** | — | 同一轮既有 Agent 工具又有替身工具 | tool_loop | A：`a_round_ending_in_foreign_tools_drops_managed_calls_and_streams_back`（行为是丢弃替身调用，见 §5.3） |

---

## 4. 静默丢弃清单

S1–S15 来自对 `translate.rs`、`responses_chat.rs` 和各适配器的逐行核对，B1 是 Claude 订阅桥的同类问题。**全部不报错，大部分不打日志。**

| # | 位置 | 触发条件 | 丢掉什么 |
|---|---|---|---|
| S1 | `gateway/translate.rs:185-193` | Anthropic 调用方 → openai 协议 API 来源（`wire.rs:1049`），或 workbuddy / qoder / trae / gemini 订阅（`wire.rs:1154,1178`） | 没有 `input_schema` 且 `type` 不是空或 `custom` 的工具，即 `bash_*`、`text_editor_*`、`computer_*`、`code_execution_*`、`memory_*` 等全部服务端工具，以及 `DeferredToolPlaceholder`。`web_search_*` / `web_fetch_*` 一般已被 tool_loop 换掉；但 Agent 另有同名普通工具时 tool_loop 不接管（`tool_loop.rs:63`），这时它也被丢。无日志 |
| S2 | `gateway/translate.rs:124-149,45-80` | 同 S1 | 历史中的 `server_tool_use`、`web_search_tool_result`、`redacted_thinking`、`document`、`search_result`、mcp 块；tool_result 里只留 text 和 image；只有 thinking 的 assistant 轮整轮跳过（`:154`）。另：`is_error` 为真但文本为空时错误标记丢失（`:141-143`，D10） |
| S3 | `gateway/translate.rs:197-203` | 同 S1 | auto / none / any / tool 以外的 tool_choice；`tool` 指向 S1 删掉的工具时会被上游拒绝 |
| S4 | `gateway/translate.rs:644-648` | Chat / Responses 调用方 → anthropic 协议 API 来源（`wire.rs:1054`），或 ZCode Start Plan（`zcode.rs:581-592`） | `type` 不是 function 的工具 |
| S5 | `gateway/translate.rs:659-669` | 同 S4 | auto / none / required / {function} 以外的 tool_choice（如 `allowed_tools`）；`parallel_tool_calls` 不映射 |
| S6 | `gateway/translate.rs:711-738,797,811` | anthropic 上游回程转 Chat（`chat_response`） | 回复里的 `server_tool_use`、`web_search_tool_result` 块，流式和非流式都丢 |
| S7 | `gateway/responses_chat.rs:185-196` | Responses 调用方 → 只有 Chat 的 API 来源（`wire.rs:1040`）、workbuddy / qoder / trae / gemini（`wire.rs:1155,1179`）或 zcode（`zcode.rs:586,722`） | function、custom 以外的工具：`web_search*`、`file_search`、`local_shell`、`computer_use*`、`mcp`、`image_generation`、`code_interpreter` 等。只有服务端 `eprintln`，调用方收不到提示；现有测试 `a_codex_request_with_history_and_tools_becomes_chat` 把这一行为当预期断言 |
| S8 | `gateway/responses_chat.rs:181,202` | input 含 `compaction_trigger` | 全部 tools 与 tool_choice（**设计如此**，压缩请求不需要工具；列入是为了完整） |
| S9 | `gateway/responses_chat.rs:202-206` | 同 S7 | 不带 `name` 的对象型 tool_choice（`{"type":"file_search"}`、`allowed_tools`）。另外，字符串型 `"required"` 在工具被 S7 删光后仍照发，可能导致上游 400 |
| S10 | `gateway/responses_chat.rs:166-167` | 同 S7 | 历史项只认 message / function_call / custom_tool_call / 两种 output / compaction；`reasoning`、`local_shell_call`、`web_search_call`、`file_search_call`、`computer_call`、`mcp_*`、`item_reference`、`image_generation_call` 全部无日志丢弃，内置工具的调用历史整段消失 |
| S11 | `gateway/responses_chat.rs:100` | 同 S7 | 内容块 `input_file`、`input_audio`、`refusal` |
| S12 | `gateway/responses_chat.rs:153-156` | 同 S7 | `function_call_output.output` 不是字符串（数组、含 `input_image`）时被序列化成 JSON 文本，图片语义丢失 |
| S13 | `gateway/adapter/gemini_chat.rs:161-188,45-55` | Gemini 订阅（所有调用方先转 Chat） | 非 function 工具；没有函数声明时 tool_choice 一并丢；`parallel_tool_calls` 不读；工具结果里的图片 |
| S14 | `gateway/adapter/qoder_chat.rs:494-502,231-242` | Qoder 订阅 | 对象型 tool_choice（指定函数）被忽略；`required` 只在系统提示里加一句；非 function 或无名工具丢；`parallel_tool_calls` 不读；孤儿工具结果并入 user 文本 |
| S15 | `gateway/adapter/gemini.rs:403,412-421` | Gemini CLI → openai 协议 API 来源 | 函数声明只读 `parameters`，用 `parametersJsonSchema` 时 schema 丢失；`allowedFunctionNames` 被忽略。?（待确认：新版 Gemini CLI 实际发哪个字段） |
| B1 | `gateway/adapter/claude_bridge.rs:141-143` + `:670` | Claude 订阅 + 请求带 `web_search_*` / `web_fetch_*` 等服务端工具 | 服务端工具不桥接，CLI 又以 `--tools ""` 启动，模型无法联网搜索，也不会收到提示。?（待确认：`--tools ""` 下 claude CLI 是否仍能用 API 侧的服务端 web_search；需要真实账号） |

---

## 5. tool_loop 边界（P3-2）

代码：`gateway/tool_loop.rs`（规则与纯函数），`gateway/wire.rs:1279-1349`（`with_tool_loop` 循环与 `deliver_message`）。

### 5.1 触发条件

- 调用方是 Anthropic，且目标不是真 Claude：`native_tools = platform == "claude" || serves_claude(base_url, model)`（`wire.rs:907-908`）。
- 请求里有 `type` 以 `web_search_` / `web_fetch_` 开头的服务端工具，且没有同名普通工具（`tool_loop.rs:52-73`）。否则原样只发一次（测试 `without_server_tools_the_request_goes_once_unchanged`）。
- 每个候选来源各跑一次完整循环（`wire.rs:911`，在候选循环里）。只有整个循环返回可转移错误时才换下一个候选（`wire.rs:935`），所以某轮在第 2 轮之后失败时，已经执行的搜索不会在新候选上复用，会重新开始。

### 5.2 轮数上限

- `MAX_ROUNDS = 5`（`tool_loop.rs:27`）。`for round in 0..5`，`round + 1 == 5` 时不再执行（`wire.rs:1298,1322-1324`）。**实际是最多问上游 5 次，执行 4 轮替身工具。**
- 用尽时 `finish(..., exhausted = true)`：去掉仍在调用的替身工具；没有剩余调用时 `stop_reason` 改为 `end_turn`；开头加一行「联网工具已连续调用 5 轮」（`tool_loop.rs:167-179`）。
- 测试：`exhausted_rounds_end_the_turn_without_dangling_managed_calls` 只测 `finish` 这个纯函数，**没有测试真的跑满 5 次**（T2）。

### 5.3 流式 → 非流式

- `rewrite_request` 把 `stream` 强制设为 `false`，并记住客户端原本是否要流式（`tool_loop.rs:75-102`）。每轮都按非流式问上游，最长读取 32 MiB（`wire.rs:1280,1305`）。
- 最终消息由 `deliver_message` 交回：客户端要流式时用 `message_to_sse` 把整条消息一次写成 SSE，否则返回 JSON（`wire.rs:1338-1349`，`tool_loop.rs:196-241`）。
- **对客户端只是「格式透明」，时间上不透明**：多轮期间客户端收不到任何字节，最后一次性收到全部事件（`tool_loop.rs:19` 模块头已说明；magpie 会边转发边插进度事件，Jolu 没做，见 `docs/plans/2026-11-10-capability-and-correctness-plan.md:285`）。客户端自身的读超时可能在长循环中触发 ?（待确认：Claude Code 的首字节超时值）。
- 回放的 SSE 里 `message_start.usage` 只有 `input_tokens`，`message_delta.usage` 只有 `output_tokens`（`tool_loop.rs:205,237`），缓存字段全部丢失（D3）。
- 上游无视 `stream:false` 仍回 SSE 时，读不出 JSON 就把这一轮原样交回（`wire.rs:1308-1310`），见 D4。
- 测试：`server_tools_become_plain_tools_and_streaming_is_turned_off`、`a_message_replays_as_well_formed_sse`、`a_round_ending_in_foreign_tools_drops_managed_calls_and_streams_back`（断言 SSE content-type）、`managed_calls_are_executed_and_the_next_round_answers`（断言 JSON content-type）。

### 5.4 混合调用

- 同一轮既有替身工具又有 Agent 自己的工具：`calls_in` 标记 `foreign`（`tool_loop.rs:114-135`）。这时**替身调用从回复里删掉、不执行**，加说明「模型同时调用了工具自己的功能，联网调用本轮未执行」，其余原样交给 Agent（`wire.rs:1317-1321`）。`stop_reason` 保持 `tool_use`。
- 后果：模型这次想做的搜索被丢弃。Agent 回传自己的工具结果后，模型通常会再发起搜索，这时进入新一轮循环。
- 测试：`calls_are_read_and_foreign_tools_are_noticed`、`a_round_ending_in_foreign_tools_drops_managed_calls_and_streams_back`。

### 5.5 工具失败

- 搜索未配置后端：返回 `is_error = true` 的「搜索不可用」（`tool_loop.rs:247-250`）。后端报错：原因文本加 `is_error = true`（`:262`）。抓取失败同样处理（`:273`）。结果作为 `tool_result` 续进下一轮（`tool_loop.rs:138-148`），**不会中断循环**。
- 给用户的说明进 `notes`，最终加在回复开头（`tool_loop.rs:181-184`）。
- 测试：`managed_calls_are_executed_and_the_next_round_answers`（覆盖「未配置后端」）。后端报错和抓取失败**无测试**（T5）。

### 5.6 超时

- 搜索后端单次 HTTP 超时 20 秒（`gateway/search/backends.rs:328`），抓取 20 秒（`gateway/search/fetch.rs:75`）。同一轮的多个调用**串行执行**（`wire.rs:1326-1331`）。
- 上游每一轮的超时由各适配器决定，`direct.rs` 本身不设超时；`with_tool_loop` **没有整体时间上限**。最坏情况是 5 次上游调用，加 4 轮 × N 个调用 × 20 秒。
- 测试：无（T11，依赖整体时限的设计决策）。

### 5.7 取消

- 循环没有显式取消逻辑。客户端断开后，只有 handler future 被丢弃时循环才会停止 ?（待确认：hyper 1.x 在 HTTP/1 上，能否在 service future 挂起期间检测到连接关闭并 drop 它；没有测试）。
- 每轮的 `forward_one` 都会套一层 `CancelWatch`（`wire.rs:1115,1226`），但循环内部已经把 body 读完（`wire.rs:1305`），所以它看不到最终的客户端断开。最终交给客户端的响应只经过外层的 `capture.meter` / `budget::meter` / `key_slots::hold`（`wire.rs:951-959`）。
- 测试：无（T9）。

### 5.8 结果能否进入后续模型请求

- **循环内能进入**：`append_round` 把这一轮的 assistant 消息原样追加，再追加带 `tool_result` 的 user 消息（`tool_loop.rs:138-148`）。测试：`rounds_append_and_the_final_message_carries_notes_and_summed_usage`，以及 `managed_calls_are_executed_and_the_next_round_answers`（断言第 2 轮请求里 `messages[2].content[0].tool_use_id == "s1"`）。
- **跨用户回合不会进入**：交给客户端的只有最后一轮的消息，外加一行说明（`tool_loop.rs:167-193`）。中间轮的 `tool_use`、搜索结果正文都不在客户端历史里，下一次用户提问时模型看不到原始搜索结果（D2）。真 Anthropic 会在 content 里返回 `server_tool_use` / `web_search_tool_result` 块，由客户端保留在历史中，这里行为不同。

### 5.9 用量

- `add_usage` 只累加 `input_tokens` / `output_tokens`（`tool_loop.rs:161-164`）；`finish` 用累加值覆盖这两项，`cache_*` 字段保留最后一轮的值（`:186-191`）。
- 订阅来源（grok / workbuddy / zcode）每一轮的 `forward_one` 都会调用 `record_usage_on_end`（`wire.rs:1205-1207`），按轮入库；命名 Key 的预算和上下文观察由外层读最终合计（`wire.rs:951-958`）。
- 每轮都调用 `record_route` 和 `begin_route`（`wire.rs:1072,1100`），所以一次客户端请求会在路由流水里写多行、发多次开始 / 结束事件 ?（待确认：流水页和灵动岛按 `request_uid` 归并后是否算一次请求）。
- 测试：`rounds_append_and_the_final_message_carries_notes_and_summed_usage`、`managed_calls_are_executed_and_the_next_round_answers`（断言两轮之和 30 / 7），都不含缓存字段（T6）。

---

## 6. claude_bridge 边界

代码：`gateway/adapter/claude_bridge.rs`；端点路由在 `gateway/server.rs:35,113,194-197`。

| 场景 | 处理 | 出处 | 测试 |
|---|---|---|---|
| 何时走桥 | 流式、带 tools，或消息里有 image / document / tool_use / tool_result；纯文本非流式仍走一次性进程 | `:41-51` | `requests_that_need_the_bridge_are_recognised` |
| 工具注册 | 调用方函数工具改名为 `mcp__jolu__<名>`（不合规的截短加哈希），schema 折成 object 根；服务端工具跳过（B1）；`tool_choice = none` 时不提供工具 | `:80-154,822-827` | `tool_names_are_kept_when_valid_and_hashed_otherwise`、`union_schemas_are_folded_to_an_object_root` |
| MCP 端点与鉴权 | 每个进程一个 32 字节令牌，写进只有本用户可读的临时 `mcp.json`，不上命令行；`POST /__jolu/claude-mcp/<令牌>` 跳过网关 Key，但仍校验回环 Host，令牌常量时间比对，不对一律 404 | `:12,403-409,848-867,949-961`；`server.rs:65-75` | `unknown_bridge_tokens_are_not_found` |
| 挂起与续跑 | CLI 的 `tools/call` 挂在 `Run.calls` 上；本段以 `tool_use` 结束交给调用方；下一次请求最后一条 user 里的 tool_result 按 id 找到 `issued`，再按「名字 + 参数」或「同名最早」配对交回 | `:331-372,485-495,711-742` | `tool_calls_park_in_the_gateway_and_resume_the_same_client` |
| 并发多个工具调用 | 同一段里的多个 `tool_use` 都记进 `issued`；结果逐个 `deliver`，每个最多等 30 秒，等 CLI 发来对应的 `tools/call`（`MATCH_WAIT`）。任意一个没对上，就结束进程，把整段对话渲染成文字重开新进程 | `:36,348-372,717-736` | **无**（T18） |
| 结果只送回一部分 | 已送达的调用继续；没送达的调用一直挂到 15 分钟（`PARK_IDLE`），而 `pump` 10 分钟无输出就报错（`EVENT_IDLE`），所以调用方会先收到「超过 10 分钟没有新输出」 | `:34,38,337,518-519` | 无 |
| 工具失败 | `is_error` 的 tool_result 变成 MCP `isError` 结果，同一条消息里的文字作为附注带上 | `:242-272` | `tool_results_become_mcp_results_with_trailing_notes` |
| 超时 | 挂起 15 分钟后 `park` 返回 tool_error 给 CLI；两条输出间隔超过 10 分钟就报错；首个事件前出错返回真正的 502 | `:34-38,337-344,518-519,769-774` | `an_early_failure_is_a_real_error_status`（只测早期失败） |
| 客户端断开 | 流式：响应流被丢弃后 `tx.send` 失败，`pump` 返回 `Failed`，进程被 `finish`（先关 stdin，20 秒后强杀）。非流式：等待期间 handler 被丢弃，效果相同 ?（待确认：同 §5.7 的 hyper 行为） | `:563-567,375-388,765` | **无**（T19） |
| 挂起后调用方不再回来 | 只在下一次 `forward` 入口调用 `sweep` 清理闲置超过 15 分钟的进程，**没有后台定时器**；如果没有后续 Claude 订阅富请求，进程和临时目录会一直留着（D8） | `:469-483,701` | **无**（T20） |
| 重复提交同一批结果 | 第一次送达后 `issued` 清空；第二次请求找不到 live run，会新起进程，把整段对话当新对话重新生成 ?（待确认：客户端重试时是否会出现重复生成和重复计费） | `:713-716,738` | 无 |
| tool_choice | 只认 auto / none，any / tool 显式 400 | `:64-67` | `sampling_options_the_client_cannot_honour_are_refused` |
| 用量 | 每段结束时在 isolated 模式下按段记账 | `:753-756` | 无（假客户端测试是非 isolated） |

---

## 7. 缺陷清单（可复现或可由代码推导）

| # | 缺陷 | 位置 | 触发条件 | 影响 |
|---|---|---|---|---|
| D1 | **强制 tool_choice 让循环跑满 5 轮** | `tool_loop.rs:75-102`（不处理 tool_choice）；`wire.rs:1298-1333` | Anthropic 调用方 + 非 Claude 上游；tools 含 `web_search_*`；`tool_choice = {"type":"tool","name":"web_search"}`，或 `{"type":"any"}` 且只有替身工具；上游遵守 tool_choice（AnthropicToChat 会转成 `required` 或指定函数，`translate.rs:197-203`） | 每轮都被迫再搜一次：5 次上游调用加 4 轮搜索，最后的消息里替身调用被删光，可能只剩说明文字。?（待确认：Claude Code 的 WebSearch 子请求是否带强制 tool_choice；如果带，这条会在日常使用中命中） |
| D2 | 中间轮搜索结果不回传给客户端 | `tool_loop.rs:167-193`；`wire.rs:1315` | 任意一次成功的代执行 | 结果不进入后续用户回合；追问时模型只能看到说明行和自己的总结 |
| D3 | 缓存用量丢失 | `tool_loop.rs:161-164,186-191,205,237` | 代执行且上游返回 `cache_read_input_tokens` / `cache_creation_input_tokens` | 非流式时只剩最后一轮的缓存数；流式回放里缓存字段全部消失。命名 Key 预算按 `input + cache_read + output` 计（`budget.rs` 测试 `meter_reads_anthropic_stream_usage_across_chunk_boundaries`），回冲会少算 |
| D4 | 上游无视 stream:false 时回退不完整 | `wire.rs:1308-1310` | 某轮上游仍回 SSE（zcode 两条通道都原样透传，是否会这样待确认） | 发生在第 1 轮：原样交回，没有替身执行，替身工具调用会原样出现在客户端；发生在第 2 轮以后：前几轮的说明和用量丢失 |
| D5 | **Responses 流把上游错误当成功** | `responses_chat.rs:326-328`（没有 `choices` 的块直接返回），`:500-513`（流结束照常 `finish()` 发出 `response.completed`） | Responses 调用方（Codex）→ Chat 上游流式中途发出 `{"error":…}` 块，例如 Qoder（`qoder.rs:635`）、Trae（`trae_stream.rs:119`）、Gemini 订阅（`gemini_subscription.rs:287`） | Codex 看到「成功完成」但内容被截断。对照：Anthropic 回程会把错误块转成 error 事件（`translate.rs:344`） |
| D6 | WorkBuddy 非流式聚合忽略错误块 | `adapter/workbuddy_retry.rs:205-260` | 请求非流式（包括 tool_loop 的每一轮），上游回 SSE 且中途有错误块 | 拼出一条看似成功、`finish_reason` 可能为 null 的回复；tool_loop 会把它当最终回答交出 |
| D7 | 并行工具参数交错时丢参数 | `translate.rs:362-376`（只有当前打开块的 index 才写入 `input_json_delta`） | Chat 上游流式并行调用，某个 index 的参数分片在后一个调用开块之后才到 | 前一个调用的参数不完整。?（待确认：有没有上游这样交错发送；OpenAI 官方是顺序发送） |
| D8 | Claude 订阅桥的闲置进程只被惰性清理 | `claude_bridge.rs:469-483,701` | 调用方拿到 `tool_use` 后放弃（取消、换模型），之后没有新的 Claude 订阅富请求 | claude 进程、临时 `mcp.json`（含令牌）和隔离授权目录一直留到下一次请求或应用退出 |
| D9 | 同名服务端工具被静默丢弃 | `tool_loop.rs:63` + `translate.rs:185-190` | Agent 同时声明普通工具 `web_search` 和服务端工具 `web_search_2025…`，上游是 openai 协议 | 服务端工具既不被接管也不被转发（S1 的子情况） |
| D10 | 空文本错误结果丢失错误标记 | `translate.rs:141-143` | AnthropicToChat，`tool_result.is_error = true` 且 content 为空或只有图片 | 上游看不出工具失败 |

---

## 8. 待补测试

以下测试名都是建议名，断言写的是**期望行为**。对 D1、D3、D5、D6 这类缺陷，修复前先写成失败测试，修复后转绿。

### 8.1 `gateway/wire.rs`（tests 模块，与 `managed_calls_are_executed_and_the_next_round_answers` 同处）

| # | 测试名 | 断言 |
|---|---|---|
| T1 | `forced_tool_choice_on_a_managed_tool_is_relaxed_after_the_first_round` | 请求 `tool_choice = {"type":"tool","name":"web_search"}`，上游第 1 轮调用 web_search、第 2 轮回文字：第 2 轮请求体的 `tool_choice` 不再强制 web_search（为 auto 或被删除），总共只发 2 次（D1） |
| T2 | `the_loop_stops_after_max_rounds_and_reports_it` | 上游每轮都回 web_search 调用：`send` 恰好被调用 `MAX_ROUNDS` 次；最终 `stop_reason == "end_turn"`，没有 tool_use 块，首个文字块含「5 轮」 |
| T3 | `a_failing_later_round_returns_that_error_status` | 第 1 轮成功调用、第 2 轮返回 503：响应状态是 503（以便候选循环转移），不是 200 |
| T4 | `an_unparseable_round_is_passed_through_untouched` | 第 1 轮返回 `text/event-stream` 正文：响应正文与 content-type 原样，`send` 只调 1 次（锁住 D4 的现状，便于以后改） |
| T5 | `fetch_failure_becomes_an_error_result_for_the_next_round` | 上游调用 `web_fetch`，url 是被 netguard 拒绝的回环地址：第 2 轮的 tool_result `is_error == true`，最终说明含「读取 … 失败」（不需要联网）。?（待确认：`search::fetch::fetch` 对回环地址是否立即拒绝） |
| T6 | `cache_usage_of_every_round_is_summed` | 两轮都带 `cache_read_input_tokens`（10、20）：最终消息的 `cache_read_input_tokens == 30`（D3） |
| T7 | `several_managed_calls_in_one_round_are_all_answered_in_order` | 一轮里有 `web_search` 和 `web_fetch` 两个调用：第 2 轮最后一条 user 消息含两个 tool_result，`tool_use_id` 依次为两个调用的 id |
| T8 | `a_foreign_call_after_a_managed_round_keeps_earlier_notes` | 第 1 轮只调用 web_search，第 2 轮混合 Read 和 web_search：最终消息开头的说明同时包含第 1 轮的搜索说明和「本轮未执行」；只剩 Read 一个 tool_use |
| T9 | `dropping_the_loop_future_stops_further_rounds` | `send` 第 2 轮永远挂起，用 `tokio::time::timeout` 丢弃 `with_tool_loop` 的 future：之后计数不再增加，没有残留任务 |
| T10 | `chat_and_responses_callers_never_enter_the_tool_loop` | 经 `handle` 发 Chat 和 Responses 请求（Responses 带 `{"type":"web_search"}`），上游为假 openai 来源：上游只收到 1 次请求；Responses 转 Chat 后的请求里没有 web_search 工具（锁住 S7 现状，等鼠鼠决策后改断言） |
| T11 | `a_slow_tool_round_is_bounded` | 待鼠鼠决定整体时限后再写：注入慢 `send`，断言超过上限后返回 504 并带已执行的说明。**依赖设计决策，暂不写** |

### 8.2 `gateway/tool_loop.rs`（tests 模块）

| # | 测试名 | 断言 |
|---|---|---|
| T12 | `streaming_replay_keeps_cache_usage_fields` | `message_to_sse` 处理带 `cache_read_input_tokens` / `cache_creation_input_tokens` 的消息：`message_start.message.usage` 保留这两个字段（D3） |

### 8.3 `gateway/responses_chat.rs`（tests 模块）

| # | 测试名 | 断言 |
|---|---|---|
| T13 | `an_upstream_error_chunk_fails_the_stream_instead_of_completing` | 向 `StreamTranslator` 依次喂一个文字块和一个 `{"error":{"message":"x"}}`，再 `finish()`：输出里没有 `response.completed`，有 `response.failed` 或 error 事件（D5） |
| T14 | `builtin_tools_dropped_in_conversion_are_reported` | 待鼠鼠决策 S7 的处理方式（显式拒绝 / 带提示降级）后写；现状由 `a_codex_request_with_history_and_tools_becomes_chat` 锁住 |

### 8.4 `gateway/adapter/workbuddy_retry.rs`（tests 模块）

| # | 测试名 | 断言 |
|---|---|---|
| T15 | `an_error_chunk_in_an_assembled_stream_is_an_error` | `assemble_chat_stream` 的输入里含错误块：结果能被调用方识别为失败（返回 Err，或带 `error` 字段），不能是 `finish_reason == null` 的成功消息（D6） |

### 8.5 `gateway/translate.rs`（tests 模块）

| # | 测试名 | 断言 |
|---|---|---|
| T16 | `interleaved_parallel_tool_arguments_are_kept` | Chat 流：index 0 开块 → index 1 开块 → index 0 的参数分片 → 结束：两个 tool_use 的 input 都完整（D7） |
| T17 | `an_empty_error_tool_result_still_says_it_failed` | `tool_result{is_error:true, content:[]}` 转 Chat 后 `content` 非空且表明失败（D10） |

### 8.6 `gateway/adapter/claude_bridge.rs`（tests 模块，复用 `tool_calls_park_in_the_gateway_and_resume_the_same_client` 的假客户端脚本）

| # | 测试名 | 断言 |
|---|---|---|
| T18 | `two_tool_calls_in_one_segment_resume_after_both_results` | 假客户端一段里发两个 `mcp__jolu__` 调用：调用方第一次收到两个 tool_use；第二次请求带两个 tool_result 后，同一进程拿到两个结果并接着输出 |
| T19 | `a_caller_dropping_the_stream_ends_the_client_process` | 流式请求读到第一个事件后丢弃响应：短时间内 `RUNS` 里不再有这个令牌，假客户端进程已退出 |
| T20 | `a_parked_run_is_swept_after_idle` | 需要把 `PARK_IDLE` 改成可注入（或测试专用值）：挂起后超过闲置时间再调用 `sweep`，令牌被移除，`Scratch` 目录被删（D8） |
| T21 | `server_tools_are_not_offered_to_the_client` | `mcp_tools` 处理 `[{name:"Read",input_schema:{}},{type:"web_search_20250305",name:"web_search"}]`：只返回 Read（锁住 B1 现状，等决策后改） |

---

## 9. 真实验证缺口

- `docs/verification/` **没有任何**模型发起工具调用 → 执行 → 回传 → 最终回答的真实记录（`docs/specs/2026-11-23-capability-matrix.md:78,206` 已列为最大缺口）。
- tool_loop：计划文档注明「实机未验证」（`docs/plans/2026-11-09-complete-remaining-work.md:88`）。
- claude_bridge：只有假客户端端到端测试，「真实账号验收归鼠鼠」（同文件 `:109`）。
- mcp_search：只有协议层单测；「重启后在 `/mcp` 里看到 jolu-search」的验收步骤（`docs/specs/2026-11-10-capability-batches-handoff.md:310`）没有执行记录。
- 建议的真实验证顺序（各需一个明确授权的账号）：① Claude Code → ZCode Coding Plan 的 WebSearch（tool_loop，并顺带确认 D1 的前提和 D4 的 stream:false）；② Claude Code → Claude 订阅调用 Read 工具（claude_bridge 挂起续跑）；③ Codex → Codex 订阅的函数工具和 web_search（原生直通）；④ 任一 Chat Agent 调用 jolu-search MCP。每项都要记录成功、工具失败、取消、混合工具和流式五种行为，以及路由流水行数与用量入账。
