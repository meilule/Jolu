//! 网关总览的组装（从 `lib.rs::gateway_status` 抽出）。
//!
//! 这里只做「把设置与接线记录拼成一份快照」，不读全局状态、不发事件：
//! 设置由调用方取好传进来，接线记录用传入的连接读取。

use rusqlite::Connection;

use crate::gateway::wiring::{self, WiringStatus};
use crate::settings::Settings;

/// 网关总览：模式、端口、模型池与接线状态。凭证不外泄（只给掩码与 slug）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GatewayStatus {
    pub mode: String,
    pub enabled: bool,
    pub port: u16,
    pub circuit_threshold: u32,
    pub auto_escape: bool,
    pub default_connection: Option<String>,
    pub wiring: Vec<WiringStatus>,
}

/// 组装总览快照。
///
/// 接线状态按配置文件里实际写入的值判断（`wiring::status_with`），所以每条都要读一次
/// 接线记录；读不到记录不当错误——那只是「没接过线」，与读库失败在界面上不同，
/// 但这里无法区分两者（`wiring_store::load` 对两者都返回 None），因此按「未接线」处理，
/// 与既有命令的行为保持一致。
pub fn status(settings: &Settings, conn: &Connection) -> GatewayStatus {
    GatewayStatus {
        mode: settings.proxy_mode.clone(),
        enabled: settings.proxy_enabled,
        port: settings.proxy_port,
        circuit_threshold: settings.gateway_circuit_threshold,
        auto_escape: settings.gateway_auto_escape,
        default_connection: settings.gateway_default_connection.clone(),
        wiring: wiring::AGENTS
            .iter()
            .map(|agent| {
                let record = crate::gateway::wiring_store::load(conn, agent);
                unreachable_when_stopped(settings, wiring::status_with(agent, settings.proxy_port, record.as_ref()))
            })
            .collect(),
    }
}

/// 工具已接到网关、但网关没在中转模式运行：配置看起来是对的，工具的请求却全部会失败。
/// 单独标成漂移「网关不可达」，别让「已接入」掩盖它（2.7）。
pub(crate) fn unreachable_when_stopped(settings: &Settings, mut status: wiring::WiringStatus) -> wiring::WiringStatus {
    let (mode, _) = crate::gateway::mode_from(settings);
    let running = settings.proxy_enabled && mode == crate::gateway::Mode::Relay;
    if status.wired && !running {
        status.drift = Some("unreachable".into());
        status.reason = Some("统一网关没有在中转模式运行：这个工具的请求会全部失败。请开启网关，或断开接线恢复直连".into());
    }
    status
}

/// 读不到数据库时的降级快照：设置照给，接线一律按「未接线」报。
///
/// 这不是「静默回退到示例数据」——接线状态本来就是从配置文件实测出来的，
/// 拿不到记录时「未接线」与实际情况一致，界面上也能看出没有接线。
pub fn status_without_db(settings: &Settings) -> GatewayStatus {
    GatewayStatus {
        mode: settings.proxy_mode.clone(),
        enabled: settings.proxy_enabled,
        port: settings.proxy_port,
        circuit_threshold: settings.gateway_circuit_threshold,
        auto_escape: settings.gateway_auto_escape,
        default_connection: settings.gateway_default_connection.clone(),
        wiring: wiring::AGENTS
            .iter()
            .map(|agent| wiring::status_with(agent, settings.proxy_port, None))
            .collect(),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn wired_tools_with_a_stopped_gateway_are_flagged_unreachable() {
        let status = crate::gateway::wiring::WiringStatus {
            agent: "claude-code".into(), wired: true, config_path: None, notice: None, reason: None,
            state: Some("wired".into()), drift: None, codex_mode: None,
        };
        let mut settings = crate::settings::Settings::default();
        settings.proxy_enabled = false;
        let flagged = super::unreachable_when_stopped(&settings, status.clone());
        assert_eq!(flagged.drift.as_deref(), Some("unreachable"));
        let unwired = crate::gateway::wiring::WiringStatus { wired: false, ..status };
        assert_eq!(super::unreachable_when_stopped(&settings, unwired.clone()), unwired, "没接线的不标");
    }
    use super::*;
    use crate::settings::Settings;

    fn memory() -> (Connection, std::path::PathBuf) {
        // `db::open` 负责建表与迁移；用临时文件而不是内存库，保证与真实启动同一套 schema
        let path = std::env::temp_dir().join(format!(
            "jolu-status-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        (crate::db::open(&path).unwrap(), path)
    }

    #[test]
    fn status_carries_settings_values_verbatim() {
        let (conn, path) = memory();
        let mut settings = Settings::default();
        settings.proxy_mode = "relay".into();
        settings.proxy_enabled = true;
        settings.proxy_port = 12731;
        settings.gateway_circuit_threshold = 7;
        settings.gateway_auto_escape = false;
        settings.gateway_default_connection = Some("conn-1".into());

        let status = status(&settings, &conn);
        assert_eq!(status.mode, "relay");
        assert!(status.enabled);
        assert_eq!(status.port, 12731);
        assert_eq!(status.circuit_threshold, 7);
        assert!(!status.auto_escape);
        assert_eq!(status.default_connection.as_deref(), Some("conn-1"));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wiring_covers_every_agent_once() {
        let (conn, path) = memory();
        let status = status(&Settings::default(), &conn);
        assert_eq!(
            status.wiring.len(),
            crate::gateway::wiring::AGENTS.len(),
            "每个 agent 一条，不重不漏"
        );
        let agents: Vec<&str> = status.wiring.iter().map(|w| w.agent.as_str()).collect();
        for agent in crate::gateway::wiring::AGENTS {
            assert!(agents.contains(&agent), "缺少 {agent}");
        }
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn status_is_read_only_and_repeatable() {
        let (conn, path) = memory();
        let settings = Settings::default();
        let first = status(&settings, &conn);
        let second = status(&settings, &conn);
        // 两次快照一致：这个函数不该改任何状态
        assert_eq!(first.wiring.len(), second.wiring.len());
        assert_eq!(
            first.wiring.iter().map(|w| w.wired).collect::<Vec<_>>(),
            second.wiring.iter().map(|w| w.wired).collect::<Vec<_>>()
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
