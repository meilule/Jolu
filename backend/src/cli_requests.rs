//! 命令行请求桌面端代办（5.5：`jolu use` / `jolu save` / `jolu serve`）。
//!
//! 这些操作要走桌面端的整套流程（接线的备份 / 原子写 / 快照、启动网关监听），命令行进程里
//! 没有 Tauri 的 AppHandle 做不了。做法是文件队列：
//! 1. CLI 在数据目录的 `cli-requests/` 下写 `<id>.req.json`（先写临时文件再改名，不会被读到半截）；
//! 2. 桌面端后台线程每秒看一次，把请求改名为 `<id>.working`（认领），执行，写 `<id>.res.json`；
//! 3. CLI 等到结果文件就读出来、删掉、打印。5 秒内请求没被认领 = 桌面端没在运行，CLI 撤回请求并说明。
//!
//! 只接受这里列出的动作；请求里没有密钥（`provider add` 在命令行进程里直接写库，不走这里）。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const DIR: &str = "cli-requests";
/// 能请求桌面端做的事
pub const ACTIONS: &[&str] = &["profile_use", "profile_save", "serve"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub ok: bool,
    pub message: String,
}

fn dir_of(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

fn new_id() -> String {
    format!("{}-{}", chrono::Utc::now().timestamp_millis(), std::process::id())
}

/// CLI 侧：发请求并等结果。`claim_wait` 内没被认领视为桌面端没在运行；`done_wait` 是认领后的最长等待。
pub fn send(data_dir: &Path, action: &str, name: Option<&str>, claim_wait: Duration, done_wait: Duration) -> Result<Response, String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("不支持的请求：{action}"));
    }
    let dir = dir_of(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录 {} 失败：{e}", dir.display()))?;
    let request = Request { id: new_id(), action: action.into(), name: name.map(str::to_string) };
    let req = dir.join(format!("{}.req.json", request.id));
    let res = dir.join(format!("{}.res.json", request.id));
    let tmp = dir.join(format!("{}.tmp", request.id));
    let body = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &req)).map_err(|e| format!("写请求失败：{e}"))?;
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(&res) {
            if let Ok(response) = serde_json::from_str::<Response>(&text) {
                let _ = std::fs::remove_file(&res);
                return Ok(response);
            }
        }
        let claimed = !req.exists();
        let elapsed = started.elapsed();
        if !claimed && elapsed >= claim_wait {
            // 撤回：改名成功说明桌面端确实没认领
            if std::fs::remove_file(&req).is_ok() {
                return Err("Jolu 桌面端没有在运行（请求 5 秒内没人接）：先打开 Jolu 再试".into());
            }
        }
        if elapsed >= claim_wait + done_wait {
            return Err("桌面端接了请求但没在限定时间内完成；稍后在桌面端查看结果".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 桌面端侧：认领并处理所有待办请求。`handle` 执行动作并给出结果。
pub fn process(data_dir: &Path, handle: &dyn Fn(&Request) -> Response) -> usize {
    let dir = dir_of(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return 0 };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".req.json")).map(str::to_string) else { continue };
        let working = dir.join(format!("{id}.working"));
        // 改名认领：CLI 同时撤回时只有一方成功
        if std::fs::rename(&path, &working).is_err() {
            continue;
        }
        let response = match std::fs::read(&working).ok().and_then(|bytes| serde_json::from_slice::<Request>(&bytes).ok()) {
            Some(request) if ACTIONS.contains(&request.action.as_str()) && request.id == id => handle(&request),
            Some(request) => Response { ok: false, message: format!("不支持的请求：{}", request.action) },
            None => Response { ok: false, message: "请求读不出来".into() },
        };
        let _ = std::fs::remove_file(&working);
        let res = dir.join(format!("{id}.res.json"));
        let tmp = dir.join(format!("{id}.res.tmp"));
        if let Ok(body) = serde_json::to_vec(&response) {
            let _ = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &res));
        }
        count += 1;
    }
    // 没人取走的旧结果（CLI 中途被关）留一小时后清掉
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let stale = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(3600));
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    count
}

/// 5.5：执行命令行请求（只有 `cli_requests::ACTIONS` 里的动作会到这里）
pub(crate) fn desktop_handle(app: &tauri::AppHandle, request: &Request) -> Response {
    use tauri::Manager;
    let reply = |result: Result<String, String>| match result {
        Ok(message) => Response { ok: true, message },
        Err(message) => Response { ok: false, message },
    };
    let name = request.name.clone().unwrap_or_default();
    match request.action.as_str() {
        "profile_use" => reply(crate::gateway_profiles_apply(app.clone(), name).map(|result| result.message)),
        "profile_save" => reply(crate::gateway_profiles_save(app.clone(), name).map(|profile| {
            format!("已保存 Profile「{}」：{} 个工具", profile.name, profile.agents.len())
        })),
        "serve" => {
            let settings = app.state::<crate::Cfg>().0.get();
            let (mode, _) = crate::gateway::mode_from(&settings);
            if settings.proxy_enabled && mode == crate::gateway::Mode::Relay {
                return Response { ok: true, message: format!("模型路由已在运行：http://127.0.0.1:{}", settings.proxy_port) };
            }
            let result = crate::gateway_set_mode(app.clone(), "relay".into(), settings.gateway_default_connection.clone())
                .map(|next| format!("已开启模型路由：http://127.0.0.1:{}", next.proxy_port));
            crate::events::settings_changed(app);
            reply(result)
        }
        other => Response { ok: false, message: format!("不支持的请求：{other}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-cli-requests-{}-{name}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn request_round_trip_through_a_desktop_thread() {
        let dir = temp("round");
        let desktop = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                while started.elapsed() < Duration::from_secs(5) {
                    let n = process(&dir, &|req| Response { ok: true, message: format!("{} {}", req.action, req.name.clone().unwrap_or_default()) });
                    if n > 0 {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
        };
        let response = send(&dir, "profile_use", Some("工作"), Duration::from_secs(3), Duration::from_secs(3)).unwrap();
        assert_eq!(response, Response { ok: true, message: "profile_use 工作".into() });
        desktop.join().unwrap();
        assert_eq!(std::fs::read_dir(dir_of(&dir)).unwrap().count(), 0, "请求与结果都清理掉了");
    }

    #[test]
    fn no_desktop_means_the_request_is_withdrawn() {
        let dir = temp("none");
        let err = send(&dir, "serve", None, Duration::from_millis(200), Duration::from_millis(200)).unwrap_err();
        assert!(err.contains("没有在运行"), "{err}");
        assert_eq!(std::fs::read_dir(dir_of(&dir)).unwrap().count(), 0);
        assert!(send(&dir, "rm_rf", None, Duration::ZERO, Duration::ZERO).is_err());
    }

    #[test]
    fn desktop_refuses_unknown_actions_written_by_hand() {
        let dir = temp("forged");
        std::fs::create_dir_all(dir_of(&dir)).unwrap();
        std::fs::write(dir_of(&dir).join("x.req.json"), r#"{"id":"x","action":"delete_everything"}"#).unwrap();
        let called = std::cell::Cell::new(false);
        assert_eq!(process(&dir, &|_| { called.set(true); Response { ok: true, message: String::new() } }), 1);
        assert!(!called.get());
        let res: Response = serde_json::from_str(&std::fs::read_to_string(dir_of(&dir).join("x.res.json")).unwrap()).unwrap();
        assert!(!res.ok);
    }
}
