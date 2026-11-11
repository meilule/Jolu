//! BigModel 个人 Coding Plan：登录态 → 项目短期令牌 → Anthropic 兼容模型接口。
//! 依据 zai-org/zcode 的 project-access-token-client.ts；不扫描其他账号/团队。
use crate::{
    gateway::{
        route::PoolEntry,
        wire::{error_response, CallerProtocol},
    },
    provider_browser_auth::{self, Credential},
    proxy::ProxyDeps,
};
use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
};
use serde_json::{json, Value};
use std::sync::Arc;
/// 两个账号家族的站点根。实际使用哪一个由连接里的 `session.family()` 决定，
/// 这组常量只在「测试换了上游域名」的判断里用到。
const ORIGIN: &str = "https://bigmodel.cn";
const ZAI_ORIGIN: &str = "https://api.z.ai";

pub(crate) fn bound(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(String, crate::zcode_browser_auth::Session), String> {
    let secret: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='zcode' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 ZCode 账号缺少完整授权，请重新登录")?;
    let Credential::Zcode { session } = provider_browser_auth::load(id, "zcode", &secret)? else {
        return Err("ZCode 授权类型不匹配".into());
    };
    if session.user_id.is_empty() || session.access_token.is_empty() {
        return Err("ZCode 授权身份不完整".into());
    }
    Ok((secret, session))
}

/// 网关来源能力判定的 ready 文案，保持与改造前一致，前端无需感知判定方式变化。
const SERVE_READY: &str = "支持 API 调用 · 个人 Coding Plan / Messages，真实账号待验收";

/// 来源能力判定：与实际转发走同一套凭据顺序（见 [`coding_plan_forward`]）——
/// 先本机 Coding Plan 凭据，再浏览器授权 OAuth 链。只看浏览器授权会把本机来源
/// 镜像（`monitor:native:zcode:*`，secret 为空）误报成「需要重新登录」：它们的
/// 登录与额度都正常（额度查询走本机令牌），缺的只是转发凭据，文案要说清这一点。
pub(crate) fn serve_capability(conn: &rusqlite::Connection, id: &str) -> Result<String, String> {
    serve_capability_with(conn, id, &forward_credential)
}

fn serve_capability_with(
    conn: &rusqlite::Connection,
    id: &str,
    local: &dyn Fn(&str) -> Result<(String, &'static str, Option<String>), String>,
) -> Result<String, String> {
    if local(id).is_ok() {
        return Ok(SERVE_READY.to_string());
    }
    match bound(conn, id) {
        Ok(_) => Ok(SERVE_READY.to_string()),
        Err(browser_reason) => {
            let has_browser_secret = conn
                .query_row("SELECT secret FROM connections WHERE id=?1", [id], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .ok()
                .flatten()
                .filter(|secret| provider_browser_auth::is_browser(Some(secret)))
                .is_some();
            if has_browser_secret {
                return Err(browser_reason);
            }
            // 本机来源镜像：缺的是转发凭据，不是登录。两条链各断一处要如实说清，
            // 不再沿用「需要重新登录」——那会指向一个并不存在的登录问题。
            Err("ZCode 转发凭据不可用：本机 Coding Plan 凭据读不出来，且该来源没有浏览器授权；请在原工具重新登录后重试，或改用浏览器授权保存的账号".to_string())
        }
    }
}

/// 家族 → Anthropic 上游。借用 [`crate::zcode_browser_auth::Session::anthropic_base`]
/// 的既有映射，不在本文件重复定义域名；这里没有真实会话，其余字段填空即可。
fn anthropic_base_of(family: &str) -> &'static str {
    crate::zcode_browser_auth::Session {
        access_token: String::new(),
        refresh_token: None,
        user_id: String::new(),
        account: None,
        zcode_token: String::new(),
        family: Some(family.to_owned()),
    }
    .anthropic_base()
}

/// 直读本机 ZCode 凭据里的 Coding Plan Key（与 magpie `zcodeOwn` 同款只读链）。
///
/// 订阅转发原本只能走 [`material_at`] 的多步 OAuth 链，对 z.ai 账号经常 401；
/// ZCode 自己的凭据文件里就有现成的 Coding Plan Key，能解密就直接用。
/// 返回 (API Key, Anthropic 上游, `zcodejwttoken`)：JWT 只表达登录态，不当转发凭据。
/// 拿不到 Key 时报错，由调用方退回 [`material_at`] 旧链——供应商页登录的产品功能仍依赖它。
/// `id` 只用于错误文案定位是哪条连接；凭据文件是整机唯一的 ZCode 登录，与连接无关。
fn forward_credential(id: &str) -> Result<(String, &'static str, Option<String>), String> {
    let home =
        crate::creds::home().ok_or_else(|| "无法定位本机 ZCode 配置目录".to_string())?;
    let (key, family) = crate::zcode_quota::coding_plan_key_at(&home)
        .map_err(|error| format!("连接 {id} 未能读取本机 ZCode Coding Plan 凭据：{error}"))?;
    let jwt = crate::zcode_quota::coding_plan_jwt_at(&home);
    Ok((key, anthropic_base_of(family), jwt))
}

async fn data(mut response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "ZCode 换取调用凭据失败（HTTP {}），请检查此账号登录或套餐",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "ZCode 响应读取失败")? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("ZCode 响应超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk)
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "ZCode 响应格式无效")?;
    if !matches!(value["code"].as_i64(), Some(0 | 200))
        && !matches!(value["code"].as_str(), Some("0" | "200"))
    {
        return Err("ZCode 拒绝换取调用凭据，请检查此账号的套餐权限".into());
    }
    value
        .get("data")
        .cloned()
        .ok_or("ZCode 未返回调用凭据数据".into())
}

fn segment(value: &Value, key: &str) -> Result<String, String> {
    let value = value[key]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or("ZCode 项目身份不完整")?;
    Ok(url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20"))
}

/// ZCode 上游请求的公共 client（代理模式 / 测试直连的处理与 material_at 一致）。
fn zcode_http(origin: &str) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30));
    // 测试里换掉上游域名（指向本地假服务器）时直连，避免测试代理干扰；
    // 正式运行时 origin 一定是官方域名之一，这里不会命中。
    let builder = if cfg!(test) && !matches!(origin, ORIGIN | ZAI_ORIGIN) {
        builder.no_proxy()
    } else {
        match crate::net::current_mode() {
            crate::net::NetMode::Direct => builder.no_proxy(),
            crate::net::NetMode::Manual(url) => {
                builder.proxy(reqwest::Proxy::all(url).map_err(|_| "ZCode 代理配置无效")?)
            }
            crate::net::NetMode::System => match crate::net::system_proxy_url() {
                Some(url) => {
                    builder.proxy(reqwest::Proxy::all(url).map_err(|_| "ZCode 系统代理无效")?)
                }
                None => builder,
            },
        }
    };
    builder.build().map_err(|_| "无法创建 ZCode 授权连接".into())
}

/// 团队套餐坐席（magpie `zcode_team.go` 同款）：setting.json 选定的 org/project →
/// 项目 api_keys 里的 `zcode-team-api-key`。团队套餐的额度在**项目 Key** 上，
/// 个人 Coding Plan Key 打上游会 1113「余额不足」。
///
/// 不代建 Key：ZCode 客户端切到团队套餐时会自己生成；Jolu 缺 Key 时报错让用户
/// 在客户端里切一次，不做额外的远端副作用。
/// 团队套餐的席位状态（magpie 插件 `teamSignIn` 同款）：套餐生效且给了这个账号席位才用团队 Key。
///
/// 为什么：账号在团队里但管理员还没分配席位（UNASSIGNED），或团队套餐已过期时，团队 Key
/// 打上游只会回含糊的余额 / 权限错误；先查一次就能直接说清原因。查不到（网络错误）时不拦——
/// 不知道不等于没有席位。
pub(crate) fn seat_verdict(detail: &serde_json::Value) -> Result<(), String> {
    let text = |key: &str| detail.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_ascii_uppercase();
    let (status, grant) = (text("status"), text("memberGrantStatus"));
    let subscribed = detail.get("hasSubscription").and_then(serde_json::Value::as_bool) != Some(false);
    if subscribed && status == "EFFECTIVE" && grant == "VALID" {
        return Ok(());
    }
    Err(match (status.as_str(), grant.as_str()) {
        ("EFFECTIVE", "UNASSIGNED") => "这个账号在团队里，但团队的 GLM Coding Plan 还没给它分配席位：请团队管理员分配后再试".into(),
        ("EXPIRED", _) => "这个账号所在团队的 GLM Coding Plan 已过期".into(),
        _ if !subscribed => "这个账号所在的团队没有订阅 GLM Coding Plan".into(),
        _ => format!("团队 GLM Coding Plan 当前不可用（状态 {}，席位 {}）", if status.is_empty() { "未知" } else { &status }, if grant.is_empty() { "未知" } else { &grant }),
    })
}

async fn team_credential(deps: &Arc<ProxyDeps>, id: &str) -> Result<(String, &'static str), String> {
    let _guard = crate::gateway::subscription::lock(id).await;
    let home = crate::creds::home().ok_or("无法定位用户目录")?;
    let Some(selection) = crate::zcode_quota::team_selection_at(&home) else {
        return Err("ZCode 未选定团队套餐".into());
    };
    // 业务根与鉴权形态按家族：Z.ai 的业务令牌走 Bearer，BigModel 的裸放（magpie 同款）
    let (biz, auth_header, base) = match selection.family.as_str() {
        "bigmodel" => (
            "https://bigmodel.cn".to_string(),
            selection.access_token.clone(),
            "https://open.bigmodel.cn/api/anthropic",
        ),
        _ => (
            "https://api.z.ai".to_string(),
            format!("Bearer {}", selection.access_token),
            "https://api.z.ai/api/anthropic",
        ),
    };
    let client = zcode_http(biz.as_str())?;
    // C3：先查席位状态；查询本身失败时不拦（不知道不等于没有席位）
    let detail = client
        .request(Method::GET, format!("{biz}/api/biz/team/subscribe/product/querySubscribeDetail"))
        .header("authorization", &auth_header)
        .header("Bigmodel-Organization", &selection.org)
        .header("Bigmodel-Project", &selection.project)
        .header("Set-Language", if selection.family == "bigmodel" { "zh" } else { "en" })
        .header("Accept-Language", "en-US,en")
        .send()
        .await;
    match detail {
        Ok(response) => match data(response).await {
            Ok(detail) => seat_verdict(&detail)?,
            Err(reason) => eprintln!("[ZCode] 团队席位状态查询失败，按未知处理继续：{reason}"),
        },
        Err(_) => eprintln!("[ZCode] 团队席位状态查询失败，按未知处理继续"),
    }
    let url = format!(
        "{biz}/api/biz/v1/organization/{}/projects/{}/api_keys",
        selection.org, selection.project
    );
    let mut request = client.request(Method::GET, &url).header("authorization", &auth_header);
    if selection.family == "bigmodel" {
        request = request
            .header("Bigmodel-Organization", &selection.org)
            .header("Bigmodel-Project", &selection.project);
    }
    request = request.header("Set-Language", "en").header("Accept-Language", "en-US,en");
    let keys = data(request.send().await.map_err(|_| "ZCode 团队项目凭据查询失败")?).await?;
    let list = keys.as_array().ok_or("ZCode 团队项目凭据列表无效")?;
    // 项目 Key：ZCode 用的是 zcode-team-api-key（keyType 2）
    let entry = list
        .iter()
        .find(|k| k["name"] == "zcode-team-api-key" || k["keyType"] == 2)
        .ok_or("团队项目还没有 zcode-team-api-key：请在 ZCode 客户端里切到该套餐一次以生成；Jolu 不代建 Key")?;
    let token = segment(entry, "apiKey")?;
    Ok((
        url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>().replace('+', "%20"),
        base,
    ))
}

async fn material_at(deps: &Arc<ProxyDeps>, id: &str, origin: &str) -> Result<String, String> {
    let _guard = crate::gateway::subscription::lock(id).await;
    let (secret, session) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 ZCode 授权")?;
        bound(&conn, id)?
    };
    let client = zcode_http(origin)?;
    let request = |method, url: String| {
        client
            .request(method, url)
            .header("authorization", &session.access_token)
    };
    let customer = data(
        request(
            Method::GET,
            format!("{origin}/api/biz/customer/getCustomerInfo"),
        )
        .send()
        .await
        .map_err(|_| "ZCode 项目查询失败")?,
    )
    .await?;
    let organizations = customer["organizations"]
        .as_array()
        .ok_or("ZCode 未返回个人项目")?;
    let mut candidates = Vec::new();
    for org in organizations {
        if let Some(projects) = org["projects"].as_array() {
            for project in projects {
                if project["projectType"] == 2 || project["projectType"] == "2" {
                    continue;
                }
                candidates.push((org, project));
            }
        }
    }
    let defaults: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|(o, p)| {
            o["organizationName"]
                .as_str()
                .is_some_and(|s| s.contains("默认机构"))
                && p["projectName"]
                    .as_str()
                    .is_some_and(|s| s.contains("默认项目"))
        })
        .collect();
    let (org, project) = if defaults.len() == 1 {
        defaults[0]
    } else if candidates.len() == 1 {
        candidates[0]
    } else {
        return Err(
            "ZCode 个人项目无法唯一确定，请在官方工具确认默认个人项目；不会自动改用团队项目".into(),
        );
    };
    let url = format!(
        "{origin}/api/biz/v1/organization/{}/projects/{}/api_keys",
        segment(org, "organizationId")?,
        segment(project, "projectId")?
    );
    let keys = data(
        request(Method::GET, url.clone())
            .send()
            .await
            .map_err(|_| "ZCode 项目凭据查询失败")?,
    )
    .await?;
    let keys = keys.as_array().ok_or("ZCode 项目凭据列表无效")?;
    // 模型入口只使用已有项目 Key；创建长期凭据是额外远端副作用，不能隐式执行。
    let key = keys
        .iter()
        .find(|k| k["name"] == "zcode-api-key")
        .ok_or("ZCode 缺少已有个人项目凭据，请先在官方工具配置；Jolu 不会自动创建 API Key")?;
    let token_url = format!("{url}/{}/access_tokens", segment(&key, "apiKey")?);
    let value = data(
        client
            .post(token_url)
            .bearer_auth(&session.access_token)
            .json(&json!({"clientType":"zcode","clientVersion":"3.17.1"}))
            .send()
            .await
            .map_err(|_| "ZCode 短期凭据签发失败")?,
    )
    .await?;
    if value["enable"] == false
        || value["tokenType"] != "Bearer"
        || value["expiresIn"].as_f64().is_none_or(|v| v <= 0.0)
        || value["expiresAt"]
            .as_f64()
            .is_none_or(|v| v <= chrono::Utc::now().timestamp() as f64)
    {
        return Err("ZCode 项目调用权限未开启或短期凭据已过期".into());
    }
    let token = value["accessToken"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("ZCode 未返回短期调用凭据")?
        .to_owned();
    let conn = deps.db.lock().map_err(|_| "无法复查 ZCode 授权")?;
    if bound(&conn, id)?.0 != secret {
        return Err("ZCode 账号授权已变化，请重试；不会使用旧账号结果".into());
    }
    Ok(token)
}

pub(crate) async fn models(deps: &Arc<ProxyDeps>, id: &str) -> Result<Vec<String>, String> {
    match coding_plan_models(deps, id).await {
        Ok(list) => Ok(list),
        // C1：只有免费 Start Plan 的账号列不出 Coding Plan 模型——这时用余额里现在
        // 还能花的礼包模型（真实数据）；读不到礼包就如实报原错，不拿静态清单冒充
        Err(error) => match super::zcode_start::gift_models().await {
            Ok(Some(gift)) if !gift.is_empty() => Ok(gift),
            _ => Err(error),
        },
    }
}

async fn coding_plan_models(deps: &Arc<ProxyDeps>, id: &str) -> Result<Vec<String>, String> {
    // 先直读本机凭据的 Coding Plan Key（新路径），失败再退回 OAuth 换取链；
    // 上游按账号家族分开：z.ai 账号去 api.z.ai，BigModel 账号去 open.bigmodel.cn。
    let mut credential = forward_credential(id).ok().map(|(key, base, _jwt)| (key, base));
    if let Ok((team_key, team_base)) = team_credential(deps, id).await {
        credential = Some((team_key, team_base));
    }
    if let Some((key, base)) = credential {
        return tokio::task::spawn_blocking(move || {
            crate::connections::fetch_models("zcode", Some("anthropic"), base, &key)
        })
        .await
        .map_err(|_| "ZCode 模型查询失败")?;
    }
    let (origin, base) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 ZCode 授权")?;
        let (_, session) = bound(&conn, id)?;
        (session.origin(), session.anthropic_base())
    };
    let token = material_at(deps, id, origin).await?;
    tokio::task::spawn_blocking(move || {
        crate::connections::fetch_models("zcode", Some("anthropic"), base, &token)
    })
    .await
    .map_err(|_| "ZCode 模型查询失败")?
}

/* ───────────────────── ③5 free_first 开关（magpie `FREE_FIRST` 同款） ───────────────────── */

/// 设置项 `zcode_free_first` 的进程级镜像（gateway/mask.rs 的 `set_enabled` 同款）：
/// 网关转发链拿不到 Tauri 状态，启动时按设置初始化、`zcode_free_first_set` 命令
/// 运行时改。默认关闭 = 现状（Coding Plan Key 先、余额类失败落 Start Plan）。
static FREE_FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 设置页改 `zcode_free_first` / 启动初始化时同步进程级开关。
pub(crate) fn set_free_first(on: bool) {
    FREE_FIRST.store(on, std::sync::atomic::Ordering::Release);
}

fn free_first() -> bool {
    FREE_FIRST.load(std::sync::atomic::Ordering::Acquire)
}

/* ───────────────────── C2 双通道按账号记忆（magpie `routes` 同款） ───────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    /// 免费 Start Plan（zcode.z.ai，会话 JWT）
    StartPlan,
    /// Coding Plan Key / 团队项目 Key / OAuth 换取（api.z.ai / open.bigmodel.cn）
    CodingPlan,
}

/// 通道记忆的有效期（magpie `routes` 的 10 分钟）：过期后按开关的默认顺序重新探测，
/// 额度恢复的通道最迟这么久就能被重新用上。
const CHANNEL_TTL_MS: i64 = 10 * 60_000;

/// 每个连接上次成功的通道。只在进程内，重启后第一次请求重新探测（代价一次往返）。
#[derive(Default)]
struct ChannelMemory(std::sync::Mutex<std::collections::HashMap<String, (Channel, i64)>>);

impl ChannelMemory {
    fn get(&self, id: &str, now_ms: i64) -> Option<Channel> {
        let map = self.0.lock().ok()?;
        map.get(id).filter(|(_, at)| now_ms - at < CHANNEL_TTL_MS).map(|(channel, _)| *channel)
    }

    fn set(&self, id: &str, channel: Channel, now_ms: i64) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(id.to_string(), (channel, now_ms));
        }
    }
}

static CHANNELS: std::sync::LazyLock<ChannelMemory> = std::sync::LazyLock::new(ChannelMemory::default);

/// 本次请求的通道顺序：有记忆先走记住的那条，否则按 `zcode_free_first`；
/// 本机没有 ZCode 会话 JWT 时 Start Plan 不可用，只走 Coding Plan。
fn channel_order(free_first: bool, remembered: Option<Channel>, start_available: bool) -> Vec<Channel> {
    if !start_available {
        return vec![Channel::CodingPlan];
    }
    let first = remembered.unwrap_or(if free_first { Channel::StartPlan } else { Channel::CodingPlan });
    let second = if first == Channel::StartPlan { Channel::CodingPlan } else { Channel::StartPlan };
    vec![first, second]
}

/// 这条通道失败后是否换另一条。
/// - Coding Plan：账单级（429/1113、402）与网络级（502/504）失败才换——两条通道主机
///   不同，一条被网络挡住时另一条可能可达；请求本身的错（400/401/404）换了也一样。
/// - Start Plan：非 2xx 一律换。它的拒绝语义（忙碌 429、额度用尽 1113/1005、风控文案）
///   上游随时会变，细分类容易在语义漂移时漏落回；代价只是多一次往返。
fn falls_back(from: Channel, status: StatusCode) -> bool {
    match from {
        Channel::StartPlan => !status.is_success(),
        Channel::CodingPlan => matches!(status.as_u16(), 402 | 429 | 502 | 504),
    }
}

/// 按顺序试通道：成功就记住这条（下次直接走它，省掉必然失败的那次往返）；
/// 失败且该换时试下一条；都失败回最后一条的回应，记忆不变。
async fn forward_channels<F, Fut>(
    memory: &ChannelMemory,
    id: &str,
    free_first: bool,
    start_available: bool,
    now_ms: i64,
    mut send: F,
) -> Response
where
    F: FnMut(Channel) -> Fut,
    Fut: std::future::Future<Output = Response>,
{
    let order = channel_order(free_first, memory.get(id, now_ms), start_available);
    let mut response = error_response(StatusCode::SERVICE_UNAVAILABLE, "ZCode 没有可用的通道");
    for (index, channel) in order.iter().copied().enumerate() {
        response = send(channel).await;
        if response.status().is_success() {
            memory.set(id, channel, now_ms);
            return response;
        }
        if index + 1 == order.len() || !falls_back(channel, response.status()) {
            break;
        }
    }
    response
}

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
    session: &str,
) -> Response {
    // 3.4：Start Plan 只说 Anthropic；Chat / Responses 调用方只走 Coding Plan 通道的 OpenAI 兼容端点
    let start_available = protocol == CallerProtocol::Anthropic && super::zcode_start::jwt_at().is_some();
    let now = chrono::Utc::now().timestamp_millis();
    forward_channels(&CHANNELS, &entry.connection_id, free_first(), start_available, now, |channel| {
        let (deps, body) = (deps.clone(), body.clone());
        async move {
            match channel {
                Channel::StartPlan => super::zcode_start::forward(deps, entry, protocol, body, session).await,
                Channel::CodingPlan => coding_plan_forward(deps, entry, body, protocol).await,
            }
        }
    })
    .await
}

/// Coding Plan 通道的一次转发（不含换通道——那由 [`forward_channels`] 决定）。
async fn coding_plan_forward(deps: Arc<ProxyDeps>, entry: &PoolEntry, body: Bytes, protocol: CallerProtocol) -> Response {
    // 凭据走三条链（对齐 magpie zcodeOwn）：个人 Coding Plan Key 先取；ZCode 切到
    // 团队套餐时项目 Key **覆盖**（团队额度在项目 Key 上，个人 Key 会 1113）；
    // 两条都拿不到再退回 `material_at` 的 OAuth 换取链——供应商页登录的产品功能
    // 仍依赖它。各链失败静默换链，不打断请求。
    let mut credential = forward_credential(&entry.connection_id).ok().map(|(key, base, _jwt)| (key, base));
    if let Ok((team_key, team_base)) = team_credential(&deps, &entry.connection_id).await {
        credential = Some((team_key, team_base));
    }
    let (token, base) = match credential {
        Some((key, base)) => (key, base),
        None => {
            let (origin, base) = {
                match deps.db.lock() {
                    Ok(conn) => match bound(&conn, &entry.connection_id) {
                        Ok((_, session)) => (session.origin(), session.anthropic_base()),
                        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
                    },
                    Err(_) => {
                        return error_response(StatusCode::SERVICE_UNAVAILABLE, "无法读取 ZCode 授权")
                    }
                }
            };
            match material_at(&deps, &entry.connection_id, origin).await {
                Ok(v) => (v, base),
                Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
            }
        }
    };
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求格式无效"),
    };
    // 模型名可以带来源前缀（`zcode/GLM-5.3`）也可以不带（默认来源兜底时）。
    // 早期这里要求必须有 `/`，会把不带前缀的请求在转发层再拒一次——路由是过了，
    // 用户看到的仍是「请选择来源和模型」。前缀只用于选路，到上游前要剥掉。
    let Some(raw_model) = value["model"].as_str() else {
        return error_response(StatusCode::BAD_REQUEST, "请求缺少模型");
    };
    let model = raw_model
        .split_once('/')
        .map_or(raw_model, |(_, m)| m)
        .trim();
    if model.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "请选择来源和模型");
    }
    let mut target = entry.clone();
    target.secret = Some(token);
    // 请求体是 JSON：必须带上 content-type，否则上游按非对象解析，
    // 返回「Invalid request body: Input should be a valid dictionary」（400）。
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    match protocol {
        CallerProtocol::Anthropic => {
            target.base_url = Some(base.into());
            target.protocol = "anthropic".into();
            super::direct::forward(deps, &target, model.into(), Method::POST, "/v1/messages", &headers, body).await
        }
        // 3.4：GLM Coding Plan 另有 OpenAI 兼容端点（同一把 Key），Chat 直接发；Responses 先翻译成 Chat
        CallerProtocol::Chat | CallerProtocol::Responses => {
            target.base_url = Some(openai_base_of(base));
            target.protocol = "openai".into();
            let (chat, custom) = if protocol == CallerProtocol::Responses {
                match crate::gateway::responses_chat::request(&value) {
                    Ok(chat) => (Bytes::from(chat.to_string()), crate::gateway::responses_chat::custom_tool_names(&value)),
                    Err(reason) => return error_response(StatusCode::BAD_REQUEST, &format!("Responses 请求转 Chat 失败：{reason}")),
                }
            } else {
                (body, Vec::new())
            };
            let response = super::direct::forward(deps, &target, model.into(), Method::POST, "/chat/completions", &headers, chat).await;
            if protocol == CallerProtocol::Responses {
                crate::gateway::responses_chat::response_body(response, custom).await
            } else {
                response
            }
        }
    }
}

/// Coding Plan 的 OpenAI 兼容端点：与 Anthropic 端点同站点，路径 `/api/coding/paas/v4`
/// （BigModel 与 Z.ai 都提供，用同一把 Coding Plan Key）。
pub(crate) fn openai_base_of(anthropic_base: &str) -> String {
    let root = anthropic_base.trim_end_matches('/').trim_end_matches("/api/anthropic");
    format!("{root}/api/coding/paas/v4")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn absent_project_key_never_creates_a_remote_key() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let deps = crate::proxy::test_support::bare_deps();
        let session = crate::zcode_browser_auth::Session {
            access_token: "fixture-login".into(),
            refresh_token: None,
            user_id: "fixture-user".into(),
            account: None,
            zcode_token: "fixture-session".into(),
            family: None,
        };
        let sealed = provider_browser_auth::seal("no-key", &Credential::Zcode { session }).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES('no-key','zcode','auth','fixture',?1,1,'no-key')", [sealed]).unwrap();
        let creates = Arc::new(AtomicUsize::new(0));
        let count = creates.clone();
        let app = axum::Router::new()
            .route("/api/biz/customer/getCustomerInfo", axum::routing::get(|| async {json!({"code":200,"data":{"organizations":[{"organizationId":"org","projects":[{"projectId":"personal","projectType":1}]}]}}).to_string()}))
            .route("/api/biz/v1/organization/org/projects/personal/api_keys", axum::routing::get(||async{json!({"code":200,"data":[]}).to_string()}).post(move || {let count=count.clone();async move {count.fetch_add(1,Ordering::SeqCst);json!({"code":200,"data":{"apiKey":"created"}}).to_string()}}));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let result = material_at(&deps, "no-key", &base).await;
        assert_eq!(
            creates.load(Ordering::SeqCst),
            0,
            "模型查询不能隐式创建远端 API Key"
        );
        assert!(result.unwrap_err().contains("已有"));
        server.abort();
    }

    #[tokio::test]
    async fn credential_exchange_rejects_redirects_without_contacting_destination() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let deps = crate::proxy::test_support::bare_deps();
        let session = crate::zcode_browser_auth::Session {
            access_token: "fixture-login".into(),
            refresh_token: None,
            user_id: "fixture-user".into(),
            account: None,
            zcode_token: "fixture-session".into(),
            family: None,
        };
        let sealed =
            provider_browser_auth::seal("redirect", &Credential::Zcode { session }).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES('redirect','zcode','auth','fixture',?1,1,'redirect')",[sealed]).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let app = axum::Router::new()
            .route(
                "/api/biz/customer/getCustomerInfo",
                axum::routing::get(|| async { axum::response::Redirect::temporary("/unexpected") }),
            )
            .route(
                "/unexpected",
                axum::routing::get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        "{}"
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert!(material_at(&deps, "redirect", &base).await.is_err());
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "携带凭据的业务查询不得跟随重定向"
        );
        server.abort();
    }

    #[test]
    fn serve_capability_follows_the_forwarding_credential_order() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        let local_ok = |_id: &str| -> Result<(String, &'static str, Option<String>), String> {
            Ok(("key.id-secret".into(), "https://api.z.ai/api/anthropic", None))
        };
        let local_broken = |_id: &str| -> Result<(String, &'static str, Option<String>), String> {
            Err("连接 x 未能读取本机 ZCode Coding Plan 凭据：测试断链".to_string())
        };
        let session = crate::zcode_browser_auth::Session {
            access_token: "fixture-login".into(),
            refresh_token: None,
            user_id: "fixture-user".into(),
            account: None,
            zcode_token: "fixture-session".into(),
            family: None,
        };
        // 本机凭据可用时，即便连接没有任何浏览器授权也判定 ready——转发走的就是本机凭据
        conn.execute("INSERT INTO connections(id,platform,kind,name,created_ms,slug) VALUES('monitor:native:zcode:main','zcode','auth','镜像',1,'zcode')", []).unwrap();
        assert_eq!(serve_capability_with(&conn, "monitor:native:zcode:main", &local_ok).unwrap(), SERVE_READY);
        // 本机凭据断链 + 无浏览器授权：needs_auth 的文案要说清缺的是转发凭据，不是登录
        let reason = serve_capability_with(&conn, "monitor:native:zcode:main", &local_broken).unwrap_err();
        assert!(reason.contains("没有浏览器授权"), "{reason}");
        assert!(!reason.contains("需要重新登录"), "{reason}");
        // 本机凭据断链 + 浏览器授权有效：走授权链判定 ready
        let sealed = provider_browser_auth::seal("browser-x", &Credential::Zcode { session: session.clone() }).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES('browser-x','zcode','auth','浏览器',?1,1,'zcode-b')", [sealed]).unwrap();
        assert_eq!(serve_capability_with(&conn, "browser-x", &local_broken).unwrap(), SERVE_READY);
        // 浏览器授权损坏（secret 无法解封）：如实报授权原因，不冒充登录问题
        conn.execute("UPDATE connections SET secret='jolu-browser:v1:broken' WHERE id='browser-x'", []).unwrap();
        let reason = serve_capability_with(&conn, "browser-x", &local_broken).unwrap_err();
        assert!(reason.contains("浏览器授权"), "{reason}");
    }

    #[tokio::test]
    async fn personal_project_token_uses_only_selected_account() {        let deps = crate::proxy::test_support::bare_deps();
        for id in ["a", "b"] {
            let session = crate::zcode_browser_auth::Session {
                access_token: format!("{id}-login"),
                refresh_token: None,
                user_id: id.into(),
                account: None,
                zcode_token: format!("{id}-session"),
                family: None,
            };
            let sealed = provider_browser_auth::seal(id, &Credential::Zcode { session }).unwrap();
            deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,'zcode','auth',?1,?2,1,?1)",rusqlite::params![id,sealed]).unwrap();
        }
        let app=axum::Router::new()
            .route("/api/biz/customer/getCustomerInfo",axum::routing::get(|h:HeaderMap|async move{
                assert_eq!(h["authorization"],"a-login");json!({"code":200,"data":{"organizations":[{"organizationId":"org-a","projects":[{"projectId":"team","projectType":2},{"projectId":"personal-a","projectType":1}]}]}}).to_string()
            }))
            .route("/api/biz/v1/organization/org-a/projects/personal-a/api_keys",axum::routing::get(|h:HeaderMap|async move{
                assert_eq!(h["authorization"],"a-login");json!({"code":200,"data":[{"name":"zcode-api-key","apiKey":"key-a"}]}).to_string()
            }))
            .route("/api/biz/v1/organization/org-a/projects/personal-a/api_keys/key-a/access_tokens",axum::routing::post(|h:HeaderMap|async move{
                assert_eq!(h["authorization"],"Bearer a-login");json!({"code":200,"data":{"accessToken":"a-short-lived","tokenType":"Bearer","expiresIn":3600,"expiresAt":4102444800i64}}).to_string()
            }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(
            material_at(&deps, "a", &base).await.unwrap(),
            "a-short-lived"
        );
        assert_eq!(
            bound(&deps.db.lock().unwrap(), "b").unwrap().1.access_token,
            "b-login"
        );
        assert!(material_at(&deps, "missing", &base).await.is_err());
        server.abort();
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    #[test]
    fn the_openai_compatible_coding_plan_base_sits_beside_the_anthropic_one() {
        assert_eq!(openai_base_of("https://open.bigmodel.cn/api/anthropic"), "https://open.bigmodel.cn/api/coding/paas/v4");
        assert_eq!(openai_base_of("https://api.z.ai/api/anthropic/"), "https://api.z.ai/api/coding/paas/v4");
    }

    #[test]
    fn team_seat_status_explains_why_the_team_key_cannot_be_used() {
        use serde_json::json;
        assert!(seat_verdict(&json!({"status": "EFFECTIVE", "memberGrantStatus": "VALID"})).is_ok());
        assert!(seat_verdict(&json!({"status": "effective", "memberGrantStatus": "valid", "hasSubscription": true})).is_ok(), "大小写不敏感");
        assert!(seat_verdict(&json!({"status": "EFFECTIVE", "memberGrantStatus": "UNASSIGNED"})).unwrap_err().contains("分配席位"));
        assert!(seat_verdict(&json!({"status": "EXPIRED", "memberGrantStatus": "VALID"})).unwrap_err().contains("过期"));
        assert!(seat_verdict(&json!({"hasSubscription": false})).unwrap_err().contains("没有订阅"));
        assert!(seat_verdict(&json!({})).unwrap_err().contains("未知"));
    }
    use std::sync::Mutex;
    use Channel::{CodingPlan, StartPlan};

    #[test]
    fn channel_order_follows_memory_then_free_first() {
        assert_eq!(channel_order(false, None, true), vec![CodingPlan, StartPlan]);
        assert_eq!(channel_order(true, None, true), vec![StartPlan, CodingPlan]);
        assert_eq!(channel_order(false, Some(StartPlan), true), vec![StartPlan, CodingPlan]);
        assert_eq!(channel_order(true, Some(CodingPlan), true), vec![CodingPlan, StartPlan]);
        // 没有会话 JWT 时 Start Plan 不可用，记忆也不让它出现
        assert_eq!(channel_order(true, Some(StartPlan), false), vec![CodingPlan]);
    }

    #[test]
    fn fall_back_rules_differ_per_channel() {
        for status in [429, 402, 502, 504] {
            assert!(falls_back(CodingPlan, StatusCode::from_u16(status).unwrap()), "{status}");
        }
        for status in [400, 401, 404] {
            assert!(!falls_back(CodingPlan, StatusCode::from_u16(status).unwrap()), "{status}");
        }
        // Start Plan 的拒绝语义上游常变，非 2xx 一律落回（与原 free_first 口径一致）
        for status in [400, 403, 429, 503] {
            assert!(falls_back(StartPlan, StatusCode::from_u16(status).unwrap()), "{status}");
        }
        assert!(!falls_back(StartPlan, StatusCode::OK));
    }

    #[test]
    fn channel_memory_expires_after_ttl() {
        let memory = ChannelMemory::default();
        memory.set("c1", StartPlan, 1_000);
        assert_eq!(memory.get("c1", 1_000 + CHANNEL_TTL_MS - 1), Some(StartPlan));
        assert_eq!(memory.get("c2", 1_000), None);
        assert_eq!(memory.get("c1", 1_000 + CHANNEL_TTL_MS), None);
    }

    fn canned(answers: Vec<(Channel, u16)>, calls: Arc<Mutex<Vec<Channel>>>) -> impl FnMut(Channel) -> std::future::Ready<Response> {
        move |channel| {
            calls.lock().unwrap().push(channel);
            let status = answers.iter().find(|(c, _)| *c == channel).map(|(_, s)| *s).unwrap();
            std::future::ready(Response::builder().status(status).body(axum::body::Body::empty()).unwrap())
        }
    }

    #[tokio::test]
    async fn coding_plan_without_quota_is_skipped_after_first_fall_back() {
        let memory = ChannelMemory::default();
        let answers = vec![(CodingPlan, 429), (StartPlan, 200)];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let response = forward_channels(&memory, "c1", false, true, 1_000, canned(answers.clone(), calls.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*calls.lock().unwrap(), vec![CodingPlan, StartPlan]);
        calls.lock().unwrap().clear();
        // 第二次直接走 Start Plan，不再先发一次必然失败的 Coding Plan
        let response = forward_channels(&memory, "c1", false, true, 2_000, canned(answers, calls.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*calls.lock().unwrap(), vec![StartPlan]);
    }

    #[tokio::test]
    async fn exhausted_start_plan_switches_memory_back_to_coding_plan() {
        let memory = ChannelMemory::default();
        memory.set("c1", StartPlan, 1_000);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let response = forward_channels(&memory, "c1", false, true, 2_000,
            canned(vec![(StartPlan, 429), (CodingPlan, 200)], calls.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*calls.lock().unwrap(), vec![StartPlan, CodingPlan]);
        assert_eq!(memory.get("c1", 2_000), Some(CodingPlan));
    }

    #[tokio::test]
    async fn failures_keep_memory_and_return_last_answer() {
        let memory = ChannelMemory::default();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let response = forward_channels(&memory, "c1", false, true, 1_000,
            canned(vec![(CodingPlan, 429), (StartPlan, 503)], calls.clone())).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(memory.get("c1", 1_000), None);
        // 不可转移的失败（400）不换通道
        calls.lock().unwrap().clear();
        let response = forward_channels(&memory, "c1", false, true, 1_000,
            canned(vec![(CodingPlan, 400), (StartPlan, 200)], calls.clone())).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(*calls.lock().unwrap(), vec![CodingPlan]);
    }
}
