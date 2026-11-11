//! 替身工具：上游不是真 Claude 时，由网关执行 Claude Code 的联网搜索 / 读网页服务端工具。
//!
//! 为什么：Claude Code 的 WebSearch / WebFetch 是 Anthropic 的**服务端工具**
//! （`web_search_2025…` / `web_fetch_2025…`），由 Anthropic 自己执行。请求经 Jolu 路由到
//! GLM、DeepSeek 等 Anthropic 兼容端点或 WorkBuddy 等订阅时，上游不会执行它们——要么报错，
//! 要么静默忽略，模型就「搜不了」。
//!
//! 管什么：请求声明了这两种服务端工具、且上游不是真 Claude 时——
//! 1. 把服务端工具换成同名的普通函数工具，模型照常「调用」它；
//! 2. 模型只调用了这两个工具时，网关用 Jolu 的搜索 / 读网页执行，把结果作为 `tool_result`
//!    续上，再问一轮（最多 [`MAX_ROUNDS`] 轮，防死循环）；
//! 3. 最后一轮的回复加一行说明后交给客户端；用量取各轮之和；客户端要流式时把整条消息
//!    重写成 SSE（为了能截住工具调用，每一轮都按非流式问上游）。
//!
//! 不管什么：
//! - 真 Claude 上游：服务端工具由 Anthropic 执行，原样转发；
//! - 模型同时调用了 agent 自己的工具：网关的那几个调用从回复里去掉，其余原样交给 agent
//!   （执行了也没法把结果和 agent 的工具结果合在一轮里交回去）；
//! - 首字延迟：多轮期间客户端收不到任何字节，最后一次性收到。
//!
//! 参照 magpie 的网关工具循环（`internal/gateway` 的 tool-loop，`3245b73` 起在会话语言里写说明）；
//! 差异：magpie 边转发边插入进度事件，Jolu 先做完再整体回放。

use serde_json::{json, Value};

/// 最多问上游几轮。
pub(crate) const MAX_ROUNDS: usize = 5;

/// 请求里网关要接管的工具名（服务端工具的 `name`，通常是 `web_search` / `web_fetch`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Managed {
    pub search: Option<String>,
    pub fetch: Option<String>,
}

impl Managed {
    pub fn is_empty(&self) -> bool {
        self.search.is_none() && self.fetch.is_none()
    }
    fn kind_of(&self, name: &str) -> Option<&'static str> {
        if self.search.as_deref() == Some(name) {
            Some("search")
        } else if self.fetch.as_deref() == Some(name) {
            Some("fetch")
        } else {
            None
        }
    }
}

/// 找出请求里的联网服务端工具。agent 自己有同名普通工具时那一个不接管（名字被占用）。
pub(crate) fn managed_tools(body: &Value) -> Managed {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else { return Managed::default() };
    let plain_names: Vec<&str> = tools
        .iter()
        .filter(|tool| tool.get("type").and_then(Value::as_str).is_none_or(|t| t == "custom"))
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();
    let mut managed = Managed::default();
    for tool in tools {
        let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
        let name = tool.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        if name.is_empty() || plain_names.contains(&name.as_str()) {
            continue;
        }
        if kind.starts_with("web_search_") {
            managed.search = Some(name);
        } else if kind.starts_with("web_fetch_") {
            managed.fetch = Some(name);
        }
    }
    managed
}

/// 改写请求：服务端工具换成普通函数工具，强制非流式。返回客户端原本是否要流式。
pub(crate) fn rewrite_request(body: &mut Value, managed: &Managed) -> bool {
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools.iter_mut() {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let is_server = tool.get("type").and_then(Value::as_str).is_some_and(|t| t.starts_with("web_search_") || t.starts_with("web_fetch_"));
            if !is_server {
                continue;
            }
            *tool = match managed.kind_of(&name) {
                Some("search") => json!({
                    "name": name,
                    "description": "联网搜索，返回网页标题、链接与摘要。需要最新信息或不确定的事实时使用。",
                    "input_schema": {"type": "object", "properties": {"query": {"type": "string", "description": "搜索关键词"}}, "required": ["query"]},
                }),
                Some(_) => json!({
                    "name": name,
                    "description": "读取一个公网网页的正文（纯文本）。",
                    "input_schema": {"type": "object", "properties": {"url": {"type": "string", "description": "网页地址"}}, "required": ["url"]},
                }),
                None => continue,
            };
        }
    }
    body["stream"] = Value::Bool(false);
    streaming
}

/// 一个要网关执行的调用。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Call {
    pub id: String,
    pub kind: &'static str,
    /// 搜索词或网址
    pub argument: String,
}

/// 读一轮回复：网关要执行的调用，以及有没有 agent 自己的工具调用。
pub(crate) fn calls_in(message: &Value, managed: &Managed) -> (Vec<Call>, bool) {
    let mut calls = Vec::new();
    let mut foreign = false;
    for block in message.get("content").and_then(Value::as_array).into_iter().flatten() {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
        match managed.kind_of(name) {
            Some(kind) => {
                let field = if kind == "search" { "query" } else { "url" };
                calls.push(Call {
                    id: block.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                    kind,
                    argument: block.pointer(&format!("/input/{field}")).and_then(Value::as_str).unwrap_or_default().to_string(),
                });
            }
            None => foreign = true,
        }
    }
    (calls, foreign)
}

/// 把这一轮的回复与执行结果续进请求：assistant 原样，user 带 tool_result。
pub(crate) fn append_round(body: &mut Value, message: &Value, results: &[(String, String, bool)]) {
    let content = message.get("content").cloned().unwrap_or_else(|| json!([]));
    let tool_results: Vec<Value> = results
        .iter()
        .map(|(id, text, is_error)| json!({"type": "tool_result", "tool_use_id": id, "content": text, "is_error": is_error}))
        .collect();
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        messages.push(json!({"role": "assistant", "content": content}));
        messages.push(json!({"role": "user", "content": tool_results}));
    }
}

/// 去掉回复里网关接管的工具调用（与 agent 自己的工具混在一轮时用）。
pub(crate) fn strip_managed(message: &mut Value, managed: &Managed) {
    if let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) {
        content.retain(|block| {
            block.get("type").and_then(Value::as_str) != Some("tool_use")
                || managed.kind_of(block.get("name").and_then(Value::as_str).unwrap_or_default()).is_none()
        });
    }
}

/// 各轮用量之和写进最终消息（预算与统计按它记账）。
pub(crate) fn add_usage(total: &mut (i64, i64), message: &Value) {
    total.0 += message.pointer("/usage/input_tokens").and_then(Value::as_i64).unwrap_or(0);
    total.1 += message.pointer("/usage/output_tokens").and_then(Value::as_i64).unwrap_or(0);
}

/// 最终消息：开头加一行说明，用量换成各轮之和；轮次用尽时把仍在调用的接管工具去掉并说明。
pub(crate) fn finish(mut message: Value, notes: &[String], usage: (i64, i64), managed: &Managed, exhausted: bool) -> Value {
    if exhausted {
        strip_managed(&mut message, managed);
        if message.get("stop_reason").and_then(Value::as_str) == Some("tool_use")
            && !message.get("content").and_then(Value::as_array).is_some_and(|c| c.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use")))
        {
            message["stop_reason"] = json!("end_turn");
        }
    }
    let mut lines: Vec<String> = notes.to_vec();
    if exhausted {
        lines.push(format!("联网工具已连续调用 {MAX_ROUNDS} 轮，停止继续调用"));
    }
    if !lines.is_empty() {
        let notice = json!({"type": "text", "text": format!("（Jolu 代为执行：{}）\n\n", lines.join("；"))});
        if let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) {
            content.insert(0, notice);
        }
    }
    if let Some(object) = message.get_mut("usage").and_then(Value::as_object_mut) {
        object.insert("input_tokens".into(), json!(usage.0));
        object.insert("output_tokens".into(), json!(usage.1));
    } else {
        message["usage"] = json!({"input_tokens": usage.0, "output_tokens": usage.1});
    }
    message
}

/// 把一条完整的 Anthropic 消息写成 Messages 流式事件（客户端要流式时用）。
pub(crate) fn message_to_sse(message: &Value) -> String {
    let mut out = String::new();
    let mut event = |name: &str, data: Value| {
        out.push_str(&format!("event: {name}\ndata: {data}\n\n"));
    };
    let mut start = message.clone();
    start["content"] = json!([]);
    start["stop_reason"] = Value::Null;
    start["stop_sequence"] = Value::Null;
    start["usage"] = json!({"input_tokens": message.pointer("/usage/input_tokens").cloned().unwrap_or(json!(0)), "output_tokens": 0});
    event("message_start", json!({"type": "message_start", "message": start}));
    for (index, block) in message.get("content").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "text" => {
                event("content_block_start", json!({"type": "content_block_start", "index": index, "content_block": {"type": "text", "text": ""}}));
                event("content_block_delta", json!({"type": "content_block_delta", "index": index, "delta": {"type": "text_delta", "text": block.get("text").cloned().unwrap_or(json!(""))}}));
            }
            "thinking" => {
                event("content_block_start", json!({"type": "content_block_start", "index": index, "content_block": {"type": "thinking", "thinking": ""}}));
                event("content_block_delta", json!({"type": "content_block_delta", "index": index, "delta": {"type": "thinking_delta", "thinking": block.get("thinking").cloned().unwrap_or(json!(""))}}));
                if let Some(signature) = block.get("signature").filter(|s| s.as_str().is_some_and(|s| !s.is_empty())) {
                    event("content_block_delta", json!({"type": "content_block_delta", "index": index, "delta": {"type": "signature_delta", "signature": signature}}));
                }
            }
            "tool_use" => {
                let mut head = block.clone();
                head["input"] = json!({});
                event("content_block_start", json!({"type": "content_block_start", "index": index, "content_block": head}));
                let input = block.get("input").cloned().unwrap_or(json!({}));
                event("content_block_delta", json!({"type": "content_block_delta", "index": index, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}));
            }
            _ => {
                event("content_block_start", json!({"type": "content_block_start", "index": index, "content_block": block}));
            }
        }
        event("content_block_stop", json!({"type": "content_block_stop", "index": index}));
    }
    event("message_delta", json!({
        "type": "message_delta",
        "delta": {"stop_reason": message.get("stop_reason").cloned().unwrap_or(Value::Null), "stop_sequence": Value::Null},
        "usage": {"output_tokens": message.pointer("/usage/output_tokens").cloned().unwrap_or(json!(0))},
    }));
    event("message_stop", json!({"type": "message_stop"}));
    out
}

/// 执行一个调用，返回（给模型的文本，是否出错，给用户的一句说明）。
pub(crate) async fn execute(call: &Call) -> (String, bool, String) {
    match call.kind {
        "search" => {
            let configs = crate::service::settings::load().map(|s| s.search_apis).unwrap_or_default();
            if configs.iter().all(|api| api.off) {
                return ("搜索不可用：用户还没有在 Jolu 里配置搜索后端。".into(), true, format!("搜索「{}」失败（未配置搜索后端）", call.argument));
            }
            match crate::gateway::search::search_in_order(&configs, &call.argument, 6).await {
                Ok(hits) => {
                    let mut text = String::new();
                    for (index, hit) in hits.iter().enumerate() {
                        text.push_str(&format!("{}. {}\n{}\n{}\n\n", index + 1, hit.title, hit.url, hit.snippet));
                    }
                    if text.is_empty() {
                        text = "没有搜索结果。".into();
                    }
                    (text, false, format!("搜索「{}」，{} 条结果", call.argument, hits.len()))
                }
                Err(reason) => (reason, true, format!("搜索「{}」失败", call.argument)),
            }
        }
        _ => match crate::gateway::search::fetch::fetch(&call.argument, 20_000).await {
            Ok(page) => {
                let mut text = format!("{}\n{}\n\n{}", page.title, page.url, page.text);
                if page.truncated {
                    text.push_str("\n\n（正文过长，已截断）");
                }
                (text, false, format!("读取 {}", page.url))
            }
            Err(reason) => (reason, true, format!("读取 {} 失败", call.argument)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_code_body() -> Value {
        json!({
            "model": "zcode/glm-5.3",
            "stream": true,
            "messages": [{"role": "user", "content": "搜一下 Rust 1.90"}],
            "tools": [
                {"name": "Read", "description": "读文件", "input_schema": {"type": "object"}},
                {"type": "web_search_20250305", "name": "web_search", "max_uses": 8},
                {"type": "web_fetch_20250910", "name": "web_fetch"}
            ]
        })
    }

    #[test]
    fn server_tools_become_plain_tools_and_streaming_is_turned_off() {
        let mut body = claude_code_body();
        let managed = managed_tools(&body);
        assert_eq!(managed, Managed { search: Some("web_search".into()), fetch: Some("web_fetch".into()) });
        assert!(rewrite_request(&mut body, &managed), "记住客户端原本要流式");
        assert_eq!(body["stream"], false);
        assert_eq!(body["tools"][0]["name"], "Read", "agent 自己的工具不动");
        assert!(body["tools"][1].get("type").is_none());
        assert_eq!(body["tools"][1]["input_schema"]["required"][0], "query");
        assert_eq!(body["tools"][2]["input_schema"]["required"][0], "url");
    }

    #[test]
    fn a_plain_tool_with_the_same_name_is_left_to_the_agent() {
        let body = json!({"tools": [{"name": "web_search", "input_schema": {}}, {"type": "web_search_20250305", "name": "web_search"}]});
        assert!(managed_tools(&body).is_empty());
        assert!(managed_tools(&json!({"tools": [{"name": "Read"}]})).is_empty());
    }

    #[test]
    fn calls_are_read_and_foreign_tools_are_noticed() {
        let managed = managed_tools(&claude_code_body());
        let message = json!({"content": [
            {"type": "text", "text": "我来搜"},
            {"type": "tool_use", "id": "t1", "name": "web_search", "input": {"query": "rust 1.90"}},
            {"type": "tool_use", "id": "t2", "name": "web_fetch", "input": {"url": "https://x"}}
        ], "stop_reason": "tool_use"});
        let (calls, foreign) = calls_in(&message, &managed);
        assert!(!foreign);
        assert_eq!(calls, vec![
            Call { id: "t1".into(), kind: "search", argument: "rust 1.90".into() },
            Call { id: "t2".into(), kind: "fetch", argument: "https://x".into() },
        ]);
        let mixed = json!({"content": [{"type": "tool_use", "id": "t3", "name": "Read", "input": {}}, {"type": "tool_use", "id": "t4", "name": "web_search", "input": {"query": "q"}}]});
        let (calls, foreign) = calls_in(&mixed, &managed);
        assert!(foreign && calls.len() == 1);
        let mut stripped = mixed.clone();
        strip_managed(&mut stripped, &managed);
        assert_eq!(stripped["content"].as_array().unwrap().len(), 1, "只剩 agent 自己的调用");
    }

    #[test]
    fn rounds_append_and_the_final_message_carries_notes_and_summed_usage() {
        let mut body = claude_code_body();
        let round = json!({"content": [{"type": "tool_use", "id": "t1", "name": "web_search", "input": {"query": "q"}}], "usage": {"input_tokens": 100, "output_tokens": 10}});
        append_round(&mut body, &round, &[("t1".into(), "结果".into(), false)]);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "t1");
        let mut usage = (0, 0);
        add_usage(&mut usage, &round);
        let last = json!({"id": "msg_1", "model": "glm", "content": [{"type": "text", "text": "答案"}], "stop_reason": "end_turn", "usage": {"input_tokens": 150, "output_tokens": 20}});
        add_usage(&mut usage, &last);
        let done = finish(last, &["搜索「q」，3 条结果".into()], usage, &managed_tools(&claude_code_body()), false);
        assert!(done["content"][0]["text"].as_str().unwrap().contains("搜索「q」"));
        assert_eq!(done["content"][1]["text"], "答案");
        assert_eq!((done["usage"]["input_tokens"].as_i64(), done["usage"]["output_tokens"].as_i64()), (Some(250), Some(30)));
    }

    #[test]
    fn exhausted_rounds_end_the_turn_without_dangling_managed_calls() {
        let managed = managed_tools(&claude_code_body());
        let last = json!({"content": [{"type": "tool_use", "id": "t9", "name": "web_search", "input": {"query": "again"}}], "stop_reason": "tool_use", "usage": {}});
        let done = finish(last, &[], (1, 1), &managed, true);
        assert_eq!(done["stop_reason"], "end_turn");
        assert!(done["content"].as_array().unwrap().iter().all(|b| b["type"] != "tool_use"));
        assert!(done["content"][0]["text"].as_str().unwrap().contains(&MAX_ROUNDS.to_string()));
    }

    #[test]
    fn a_message_replays_as_well_formed_sse() {
        let message = json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "m",
            "content": [{"type": "text", "text": "你好"}, {"type": "tool_use", "id": "t", "name": "Read", "input": {"path": "a"}}],
            "stop_reason": "tool_use", "usage": {"input_tokens": 5, "output_tokens": 7}});
        let sse = message_to_sse(&message);
        let names: Vec<&str> = sse.lines().filter_map(|l| l.strip_prefix("event: ")).collect();
        assert_eq!(names, ["message_start", "content_block_start", "content_block_delta", "content_block_stop",
            "content_block_start", "content_block_delta", "content_block_stop", "message_delta", "message_stop"]);
        assert!(sse.contains(r#""partial_json":"{\"path\":\"a\"}""#));
        assert!(sse.contains(r#""stop_reason":"tool_use""#));
        assert!(sse.contains(r#""output_tokens":7"#));
    }
}
