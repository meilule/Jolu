//! 本机当前生效凭证映射（2B 实时入库打标 + 3A 外部切号与陌生 Key 防呆 + 本地 OAuth 切号缓存刷新）。
//!
//! 安全边界：
//! - 明文 `secret` 只在本模块函数栈内用于匹配 `connections`，**绝不进入结构体、缓存或事件载荷**；
//! - 跨进程与缓存只保留 `masked`（如 `sk-ant-****9f2a` 或已核实邮箱 / 登录类型标签）、匹配状态与已保存连接的 id / 名称。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::agent_configs::LOCAL_OAUTH_PREFIX;
use crate::current_connection::LocalConnection;

/// 某平台在本机磁盘配置上当前生效的账号状态（可直接发给前端，不含任何明文凭据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalActiveAccount {
    pub platform: String,
    /// `"auth"` | `"api"`
    pub kind: String,
    /// `"matched"`（命中已存连接）| `"unregistered"`（本机生效但未录入 Jolu）| `"gateway_wired"`（已接入 Jolu 网关）
    pub state: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub masked: String,
    pub account_email: Option<String>,
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    /// 仅供后端内部识别本地 OAuth 换号（如同一 `monitor:native:zcode:account` 下切换了底层登录人），不发给前端
    #[serde(skip)]
    pub identity_tag: Option<String>,
    pub updated_ms: i64,
}

static ACTIVE_MAP: OnceLock<Mutex<BTreeMap<String, LocalActiveAccount>>> = OnceLock::new();

fn active_cell() -> &'static Mutex<BTreeMap<String, LocalActiveAccount>> {
    ACTIVE_MAP.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn same_identity(a: &LocalActiveAccount, b: &LocalActiveAccount) -> bool {
    a.platform == b.platform
        && a.kind == b.kind
        && a.state == b.state
        && a.connection_id == b.connection_id
        && a.connection_name == b.connection_name
        && a.masked == b.masked
        && a.account_email == b.account_email
        && a.base_url == b.base_url
        && a.source_agent == b.source_agent
        && a.identity_tag == b.identity_tag
}

fn find_connection_meta(conn: &Connection, id: &str) -> Option<(String, String, Option<String>)> {
    conn.query_row(
        "SELECT platform, name, account_email FROM connections WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .ok()
    .flatten()
}

fn canonical_upstream_url(base: Option<&str>, platform: &str) -> Option<String> {
    if let Some(raw) = base.map(str::trim).filter(|s| !s.is_empty()) {
        let mut trimmed = raw.trim_end_matches('/').to_ascii_lowercase();
        if let Some(without_v1) = trimmed.strip_suffix("/v1") {
            trimmed = without_v1.to_string();
        }
        return Some(trimmed);
    }
    match platform {
        "claude" => Some("https://api.anthropic.com".into()),
        "codex" => Some("https://api.openai.com".into()),
        "gemini" => Some("https://generativelanguage.googleapis.com".into()),
        "grok" => Some("https://api.x.ai".into()),
        _ => None,
    }
}

fn same_upstream_base(
    local_base: Option<&str>,
    local_platform: &str,
    saved_base: Option<&str>,
    saved_platform: &str,
) -> bool {
    match (
        canonical_upstream_url(local_base, local_platform),
        canonical_upstream_url(saved_base, saved_platform),
    ) {
        (Some(a), Some(b)) => a == b,
        (None, None) => local_platform == saved_platform,
        _ => false,
    }
}

/// 匹配本机原生客户端登录关联条目（`monitor:native:*`）：排除浏览器独立封存授权（`browser:*`）。
fn find_native_monitor_auth(conn: &Connection, platform: &str) -> Option<(String, String, String)> {
    conn.query_row(
        "SELECT id, platform, name
         FROM connections
         WHERE platform = ?1 AND kind = 'auth'
           AND id LIKE 'monitor:native:%'
           AND status != 'paused'
         ORDER BY created_ms ASC, id ASC
         LIMIT 1",
        params![platform],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .ok()
    .flatten()
}

/// 在 `connections` 中匹配本机凭证（优先同平台精确匹配 / 原生本机登录关联，其次跨平台匹配相同上游 API Key）。
pub(crate) fn find_matching_saved_connection(
    conn: &Connection,
    local: &LocalConnection,
) -> Option<(String, String, String)> {
    if local.kind == "auth" && local.secret.starts_with(LOCAL_OAUTH_PREFIX) {
        return find_native_monitor_auth(conn, &local.platform);
    }
    if let Some(id) = crate::connections::find_current_local(conn, local).ok().flatten() {
        if let Some((plat, name, _)) = find_connection_meta(conn, &id) {
            return Some((id, plat, name));
        }
    }
    if local.kind != "api" {
        return None;
    }
    let cleaned_base = local
        .base_url
        .as_deref()
        .map(|v| v.trim().trim_end_matches('/'))
        .filter(|v| !v.is_empty());
    let mut stmt = conn
        .prepare(
            "SELECT id, platform, name, base_url
             FROM connections
             WHERE kind = 'api' AND secret = ?1
             ORDER BY created_ms ASC, id ASC",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![local.secret], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .ok()?;
    for row in rows.flatten() {
        let (id, saved_plat, name, saved_base) = row;
        if same_upstream_base(cleaned_base, &local.platform, saved_base.as_deref(), &saved_plat) {
            return Some((id, saved_plat, name));
        }
    }
    None
}

/// OAuth access_token 被 CLI 后台自动刷新后，原值可能与入库快照不同；
/// 若本机凭证能提取出已核实的邮箱，且该平台下唯一命中一条同邮箱的 `auth` 连接，仍认作已匹配。
fn find_unique_auth_by_email(conn: &Connection, platform: &str, email: &str) -> Option<(String, String)> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM connections
             WHERE platform = ?1 AND kind = 'auth'
               AND account_email IS NOT NULL
               AND lower(trim(account_email)) = lower(?2)",
        )
        .ok()?;
    let hits: Vec<(String, String)> = stmt
        .query_map(params![platform, trimmed], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok()?
        .flatten()
        .collect();
    match hits.as_slice() {
        [(id, name)] => Some((id.clone(), name.clone())),
        _ => None,
    }
}

/// 纯解析函数：输入本机读取到的凭证列表与邮箱提取器，返回脱敏后的生效状态列表。
pub(crate) fn resolve_with_lookup(
    conn: &Connection,
    locals: Vec<LocalConnection>,
    now_ms: i64,
    mut email_of: impl FnMut(&str, &str) -> Option<String>,
) -> Vec<LocalActiveAccount> {
    let mut out = Vec::with_capacity(locals.len());
    for local in locals {
        let is_local_oauth_fp = local.secret.starts_with(LOCAL_OAUTH_PREFIX);
        let identity_tag = is_local_oauth_fp.then(|| local.secret.clone());
        let masked = local
            .masked_override
            .clone()
            .unwrap_or_else(|| crate::connections::mask(&local.secret));
        let cleaned_base = local
            .base_url
            .as_ref()
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty());
        let normalized_base = crate::connections::normalized_base(&local.platform, &local.kind, cleaned_base.clone());

        let wired = crate::gateway::wiring::is_gateway_value(&local.secret)
            || cleaned_base
                .as_deref()
                .is_some_and(crate::gateway::wiring::is_gateway_value);
        if wired {
            out.push(LocalActiveAccount {
                platform: local.platform,
                kind: local.kind,
                state: "gateway_wired".into(),
                connection_id: None,
                connection_name: None,
                masked,
                account_email: None,
                base_url: cleaned_base,
                source_agent: local.source_agent,
                identity_tag: None,
                updated_ms: now_ms,
            });
            continue;
        }

        let detected_email = local.detected_email.clone().or_else(|| {
            if local.kind == "auth" && !is_local_oauth_fp {
                email_of(&local.platform, &local.secret)
            } else {
                None
            }
        });

        if let Some((id, _, name)) = find_matching_saved_connection(conn, &local) {
            let saved_email = find_connection_meta(conn, &id).and_then(|(_, _, e)| e);
            out.push(LocalActiveAccount {
                platform: local.platform,
                kind: local.kind,
                state: "matched".into(),
                connection_id: Some(id),
                connection_name: Some(name),
                masked,
                account_email: detected_email.or(saved_email),
                base_url: normalized_base,
                source_agent: local.source_agent,
                identity_tag,
                updated_ms: now_ms,
            });
            continue;
        }

        if let Some(email) = detected_email.as_deref() {
            if let Some((id, name)) = find_unique_auth_by_email(conn, &local.platform, email) {
                out.push(LocalActiveAccount {
                    platform: local.platform,
                    kind: local.kind,
                    state: "matched".into(),
                    connection_id: Some(id),
                    connection_name: Some(name),
                    masked,
                    account_email: detected_email,
                    base_url: None,
                    source_agent: local.source_agent,
                    identity_tag,
                    updated_ms: now_ms,
                });
                continue;
            }
        }

        out.push(LocalActiveAccount {
            platform: local.platform,
            kind: local.kind,
            state: "unregistered".into(),
            connection_id: None,
            connection_name: None,
            masked,
            account_email: detected_email,
            base_url: normalized_base,
            source_agent: local.source_agent,
            identity_tag,
            updated_ms: now_ms,
        });
    }
    out
}

fn should_replace_same_platform(existing: &LocalActiveAccount, candidate: &LocalActiveAccount) -> bool {
    let rank = |state: &str| match state {
        "unregistered" => 3,
        "matched" => 2,
        _ => 1,
    };
    rank(&candidate.state) >= rank(&existing.state)
}

/// 重读本机配置文件并与缓存比对，返回 `(最新快照, 身份发生变化的平台列表)`。
pub(crate) fn refresh_and_diff_platforms(conn: &Connection) -> (Vec<LocalActiveAccount>, Vec<String>) {
    if crate::validation::isolated() {
        return (Vec::new(), Vec::new());
    }
    let now_ms = chrono::Local::now().timestamp_millis();
    let customs = crate::custom_agents::list(conn).unwrap_or_default();
    let fresh = resolve_with_lookup(
        conn,
        crate::current_connection::read_all_local(&customs),
        now_ms,
        crate::login_detect::account_for_secret,
    );
    let mut guard = active_cell().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut next_map: BTreeMap<String, LocalActiveAccount> = BTreeMap::new();
    for item in fresh {
        if let Some(existing) = next_map.get(&item.platform) {
            if !should_replace_same_platform(existing, &item) {
                continue;
            }
        }
        next_map.insert(item.platform.clone(), item);
    }
    let mut changed_platforms = Vec::new();
    for (plat, item) in &mut next_map {
        if let Some(prev) = guard.get(plat) {
            if same_identity(prev, item) {
                item.updated_ms = prev.updated_ms;
            } else {
                changed_platforms.push(plat.clone());
            }
        } else {
            changed_platforms.push(plat.clone());
        }
    }
    for old_plat in guard.keys() {
        if !next_map.contains_key(old_plat) {
            changed_platforms.push(old_plat.clone());
        }
    }
    let list: Vec<LocalActiveAccount> = next_map.values().cloned().collect();
    *guard = next_map;
    (list, changed_platforms)
}

pub(crate) fn refresh_and_diff(conn: &Connection) -> (Vec<LocalActiveAccount>, bool) {
    let (list, changed) = refresh_and_diff_platforms(conn);
    (list, !changed.is_empty())
}

/// 读取当前本机生效账号列表；首次尚未初始化时自动填充一次。
pub(crate) fn current_list(conn: &Connection) -> Vec<LocalActiveAccount> {
    if crate::validation::isolated() {
        return Vec::new();
    }
    {
        let guard = active_cell().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !guard.is_empty() {
            return guard.values().cloned().collect();
        }
    }
    refresh_and_diff(conn).0
}

/// 取出当前全平台所有已确认匹配（`matched`）的 `(platform, connection_id)` 对（2B 实时打标用）。
pub(crate) fn matched_connections_or_init(conn: &Connection) -> Vec<(String, String)> {
    if crate::validation::isolated() {
        return Vec::new();
    }
    let initialized = {
        let guard = active_cell().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !guard.is_empty() {
            return guard
                .values()
                .filter(|item| item.state == "matched")
                .filter_map(|item| Some((item.platform.clone(), item.connection_id.clone()?)))
                .collect();
        }
        false
    };
    if !initialized {
        let _ = refresh_and_diff(conn);
    }
    let guard = active_cell().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .values()
        .filter(|item| item.state == "matched")
        .filter_map(|item| Some((item.platform.clone(), item.connection_id.clone()?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::{add, NewConnection};
    use std::path::Path;

    fn mem_db() -> Connection {
        crate::db::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn resolves_matched_unregistered_cross_platform_custom_and_native_oauth() {
        let conn = mem_db();
        let saved_id = add(
            &conn,
            NewConnection {
                service: None,
                platform: "claude".into(),
                kind: "api".into(),
                name: "Momo 中转".into(),
                secret: Some("sk-ant-saved-secret-1234".into()),
                base_url: Some("https://relay.example.test".into()),
                model: None,
                effort: None,
                context_1m: None,
                protocol: None,
                slug_base: None,
            },
        )
        .unwrap();

        let custom_id = add(
            &conn,
            NewConnection {
                service: None,
                platform: "custom".into(),
                kind: "api".into(),
                name: "硅基主力 Key".into(),
                secret: Some("sk-sf-shared-5678".into()),
                base_url: Some("https://api.siliconflow.cn".into()),
                model: None,
                effort: None,
                context_1m: None,
                protocol: Some("openai".into()),
                slug_base: Some("siliconflow".into()),
            },
        )
        .unwrap();

        conn.execute(
            "INSERT INTO connections(id, platform, kind, name, label, masked, status, created_ms, slug)
             VALUES('monitor:native:zcode:account', 'zcode', 'auth', 'ZCode 账号', '本机来源', '', 'connected', 1, 'zcode-local')",
            [],
        )
        .unwrap();

        let locals = vec![
            LocalConnection {
                platform: "claude".into(),
                kind: "api".into(),
                secret: "sk-ant-saved-secret-1234".into(),
                base_url: Some("https://relay.example.test/".into()),
                protocol: Some("anthropic".into()),
                source_agent: Some("Claude Code".into()),
                masked_override: None,
                detected_email: None,
            },
            LocalConnection {
                platform: "kimi".into(),
                kind: "api".into(),
                secret: "sk-sf-shared-5678".into(),
                base_url: Some("https://api.siliconflow.cn/v1".into()),
                protocol: Some("openai".into()),
                source_agent: Some("Kimi Code".into()),
                masked_override: None,
                detected_email: None,
            },
            LocalConnection {
                platform: "zcode".into(),
                kind: "auth".into(),
                secret: "local-oauth:zcode:1122334455667788".into(),
                base_url: None,
                protocol: None,
                source_agent: Some("ZCode".into()),
                masked_override: Some("ZCode · BigModel".into()),
                detected_email: None,
            },
        ];
        let resolved = resolve_with_lookup(&conn, locals, 1_700_000_000_000, |_, _| None);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].connection_id.as_deref(), Some(saved_id.as_str()));
        assert_eq!(resolved[1].connection_id.as_deref(), Some(custom_id.as_str()));
        assert_eq!(resolved[2].state, "matched");
        assert_eq!(
            resolved[2].connection_id.as_deref(),
            Some("monitor:native:zcode:account")
        );
        // identity_tag 不得序列化暴露到前端 JSON
        let json = serde_json::to_string(&resolved).unwrap();
        assert!(!json.contains("1122334455667788"));
    }

    #[test]
    fn rotated_oauth_token_still_matches_unique_verified_email_account() {
        let conn = mem_db();
        let saved_id = add(
            &conn,
            NewConnection {
                service: None,
                platform: "codex".into(),
                kind: "auth".into(),
                name: "Alex Plus".into(),
                secret: Some("old-oauth-token-0001".into()),
                base_url: None,
                model: None,
                effort: None,
                context_1m: None,
                protocol: None,
                slug_base: None,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE connections SET account_email = 'alex@example.test' WHERE id = ?1",
            params![saved_id],
        )
        .unwrap();

        let resolved = resolve_with_lookup(
            &conn,
            vec![LocalConnection {
                platform: "codex".into(),
                kind: "auth".into(),
                secret: "rotated-oauth-token-9999".into(),
                base_url: None,
                protocol: None,
                source_agent: Some("Codex".into()),
                masked_override: None,
                detected_email: None,
            }],
            100,
            |platform, _| (platform == "codex").then(|| "alex@example.test".into()),
        );
        assert_eq!(resolved[0].state, "matched");
        assert_eq!(resolved[0].connection_id.as_deref(), Some(saved_id.as_str()));
    }
}
