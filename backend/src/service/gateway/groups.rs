//! 网关路径分组的增删改（从 `lib.rs::gateway_group_*` 抽出）。
//!
//! 参数校验与 slug 规则留在 `crate::gateway::groups`（那边是唯一的真相来源）；
//! 这里只做「拿到连接 → 调用 → 统一错误文案」，好让 Tauri 命令和 CLI 共用。

use rusqlite::Connection;

use crate::gateway::groups::{self, GroupDto, GroupsDto};

/// 分组种类只接受这两个值，写错直接报错而不是静默当成来源。
fn check_kind(kind: &str) -> Result<(), String> {
    if matches!(kind, "source" | "agent") {
        Ok(())
    } else {
        Err(format!("分组种类只能是 source 或 agent，收到「{kind}」"))
    }
}

/// 全部来源分组与 Agent 分组。
pub fn list(conn: &Connection) -> Result<GroupsDto, String> {
    groups::list(conn).map_err(|error| error.to_string())
}

/// 新建分组。
pub fn create(conn: &Connection, kind: &str, name: &str) -> Result<GroupDto, String> {
    check_kind(kind)?;
    groups::create(conn, kind, name)
}

/// 改名。
pub fn rename(conn: &Connection, kind: &str, id: &str, name: &str) -> Result<(), String> {
    check_kind(kind)?;
    groups::rename(conn, kind, id, name)
}

/// 删除分组（成员回到未分组，不删成员本身）。
pub fn delete(conn: &Connection, kind: &str, id: &str) -> Result<(), String> {
    check_kind(kind)?;
    groups::delete(conn, kind, id)
}

/// 按给定顺序重排；`ids` 里没提到的分组排在其后。
pub fn reorder(conn: &Connection, kind: &str, ids: &[String]) -> Result<(), String> {
    check_kind(kind)?;
    groups::reorder(conn, kind, ids)
}

/// 把成员移入某个分组；`group_id` 为 None 表示移出分组。
pub fn move_member(
    conn: &Connection, kind: &str, member: &str, group_id: Option<&str>,
) -> Result<(), String> {
    check_kind(kind)?;
    groups::move_member(conn, kind, member, group_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (Connection, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "jolu-groups-svc-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        (crate::db::open(&path).unwrap(), path)
    }

    #[test]
    fn unknown_kind_is_rejected_with_the_value_in_the_message() {
        let (conn, path) = database();
        let error = list(&conn).and_then(|_| create(&conn, "group", "x")).unwrap_err();
        assert!(error.contains("group"), "报错要带上收到的值，便于对照：{error}");
        assert!(create(&conn, "source", "可用").is_ok());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn every_mutation_rejects_a_bad_kind() {
        let (conn, path) = database();
        assert!(rename(&conn, "bogus", "1", "x").is_err());
        assert!(delete(&conn, "bogus", "1").is_err());
        assert!(reorder(&conn, "bogus", &[]).is_err());
        assert!(move_member(&conn, "bogus", "m", None).is_err());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn create_then_list_round_trip() {
        let (conn, path) = database();
        let created = create(&conn, "agent", "写代码").unwrap();
        let all = list(&conn).unwrap();
        assert!(all.agents.iter().any(|group| group.id == created.id));
        // 清理
        delete(&conn, "agent", &created.id).unwrap();
        let after = list(&conn).unwrap();
        assert!(!after.agents.iter().any(|group| group.id == created.id));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
