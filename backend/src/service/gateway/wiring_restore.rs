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
    // 互斥量在还原全程持有：这段里要读 wiring 记录、写各 agent 的配置文件、
    // 改 wiring 表，任何一步与桌面端并发都会互相踩。
    let outcome = crate::cli_switch::with_switch(|| -> Result<RestoreReport, String> {
        let mut report = RestoreReport::default();
        for agent in crate::gateway::wiring::agents() {
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

    /// 在隔离的空用户目录子进程中运行测试，绝不读写开发机真实的 `~/.claude` / `~/.codex` 等配置文件。
    fn run_isolated(test_name: &str, body: impl FnOnce(&std::path::Path)) {
        const ENV_KEY: &str = "JOLU_WIRING_RESTORE_ISOLATED_DIR";
        if let Some(root) = std::env::var_os(ENV_KEY) {
            body(std::path::Path::new(&root));
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "jolu-restore-iso-{}-{test_name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        let appdata = root.join("appdata");
        let localappdata = root.join("localappdata");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&appdata).unwrap();
        std::fs::create_dir_all(&localappdata).unwrap();

        let exact = format!("{}::{test_name}", module_path!().trim_start_matches("jolu_lib::"));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &exact, "--nocapture"])
            .env(ENV_KEY, &root)
            .env("USERPROFILE", &home)
            .env("HOME", &home)
            .env("APPDATA", &appdata)
            .env("LOCALAPPDATA", &localappdata)
            .env("XDG_CONFIG_HOME", &appdata)
            .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
            .env("CODEX_HOME", home.join(".codex"))
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            output.status.success(),
            "isolated {test_name} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn database(root: &std::path::Path) -> Connection {
        crate::db::open(&root.join("usage.db")).unwrap()
    }

    #[test]
    fn an_empty_table_does_not_fail() {
        run_isolated("an_empty_table_does_not_fail", |root| {
            let mut conn = database(root);
            let report = restore_all(&mut conn, 12731);
            assert!(
                report.restored.is_empty() && report.skipped.is_empty() && report.failed.is_empty(),
                "空隔离目录与空记录表不应有任何动作或失败：{report:?}"
            );
        });
    }

    #[test]
    fn wired_record_without_config_file_is_a_no_op_like_escape() {
        run_isolated("wired_record_without_config_file_is_a_no_op_like_escape", |root| {
            let mut conn = database(root);
            conn.execute(
                "INSERT INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
                 VALUES('claude-code', 'test', 'test', 'wired', '{}', '{}', NULL, 1)",
                [],
            )
            .ok();
            let report = restore_all(&mut conn, 12731);
            assert!(
                report.restored.is_empty() && report.skipped.is_empty() && report.failed.is_empty(),
                "{report:?}"
            );
        });
    }

    #[test]
    fn pending_record_goes_through_unwire_and_lands_on_skipped() {
        run_isolated("pending_record_goes_through_unwire_and_lands_on_skipped", |root| {
            let mut conn = database(root);
            conn.execute(
                "INSERT INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
                 VALUES('claude-code', 'test', 'test', 'pending', '{}', '{}', NULL, NULL)",
                [],
            )
            .ok();
            let report = restore_all(&mut conn, 12731);
            assert!(
                report.skipped.iter().any(|agent| agent == "claude-code"),
                "文件不存在 → Ok(false) → skipped：{report:?}"
            );
            assert!(report.failed.is_empty(), "{report:?}");
            let state: String = conn
                .query_row(
                    "SELECT state FROM agent_wiring WHERE agent_id='claude-code'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "pending");
        });
    }
}
