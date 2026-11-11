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

/* ---------------------- 400 之后的修复重试（兜底） ---------------------- */
//
// 预防式剔除（上面的 `strip_foreign_thinking`）只认得空签名与 UUID。另一个 Claude
// 账号签的块（同一会话换了账号）、过期签名等格式上看不出来，只能等 Claude 自己拒：
// 读 400 的错误文案与 `messages.<m>.content.<b>` 位置，把该块及其后的所有 thinking
// 块记成「这个来源拒过」，剔掉重发一次；记忆按来源保留，同一会话后面的轮次首发就
// 不带这些块，不必每轮先挨一次 400。
//
// 参照 magpie `internal/gateway/thinking_bound.go::refuseBoundThinking` 与
// `withoutRefusedThinking`（克隆 `0ee424e`）。差异：magpie 用正则，Jolu 没有 regex
// 依赖，改为去反引号后小写子串匹配，四种文案一一对应。

/// 记忆上限：超过就清空重来——仍在进行的会话最多再挨一次 400 就修好。
const REFUSED_MOST: usize = 1 << 14;

static REFUSED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

/// 400 错误体是不是 Claude 拒绝 thinking 签名。四种文案：签名无效、绑定到别的会话、
/// 不是本会话产生的、redacted_thinking 数据无效。其他 400（参数错误等）不修。
pub(crate) fn is_bound_refusal(error_body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(error_body).replace('`', "").to_ascii_lowercase();
    [
        "invalid signature in thinking block",
        "bound to a different conversation",
        "not created in this conversation",
        "invalid data in redacted_thinking block",
    ]
    .iter()
    .any(|wording| text.contains(wording))
}

/// 错误文案点名的位置 `messages.<m>.content.<b>`。
pub(crate) fn bound_at(error: &str) -> Option<(usize, usize)> {
    let rest = &error[error.find("messages.")? + "messages.".len()..];
    let digits = |text: &str| -> Option<(usize, usize)> {
        let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
        Some((text[..end].parse().ok()?, end))
    };
    let (message, end) = digits(rest)?;
    let rest = rest[end..].strip_prefix(".content.")?;
    let (block, _) = digits(rest)?;
    Some((message, block))
}

fn is_thinking_block(block: &Value) -> bool {
    matches!(block.get("type").and_then(Value::as_str), Some("thinking" | "redacted_thinking"))
}

/// 块的身份：签名（redacted 用 data，都没有用正文）的摘要。只存摘要，不在内存里囤签名原文。
fn block_key(block: &Value) -> String {
    use sha2::Digest;
    let field = |name: &str| block.get(name).and_then(Value::as_str).unwrap_or_default();
    let material = [field("signature"), field("data"), field("thinking")]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    let digest = sha2::Sha256::digest(format!("{} {material}", field("type")).as_bytes());
    digest[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

fn refused(owner: &str, key: &str) -> bool {
    REFUSED.lock().map(|set| set.contains(&format!("{owner} {key}"))).unwrap_or(false)
}

/// 读一次 400：是 thinking 签名拒绝就把点名的块（点名位置不是 thinking 块或没点名时从头）
/// 及其后所有 thinking 块记为 `owner` 拒过。返回是否记下了新块——没有新块说明剔过还拒，
/// 调用方不该再重试。
pub(crate) fn remember_refusal(owner: &str, error_body: &[u8], sent: &Value) -> bool {
    if !is_bound_refusal(error_body) {
        return false;
    }
    let Some(messages) = sent.get("messages").and_then(Value::as_array) else { return false };
    let blocks_of = |index: usize| messages.get(index).and_then(|m| m.get("content")).and_then(Value::as_array);
    let (from_message, from_block) = bound_at(&String::from_utf8_lossy(error_body))
        .filter(|&(m, b)| blocks_of(m).and_then(|blocks| blocks.get(b)).is_some_and(is_thinking_block))
        .unwrap_or((0, 0));
    let mut fresh = Vec::new();
    for (m, message) in messages.iter().enumerate().skip(from_message) {
        let Some(blocks) = message.get("content").and_then(Value::as_array) else { continue };
        for (b, block) in blocks.iter().enumerate() {
            if (m == from_message && b < from_block) || !is_thinking_block(block) {
                continue;
            }
            let key = block_key(block);
            if !refused(owner, &key) {
                fresh.push(format!("{owner} {key}"));
            }
        }
    }
    if fresh.is_empty() {
        return false;
    }
    if let Ok(mut set) = REFUSED.lock() {
        if set.len() + fresh.len() > REFUSED_MOST {
            set.clear();
        }
        set.extend(fresh);
    }
    true
}

/// 剔除 `owner` 拒过的 thinking 块，剔空的消息整条移除。返回剔除数量。
pub(crate) fn strip_refused(owner: &str, body: &mut Value) -> usize {
    if REFUSED.lock().map(|set| set.is_empty()).unwrap_or(true) {
        return 0;
    }
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else { return 0 };
    let mut removed = 0;
    for index in (0..messages.len()).rev() {
        let Some(content) = messages[index].get_mut("content").and_then(Value::as_array_mut) else { continue };
        let before = content.len();
        content.retain(|block| !(is_thinking_block(block) && refused(owner, &block_key(block))));
        removed += before - content.len();
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

    fn refused_body() -> Value {
        body_with(json!([
            json!({"role": "user", "content": "你好"}),
            assistant_message(json!([
                {"type": "thinking", "thinking": "别家签的", "signature": "sig-from-another-account-xxxxxxxx"},
                {"type": "text", "text": "回答一"}
            ])),
            json!({"role": "user", "content": "继续"}),
            assistant_message(json!([
                {"type": "redacted_thinking", "data": "opaque"},
                {"type": "text", "text": "回答二"}
            ])),
            json!({"role": "user", "content": "再来"}),
        ]))
    }

    #[test]
    fn bound_refusal_matches_claude_wording_with_or_without_backticks() {
        assert!(is_bound_refusal(br#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid `signature` in `thinking` block"}}"#));
        assert!(is_bound_refusal(b"invalid signature in thinking block"));
        assert!(is_bound_refusal(b"This thinking block is bound to a different conversation"));
        assert!(is_bound_refusal(b"block was not created in this conversation"));
        assert!(is_bound_refusal(b"Invalid data in `redacted_thinking` block"));
        assert!(!is_bound_refusal(br#"{"error":{"message":"max_tokens: must be positive"}}"#), "别的 400 不修");
        assert_eq!(bound_at("messages.3.content.0: Invalid `signature`"), Some((3, 0)));
        assert_eq!(bound_at("no position"), None);
    }

    #[test]
    fn refusal_removes_the_named_block_and_every_later_thinking_block_then_sticks() {
        let owner = "test-owner-refusal-sticks";
        let sent = refused_body();
        let error = br#"{"error":{"message":"messages.1.content.0: Invalid `signature` in `thinking` block"}}"#;
        assert!(remember_refusal(owner, error, &sent), "第一次拒绝记下新块");
        assert!(!remember_refusal(owner, error, &sent), "同样的块再拒：没有新块，不再重试（防死循环）");
        let mut retry = sent.clone();
        assert_eq!(strip_refused(owner, &mut retry), 2, "被点名的块与其后的 redacted_thinking 一起剔除");
        let messages = retry["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5, "两条 assistant 都还有 text，不整条移除");
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[3]["content"][0]["type"], "text");
        // 记住之后，同一来源后续轮次首发就不带这些块；别的来源原样
        let mut other = sent.clone();
        assert_eq!(strip_refused("another-owner", &mut other), 0);
    }

    #[test]
    fn refusal_naming_a_non_thinking_block_falls_back_to_the_start() {
        let owner = "test-owner-refusal-start";
        let sent = refused_body();
        assert!(remember_refusal(owner, b"messages.0.content.0: invalid signature in thinking block", &sent));
        let mut retry = sent.clone();
        assert_eq!(strip_refused(owner, &mut retry), 2, "点名位置不是 thinking 块时从头算");
    }

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
