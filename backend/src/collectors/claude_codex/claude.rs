use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use serde_json::Value;
use crate::db::RequestRecord;
use super::{iso_to_millis, n, SessionFileState, SOURCE_LOCAL};

/// Claude Code 的 ai-title；Codex 标题从独立的 session_index.jsonl 同步。
pub(crate) struct TitleHit {
    pub(crate) platform: &'static str,
    pub(crate) session_id: String,
    pub(crate) title: String,
}

pub(crate) fn parse_claude_title(v: &Value) -> Option<TitleHit> {
    if v.get("type")?.as_str()? != "ai-title" {
        return None;
    }
    Some(TitleHit {
        platform: "claude",
        session_id: v.get("sessionId")?.as_str()?.to_string(),
        title: v.get("aiTitle")?.as_str()?.trim().to_string(),
    })
}

pub(crate) fn claude_projects_dir(home: &Path) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
        .join("projects")
}

/// NULL，界面显示「—」，不伪造。
pub(crate) fn parse_claude_line(v: &Value) -> Option<RequestRecord> {
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let msg = v.get("message")?;
    let usage = msg.get("usage")?;
    let dedup_key = msg.get("id")?.as_str()?.to_string();
    let ts = iso_to_millis(v.get("timestamp")?.as_str()?)?;

    let input = n(usage, "input_tokens");
    let output = n(usage, "output_tokens");
    let cache_write = n(usage, "cache_creation_input_tokens");
    let cache_read = n(usage, "cache_read_input_tokens");
    let model = msg.get("model").and_then(Value::as_str).map(String::from);

    // Claude Code 在权限拒绝、本地报错或合成响应时会写出 model="<synthetic>" 且四项 token 全 0 的占位行；
    // 跳过它避免虚增调用次数与制造虚假「<synthetic> 未计价」告警。
    if model.as_deref() == Some("<synthetic>")
        && [input, output, cache_write, cache_read]
            .iter()
            .all(|v| v.unwrap_or(0) == 0)
    {
        return None;
    }

    // Claude 的 input_tokens 不含缓存部分（缓存单列），因此总值需要相加，
    // 这不是「把缓存再次累加」。来源未提供显式 total，故由此口径计算。
    let total = input
        .zip(output)
        .zip(cache_write)
        .zip(cache_read)
        .map(|(((input, output), cache_write), cache_read)| input + output + cache_write + cache_read);

    Some(RequestRecord {
        platform: "claude".into(),
        source: SOURCE_LOCAL.into(),
        dedup_key,
        session_id: v.get("sessionId").and_then(Value::as_str).map(String::from),
        ts,
        model,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        total_tokens: total,
        // 思考强度就在同一条 assistant 行上，与 usage 同源，无需跨行继承
        effort: v.get("effort").and_then(Value::as_str).map(String::from),
    })
}

pub(crate) fn parse_claude_cache_write_1h(v: &Value) -> Option<i64> {
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    v.pointer("/message/usage/cache_creation/ephemeral_1h_input_tokens")
        .and_then(Value::as_i64)
        .filter(|&tokens| tokens > 0)
}

/// 测试用：按原始日志行驱动生命周期解析。
#[cfg(test)]
pub(crate) fn apply_claude_file_state(state: &mut SessionFileState, line: &str) {
    if let Ok(value) = serde_json::from_str::<Value>(line) {
        apply_claude_value(state, &value);
    }
}

pub(crate) fn apply_claude_value(state: &mut SessionFileState, v: &Value) {
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) { return; }
    let Some(id) = v.get("sessionId").and_then(Value::as_str) else { return };
    let Some(ts) = v.get("timestamp").and_then(Value::as_str).and_then(iso_to_millis) else { return };
    if state.updated_at_ms.is_some_and(|old| ts < old) { return; }
    state.session_id = Some(id.into());
    state.updated_at_ms = Some(ts);
    let kind = v.get("type").and_then(Value::as_str);
    let msg = v.get("message");
    let content = msg.and_then(|m| m.get("content"));
    let interrupted_text = |s: &str| {
        matches!(
            s.trim(),
            "[Request interrupted by user]" | "[Request interrupted by user for tool use]"
        )
    };
    let interrupted = content.and_then(Value::as_str).is_some_and(interrupted_text)
        || content.and_then(Value::as_array).is_some_and(|parts| {
            parts.iter().any(|p| {
                (p.get("type").and_then(Value::as_str) == Some("text")
                    && p.get("text").and_then(Value::as_str).is_some_and(interrupted_text))
                    || (p.get("type").and_then(Value::as_str) == Some("tool_result")
                        && p.get("content").and_then(Value::as_str).is_some_and(interrupted_text))
            })
        });
    let tool_result = v.get("toolUseResult").is_some()
        || v.get("sourceToolAssistantUUID").is_some()
        || content.and_then(Value::as_array).is_some_and(|parts| {
            parts.iter().any(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
        });
    let has_tool_use = content.and_then(Value::as_array).is_some_and(|parts| {
        parts.iter().any(|p| {
            matches!(
                p.get("type").and_then(Value::as_str),
                Some("tool_use" | "server_tool_use" | "mcp_tool_use")
            )
        })
    });
    let stop_reason = msg.and_then(|m| m.get("stop_reason")).and_then(Value::as_str);
    match kind {
        Some("user") if interrupted => {
            state.activity_state = Some("failed".into());
        }
        Some("user") if !tool_result && v.get("isMeta").and_then(Value::as_bool) != Some(true) => {
            if state.activity_state.as_deref() != Some("running") {
                state.activity_state = Some("running".into());
                state.turn_started_ms = Some(ts);
            }
        }
        // Claude Code 2.x 在每次真正发出请求瞬间写入 api-request（含队列出队续跑场景），
        // 无需傻等十几秒后的首条 assistant 落盘即可点亮运行态。
        Some("api-request") => {
            if state.activity_state.as_deref() != Some("running") {
                state.activity_state = Some("running".into());
                state.turn_started_ms = Some(ts);
            }
        }
        // 重试耗尽或不可重试时 CLI 写出的合成错误记录（isApiErrorMessage）是本轮
        // 最终失败的唯一终止信号；期间 source=request_retry 的 system 记录只是自动
        // 重试，不算结束。缺了这一臂，报错的会话会永远停留在 running 一直计时。
        Some("assistant") if v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) => {
            state.activity_state = Some("failed".into());
        }
        // 只要消息携带 tool_use 块或明确 stop_reason=tool_use，接下来必然进入工具/命令执行期
        // （部分第三方网关把带 tool_calls 的响应仍映射成 stop_reason=end_turn，绝不能据此误判结束）。
        Some("assistant") if has_tool_use || stop_reason == Some("tool_use") => {
            if state.activity_state.as_deref() != Some("running") {
                state.turn_started_ms = Some(ts);
            }
            state.activity_state = Some("running".into());
        }
        Some("assistant") if matches!(stop_reason, Some("end_turn" | "stop_sequence" | "max_tokens")) => {
            state.activity_state = Some("done".into());
        }
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("turn_duration") => {
            state.activity_state = Some("done".into());
        }
        _ => {}
    }
}

pub(crate) const CLAUDE_CHAIN_SEED_BYTES: i64 = 256 * 1024;
pub(crate) const MAX_CHAIN_HOPS: usize = 32;

pub(crate) struct ClaudeChainNode {
    pub(crate) parent: Option<String>,
    pub(crate) ts_ms: Option<i64>,
    pub(crate) is_attachment: bool,
}

pub(crate) struct ClaudeMsgTiming {
    pub(crate) first_parent: Option<String>,
    pub(crate) starts_at_first_block: bool,
    pub(crate) end_ms: Option<i64>,
    pub(crate) completed: bool,
}

pub(crate) fn record_claude_timing_line(
    v: &Value,
    chain: &mut HashMap<String, ClaudeChainNode>,
    timings: &mut HashMap<String, ClaudeMsgTiming>,
) {
    let ts_ms = v.get("timestamp").and_then(Value::as_str).and_then(iso_to_millis);
    if let Some(uuid) = v.get("uuid").and_then(Value::as_str) {
        chain.insert(
            uuid.to_string(),
            ClaudeChainNode {
                parent: v.get("parentUuid").and_then(Value::as_str).map(String::from),
                ts_ms,
                is_attachment: v.get("type").and_then(Value::as_str) == Some("attachment"),
            },
        );
    }
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let Some(msg) = v.get("message") else { return };
    let Some(msg_id) = msg.get("id").and_then(Value::as_str) else { return };
    let entry = timings.entry(msg_id.to_string()).or_insert_with(|| ClaudeMsgTiming {
        first_parent: v.get("parentUuid").and_then(Value::as_str).map(String::from),
        starts_at_first_block: v.get("apiBlockIndex").is_none_or(|idx| idx.as_u64() == Some(0)),
        end_ms: None,
        completed: false,
    });
    if let Some(ts) = ts_ms {
        entry.end_ms = Some(entry.end_ms.map_or(ts, |old| old.max(ts)));
    }
    if msg.get("stop_reason").and_then(Value::as_str).is_some() {
        entry.completed = true;
    }
}

pub(crate) fn resolve_claude_start_ms(chain: &HashMap<String, ClaudeChainNode>, first_parent: Option<&str>) -> Option<i64> {
    let mut cursor = first_parent?;
    for _ in 0..MAX_CHAIN_HOPS {
        let node = chain.get(cursor)?;
        if !node.is_attachment {
            return node.ts_ms;
        }
        cursor = node.parent.as_deref()?;
    }
    None
}

pub(crate) fn seed_claude_chain(
    file: &mut std::fs::File,
    offset: i64,
    chain: &mut HashMap<String, ClaudeChainNode>,
    timings: &mut HashMap<String, ClaudeMsgTiming>,
) {
    let len = offset.clamp(0, CLAUDE_CHAIN_SEED_BYTES);
    if len == 0 {
        return;
    }
    let mut buf = vec![0u8; len as usize];
    let ok = file
        .seek(SeekFrom::Start((offset - len) as u64))
        .and_then(|_| file.read_exact(&mut buf))
        .is_ok();
    let _ = file.seek(SeekFrom::Start(offset as u64));
    if !ok {
        return;
    }
    let mut lines = buf.split(|b| *b == b'\n');
    if offset > len {
        lines.next();
    }
    for line in lines {
        if let Ok(v) = serde_json::from_slice::<Value>(line) {
            record_claude_timing_line(&v, chain, timings);
        }
    }
}
