//! 受控轻量级 Agent Hooks（4B）：感知后台终端「等待授权确认 / 等待输入 / 本轮完成」。
//!
//! 纪律（严守 AGENTS.md §4）：
//! - 默认关闭（`island_linkage.agent_hooks = false`），未经用户显式开启绝不触碰任何 CLI 配置；
//! - 开启时仅追加标记为 `--agent-hook` 的通知回执命令，保留用户原有的全部 hooks；
//! - 关闭或卸载（`--restore-agents`）时按快照零残留还原；
//! - 状态回执文件只写事件类型、会话 ID 与脱敏短摘要（≤ 80 字符），不记录提示词正文与任何密钥；
//! - `--agent-hook` 恒以退出码 0、空 stdout 立即返回，绝不改变或阻断 CLI 的交互审批行为。

use crate::gateway::wiring::write_atomic;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

const HOOK_FLAG: &str = "--agent-hook";
const CLAUDE_EVENTS: [&str; 4] = ["PermissionRequest", "Notification", "UserPromptSubmit", "Stop"];
const APPROVAL_TTL_MS: i64 = 15 * 60 * 1_000;
const COMPLETE_TTL_MS: i64 = 12 * 1_000;

/// 暴露给前端与状态文件的轻量回执（不含提示词或凭据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHookAlertDto {
    /// `"claude"` | `"codex"`
    pub platform: String,
    /// `"waiting_approval"` | `"waiting_input"` | `"turn_complete"` | `"running"` | `"idle"`
    pub kind: String,
    pub session_id: Option<String>,
    pub summary: String,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HookSnapshot {
    codex_prior_notify: Option<Vec<String>>,
}

pub(crate) fn hooks_state_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-hooks")
}

fn snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-hooks-snapshot.json")
}

fn read_snapshot(data_dir: &Path) -> HookSnapshot {
    std::fs::read_to_string(snapshot_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str::<HookSnapshot>(&text).ok())
        .unwrap_or_default()
}

fn write_snapshot(data_dir: &Path, snap: &HookSnapshot) {
    if let Ok(bytes) = serde_json::to_vec_pretty(snap) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = write_atomic(&snapshot_path(data_dir), bytes);
    }
}

/// 清洗并截断简短说明：剔除控制字符、遮蔽疑似密钥前缀、限制 80 字符。
pub(crate) fn sanitize_summary(raw: &str) -> String {
    let collapsed: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut masked_words = Vec::new();
    for word in collapsed.split(' ') {
        if word.is_empty() {
            continue;
        }
        let hit_prefix = crate::infra::redact::PREFIX_KEYS
            .iter()
            .any(|prefix| word.starts_with(prefix) && word.len() >= prefix.len() + 8);
        if hit_prefix {
            masked_words.push("[已脱敏]".to_string());
        } else {
            masked_words.push(word.to_string());
        }
    }
    let joined = masked_words.join(" ");
    joined.chars().take(80).collect()
}

fn safe_session_id(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() || trimmed.len() > 128 || trimmed.chars().any(char::is_control) {
        return None;
    }
    Some(trimmed.to_string())
}

fn is_our_command(cmd: &str) -> bool {
    cmd.contains(HOOK_FLAG)
}

/// 在一份 Claude `settings.json` 文本上启停 Jolu 的轻量级通知 hooks（纯函数，便于单测）。
pub(crate) fn patch_claude_hooks_json(text: &str, exe_path: &Path, enabled: bool) -> Result<String, String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text).map_err(|e| format!("Claude settings.json 不是有效 JSON：{e}"))?
    };
    let obj = root.as_object_mut().ok_or("Claude settings.json 顶层不是对象")?;

    if !enabled {
        if let Some(hooks_obj) = obj.get_mut("hooks").and_then(|v| v.as_object_mut()) {
            for event_name in CLAUDE_EVENTS {
                if let Some(groups) = hooks_obj.get_mut(event_name).and_then(|v| v.as_array_mut()) {
                    groups.retain_mut(|group| {
                        let Some(inner) = group.get_mut("hooks").and_then(|v| v.as_array_mut()) else {
                            return true;
                        };
                        inner.retain(|item| {
                            !item
                                .get("command")
                                .and_then(|v| v.as_str())
                                .is_some_and(is_our_command)
                        });
                        !inner.is_empty()
                    });
                    if groups.is_empty() {
                        hooks_obj.remove(event_name);
                    }
                }
            }
            if hooks_obj.is_empty() {
                obj.remove("hooks");
            }
        }
        return serde_json::to_string_pretty(&root).map_err(|e| e.to_string());
    }

    let command = format!("\"{}\" {HOOK_FLAG} claude", exe_path.display());
    let hooks_val = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks_val.as_object_mut().ok_or("Claude settings.json 的 hooks 字段不是对象")?;

    for event_name in CLAUDE_EVENTS {
        let groups_val = hooks_obj.entry(event_name).or_insert_with(|| serde_json::json!([]));
        let groups = groups_val
            .as_array_mut()
            .ok_or_else(|| format!("Claude hooks.{event_name} 不是数组"))?;
        groups.retain_mut(|group| {
            let Some(inner) = group.get_mut("hooks").and_then(|v| v.as_array_mut()) else {
                return true;
            };
            inner.retain(|item| {
                !item
                    .get("command")
                    .and_then(|v| v.as_str())
                    .is_some_and(is_our_command)
            });
            !inner.is_empty()
        });
        groups.push(serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": 5
            }]
        }));
    }
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

fn codex_notify_is_ours(doc: &toml_edit::DocumentMut) -> bool {
    doc.get("notify")
        .and_then(|v| v.as_array())
        .is_some_and(|arr| arr.iter().any(|item| item.as_str().is_some_and(is_our_command)))
}

/// 在一份 Codex `config.toml` 文本上启停 Jolu 的 `notify` 钩子（纯函数，保留注释与排版）。
pub(crate) fn patch_codex_notify_toml(
    text: &str,
    exe_path: &Path,
    enabled: bool,
    prior_notify: &mut Option<Vec<String>>,
) -> Result<String, String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("Codex config.toml 格式无效：{e}"))?;

    if enabled {
        if !codex_notify_is_ours(&doc) {
            if let Some(existing) = doc.get("notify").and_then(|v| v.as_array()) {
                let saved: Vec<String> = existing
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                if !saved.is_empty() {
                    *prior_notify = Some(saved);
                }
            }
        }
        let mut arr = toml_edit::Array::new();
        arr.push(exe_path.display().to_string());
        arr.push(HOOK_FLAG);
        arr.push("codex");
        doc["notify"] = toml_edit::value(arr);
        return Ok(doc.to_string());
    }

    if codex_notify_is_ours(&doc) {
        if let Some(saved) = prior_notify.take() {
            let mut arr = toml_edit::Array::new();
            for token in saved {
                arr.push(token);
            }
            doc["notify"] = toml_edit::value(arr);
        } else {
            doc.remove("notify");
        }
    }
    Ok(doc.to_string())
}

/// 启停受控 Agent Hooks（写盘前备份原值，关闭时清理状态文件与快照）。
pub fn apply_hooks(data_dir: &Path, exe_path: &Path, enabled: bool) -> Result<(), String> {
    if crate::validation::isolated() {
        return Ok(());
    }
    let mut snap = read_snapshot(data_dir);

    if let Some(claude_dir) = crate::creds::claude_dir().filter(|dir| dir.exists()) {
        let settings_path = claude_dir.join("settings.json");
        if enabled || settings_path.exists() {
            let current = crate::steady::read_optional(&settings_path)
                .map_err(|e| format!("读取 {} 失败：{e}", settings_path.display()))?
                .unwrap_or_default();
            let patched = patch_claude_hooks_json(&current, exe_path, enabled)?;
            if patched != current {
                write_atomic(&settings_path, patched.into_bytes())
                    .map_err(|e| format!("写入 {} 失败：{e}", settings_path.display()))?;
            }
        }
    }

    if let Some(codex_dir) = crate::creds::codex_dir().filter(|dir| dir.exists()) {
        let config_path = codex_dir.join("config.toml");
        if enabled || config_path.exists() {
            let current = crate::steady::read_optional(&config_path)
                .map_err(|e| format!("读取 {} 失败：{e}", config_path.display()))?
                .unwrap_or_default();
            let patched = patch_codex_notify_toml(&current, exe_path, enabled, &mut snap.codex_prior_notify)?;
            if patched != current {
                write_atomic(&config_path, patched.into_bytes())
                    .map_err(|e| format!("写入 {} 失败：{e}", config_path.display()))?;
            }
        }
    }

    let state_dir = hooks_state_dir(data_dir);
    if enabled {
        let _ = std::fs::create_dir_all(&state_dir);
        write_snapshot(data_dir, &snap);
    } else {
        let _ = std::fs::remove_file(snapshot_path(data_dir));
        if let Ok(entries) = std::fs::read_dir(&state_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// 卸载（`--restore-agents`）或回退时无条件清理全部已注入的 Agent Hooks。
pub fn restore_all(data_dir: &Path) -> Vec<String> {
    let dummy_exe = Path::new("Jolu.exe");
    match apply_hooks(data_dir, dummy_exe, false) {
        Ok(()) => vec!["已还原 Agent 确认提醒钩子".into()],
        Err(error) => vec![format!("失败：还原 Agent 钩子出错：{error}")],
    }
}

pub(crate) fn parse_claude_hook_payload(raw: &str, now_ms: i64) -> Option<AgentHookAlertDto> {
    let val: serde_json::Value = serde_json::from_str(raw).ok()?;
    let event = val.get("hook_event_name")?.as_str()?.trim();
    let session_id = safe_session_id(val.get("session_id").and_then(|v| v.as_str()));
    match event {
        "PermissionRequest" => {
            let tool = val
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(sanitize_summary)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "工具".into());
            Some(AgentHookAlertDto {
                platform: "claude".into(),
                kind: "waiting_approval".into(),
                session_id,
                summary: sanitize_summary(&format!("等待确认 {tool} 权限")),
                updated_ms: now_ms,
            })
        }
        "Notification" => {
            let ntype = val
                .get("notification_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match ntype {
                "permission_prompt" => Some(AgentHookAlertDto {
                    platform: "claude".into(),
                    kind: "waiting_approval".into(),
                    session_id,
                    summary: "等待终端授权确认".into(),
                    updated_ms: now_ms,
                }),
                "idle_prompt" | "elicitation_dialog" => Some(AgentHookAlertDto {
                    platform: "claude".into(),
                    kind: "waiting_input".into(),
                    session_id,
                    summary: "等待你在终端输入".into(),
                    updated_ms: now_ms,
                }),
                _ => None,
            }
        }
        "UserPromptSubmit" => Some(AgentHookAlertDto {
            platform: "claude".into(),
            kind: "running".into(),
            session_id,
            summary: "已继续执行".into(),
            updated_ms: now_ms,
        }),
        "Stop" => Some(AgentHookAlertDto {
            platform: "claude".into(),
            kind: "idle".into(),
            session_id,
            summary: "本轮已完成".into(),
            updated_ms: now_ms,
        }),
        _ => None,
    }
}

pub(crate) fn parse_codex_hook_payload(raw: &str, now_ms: i64) -> Option<AgentHookAlertDto> {
    let val: serde_json::Value = serde_json::from_str(raw).ok()?;
    let kind_str = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let session_id = safe_session_id(
        val.get("thread-id")
            .or_else(|| val.get("thread_id"))
            .or_else(|| val.get("session_id"))
            .and_then(|v| v.as_str()),
    );
    match kind_str {
        "agent-turn-complete" => Some(AgentHookAlertDto {
            platform: "codex".into(),
            kind: "turn_complete".into(),
            session_id,
            summary: "本轮已完成 · 等待下一步".into(),
            updated_ms: now_ms,
        }),
        "approval-requested" | "permission-request" => Some(AgentHookAlertDto {
            platform: "codex".into(),
            kind: "waiting_approval".into(),
            session_id,
            summary: "等待终端命令审批".into(),
            updated_ms: now_ms,
        }),
        _ => None,
    }
}

/// CLI 入口：`Jolu.exe --agent-hook <claude|codex> [payload]`。
/// 恒返回退出码 0、不写 stdout，确保绝不阻断或自动审批原工具操作。
pub fn run_agent_hook_cli(data_dir: &Path) -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let Some(pos) = args.iter().position(|a| a == HOOK_FLAG) else {
        return 0;
    };
    let Some(agent) = args.get(pos + 1).map(|s| s.trim().to_ascii_lowercase()) else {
        return 0;
    };
    let now_ms = chrono::Local::now().timestamp_millis();

    let alert = match agent.as_str() {
        "claude" => {
            let mut buf = String::new();
            let _ = std::io::stdin().take(64 * 1024).read_to_string(&mut buf);
            parse_claude_hook_payload(&buf, now_ms)
        }
        "codex" => args
            .get(pos + 2)
            .and_then(|payload| parse_codex_hook_payload(payload, now_ms)),
        _ => None,
    };

    if let Some(alert) = alert {
        let dir = hooks_state_dir(data_dir);
        if std::fs::create_dir_all(&dir).is_ok() {
            if let Ok(bytes) = serde_json::to_vec(&alert) {
                let target = dir.join(format!("{}.json", alert.platform));
                let _ = write_atomic(&target, bytes);
            }
        }
    }
    0
}

fn session_resumed_since(conn: &Connection, platform: &str, session_id: Option<&str>, since_ms: i64) -> bool {
    let threshold = since_ms + 1_500;
    let latest: Option<i64> = match session_id {
        Some(sid) => conn
            .query_row(
                "SELECT MAX(ts) FROM usage_events WHERE platform = ?1 AND session_id = ?2",
                params![platform, sid],
                |r| r.get(0),
            )
            .ok()
            .flatten(),
        None => conn
            .query_row(
                "SELECT MAX(ts) FROM usage_events WHERE platform = ?1",
                params![platform],
                |r| r.get(0),
            )
            .ok()
            .flatten(),
    };
    latest.is_some_and(|ts| ts > threshold)
}

/// 读取当前仍有效的确认提醒列表（自动过滤已恢复运行、已完成或过期的回执）。
pub(crate) fn active_alerts(data_dir: &Path, conn: Option<&Connection>, now_ms: i64) -> Vec<AgentHookAlertDto> {
    if crate::validation::isolated() {
        return Vec::new();
    }
    let dir = hooks_state_dir(data_dir);
    let mut out = Vec::new();
    for platform in ["claude", "codex"] {
        let path = dir.join(format!("{platform}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(alert) = serde_json::from_str::<AgentHookAlertDto>(&text) else { continue };
        let age = now_ms.saturating_sub(alert.updated_ms);
        let valid = match alert.kind.as_str() {
            "waiting_approval" | "waiting_input" => age <= APPROVAL_TTL_MS,
            "turn_complete" => age <= COMPLETE_TTL_MS,
            _ => false,
        };
        if !valid {
            continue;
        }
        if let Some(c) = conn {
            if session_resumed_since(c, &alert.platform, alert.session_id.as_deref(), alert.updated_ms) {
                continue;
            }
        }
        out.push(alert);
    }
    out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_hooks_patch_preserves_user_hooks_and_restores_zero_residue() {
        let original = r#"{
  "model": "sonnet",
  "hooks": {
    "Notification": [
      {
        "matcher": "custom",
        "hooks": [
          { "type": "command", "command": "echo user-hook" }
        ]
      }
    ]
  }
}"#;
        let exe = Path::new(r"C:\Users\Demo\AppData\Local\Jolu\Jolu.exe");
        let enabled = patch_claude_hooks_json(original, exe, true).unwrap();
        assert!(enabled.contains("echo user-hook"), "必须完整保留用户已有 hook");
        assert!(enabled.contains("--agent-hook claude"));

        let restored = patch_claude_hooks_json(&enabled, exe, false).unwrap();
        let val: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(!restored.contains("--agent-hook"));
        assert_eq!(val["hooks"]["Notification"].as_array().unwrap().len(), 1);
        assert!(val["hooks"].get("PermissionRequest").is_none());

        // 原本无任何 hooks 的配置关闭后连顶层 hooks 键一并移除
        let clean = patch_claude_hooks_json(r#"{"model":"opus"}"#, exe, true).unwrap();
        let clean_back = patch_claude_hooks_json(&clean, exe, false).unwrap();
        let clean_val: serde_json::Value = serde_json::from_str(&clean_back).unwrap();
        assert!(clean_val.get("hooks").is_none());
    }

    #[test]
    fn codex_notify_patch_snapshots_and_restores_user_notify() {
        let original = "model = \"gpt-5-codex\"\nnotify = [\"notify-send\", \"Codex\"]\n";
        let exe = Path::new(r"C:\Jolu\Jolu.exe");
        let mut prior = None;
        let wired = patch_codex_notify_toml(original, exe, true, &mut prior).unwrap();
        assert_eq!(prior, Some(vec!["notify-send".into(), "Codex".into()]));
        assert!(wired.contains("--agent-hook"));

        let restored = patch_codex_notify_toml(&wired, exe, false, &mut prior).unwrap();
        assert!(!restored.contains("--agent-hook"));
        assert!(restored.contains("notify-send"));
    }

    #[test]
    fn payload_parsers_sanitize_secrets_and_extract_approval_states() {
        let claude_perm = r#"{"session_id":"s-1","hook_event_name":"PermissionRequest","tool_name":"Bash sk-1234567890abcdefghij"}"#;
        let dto = parse_claude_hook_payload(claude_perm, 5_000).unwrap();
        assert_eq!(dto.kind, "waiting_approval");
        assert!(!dto.summary.contains("sk-1234567890abcdefghij"));
        assert!(dto.summary.contains("[已脱敏]"));

        let codex_done = r#"{"type":"agent-turn-complete","thread-id":"t-42"}"#;
        let cdto = parse_codex_hook_payload(codex_done, 6_000).unwrap();
        assert_eq!(cdto.kind, "turn_complete");
        assert_eq!(cdto.session_id.as_deref(), Some("t-42"));
    }
}
