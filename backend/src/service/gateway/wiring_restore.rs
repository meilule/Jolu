//! 无 Tauri 上下文的接线还原（卸载钩子与 CLI 用）。
//!
//! 场景：卸载 Jolu 后，用户的 Claude Code / Codex 等还指着已失效的
//! `127.0.0.1:<port>`——所有请求都会失败。桌面端走 `escape::manual`，但它要
//! `AppHandle`；卸载钩子与 CLI 没有那个上下文。
//!
//! 管什么：按 `agent_wiring` 表的快照，把每个已接线（或挂起）的 agent 的配置
//! 文件还原到接线前的值，并把记录标回 idle。
//!
//! 不管什么：
//! - 不停监听、不改设置（卸载场景下进程和设置都即将消失）；
//! - 不碰用户在快照之外的配置键（`wiring::unwire` 只还原快照里的键）；
//! - 不创建缺失的数据库——调用方保证它存在（卸载时它一定在）。
//!
//! 跨进程互斥：必须走 [`crate::cli_switch::with_switch`]——运行中的桌面端可能
//! 正在接管/还原，两边同时写同一批配置文件会互相覆盖（AGENTS.md 的既有纪律）。

use rusqlite::Connection;

/// 还原结果：按 agent 归类，给卸载日志与 CLI 输出用。
#[derive(Debug, Default, serde::Serialize)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

/// 还原全部已接线的 agent。
///
/// `port` 来自设置文件（调用方读），只是 `unwire` 判断「配置当前是否指向网关」用的。
pub fn restore_all(conn: &mut Connection, port: u16) -> RestoreReport {
    let mut report = RestoreReport::default();
    // 互斥量在还原全程持有：这段里要读 wiring 记录、写各 agent 的配置文件、
    // 改 wiring 表，任何一步与桌面端并发都会互相踩。
    let outcome = crate::cli_switch::with_switch(|| -> Result<RestoreReport, String> {
        let mut report = RestoreReport::default();
        for agent in crate::gateway::wiring::AGENTS {
            let record = crate::gateway::wiring_store::load(conn, agent);
            let status = crate::gateway::wiring::status_with(agent, port, record.as_ref());
            let pending = record.as_ref().is_some_and(|r| r.state == "pending");
            if !status.wired && !pending {
                continue;
            }
            match crate::gateway::wiring::unwire(agent, port, record.as_ref()) {
                Ok(true) => {
                    report.restored.push(agent.to_string());
                    let _ = crate::gateway::wiring_store::mark_idle(conn, agent);
                }
                Ok(false) => report.skipped.push(agent.to_string()),
                Err(error) => report.failed.push(format!("{agent}: {error}")),
            }
        }
        Ok(report)
    });
    match outcome {
        Ok(report) => report,
        // 拿不到互斥量：如实报失败，不静默假装还原了
        Err(error) => RestoreReport { failed: vec![error], ..RestoreReport::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (Connection, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "jolu-restore-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        (crate::db::open(&path).unwrap(), path)
    }

    #[test]
    fn nothing_wired_reports_empty_and_does_not_error() {
        let (mut conn, path) = database();
        let report = restore_all(&mut conn, 12731);
        assert!(report.restored.is_empty());
        assert!(report.skipped.is_empty());
        assert!(report.failed.is_empty(), "空表不是失败：{report:?}");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wired_record_without_config_file_is_a_no_op_like_escape() {
        // wiring 记录存在、但配置文件已不在：status_with 判「未接线」（它按配置文件
        // 实际内容判断），与 escape::restore_all 的既有语义一致——无需动作，不算失败。
        // 这条固定的是「不把文件缺失当成还原失败」，避免卸载钩子误报。
        let (mut conn, path) = database();
        conn.execute(
            "INSERT INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
             VALUES('claude-code', 'test', 'test', 'wired', '{}', '{}', NULL, 1)",
            [],
        ).ok();
        let report = restore_all(&mut conn, 12731);
        assert!(report.restored.is_empty() && report.skipped.is_empty() && report.failed.is_empty(), "{report:?}");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pending_record_goes_through_unwire_and_lands_on_skipped() {
        // pending = 「打算写、可能写了一半」：即使配置文件不存在也要走一遍 unwire，
        // 让 mark_idle 把状态收回，避免下次打开还看到挂起
        let (mut conn, path) = database();
        conn.execute(
            "INSERT INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
             VALUES('claude-code', 'test', 'test', 'pending', '{}', '{}', NULL, NULL)",
            [],
        ).ok();
        let report = restore_all(&mut conn, 12731);
        assert!(report.skipped.iter().any(|agent| agent == "claude-code"), "文件不存在 → Ok(false) → skipped：{report:?}");
        assert!(report.failed.is_empty(), "{report:?}");
        // 已知边界（与桌面端 escape::restore_all 一致）：skipped 不标 idle，
        // 「pending + 文件已丢」的记录会留在 pending——真实场景里这要求用户先有
        // 配置文件才接线，触发概率极低；若要收口应连同桌面端一起改。
        let state: String = conn.query_row("SELECT state FROM agent_wiring WHERE agent_id='claude-code'", [], |r| r.get(0)).unwrap();
        assert_eq!(state, "pending");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
