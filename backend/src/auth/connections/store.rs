use super::*;
use rusqlite::{params, Connection as Sql, OptionalExtension};

/// 按凭证原值找连接（只在后端比对）；同一凭证重复保存时取最早的一条。
pub fn find_by_secret(conn: &Sql, platform: &str, kind: &str, secret: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM connections WHERE platform=?1 AND kind=?2 AND secret=?3 ORDER BY created_ms LIMIT 1",
        params![platform, kind, secret],
        |r| r.get(0),
    ).optional().map_err(|e| e.to_string())
}

/// 先选择本机实际来源，再匹配地址与密钥；未登记的 API 不能回退命中残留 OAuth。
pub fn find_current_local(conn: &Sql, local: &crate::current_connection::LocalConnection) -> Result<Option<String>, String> {
    let normalize = |base: Option<String>| normalized_base(&local.platform, &local.kind,
        base.map(|value| value.trim().trim_end_matches('/').to_string()).filter(|value| !value.is_empty()));
    let expected = normalize(local.base_url.clone());
    let mut statement = conn.prepare("SELECT id, base_url FROM connections WHERE platform=?1 AND kind=?2 AND secret=?3 ORDER BY created_ms, id")
        .map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![local.platform, local.kind, local.secret], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, base) = row.map_err(|error| error.to_string())?;
        if normalize(base) == expected { return Ok(Some(id)); }
    }
    Ok(None)
}

pub fn register_local(conn: &Sql, input: NewConnection) -> Result<(String, bool), String> {
    let c = prepare_new(input)?;
    let mut stmt = conn.prepare("SELECT id, base_url FROM connections WHERE platform=?1 AND kind=?2 AND secret=?3").map_err(|e| e.to_string())?;
    let matches = stmt.query_map(params![c.platform, c.kind, c.secret], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    let existing = matches.into_iter().find(|(_, base)| normalized_base(&c.platform, &c.kind, base.clone()) == c.base_url).map(|(id, _)| id);
    if let Some(id) = existing {
        if c.kind == "auth" { crate::auth_snapshot::capture(conn, &id, &c.platform, c.secret.as_deref().unwrap_or_default())?; }
        return Ok((id, false));
    }
    let id = add(conn, c)?;
    conn.execute("UPDATE connections SET status='paused', last_sync_ms=NULL WHERE id=?1", [&id]).map_err(|e| e.to_string())?;
    Ok((id, true))
}

pub fn selectable_creds(conn: &Sql, id: &str) -> Result<Option<Creds>, String> {
    let status: Option<String> = conn.query_row("SELECT status FROM connections WHERE id=?1", [id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    if status.as_deref() != Some("connected") { return Err("请先连接成功，再启用到灵动岛".into()); }
    creds_of(conn, id).map_err(|e| e.to_string())
}


/// 取出连接的凭证与地址，**仅供后端查额度/余额使用**。
/// 返回值含凭证原值，绝不可序列化给前端或写进日志。
#[derive(Clone, PartialEq)]
pub struct Creds {
    pub paused: bool,
    pub name: String,
    pub platform: String,
    pub kind: String,
    pub secret: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

pub fn creds_of(conn: &Sql, id: &str) -> rusqlite::Result<Option<Creds>> {
    conn.query_row(
        "SELECT name, platform, kind, secret, base_url, status, model FROM connections WHERE id = ?1",
        params![id],
        |r| {
            let platform: String = r.get(1)?;
            let kind: String = r.get(2)?;
            let base_url = normalized_base(&platform, &kind, r.get(4)?);
            Ok(Creds {
                paused: r.get::<_, String>(5)? == "paused",
                name: r.get(0)?,
                platform: r.get(1)?,
                kind: r.get(2)?,
                secret: r.get(3)?,
                base_url,
                model: r.get(6)?,
            })
        },
    )
    .optional()
}

/// 本地代理按凭证归账用：API Key 连接的 (id, platform, secret)。只在内存里做哈希比对，不返回给前端。
pub fn api_key_secrets(conn: &Sql) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare("SELECT id, platform, secret FROM connections WHERE kind = 'api' AND secret IS NOT NULL AND secret != ''")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 所有远程查询在命中缓存或发送请求之前，都必须经过此检查。
pub fn query_creds(conn: &Sql, id: &str) -> Result<Option<Creds>, String> {
    let current = creds_of(conn, id).map_err(|e| e.to_string())?;
    if current.as_ref().is_some_and(|c| c.paused) {
        return Err("连接已断开，请先重新连接".into());
    }
    Ok(current)
}

/// 记录一次成功查询的时间，用于「最近同步」列
pub fn touch_sync(conn: &Sql, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE connections SET last_sync_ms = ?2, status = 'connected' WHERE id = ?1 AND status != 'paused'",
        params![id, chrono::Local::now().timestamp_millis()],
    )?;
    Ok(())
}

/// 把连接标记为凭证失效，供额度查询返回 401 时使用
pub fn mark_expired(conn: &Sql, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE connections SET status = 'expired' WHERE id = ?1 AND status != 'paused'",
        params![id],
    )?;
    Ok(())
}

pub fn record_query_status(conn: &Sql, id: &str, success: bool) -> Result<bool, String> {
    let old: Option<String> = conn.query_row("SELECT status FROM connections WHERE id=?1", [id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    if old.is_none() || old.as_deref() == Some("paused") { return Ok(false); }
    if success { touch_sync(conn, id) } else { mark_expired(conn, id) }.map_err(|e| e.to_string())?;
    Ok(old.as_deref() != Some(if success { "connected" } else { "expired" }))
}

/// 应在同一次 with_database 闭包（持有数据库锁）中校验查询归属并写入状态。
/// 临时失败/不支持（outcome=None）也必须校验，避免将旧凭证结果返回当前连接。
/// 名称不参与查询，允许请求期间改名；此快照比较不能检测先改后恢复的 ABA 变更。
pub fn record_query_result(conn: &Sql, id: &str, expected: &Creds, outcome: Option<bool>) -> Result<bool, String> {
    let current = query_creds(conn, id)
        .map_err(|reason| format!("本次查询结果无法应用：{reason}"))?
        .ok_or("连接已删除，本次查询结果已过期，请刷新")?;
    if current.paused != expected.paused
        || current.platform != expected.platform
        || current.kind != expected.kind
        || current.secret != expected.secret
        || current.base_url != expected.base_url
        || current.model != expected.model
    {
        return Err("连接凭证或查询目标已变化，本次查询结果已过期，请刷新".into());
    }
    outcome.map_or(Ok(false), |success| record_query_status(conn, id, success))
}

pub fn replace_api_key(conn: &Sql, id: &str, secret: &str) -> Result<(), String> {
    let changed = conn.execute(
        "UPDATE connections
         SET secret = ?2, masked = ?3, status = 'connected', last_sync_ms = ?4
         WHERE id = ?1 AND kind = 'api' AND status != 'paused'",
        params![id, secret, mask(secret), chrono::Local::now().timestamp_millis()],
    ).map_err(|e| e.to_string())?;
    if changed == 0 { return Err("API Key 连接不存在".into()); }
    refresh_account_fp(conn, id).map_err(|e| e.to_string())?;
    Ok(())
}

/// 改菜单 id（slug）：保留字与字符集校验；旧 id 写进历史，404 时能说明现在叫什么。
pub fn set_slug(conn: &Sql, id: &str, slug: &str) -> Result<String, String> {
    let slug = slug.trim().to_ascii_lowercase();
    if !crate::gateway::identity::valid_slug(&slug) {
        return Err("菜单 id 只能使用小写字母、数字和连字符，不能以连字符开头，也不能是 route / auto / jolu / group / default".into());
    }
    let current: Option<(Option<String>, Option<String>)> = conn
        .query_row("SELECT slug, account_fp FROM connections WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((old_slug, account_fp)) = current else {
        return Err("连接不存在".into());
    };
    if old_slug.as_deref() == Some(slug.as_str()) {
        return Ok(slug);
    }
    let taken: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM connections WHERE slug = ?1 AND id != ?2)",
            params![slug, id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if taken {
        return Err(format!("菜单 id「{slug}」已被其他来源使用"));
    }
    let historical: Option<String> = conn
        .query_row("SELECT connection_id FROM slug_history WHERE slug = ?1", [&slug], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(owner) = historical {
        if owner != id {
            return Err(format!("菜单 id「{slug}」属于另一个已删除的来源，不能复用"));
        }
        conn.execute("DELETE FROM slug_history WHERE slug = ?1", [&slug]).map_err(|e| e.to_string())?;
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    if let Some(old) = old_slug.filter(|old| !old.is_empty()) {
        tx.execute(
            "INSERT OR REPLACE INTO slug_history(slug, connection_id, account_fp, retired_ms) VALUES(?1, ?2, ?3, ?4)",
            params![old, id, account_fp, chrono::Local::now().timestamp_millis()],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.execute("UPDATE connections SET slug = ?2 WHERE id = ?1", params![id, slug]).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(slug)
}

/// 重命名连接：只改显示名称，slug、凭证、脱敏标识与状态一概不动。
/// 纯元数据操作，断开的连接同样允许改名。
pub fn rename_connection(conn: &Sql, id: &str, name: &str) -> Result<(), String> {
    let name = validate_name(name)?;
    let changed = conn
        .execute("UPDATE connections SET name = ?2 WHERE id = ?1", params![id, name])
        .map_err(|e| e.to_string())?;
    if changed == 0 { return Err("连接不存在".into()); }
    Ok(())
}

/// 统一编辑（画布 18）：更新 API 连接的网关地址与默认参数（模型/思考强度/1M）。
/// 地址传入前须已经过 normalized_base 归一；名称与凭证走各自的函数。
pub fn update_params(
    conn: &Sql,
    id: &str,
    base_url: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    context_1m: Option<bool>,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE connections SET base_url = ?2, model = ?3, effort = ?4, context_1m = ?5
             WHERE id = ?1 AND kind = 'api'",
            params![id, base_url, model, effort, context_1m],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 { return Err("API Key 连接不存在".into()); }
    Ok(())
}

/// API Key 可手填服务名称；订阅只采用匹配凭证的官方账号或已探测的来源名称。
pub fn with_custom_name(
    inputs: Vec<NewConnection>,
    name: Option<String>,
) -> Result<Vec<NewConnection>, String> {
    inputs.into_iter().map(|mut input| {
        if input.kind == "auth" {
            if let Some(account) = input.secret.as_deref().and_then(|secret| crate::login_detect::account_for_secret(&input.platform, secret)) {
                input.name = account.chars().take(80).collect();
            }
        } else if let Some(name) = name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
            input.name = validate_name(name)?;
        }
        Ok(input)
    }).collect()
}
