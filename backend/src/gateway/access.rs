//! 手动接入密钥独立于一键接线凭据；刷新不修改任何 Agent 配置。
//!
//! 命名客户端 Key（P8.3 局域网共享）也在本模块：签发 / 撤销 / 严格模式匹配 /
//! 当日请求限额。加密沿用同一套 DPAPI 保护（各自独立的熵域），原文只在签发
//! 结果里出现一次，落库的是十六进制密文，列表只出掩码。
use aes_gcm::aead::{rand_core::RngCore, OsRng};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const ENTROPY: &[u8] = b"jolu-gateway-manual-key-v1";

/// 命名客户端 Key 的加密熵域：与手动 Key 分开，避免两类密文可互换解读。
const CLIENT_KEY_ENTROPY: &[u8] = b"jolu-gateway-client-key-v1";

/// 命名客户端 Key 的令牌前缀。与 `jolu-manual-` 不相交；`wire::agent_from_headers`
/// 解析 `jolu-<agent>` 前缀时不会把 `client` 误认成 agent（不在 AGENTS 列表里）。
const CLIENT_KEY_PREFIX: &str = "jolu-client-";

/// Key 名称长度上限（字符数），对齐 magpie 对 name 的长度约束。
const CLIENT_KEY_MAX_NAME: usize = 64;

fn init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS gateway_access (id INTEGER PRIMARY KEY CHECK(id=1), protected BLOB NOT NULL)")
        .map_err(|_| "无法准备网关密钥存储".into())
}

fn client_store_init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gateway_client_keys (
            name             TEXT PRIMARY KEY,
            protected        TEXT NOT NULL,
            tokens_per_day   INTEGER,
            requests_per_day INTEGER,
            created_ms       INTEGER NOT NULL
        )",
    )
    .map_err(|_| "无法准备网关客户端密钥存储".into())
}

fn load(conn: &Connection) -> Result<Option<String>, String> {
    init(conn)?;
    let blob: Option<Vec<u8>> = conn
        .query_row("SELECT protected FROM gateway_access WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|_| "无法读取网关密钥")?;
    blob.map(|bytes| {
        let plain = crate::auth_snapshot::protect(&bytes, ENTROPY, false)?;
        String::from_utf8(plain).map_err(|_| "网关密钥数据损坏".into())
    })
    .transpose()
}

fn rotate(conn: &Connection) -> Result<String, String> {
    init(conn)?;
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| "无法生成安全的网关密钥")?;
    let key = format!(
        "jolu-manual-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let protected = crate::auth_snapshot::protect(key.as_bytes(), ENTROPY, true)?;
    // 单条原子更新；加密或写入失败时旧 Key 保持有效。
    conn.execute("INSERT INTO gateway_access(id,protected) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET protected=excluded.protected", [protected])
        .map_err(|_| "无法保存网关密钥")?;
    Ok(key)
}

fn current(conn: &Connection) -> Result<String, String> {
    match load(conn)? {
        Some(key) => Ok(key),
        None => rotate(conn),
    }
}

pub fn masked(conn: &Connection, refresh: bool) -> Result<String, String> {
    let key = if refresh {
        rotate(conn)?
    } else {
        current(conn)?
    };
    Ok(format!("••••••••{}", &key[key.len() - 4..]))
}

pub fn copy(conn: &Connection) -> Result<(), String> {
    let key = current(conn)?;
    let mut clipboard = arboard::Clipboard::new().map_err(|_| "无法打开剪贴板，请重试")?;
    clipboard
        .set_text(key)
        .map_err(|_| "复制失败，请重试".into())
}

fn equal(a: &str, b: &str) -> bool {
    let a = Sha256::digest(a.as_bytes());
    let b = Sha256::digest(b.as_bytes());
    a.iter()
        .zip(b.iter())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/* ─────────────── 命名客户端 Key（P8.3 局域网共享） ─────────────── */

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// 签发结果：Key 原文只在这里出现一次，调用方立刻交给客户端；后端只留密文。
#[derive(Debug, Clone, serde::Serialize)]
pub struct IssuedClientKey {
    pub name: String,
    pub key: String,
    pub tokens_per_day: Option<i64>,
    pub requests_per_day: Option<i64>,
}

/// 列表条目：只有掩码与限额，Key 原文不出后端。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientKeySummary {
    pub name: String,
    pub masked: String,
    pub tokens_per_day: Option<i64>,
    pub requests_per_day: Option<i64>,
    pub created_ms: i64,
    /// 最近一次计入流水的时间（保留范围内）；从未使用为 None
    pub last_used_ms: Option<i64>,
    /// 当日已计入限额的成功请求数（口径见 [`check_client_key_limit`]）
    pub today_requests: i64,
}

fn generate_client_key() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| "无法生成安全的客户端 Key")?;
    Ok(format!("{}{}", CLIENT_KEY_PREFIX, hex_encode(&bytes)))
}

fn client_key_violation(error: &rusqlite::Error, name: &str) -> String {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        format!("已存在名为「{name}」的客户端 Key：撤销后可重新签发，或换一个名称")
    } else {
        "无法保存网关客户端密钥".into()
    }
}

/// 签发一把命名客户端 Key。名称唯一；限额留空（None）表示不限。
/// 加密或写库失败时旧数据不受影响（INSERT 原子），调用方重试即可。
pub fn client_key_issue(
    conn: &Connection,
    name: &str,
    tokens_per_day: Option<i64>,
    requests_per_day: Option<i64>,
) -> Result<IssuedClientKey, String> {
    client_store_init(conn)?;
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err("客户端 Key 名称不能为空，也不能包含控制字符".into());
    }
    if name.chars().count() > CLIENT_KEY_MAX_NAME {
        return Err(format!("客户端 Key 名称最长 {CLIENT_KEY_MAX_NAME} 个字符"));
    }
    for (label, limit) in [("每日 Token 上限", tokens_per_day), ("每日请求上限", requests_per_day)] {
        if limit.is_some_and(|n| n <= 0) {
            return Err(format!("{label}只能留空（不限）或填正整数，当前为 {}", limit.unwrap_or(0)));
        }
    }
    let key = generate_client_key()?;
    let protected = crate::auth_snapshot::protect(key.as_bytes(), CLIENT_KEY_ENTROPY, true)?;
    conn.execute(
        "INSERT INTO gateway_client_keys(name, protected, tokens_per_day, requests_per_day, created_ms) VALUES(?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            name,
            hex_encode(&protected),
            tokens_per_day,
            requests_per_day,
            chrono::Local::now().timestamp_millis(),
        ],
    )
    .map_err(|error| client_key_violation(&error, name))?;
    Ok(IssuedClientKey { name: name.to_string(), key, tokens_per_day, requests_per_day })
}

/// 撤销 = 删除行。撤销后该 Key 立即失效（鉴权查表），但流水里的历史归属保留。
pub fn client_key_revoke(conn: &Connection, name: &str) -> Result<(), String> {
    client_store_init(conn)?;
    let deleted = conn
        .execute("DELETE FROM gateway_client_keys WHERE name = ?1", [name])
        .map_err(|_| "无法删除网关客户端密钥".to_string())?;
    if deleted == 0 {
        Err(format!("没有名为「{name}」的客户端 Key"))
    } else {
        Ok(())
    }
}

/// 解出一条命名 Key 的原文；数据损坏时明确报错，不静默当作不存在。
fn decrypt_client_key(name: &str, protected: &str) -> Result<String, String> {
    let bytes = hex_decode(protected).ok_or_else(|| format!("客户端 Key「{name}」数据损坏"))?;
    let plain = crate::auth_snapshot::protect(&bytes, CLIENT_KEY_ENTROPY, false)?;
    String::from_utf8(plain).map_err(|_| format!("客户端 Key「{name}」数据损坏"))
}

/// 列出全部命名 Key（按创建时间），Key 原文不出后端。
pub fn client_keys_list(conn: &Connection) -> Result<Vec<ClientKeySummary>, String> {
    client_store_init(conn)?;
    let mut stmt = conn
        .prepare("SELECT name, protected, tokens_per_day, requests_per_day, created_ms FROM gateway_client_keys ORDER BY created_ms, name")
        .map_err(|_| "无法读取网关客户端密钥".to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| "无法读取网关客户端密钥".to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (name, protected, tokens_per_day, requests_per_day, created_ms) =
            row.map_err(|_| "无法读取网关客户端密钥".to_string())?;
        let (last_used_ms, today_requests) = client_key_usage(conn, &name)?;
        let key = decrypt_client_key(&name, &protected)?;
        out.push(ClientKeySummary {
            masked: format!("••••••••{}", &key[key.len() - 4..]),
            name,
            tokens_per_day,
            requests_per_day,
            created_ms,
            last_used_ms,
            today_requests,
        });
    }
    Ok(out)
}

/// 某 Key 的流水使用情况：最近使用时间与当日成功请求数（限额同一口径）。
fn client_key_usage(conn: &Connection, name: &str) -> Result<(Option<i64>, i64), String> {
    let (start_ms, _) = day_window(chrono::Local::now());
    conn.query_row(
        "SELECT MAX(ts), COALESCE(SUM(result = 'ok' AND ts >= ?2), 0) FROM gateway_route_log WHERE client_key = ?1",
        rusqlite::params![name, start_ms],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(|_| "无法统计网关请求数".to_string())
}

/// 严格模式下的命名 Key 匹配：逐把解密、常数时间比较，命中返回 Key 名。
fn match_client_key(conn: &Connection, supplied: &str) -> Result<Option<String>, String> {
    client_store_init(conn)?;
    let mut stmt = conn
        .prepare("SELECT name, protected FROM gateway_client_keys")
        .map_err(|_| "无法读取网关客户端密钥".to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|_| "无法读取网关客户端密钥".to_string())?;
    for row in rows {
        let (name, protected) = row.map_err(|_| "无法读取网关客户端密钥".to_string())?;
        if equal(&decrypt_client_key(&name, &protected)?, supplied) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// 本地时区的「当日」窗口：(当日零点毫秒, 次日零点毫秒)。
/// 限额按这个窗口计数与重置；DST 缺失的午夜按最早解释，再退回 UTC 解释
/// ——只影响极端时区某一天的窗口边界，不改变计数口径。
fn day_window(now: chrono::DateTime<chrono::Local>) -> (i64, i64) {
    let midnight_ms = |date: chrono::NaiveDate| -> i64 {
        let naive = date.and_hms_opt(0, 0, 0).expect("00:00 是合法时刻");
        let resolved = match naive.and_local_timezone(*now.offset()) {
            chrono::LocalResult::Single(t) => t,
            chrono::LocalResult::Ambiguous(t, _) => t,
            chrono::LocalResult::None => naive
                .and_local_timezone(chrono::Utc)
                .single()
                .unwrap_or_else(|| now.with_timezone(&chrono::Utc))
                .with_timezone(now.offset()),
        };
        resolved.timestamp_millis()
    };
    let today = now.date_naive();
    let tomorrow = today.succ_opt().unwrap_or(today);
    (midnight_ms(today), midnight_ms(tomorrow))
}

/// 超限拒绝：说明 Key 名、限额与重置时间（当日结束），并带 Retry-After 秒数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestLimitRefusal {
    pub message: String,
    pub retry_after_seconds: i64,
}

/// 命名 Key 的当日请求限额，在请求进入任何上游适配器之前检查（对齐 magpie
/// keyLimited：超限 429，读取类请求与工具回调不计费）。
///
/// 计数口径：`gateway_route_log` 里该 Key 在当日（本地零点起）`result = 'ok'`
/// 的行数——只有成功转发到上游的请求计入；被拒、上游失败与读取类请求不计。
/// 流水只保留最近约 1000 行：单日成功请求超出保留范围时计数是下限，
/// 限流会偏保守（提前拒绝），不会放超。
///
/// `tokens_per_day` 字段本期只保存、不强制：精确 Token 计数依赖用量归账时序
/// （响应结束后才拿得到用量，无法在请求前预留），强行计数会把请求管线改乱；
/// 限额判断只看请求数。
pub(crate) fn check_client_key_limit(
    conn: &Connection,
    name: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Result<Option<RequestLimitRefusal>, String> {
    client_store_init(conn)?;
    let limit: Option<Option<i64>> = conn
        .query_row(
            "SELECT requests_per_day FROM gateway_client_keys WHERE name = ?1",
            [name],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "无法读取网关客户端密钥".to_string())?;
    let Some(Some(limit)) = limit else {
        return Ok(None); // Key 已被撤销（鉴权已放行到这一步）或未设请求数限额
    };
    let (start_ms, reset_ms) = day_window(now);
    let used: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM gateway_route_log WHERE client_key = ?1 AND result = 'ok' AND ts >= ?2",
            rusqlite::params![name, start_ms],
            |r| r.get(0),
        )
        .map_err(|_| "无法统计网关请求数".to_string())?;
    if used < limit {
        return Ok(None);
    }
    let reset = chrono::DateTime::from_timestamp_millis(reset_ms)
        .unwrap_or(now.with_timezone(&chrono::Utc))
        .with_timezone(&chrono::Local);
    Ok(Some(RequestLimitRefusal {
        message: format!(
            "客户端 Key「{name}」今日请求数已达上限（{limit} 条），将于 {} 重置",
            reset.format("%Y-%m-%d %H:%M")
        ),
        retry_after_seconds: ((reset_ms - now.timestamp_millis()) / 1000).max(1),
    }))
}

/// 鉴权结果：是否放行，以及严格模式下命中的命名客户端 Key。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthOutcome {
    pub allowed: bool,
    /// 命中的命名 Key 名称（只有 `jolu-client-` 前缀的令牌可能返回）。
    /// 手动 Key 与接线令牌不属于命名 Key，仍为 None；局域网共享的
    /// 限额检查与路由流水归属都依赖它。
    pub client_key: Option<String>,
}

impl AuthOutcome {
    fn allow() -> Self {
        AuthOutcome { allowed: true, client_key: None }
    }

    fn deny() -> Self {
        AuthOutcome { allowed: false, client_key: None }
    }
}

/// 原生 OAuth 与桥接继续走既有接线；手动 Key 永远先校验，不能借路径绕过撤销。
///
/// `relaxed`（宽松模式，参照 magpie）：网关只监听 127.0.0.1 回环且未开启局域网共享时
/// （`proxy.rs::start` 里 `gateway_auth_relaxed`），令牌只用于识别调用方
/// （`jolu-<agent>` 前缀与 User-Agent，见 `wire::agent_from_headers`），不再比对任何
/// 密钥——手动 Key 刷新后，工具配置里的旧令牌依然放行，不会集体 401。
/// 安全是「只监听回环」这条底线给的，不是令牌给的；共享开关开启后监听 0.0.0.0，
/// 调用方必须传 `false` 走严格校验（见 `authorize_detail`）。
pub fn authorize(
    conn: &Connection,
    token: Option<&str>,
    path: &str,
    relaxed: bool,
) -> Result<bool, String> {
    Ok(authorize_detail(conn, token, path, relaxed)?.allowed)
}

/// [`authorize`] 的完整版：把命中的命名客户端 Key 名带回调用方（局域网共享时
/// 用于限额检查与路由流水归属）。
pub(crate) fn authorize_detail(
    conn: &Connection,
    token: Option<&str>,
    path: &str,
    relaxed: bool,
) -> Result<AuthOutcome, String> {
    authorize_with_detail(conn, token, path, relaxed, || {
        crate::creds::local_codex_auth().map(|(token, _)| token)
    })
}

fn authorize_with(
    conn: &Connection,
    token: Option<&str>,
    path: &str,
    relaxed: bool,
    oauth: impl Fn() -> Option<String>,
) -> Result<bool, String> {
    Ok(authorize_with_detail(conn, token, path, relaxed, oauth)?.allowed)
}

fn authorize_with_detail(
    conn: &Connection,
    token: Option<&str>,
    path: &str,
    relaxed: bool,
    oauth: impl Fn() -> Option<String>,
) -> Result<AuthOutcome, String> {
    // 宽松模式：回环上的任意令牌（含未携带令牌）都放行，甚至不读取密钥存储。
    // 调用方识别照旧由 wire 层按 `jolu-<agent>` 前缀与 User-Agent 完成。
    if relaxed {
        return Ok(AuthOutcome::allow());
    }
    // 命名客户端 Key（局域网共享，P8.3）：`jolu-client-` 前缀的令牌只与
    // gateway_client_keys 表比对——命中未撤销的 Key 即放行并带回 Key 名；
    // 未命中直接拒绝，不落到接线令牌比对（两种 Key 的格式不相交）。
    if let Some(supplied) = token.filter(|t| t.starts_with(CLIENT_KEY_PREFIX)) {
        let matched = match_client_key(conn, supplied)?;
        return Ok(AuthOutcome { allowed: matched.is_some(), client_key: matched });
    }
    let key = load(conn)?;
    if token.is_some_and(|t| t.starts_with("jolu-manual-")) {
        return Ok(if key.as_deref().zip(token).is_some_and(|(a, b)| equal(a, b)) {
            AuthOutcome::allow()
        } else {
            AuthOutcome::deny()
        });
    }
    // 未开通过手动接入的旧安装维持原有行为。
    let Some(key) = key else { return Ok(AuthOutcome::allow()) };
    if token.is_some_and(|t| equal(&key, t)) {
        return Ok(AuthOutcome::allow());
    }
    if path == "/__jolu/bridge/callback" {
        return Ok(AuthOutcome::allow());
    }
    let wired = super::wiring_store::all(conn).iter().any(|record| {
        record.owner == "relay"
            && matches!(record.state.as_str(), "wired" | "pending")
            && (record
                .token
                .as_deref()
                .zip(token)
                .is_some_and(|(a, b)| equal(a, b))
                || (record.agent == "codex"
                    && record
                        .applied()
                        .is_some_and(|a| a.mode.as_deref() == Some("oauth"))
                    && matches!(
                        path,
                        "/backend-api/codex/models" | "/backend-api/codex/responses"
                    )
                    && token
                        .zip(oauth())
                        .is_some_and(|(supplied, current)| equal(supplied, &current))))
    });
    Ok(if wired { AuthOutcome::allow() } else { AuthOutcome::deny() })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn manual_key_is_protected_persistent_and_revoked_on_rotation() {
        let conn = Connection::open_in_memory().unwrap();
        let first = current(&conn).unwrap();
        assert_eq!(current(&conn).unwrap(), first);
        let blob: Vec<u8> = conn
            .query_row("SELECT protected FROM gateway_access", [], |r| r.get(0))
            .unwrap();
        assert!(!blob.windows(first.len()).any(|w| w == first.as_bytes()));
        assert!(authorize(&conn, Some(&first), "/v1/models", false).unwrap());
        assert!(!masked(&conn, false).unwrap().contains(&first));
        let second = rotate(&conn).unwrap();
        assert_ne!(first, second);
        for path in [
            "/v1/models",
            "/backend-api/codex/models",
            "/__jolu/bridge/callback",
        ] {
            assert!(!authorize(&conn, Some(&first), path, false).unwrap());
            assert!(authorize(&conn, Some(&second), path, false).unwrap());
        }
        assert!(!authorize(&conn, None, "/v1/models", false).unwrap());
        assert!(!authorize(&conn, Some("arbitrary"), "/v1/models", false).unwrap());
    }

    #[tokio::test]
    async fn rotation_rejects_old_http_keys_but_preserves_wired_agents() {
        let deps = crate::proxy::test_support::bare_deps();
        let first = {
            let conn = deps.db.lock().unwrap();
            super::super::wiring_store::mark_pending(
                &conn,
                "claude-code",
                "test",
                "relay",
                None,
                &super::super::wiring_store::Applied::default(),
                Some("jolu-claude-code.test"),
            )
            .unwrap();
            current(&conn).unwrap()
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
        let router = super::super::server::router(deps.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for header in ["authorization", "x-api-key", "x-goog-api-key"] {
            let value = if header == "authorization" {
                format!("Bearer {first}")
            } else {
                first.clone()
            };
            assert_eq!(
                client
                    .get(&url)
                    .header(header, value)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
        let second = rotate(&deps.db.lock().unwrap()).unwrap();
        for (token, status) in [
            (&first[..], 401),
            (&second[..], 200),
            ("jolu-claude-code.test", 200),
            ("unknown", 401),
        ] {
            assert_eq!(
                client
                    .get(&url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
        assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(&second)
                .header("origin", "https://example.com")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        task.abort();
    }

    #[test]
    fn damaged_storage_does_not_silently_replace_the_key() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn.execute(
            "INSERT INTO gateway_access VALUES(1,?1)",
            [b"broken".as_slice()],
        )
        .unwrap();
        assert!(current(&conn).is_err());
        assert!(authorize(&conn, Some("anything"), "/v1/models", false).is_err());
    }

    #[test]
    fn oauth_requires_current_native_token_and_is_unaffected_by_rotation() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        super::super::wiring_store::mark_pending(
            &conn,
            "codex",
            "test",
            "relay",
            None,
            &super::super::wiring_store::Applied {
                mode: Some("oauth".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let first = current(&conn).unwrap();
        for _ in 0..2 {
            let oauth = || Some("synthetic-native-token".to_string());
            assert!(authorize_with(
                &conn,
                Some("synthetic-native-token"),
                "/backend-api/codex/models",
                false,
                oauth
            )
            .unwrap());
            assert!(
                !authorize_with(
                    &conn,
                    Some("arbitrary"),
                    "/backend-api/codex/models",
                    false,
                    oauth
                )
                .unwrap()
            );
            assert!(
                !authorize_with(&conn, Some("synthetic-native-token"), "/v1/models", false, oauth)
                    .unwrap()
            );
            rotate(&conn).unwrap();
        }
        assert!(
            !authorize_with(
                &conn,
                Some(&first),
                "/backend-api/codex/models",
                false,
                || Some(first.clone())
            )
            .unwrap()
        );
    }

    /// 宽松模式（回环中转，参照 magpie）：任意令牌、无令牌都放行，
    /// 且不依赖密钥存储是否可读——Key 刷新与存储损坏都不再打断调用方。
    /// 命名客户端 Key 的令牌在宽松模式下同样只当作普通令牌（不带 Key 名）。
    #[test]
    fn relaxed_loopback_mode_accepts_any_token_without_key_comparison() {
        let conn = Connection::open_in_memory().unwrap();
        let key = current(&conn).unwrap();
        for token in [
            Some("anything"),
            Some("jolu-codex.stalekey"),
            Some("jolu-manual-0000"),
            Some("jolu-client-not-issued-yet"),
            Some(key.as_str()),
            None,
        ] {
            assert!(authorize(&conn, token, "/v1/models", true).unwrap(), "{token:?}");
        }
        // 宽松分支不读密钥存储：存储损坏照样放行；严格模式则报错不静默换钥。
        conn.execute(
            "UPDATE gateway_access SET protected=?1",
            [b"broken".as_slice()],
        )
        .unwrap();
        assert!(authorize(&conn, Some("anything"), "/v1/models", true).unwrap());
        assert!(authorize(&conn, Some("anything"), "/v1/models", false).is_err());
    }

    /// 安全底线：宽松只属于回环监听。一旦监听地址可能不是回环（未来局域网共享），
    /// 调用方必须传 relaxed=false，任意令牌重新被严格校验拒绝。
    #[test]
    fn non_loopback_listening_requires_strict_auth() {
        let conn = Connection::open_in_memory().unwrap();
        current(&conn).unwrap();
        assert!(!authorize(&conn, Some("stale-jolu-token"), "/v1/models", false).unwrap());
        assert!(!authorize(&conn, None, "/v1/models", false).unwrap());
        assert!(!authorize(&conn, Some("jolu-codex.old"), "/v1/messages", false).unwrap());
    }

    /// HTTP 端到端（宽松 deps）：旧 Key、任意令牌、无令牌都放行；
    /// 非回环 Host 与浏览器跨站仍然在进入鉴权之前被拒。
    #[tokio::test]
    async fn relaxed_http_gateway_accepts_stale_and_arbitrary_tokens() {
        let bare = std::sync::Arc::try_unwrap(crate::proxy::test_support::bare_deps())
            .ok()
            .expect("新建的 deps 只有一个引用");
        let deps = {
            let mut relaxed = bare;
            relaxed.gateway_auth_relaxed = true;
            std::sync::Arc::new(relaxed)
        };
        let first = {
            let conn = deps.db.lock().unwrap();
            super::super::wiring_store::mark_pending(
                &conn,
                "claude-code",
                "test",
                "relay",
                None,
                &super::super::wiring_store::Applied::default(),
                Some("jolu-claude-code.test"),
            )
            .unwrap();
            current(&conn).unwrap()
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
        let router = super::super::server::router(deps.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (header, value) in [
            ("authorization", "Bearer jolu-manual-stale".to_string()),
            ("x-api-key", "anything".to_string()),
            ("x-goog-api-key", first.clone()),
            ("authorization", "Bearer jolu-claude-code.test".to_string()),
        ] {
            assert_eq!(
                client
                    .get(&url)
                    .header(header, value)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200,
                "宽松模式接受任意调用方令牌：{header}"
            );
        }
        assert_eq!(
            client.get(&url).send().await.unwrap().status(),
            200,
            "未携带令牌的回环请求同样放行"
        );
        assert_eq!(
            client
                .get(&url)
                .header("host", "evil.example:12731")
                .send()
                .await
                .unwrap()
                .status(),
            403,
            "非回环 Host 在宽松模式下仍然拒绝"
        );
        assert_eq!(
            client
                .get(&url)
                .header("origin", "https://example.com")
                .send()
                .await
                .unwrap()
                .status(),
            403,
            "浏览器跨站在宽松模式下仍然拒绝"
        );
        task.abort();
    }

    /* ─────────────── 命名客户端 Key（P8.3 局域网共享） ─────────────── */

    /// 用迁移过的内存库：限额与列表要查 gateway_route_log，裸连接没有这张表。
    fn migrated_conn() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn client_key_issue_mask_and_revoke_roundtrip() {
        let conn = migrated_conn();
        let issued = client_key_issue(&conn, "  lab-agent  ", None, Some(5)).unwrap();
        assert!(issued.key.starts_with("jolu-client-"), "原文前缀固定：{}", issued.key);
        assert_eq!(issued.name, "lab-agent", "名称去掉首尾空白");
        // 名称唯一：重复签发明确报错，不静默覆盖
        assert!(client_key_issue(&conn, "lab-agent", None, None).is_err());
        // 列表：只有掩码，原文不出后端；限额与计数可见
        let list = client_keys_list(&conn).unwrap();
        assert_eq!(list.len(), 1);
        let summary = &list[0];
        assert_eq!(summary.name, "lab-agent");
        assert_eq!((summary.tokens_per_day, summary.requests_per_day), (None, Some(5)));
        assert_eq!(summary.masked, format!("••••••••{}", &issued.key[issued.key.len() - 4..]));
        assert!(!summary.masked.contains(&issued.key));
        assert_eq!((summary.last_used_ms, summary.today_requests), (None, 0));
        // 未知名称撤销报错；撤销后再签发同名是全新的原文
        assert!(client_key_revoke(&conn, "missing").is_err());
        client_key_revoke(&conn, "lab-agent").unwrap();
        assert!(client_keys_list(&conn).unwrap().is_empty());
        let again = client_key_issue(&conn, "lab-agent", None, None).unwrap();
        assert_ne!(issued.key, again.key);
        // 非法输入：空名、控制字符、超长名称、非正数限额
        assert!(client_key_issue(&conn, "", None, None).is_err());
        assert!(client_key_issue(&conn, "bad\u{0007}name", None, None).is_err());
        assert!(client_key_issue(&conn, &"x".repeat(65), None, None).is_err());
        assert!(client_key_issue(&conn, "ok", Some(0), None).is_err());
        assert!(client_key_issue(&conn, "ok", None, Some(-1)).is_err());
    }

    #[test]
    fn named_client_keys_pass_strict_auth_and_revoke_closes_them() {
        let conn = migrated_conn();
        current(&conn).unwrap(); // 开通手动 Key，关闭「旧安装无密钥全放行」分支
        let issued = client_key_issue(&conn, "lab-agent", None, None).unwrap();
        // 宽松模式不受命名 Key 影响：任意令牌照旧放行，且不带 Key 名
        let relaxed = authorize_detail(&conn, Some(&issued.key), "/v1/models", true).unwrap();
        assert!(relaxed.allowed && relaxed.client_key.is_none());
        // 严格模式：命名 Key 放行并带回名字；手动 Key 与命名 Key 互不混淆
        let outcome = authorize_detail(&conn, Some(&issued.key), "/v1/messages", false).unwrap();
        assert!(outcome.allowed);
        assert_eq!(outcome.client_key.as_deref(), Some("lab-agent"));
        let manual = authorize_detail(&conn, Some(&current(&conn).unwrap()), "/v1/models", false).unwrap();
        assert!(manual.allowed && manual.client_key.is_none());
        // 未知 Key 拒绝；撤销后立即失效；无令牌也不放行
        assert!(!authorize_detail(&conn, Some("jolu-client-0000"), "/v1/models", false).unwrap().allowed);
        client_key_revoke(&conn, "lab-agent").unwrap();
        assert!(!authorize_detail(&conn, Some(&issued.key), "/v1/models", false).unwrap().allowed);
        assert!(!authorize_detail(&conn, None, "/v1/models", false).unwrap().allowed);
    }

    #[test]
    fn request_limit_counts_today_ok_rows_and_says_reset_time() {
        let conn = migrated_conn();
        client_key_issue(&conn, "quota", None, Some(3)).unwrap();
        client_key_issue(&conn, "free", None, None).unwrap();
        let now = chrono::Local::now();
        let row = |result: &'static str, client: &'static str| super::super::route_log::NewEntry {
            agent: None,
            requested_model: "a/b",
            source: None,
            upstream_model: None,
            status_code: Some(200),
            result,
            duration_ms: None,
            connection_id: None,
            session_platform: None,
            session_id: None,
            client_key: Some(client),
        };
        // failed / rejected 与别的 Key 的请求都不计入当日额度
        super::super::route_log::record(&conn, row("ok", "quota"));
        super::super::route_log::record(&conn, row("failed", "quota"));
        super::super::route_log::record(&conn, row("rejected", "quota"));
        super::super::route_log::record(&conn, row("ok", "free"));
        assert!(check_client_key_limit(&conn, "quota", now).unwrap().is_none());
        super::super::route_log::record(&conn, row("ok", "quota"));
        assert!(check_client_key_limit(&conn, "quota", now).unwrap().is_none());
        // 前一天的请求不进当日窗口
        let (start_ms, reset_ms) = day_window(now);
        conn.execute(
            "UPDATE gateway_route_log SET ts = ?1 WHERE result = 'ok' AND client_key = 'quota' AND ts >= ?2",
            rusqlite::params![start_ms - 1, start_ms],
        )
        .unwrap();
        assert!(check_client_key_limit(&conn, "quota", now).unwrap().is_none());
        // 回填到上限：两条昨天的旧数据搬走后，今天补 3 条成功请求
        super::super::route_log::record(&conn, row("ok", "quota"));
        super::super::route_log::record(&conn, row("ok", "quota"));
        assert!(check_client_key_limit(&conn, "quota", now).unwrap().is_none(), "当日 2 条，未到 3 条上限");
        super::super::route_log::record(&conn, row("ok", "quota"));
        let refusal = check_client_key_limit(&conn, "quota", now).unwrap().expect("3 条已达上限");
        assert!(refusal.message.contains("quota"), "说明 Key 名：{}", refusal.message);
        assert!(refusal.message.contains("3 条"), "说明限额：{}", refusal.message);
        let reset = chrono::DateTime::from_timestamp_millis(reset_ms)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert!(
            refusal.message.contains(&reset.format("%Y-%m-%d %H:%M").to_string()),
            "说明重置时间（当日结束）：{}", refusal.message
        );
        assert!(refusal.retry_after_seconds > 0);
        // 无限额 Key 永不拒绝
        assert!(check_client_key_limit(&conn, "free", now).unwrap().is_none());
        // 被撤销的 Key（行已删）不再触发限额分支
        client_key_revoke(&conn, "quota").unwrap();
        assert!(check_client_key_limit(&conn, "quota", now).unwrap().is_none());
    }

    /// HTTP 端到端（局域网共享 deps）：命名 Key 放行、未知 Key 401，
    /// 限额超限在进 provider 之前返回 429 并说明重置时间；读取类请求不受限额影响。
    #[tokio::test]
    async fn shared_gateway_enforces_named_keys_and_request_limits() {
        let deps = crate::proxy::test_support::lan_shared_deps();
        let issued = {
            let conn = deps.db.lock().unwrap();
            current(&conn).unwrap(); // 关闭「旧安装无密钥全放行」分支
            client_key_issue(&conn, "lan-agent", None, Some(1)).unwrap()
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = super::super::server::router(deps.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 合法命名 Key：GET 模型清单放行；未知 Key 与无令牌都 401
        assert_eq!(client.get(format!("{base}/v1/models")).bearer_auth(&issued.key).send().await.unwrap().status(), 200);
        assert_eq!(client.get(format!("{base}/v1/models")).bearer_auth("jolu-client-unknown").send().await.unwrap().status(), 401);
        assert_eq!(client.get(format!("{base}/v1/models")).send().await.unwrap().status(), 401);
        // 未超限：中间件放行（POST 缺 model 在 handler 里报 400，不是 401/429）
        assert_eq!(
            client.post(format!("{base}/v1/messages")).bearer_auth(&issued.key).body("{}").send().await.unwrap().status(),
            400
        );
        // 造一条成功流水把当日额度（1 条）用满；再 POST 同样的请求 → 429
        {
            let conn = deps.db.lock().unwrap();
            super::super::route_log::record(
                &conn,
                super::super::route_log::NewEntry {
                    agent: None,
                    requested_model: "a/b",
                    source: None,
                    upstream_model: None,
                    status_code: Some(200),
                    result: "ok",
                    duration_ms: None,
                    connection_id: None,
                    session_platform: None,
                    session_id: None,
                    client_key: Some("lan-agent"),
                },
            );
        }
        let refused = client
            .post(format!("{base}/v1/messages"))
            .bearer_auth(&issued.key)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 429, "限额超限必须在进 provider 之前拒绝");
        let retry_after = refused
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .expect("429 必须带 Retry-After");
        assert!(retry_after > 0);
        let body = refused.text().await.unwrap();
        assert!(body.contains("lan-agent"), "说明 Key 名：{body}");
        assert!(body.contains("1 条"), "说明限额：{body}");
        assert!(body.contains("重置"), "说明重置时间：{body}");
        // 读取类请求不计费、不受限额影响；工具回调路径同样豁免
        assert_eq!(client.get(format!("{base}/v1/models")).bearer_auth(&issued.key).send().await.unwrap().status(), 200);
        assert_eq!(
            client.post(format!("{base}/__jolu/bridge/callback")).bearer_auth(&issued.key).body("{}").send().await.unwrap().status(),
            400, "工具回调不进限额分支（400 是回调体校验）"
        );
        task.abort();
    }
}
