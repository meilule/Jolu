//! 连接管理（§6.3）
//!
//! 连接只有两种：官方订阅（`auth`）与 API Key（`api`）。
//! **本地会话记录不是连接**——它是统计来源，由后台自动发现，不入此表。
//!
//! ## 凭证纪律
//!
//! `secret` 只在后端查额度/余额时读取；所有跨进程边界的结构体（`ConnectionDto`）
//! 只带 `masked`，前端拿不到原值，日志也不打印原值。
//! 脱敏格式保留前缀与末 4 位，足以区分多个 Key 又不泄露内容。

use rusqlite::{params, Connection as Sql, OptionalExtension};
use serde::{Deserialize, Serialize};


pub mod store;
pub mod sync;
pub mod validate;
#[cfg(test)]
mod tests;

pub use store::*;
pub use sync::*;
pub use validate::*;

/// 返回给前端的连接。**不含** secret 字段——这是刻意的，不要加。
#[derive(Debug, Serialize)]
pub struct ConnectionDto {
    /// 仅返回供应商版本；本机来源 ID 或自身封存授权是唯一依据。
    pub edition: Option<String>,
    /// 已核对凭证的账号邮箱；未知为 None，绝不由连接名称或 Key 推断。
    pub account_email: Option<String>,
    pub service: Option<String>,
    pub id: String,
    pub platform: String,
    pub kind: String,
    pub name: String,
    pub label: String,
    /// ZCode 订阅通道手动锁定：start-plan / coding-plan；NULL = 自动双通道
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_channel: Option<String>,
    /// 脱敏标识，如 `sk-ant-****3f9a`；Auth 连接为套餐名或空
    pub masked: String,
    pub status: String,
    /// 相对时间文案，如「刚刚」「12 秒前」；从未同步过为 None
    pub last_sync_text: Option<String>,
    /// 自建网关（sub2api）的部署地址；官方直连为 None。
    /// 地址不是凭证，可以回显，便于用户核对填错没有
    pub base_url: Option<String>,
    /// 手动添加时保存的默认参数：模型 / 思考强度 / 1M 上下文。
    /// 本机读取的连接与历史连接为 None；参数随连接保存，供请求与成本估算使用
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context_1m: Option<bool>,
    /// 自定义供应商的接口协议：openai / anthropic；其他平台为 None
    pub protocol: Option<String>,
    /// 账号汇聚网关：是否进入模型池对外服务。旧连接默认 false，不改变观测行为。
    #[serde(default)]
    pub serve_enabled: bool,
    /// 模型入口的持久化菜单 id 前缀（`<slug>/<model>`）；改名不变。
    #[serde(default)]
    pub slug: Option<String>,
    /// 连接级计费口径：`auto` / `api` / `subscription` / `free`
    #[serde(default = "default_billing_mode")]
    pub billing_mode: String,
    /// 连接级价格倍率（缩放目录与全局基准价，不影响该连接专属单模型覆盖）
    pub price_multiplier: f64,
    /// 连接级日/月预算上限（USD）与超限动作（`alert` 仅提醒 | `block` 网关硬限额 429）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_daily_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_monthly_usd: Option<f64>,
    #[serde(default = "default_budget_action")]
    pub budget_action: String,
}

pub fn default_billing_mode() -> String { "auto".into() }
pub fn default_budget_action() -> String { "alert".into() }

#[derive(Debug, Deserialize)]
pub struct NewConnection {
    #[serde(default)]
    pub service: Option<String>,
    pub platform: String,
    pub kind: String,
    pub name: String,
    /// 仅 kind = "api" 时有值。落库后前端不再回读
    pub secret: Option<String>,
    /// sub2api 等自建网关的部署地址；官方直连留空
    pub base_url: Option<String>,
    /// 手动添加的可选默认参数；缺失时为 None
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub context_1m: Option<bool>,
    /// 接口协议：openai（Bearer）或 anthropic（x-api-key）；自定义供应商必填
    #[serde(default)]
    pub protocol: Option<String>,
    /// 菜单前缀（slug）的基名；None 时用 `name`。
    ///
    /// 给「显示名要拼来源标签、前缀不能跟着拼」的调用方用：从本机导入把来源标签拼进了
    /// 显示名（「Claude Code · Claude Code 上游」），拿它当前缀会得到 `claude-code-claude-code`
    /// 这种重复词。只影响新建时分配，已有连接的 slug 永不改写。
    #[serde(default)]
    pub slug_base: Option<String>,
}

/// 脱敏：保留可识别的前缀与末 4 位，中间固定 4 个星号。
/// 太短的 Key（异常输入）整体打码，避免反而暴露大部分内容。
pub fn mask(secret: &str) -> String {
    let s = secret.trim();
    if s.len() <= 8 {
        return "****".into();
    }
    // 常见前缀 sk-ant- / sk- 保留，便于用户辨认是哪一类 Key
    let prefix_len = if s.starts_with("sk-ant-") {
        7
    } else if s.starts_with("sk-") {
        3
    } else {
        4
    };
    let head: String = s.chars().take(prefix_len).collect();
    let tail: String = s.chars().skip(s.chars().count().saturating_sub(4)).collect();
    format!("{head}****{tail}")
}

/// 毫秒时间戳 → 相对时间文案（§6.3 的「最近同步」列）
pub fn relative(ms: i64, now: i64) -> String {
    let d = (now - ms).max(0) / 1000;
    match d {
        0..=4 => "刚刚".into(),
        5..=59 => format!("{d} 秒前"),
        60..=3599 => format!("{} 分钟前", d / 60),
        3600..=86399 => format!("{} 小时前", d / 3600),
        _ => format!("{} 天前", d / 86400),
    }
}

pub fn row_to_dto(r: &rusqlite::Row, now: i64) -> rusqlite::Result<ConnectionDto> {
    let last: Option<i64> = r.get("last_sync_ms")?;
    let platform: String = r.get("platform")?;
    let kind: String = r.get("kind")?;
    let base_url = normalized_base(&platform, &kind, r.get("base_url")?);
    let id:String=r.get("id")?;
    let edition=match id.as_str() {
        "monitor:native:qoder:cn"|"monitor:native:workbuddy:main"|"monitor:native:trae:cn"=>Some("cn".into()),
        "monitor:native:qoder:global"|"monitor:native:trae:ide"=>Some("global".into()),
        "monitor:native:workbuddy:ai"=>Some("ai".into()),
        _ if kind=="auth"=>crate::provider_browser_auth::edition_for(&id,&platform,r.get::<_,Option<String>>("secret")?.as_deref()),
        _=>None,
    };
    Ok(ConnectionDto {
        edition,
        account_email: r.get("account_email")?,
        id: r.get("id")?,
        platform: r.get("platform")?,
        kind: r.get("kind")?,
        name: r.get("name")?,
        label: r.get("label")?,
        masked: r.get("masked")?,
        status: r.get("status")?,
        last_sync_text: last.map(|m| relative(m, now)),
        base_url,
        model: r.get("model")?,
        effort: r.get("effort")?,
        context_1m: r.get("context_1m")?,
        protocol: r.get("protocol")?,
        service: r.get("service")?,
        serve_enabled: r.get::<_, Option<i64>>("serve_enabled")?.unwrap_or(0) != 0,
        slug: r.get("slug")?,
        start_channel: r.get("start_channel")?,
        billing_mode: r.get::<_, Option<String>>("billing_mode").ok().flatten().unwrap_or_else(default_billing_mode),
        price_multiplier: r.get::<_, Option<f64>>("price_multiplier").ok().flatten().unwrap_or(1.0),
        budget_daily_usd: r.get::<_, Option<f64>>("budget_daily_usd").ok().flatten(),
        budget_monthly_usd: r.get::<_, Option<f64>>("budget_monthly_usd").ok().flatten(),
        budget_action: r.get::<_, Option<String>>("budget_action").ok().flatten().unwrap_or_else(default_budget_action),
    })
}

pub fn validate_billing(mode: &str, multiplier: f64) -> Result<(&'static str, f64), String> {
    let normalized = match mode.trim() {
        "auto" => "auto",
        "api" => "api",
        "subscription" => "subscription",
        "free" => "free",
        other => return Err(format!("未知的计费口径：{other}")),
    };
    if !multiplier.is_finite() || !(0.0..=1000.0).contains(&multiplier) {
        return Err("价格倍率须在 0 到 1000 之间".into());
    }
    Ok((normalized, multiplier))
}

pub fn set_billing(conn: &Sql, id: &str, billing_mode: &str, price_multiplier: f64) -> Result<(), String> {
    let (mode, mult) = validate_billing(billing_mode, price_multiplier)?;
    let changed = conn
        .execute(
            "UPDATE connections SET billing_mode = ?2, price_multiplier = ?3 WHERE id = ?1",
            rusqlite::params![id, mode, mult],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("连接不存在".into());
    }
    Ok(())
}


/// ZCode 订阅通道手动锁定：`Some("start-plan" | "coding-plan")` 锁定单通道，
/// `None` 回到自动双通道。只对 zcode 来源生效，其他平台原样返回（不报错，卡片不显示）。
pub fn set_start_channel(conn: &Sql, id: &str, channel: Option<&str>) -> Result<(), String> {
    let matched = match channel {
        None | Some("auto") => None,
        Some(c @ ("start-plan" | "coding-plan")) => Some(c),
        Some(other) => return Err(format!("未知的订阅通道：{other}")),
    };
    let platform: String = conn
        .query_row("SELECT platform FROM connections WHERE id = ?1", [id], |r| r.get(0))
        .map_err(|_| "来源不存在".to_string())?;
    if platform != "zcode" {
        return Err("只有 ZCode 连接有订阅通道".into());
    }
    conn.execute("UPDATE connections SET start_channel = ?2 WHERE id = ?1", rusqlite::params![id, matched])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list(conn: &Sql) -> rusqlite::Result<Vec<ConnectionDto>> {
    list_with_account_lookup(conn, crate::login_detect::account_for_secret)
}

pub fn list_with_account_lookup(
    conn: &Sql,
    mut account_lookup: impl FnMut(&str, &str) -> Option<String>,
) -> rusqlite::Result<Vec<ConnectionDto>> {
    let now = chrono::Local::now().timestamp_millis();
    let mut stmt = conn.prepare(
        "SELECT id, platform, kind, name, label, masked, status, last_sync_ms, base_url, model, effort, context_1m, protocol, service, account_email, serve_enabled, secret, slug, start_channel, billing_mode, price_multiplier, budget_daily_usd, budget_monthly_usd, budget_action
         FROM connections ORDER BY created_ms ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let mut dto = row_to_dto(r, now)?;
            // 列表走只读连接：旧账号的未知邮箱仅补到返回值，不在查询时写库。
            // 凭证必须与本机登录完全匹配；已保存的身份不能被当前账号覆盖。
            if dto.account_email.is_none()
                && dto.kind == "auth"
                && matches!(dto.platform.as_str(), "claude" | "codex")
            {
                let secret: Option<String> = r.get("secret")?;
                dto.account_email = secret
                    .as_deref()
                    .filter(|secret|!crate::provider_browser_auth::is_browser(Some(secret)))
                    .and_then(|s| account_lookup(&dto.platform, s));
            }
            Ok(dto)
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// 连接名称校验：新增与改名共用同一口径（去首尾空白，1–80 个字符）。
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("连接名称应为 1 到 80 个字符".into());
    }
    Ok(name)
}

/// 校验连接输入，显式凭证不触发本机读取。
pub fn prepare_new(mut c: NewConnection) -> Result<NewConnection, String> {
    if !crate::platforms::known(&c.platform) {
        return Err("不支持的平台标识".into());
    }
    // 自定义供应商（D-11）：只接受用户显式填写的地址与 Key，不读取本机凭证
    if c.platform == "custom" {
        if c.kind != "api" { return Err("自定义供应商只支持 API Key 连接".into()); }
        if c.secret.as_deref().map_or(true, |s| s.trim().is_empty()) { return Err("请填写 API Key".into()); }
        if c.base_url.as_deref().map_or(true, |s| s.trim().is_empty()) { return Err("请填写 Base URL".into()); }
    }
    if c.platform=="opencode" {
        if c.kind!="api" {return Err("OpenAI 订阅请从 OpenCode 本机登录获取".into())}
        if !matches!(c.service.as_deref(),Some("opencode-go"|"openai-compatible")){return Err("请选择 OpenCode 模型服务".into())}
        if c.service.as_deref()==Some("opencode-go") && c.base_url.as_deref()!=Some("https://opencode.ai/zen/go/v1"){return Err("OpenCode Go 必须使用所选服务的官方地址".into())}
        if c.secret.as_deref().is_none_or(|s|s.trim().is_empty()){return Err("请填写 API Key".into())}
    } else { c.service=None; }
    c.protocol = valid_protocol(&c.platform, c.protocol.take())?;
    if c.platform=="zcode" && c.kind=="api" {
        let base=c.base_url.as_deref().unwrap_or("").trim_end_matches('/');
        let valid=["https://open.bigmodel.cn/api/anthropic","https://open.bigmodel.cn/api/coding/paas/v4","https://api.z.ai/api/anthropic","https://api.z.ai/api/coding/paas/v4"];
        if !valid.contains(&base){return Err("请选择 ZCode 官方服务地址".into())}
        c.protocol=Some(if base.ends_with("/anthropic"){"anthropic"}else{"openai"}.into());
    }
    if crate::platforms::native(&c.platform) && !(c.kind == "api" && (crate::provider_key::supported(&c.platform) || matches!(c.platform.as_str(),"zcode"|"opencode"|"ollama"))) { return Err("该平台请通过本机来源获取，不保存猜测的登录凭证".into()); }
    // 仅获取型平台（画布 18 后续）：凭证只能来自本机读取
    if crate::creds::is_fetch_only_platform(&c.platform) && c.kind != "auth" && !crate::provider_key::supported(&c.platform) && !matches!(c.platform.as_str(),"zcode"|"opencode"|"ollama") {
        return Err("该平台只支持从本机获取凭证，不支持手动填写".into());
    }
    if !matches!(c.kind.as_str(), "auth" | "api") {
        return Err("接入方式必须是 auth 或 api".into());
    }
    c.name = validate_name(&c.name)?;
    let using_local = c.secret.as_deref().map_or(true, |s| s.trim().is_empty());
    let local = using_local.then(|| crate::creds::read_local_pair(&c.platform, &c.kind)).flatten();
    if using_local && c.base_url.as_deref().map_or(true, |s| s.trim().is_empty()) {
        c.base_url = local.as_ref().and_then(|pair| pair.1.clone());
    }
    c.secret = c.secret
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| local.map(|pair| pair.0));
    if c.secret.is_none() {
        return Err("未找到可保存的凭证；请粘贴 API Key，或先在对应 CLI 中登录".into());
    }
    c.base_url = c.base_url
        .take()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    if let Some(base) = c.base_url.as_deref() {
        let url = reqwest::Url::parse(base).map_err(|_| "网关地址格式无效".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("网关地址必须使用 HTTP 或 HTTPS".into());
        }
    }
    if crate::provider_key::supported(&c.platform) {
        crate::provider_key::check_base(&c.platform, c.base_url.as_deref())?;
        c.model = None; c.effort = None; c.context_1m = None;
    }
    c.base_url = normalized_base(&c.platform, &c.kind, c.base_url);
    Ok(c)
}

/// 协议只认 openai / anthropic；自定义供应商必须选择其一。
pub fn valid_protocol(platform: &str, protocol: Option<String>) -> Result<Option<String>, String> {
    match protocol.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p @ ("openai" | "anthropic")) => Ok(Some(p.to_string())),
        Some(_) => Err("协议只能是 OpenAI 兼容或 Anthropic 兼容".into()),
        None if platform == "custom" => Err("请选择接口协议".into()),
        None => Ok(None),
    }
}

pub fn normalized_base(platform: &str, kind: &str, base: Option<String>) -> Option<String> {
    if kind == "auth" { return None; }
    base.filter(|value| {
        let Ok(url) = reqwest::Url::parse(value) else { return true };
        let host = match platform { "claude" => "api.anthropic.com", "codex" => "api.openai.com", _ => return true };
        !(url.scheme() == "https" && url.host_str() == Some(host) && url.port_or_known_default() == Some(443)
            && matches!(url.path().trim_end_matches('/'), "" | "/v1") && url.query().is_none() && url.fragment().is_none()
            && url.username().is_empty() && url.password().is_none())
    })
}

/// 编辑命令用：api 连接的地址归一（官方地址返回 None，不落库）。
pub fn normalized_base_for(platform: &str, base: Option<String>) -> Option<String> {
    normalized_base(platform, "api", base)
}

/// 仅写入已经完成凭证解析与网络验证的连接。
pub fn add(conn: &Sql, c: NewConnection) -> Result<String, String> {
    if c.secret.as_deref().map_or(true, |secret| secret.is_empty()) {
        return Err("连接尚未验证".into());
    }
    let now = chrono::Local::now().timestamp_millis();
    let suffix: String = conn.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let id = format!("{}-{suffix}", c.platform);
    let masked = c.secret.as_deref().map(mask).unwrap_or_default();
    let label = if c.kind == "api" { "API Key" } else { "官方订阅" };
    let base = c.base_url;
    let account_email = if c.kind == "auth" { c.secret.as_deref().and_then(|s| crate::login_detect::account_for_secret(&c.platform, s)) } else { None };
    let account_fp = crate::gateway::identity::account_fingerprint(&c.platform, &c.kind, c.secret.as_deref());
    // 前缀基名可以单独给：显示名里的来源标签不该进 slug
    let slug_name = c.slug_base.as_deref().filter(|base| !base.trim().is_empty()).unwrap_or(&c.name);
    let slug = crate::gateway::identity::allocate_slug(conn, &id, &c.platform, slug_name, account_fp.as_deref())
        .map_err(|error| format!("分配连接标识失败：{error}"))?;

    conn.execute_batch("SAVEPOINT jolu_add_connection").map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
    conn.execute(
        "INSERT INTO connections
           (id, platform, kind, name, label, secret, masked, base_url, model, effort, context_1m, status, last_sync_ms, created_ms, protocol, service, account_email, slug, account_fp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'connected', ?12, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![id, c.platform, c.kind, c.name, label, c.secret, masked, base, c.model, c.effort, c.context_1m, now, c.protocol, c.service, account_email, slug, account_fp],
    ).map_err(|e| e.to_string())?;
    if c.kind == "auth" {
        crate::auth_snapshot::capture(conn, &id, &c.platform, c.secret.as_deref().unwrap_or_default())?;
    }
        Ok(())
    })();
    if let Err(error) = result {
        conn.execute_batch("ROLLBACK TO jolu_add_connection; RELEASE jolu_add_connection")
            .map_err(|_| "添加连接失败且事务无法回滚，请重新启动后检查连接状态")?;
        return Err(error);
    }
    conn.execute_batch("RELEASE jolu_add_connection").map_err(|e| e.to_string())?;
    Ok(id)
}


/// 调用方持有同一个数据库锁，引用检查与删除不能拆到不同 with_database 闭包。
/// 已结束的历史保留原连接标识；仍可执行/续跑的任务和运行必须先解除引用。
pub fn remove(conn: &Sql, id: &str) -> Result<(), String> {
    let table_exists = |name: &str| -> Result<bool, String> {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [name], |row| row.get(0))
            .map_err(|error| error.to_string())
    };
    // 运行使用创建时的快照，任务改用另一个账号后，等待/续跑中的旧运行仍依赖原账号。
    if table_exists("automation_runs")? {
        let referenced: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM automation_runs WHERE status IN ('queued','waiting','running','attention')
             AND json_extract(task_snapshot, '$.connection_id')=?1)",
            [id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if referenced {
            return Err("该连接仍被未结束的自动化运行使用，请先取消相关运行，再删除账号".into());
        }
    }
    if table_exists("automation_tasks")? {
        let referenced: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM automation_tasks WHERE connection_id=?1 AND state IN ('active','paused'))",
            [id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if referenced {
            return Err("该连接仍被自动化任务使用，请先调整任务使用的账号或取消任务，再删除账号".into());
        }
    }
    // slug 历史与删除在同一事务：旧模型 id 以后要能回答「现在叫什么 / 已删除」，
    // 同一账号重建时也靠它恢复原来的 id。
    let tx = conn.unchecked_transaction().map_err(|error| error.to_string())?;
    let row: Option<(Option<String>, Option<String>, Option<String>)> = tx
        .query_row("SELECT slug, account_fp, secret FROM connections WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((slug, account_fp, _)) = row {
        if let Some(slug) = slug {
            tx.execute(
                "INSERT OR REPLACE INTO slug_history(slug, connection_id, account_fp, retired_ms) VALUES(?1, ?2, ?3, ?4)",
                params![slug, id, account_fp, chrono::Local::now().timestamp_millis()],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    tx.execute("DELETE FROM connections WHERE id = ?1", params![id]).map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(())
}

/// 暂停只改变连接状态，保留凭证、配置与所有本机历史。
pub fn set_paused(conn: &Sql, id: &str, paused: bool) -> Result<(), String> {
    let changed = conn.execute(
        "UPDATE connections SET status = ?2 WHERE id = ?1",
        params![id, if paused { "paused" } else { "connected" }],
    ).map_err(|e| e.to_string())?;
    if changed == 0 { return Err("连接不存在".into()); }
    Ok(())
}

/// 账号汇聚网关：把连接加入 / 移出模型池。
/// 只有具备网关服务能力的平台可开启：API Key 平台按 protocol 直通，
/// 来源选择独立于调用就绪；授权及协议由对应适配器在请求时校验。
/// Tauri 命令 `gateway_set_serve` 在第 11 步接线，先保留公共 API。
#[allow(dead_code)]
pub fn set_serve(conn: &Sql, id: &str, enabled: bool) -> Result<(), String> {
    let row: Option<(String, String, Option<String>, Option<String>)> = conn
        .query_row("SELECT platform, kind, protocol, account_fp FROM connections WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map(Some)
        .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e.to_string()) })
        .map_err(|e: String| e)?;
    let Some((platform, kind, protocol, account_fp)) = row else { return Err("连接不存在".into()); };
    let capable = kind == "api" || (kind == "auth" && matches!(platform.as_str(), "claude" | "codex" | "gemini" | "grok" | "opencode" | "workbuddy" | "qoder" | "trae" | "zcode"));
    if enabled && !capable {
        return Err(format!("{platform} 的订阅连接暂不支持对外服务"));
    }
    if enabled {
        conflict_of_serving(conn, id, &platform, &kind, protocol.as_deref(), account_fp.as_deref())?;
    }
    conn.execute(
        "UPDATE connections SET serve_enabled = ?2 WHERE id = ?1",
        params![id, enabled as i64],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// 开启服务前的唯一性检查：同一账号不能重复接入，不限制同平台的不同账号。
/// 与数据库部分唯一索引同口径；这里先给出可读的名字，避免用户只看到一个约束错误。
pub fn conflict_of_serving(
    conn: &Sql,
    id: &str,
    _platform: &str,
    kind: &str,
    protocol: Option<&str>,
    account_fp: Option<&str>,
) -> Result<(), String> {
    if kind == "auth" {
        let Some(fp) = account_fp else { return Ok(()); };
        let conflict: Option<String> = conn
            .query_row(
                "SELECT name FROM connections
                 WHERE serve_enabled = 1 AND kind = 'auth' AND account_fp = ?1 AND id != ?2
                 LIMIT 1",
                params![fp, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(name) = conflict {
            return Err(format!("此订阅账号已通过「{name}」接入，请使用已有来源或先关闭它"));
        }
        return Ok(());
    }
    let Some(fp) = account_fp else {
        return Err("该连接没有可用的凭证指纹，无法参与服务".into());
    };
    let conflict: Option<String> = conn
        .query_row(
            "SELECT name FROM connections
             WHERE serve_enabled = 1 AND kind = 'api' AND account_fp = ?1
               AND IFNULL(protocol, '') = ?2 AND id != ?3
             LIMIT 1",
            params![fp, protocol.unwrap_or(""), id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(name) = conflict {
        return Err(format!("该 Key 与「{name}」重复：同一账号同一协议只能有一个来源参与服务"));
    }
    Ok(())
}

/// 凭证更新后重算 account_fp。新指纹与正在服务的来源重复时，只关闭本连接的服务并记日志，
/// 凭证照常保存（不删数据、不阻断用户改 Key）。
pub fn refresh_account_fp(conn: &Sql, id: &str) -> rusqlite::Result<()> {
    let row: Option<(String, String, Option<String>)> = conn
        .query_row("SELECT platform, kind, secret FROM connections WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((platform, kind, secret)) = row else { return Ok(()); };
    let fp = if kind == "auth" && crate::provider_browser_auth::is_browser(secret.as_deref()) {
        secret.as_deref().and_then(|secret| crate::provider_browser_auth::load(id,&platform,secret).ok()).and_then(|credential| credential.fingerprint())
    } else { crate::gateway::identity::account_fingerprint(&platform, &kind, secret.as_deref()) };
    if let Err(error) = conn.execute("UPDATE connections SET account_fp = ?2 WHERE id = ?1", params![id, fp]) {
        let unique = error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation);
        if !unique { return Err(error); }
        conn.execute("UPDATE connections SET serve_enabled = 0 WHERE id = ?1", [id])?;
        conn.execute(
            "INSERT INTO migration_log(at_ms, kind, detail) VALUES(?1, 'duplicate_serve', ?2)",
            params![chrono::Local::now().timestamp_millis(), format!("{id}: 凭证更新后与已有来源重复，已关闭本连接的服务")],
        )?;
        conn.execute("UPDATE connections SET account_fp = ?2 WHERE id = ?1", params![id, fp])?;
    }
    Ok(())
}
