//! 命名客户端 Key 的签发 / 列表 / 撤销（从 `lib.rs::gateway_client_keys_*` 抽出）。
//!
//! 语义全在 `crate::gateway::access`：这里只负责把「可选的限额参数」整理成
//! access 期望的形状，并保持「Key 原文只在签发结果里出现一次」这条约束不被绕过。

use rusqlite::Connection;

use crate::gateway::access::{ClientKeySummary, IssuedClientKey};

/// 列出命名客户端 Key：名称 + 掩码 + 限额 + 最近使用与当日计数。
///
/// Key 原文不出后端——列表里只有掩码，原文仅在签发结果里出现一次。
pub fn list(conn: &Connection) -> Result<Vec<ClientKeySummary>, String> {
    crate::gateway::access::client_keys_list(conn)
}

/// 签发一把命名客户端 Key，返回结果里带 Key 原文（只此一次，复制给客户端）。
///
/// `tokens_per_day` / `requests_per_day` 传 None 表示不限；请求数限额即时生效，
/// Token 限额本期只保存不强制（见 `access::check_client_key_limit` 的说明）。
pub fn issue(
    conn: &Connection,
    name: &str,
    tokens_per_day: Option<i64>,
    requests_per_day: Option<i64>,
    expires_ms: Option<i64>,
    models: Option<Vec<String>>,
    read_only: bool,
) -> Result<IssuedClientKey, String> {
    crate::gateway::access::client_key_issue(
        conn, name, tokens_per_day, requests_per_day, expires_ms, models, read_only,
    )
}

/// 撤销命名客户端 Key（删除行，立即失效；流水里的历史归属保留）。
pub fn revoke(conn: &Connection, name: &str) -> Result<(), String> {
    crate::gateway::access::client_key_revoke(conn, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (Connection, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "jolu-keys-svc-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        (crate::db::open(&path).unwrap(), path)
    }

    #[test]
    fn issue_list_revoke_round_trip_keeps_plaintext_out_of_the_list() {
        let (conn, path) = database();
        let issued = issue(&conn, "书房台式机", Some(1000), Some(10), None, None, false).unwrap();

        let rows = list(&conn).unwrap();
        let row = rows.iter().find(|row| row.name == "书房台式机").expect("刚签发的 Key 应在列表里");
        assert_eq!(row.requests_per_day, Some(10));
        // 列表里绝不能有原文
        assert!(!row.masked.contains(&issued.key), "列表只能给掩码：{}", row.masked);

        revoke(&conn, "书房台式机").unwrap();
        assert!(list(&conn).unwrap().is_empty());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_only_flag_is_passed_through() {
        let (conn, path) = database();
        issue(&conn, "只读机", None, None, None, None, true).unwrap();
        let rows = list(&conn).unwrap();
        let row = rows.iter().find(|row| row.name == "只读机").unwrap();
        assert!(row.read_only, "只读标记要存下来");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
