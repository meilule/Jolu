//! `jolu claude run`（5.5，参照 magpie `cli/claude_launch_cli.go`）：让这一次启动的 Claude Code
//! 走 Jolu 的模型路由，用户自己的 `~/.claude/settings.json` 和直接敲的 `claude` 都不变。
//!
//! 做法：在数据目录写一份启动用设置 `claude-launch.json`（env 指向本机网关 + 令牌，可选模型），
//! 用 `claude --settings <文件>` 启动。`--settings` 属于 flag 来源，优先级高于 user / project / local
//! （Claude Code `SettingsSource`：user < project < local < flag < policy，Context7 核对）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::settings::Settings;

pub const LAUNCH_FILE: &str = "claude-launch.json";

/// 启动用设置的内容。令牌用接线同款 `jolu-claude-code.<实例>`，网关据此把请求归到 Claude Code。
pub fn claude_launch_settings(settings: &Settings, model: Option<&str>) -> Value {
    let port = if settings.proxy_port > 0 { settings.proxy_port } else { 12731 };
    let instance: String = settings.instance_id.chars().take(8).collect();
    let mut env = json!({
        "ANTHROPIC_BASE_URL": format!("http://127.0.0.1:{port}"),
        "ANTHROPIC_AUTH_TOKEN": format!("jolu-claude-code.{instance}"),
    });
    let mut root = json!({});
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        // 顶层 model 是 /model 的初值；env 的 ANTHROPIC_MODEL 同写，压过用户自己 env 里的同名键
        env["ANTHROPIC_MODEL"] = json!(model);
        root["model"] = json!(model);
    }
    root["env"] = env;
    root
}

/// 模型路由是否在本机可连：设置里开着转发模式，且端口真有人在听
pub fn gateway_ready(settings: &Settings) -> Result<(), String> {
    let (mode, _) = crate::gateway::mode_from(settings);
    if !settings.proxy_enabled || mode != crate::gateway::Mode::Relay {
        return Err("模型路由没有开启：先在 Jolu 桌面端打开「模型路由」".into());
    }
    let port = if settings.proxy_port > 0 { settings.proxy_port } else { 12731 };
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&address, Duration::from_millis(800))
        .map(|_| ())
        .map_err(|_| format!("127.0.0.1:{port} 上没有模型路由在监听：确认 Jolu 桌面端正在运行"))
}

pub struct ClaudeLaunch {
    pub exe: PathBuf,
    pub settings_file: PathBuf,
}

/// 准备一次启动：检查网关、找到 claude、写好启动用设置。
pub fn prepare_claude(directory: &Path, model: Option<&str>) -> Result<ClaudeLaunch, String> {
    let settings = crate::service::settings::read_settings(directory);
    gateway_ready(&settings)?;
    let home = crate::creds::home().ok_or("找不到用户目录")?;
    let exe = crate::agent_discovery::find_bin("claude", &home).ok_or("没找到 claude 命令：先安装 Claude Code")?;
    let settings_file = directory.join(LAUNCH_FILE);
    let text = serde_json::to_vec_pretty(&claude_launch_settings(&settings, model)).map_err(|e| e.to_string())?;
    std::fs::write(&settings_file, text).map_err(|e| format!("写 {} 失败：{e}", settings_file.display()))?;
    Ok(ClaudeLaunch { exe, settings_file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_settings_point_at_gateway_and_only_add_model_when_given() {
        let mut settings = Settings::default();
        settings.proxy_port = 15000;
        settings.instance_id = "abcdef1234567890".into();
        let plain = claude_launch_settings(&settings, None);
        assert_eq!(plain["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:15000");
        assert_eq!(plain["env"]["ANTHROPIC_AUTH_TOKEN"], "jolu-claude-code.abcdef12");
        assert!(plain.get("model").is_none() && plain["env"].get("ANTHROPIC_MODEL").is_none());
        let with = claude_launch_settings(&settings, Some(" kimi/k2 "));
        assert_eq!(with["model"], "kimi/k2");
        assert_eq!(with["env"]["ANTHROPIC_MODEL"], "kimi/k2");
    }

    #[test]
    fn gateway_ready_explains_why_not() {
        let mut settings = Settings::default();
        settings.proxy_enabled = false;
        assert!(gateway_ready(&settings).unwrap_err().contains("没有开启"));
        settings.proxy_enabled = true;
        settings.proxy_mode = "relay".into();
        // 找一个没人听的端口：先绑再放
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        settings.proxy_port = port;
        assert!(gateway_ready(&settings).unwrap_err().contains("没有模型路由在监听"));
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        settings.proxy_port = listener.local_addr().unwrap().port();
        assert!(gateway_ready(&settings).is_ok());
    }
}
