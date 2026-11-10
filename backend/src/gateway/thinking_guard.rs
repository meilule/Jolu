//! 剔除发往 Claude 的请求里那些不可能是 Claude 签名的 thinking 块。
//!
//! 管什么：跨模型分组切换后，上游模型（Grok / DeepSeek / GLM 等）产出的 thinking
//! 块被 Claude Code 保留下来、随下一轮请求带给真 Claude 的情况；以及 Jolu 自己
//! 在 [`crate::gateway::translate`] 两处（Anthropic 响应重建与流式 delta）生成的
//! `signature: ""` 块——两条来源都要拦。
//!
//! 不管什么：
//! - 非 Claude 上游一律原样发（DeepSeek 的 Anthropic 端点要回自己的 thinking
//!   且接受空签名；GLM 同理）——所以过滤只在上游「真的服务 Claude」时进行；
//! - `redacted_thinking` 不删：它没有明文但可能带有效签名；
//! - Claude 自己的签名（base64，实测 800+ 字符）原样保留。
//!
//! 关键取舍：按**签名格式**判断来源，不按模型名。Claude 的签名是长 base64，
//! DeepSeek 的是 UUID，空串一定不是 Claude 的；按模型名判断会漏掉
//! Bedrock / Vertex 的 id 形态（`us.anthropic.claude-*`、`claude-3-5-sonnet@vertex`）。
//! 模型名只用来补一个方向：非 anthropic.com 的 host 上，`claude-*` 模型
//! （经中转的 Claude）同样按 Claude 对待。

use serde_json::Value;

/// DeepSeek 的 Anthropic 端点给 thinking 的签名：回复的 id，一个 UUID。
/// 参照 magpie `thinking_bound.go::deepseekSignature`。
fn is_uuid_signature(signature: &str) -> bool {
    let bytes = signature.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

/// 这次转发是否「上游真的服务 Claude」——只有那时才需要担心外来的签名。
///
/// 规则与 magpie 一致：host 是 `api.anthropic.com`，或（host 不是它时）模型名
/// 是 `claude-*`（含 Bedrock 的 `us.anthropic.claude-*` 与 Vertex 的
/// `claude-3-5-sonnet@vertex` 形态）。zcode 的 GLM、DeepSeek 的兼容端点都不是。
pub(crate) fn serves_claude(base_url: Option<&str>, model: &str) -> bool {
    let is_anthropic_host = base_url
        .map(|base| {
            // 取 host 比较，避免路径、端口写法差异造成漏判
            url::Url::parse(base)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .is_some_and(|host| host.eq_ignore_ascii_case("api.anthropic.com"))
        })
        .unwrap_or(false);
    if is_anthropic_host {
        return true;
    }
    // 模型名判定：`claude-` 开头，或 Bedrock/Vertex 形态里含 `.claude-` / `claude-` 段。
    // 参照 magpie 的 anthropicModel 正则的意图，这里用更直白的前后缀判断。
    let lowered = model.to_ascii_lowercase();
    lowered.starts_with("claude-")
        || lowered.contains(".claude-")
        || lowered.contains("@claude-")
        || lowered.contains(":claude-")
}

/// 剔除发往 Claude 的请求体里不可能是 Claude 签名的 thinking 块，返回剔除数量。
///
/// 为什么：分组从 Grok 4.7 切到 Kiro 上的 Claude Opus 5.5 后，Claude Code 保留了
/// Grok 产的 thinking 块，Kiro 连续 13 轮回裸 `invalid_request_error`；同一请求
/// 去掉那个块就能过。Jolu 自己也在 `translate.rs:264` 与 `:355` 两处生成
/// `signature: ""` 的块，所以两条来源都要拦。
///
/// 只动 `messages` 里 assistant 消息的 `content` 数组：`type == "thinking"` 且
/// 签名为空串或 UUID 的块移除；`text` / `tool_use` / 后续的 `tool_result` 全部保留；
/// 移除后 content 变空的消息整条移除（Claude 不收空 content）。
///
/// 参照 magpie `internal/gateway/thinking_bound.go::withoutForeignThinking`
/// （commit `a6a9971`）。
/// 差异：magpie 还处理 `refuseBoundThinking` 的签名修复与 DeepSeek 的
/// 「要回自己 thinking」例外——那由 [`serves_claude`] 的调用方判定承担。
pub(crate) fn strip_foreign_thinking(body: &mut Value) -> usize {
    // 快速短路：没有签名字段的请求体原样返回，不付出解析遍历的代价
    let Ok(text) = serde_json::to_string(&*body) else { return 0 };
    if !text.contains("\"signature\"") {
        return 0;
    }
    let mut removed = 0;
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return 0;
    };
    // 倒序遍历以便整条移除空消息
    for index in (0..messages.len()).rev() {
        let Some(message) = messages.get_mut(index) else { continue };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = content.len();
        content.retain(|block| {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                return true;
            }
            let signature = block.get("signature").and_then(Value::as_str).unwrap_or_default();
            let foreign = signature.is_empty() || is_uuid_signature(signature);
            if foreign { removed += 1; }
            !foreign
        });
        if content.is_empty() && before > 0 {
            messages.remove(index);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assistant_message(blocks: Value) -> Value {
        json!({"role": "assistant", "content": blocks})
    }

    fn body_with(messages: Value) -> Value {
        json!({"model": "claude-sonnet-5", "messages": messages, "max_tokens": 16})
    }

    #[test]
    fn empty_and_uuid_signatures_are_removed_long_base64_is_kept() {
        let long_base64 = "x".repeat(800);
        let mut body = body_with(json!([
            assistant_message(json!([
                {"type": "thinking", "thinking": "外来的", "signature": ""},
                {"type": "text", "text": "回答"}
            ])),
            assistant_message(json!([
                {"type": "thinking", "thinking": "DeepSeek 的", "signature": "0b7f1e2a-93c4-4d58-8f21-aabbccddeeff"},
                {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {}}
            ])),
            json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "内容"}]}),
            assistant_message(json!([
                {"type": "thinking", "thinking": "Claude 自己的", "signature": long_base64},
                {"type": "text", "text": "最终"}
            ]))
        ]));
        let removed = strip_foreign_thinking(&mut body);
        assert_eq!(removed, 2, "空签名与 UUID 各剔一个");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "第一条还有 text，不整条移除");
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(messages[0]["content"][0]["type"], "text", "thinking 块剔除后 text 保留");
        assert_eq!(messages[1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(messages[1]["content"][0]["type"], "tool_use", "tool_use 保留");
        assert_eq!(messages[3]["content"].as_array().unwrap().len(), 2, "Claude 自己的长签名块保留");
    }

    #[test]
    fn message_left_empty_is_removed_whole() {
        let mut body = body_with(json!([
            assistant_message(json!([{"type": "thinking", "thinking": "只有思考", "signature": ""}])),
            json!({"role": "user", "content": [{"type": "text", "text": "继续"}]})
        ]));
        let removed = strip_foreign_thinking(&mut body);
        assert_eq!(removed, 1);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "剔空后 assistant 整条移除");
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn bodies_without_signature_are_untouched_without_parse_cost() {
        // 有 messages 但无签名字段：结果不变
        let mut body = body_with(json!([assistant_message(json!([{"type": "text", "text": "无签名"}]))]));
        assert_eq!(strip_foreign_thinking(&mut body), 0);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        // 非 JSON（这里用空对象模拟）也不炸
        let mut empty = json!({});
        assert_eq!(strip_foreign_thinking(&mut empty), 0);
    }

    #[test]
    fn non_assistant_thinking_blocks_are_kept() {
        // thinking 出现在非 assistant 消息里（理论上不该有）不动它——只管 assistant
        let mut body = body_with(json!([
            json!({"role": "user", "content": [{"type": "thinking", "thinking": "怪", "signature": ""}]})
        ]));
        assert_eq!(strip_foreign_thinking(&mut body), 0);
        assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn serves_claude_matches_host_and_model_shapes() {
        assert!(serves_claude(Some("https://api.anthropic.com"), "claude-sonnet-5"));
        assert!(serves_claude(Some("https://api.anthropic.com/v1"), "anything"), "host 判定优先于模型名");
        // 非 anthropic host 上的 claude 模型（中转）也算
        assert!(serves_claude(Some("https://relay.example.com"), "claude-sonnet-5"));
        assert!(serves_claude(Some("https://relay.example.com"), "us.anthropic.claude-sonnet-5-v2:0"), "Bedrock 形态");
        assert!(serves_claude(Some("https://relay.example.com"), "claude-3-5-sonnet@20240307"), "Vertex 形态");
        // 类 Claude 但不是 Claude：不过滤
        assert!(!serves_claude(Some("https://open.bigmodel.cn/api/anthropic"), "GLM-5.3"));
        assert!(!serves_claude(Some("https://api.deepseek.com/anthropic"), "deepseek-v4"));
        assert!(!serves_claude(None, "glm-5.3"));
    }

    #[test]
    fn uuid_signature_checker_is_strict() {
        assert!(is_uuid_signature("0b7f1e2a-93c4-4d58-8f21-aabbccddeeff"));
        assert!(is_uuid_signature("0B7F1E2A-93C4-4D58-8F21-AABBCCDDEEFF"), "大小写都算");
        assert!(!is_uuid_signature(""), "空串由调用方单独判");
        assert!(!is_uuid_signature("0b7f1e2a93c44d588f21aabbccddeeff"), "无连字符不算");
        assert!(!is_uuid_signature("0b7f1e2a-93c4-4d58-8f21-aabbccddxeff"), "非十六进制不算");
        assert!(!is_uuid_signature("x"), "长度不够不算");
    }
}
