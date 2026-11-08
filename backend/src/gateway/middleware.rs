//! 网关中间件链（P6.1，对齐 magpie 的 gateway middleware 语义）：
//! 按配置顺序对请求体做改写，失败隔离——单个中间件出错只跳过自己，请求原样继续。
//!
//! 首发两个内置：
//! - `model-map`：模型改名映射（New API 同款语义）——`{"mapping": {"fast": "来源/模型"}}`，
//!   精确命中（忽略大小写）即改写请求体 model；这是「任何模型名映射」的正解。
//! - `param-override`：顶层参数覆盖——`{"set": {字段: 值}, "delete": [字段]}`。
//!
//! 与路由的先后：中间件在**选路之前**执行（agent 发的原始名字先被映射，再进入
//! P1.1 的解析链），映射后的 id 就是路由、流水与用量记账的 id（magpie 同款：
//! 「the one id its routing, stats and usage rows know it by」）。
//!
//! word-guard / system-prompt / think-tags 为后续内置（本模块留扩展位）：
//! 它们触碰消息内容，与 P5 隐私脱敏的执行顺序要先定义（脱敏最先）。

use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Middleware {
    pub name: String,
    pub kind: String,
    pub config: Value,
    pub enabled: bool,
    pub sort: i64,
}

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Middleware>> {
    let mut stmt = conn.prepare(
        "SELECT name, kind, config_json, enabled, sort FROM gateway_middleware ORDER BY sort ASC, name ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (name, kind, config_json, enabled, sort) = row?;
        match serde_json::from_str::<Value>(&config_json) {
            Ok(config) => out.push(Middleware { name, kind, config, enabled: enabled != 0, sort }),
            Err(error) => eprintln!("[中间件] 「{name}」的配置无效，已跳过：{error}"),
        }
    }
    Ok(out)
}

/// 新增 / 覆盖一个中间件（按 name 幂等）。
pub(crate) fn set(conn: &Connection, middleware: &Middleware) -> Result<(), String> {
    conn.execute(
        "INSERT INTO gateway_middleware(name, kind, config_json, enabled, sort) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(name) DO UPDATE SET kind = excluded.kind, config_json = excluded.config_json, enabled = excluded.enabled, sort = excluded.sort",
        rusqlite::params![middleware.name, middleware.kind, middleware.config.to_string(), middleware.enabled as i64, middleware.sort],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn delete(conn: &Connection, name: &str) -> Result<bool, String> {
    let changed = conn
        .execute("DELETE FROM gateway_middleware WHERE name = ?1", [name])
        .map_err(|error| error.to_string())?;
    Ok(changed > 0)
}

/// 中间件链的执行结果：映射前后的模型名（流水与用量记账用）与跳过原因。
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    /// model-map 命中时的「原始名 → 映射名」；未命中为 None
    pub mapped_from: Option<String>,
    /// 被隔离跳过的中间件（配置无效 / 执行出错），trace 用
    pub skipped: Vec<String>,
}

/// 按顺序执行启用的中间件，原地改写请求体。
///
/// 纪律（magpie 同款）：单个中间件失败**只跳过自己**，链上其余照常；model-map 只改
/// `model` 字段，不碰工具声明与 stream（客户端按它读回复）。
pub(crate) fn apply_chain(middlewares: &[Middleware], body: &mut Value) -> Outcome {
    let mut outcome = Outcome::default();
    for middleware in middlewares {
        if !middleware.enabled {
            continue;
        }
        let result = match middleware.kind.as_str() {
            "model-map" => apply_model_map(&middleware.config, body, &mut outcome),
            "param-override" => apply_param_override(&middleware.config, body),
            _ => Err(format!("未知中间件类型「{}」", middleware.kind)),
        };
        if let Err(error) = result {
            outcome.skipped.push(format!("{}（{error}）", middleware.name));
        }
    }
    outcome
}

/// model-map：`{"mapping": {"请求名": "真实名"}}`，忽略大小写精确命中。
fn apply_model_map(config: &Value, body: &mut Value, outcome: &mut Outcome) -> Result<(), String> {
    let Some(mapping) = config.get("mapping").and_then(Value::as_object) else {
        return Err("model-map 配置缺少 mapping 表".into());
    };
    let Some(current) = body.get("model").and_then(Value::as_str) else {
        return Ok(()); // 请求没有 model 字段：本中间件无事可做，不算失败
    };
    let Some(target) = mapping
        .iter()
        .find(|(from, _)| from.as_str().eq_ignore_ascii_case(current))
        .and_then(|(_, to)| to.as_str())
    else {
        return Ok(()); // 映射表没这个名字：原样放行（裸名解析与默认来源接着处理）
    };
    if target.trim().is_empty() {
        return Err("model-map 的目标模型名为空".into());
    }
    outcome.mapped_from = Some(current.to_string());
    body["model"] = Value::String(target.trim().to_string());
    Ok(())
}

/// param-override：`{"set": {字段: 值}, "delete": [字段]}`，只动顶层字段。
fn apply_param_override(config: &Value, body: &mut Value) -> Result<(), String> {
    let object = body.as_object_mut().ok_or("请求体顶层不是对象")?;
    // set / delete 键存在但类型不对 = 配置错误（静默当 no-op 会掩盖写错的配置）
    if let Some(set) = config.get("set") {
        let Some(set) = set.as_object() else { return Err("param-override 的 set 不是对象".into()) };
        for (field, value) in set {
            object.insert(field.clone(), value.clone());
        }
    }
    if let Some(delete) = config.get("delete") {
        let Some(delete) = delete.as_array() else { return Err("param-override 的 delete 不是数组".into()) };
        for field in delete {
            if let Some(name) = field.as_str() {
                object.remove(name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn middleware(kind: &str, config: Value) -> Middleware {
        Middleware { name: kind.into(), kind: kind.into(), config, enabled: true, sort: 0 }
    }

    #[test]
    fn model_map_rewrites_exact_case_insensitive_and_records_from() {
        let m = middleware("model-map", json!({"mapping": {"fast": "zcode/glm-5.3-flash"}}));
        let mut body = json!({"model": "Fast", "max_tokens": 8});
        let outcome = apply_chain(&[m.clone()], &mut body);
        assert_eq!(body["model"], "zcode/glm-5.3-flash");
        assert_eq!(outcome.mapped_from.as_deref(), Some("Fast"));
        // 未映射的名字原样放行
        let mut body = json!({"model": "other", "max_tokens": 8});
        let outcome = apply_chain(&[m], &mut body);
        assert_eq!(body["model"], "other");
        assert!(outcome.mapped_from.is_none());
    }

    #[test]
    fn model_map_rejects_empty_target_without_touching_body() {
        let m = middleware("model-map", json!({"mapping": {"fast": "  "}}));
        let mut body = json!({"model": "fast"});
        let outcome = apply_chain(&[m], &mut body);
        assert_eq!(body["model"], "fast", "失败的中间件不改写请求");
        assert!(outcome.skipped.len() == 1, "失败要进 skipped：{:?}", outcome.skipped);
    }

    #[test]
    fn param_override_sets_and_deletes_top_level_fields() {
        let m = middleware(
            "param-override",
            json!({"set": {"max_tokens": 512, "temperature": 0.2}, "delete": ["top_p"]}),
        );
        let mut body = json!({"model": "m", "top_p": 0.9});
        apply_chain(&[m], &mut body);
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["temperature"], 0.2);
        assert!(body.get("top_p").is_none());
    }

    #[test]
    fn chain_applies_in_order_and_failure_isolates() {
        // 坏中间件夹在中间：它被跳过，后面的照常执行
        let chain = [
            middleware("model-map", json!({"mapping": {"fast": "zcode/glm-5.3-flash"}})),
            middleware("param-override", json!({"set": 1})), // set 不是对象 → 失败
            middleware("param-override", json!({"set": {"max_tokens": 32}})),
        ];
        let mut body = json!({"model": "fast"});
        let outcome = apply_chain(&chain, &mut body);
        assert_eq!(body["model"], "zcode/glm-5.3-flash");
        assert_eq!(body["max_tokens"], 32, "失败之后的中间件照常执行");
        assert_eq!(outcome.skipped.len(), 1);
    }

    #[test]
    fn disabled_and_unknown_middlewares_are_skipped() {
        let mut disabled = middleware("model-map", json!({"mapping": {"fast": "x/y"}}));
        disabled.enabled = false;
        let unknown = middleware("what", json!({}));
        let mut body = json!({"model": "fast"});
        let outcome = apply_chain(&[disabled, unknown], &mut body);
        assert_eq!(body["model"], "fast", "禁用的不执行");
        assert_eq!(outcome.skipped.len(), 1, "未知类型进 skipped");
    }

    #[test]
    fn store_round_trip_keeps_config_and_order() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS gateway_middleware (
                name TEXT PRIMARY KEY, kind TEXT NOT NULL, config_json TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1, sort INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        let mut first = middleware("model-map", json!({"mapping": {}}));
        first.sort = 0;
        set(&conn, &first).unwrap();
        let mut second = middleware("param-override", json!({"set": {}}));
        second.sort = 1;
        set(&conn, &second).unwrap();
        let listed = list(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "model-map");
        assert!(listed[0].enabled);
        delete(&conn, "model-map").unwrap();
        assert!(list(&conn).unwrap().len() == 1);
    }
}
