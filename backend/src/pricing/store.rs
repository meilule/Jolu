//! 用户价目覆盖、同价映射与连接计费配置的 SQLite 存取。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const GLOBAL_SCOPE: &str = "*";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricingOverride {
    pub scope: String,
    pub model_id: String,
    #[serde(default)]
    pub display_name: String,
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub cache_write_1h: Option<f64>,
    #[serde(default)]
    pub long_ctx_threshold: Option<i64>,
    #[serde(default)]
    pub long_ctx_input: Option<f64>,
    #[serde(default)]
    pub long_ctx_output: Option<f64>,
    #[serde(default)]
    pub long_ctx_cache_read: Option<f64>,
    #[serde(default)]
    pub long_ctx_cache_write: Option<f64>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricingAlias {
    pub scope: String,
    pub alias_model: String,
    pub target_model: String,
    #[serde(default)]
    pub updated_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ConnectionBilling {
    pub id: String,
    pub kind: String,
    pub billing_mode: String,
    pub price_multiplier: f64,
}

fn validate_non_neg(label: &str, value: f64) -> Result<f64, String> {
    if !value.is_finite() || value < 0.0 || value > 1_000_000.0 {
        return Err(format!("{label} 单价须为 0 到 1,000,000 之间的有效数字"));
    }
    Ok(value)
}

fn validate_opt_non_neg(label: &str, value: Option<f64>) -> Result<Option<f64>, String> {
    match value {
        Some(v) => validate_non_neg(label, v).map(Some),
        None => Ok(None),
    }
}

pub fn normalize_scope(scope: &str) -> Result<String, String> {
    let trimmed = scope.trim();
    if trimmed.is_empty() {
        return Err("作用域不能为空".into());
    }
    Ok(trimmed.to_string())
}

pub fn normalize_model_key(model_id: &str, allow_wildcard: bool) -> Result<String, String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return Err("模型 ID 不能为空".into());
    }
    if trimmed == "*" {
        if !allow_wildcard {
            return Err("全局价目不允许使用通配符 *".into());
        }
        return Ok("*".into());
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub fn list_overrides(conn: &Connection) -> rusqlite::Result<Vec<PricingOverride>> {
    let mut stmt = conn.prepare(
        "SELECT scope, model_id, display_name, input, output, cache_read, cache_write, cache_write_1h,
                long_ctx_threshold, long_ctx_input, long_ctx_output, long_ctx_cache_read, long_ctx_cache_write,
                disabled, updated_ms
         FROM pricing_overrides
         ORDER BY scope ASC, model_id ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PricingOverride {
            scope: r.get(0)?,
            model_id: r.get(1)?,
            display_name: r.get(2)?,
            input: r.get(3)?,
            output: r.get(4)?,
            cache_read: r.get(5)?,
            cache_write: r.get(6)?,
            cache_write_1h: r.get(7)?,
            long_ctx_threshold: r.get(8)?,
            long_ctx_input: r.get(9)?,
            long_ctx_output: r.get(10)?,
            long_ctx_cache_read: r.get(11)?,
            long_ctx_cache_write: r.get(12)?,
            disabled: r.get::<_, i64>(13)? != 0,
            updated_ms: r.get(14)?,
        })
    })?;
    rows.collect()
}

pub fn upsert_override(conn: &Connection, mut item: PricingOverride) -> Result<PricingOverride, String> {
    item.scope = normalize_scope(&item.scope)?;
    let allow_wildcard = item.scope != GLOBAL_SCOPE;
    item.model_id = normalize_model_key(&item.model_id, allow_wildcard)?;
    item.display_name = item.display_name.trim().to_string();
    item.input = validate_non_neg("输入", item.input)?;
    item.output = validate_non_neg("输出", item.output)?;
    item.cache_read = validate_non_neg("缓存读", item.cache_read)?;
    item.cache_write = validate_opt_non_neg("缓存写", item.cache_write)?;
    item.cache_write_1h = validate_opt_non_neg("1h 缓存写", item.cache_write_1h)?;
    if let Some(threshold) = item.long_ctx_threshold {
        if threshold <= 0 {
            return Err("长上下文阈值须大于 0".into());
        }
        item.long_ctx_input = validate_opt_non_neg("长上下文输入", item.long_ctx_input)?;
        item.long_ctx_output = validate_opt_non_neg("长上下文输出", item.long_ctx_output)?;
        item.long_ctx_cache_read = validate_opt_non_neg("长上下文缓存读", item.long_ctx_cache_read)?;
        item.long_ctx_cache_write = validate_opt_non_neg("长上下文缓存写", item.long_ctx_cache_write)?;
    } else {
        item.long_ctx_input = None;
        item.long_ctx_output = None;
        item.long_ctx_cache_read = None;
        item.long_ctx_cache_write = None;
    }
    item.updated_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO pricing_overrides(
            scope, model_id, display_name, input, output, cache_read, cache_write, cache_write_1h,
            long_ctx_threshold, long_ctx_input, long_ctx_output, long_ctx_cache_read, long_ctx_cache_write,
            disabled, updated_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(scope, model_id) DO UPDATE SET
            display_name = excluded.display_name,
            input = excluded.input,
            output = excluded.output,
            cache_read = excluded.cache_read,
            cache_write = excluded.cache_write,
            cache_write_1h = excluded.cache_write_1h,
            long_ctx_threshold = excluded.long_ctx_threshold,
            long_ctx_input = excluded.long_ctx_input,
            long_ctx_output = excluded.long_ctx_output,
            long_ctx_cache_read = excluded.long_ctx_cache_read,
            long_ctx_cache_write = excluded.long_ctx_cache_write,
            disabled = excluded.disabled,
            updated_ms = excluded.updated_ms",
        params![
            item.scope,
            item.model_id,
            item.display_name,
            item.input,
            item.output,
            item.cache_read,
            item.cache_write,
            item.cache_write_1h,
            item.long_ctx_threshold,
            item.long_ctx_input,
            item.long_ctx_output,
            item.long_ctx_cache_read,
            item.long_ctx_cache_write,
            item.disabled as i64,
            item.updated_ms,
        ],
    )
    .map_err(|e| format!("保存模型价目失败：{e}"))?;
    Ok(item)
}

/// 删除一条用户覆盖记录（若原模型有内置或 models.dev 价目，即恢复为默认目录价）。
pub fn delete_override(conn: &Connection, scope: &str, model_id: &str) -> Result<(), String> {
    let scope = normalize_scope(scope)?;
    let model_id = normalize_model_key(model_id, scope != GLOBAL_SCOPE)?;
    conn.execute(
        "DELETE FROM pricing_overrides WHERE scope = ?1 AND model_id = ?2",
        params![scope, model_id],
    )
    .map_err(|e| format!("删除模型价目失败：{e}"))?;
    Ok(())
}

/// 标记模型为禁用（墓碑）：即使内置或 models.dev 有该模型，也不再计价。
pub fn disable_override(conn: &Connection, scope: &str, model_id: &str) -> Result<(), String> {
    let scope = normalize_scope(scope)?;
    let model_id = normalize_model_key(model_id, false)?;
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO pricing_overrides(scope, model_id, disabled, updated_ms)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(scope, model_id) DO UPDATE SET disabled = 1, updated_ms = excluded.updated_ms",
        params![scope, model_id, now],
    )
    .map_err(|e| format!("停用模型价目失败：{e}"))?;
    Ok(())
}

pub fn list_aliases(conn: &Connection) -> rusqlite::Result<Vec<PricingAlias>> {
    let mut stmt = conn.prepare(
        "SELECT scope, alias_model, target_model, updated_ms
         FROM pricing_aliases
         ORDER BY scope ASC, alias_model ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PricingAlias {
            scope: r.get(0)?,
            alias_model: r.get(1)?,
            target_model: r.get(2)?,
            updated_ms: r.get(3)?,
        })
    })?;
    rows.collect()
}

pub fn upsert_alias(conn: &Connection, scope: &str, alias_model: &str, target_model: &str) -> Result<PricingAlias, String> {
    let scope = normalize_scope(scope)?;
    let alias_model = normalize_model_key(alias_model, false)?;
    let target_model = normalize_model_key(target_model, false)?;
    if alias_model == target_model {
        return Err("别名模型与目标模型不能相同".into());
    }
    let updated_ms = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO pricing_aliases(scope, alias_model, target_model, updated_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(scope, alias_model) DO UPDATE SET
            target_model = excluded.target_model,
            updated_ms = excluded.updated_ms",
        params![scope, alias_model, target_model, updated_ms],
    )
    .map_err(|e| format!("保存同价映射失败：{e}"))?;
    Ok(PricingAlias { scope, alias_model, target_model, updated_ms })
}

pub fn delete_alias(conn: &Connection, scope: &str, alias_model: &str) -> Result<(), String> {
    let scope = normalize_scope(scope)?;
    let alias_model = normalize_model_key(alias_model, false)?;
    conn.execute(
        "DELETE FROM pricing_aliases WHERE scope = ?1 AND alias_model = ?2",
        params![scope, alias_model],
    )
    .map_err(|e| format!("删除同价映射失败：{e}"))?;
    Ok(())
}

pub fn load_connection_billing(conn: &Connection) -> rusqlite::Result<HashMap<String, ConnectionBilling>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, billing_mode, price_multiplier FROM connections",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ConnectionBilling {
            id: r.get(0)?,
            kind: r.get(1)?,
            billing_mode: r.get::<_, Option<String>>(2)?.unwrap_or_else(|| "auto".into()),
            price_multiplier: r.get::<_, Option<f64>>(3)?.unwrap_or(1.0),
        })
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let item = row?;
        out.insert(item.id.clone(), item);
    }
    Ok(out)
}
