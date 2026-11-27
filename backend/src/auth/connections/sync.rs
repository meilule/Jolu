use super::*;
use rusqlite::{params, Connection as Sql, OptionalExtension};
use serde::Serialize;

/// 「获取」的结果（§6.3 的四种状态里，后端只负责成功/未找到）
#[derive(Debug, Serialize)]
pub struct FetchResult {
    pub ok: bool,
    /// 成功时为新的脱敏标识
    pub masked: Option<String>,
    /// 失败原因，直接作为界面提示文案
    pub message: String,
    /// 扫描过的位置，界面需要说明读取范围，不静默读取
    pub scanned: Vec<String>,
    /// 本次是否真的写入了变更；「已更新」与「已是最新」靠它区分
    pub changed: bool,
}

/// 本机当前应用的凭证与地址，是「更新」与自动同步共用的最小变更集。
#[derive(Debug, Clone)]
pub struct LocalPair {
    pub secret: String,
    pub base_url: Option<String>,
}

/// 把本机当前配置原位应用到一个已有连接上。
///
/// local 为 None 表示本机没有读到凭证：只读不写，原状态与凭证保持不变。
/// 更新范围是 secret、masked 与 base_url——cc-switch 切换中转时两者一起变，
/// 只刷 Key 会把新 Key 发往旧网关。同平台同类型有多条连接且凭证原值对不上
/// 时拒绝覆盖，让用户显式选择，绝不静默改错行。
pub fn apply_local(
    conn: &Sql,
    id: &str,
    local: Option<LocalPair>,
    scanned: Vec<String>,
) -> rusqlite::Result<FetchResult> {
    let row: Option<(String, String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT platform, kind, secret, base_url FROM connections WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((platform, kind, current_secret, current_base)) = row else {
        return Ok(FetchResult { ok: false, masked: None, message: "连接不存在".into(), scanned, changed: false });
    };
    // 失败不得清空或破坏现有连接：这里只读不写
    let Some(local) = local else {
        return Ok(FetchResult { ok: false, masked: None, message: "未找到本机凭证".into(), scanned, changed: false });
    };
    let masked = mask(&local.secret);
    // 与 prepare_new 同一清洗口径：去首尾空白、去结尾斜杠，官方地址归一成 None
    let cleaned_base = local
        .base_url
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    let base_url = normalized_base(&platform, &kind, cleaned_base);
    let same_kind_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM connections WHERE platform = ?1 AND kind = ?2",
        params![platform, kind],
        |row| row.get(0),
    )?;
    if same_kind_count > 1 && current_secret.is_some() && current_secret.as_deref() != Some(local.secret.as_str())
        && !(kind == "auth" && matches!(platform.as_str(), "codex" | "claude")) {
        return Ok(FetchResult {
            ok: false,
            masked: Some(masked),
            message: "本机当前登录账号与此命名连接不同，已拒绝覆盖；请新增连接或切回对应 CLI 账号".into(),
            scanned,
            changed: false,
        });
    }
    if kind == "auth" && matches!(platform.as_str(), "codex" | "claude")
        && !crate::auth_snapshot::can_refresh(conn, id, &platform, current_secret.as_deref(), &local.secret)
    {
        return Ok(FetchResult { ok: false, masked: Some(masked),
            message: "无法确认本机登录仍属于此账号，已保留原账号；请新增连接，或切回该账号后重新同步完整授权".into(),
            scanned, changed: false });
    }
    let changed = current_secret.as_deref() != Some(local.secret.as_str())
        || normalized_base(&platform, &kind, current_base) != base_url;
    if changed {
        let now = chrono::Local::now().timestamp_millis();
        let email = if kind == "auth" { crate::login_detect::account_for_secret(&platform, &local.secret) } else { None };
        conn.execute_batch("SAVEPOINT jolu_refresh_connection")?;
        let result = (|| -> rusqlite::Result<()> {
        conn.execute(
            "UPDATE connections
                SET secret = ?2, masked = ?3, base_url = ?4, status = 'connected', last_sync_ms = ?5,
                    account_email = CASE WHEN secret = ?2 THEN account_email ELSE ?6 END
              WHERE id = ?1",
            params![id, local.secret, masked, base_url, now, email],
        )?;
        refresh_account_fp(conn, id)?;
        if kind == "auth" && platform == "codex" {
            let captured = crate::auth_snapshot::capture(conn, id, &platform, &local.secret)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error))))?;
            if !captured { return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other("本机授权在同步期间发生变化，请重试")))); }
        }
        Ok(())
        })();
        if let Err(error) = result {
            conn.execute_batch("ROLLBACK TO jolu_refresh_connection; RELEASE jolu_refresh_connection")?;
            return Err(error);
        }
        conn.execute_batch("RELEASE jolu_refresh_connection")?;
    }
    Ok(FetchResult {
        ok: true,
        masked: Some(masked),
        message: if changed { "已更新凭证".into() } else { "本机凭证未变化".into() },
        scanned,
        changed,
    })
}

/// 「更新」按钮：重读本机配置刷新**已存在**连接的 secret 与 base_url。
/// 不新建连接、不改名称与类型（§6.3）。
pub fn fetch_local(conn: &Sql, id: &str) -> rusqlite::Result<FetchResult> {
    if creds_of(conn, id)?.is_some_and(|c| c.paused) {
        return Ok(FetchResult { ok: false, masked: None, message: "连接已断开，请先重新连接".into(), scanned: vec![], changed: false });
    }
    let row: Option<(String, String)> = conn
        .query_row("SELECT platform, kind FROM connections WHERE id = ?1", params![id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((platform, kind)) = row else {
        return Ok(FetchResult { ok: false, masked: None, message: "连接不存在".into(), scanned: vec![], changed: false });
    };
    let scanned = crate::creds::scanned_locations(&platform, &kind);
    let local = crate::creds::read_local_pair(&platform, &kind)
        .map(|(secret, base_url)| LocalPair { secret, base_url });
    apply_local(conn, id, local, scanned)
}

/// 自动同步（默认常开）：把本机当前应用的配置原位写回已有连接。
///
/// 只处理无歧义的连接：同平台同类型恰好一条时才尝试原位更新；本机没读到、
/// 多条同类型（分不清本机配置属于哪条）时一律不动。paused 的连接会被
/// apply_local 拒绝，不会被打断或改写。返回是否有实际变更，供调用方
/// 决定是否推送事件。
pub fn sync_from_local_with(
    conn: &Sql,
    read: &dyn Fn(&str, &str) -> Option<LocalPair>,
) -> Result<bool, String> {
    let rows: Vec<(String, String, String)> = conn
        .prepare("SELECT id, platform, kind FROM connections")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?;
    let mut changed = false;
    for (id, platform, kind) in rows {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM connections WHERE platform = ?1 AND kind = ?2",
                params![platform, kind],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count != 1 { continue; }
        let Some(local) = read(&platform, &kind) else { continue };
        let result = apply_local(conn, &id, Some(local), vec![]).map_err(|e| e.to_string())?;
        changed |= result.changed;
    }
    Ok(changed)
}

/// 自动同步入口：读本机真实配置。应用启动时与配置文件变化时各调一次。
pub fn sync_from_local(conn: &Sql) -> Result<bool, String> {
    sync_from_local_with(conn, &|platform, kind| {
        crate::creds::read_local_pair(platform, kind)
            .map(|(secret, base_url)| LocalPair { secret, base_url })
    })
}
