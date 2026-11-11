//! WorkBuddy 拒绝「未授权渠道」时的改写重发，以及非流式请求的回复拼装（3.6）。
//!
//! 为什么：WorkBuddy（腾讯 CodeBuddy 系）按整行匹配已知 agent 的开场白（如「You are … agent powered by …」），
//! 命中就回 400「unapproved channel / illegal api invocation」。同样的话多一个逗号就能过；还不行时，
//! 把系统提示挪进第一条用户消息（`<instructions>` 包着），系统位换成 WorkBuddy 自己的默认句子，
//! 因为它要求第一条必须是系统消息（11128）。哪种改法过了就按系统提示记住，同一 agent 下一次直接用。
//!
//! 参照 magpie 社区插件 `packages/workbuddy` 的 `commaPaused`、`systemAsUser`、`REFUSED`。
//! 差异：插件还维护一张已知拒绝行表（FLAGGED）在首发前就改写，Jolu 只在被拒后改写并记住。

use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// WorkBuddy 自己的默认系统提示。
pub(super) const DEFAULT_SYSTEM: &str = "You are a helpful assistant.";

/// 被拒后改成「系统提示挪进用户消息」才过的系统提示（按摘要记，最多 256 条）。
static MOVED: LazyLock<Mutex<HashSet<u64>>> = LazyLock::new(Default::default);
/// 加了逗号就过的系统提示。
static COMMA: LazyLock<Mutex<HashSet<u64>>> = LazyLock::new(Default::default);

fn remember(set: &Mutex<HashSet<u64>>, key: u64) {
    if let Ok(mut set) = set.lock() {
        if set.len() >= 256 {
            set.clear();
        }
        set.insert(key);
    }
}

fn known(set: &Mutex<HashSet<u64>>, key: u64) -> bool {
    set.lock().map(|set| set.contains(&key)).unwrap_or(false)
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// 系统消息的文字摘要；只有 WorkBuddy 默认句子时为 None（没有要挪的）。
pub(super) fn system_key(body: &Value) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let text: Vec<String> = body.get("messages")?.as_array()?.iter().filter(|m| m["role"] == "system").map(|m| text_of(&m["content"])).collect();
    let joined = text.join("\n");
    if joined.trim().is_empty() || joined.trim() == DEFAULT_SYSTEM {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    joined.hash(&mut hasher);
    Some(hasher.finish())
}

/// 400 的错误文字是不是「未授权渠道」类拒绝。
pub(super) fn refused(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("unapproved channel") || lower.contains("illegal api invocation")
}

const OPENER_LINKS: [&str; 5] = ["powered by", "running on", "running in", "built on", "based on"];

/// 开场白「You are a/an/the …agent|assistant powered by …」在 agent 与连接词之间加一个逗号；没有可加的返回 None。
pub(super) fn opened(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 4);
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find("You are ") {
        out.push_str(&rest[..at]);
        let candidate = &rest[at..];
        let after = &candidate["You are ".len()..];
        let article_ok = ["a ", "an ", "the "].iter().any(|a| after.starts_with(a));
        // 名词短语：60 个字符以内、不跨换行 / 句号 / 逗号，以 agent 或 assistant 结尾
        let span: String = after.chars().take_while(|c| *c != '\n' && *c != '.' && *c != ',').take(80).collect();
        let hit = article_ok.then(|| {
            ["agent", "assistant"].iter().find_map(|noun| {
                let mut from = 0;
                while let Some(i) = span[from..].find(noun) {
                    let end = from + i + noun.len();
                    let tail = span[end..].trim_start();
                    if end <= 64 + noun.len() && span[end..].starts_with(char::is_whitespace) && OPENER_LINKS.iter().any(|link| tail.to_ascii_lowercase().starts_with(link)) {
                        return Some(end);
                    }
                    from = end;
                }
                None
            })
        });
        match hit.flatten() {
            Some(end) => {
                let prefix_len = "You are ".len() + end;
                out.push_str(&candidate[..prefix_len]);
                out.push(',');
                rest = &candidate[prefix_len..];
                changed = true;
            }
            None => {
                out.push_str("You are ");
                rest = &candidate["You are ".len()..];
            }
        }
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// 在系统与助手消息里给开场白加逗号（用户消息与工具结果不动——那是用户自己的话）。返回是否改了系统提示。
pub(super) fn comma_paused(body: &mut Value) -> (bool, bool) {
    let (mut any, mut system) = (false, false);
    for message in body.get_mut("messages").and_then(Value::as_array_mut).into_iter().flatten() {
        let role = message["role"].as_str().unwrap_or_default().to_string();
        if role != "system" && role != "assistant" {
            continue;
        }
        if let Some(text) = message["content"].as_str().and_then(opened) {
            message["content"] = Value::String(text);
            any = true;
            system |= role == "system";
        }
    }
    (any, system)
}

/// 系统提示挪进第一条用户消息（`<instructions>` 包着），系统位换成 WorkBuddy 默认句子；
/// 后面出现的系统消息就地改成用户消息。没有可挪的返回 false。
pub(super) fn system_as_user(body: &mut Value) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else { return false };
    let lead: Vec<String> = messages.iter().take_while(|m| m["role"] == "system").map(|m| text_of(&m["content"])).collect();
    let said = lead.join("\n\n").trim().to_string();
    let mut rest: Vec<Value> = messages.drain(lead.len()..).collect();
    let later_system = rest.iter().any(|m| m["role"] == "system");
    let moved = !said.is_empty() && said != DEFAULT_SYSTEM;
    if !moved && !later_system {
        // 没有可挪的：放回原样
        let mut original: Vec<Value> = lead.iter().map(|text| json!({"role": "system", "content": text})).collect();
        original.append(&mut rest);
        *messages = original;
        return false;
    }
    for message in rest.iter_mut().filter(|m| m["role"] == "system") {
        message["role"] = json!("user");
    }
    if moved {
        let wrapped = format!("<instructions>\n{said}\n</instructions>");
        match rest.iter().position(|m| m["role"] == "user") {
            Some(i) => {
                let content = rest[i]["content"].clone();
                rest[i]["content"] = match content {
                    Value::String(text) => Value::String(format!("{wrapped}\n\n{text}")),
                    Value::Array(mut parts) => {
                        parts.insert(0, json!({"type": "text", "text": wrapped}));
                        Value::Array(parts)
                    }
                    _ => Value::String(wrapped),
                };
            }
            None => rest.insert(0, json!({"role": "user", "content": wrapped})),
        }
    }
    let mut next = vec![json!({"role": "system", "content": DEFAULT_SYSTEM})];
    next.extend(rest);
    *messages = next;
    true
}

/// 首发前按记忆改写：这个系统提示以前要挪过 / 要加逗号过，就直接改好再发。
pub(super) fn prepare(body: &mut Value) {
    let Some(key) = system_key(body) else { return };
    if known(&MOVED, key) {
        system_as_user(body);
    } else if known(&COMMA, key) {
        comma_paused(body);
    }
}

/// 被拒后的下一种改法：先加逗号（改了系统提示才记），还被拒再挪系统提示。返回改写后的请求体，没有可试的返回 None。
pub(super) fn next_attempt(original_key: Option<u64>, body: &Value, tried_comma: bool) -> Option<(Value, &'static str)> {
    let mut next = body.clone();
    if !tried_comma {
        let (any, _) = comma_paused(&mut next);
        if any {
            return Some((next, "comma"));
        }
    }
    let key = original_key?;
    if known(&MOVED, key) {
        return None;
    }
    system_as_user(&mut next).then_some((next, "moved"))
}

/// 改法成功后记住（同一系统提示下次首发就用）。
pub(super) fn succeeded(original_key: Option<u64>, how: &str, system_changed: bool) {
    let Some(key) = original_key else { return };
    match how {
        "moved" => remember(&MOVED, key),
        "comma" if system_changed => remember(&COMMA, key),
        _ => {}
    }
}

/// 客户端要非流式、上游却回了事件流时：把 Chat 的流拼成一条完整回复（文字、推理、工具调用按序号合并、用量、结束原因）。
pub(super) fn assemble_chat_stream(text: &str) -> Value {
    let mut id = String::new();
    let mut model = String::new();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<Value> = Vec::new();
    let mut finish = Value::Null;
    let mut usage = Value::Null;
    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
        let Ok(chunk) = serde_json::from_str::<Value>(data) else { continue };
        if id.is_empty() {
            id = chunk["id"].as_str().unwrap_or_default().to_string();
            model = chunk["model"].as_str().unwrap_or_default().to_string();
        }
        if chunk["usage"].is_object() {
            usage = chunk["usage"].clone();
        }
        let choice = &chunk["choices"][0];
        if !choice["finish_reason"].is_null() {
            finish = choice["finish_reason"].clone();
        }
        let delta = &choice["delta"];
        content.push_str(delta["content"].as_str().unwrap_or_default());
        reasoning.push_str(delta["reasoning_content"].as_str().unwrap_or_default());
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            while calls.len() <= index {
                calls.push(json!({"id": "", "type": "function", "function": {"name": "", "arguments": ""}}));
            }
            let slot = &mut calls[index];
            if let Some(v) = call["id"].as_str().filter(|v| !v.is_empty()) {
                slot["id"] = json!(v);
            }
            if let Some(v) = call.pointer("/function/name").and_then(Value::as_str).filter(|v| !v.is_empty()) {
                slot["function"]["name"] = json!(v);
            }
            if let Some(v) = call.pointer("/function/arguments").and_then(Value::as_str) {
                let joined = format!("{}{v}", slot["function"]["arguments"].as_str().unwrap_or_default());
                slot["function"]["arguments"] = json!(joined);
            }
        }
    }
    let mut message = json!({"role": "assistant", "content": if content.is_empty() && !calls.is_empty() { Value::Null } else { json!(content) }});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = Value::Array(calls);
    }
    let mut out = json!({"id": id, "object": "chat.completion", "model": model, "choices": [{"index": 0, "message": message, "finish_reason": finish}]});
    if !usage.is_null() {
        out["usage"] = usage;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_opener_gets_a_comma_and_nothing_else_changes() {
        assert_eq!(opened("You are an AI coding agent powered by DeepSeek.").as_deref(), Some("You are an AI coding agent, powered by DeepSeek."));
        assert_eq!(opened("Hi. You are the assistant running in Trae and more").as_deref(), Some("Hi. You are the assistant, running in Trae and more"));
        assert!(opened("You are Claude Code, Anthropic's official CLI").is_none(), "没有 a/an/the 冠词的不动");
        assert!(opened("You are a helpful assistant.").is_none(), "后面没有连接词的不动");
    }

    #[test]
    fn system_moves_into_the_first_user_message_under_the_default_system() {
        let mut body = json!({"messages": [
            {"role": "system", "content": "你是编程助手"},
            {"role": "user", "content": "你好"},
            {"role": "system", "content": "后来的提醒"}
        ]});
        assert!(system_as_user(&mut body));
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m[0], json!({"role": "system", "content": DEFAULT_SYSTEM}));
        assert_eq!(m[1]["content"], "<instructions>\n你是编程助手\n</instructions>\n\n你好");
        assert_eq!(m[2]["role"], "user", "后来的系统消息就地改成用户消息");
        let mut plain = json!({"messages": [{"role": "system", "content": DEFAULT_SYSTEM}, {"role": "user", "content": "x"}]});
        assert!(!system_as_user(&mut plain), "只有默认句子没有可挪的");
        assert_eq!(plain["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn refusals_are_recognized_and_the_next_attempt_goes_comma_then_moved() {
        assert!(refused(r#"{"error":{"message":"Unapproved Channel"}}"#));
        assert!(refused("illegal API invocation"));
        assert!(!refused("rate limited"));
        let body = json!({"messages": [{"role": "system", "content": "You are a coding agent built on GLM."}, {"role": "user", "content": "hi"}]});
        let key = system_key(&body);
        let (first, how) = next_attempt(key, &body, false).unwrap();
        assert_eq!(how, "comma");
        assert!(first["messages"][0]["content"].as_str().unwrap().contains("agent, built on"));
        let (second, how) = next_attempt(key, &first, true).unwrap();
        assert_eq!(how, "moved");
        assert_eq!(second["messages"][0]["content"], DEFAULT_SYSTEM);
        succeeded(key, "moved", false);
        let mut fresh = body.clone();
        prepare(&mut fresh);
        assert_eq!(fresh["messages"][0]["content"], DEFAULT_SYSTEM, "记住后首发就挪");
    }

    #[test]
    fn a_streamed_reply_is_assembled_for_a_non_streaming_client() {
        let sse = "data: {\"id\":\"c1\",\"model\":\"glm\",\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"t1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"p\"}}]}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\":1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n\
data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n";
        let out = assemble_chat_stream(sse);
        assert_eq!(out["choices"][0]["message"]["content"], "你");
        assert_eq!(out["choices"][0]["message"]["reasoning_content"], "想");
        assert_eq!(out["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"], "{\"p\":1}");
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(out["usage"]["completion_tokens"], 2);
    }
}
