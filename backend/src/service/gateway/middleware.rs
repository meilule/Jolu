//! 网关中间件的配置入口（列表 / 保存 / 删除）。
//!
//! 执行语义全在 `crate::gateway::middleware`（选路前按顺序改写请求体，失败隔离）。
//! 这里负责「写进库之前先确认配置真能跑」：执行链遇到坏配置只会静默跳过，
//! 如果保存时不校验，用户会以为开了中间件，实际一次都没生效。

use rusqlite::Connection;
use serde_json::Value;

use crate::gateway::middleware::{self as chain, Middleware};

/// 内置中间件类型（与 `gateway::middleware::apply_chain` 的分支一一对应）。
pub const KINDS: [&str; 5] = ["model-map", "param-override", "system-prompt", "word-guard", "think-tags"];

/// 用来试跑请求侧中间件的样例请求（Anthropic 形态，含 system 与一条用户消息）。
fn sample_request() -> Value {
    serde_json::json!({
        "model": "sample/model",
        "system": "sample",
        "messages": [{ "role": "user", "content": "sample" }],
    })
}

/// 校验名称、类型与配置：名称 1–64 个可见字符；类型必须是内置之一；
/// 请求侧中间件在样例请求上试跑一次，被执行链判为「跳过」的配置直接拒绝保存。
pub fn validate(name: &str, kind: &str, config: &Value) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err("中间件名称需为 1–64 个可见字符".into());
    }
    if !KINDS.contains(&kind) {
        return Err(format!("未知的中间件类型「{kind}」，可选：{}", KINDS.join(" / ")));
    }
    if !config.is_object() {
        return Err("中间件配置必须是 JSON 对象".into());
    }
    if kind == "think-tags" {
        return match config.get("mode").and_then(Value::as_str).unwrap_or("strip") {
            "strip" | "wrap" => Ok(()),
            other => Err(format!("think-tags 的 mode 只能是 strip 或 wrap，收到「{other}」")),
        };
    }
    if kind == "param-override" && config.get("set").is_none() && config.get("delete").is_none() {
        return Err("param-override 至少要有 set 或 delete 之一".into());
    }
    let probe = Middleware { name: name.into(), kind: kind.into(), config: config.clone(), enabled: true, sort: 0 };
    let outcome = chain::apply_chain(&[probe], &mut sample_request());
    match outcome.skipped.first() {
        Some(reason) => Err(format!("配置无效：{reason}")),
        None => Ok(()),
    }
}

pub fn list(conn: &Connection) -> Result<Vec<Middleware>, String> {
    chain::list(conn).map_err(|_| "无法读取网关中间件".to_string())
}

/// 新增或覆盖一个中间件（按名称幂等）；先校验，校验不过不写库。
pub fn save(
    conn: &Connection,
    name: &str,
    kind: &str,
    config: Value,
    enabled: bool,
    sort: i64,
) -> Result<Middleware, String> {
    validate(name, kind, &config)?;
    let middleware = Middleware { name: name.trim().into(), kind: kind.into(), config, enabled, sort };
    chain::set(conn, &middleware)?;
    Ok(middleware)
}

pub fn delete(conn: &Connection, name: &str) -> Result<(), String> {
    if chain::delete(conn, name)? {
        Ok(())
    } else {
        Err(format!("中间件「{name}」不存在"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn database() -> (Connection, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "jolu-middleware-svc-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        (crate::db::open(&path).unwrap(), path)
    }

    #[test]
    fn every_builtin_kind_accepts_a_working_config() {
        let cases = [
            ("model-map", json!({"mapping": {"fast": "zcode/GLM-5.2"}})),
            ("param-override", json!({"set": {"max_tokens": 1024}})),
            ("param-override", json!({"delete": ["temperature"]})),
            ("system-prompt", json!({"prompt": "始终用中文回答"})),
            ("word-guard", json!({"block": ["机密项目"]})),
            ("word-guard", json!({"mask": ["1234"]})),
            ("think-tags", json!({"mode": "strip"})),
            ("think-tags", json!({})),
        ];
        for (kind, config) in cases {
            assert!(validate("规则", kind, &config).is_ok(), "{kind} {config}");
        }
    }

    #[test]
    fn broken_configs_are_rejected_before_they_reach_the_chain() {
        let rejected = [
            ("", "model-map", json!({"mapping": {}})),
            ("规则", "unknown", json!({})),
            ("规则", "model-map", json!([])),
            ("规则", "model-map", json!({})),
            ("规则", "param-override", json!({})),
            ("规则", "param-override", json!({"set": 1})),
            ("规则", "system-prompt", json!({"prompt": "   "})),
            ("规则", "word-guard", json!({})),
            ("规则", "think-tags", json!({"mode": "keep"})),
        ];
        for (name, kind, config) in rejected {
            assert!(validate(name, kind, &config).is_err(), "{name:?} {kind} {config}");
        }
    }

    #[test]
    fn save_list_delete_round_trip_keeps_order_and_reports_missing() {
        let (conn, path) = database();
        save(&conn, "改名", "model-map", json!({"mapping": {"fast": "a/b"}}), true, 1).unwrap();
        save(&conn, "中文", "system-prompt", json!({"prompt": "中文"}), false, 0).unwrap();
        assert!(save(&conn, "坏的", "word-guard", json!({}), true, 2).is_err(), "校验不过不写库");
        let names: Vec<_> = list(&conn).unwrap().into_iter().map(|m| (m.name, m.enabled)).collect();
        assert_eq!(names, vec![("中文".to_string(), false), ("改名".to_string(), true)]);
        delete(&conn, "改名").unwrap();
        assert!(delete(&conn, "改名").is_err());
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
