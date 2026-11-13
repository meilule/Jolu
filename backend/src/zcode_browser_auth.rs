//! ZCode 设备轮询授权；不接管 zcode://，不读取原工具凭证。
//! 依据 zai-org/zcode packages/services/src/oauth/oauthService.ts 及本机官方 CLI。
//!
//! ZCode 有两个账号家族：`bigmodel`（智谱中国版）与 `zai`（国际版 z.ai）。
//! 早期实现把 ZCode 等同于 BigModel——登录固定请求 `provider=bigmodel`、只解析响应里的
//! `bigmodel` 节点、上游固定指向 `open.bigmodel.cn`。z.ai 账号走这条路会在授权后拿不到凭据，
//! 存进连接的 `secret` 是空的，网关转发时报「缺少完整授权」。两边必须都支持。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Read,
    time::{Duration, Instant},
};

/// 账号家族。上游域名、OAuth 凭据节点、授权地址白名单都随它变化。
pub(crate) const FAMILY_BIGMODEL: &str = "bigmodel";
pub(crate) const FAMILY_ZAI: &str = "zai";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) user_id: String,
    pub(crate) account: Option<String>,
    pub(crate) zcode_token: String,
    /// 账号家族（[`FAMILY_BIGMODEL`] / [`FAMILY_ZAI`]）。
    /// 早期记录没有这个字段，只可能来自写死 BigModel 的登录流程，按 BigModel 处理。
    #[serde(default)]
    pub(crate) family: Option<String>,
}

impl Session {
    /// 缺省视为 BigModel：旧版记录都出自旧登录流程。
    pub(crate) fn family(&self) -> &str {
        self.family
            .as_deref()
            .filter(|f| matches!(*f, FAMILY_BIGMODEL | FAMILY_ZAI))
            .unwrap_or(FAMILY_BIGMODEL)
    }

    /// 转发上游：Anthropic 兼容端点，按家族分开。
    pub(crate) fn anthropic_base(&self) -> &'static str {
        match self.family() {
            FAMILY_ZAI => "https://api.z.ai/api/anthropic",
            _ => "https://open.bigmodel.cn/api/anthropic",
        }
    }

    /// 刷新/查询用的站点根，容忍上面的 base 再拼路径。
    pub(crate) fn origin(&self) -> &'static str {
        match self.family() {
            FAMILY_ZAI => "https://api.z.ai",
            _ => "https://bigmodel.cn",
        }
    }
}

pub(crate) struct Pending {
    pub(crate) login_url: String,
    pub(crate) user_code: Option<String>,
    pub(crate) interval: Duration,
    flow_id: String,
    poll_token: String,
    expires: Instant,
    next_poll: Instant,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 32768 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}

/// 授权地址白名单。
///
/// 早期只允许 BigModel 站点，z.ai 账号拿到的 `chat.z.ai` 授权地址会被判成「不属于官方站点」，
/// 于是 z.ai 用户根本走不完登录。现在两个家族的官方域名都收：白名单只管「是不是官方」，
/// 不负责区分家族——家族由授权结果里的凭据节点决定。
fn login_url(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "ZCode 授权地址无效")?;
    if raw.len() > 16384
        || raw.chars().any(char::is_control)
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || !url.host_str().is_some_and(|h| {
            matches!(
                h,
                "bigmodel.cn" | "www.bigmodel.cn" | "zcode.z.ai" | "z.ai" | "www.z.ai" | "chat.z.ai"
            )
        })
    {
        return Err("ZCode 返回的授权地址不属于官方站点".into());
    }
    Ok(url.to_string())
}

#[cfg(windows)]
fn poll_token() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = [0u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err("无法生成 ZCode 授权安全随机数".into());
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
#[cfg(not(windows))]
fn poll_token() -> Result<String, String> {
    Err("ZCode 浏览器授权当前仅支持 Windows".into())
}

fn client() -> Result<reqwest::blocking::Client, String> {
    crate::net::blocking_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法创建 ZCode 授权客户端".into())
}

fn json_response(mut response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("ZCode 授权服务未成功（HTTP {status}）"));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(262145)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取 ZCode 授权响应失败")?;
    if bytes.len() > 262144 {
        return Err("ZCode 授权响应过大".into());
    }
    let envelope: Value = serde_json::from_slice(&bytes).map_err(|_| "ZCode 授权响应格式无效")?;
    if envelope.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("ZCode 授权服务拒绝了请求，请重新授权".into());
    }
    envelope
        .get("data")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or("ZCode 授权响应缺少有效数据".into())
}

fn pending(value: &Value, poll_token: String) -> Result<Pending, String> {
    let flow_id = text(value, "flow_id")
        .filter(|s| s.len() <= 512)
        .ok_or("ZCode 未返回授权流程标识")?;
    let login_url = login_url(
        value
            .get("authorize_url")
            .and_then(Value::as_str)
            .ok_or("ZCode 未返回授权地址")?,
    )?;
    let seconds = value
        .get("expires_at")
        .and_then(Value::as_i64)
        .and_then(|expiry| expiry.checked_sub(chrono::Utc::now().timestamp()))
        .ok_or("ZCode 未返回有效授权时限")?;
    if !(1..=3600).contains(&seconds) {
        return Err("ZCode 授权时限无效或已过期".into());
    }
    let interval_secs = value
        .get("poll_interval_sec")
        .and_then(Value::as_u64)
        .filter(|n| (1..=300).contains(n))
        .ok_or("ZCode 未返回有效查询间隔")?;
    let now = Instant::now();
    Ok(Pending {
        login_url,
        user_code: None,
        interval: Duration::from_secs(interval_secs),
        flow_id,
        poll_token,
        expires: now + Duration::from_secs(seconds as u64),
        next_poll: now + Duration::from_secs(interval_secs),
    })
}

/// 家族名的归一化。
///
/// 前端对没有双版本的供应商统一传 `default`（`providerCatalog` 里 ZCode 没有 edition 字段），
/// 空串也可能出现。这些都表示「没指定」，按 BigModel 处理——与加入 z.ai 之前的行为一致，
/// 不会让已装好的 BigModel 用户因为升级而登不上。
pub(crate) fn normalize_family(family: &str) -> &'static str {
    match family.trim().to_ascii_lowercase().as_str() {
        FAMILY_ZAI => FAMILY_ZAI,
        _ => FAMILY_BIGMODEL,
    }
}

/// 发起授权。
///
/// `family` 接受两种家族名，其余值（含前端默认传下来的 `default`、空串）一律按 BigModel：
/// 界面里非双版本供应商没有家族概念，`ConnectDialog` 会统一传 `"default"`，
/// 这里把它当成「没指定」而不是错误——否则 ZCode 的登录入口直接不可用。
pub(crate) fn begin_family(family: &str) -> Result<Pending, String> {
    if crate::validation::isolated() {
        return Err("隔离验收不创建真实 ZCode 授权".into());
    }
    let provider = normalize_family(family);
    let token = poll_token()?;
    let response = client()?
        .post("https://zcode.z.ai/api/v1/oauth/cli/init")
        .bearer_auth(&token)
        .json(&json!({ "provider": provider }))
        .send()
        .map_err(|_| "无法创建 ZCode 授权，请检查网络")?;
    pending(&json_response(response)?, token)
}

/// 从授权结果里取出会话。
///
/// 凭据节点的名字随家族变化：BigModel 是 `bigmodel`，z.ai 是 `zai`。
/// 早期只读 `bigmodel`，z.ai 账号必然解析失败——这是「登录后仍报缺少授权」的直接原因。
/// 两个节点都不在时，再退回 `oauth:active_provider` 之外的线索：响应里若只有 `token`
/// 没有业务凭据，只能判断为不完整。
fn session(value: &Value) -> Result<Session, String> {
    let user = value.get("user").ok_or("ZCode 未返回已验证账号")?;
    // 先看响应自己声明的 provider，再看哪个节点有 access_token——两者都缺才报错。
    let declared = text(value, "provider").or_else(|| text(value, "active_provider"));
    let pick = declared
        .as_deref()
        .filter(|p| matches!(*p, FAMILY_BIGMODEL | FAMILY_ZAI))
        .filter(|p| value.get(*p).is_some())
        .or_else(|| {
            [FAMILY_BIGMODEL, FAMILY_ZAI]
                .into_iter()
                .find(|p| value.get(*p).is_some())
        })
        .ok_or("ZCode 未返回登录凭据（bigmodel / zai 均缺失）")?;
    let provider = value.get(pick).ok_or("ZCode 未返回登录凭据")?;
    Ok(Session {
        user_id: text(user, "user_id").ok_or("ZCode 未返回有效账号身份")?,
        account: text(user, "email").or_else(|| text(user, "name")),
        zcode_token: text(value, "token").ok_or("ZCode 未返回有效会话")?,
        access_token: text(provider, "access_token")
            .or_else(|| text(provider, "accessToken"))
            .ok_or_else(|| format!("ZCode 未返回 {pick} 业务凭据"))?,
        refresh_token: text(provider, "refresh_token").or_else(|| text(provider, "refreshToken")),
        family: Some(pick.to_string()),
    })
}

pub(crate) fn poll(pending: &mut Pending) -> Result<Option<Session>, String> {
    let now = Instant::now();
    if now >= pending.expires {
        return Err("ZCode 授权已超时，请重新授权".into());
    }
    if now < pending.next_poll {
        return Ok(None);
    }
    pending.next_poll = now + pending.interval;
    let mut url = reqwest::Url::parse("https://zcode.z.ai/api/v1/oauth/cli/poll/")
        .map_err(|_| "ZCode 授权端点无效")?;
    url.path_segments_mut()
        .map_err(|_| "ZCode 授权端点无效")?
        .pop_if_empty()
        .push(&pending.flow_id);
    let response = client()?
        .get(url)
        .bearer_auth(&pending.poll_token)
        .send()
        .map_err(|_| "ZCode 授权查询网络失败，请重新授权")?;
    let value = json_response(response)?;
    match value.get("status").and_then(Value::as_str) {
        Some("pending") => Ok(None),
        Some("ready") => session(&value).map(Some),
        Some("failed") => Err("ZCode 官方授权未成功，请重新授权".into()),
        _ => Err("ZCode 返回了未知授权状态".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bigmodel_ready_requires_identity_and_provider_specific_token() {
        let value = json!({"token":"synthetic-session","user":{"user_id":"test-user","email":"test@example.test"},"bigmodel":{"accessToken":"synthetic-business-token"}});
        assert_eq!(
            session(&value).unwrap().account.as_deref(),
            Some("test@example.test")
        );
        let mut missing = value.clone();
        missing["user"] = json!({});
        assert!(session(&missing).is_err());
        missing = value;
        missing["bigmodel"] = Value::Null;
        assert!(session(&missing).is_err());
        assert!(login_url("https://bigmodel.cn.evil.test/login").is_err());
        // z.ai 是官方站点之一（国际版账号从这里登录），必须放行；
        // 但仿冒域名仍然要挡住。
        assert!(login_url("https://chat.z.ai/api/oauth/authorize").is_ok());
        assert!(login_url("https://chat.z.ai.evil.test/x").is_err());
    }

    #[test]
    fn zai_ready_parses_zai_node_and_marks_family() {
        // z.ai 账号的授权结果里业务凭据在 `zai` 节点，不是 `bigmodel`。
        // 早期只读 bigmodel，z.ai 账号必然解析失败 → 连接里 secret 为空 → 网关报「缺少完整授权」。
        let value = json!({
            "token":"synthetic-session",
            "user":{"user_id":"zai-user","email":"zai@example.test"},
            "zai":{"access_token":"synthetic-zai-token"}
        });
        let parsed = session(&value).expect("z.ai 凭据应能解析");
        assert_eq!(parsed.family(), FAMILY_ZAI);
        assert_eq!(parsed.access_token, "synthetic-zai-token");
        // 上游必须跟着家族走
        assert_eq!(parsed.anthropic_base(), "https://api.z.ai/api/anthropic");
        assert_eq!(parsed.origin(), "https://api.z.ai");
    }

    #[test]
    fn bigmodel_session_keeps_bigmodel_upstream() {
        let value = json!({
            "token":"s",
            "user":{"user_id":"u"},
            "bigmodel":{"access_token":"t"}
        });
        let parsed = session(&value).unwrap();
        assert_eq!(parsed.family(), FAMILY_BIGMODEL);
        assert_eq!(parsed.anthropic_base(), "https://open.bigmodel.cn/api/anthropic");
    }

    #[test]
    fn response_declaring_provider_wins_when_both_nodes_exist() {
        // 两个节点都在时按响应声明的 provider 选，不能瞎猜。
        let value = json!({
            "token":"s",
            "provider":"zai",
            "user":{"user_id":"u"},
            "bigmodel":{"access_token":"bm"},
            "zai":{"access_token":"zai-token"}
        });
        let parsed = session(&value).unwrap();
        assert_eq!(parsed.family(), FAMILY_ZAI);
        assert_eq!(parsed.access_token, "zai-token");
    }

    #[test]
    fn legacy_session_without_family_defaults_to_bigmodel() {
        // 旧记录没有 family 字段，只可能来自写死 BigModel 的登录流程。
        let value = json!({"token":"s","user":{"user_id":"u"},"bigmodel":{"access_token":"t"}});
        let parsed = session(&value).unwrap();
        let legacy = Session { family: None, ..parsed };
        assert_eq!(legacy.family(), FAMILY_BIGMODEL);
        assert_eq!(legacy.anthropic_base(), "https://open.bigmodel.cn/api/anthropic");
    }

    #[test]
    fn family_normalization_treats_unknown_as_bigmodel() {
        // 前端对没有双版本的供应商统一传 "default"，这不能当成错误——
        // 之前 begin_family 直接拒绝未知家族，ZCode 的登录入口整个不可用。
        assert_eq!(normalize_family(FAMILY_ZAI), FAMILY_ZAI);
        assert_eq!(normalize_family("ZAI"), FAMILY_ZAI, "大小写不敏感");
        assert_eq!(normalize_family(FAMILY_BIGMODEL), FAMILY_BIGMODEL);
        assert_eq!(normalize_family("default"), FAMILY_BIGMODEL);
        assert_eq!(normalize_family(""), FAMILY_BIGMODEL);
        assert_eq!(normalize_family("  "), FAMILY_BIGMODEL);
        assert_eq!(normalize_family("cn"), FAMILY_BIGMODEL, "地区名不是家族名，按默认处理");
    }

    #[test]
    fn missing_both_provider_nodes_is_an_error() {
        let value = json!({"token":"s","user":{"user_id":"u"}});
        let error = session(&value).err().expect("两个节点都缺时必须报错");
        assert!(error.contains("bigmodel") && error.contains("zai"), "要指出缺哪两个节点：{error}");
    }
}
