use serde_json::Value;
use crate::db::RequestRecord;
use super::{iso_to_millis, n, SessionFileState, SOURCE_LOCAL};

/// Codex：`~/.codex/sessions/<y>/<m>/<d>/rollout-*.jsonl`
///
/// `token_usage_record` 行的 payload：
///   - `usage`              每次响应的**增量**（我们要的）
///   - `turn_token_usage` / `thread_token_usage`  **累计值**，不能直接求和
///   - `response_id`        去重键
///
/// model 不在该行上，需沿用文件中最近一条 `turn_context` 的 payload.model。
/// 按路径逐层取字符串；任一层缺失或类型不符都返回 None。
pub(crate) fn nested_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(key)?;
    }
    current.as_str()
}

pub(crate) fn parse_codex_line(
    v: &Value,
    current_model: &mut Option<String>,
    current_effort: &mut Option<String>,
) -> Option<RequestRecord> {
    let ty = v.get("type").and_then(Value::as_str)?;

    if ty == "turn_context" {
        let payload = v.get("payload");
        if let Some(m) = payload.and_then(|p| p.get("model")).and_then(Value::as_str) {
            *current_model = Some(m.to_string());
        }
        // 思考强度同样只在 turn_context 上，按会话继承到后续的用量行。
        // 实测存在三种嵌套位置，按由内到外的顺序回退；都没有就保持上一轮的值。
        if let Some(p) = payload {
            if let Some(e) = nested_str(p, &["thread_settings", "reasoning_effort"])
                .or_else(|| nested_str(p, &["thread_settings", "collaboration_mode", "settings", "reasoning_effort"]))
                .or_else(|| nested_str(p, &["collaboration_mode", "settings", "reasoning_effort"]))
            {
                *current_effort = Some(e.to_string());
            }
        }
        return None;
    }
    if ty != "token_usage_record" {
        return None;
    }

    let payload = v.get("payload")?;
    let dedup_key = payload.get("response_id")?.as_str()?.to_string();
    let ts = iso_to_millis(v.get("timestamp")?.as_str()?)?;
    // 取增量而非累计
    let usage = payload.get("usage")?;

    let input = n(usage, "input_tokens");
    let output = n(usage, "output_tokens");
    let cached = n(usage, "cached_input_tokens");
    let cache_write = n(usage, "cache_write_input_tokens");

    // Codex 提供语义明确的 total_tokens，且其 input_tokens 已含 cached 部分，
    // 因此直接采用来源总值，不再把缓存累加一次（§7.5）。
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_i64)
        .or_else(|| input.zip(output).map(|(input, output)| input + output));

    Some(RequestRecord {
        platform: "codex".into(),
        source: SOURCE_LOCAL.into(),
        dedup_key,
        session_id: payload.get("session_id").and_then(Value::as_str).map(String::from),
        ts,
        model: current_model.clone(),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: cache_write,
        total_tokens: total,
        effort: current_effort.clone(),
    })
}

#[cfg(test)]
pub(crate) fn parse_codex_file_state<I, S>(lines: I) -> SessionFileState
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut state = SessionFileState::default();
    for line in lines {
        if let Ok(value) = serde_json::from_str::<Value>(line.as_ref()) {
            apply_codex_file_state(&mut state, &value);
        }
    }
    state
}

pub(crate) fn apply_codex_file_state(state: &mut SessionFileState, value: &Value) {
    let Some(kind) = value.get("type").and_then(Value::as_str) else { return };
    let payload = value.get("payload");

    if let Some(ts) = value.get("timestamp").and_then(Value::as_str).and_then(iso_to_millis) {
        state.updated_at_ms = Some(state.updated_at_ms.map_or(ts, |old| old.max(ts)));
    }
    if kind == "session_meta" {
        if let Some(session_id) = payload.and_then(|p| p.get("id")).and_then(Value::as_str) {
            state.session_id = Some(session_id.to_string());
        }
    }
    if kind == "turn_context" {
        if let Some(model) = payload.and_then(|p| p.get("model")).and_then(Value::as_str) {
            state.model = Some(model.to_string());
        }
    }
    if kind == "token_usage_record" && state.session_id.is_none() {
        state.session_id = payload
            .and_then(|p| p.get("session_id"))
            .and_then(Value::as_str)
            .map(String::from);
    }

    let event = payload.and_then(|p| p.get("type")).and_then(Value::as_str);
    let line_ts = value.get("timestamp").and_then(Value::as_str).and_then(iso_to_millis);
    match event {
        Some("task_started") => {
            state.activity_state = Some("running".into());
            state.turn_started_ms = line_ts;
        }
        Some("user_message") => {
            if state.activity_state.as_deref() != Some("running") {
                state.activity_state = Some("running".into());
                state.turn_started_ms = line_ts;
            }
        }
        Some(
            "function_call"
            | "custom_tool_call"
            | "local_shell_call"
            | "web_search_call"
            | "exec_command_begin"
            | "mcp_tool_call_begin",
        ) => {
            if state.activity_state.as_deref() != Some("running") {
                state.turn_started_ms = state.turn_started_ms.or(line_ts);
            }
            state.activity_state = Some("running".into());
        }
        Some("task_complete") => state.activity_state = Some("done".into()),
        Some("turn_aborted") => state.activity_state = Some("failed".into()),
        Some("agent_message")
            if payload.and_then(|p| p.get("phase")).and_then(Value::as_str)
                == Some("final_answer") => state.activity_state = Some("done".into()),
        _ if kind == "response_item"
            && event == Some("message")
            && payload.and_then(|p| p.get("phase")).and_then(Value::as_str)
                == Some("final_answer") => state.activity_state = Some("done".into()),
        _ => {}
    }
}

pub(crate) fn is_codex_boundary(v: &Value) -> bool {
    let ty = v.get("type").and_then(Value::as_str);
    let payload_ty = v.pointer("/payload/type").and_then(Value::as_str);
    matches!(ty, Some("turn_context"))
        || matches!(
            payload_ty,
            Some("user_message" | "task_started" | "function_call_output" | "custom_tool_call_output")
        )
        || (ty == Some("response_item") && payload_ty == Some("message") && v.pointer("/payload/role").and_then(Value::as_str) == Some("user"))
}
