//! 本地代理与网关共用的运行时依赖、平台判定、去重键提取与待关联计时队列。
//!
//! 严格保持零 `gateway::server / wire / escape / adapter` 与零 `commands` 依赖，
//! 使 `gateway/*` 单向依赖本模块，而监听组装由上层 `proxy::host` 完成。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

use crate::proxy_config::ProxyPlatform;

/// 响应流头部快照上限：Claude `message_start` 与 Codex `response.created`
/// 都在流的最开头，64 KiB 足以覆盖，同时防止大响应常驻内存。
pub(crate) const HEAD_CAP: usize = 64 * 1024;
/// 请求体缓冲上限：长上下文请求可达数十 MB，取 512 MB（axum 默认 2 MB 会 413）。
pub(crate) const REQUEST_BODY_LIMIT: usize = 512 * 1024 * 1024;
/// 待关联队列 TTL：超过后放弃（响应结束到 CLI 写日志通常毫秒~秒级）。
pub(crate) const PENDING_TTL_MS: i64 = 10 * 60 * 1000;
/// 上游连续不可达次数达到该值即触发自动回退直连。
pub(crate) const FALLBACK_THRESHOLD: u32 = 3;

/// 待关联队列：响应结束时请求记录尚未入库，先暂存等采集扫描后补写。
#[derive(Debug, Clone)]
pub(crate) struct PendingTiming {
    pub dedup_key: String,
    pub first_token_ms: i64,
    pub duration_ms: i64,
    pub status_code: i64,
    pub created_ms: i64,
    pub connection_id: Option<String>,
    pub upstream_model: Option<String>,
    pub served_model: Option<String>,
}

/// 凭证指纹索引：API Key 连接的 SHA-256 → 连接 id。只在内存里保存哈希，定期重建以跟随连接变更。
#[derive(Default)]
pub(crate) struct KeyIndex {
    refreshed_ms: i64,
    entries: Vec<(ProxyPlatform, [u8; 32], String)>,
}
const KEY_INDEX_TTL_MS: i64 = 30_000;

/// 网关会话键 → claude 的 session_id（用于 `--resume`）。
pub(crate) struct ClaudeSessions {
    pub(crate) entries: HashMap<String, (String, i64)>,
}

pub(crate) const CLAUDE_SESSION_CAP: usize = 200;
pub(crate) const CLAUDE_SESSION_IDLE: Duration = Duration::from_secs(10 * 60);

impl ClaudeSessions {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub(crate) fn resume_for(&mut self, session: &str) -> Option<String> {
        let (id, used) = self.entries.get(session)?.clone();
        if now_ms() - used > CLAUDE_SESSION_IDLE.as_millis() as i64 {
            self.entries.remove(session);
            return None;
        }
        Some(id)
    }

    pub(crate) fn bind(&mut self, session: &str, claude_session: &str) {
        if self.entries.len() >= CLAUDE_SESSION_CAP && !self.entries.contains_key(session) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries
            .insert(session.to_string(), (claude_session.to_string(), now_ms()));
    }

    pub(crate) fn forget(&mut self, session: &str) {
        self.entries.remove(session);
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

fn fingerprint(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.trim().as_bytes()).into()
}

fn request_credential(platform: ProxyPlatform, headers: &HeaderMap) -> Option<String> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let bearer = header("authorization")
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(str::trim)
        .filter(|v| !v.is_empty());
    match platform {
        ProxyPlatform::Claude => header("x-api-key").or(bearer),
        _ => bearer,
    }
    .map(str::to_string)
}

pub(crate) static PENDING: LazyLock<Arc<Mutex<Vec<PendingTiming>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
pub(crate) static LAST_ERROR: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
pub(crate) static FALLBACK_REASON: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));
pub(crate) static PROXY: LazyLock<Mutex<Option<Arc<ProxyRuntime>>>> =
    LazyLock::new(|| Mutex::new(None));

impl ProxyPlatform {
    pub(crate) fn from_platform(platform: &str) -> Self {
        match platform {
            "codex" | "opencode" => ProxyPlatform::Codex,
            _ => ProxyPlatform::Claude,
        }
    }
}

#[cfg(test)]
pub(crate) fn pending_len() -> usize {
    PENDING.lock().map(|q| q.len()).unwrap_or(0)
}

pub(crate) fn queue_pending_timing(
    dedup_key: &str,
    first_token_ms: i64,
    duration_ms: i64,
    status_code: i64,
    connection_id: Option<&str>,
) {
    queue_pending_timing_with_models(
        dedup_key,
        first_token_ms,
        duration_ms,
        status_code,
        connection_id,
        None,
        None,
    );
}

pub(crate) fn queue_pending_timing_with_models(
    dedup_key: &str,
    first_token_ms: i64,
    duration_ms: i64,
    status_code: i64,
    connection_id: Option<&str>,
    upstream_model: Option<&str>,
    served_model: Option<&str>,
) {
    if let Ok(mut queue) = PENDING.lock() {
        queue.push(PendingTiming {
            dedup_key: dedup_key.to_string(),
            first_token_ms,
            duration_ms,
            status_code,
            created_ms: now_ms(),
            connection_id: connection_id.map(str::to_string),
            upstream_model: upstream_model.filter(|s| !s.is_empty()).map(str::to_string),
            served_model: served_model.filter(|s| !s.is_empty()).map(str::to_string),
        });
    }
}

/// 停止监听（CLI 配置的还原由调用方负责）。
pub fn stop() {
    let Some(runtime) = PROXY.lock().expect("代理状态锁").take() else {
        return;
    };
    let _ = runtime.shutdown.send(true);
}

pub(crate) fn running_deps() -> Option<Arc<ProxyDeps>> {
    PROXY.lock().ok()?.as_ref().map(|runtime| runtime.deps.clone())
}

pub(crate) fn detached_deps(db: Arc<Mutex<rusqlite::Connection>>) -> Arc<ProxyDeps> {
    let (shutdown_tx, _rx) = tokio::sync::watch::channel(false);
    Arc::new(ProxyDeps {
        client: reqwest::Client::new(),
        db,
        upstreams: HashMap::new(),
        pending: Arc::new(Mutex::new(Vec::new())),
        consecutive_failures: Arc::new(AtomicU32::new(0)),
        fallback_enabled: false,
        key_index: Mutex::new(KeyIndex::default()),
        shutdown: Arc::new(shutdown_tx),
        on_fallback: Arc::new(|_| {}),
        pool_index: None,
        gateway_default_connection: None,
        affinity: None,
        cooldowns: None,
        claude_sessions: None,
        circuit_threshold: u32::MAX,
        live_circuit_threshold: None,
        on_circuit: Arc::new(|_| {}),
        on_activity: Arc::new(|_, _| {}),
        on_quota_stale: Arc::new(|_| {}),
        gateway_auth_relaxed: true,
        gateway_lan_shared: false,
        request_uids: Default::default(),
    })
}

pub(crate) fn running_port() -> Option<u16> {
    PROXY.lock().ok()?.as_ref().map(|runtime| runtime.port)
}

pub(crate) type LiveSetting<T> = Arc<dyn Fn() -> T + Send + Sync>;

pub(crate) struct ProxyRuntime {
    pub port: u16,
    pub(crate) mode: crate::gateway::Mode,
    pub(crate) shutdown: Arc<tokio::sync::watch::Sender<bool>>,
    pub deps: Arc<ProxyDeps>,
}

pub(crate) struct ProxyDeps {
    pub(crate) client: reqwest::Client,
    pub(crate) db: Arc<Mutex<rusqlite::Connection>>,
    pub(crate) upstreams: HashMap<ProxyPlatform, String>,
    pub(crate) pending: Arc<Mutex<Vec<PendingTiming>>>,
    pub(crate) consecutive_failures: Arc<AtomicU32>,
    pub(crate) fallback_enabled: bool,
    pub(crate) key_index: Mutex<KeyIndex>,
    pub(crate) shutdown: Arc<tokio::sync::watch::Sender<bool>>,
    pub(crate) on_fallback: Arc<dyn Fn(&str) + Send + Sync>,
    pub(crate) pool_index: Option<Mutex<crate::gateway::route::PoolIndex>>,
    pub(crate) gateway_default_connection: Option<LiveSetting<Option<String>>>,
    pub(crate) affinity: Option<Mutex<crate::gateway::route::Affinity>>,
    pub(crate) cooldowns: Option<Mutex<crate::gateway::route::Cooldowns>>,
    pub(crate) claude_sessions: Option<Mutex<ClaudeSessions>>,
    pub(crate) circuit_threshold: u32,
    pub(crate) live_circuit_threshold: Option<LiveSetting<u32>>,
    pub(crate) on_circuit: Arc<dyn Fn(&str) + Send + Sync>,
    pub(crate) on_activity: crate::gateway::activity::Emit,
    pub(crate) on_quota_stale: Arc<dyn Fn(&str) + Send + Sync>,
    pub(crate) gateway_auth_relaxed: bool,
    pub(crate) gateway_lan_shared: bool,
    pub(crate) request_uids: crate::gateway::activity::UidBlock,
}

impl crate::gateway::route::RouteDeps for ProxyDeps {
    fn db(&self) -> &Mutex<rusqlite::Connection> {
        &self.db
    }
    fn opt_pool_index(&self) -> Option<&Mutex<crate::gateway::route::PoolIndex>> {
        self.pool_index.as_ref()
    }
}

impl crate::gateway::route::RouteDeps for Arc<ProxyDeps> {
    fn db(&self) -> &Mutex<rusqlite::Connection> {
        &self.db
    }
    fn opt_pool_index(&self) -> Option<&Mutex<crate::gateway::route::PoolIndex>> {
        self.pool_index.as_ref()
    }
}

impl ProxyDeps {
    pub(crate) fn pool_index(&self) -> Result<&Mutex<crate::gateway::route::PoolIndex>, ()> {
        self.pool_index.as_ref().ok_or(())
    }

    pub(crate) fn gateway_default_connection(&self) -> Option<String> {
        self.gateway_default_connection.as_ref().and_then(|read| read())
    }

    pub(crate) fn circuit_threshold(&self) -> u32 {
        self.live_circuit_threshold
            .as_ref()
            .map(|read| read())
            .unwrap_or(self.circuit_threshold)
    }

    pub(crate) fn upstream_for(&self, platform: Option<ProxyPlatform>) -> Option<(ProxyPlatform, &str)> {
        if let Some(platform) = platform {
            return self
                .upstreams
                .get(&platform)
                .map(|base| (platform, base.as_str()));
        }
        if self.upstreams.len() == 1 {
            return self
                .upstreams
                .iter()
                .next()
                .map(|(p, base)| (*p, base.as_str()));
        }
        None
    }

    pub(crate) fn connection_for(&self, platform: ProxyPlatform, headers: &HeaderMap) -> Option<String> {
        let credential = request_credential(platform, headers)?;
        let digest = fingerprint(&credential);
        let mut index = self.key_index.lock().ok()?;
        let now = now_ms();
        if now - index.refreshed_ms > KEY_INDEX_TTL_MS || index.refreshed_ms == 0 {
            if let Ok(conn) = self.db.try_lock() {
                if let Ok(rows) = crate::connections::api_key_secrets(&conn) {
                    index.entries = rows
                        .into_iter()
                        .filter_map(|(id, platform, secret)| {
                            let platform = match platform.as_str() {
                                "claude" => ProxyPlatform::Claude,
                                "codex" => ProxyPlatform::Codex,
                                _ => return None,
                            };
                            Some((platform, fingerprint(&secret), id))
                        })
                        .collect();
                    index.refreshed_ms = now;
                }
            }
        }
        index
            .entries
            .iter()
            .find(|(p, f, _)| *p == platform && *f == digest)
            .map(|(_, _, id)| id.clone())
    }

    pub(crate) fn settle_timing(
        &self,
        platform: ProxyPlatform,
        head: &[u8],
        first: i64,
        duration: i64,
        status: i64,
        connection_id: Option<&str>,
    ) {
        let Some(dedup_key) = extract_dedup_key(platform, head) else {
            return;
        };
        let served_model = extract_served_model(head);
        let applied = match self.db.try_lock() {
            Ok(conn) => crate::db::apply_proxy_timing_with_models(
                &conn,
                &dedup_key,
                Some(first),
                duration,
                status,
                connection_id,
                None,
                served_model.as_deref(),
            )
            .unwrap_or(0),
            Err(_) => 0,
        };
        if applied == 0 {
            if let Ok(mut queue) = self.pending.lock() {
                queue.push(PendingTiming {
                    dedup_key,
                    first_token_ms: first,
                    duration_ms: duration,
                    status_code: status,
                    created_ms: now_ms(),
                    connection_id: connection_id.map(str::to_string),
                    upstream_model: None,
                    served_model,
                });
            }
        }
    }

    pub(crate) fn note_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        if let Ok(mut slot) = LAST_ERROR.lock() {
            *slot = None;
        }
    }

    pub(crate) fn note_upstream_unreachable(&self, reason: String) {
        if let Ok(mut slot) = LAST_ERROR.lock() {
            *slot = Some(reason.clone());
        }
        if !self.fallback_enabled {
            return;
        }
        let count = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= FALLBACK_THRESHOLD {
            self.consecutive_failures.store(0, Ordering::Relaxed);
            (self.on_fallback)(&reason);
            let _ = self.shutdown.send(true);
        }
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn elapsed_ms(t0: Instant) -> i64 {
    t0.elapsed().as_millis().min(i64::MAX as u128) as i64
}

pub(crate) fn detect_platform(headers: &HeaderMap, path: &str) -> Option<ProxyPlatform> {
    if headers.contains_key("anthropic-version")
        || headers.contains_key("x-api-key")
        || headers.contains_key("anthropic-beta")
    {
        return Some(ProxyPlatform::Claude);
    }
    if headers.contains_key("chatgpt-account-id")
        || headers
            .get("originator")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("codex"))
    {
        return Some(ProxyPlatform::Codex);
    }
    let path = path.to_ascii_lowercase();
    if path.starts_with("/v1/messages")
        || path.starts_with("/v1/complete")
        || path.starts_with("/api/")
    {
        return Some(ProxyPlatform::Claude);
    }
    if path.starts_with("/v1/responses")
        || path.starts_with("/v1/chat/completions")
        || path.starts_with("/v1/completions")
        || path.starts_with("/v1/embeddings")
        || path.starts_with("/backend-api")
    {
        return Some(ProxyPlatform::Codex);
    }
    None
}

fn trim_ws(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[start..end]
}

pub(crate) fn extract_dedup_key(platform: ProxyPlatform, head: &[u8]) -> Option<String> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(head) {
        if let Some(key) = json_id(platform, &value) {
            return Some(key);
        }
    }
    for line in head.split(|byte| *byte == b'\n') {
        let line = trim_ws(line);
        let Some(payload) = line.strip_prefix(b"data:").map(trim_ws) else {
            continue;
        };
        if payload.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload) {
            if let Some(key) = json_id(platform, &value) {
                return Some(key);
            }
        }
    }
    None
}

pub(crate) fn extract_served_model(head: &[u8]) -> Option<String> {
    let pick = |value: &serde_json::Value| {
        value
            .pointer("/message/model")
            .or_else(|| value.pointer("/response/model"))
            .or_else(|| value.get("model"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(head) {
        if let Some(model) = pick(&value) {
            return Some(model);
        }
    }
    for line in head.split(|byte| *byte == b'\n') {
        let line = trim_ws(line);
        let Some(payload) = line.strip_prefix(b"data:").map(trim_ws) else {
            continue;
        };
        if payload.is_empty() || payload == b"[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(payload) {
            if let Some(model) = pick(&value) {
                return Some(model);
            }
        }
    }
    None
}

fn json_id(platform: ProxyPlatform, value: &serde_json::Value) -> Option<String> {
    let candidate = match platform {
        ProxyPlatform::Claude => value
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("message").and_then(|m| m.get("id")).and_then(|v| v.as_str()))
            .or_else(|| value.get("response").and_then(|r| r.get("id")).and_then(|v| v.as_str())),
        ProxyPlatform::Codex => value
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("response").and_then(|r| r.get("id")).and_then(|v| v.as_str()))
            .or_else(|| value.get("message").and_then(|m| m.get("id")).and_then(|v| v.as_str())),
    };
    candidate
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 采集扫描完成后补写待关联的计时（在已持有数据库锁的上下文里调用）。
pub fn flush_pending(conn: &rusqlite::Connection) {
    let Ok(mut queue) = PENDING.lock() else {
        return;
    };
    if queue.is_empty() {
        return;
    }
    let _ = flush_entries(conn, &mut queue);
}

pub(crate) fn flush_entries(
    conn: &rusqlite::Connection,
    queue: &mut Vec<PendingTiming>,
) -> rusqlite::Result<()> {
    let cutoff = now_ms() - PENDING_TTL_MS;
    queue.retain(|entry| entry.created_ms >= cutoff);
    let mut remaining = Vec::new();
    std::mem::swap(queue, &mut remaining);
    for entry in remaining {
        let applied = crate::db::apply_proxy_timing_with_models(
            conn,
            &entry.dedup_key,
            (entry.first_token_ms >= 0).then_some(entry.first_token_ms),
            entry.duration_ms,
            entry.status_code,
            entry.connection_id.as_deref(),
            entry.upstream_model.as_deref(),
            entry.served_model.as_deref(),
        )
        .unwrap_or(0);
        if applied == 0 && entry.created_ms >= cutoff {
            queue.push(entry);
        }
    }
    Ok(())
}

/// 测试支持：给 `gateway` 模块的单测构造最小可用的 deps。
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    pub fn bare_deps() -> Arc<ProxyDeps> {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let (shutdown_tx, _rx) = tokio::sync::watch::channel(false);
        Arc::new(ProxyDeps {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            db: Arc::new(Mutex::new(db)),
            upstreams: HashMap::new(),
            pending: Arc::new(Mutex::new(Vec::new())),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            fallback_enabled: false,
            key_index: Mutex::new(KeyIndex::default()),
            shutdown: Arc::new(shutdown_tx),
            on_fallback: Arc::new(|_| {}),
            pool_index: Some(Mutex::new(crate::gateway::route::PoolIndex::new())),
            gateway_default_connection: None,
            affinity: Some(Mutex::new(crate::gateway::route::Affinity::new())),
            cooldowns: Some(Mutex::new(crate::gateway::route::Cooldowns::new())),
            claude_sessions: Some(Mutex::new(ClaudeSessions::new())),
            circuit_threshold: 5,
            live_circuit_threshold: None,
            on_circuit: Arc::new(|_| {}),
            on_activity: Arc::new(|_, _| {}),
            on_quota_stale: Arc::new(|_| {}),
            gateway_auth_relaxed: false,
            gateway_lan_shared: false,
            request_uids: Default::default(),
        })
    }

    pub(crate) fn lan_shared_deps() -> Arc<ProxyDeps> {
        let base = bare_deps();
        let mut deps = Arc::try_unwrap(base).ok().expect("新建的 deps 只有一个引用");
        deps.gateway_lan_shared = true;
        Arc::new(deps)
    }
}
