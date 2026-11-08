//! Profiles（配置一键切换，magpie 同名功能）：把「每个 agent 接线时选的主模型」
//! 存成一个命名快照，切换时对**当前已接线**的 agent 逐个重放接线（复用
//! [`crate::gateway::escape::wire_agents`] 的备份 / 原子写 / 快照路径）。
//!
//! 纪律：
//! - 快照只存 **Jolu 自己的接线决策**（agent → 主模型），不存用户原配置——
//!   用户原配置的备份/还原归接线层管，Profile 不碰它，两层职责不混。
//! - 切换前必须过 drift 检测：接线层的 `status_with` 发现配置被外部改写时，
//!   该 agent 跳过并在结果里说明，绝不静默覆盖用户的手动改动。
//! - 未接线的 agent 不自动接线：Profile 只改变「已接东西」的模型，不替用户决定接什么。

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// agent → 主模型的有序映射（BTreeMap 保证快照 JSON 稳定，便于对比与测试）。
pub(crate) type AgentModels = BTreeMap<String, String>;

/// 切换结果：明细给 Toast，跳过的要有原因。
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ApplyResult {
    pub switched: Vec<String>,
    pub skipped: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Profile {
    pub name: String,
    pub agents: AgentModels,
    pub created_ms: i64,
}

fn table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profiles (
            name         TEXT PRIMARY KEY,
            agents_json  TEXT NOT NULL,
            created_ms   INTEGER NOT NULL
        );",
    )
}

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Profile>> {
    table(conn)?;
    let mut stmt = conn.prepare("SELECT name, agents_json, created_ms FROM profiles ORDER BY created_ms ASC")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (name, agents_json, created_ms) = row?;
        match serde_json::from_str::<AgentModels>(&agents_json) {
            Ok(agents) => out.push(Profile { name, agents, created_ms }),
            Err(error) => eprintln!("[Profile] 「{name}」的快照无效，已跳过：{error}"),
        }
    }
    Ok(out)
}

/// 保存 / 覆盖一个 Profile。`agents` 里模型为空的条目被丢弃——Profile 存的是决策，不是空占位。
pub(crate) fn save(conn: &Connection, name: &str, agents: &AgentModels) -> Result<Profile, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err("Profile 名称不能为空且不超过 64 字符".into());
    }
    let agents: AgentModels = agents
        .iter()
        .filter(|(_, model)| !model.trim().is_empty())
        .map(|(agent, model)| (agent.clone(), model.trim().to_string()))
        .collect();
    if agents.is_empty() {
        return Err("Profile 至少要包含一个 agent 的模型".into());
    }
    table(conn).map_err(|error| error.to_string())?;
    let agents_json = serde_json::to_string(&agents).map_err(|error| format!("快照序列化失败：{error}"))?;
    conn.execute(
        "INSERT INTO profiles(name, agents_json, created_ms) VALUES(?1,?2,?3)
         ON CONFLICT(name) DO UPDATE SET agents_json = excluded.agents_json",
        rusqlite::params![name, agents_json, chrono::Utc::now().timestamp_millis()],
    )
    .map_err(|error| error.to_string())?;
    let created_ms: i64 = conn
        .query_row("SELECT created_ms FROM profiles WHERE name = ?1", [name], |r| r.get(0))
        .map_err(|error| error.to_string())?;
    Ok(Profile { name: name.into(), agents, created_ms })
}

pub(crate) fn delete(conn: &Connection, name: &str) -> Result<bool, String> {
    table(conn).map_err(|error| error.to_string())?;
    let changed = conn
        .execute("DELETE FROM profiles WHERE name = ?1", [name])
        .map_err(|error| error.to_string())?;
    Ok(changed > 0)
}

pub(crate) fn get(conn: &Connection, name: &str) -> Result<Option<Profile>, String> {
    table(conn).map_err(|error| error.to_string())?;
    conn.query_row(
        "SELECT name, agents_json, created_ms FROM profiles WHERE name = ?1",
        [name],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )
    .optional()
    .map_err(|error| error.to_string())?
    .map(|(name, agents_json, created_ms)| {
        let display = name.clone();
        serde_json::from_str::<AgentModels>(&agents_json)
            .map(|agents| Profile { name, agents, created_ms })
            .map_err(|error| format!("Profile「{display}」的快照无效：{error}"))
    })
    .transpose()
}

/// 「保存当前」：从各 agent 的接线记录里收集已接线 agent 的主模型。
///
/// 只收 `state = wired` 且 applied 里带 model 的记录——没选过模型的接线（只有地址令牌）
/// 进 Profile 没有意义，切换时也无从重放。
pub(crate) fn capture_current(conn: &Connection) -> Result<AgentModels, String> {
    let mut out = AgentModels::new();
    let mut stmt = conn
        .prepare("SELECT agent_id, state, applied_json FROM agent_wiring WHERE state = 'wired'")
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (agent, _state, applied_json) = row.map_err(|error| error.to_string())?;
        let Ok(applied) = serde_json::from_str::<Value>(&applied_json) else { continue };
        let Some(model) = applied.get("model").and_then(Value::as_str).filter(|m| !m.trim().is_empty()) else {
            continue;
        };
        out.insert(agent, model.trim().to_string());
    }
    Ok(out)
}

/// 切换的差异说明：每个 agent 要从什么模型换到什么；`None` 表示该 agent 当前没接线，
/// Profile 不动它。调用方（命令层）拿它做确认弹窗与结果 Toast。
pub(crate) fn diff(current_wired: &AgentModels, profile: &Profile) -> BTreeMap<String, Option<String>> {
    let mut out = BTreeMap::new();
    for (agent, model) in &profile.agents {
        let from = current_wired.get(agent);
        // 未接线的 agent 不在切换范围（纪律：不替用户决定接什么）
        if current_wired.contains_key(agent) {
            out.insert(agent.clone(), Some(model.clone()).filter(|m| from.is_some_and(|f| f != m)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        table(&conn).unwrap();
        conn
    }

    #[test]
    fn save_round_trip_and_overwrite() {
        let conn = memory_db();
        let mut agents = AgentModels::new();
        agents.insert("claude-code".into(), "zcode/glm-4.6".into());
        save(&conn, "Focus", &agents).unwrap();
        agents.insert("codex".into(), "kimi/kimi-k2.5".into());
        save(&conn, "Focus", &agents).unwrap();
        let list = list(&conn).unwrap();
        assert_eq!(list.len(), 1, "同名覆盖不新增");
        assert_eq!(list[0].agents["codex"], "kimi/kimi-k2.5");
    }

    #[test]
    fn save_rejects_empty_name_and_empty_agents() {
        let conn = memory_db();
        assert!(save(&conn, "  ", &AgentModels::new()).is_err());
        assert!(save(&conn, "Focus", &AgentModels::new()).is_err(), "空 Profile 无意义");
        let mut agents = AgentModels::new();
        agents.insert("claude-code".into(), "  ".into());
        assert!(save(&conn, "Focus", &agents).is_err(), "纯空白的模型等于没决策");
    }

    #[test]
    fn capture_current_only_counts_wired_with_model() {
        let conn = memory_db();
        conn.execute_batch(
            "CREATE TABLE agent_wiring (agent_id TEXT PRIMARY KEY, state TEXT, applied_json TEXT);
             INSERT INTO agent_wiring VALUES ('claude-code', 'wired', '{\"model\":\"zcode/glm-4.6\"}');
             INSERT INTO agent_wiring VALUES ('codex', 'wired', '{}');
             INSERT INTO agent_wiring VALUES ('gemini', 'idle', '{\"model\":\"x\"}');",
        )
        .unwrap();
        let captured = capture_current(&conn).unwrap();
        assert_eq!(captured.len(), 1, "只有 wired 且带模型的才算决策");
        assert_eq!(captured["claude-code"], "zcode/glm-4.6");
    }

    #[test]
    fn diff_only_touches_wired_agents_and_reports_changes() {
        let mut current = AgentModels::new();
        current.insert("claude-code".into(), "zcode/glm-4.6".into());
        current.insert("codex".into(), "kimi/kimi-k2.5".into());
        let mut agents = AgentModels::new();
        agents.insert("claude-code".into(), "kimi/kimi-k2.5".into());
        agents.insert("codex".into(), "kimi/kimi-k2.5".into());
        agents.insert("gemini".into(), "x/y".into());
        let profile = Profile { name: "p".into(), agents, created_ms: 0 };
        let changes = diff(&current, &profile);
        // claude-code 有变化；codex 目标与当前相同 → 无需切换；gemini 未接线 → 不动
        assert_eq!(changes["claude-code"], Some("kimi/kimi-k2.5".into()));
        assert_eq!(changes["codex"], None);
        assert!(!changes.contains_key("gemini"), "未接线的 agent 不进切换范围");
    }

    #[test]
    fn delete_reports_whether_anything_was_removed() {
        let conn = memory_db();
        let mut agents = AgentModels::new();
        agents.insert("claude-code".into(), "a/b".into());
        save(&conn, "Focus", &agents).unwrap();
        assert!(delete(&conn, "Focus").unwrap());
        assert!(!delete(&conn, "Focus").unwrap(), "重复删除如实报告没删到");
    }

    #[test]
    fn snapshot_json_is_stable_across_resave() {
        let conn = memory_db();
        let mut agents = AgentModels::new();
        agents.insert("claude-code".into(), "a/b".into());
        agents.insert("codex".into(), "c/d".into());
        save(&conn, "Focus", &agents).unwrap();
        let first: String = conn
            .query_row("SELECT agents_json FROM profiles WHERE name='Focus'", [], |r| r.get(0))
            .unwrap();
        save(&conn, "Focus", &agents).unwrap();
        let again: String = conn
            .query_row("SELECT agents_json FROM profiles WHERE name='Focus'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(first, again, "BTreeMap 保证同内容同 JSON（切换对比与测试都依赖它）");
        let _ = json!(());
    }
}
