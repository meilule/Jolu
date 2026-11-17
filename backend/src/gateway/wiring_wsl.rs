//! WSL 里的工具接线（5.6 / P2-6，参照 magpie `internal/agent/wsl.go`）。无画板，范围待确认。
//!
//! - 支持 Claude Code、Codex、OpenCode（配置格式最简单的三家），复用 `wiring.rs` 的纯函数，
//!   只是文件换成发行版里的那份：经 `\\wsl.localhost\<发行版>\<家目录>\…` 读写。
//! - 网关地址看 WSL 的网络模式：mirrored（`wslinfo --networking-mode`，或 `.wslconfig` 写了
//!   `networkingMode=mirrored`）下发行版的 127.0.0.1 就是 Windows 的，和本机接线一样；
//!   NAT 下只能经 Windows 的主机地址（发行版默认路由的网关）访问，网关得监听回环以外的地址——
//!   那就是「局域网共享」，非本机来源的请求必须带命名客户端 Key。所以 NAT 下**只在用户自己
//!   打开了局域网共享时**才接：Jolu 给这一个工具签一把专用 Key（`wsl-<发行版>-<工具>`），断开时撤销；
//!   没开就拒绝并说明两条路（改 mirrored，或自己开局域网共享）。Jolu 不替用户打开局域网共享。
//! - 记账在 `wsl_wiring` 表；快照存接线前的值，断开时文件里仍是我们写的地址才还原。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::wiring;

pub(crate) const WSL_AGENTS: [&str; 3] = ["claude-code", "codex", "opencode"];

pub(crate) fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wsl_wiring (
            distro      TEXT NOT NULL,
            agent       TEXT NOT NULL,
            path        TEXT NOT NULL,
            base        TEXT NOT NULL,
            client_key  TEXT,
            snapshot    TEXT,
            wired_ms    INTEGER NOT NULL,
            PRIMARY KEY (distro, agent)
        );",
    )
}

/// 相对家目录的配置文件
pub(crate) fn relative_config(agent: &str) -> Option<&'static str> {
    Some(match agent {
        "claude-code" => ".claude/settings.json",
        "codex" => ".codex/config.toml",
        "opencode" => ".config/opencode/opencode.json",
        _ => return None,
    })
}

/// 发行版里的 Linux 路径 → Windows 能打开的 UNC 路径
pub(crate) fn unc(distro: &str, linux_path: &str) -> PathBuf {
    PathBuf::from(format!(r"\\wsl.localhost\{distro}{}", linux_path.replace('/', "\\")))
}

/// 发行版的网络情况
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DistroNet {
    pub distro: String,
    pub home: String,
    /// mirrored / nat / virtioproxy / unknown …（`wslinfo` 的原话；没有 wslinfo 时按 .wslconfig）
    pub mode: String,
    /// NAT 下 Windows 在发行版眼里的地址（默认路由的网关）
    pub host_ip: Option<String>,
}

/// 能不能经 127.0.0.1 到达 Windows 上的网关
pub(crate) fn loopback_reaches_windows(mode: &str) -> bool {
    matches!(mode, "mirrored" | "virtioproxy")
}

/// `.wslconfig` 里 `[wsl2]` 下的 `networkingMode=mirrored`
pub(crate) fn wslconfig_mirrored(text: &str) -> bool {
    let mut in_wsl2 = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_wsl2 = line.eq_ignore_ascii_case("[wsl2]");
            continue;
        }
        if !in_wsl2 {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case("networkingMode") && value.trim().trim_matches('"').eq_ignore_ascii_case("mirrored") {
                return true;
            }
        }
    }
    false
}

/// 解析探测脚本的输出（三行：家目录、网络模式、默认网关）
pub(crate) fn parse_net(distro: &str, text: &str, wslconfig: Option<&str>) -> Result<DistroNet, String> {
    let mut lines = text.lines().map(str::trim);
    let home = lines.next().filter(|h| h.starts_with('/')).ok_or("读不到发行版里的家目录")?.to_string();
    let mut mode = lines.next().unwrap_or("").to_ascii_lowercase();
    if mode.is_empty() || mode == "unknown" || mode.contains(' ') {
        mode = if wslconfig.is_some_and(wslconfig_mirrored) { "mirrored".into() } else { "unknown".into() };
    }
    let host_ip = lines.next().filter(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok()).map(str::to_string);
    Ok(DistroNet { distro: distro.into(), home, mode, host_ip })
}

#[cfg(windows)]
pub(crate) fn probe_net(distro: &str) -> Result<DistroNet, String> {
    let exe = crate::session_threads::find_executable("wsl").ok_or("未检测到 wsl.exe")?;
    let script = "printf '%s\\n' \"$HOME\"; (wslinfo --networking-mode 2>/dev/null || echo unknown); ip route show default 2>/dev/null | head -n 1 | cut -d \" \" -f 3";
    // 脚本里不要用 `$1` 一类字段变量：wsl.exe 转交参数时会再展开一次（awk 的 `$3` 会变成空）
    let mut command = std::process::Command::new(exe);
    command.args(["-d", distro, "--", "sh", "-lc", script]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| format!("无法执行 wsl.exe：{e}"))?;
    crate::proc::wait_or_kill_tree(&mut child, std::time::Duration::from_secs(10), std::time::Duration::from_millis(30)).ok_or("wsl.exe 执行超时")?;
    let mut out = Vec::new();
    use std::io::Read;
    child.stdout.take().ok_or("读不到 wsl.exe 输出")?.read_to_end(&mut out).map_err(|e| e.to_string())?;
    let wslconfig = crate::creds::home().and_then(|home| std::fs::read_to_string(home.join(".wslconfig")).ok());
    parse_net(distro, &String::from_utf8_lossy(&out), wslconfig.as_deref())
}

#[cfg(not(windows))]
pub(crate) fn probe_net(_distro: &str) -> Result<DistroNet, String> {
    Err("只有 Windows 上有 WSL".into())
}

/// 这次接线用的地址与令牌来源
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    /// mirrored：和本机一样走 127.0.0.1，令牌 `jolu-<agent>.<实例>`
    Loopback { base: String },
    /// NAT + 局域网共享：主机地址 + 专用客户端 Key（名字）
    Host { base: String, key_name: String },
}

pub(crate) fn route_for(net: &DistroNet, agent: &str, port: u16, lan_shared: bool) -> Result<Route, String> {
    if loopback_reaches_windows(&net.mode) {
        return Ok(Route::Loopback { base: format!("http://127.0.0.1:{port}") });
    }
    let advice = "两条路任选：在 %UserProfile%\\.wslconfig 的 [wsl2] 下写 networkingMode=mirrored 后执行 wsl --shutdown；或在「模型路由 › 对外访问」打开局域网共享（Jolu 会给这个工具签一把专用 Key，还要允许 Windows 防火墙放行 WSL）";
    if !lan_shared {
        return Err(format!("WSL「{}」是 {} 网络，发行版里的 127.0.0.1 到不了 Windows 上的模型路由。{advice}", net.distro, net.mode));
    }
    let ip = net.host_ip.as_deref().ok_or_else(|| format!("读不到 WSL「{}」里 Windows 的地址（默认路由）。{advice}", net.distro))?;
    Ok(Route::Host { base: format!("http://{ip}:{port}"), key_name: format!("wsl-{}-{agent}", net.distro.to_ascii_lowercase()) })
}

/// 接线后的文件内容与快照（纯函数）
pub(crate) fn wire_text(agent: &str, original: &str, base: &str, token: &str) -> Result<(String, Option<serde_json::Value>), String> {
    match agent {
        "claude-code" => {
            let snapshot = wiring::claude_snapshot(original, None, None, &[]);
            let text = if original.trim().is_empty() { "{}" } else { original };
            let next = wiring::claude_wire(text, base, token, None, None, None, None, None, &|_| (None, None), &wiring::ClaudeExtras::default())?;
            Ok((next, snapshot))
        }
        // WSL 里不判断 ChatGPT 登录：一律新增 Jolu provider + 静态令牌（断开时按快照还原）
        "codex" => Ok((wiring::codex_wire(original, base, wiring::CODEX_PROVIDER, false, Some(token))?, wiring::codex_snapshot(original))),
        "opencode" => Ok((wiring::opencode_wire(original, base, token)?, None)),
        _ => Err(format!("WSL 里暂不支持接入 {agent}")),
    }
}

pub(crate) fn current_base(agent: &str, text: &str) -> Option<String> {
    match agent {
        "claude-code" => wiring::claude_current_base(text),
        "codex" => wiring::codex_effective_base(text),
        "opencode" => wiring::opencode_current_base(text),
        _ => None,
    }
}

pub(crate) fn unwire_text(agent: &str, current: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    match agent {
        "claude-code" => wiring::claude_restore(current, saved, None, None, &[]),
        "codex" => wiring::codex_restore(current, saved, wiring::CODEX_PROVIDER),
        "opencode" => wiring::opencode_restore(current, saved),
        _ => Err(format!("WSL 里暂不支持接入 {agent}")),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WslWiringRow {
    pub distro: String,
    pub agent: String,
    pub path: String,
    pub base: String,
    /// NAT 下签的专用客户端 Key 名
    pub client_key: Option<String>,
    /// 文件里还是我们写的地址
    pub intact: bool,
    pub wired_ms: i64,
}

pub(crate) fn list(conn: &Connection) -> Vec<WslWiringRow> {
    let Ok(mut stmt) = conn.prepare("SELECT distro, agent, path, base, client_key, wired_ms FROM wsl_wiring ORDER BY distro, agent") else { return Vec::new() };
    let rows: Vec<(String, String, String, String, Option<String>, i64)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).map(|rows| rows.flatten().collect()).unwrap_or_default();
    rows.into_iter()
        .map(|(distro, agent, path, base, client_key, wired_ms)| {
            let intact = crate::steady::read_to_string(Path::new(&path)).ok().and_then(|text| current_base(&agent, &text)).is_some_and(|now| now.trim_end_matches('/').starts_with(base.trim_end_matches('/')));
            WslWiringRow { distro, agent, path, base, client_key, intact, wired_ms }
        })
        .collect()
}

/// 接入：`path` 是 UNC 路径，`token` 是这次用的令牌。写前备份到 `backup_dir`。
pub(crate) fn wire_at(conn: &Connection, distro: &str, agent: &str, path: &Path, base: &str, token: &str, client_key: Option<&str>, backup_dir: &Path, now: i64) -> Result<(), String> {
    let original = crate::steady::read_optional(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?.unwrap_or_default();
    // 已经接过：沿用旧快照，不把自己写的值当原值
    let prior: Option<String> = conn
        .query_row("SELECT snapshot FROM wsl_wiring WHERE distro = ?1 AND agent = ?2", params![distro, agent], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let (next, snapshot) = wire_text(agent, &original, base, token)?;
    let snapshot = prior.or_else(|| snapshot.map(|s| s.to_string()));
    wiring::backup(path, backup_dir)?;
    wiring::write_json_atomic_public(path, next.into_bytes())?;
    conn.execute(
        "INSERT INTO wsl_wiring(distro, agent, path, base, client_key, snapshot, wired_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(distro, agent) DO UPDATE SET path = excluded.path, base = excluded.base, client_key = excluded.client_key, snapshot = excluded.snapshot, wired_ms = excluded.wired_ms",
        params![distro, agent, path.display().to_string(), base, client_key, snapshot, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 断开：文件里还是我们写的地址才还原；返回（是否改了文件，要撤销的客户端 Key 名）。
pub(crate) fn unwire(conn: &Connection, distro: &str, agent: &str, backup_dir: &Path) -> Result<(bool, Option<String>), String> {
    let row: Option<(String, String, Option<String>, Option<String>)> = conn
        .query_row("SELECT path, base, client_key, snapshot FROM wsl_wiring WHERE distro = ?1 AND agent = ?2", params![distro, agent], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((path, base, client_key, snapshot)) = row else { return Err("这个工具在 WSL 里没有接线记录".into()) };
    let path = PathBuf::from(path);
    let mut changed = false;
    if let Some(current) = crate::steady::read_optional(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))? {
        let ours = current_base(agent, &current).is_some_and(|now| now.trim_end_matches('/').starts_with(base.trim_end_matches('/')));
        if ours {
            let saved = snapshot.and_then(|s| serde_json::from_str(&s).ok());
            let next = unwire_text(agent, &current, saved.as_ref())?;
            wiring::backup(&path, backup_dir)?;
            wiring::write_json_atomic_public(&path, next.into_bytes())?;
            changed = true;
        }
    }
    conn.execute("DELETE FROM wsl_wiring WHERE distro = ?1 AND agent = ?2", params![distro, agent]).map_err(|e| e.to_string())?;
    Ok((changed, client_key))
}

/// 卸载钩子：断开全部 WSL 接线并撤销专用 Key（发行版没在运行时文件读不到，就只清记录）
pub(crate) fn restore_all(conn: &Connection, backup_dir: &Path) -> Vec<String> {
    list(conn)
        .into_iter()
        .map(|row| match unwire(conn, &row.distro, &row.agent, backup_dir) {
            Ok((changed, key)) => {
                if let Some(key) = key {
                    let _ = crate::gateway::access::client_key_revoke(conn, &key);
                }
                format!("WSL {} 的 {}：{}", row.distro, row.agent, if changed { "已还原" } else { "文件已不是 Jolu 写的，只清记录" })
            }
            Err(error) => format!("失败：WSL {} 的 {}：{error}", row.distro, row.agent),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_mode_parsing_and_wslconfig_fallback() {
        let nat = parse_net("Ubuntu", "/root\nnat\n172.20.0.1\n", None).unwrap();
        assert_eq!((nat.home.as_str(), nat.mode.as_str(), nat.host_ip.as_deref()), ("/root", "nat", Some("172.20.0.1")));
        let old = parse_net("Ubuntu", "/home/u\nunknown\n\n", Some("[wsl2]\nnetworkingMode = mirrored\n")).unwrap();
        assert_eq!(old.mode, "mirrored", "没有 wslinfo 时按 .wslconfig");
        assert!(!wslconfig_mirrored("[other]\nnetworkingMode=mirrored\n"), "只认 [wsl2] 段");
        assert!(parse_net("Ubuntu", "\n", None).is_err());
        assert_eq!(unc("Ubuntu", "/root/.claude/settings.json"), PathBuf::from(r"\\wsl.localhost\Ubuntu\root\.claude\settings.json"));
    }

    #[test]
    fn route_needs_mirrored_or_user_enabled_lan_sharing() {
        let net = |mode: &str| DistroNet { distro: "Ubuntu".into(), home: "/root".into(), mode: mode.into(), host_ip: Some("172.20.0.1".into()) };
        assert_eq!(route_for(&net("mirrored"), "codex", 12731, false).unwrap(), Route::Loopback { base: "http://127.0.0.1:12731".into() });
        let refused = route_for(&net("nat"), "codex", 12731, false).unwrap_err();
        assert!(refused.contains("mirrored") && refused.contains("局域网共享"), "{refused}");
        assert_eq!(
            route_for(&net("nat"), "claude-code", 12731, true).unwrap(),
            Route::Host { base: "http://172.20.0.1:12731".into(), key_name: "wsl-ubuntu-claude-code".into() }
        );
        let mut no_ip = net("nat");
        no_ip.host_ip = None;
        assert!(route_for(&no_ip, "codex", 12731, true).is_err());
    }

    #[test]
    fn wire_and_unwire_round_trip_for_each_agent() {
        let dir = std::env::temp_dir().join(format!("jolu-wsl-wiring-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let originals = [
            ("claude-code", "settings.json", r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.example.com","ANTHROPIC_AUTH_TOKEN":"sk-mine"},"theme":"dark"}"#),
            ("codex", "config.toml", "model = \"gpt-5\"\n"),
            ("opencode", "opencode.json", r#"{"theme":"x"}"#),
        ];
        for (agent, file, original) in originals {
            let path = dir.join(file);
            std::fs::write(&path, original).unwrap();
            wire_at(&conn, "Ubuntu", agent, &path, "http://172.20.0.1:12731", "jolu-client-abc", Some("wsl-ubuntu-x"), &dir.join("bak"), 1).unwrap();
            let wired = std::fs::read_to_string(&path).unwrap();
            assert!(current_base(agent, &wired).is_some_and(|b| b.starts_with("http://172.20.0.1:12731")), "{agent}: {wired}");
            // 再接一次不把自己写的当原值
            wire_at(&conn, "Ubuntu", agent, &path, "http://172.20.0.1:12731", "jolu-client-abc", Some("wsl-ubuntu-x"), &dir.join("bak"), 2).unwrap();
            assert!(list(&conn).iter().any(|r| r.agent == agent && r.intact));
            let (changed, key) = unwire(&conn, "Ubuntu", agent, &dir.join("bak")).unwrap();
            assert!(changed);
            assert_eq!(key.as_deref(), Some("wsl-ubuntu-x"));
            let restored = std::fs::read_to_string(&path).unwrap();
            assert!(current_base(agent, &restored).is_none_or(|b| !b.contains("172.20.0.1")), "{agent} 还原后不再指向网关：{restored}");
        }
        // Claude 的原值回来了
        let claude: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(claude["env"]["ANTHROPIC_BASE_URL"], "https://api.example.com");
        assert_eq!(claude["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-mine");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 真机：本机 WSL 默认发行版里探测网络并经 UNC 往返写 /tmp 下的文件（不碰家目录）。
    /// `JOLU_WSL_DISTRO=Ubuntu cargo test … wiring_wsl -- --ignored`
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn real_wsl_round_trip_under_tmp() {
        let Ok(distro) = std::env::var("JOLU_WSL_DISTRO") else { return };
        let net = probe_net(&distro).unwrap();
        eprintln!("网络：{net:?}");
        assert!(net.home.starts_with('/'));
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let path = unc(&distro, "/tmp/jolu-wsl-test/opencode.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"theme":"mine"}"#).unwrap();
        let backup = std::env::temp_dir().join("jolu-wsl-real-bak");
        wire_at(&conn, &distro, "opencode", &path, "http://127.0.0.1:12731", "jolu-opencode.test0000", None, &backup, 1).unwrap();
        assert!(list(&conn)[0].intact);
        assert!(unwire(&conn, &distro, "opencode", &backup).unwrap().0);
        let back: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back["theme"], "mine");
        assert!(back["provider"].get("jolu").is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unwire_leaves_a_file_the_user_repointed() {
        let dir = std::env::temp_dir().join(format!("jolu-wsl-wiring-user-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let path = dir.join("opencode.json");
        wire_at(&conn, "Ubuntu", "opencode", &path, "http://127.0.0.1:12731", "jolu-opencode.abcd1234", None, &dir.join("bak"), 1).unwrap();
        let repointed = std::fs::read_to_string(&path).unwrap().replace("http://127.0.0.1:12731", "https://elsewhere");
        std::fs::write(&path, &repointed).unwrap();
        assert!(!list(&conn)[0].intact);
        let (changed, _) = unwire(&conn, "Ubuntu", "opencode", &dir.join("bak")).unwrap();
        assert!(!changed, "用户改走了就不动");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), repointed);
        assert!(list(&conn).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
