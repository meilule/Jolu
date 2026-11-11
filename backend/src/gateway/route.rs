//! 网关路由：slug 索引、模型解析、协议匹配。
//!
//! slug 是**持久化**的菜单 id 前缀（`connections.slug`），改名不再改变它；
//! 冷却按账号指纹（`account_fp`）计数，同一把 Key 的多个端点共享冷却。
//! 路由与接线代码不读取任何分组表——分组只影响菜单顺序。

use std::sync::{Arc, Mutex};

use crate::connections;

/// 模型池里的一条连接（快照，含凭证——**绝不可序列化给前端或写日志**）。
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct PoolEntry {
    pub connection_id: String,
    /// 持久化 slug；模型 id 的前缀。
    pub slug: String,
    /// 来源名称，仅用于菜单 display_name 与错误文案。
    pub name: String,
    pub platform: String,
    pub kind: String,
    /// anthropic / openai：该连接上游说的协议。
    pub protocol: String,
    /// 上游 base（无尾斜杠）；auth 连接为 None（订阅适配器自定端点）。
    pub base_url: Option<String>,
    /// 凭证原值；订阅适配器只按 connection_id 读取所选连接的完整授权。
    pub secret: Option<String>,
    /// 账号指纹；没有凭证的行为 None。
    pub account_fp: Option<String>,
    /// 手动保存的默认模型。
    pub model: Option<String>,
    pub status: String,
}

impl PoolEntry {
    /// 冷却与去重的键：优先账号指纹，缺失时退回连接 id。
    pub(crate) fn cred_key(&self) -> String {
        self.account_fp
            .clone()
            .unwrap_or_else(|| format!("id:{}", self.connection_id))
    }
}

/// 菜单快照不能授权调用：发出请求前复查来源，关闭、切号或换地址立即生效。
pub(crate) fn source_is_current(deps: &crate::proxy::ProxyDeps, entry: &PoolEntry) -> bool {
    let current = deps.db.lock().ok().and_then(|conn| {
        conn.query_row(
            "SELECT serve_enabled, platform, kind, slug, status, account_fp, secret, base_url, protocol
             FROM connections WHERE id=?1",
            [&entry.connection_id],
            |r| {
                let enabled: bool = r.get(0)?;
                let platform: String = r.get(1)?;
                let kind: String = r.get(2)?;
                let slug: Option<String> = r.get(3)?;
                let status: String = r.get(4)?;
                let fingerprint: Option<String> = r.get(5)?;
                let secret: Option<String> = r.get(6)?;
                let base: Option<String> = r.get(7)?;
                let protocol: Option<String> = r.get(8)?;
                Ok(enabled && platform == entry.platform && kind == entry.kind
                    && slug.as_deref() == Some(entry.slug.as_str()) && status == entry.status
                    && fingerprint == entry.account_fp
                    && (kind != "api" || (secret == entry.secret
                        && base.map(|s| s.trim().trim_end_matches('/').to_owned()) == entry.base_url
                        && connections::auth_protocol(&platform, protocol.as_deref()) == entry.protocol)))
            },
        ).ok()
    }).unwrap_or(false);
    if !current {
        if let Ok(index) = deps.pool_index() {
            if let Ok(mut index) = index.lock() { index.refreshed_ms = 0; }
        }
    }
    current
}

/// slug 索引缓存。
pub(crate) struct PoolIndex {
    refreshed_ms: i64,
    entries: Vec<PoolEntry>,
}

const POOL_TTL_MS: i64 = 30_000;

impl PoolIndex {
    pub(crate) fn new() -> Self {
        Self { refreshed_ms: 0, entries: Vec::new() }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 从数据库读出模型池（serve_enabled=1 的连接）。
/// slug 以列为准；列缺失（迁移未跑完）或为空时跳过并记诊断，绝不再按名字现算——
/// 现算会让改名悄悄改掉菜单 id，已经存进 agent 配置的 id 就失效了。
pub(crate) fn load_pool(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<PoolEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, slug, name, platform, kind, protocol, base_url, secret, account_fp, model, status
         FROM connections WHERE serve_enabled = 1 ORDER BY created_ms ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            PoolEntry {
                connection_id: r.get(0)?,
                slug: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                name: r.get(2)?,
                platform: r.get(3)?,
                kind: r.get(4)?,
                protocol: String::new(),
                base_url: r.get(6)?,
                secret: r.get(7)?,
                account_fp: r.get(8)?,
                model: r.get(9)?,
                status: r.get(10)?,
            },
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (mut entry, declared_protocol) = row?;
        if entry.slug.trim().is_empty() {
            eprintln!("[网关] 连接 {} 缺少 slug，已跳过；请重启应用完成迁移", entry.connection_id);
            continue;
        }
        entry.protocol = connections::auth_protocol(&entry.platform, declared_protocol.as_deref()).to_string();
        entry.base_url = entry.base_url.map(|b| b.trim().trim_end_matches('/').to_string());
        out.push(entry);
    }
    Ok(out)
}

/// 解析请求里的模型 id：`slug/model` → (slug, 模型名)；无斜杠时为 None。
pub(crate) fn split_model_id(model: &str) -> Option<(&str, &str)> {
    let (slug, rest) = model.split_once('/')?;
    let slug = slug.trim();
    let rest = rest.trim();
    if slug.is_empty() || rest.is_empty() {
        return None;
    }
    Some((slug, rest))
}

/// 旧 slug 的说明：命中历史记录时给出「现在叫什么 / 已删除」，而不是落到别的来源上。
pub(crate) fn historical_slug_message(conn: &rusqlite::Connection, slug: &str) -> Option<String> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT c.slug, c.name FROM slug_history h
             LEFT JOIN connections c ON c.id = h.connection_id
             WHERE h.slug = ?1 ORDER BY h.retired_ms DESC LIMIT 1",
            [slug],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    match row {
        Some((Some(current), Some(name))) => Some(format!(
            "没有名为「{slug}」的来源；它现在叫「{current}」（{name}），请改用「{current}/…」"
        )),
        Some(_) => Some(format!("来源「{slug}」已删除，请重新添加或在模型入口选择其他来源")),
        None => None,
    }
}

/// 成员已用量（路由分组 usage 策略用）：按来源 slug 数流水条数。
/// 只数已发出的请求（rejected 是没发出去的，不算消耗）。
pub(crate) fn member_usage(conn: &rusqlite::Connection, slug: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM gateway_route_log WHERE source = ?1 AND result = 'ok'",
        [slug],
        |r| r.get(0),
    )
}

/// 会话亲和表：会话键 → 已应答的连接 id。
/// prompt cache 有前缀一致性——换连接等于缓存全失效按写重算，所以会话黏住首个应答的连接
/// （对齐 magpie 的 Stays 语义）。LRU 上限 [`AFFINITY_CAP`]，进程重启自然清空。
pub(crate) struct Affinity {
    entries: std::collections::HashMap<String, (String, i64)>,
}

pub(crate) const AFFINITY_CAP: usize = 1000;

impl Affinity {
    pub(crate) fn new() -> Self {
        Self { entries: std::collections::HashMap::new() }
    }

    pub(crate) fn get(&self, session: &str) -> Option<&str> {
        self.entries.get(session).map(|(id, _)| id.as_str())
    }

    /// 记录绑定；超上限时淘汰最久未使用的条目。
    pub(crate) fn bind(&mut self, session: &str, connection_id: &str, now: i64) {
        if self.entries.len() >= AFFINITY_CAP && !self.entries.contains_key(session) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(session.into(), (connection_id.into(), now));
    }
}

/// 连接冷却表：按账号指纹计数（429 后短期内不再把新请求派给该账号）。
pub(crate) struct Cooldowns {
    until: std::collections::HashMap<String, i64>,
}

impl Cooldowns {
    pub(crate) fn new() -> Self {
        Self { until: std::collections::HashMap::new() }
    }

    pub(crate) fn cool(&mut self, cred_key: &str, until_ms: i64) {
        self.until.insert(cred_key.into(), until_ms);
    }

    pub(crate) fn remaining_ms(&self, cred_key: &str, now: i64) -> i64 {
        self.until.get(cred_key).map_or(0, |until| (*until - now).max(0))
    }
}

/// 拒绝原因的类型，决定返回给调用方的 HTTP 状态码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectKind {
    BadRequest,
    NotFound,
    Unauthorized,
    PaymentRequired,
    TooManyRequests,
    ServiceUnavailable,
}

/// 一次选路的结果。
#[derive(Debug)]
pub(crate) enum RouteDecision<'a> {
    /// 命中连接：按其协议直通（direct 适配器）。
    Direct(&'a PoolEntry, String),
    /// 命中订阅连接：走订阅适配器。
    Subscription(&'a PoolEntry),
    /// 无法服务：原因直接面向调用方（中文，说明下一步怎么做）。
    Reject { kind: RejectKind, message: String, retry_after: Option<u64> },
}

impl RouteDecision<'_> {
    fn reject(kind: RejectKind, message: impl Into<String>) -> Self {
        Self::Reject { kind, message: message.into(), retry_after: None }
    }
}

/// 旧 slug 查询：只在 slug 未命中时调用，避免每次请求都读历史表。
pub(crate) type SlugHistoryLookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// 裸模型名 → 来源 slug 的目录查询（magpie `provider::Resolve` 的裸名分支）。
/// 只在模型 id 不带前缀时调用；实现方按目录顺序返回**第一个**暴露该模型的来源。
pub(crate) type BareModelLookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// Claude Code 的 1M 上下文窗口标记：`settings.json` 里的模型名可能带 `[1m]`/`[1M]`，
/// 上游不认识这个后缀。路由前剥掉；工具配置侧（wiring）负责在写入时按需补回。
/// 大小写不敏感——官方写小写，用户手写常变大写。
pub(crate) fn canonical_model(model: &str) -> &str {
    // 两端去空白（对齐 magpie validateModel 的 TrimSpace），再剥尾部标记
    let trimmed = model.trim();
    if trimmed.len() > 4 && trimmed.as_bytes()[trimmed.len() - 4..].eq_ignore_ascii_case(b"[1m]") {
        trimmed[..trimmed.len() - 4].trim_end()
    } else {
        trimmed
    }
}

/// 发请求的 agent 自己的订阅平台：Claude Code 的 Claude 账号、Codex 的 ChatGPT 账号等。
fn own_platforms(agent: &str) -> &'static [&'static str] {
    match agent {
        "claude-code" => &["claude"],
        "codex" => &["codex"],
        "gemini" => &["gemini"],
        "opencode" => &["opencode"],
        "zcode" => &["zcode"],
        _ => &[],
    }
}

/// 裸模型名选哪个来源：发请求的 agent 自己的订阅能服务这个名字时先给它，否则目录里第一个。
///
/// 为什么：Codex 自己登录着 ChatGPT、却因目录顺序把裸名 `gpt-6` 送去了别家（magpie freefcw 的
/// 现场）——本该自己的订阅服务的请求，被白白送给池子里的第三方。参照 magpie `608c154`
/// （`ownFirst` / `provider.OwnRoute`）；差异：Jolu 没有「自动发现分组」，所以没有 magpie
/// 「分组关闭时多家都能服务就拒绝」那一条。带来源前缀的 id 与 `group/…` 不经过这里。
///
/// `entries`：（来源 slug，平台，种类 `api` / `auth`，模型名）。
pub(crate) fn pick_bare<'a>(entries: impl IntoIterator<Item = (&'a str, Option<&'a str>, &'a str, &'a str)>, name: &str, agent: Option<&str>) -> Option<String> {
    let own = agent.map(own_platforms).unwrap_or(&[]);
    let mut first: Option<&str> = None;
    for (slug, platform, kind, model) in entries {
        if model != name {
            continue;
        }
        if kind != "api" && platform.is_some_and(|p| own.contains(&p)) {
            return Some(slug.to_string());
        }
        first.get_or_insert(slug);
    }
    first.map(str::to_string)
}

/// 选路：模型 id → slug 命中 → 协议检查 → 按连接类型分发。
///
/// 模型 id 的解析顺序（对齐 magpie `provider::Resolve`）：
/// 1. 剥 `[1m]` 标记；
/// 2. 带 `<slug>/` 前缀 → 按前缀精确路由；前缀一半为空是明确的请求错误；
/// 3. 裸模型名 → 目录里第一个暴露它的来源（`bare`）；
/// 4. 目录未命中 → 默认来源兜底；
/// 5. 都没有 → 报错并说明两条设置路径。
pub(crate) fn decide<'a>(
    entries: &'a [PoolEntry],
    model: &str,
    default_connection: Option<&str>,
    history: &SlugHistoryLookup<'_>,
    bare: &BareModelLookup<'_>,
) -> RouteDecision<'a> {
    let model = canonical_model(model);
    if model.contains('/') {
        // 前缀一半为空不是「裸名」，是写错了：与 magpie 同样明确拒绝
        let (head, tail) = model.split_once('/').expect("contains('/') 已判定");
        if head.trim().is_empty() || tail.trim().is_empty() {
            return RouteDecision::reject(
                RejectKind::BadRequest,
                format!("模型「{model}」格式无效：来源与模型两部分都不能为空"),
            );
        }
    }
    let (slug, model_name) = match split_model_id(model) {
        Some(parts) => parts,
        None => {
            // 裸模型名：目录里第一个暴露它的来源优先（对齐 magpie），再落默认来源兜底
            if let Some(slug) = bare(model) {
                if let Some(entry) = entries.iter().find(|e| e.slug == slug) {
                    if let Some(reject) = status_reject(entry, None) {
                        return reject;
                    }
                    return classify(entry, model.trim().to_string());
                }
            }
            let Some(id) = default_connection else {
                return RouteDecision::reject(
                    RejectKind::BadRequest,
                    format!(
                        "模型「{model}」未带来源前缀：请使用「来源/模型」形式，或先在模型入口设置中选择默认来源"
                    ),
                );
            };
            let Some(entry) = entries.iter().find(|e| e.connection_id == id) else {
                // 分两种情形给不同指引：默认来源被删了 vs 它只是被移出了模型池
                let known: Vec<&str> = entries.iter().map(|e| e.slug.as_str()).collect();
                let message = if known.is_empty() {
                    "默认来源不在模型池中，且当前池里没有任何来源：请在「管理来源」里把至少一个来源设为对外服务"
                        .to_string()
                } else {
                    format!(
                        "默认来源不在模型池中：请在「管理来源」里把它重新加入；当前池中可用来源：{}",
                        known.join("、")
                    )
                };
                return RouteDecision::reject(RejectKind::ServiceUnavailable, &message);
            };
            if let Some(reject) = status_reject(entry, None) {
                return reject;
            }
            return classify(entry, model.trim().to_string());
        }
    };
    let Some(entry) = entries.iter().find(|e| e.slug == slug) else {
        if let Some(message) = history(slug) {
            return RouteDecision::reject(RejectKind::NotFound, message);
        }
        let known: Vec<String> = entries.iter().map(|e| e.slug.clone()).collect();
        return RouteDecision::reject(
            if known.is_empty() { RejectKind::BadRequest } else { RejectKind::NotFound },
            if known.is_empty() {
                "模型池为空：先在模型入口中把至少一个来源设为对外服务".to_string()
            } else {
                format!("没有名为「{slug}」的来源；可用的来源：{}", known.join("、"))
            },
        );
    };
    if let Some(reject) = status_reject(entry, None) {
        return reject;
    }
    classify(entry, model_name.to_string())
}

/// 非 connected 状态按原因给出不同的状态码，让 agent 能正确退避或提示用户。
fn status_reject(entry: &PoolEntry, retry_after: Option<u64>) -> Option<RouteDecision<'static>> {
    let (kind, message) = match entry.status.as_str() {
        "paused" => (
            RejectKind::ServiceUnavailable,
            format!("来源「{}」已暂停，请在供应商页恢复后再试", entry.slug),
        ),
        "invalid" | "expired" => (
            RejectKind::Unauthorized,
            format!("来源「{}」的登录已失效或过期，请在供应商页重新登录", entry.slug),
        ),
        "exhausted" => (
            RejectKind::PaymentRequired,
            format!("来源「{}」的额度已用尽，请切换其他来源或等待恢复", entry.slug),
        ),
        _ => return None,
    };
    Some(RouteDecision::Reject { kind, message, retry_after })
}

fn classify(entry: &PoolEntry, model_name: String) -> RouteDecision<'_> {
    if entry.kind == "api" {
        if entry.base_url.as_deref().map_or(true, |b| b.is_empty()) {
            return RouteDecision::reject(
                RejectKind::ServiceUnavailable,
                format!("来源「{}」缺少上游地址，无法直通", entry.slug),
            );
        }
        if entry.secret.as_deref().map_or(true, |s| s.is_empty()) {
            return RouteDecision::reject(
                RejectKind::ServiceUnavailable,
                format!("来源「{}」缺少凭证，无法直通", entry.slug),
            );
        }
        RouteDecision::Direct(entry, model_name)
    } else {
        RouteDecision::Subscription(entry)
    }
}

/// 带会话亲和的选路（主入口）。
/// slug 是调用方的明确意图：换 slug = 故意换模型，亲和不覆盖它。
/// 亲和的真实作用：请求本应命中会话已绑定的连接、而该账号正处 429 冷却时，
/// **不切换**——换连接等于 prompt cache 全失效按写重算（对齐 magpie 的 Stays 语义）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_with_affinity<'a>(
    entries: &'a [PoolEntry],
    model: &str,
    default_connection: Option<&str>,
    session: Option<&str>,
    affinity: &mut Affinity,
    cooldowns: &Cooldowns,
    history: &SlugHistoryLookup<'_>,
    bare: &BareModelLookup<'_>,
) -> RouteDecision<'a> {
    let now = now_ms();
    let decision = decide(entries, model, default_connection, history, bare);
    let target = match &decision {
        RouteDecision::Direct(entry, _) | RouteDecision::Subscription(entry) => Some(entry),
        RouteDecision::Reject { .. } => None,
    };
    let Some(entry) = target else { return decision };
    // 冷却按账号指纹：单来源不换别家；会话已绑定时更要说清楚为什么不切换。
    let remaining = cooldowns.remaining_ms(&entry.cred_key(), now);
    if remaining > 0 {
        let seconds = remaining as u64 / 1000 + 1;
        let bound = session.is_some_and(|session| affinity.get(session) == Some(entry.connection_id.as_str()));
        let message = if bound {
            format!(
                "会话固定的来源「{}」已被限流，约 {seconds} 秒后恢复；为保留提示词缓存本会话不自动切换来源",
                entry.slug
            )
        } else {
            format!("来源「{}」正在限流，约 {seconds} 秒后恢复", entry.slug)
        };
        return RouteDecision::Reject {
            kind: RejectKind::TooManyRequests,
            message,
            retry_after: Some(seconds),
        };
    }
    if let Some(session) = session {
        // 命中即绑定（含续期）：调用方换 slug 换了连接，会话跟着走
        affinity.bind(session, &entry.connection_id, now);
    }
    decision
}

/// 模型池快照（带 TTL 缓存）。锁被采集占用时沿用旧快照，与观测模式 KeyIndex 一致。
/// 因 402 标为额度用尽、且不是今天标的来源，恢复为可用。返回恢复的数量。
///
/// 为什么：额度多按天重置，标上「用尽」后不自动恢复，来源就永远停在不可用，只能手动点恢复
/// （magpie 的 402 `exhausted` 同样在次日零点回来）。手动暂停的不动，旧数据没有日期的不动。
pub(crate) fn recover_exhausted(conn: &rusqlite::Connection, today: &str) -> usize {
    conn.execute(
        "UPDATE connections SET status = 'connected', exhausted_day = NULL
          WHERE status = 'exhausted' AND exhausted_day IS NOT NULL AND exhausted_day < ?1",
        [today],
    )
    .unwrap_or(0)
}

/// 普通来源（非分组）失败时的替补：同平台、同种类、同协议、可用、不同账号、且目录里也有这个模型的
/// 其他连接，最多 `limit` 个。为什么只找同平台：同一个服务的另一个账号行为一致，换去别家模型
/// 会让用户拿到不是自己选的东西。`serves` 判断某来源 slug 的目录里有没有这个模型。
pub(crate) fn same_model_alternates(
    pool: &[PoolEntry],
    primary: &PoolEntry,
    serves: &dyn Fn(&str) -> bool,
    limit: usize,
) -> Vec<PoolEntry> {
    let mut accounts: Vec<&str> = primary.account_fp.as_deref().into_iter().collect();
    let mut out = Vec::new();
    for entry in pool {
        if out.len() >= limit {
            break;
        }
        if entry.connection_id == primary.connection_id
            || entry.platform != primary.platform
            || entry.kind != primary.kind
            || entry.protocol != primary.protocol
            || entry.status != "connected"
            || !serves(&entry.slug)
        {
            continue;
        }
        if let Some(fp) = entry.account_fp.as_deref() {
            if accounts.contains(&fp) {
                continue;
            }
            accounts.push(fp);
        }
        out.push(entry.clone());
    }
    out
}

pub(crate) fn pool_snapshot(
    db: &Arc<Mutex<rusqlite::Connection>>,
    index: &Mutex<PoolIndex>,
) -> Vec<PoolEntry> {
    let now = now_ms();
    if let Ok(guard) = index.lock() {
        if guard.refreshed_ms > 0 && now - guard.refreshed_ms < POOL_TTL_MS {
            return guard.entries.clone();
        }
    }
    if let Ok(conn) = db.try_lock() {
        let recovered = recover_exhausted(&conn, &chrono::Local::now().format("%Y-%m-%d").to_string());
        if recovered > 0 {
            eprintln!("[路由] {recovered} 个额度用尽的来源已到新的一天，恢复可用");
        }
        if let Ok(entries) = load_pool(&conn) {
            if let Ok(mut guard) = index.lock() {
                guard.refreshed_ms = now;
                guard.entries = entries.clone();
            }
            return entries;
        }
    }
    // 拿不到锁或读取失败：返回旧快照（可能为空），下次再刷新
    if let Ok(guard) = index.lock() {
        return guard.entries.clone();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(slug: &str, kind: &str, protocol: &str, status: &str) -> PoolEntry {
        PoolEntry {
            connection_id: format!("c-{slug}"),
            slug: slug.into(),
            name: slug.into(),
            platform: "custom".into(),
            kind: kind.into(),
            protocol: protocol.into(),
            base_url: if kind == "api" { Some("https://gw.test".into()) } else { None },
            secret: if kind == "api" { Some("sk-x".into()) } else { None },
            account_fp: if kind == "api" { Some(format!("fp-{slug}")) } else { None },
            model: None,
            status: status.into(),
        }
    }

    fn no_history(_: &str) -> Option<String> {
        None
    }
    #[test]
    fn a_bare_name_goes_to_the_asking_agents_own_subscription_first() {
        let catalog = [
            ("relay", Some("custom"), "api", "gpt-6"),
            ("chatgpt-me", Some("codex"), "auth", "gpt-6"),
            ("claude-me", Some("claude"), "auth", "claude-opus-5-5"),
        ];
        assert_eq!(pick_bare(catalog, "gpt-6", Some("codex")).as_deref(), Some("chatgpt-me"), "Codex 自己的 ChatGPT 订阅先");
        assert_eq!(pick_bare(catalog, "gpt-6", Some("claude-code")).as_deref(), Some("relay"), "别的 agent 仍按目录顺序");
        assert_eq!(pick_bare(catalog, "gpt-6", None).as_deref(), Some("relay"), "认不出的客户端按目录顺序");
        assert_eq!(pick_bare(catalog, "nope", Some("codex")), None);
        // 同平台的 API Key 来源不算「自己的订阅」
        let keys = [("relay", Some("custom"), "api", "m"), ("codex-key", Some("codex"), "api", "m")];
        assert_eq!(pick_bare(keys, "m", Some("codex")).as_deref(), Some("relay"));
    }

    #[test]
    fn exhausted_sources_come_back_the_next_day_but_not_the_same_day() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE connections (id TEXT, status TEXT, exhausted_day TEXT);
             INSERT INTO connections VALUES ('yesterday','exhausted','2026-11-10');
             INSERT INTO connections VALUES ('today','exhausted','2026-11-11');
             INSERT INTO connections VALUES ('legacy','exhausted',NULL);
             INSERT INTO connections VALUES ('paused','paused','2026-11-01');",
        )
        .unwrap();
        assert_eq!(recover_exhausted(&conn, "2026-11-11"), 1);
        let status = |id: &str| conn.query_row("SELECT status FROM connections WHERE id = ?1", [id], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(status("yesterday"), "connected");
        assert_eq!(status("today"), "exhausted", "当天标的不恢复");
        assert_eq!(status("legacy"), "exhausted", "没有日期的旧数据不动");
        assert_eq!(status("paused"), "paused");
    }

    #[test]
    fn alternates_are_other_accounts_of_the_same_platform_serving_the_model() {
        let mut primary = entry("kimi-a", "api", "anthropic", "connected");
        primary.account_fp = Some("fp-a".into());
        let mut same_account = entry("kimi-a2", "api", "anthropic", "connected");
        same_account.account_fp = Some("fp-a".into());
        let mut other = entry("kimi-b", "api", "anthropic", "connected");
        other.account_fp = Some("fp-b".into());
        let mut lacks_model = entry("kimi-c", "api", "anthropic", "connected");
        lacks_model.account_fp = Some("fp-c".into());
        let mut paused = entry("kimi-d", "api", "anthropic", "paused");
        paused.account_fp = Some("fp-d".into());
        let mut other_platform = entry("glm", "api", "anthropic", "connected");
        other_platform.platform = "zcode".into();
        let mut third = entry("kimi-e", "api", "anthropic", "connected");
        third.account_fp = Some("fp-e".into());
        let mut fourth = entry("kimi-f", "api", "anthropic", "connected");
        fourth.account_fp = Some("fp-f".into());
        let pool = vec![primary.clone(), same_account, other, lacks_model, paused, other_platform, third, fourth];
        let serves = |slug: &str| slug != "kimi-c";
        let picked: Vec<String> = same_model_alternates(&pool, &primary, &serves, 2).into_iter().map(|e| e.slug).collect();
        assert_eq!(picked, vec!["kimi-b".to_string(), "kimi-e".to_string()], "跳过同账号、缺模型、暂停、别家平台，最多 2 个");
    }

    fn no_bare(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn canonical_model_strips_1m_marker() {
        // Claude Code 的 1M 窗口标记：settings 里常见小写，用户手写常变大写；上游不认识
        assert_eq!(canonical_model("GLM-5.3-Flash[1m]"), "GLM-5.3-Flash");
        assert_eq!(canonical_model("GLM-5.3-Flash[1M]"), "GLM-5.3-Flash");
        assert_eq!(canonical_model("zcode/GLM-5.3-Flash[1m]"), "zcode/GLM-5.3-Flash");
        assert_eq!(canonical_model("glm-5.3"), "glm-5.3", "没有标记就不动");
        assert_eq!(canonical_model("[1m]"), "[1m]", "只有标记本身不算标记，避免剥成空串");
        assert_eq!(canonical_model("  ab[1m]  "), "ab", "尾部空白随标记一起去掉");
    }

    #[test]
    fn bare_model_resolves_via_catalog_before_default_source() {
        // magpie 的裸名语义：目录里第一个暴露该模型的来源优先，默认来源只是兜底
        let pool = vec![
            entry("zcode", "auth", "anthropic", "connected"),
            entry("kimi", "api", "anthropic", "connected"),
        ];
        let catalog = |name: &str| -> Option<String> {
            // 目录顺序：zcode 在前。zcode 不暴露 kimi-k2，kimi 暴露
            if name == "GLM-5.3-Flash" { Some("zcode".into()) } else { None }
        };
        match decide(&pool, "GLM-5.3-Flash", Some("c-kimi"), &no_history, &catalog) {
            RouteDecision::Subscription(e) => assert_eq!(e.slug, "zcode", "目录命中优先于默认来源"),
            other => panic!("应经目录解析到 zcode，实际 {other:?}"),
        }
        // 目录未命中 → 默认来源兜底
        match decide(&pool, "other-model", Some("c-kimi"), &no_history, &catalog) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "kimi");
                assert_eq!(m, "other-model");
            }
            other => panic!("目录未命中应落默认来源，实际 {other:?}"),
        }
    }

    #[test]
    fn half_prefix_model_is_a_bad_request_not_a_bare_name() {
        // 「/model」「slug/」是写错的前缀，不能当成裸名去目录里撞——
        // 那会把请求静默路由到不相干的来源。
        let pool = vec![entry("kimi", "api", "anthropic", "connected")];
        for bad in ["/kimi-k2", "kimi/"] {
            match decide(&pool, bad, Some("c-kimi"), &no_history, &no_bare) {
                RouteDecision::Reject { kind, message, .. } => {
                    assert_eq!(kind, RejectKind::BadRequest);
                    assert!(message.contains("不能为空"), "{bad}: {message}");
                }
                other => panic!("{bad} 应明确报格式错误，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn prefixed_model_with_1m_marker_strips_before_upstream() {
        // 带前缀 + [1m]：前缀用于选路，剥掉标记后的名字才是发给上游的
        let pool = vec![entry("zcode", "auth", "anthropic", "connected")];
        match decide(&pool, "zcode/GLM-5.3-Flash[1M]", None, &no_history, &no_bare) {
            RouteDecision::Subscription(e) => assert_eq!(e.slug, "zcode"),
            other => panic!("{other:?}"),
        }
        // 上游拿到的模型名经 RouteDecision 验证：Direct 分支带 upstream_model，
        // Subscription 的剥前缀在 classify 内完成，这里验证路由本身不因标记失败
    }

    #[test]
    fn unprefixed_model_routes_to_default_source() {
        // 默认来源就是为「工具发不带前缀的模型名」准备的：Claude Code 自带的 claude-*、
        // 用户手写的 GLM-5.3-Flash 都属于这种。没有它这些请求只能被拒。
        let pool = vec![
            entry("zcode", "auth", "anthropic", "connected"),
            entry("kimi", "api", "anthropic", "connected"),
        ];
        match decide(&pool, "GLM-5.3-Flash", Some("c-zcode"), &no_history, &no_bare) {
            RouteDecision::Subscription(e) => assert_eq!(e.slug, "zcode"),
            other => panic!("应兜底到默认来源，实际 {other:?}"),
        }
    }

    #[test]
    fn unprefixed_model_without_default_source_is_rejected_with_actionable_message() {
        let pool = vec![entry("zcode", "auth", "anthropic", "connected")];
        match decide(&pool, "GLM-5.3-Flash", None, &no_history, &no_bare) {
            RouteDecision::Reject { kind, message, .. } => {
                assert_eq!(kind, RejectKind::BadRequest);
                assert!(message.contains("未带来源前缀"), "要说清是前缀问题：{message}");
                assert!(message.contains("默认来源"), "要告诉用户去哪儿设：{message}");
            }
            other => panic!("无默认来源时应拒绝，实际 {other:?}"),
        }
    }

    #[test]
    fn default_source_pointing_at_missing_connection_is_service_unavailable() {
        // 默认来源被删掉后不能静默落到别的来源上，否则请求会打到用户没预期的账号。
        let pool = vec![entry("kimi", "api", "anthropic", "connected")];
        match decide(&pool, "GLM-5.3-Flash", Some("c-gone"), &no_history, &no_bare) {
            RouteDecision::Reject { kind, message, .. } => {
                assert_eq!(kind, RejectKind::ServiceUnavailable);
                // 指引要说明去哪儿修，并列出当前池里可用的来源
                assert!(message.contains("管理来源"), "{message}");
                assert!(message.contains("kimi"), "要列出可用来源：{message}");
            }
            other => panic!("应报默认来源不存在，实际 {other:?}"),
        }
    }

    #[test]
    fn prefixed_model_ignores_default_source() {
        // 明确写了前缀就按前缀走，默认来源不能抢走这条请求。
        let pool = vec![
            entry("zcode", "auth", "anthropic", "connected"),
            entry("kimi", "api", "anthropic", "connected"),
        ];
        match decide(&pool, "kimi/kimi-k2", Some("c-zcode"), &no_history, &no_bare) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "kimi");
                assert_eq!(m, "kimi-k2");
            }
            other => panic!("前缀优先，实际 {other:?}"),
        }
    }

    #[test]
    fn split_model_requires_both_parts() {
        assert_eq!(split_model_id("kimi/kimi-k2"), Some(("kimi", "kimi-k2")));
        assert_eq!(split_model_id("kimi/"), None);
        assert_eq!(split_model_id("/k2"), None);
        assert_eq!(split_model_id("kimi-k2"), None);
    }

    #[test]
    fn decide_routes_direct_subscription_and_rejects() {
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("codex", "auth", "openai", "connected"),
        ];
        match decide(&pool, "kimi/kimi-k2", None, &no_history, &no_bare) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "kimi");
                assert_eq!(m, "kimi-k2");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(decide(&pool, "codex/gpt-5.5", None, &no_history, &no_bare), RouteDecision::Subscription(_)));
        let err = match decide(&pool, "nope/m", None, &no_history, &no_bare) {
            RouteDecision::Reject { message, kind, .. } => {
                assert_eq!(kind, RejectKind::NotFound);
                message
            }
            other => panic!("{other:?}"),
        };
        assert!(err.contains("kimi"), "{err}");
        // 无前缀 + 无默认连接 → 明确指引
        assert!(matches!(
            decide(&pool, "kimi-k2", None, &no_history, &no_bare),
            RouteDecision::Reject { kind: RejectKind::BadRequest, .. }
        ));
        // 无前缀 + 默认连接命中该连接
        match decide(&pool, "kimi-k2", Some("c-kimi"), &no_history, &no_bare) {
            RouteDecision::Direct(_, m) => assert_eq!(m, "kimi-k2"),
            other => panic!("{other:?}"),
        }
        // 暂停的连接明确拒绝，返回 503
        let paused = vec![entry("kimi", "api", "anthropic", "paused")];
        assert!(matches!(
            decide(&paused, "kimi/kimi-k2", None, &no_history, &no_bare),
            RouteDecision::Reject { kind: RejectKind::ServiceUnavailable, .. }
        ));
        // 失效的登录返回 401，额度用尽返回 402
        let invalid = vec![entry("kimi", "api", "anthropic", "invalid")];
        assert!(matches!(
            decide(&invalid, "kimi/kimi-k2", None, &no_history, &no_bare),
            RouteDecision::Reject { kind: RejectKind::Unauthorized, .. }
        ));
        let exhausted = vec![entry("kimi", "api", "anthropic", "exhausted")];
        assert!(matches!(
            decide(&exhausted, "kimi/kimi-k2", None, &no_history, &no_bare),
            RouteDecision::Reject { kind: RejectKind::PaymentRequired, .. }
        ));
    }

    #[test]
    fn historical_lookup_explains_rename_and_deletion() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO connections(id,platform,kind,name,slug,created_ms)
             VALUES('c1','custom','api','Kimi','kimi',1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO slug_history(slug,connection_id,account_fp,retired_ms) VALUES('old-name','c1',NULL,1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO slug_history(slug,connection_id,account_fp,retired_ms) VALUES('gone','deleted-id',NULL,2)",
            [],
        )
        .unwrap();
        let renamed = historical_slug_message(&db, "old-name").unwrap();
        assert!(renamed.contains("现在叫「kimi」"), "{renamed}");
        let deleted = historical_slug_message(&db, "gone").unwrap();
        assert!(deleted.contains("已删除"), "{deleted}");
        assert_eq!(historical_slug_message(&db, "never-existed"), None);
    }

    #[test]
    fn old_slug_returns_404_with_explanation() {
        let pool = vec![entry("kimi", "api", "anthropic", "connected")];
        let history = |slug: &str| {
            (slug == "old").then(|| "没有名为「old」的来源；它现在叫「kimi」".to_string())
        };
        match decide(&pool, "old/kimi-k2", None, &history, &no_bare) {
            RouteDecision::Reject { kind: RejectKind::NotFound, message, .. } => {
                assert!(message.contains("现在叫「kimi」"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn load_pool_reads_persisted_slug_and_fingerprint() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (id, name, slug) in [("c1", "Kimi", "kimi"), ("c2", "Kimi", "kimi-2")] {
            db.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms,slug,account_fp)
                 VALUES(?1,'custom','api',?2,'k','k','https://gw.test','anthropic',1,1,?3,?4)",
                rusqlite::params![id, name, slug, format!("fp-{id}")],
            )
            .unwrap();
        }
        let pool = load_pool(&db).unwrap();
        let slugs: Vec<&str> = pool.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, vec!["kimi", "kimi-2"]);
        assert_eq!(pool[0].name, "Kimi");
        assert_eq!(pool[0].protocol, "anthropic");
    }

    #[test]
    fn affinity_follows_request_slug_and_rebinds() {
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("glm", "api", "anthropic", "connected"),
        ];
        let mut affinity = Affinity::new();
        let cooldowns = Cooldowns::new();
        affinity.bind("s1", "c-kimi", 100);
        // 换 slug = 调用方故意换模型：亲和不覆盖明确意图，改走新连接并重绑
        match decide_with_affinity(&pool, "glm/glm-4.6", None, Some("s1"), &mut affinity, &cooldowns, &no_history, &no_bare) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "glm");
                assert_eq!(m, "glm-4.6");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(affinity.get("s1"), Some("c-glm"), "命中新连接后应重绑会话");
        // 无 slug + 默认连接兜底也照常
        match decide_with_affinity(&pool, "glm-4.6", Some("c-kimi"), Some("s1"), &mut affinity, &cooldowns, &no_history, &no_bare) {
            RouteDecision::Direct(e, _) => assert_eq!(e.slug, "kimi"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn affinity_rejects_instead_of_switching_when_account_rate_limited() {
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("glm", "api", "anthropic", "connected"),
        ];
        let mut affinity = Affinity::new();
        let mut cooldowns = Cooldowns::new();
        affinity.bind("s1", "c-kimi", 100);
        cooldowns.cool(&pool[0].cred_key(), now_ms() + 30_000);
        // 请求命中的正是会话绑定的 kimi，且 kimi 冷却中：拒绝而非悄悄切到 glm
        let decision =
            decide_with_affinity(&pool, "kimi/kimi-k2", None, Some("s1"), &mut affinity, &cooldowns, &no_history, &no_bare);
        let RouteDecision::Reject { kind: RejectKind::TooManyRequests, message, retry_after } = decision else {
            panic!("{decision:?}")
        };
        assert!(message.contains("限流") && message.contains("不自动切换"), "{message}");
        assert!(retry_after.is_some());
        // 未绑定该账号的会话同样按账号冷却拒绝（单来源不换别家）
        let decision =
            decide_with_affinity(&pool, "kimi/kimi-k2", None, Some("s2"), &mut affinity, &cooldowns, &no_history, &no_bare);
        assert!(matches!(decision, RouteDecision::Reject { kind: RejectKind::TooManyRequests, .. }));
        // 冷却过期后恢复服务
        cooldowns.cool(&pool[0].cred_key(), now_ms() - 1);
        assert!(matches!(
            decide_with_affinity(&pool, "kimi/kimi-k2", None, Some("s1"), &mut affinity, &cooldowns, &no_history, &no_bare),
            RouteDecision::Direct(_, _)
        ));
    }

    #[test]
    fn affinity_no_switch_guard_requires_bound_connection() {
        // 会话绑定 kimi，请求指向 glm：即使 kimi 冷却也不拦截 glm（意图明确）
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("glm", "api", "anthropic", "connected"),
        ];
        let mut affinity = Affinity::new();
        let mut cooldowns = Cooldowns::new();
        affinity.bind("s1", "c-kimi", 100);
        cooldowns.cool(&pool[0].cred_key(), now_ms() + 30_000);
        assert!(matches!(
            decide_with_affinity(&pool, "glm/glm-4.6", None, Some("s1"), &mut affinity, &cooldowns, &no_history, &no_bare),
            RouteDecision::Direct(_, _)
        ));
    }

    #[test]
    fn affinity_lru_evicts_oldest_beyond_cap() {
        let mut affinity = Affinity::new();
        for i in 0..AFFINITY_CAP {
            affinity.bind(&format!("s{i}"), "c", i as i64);
        }
        affinity.bind("newcomer", "c", AFFINITY_CAP as i64 + 10);
        assert!(affinity.get("s0").is_none(), "最旧的条目应被淘汰");
        assert_eq!(affinity.get("newcomer"), Some("c"));
        assert!(affinity.entries.len() <= AFFINITY_CAP);
    }
}
