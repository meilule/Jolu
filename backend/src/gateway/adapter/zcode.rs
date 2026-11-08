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

/// Start Plan 的静态清单（免费套餐：GLM-5.3-Flash / GLM-5.2 / GLM-5-Turbo）。
pub(crate) fn start_plan_models() -> Option<Vec<String>> {
    super::zcode_start::jwt_at().map(|_| super::zcode_start::models())
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

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
    session: &str,
) -> Response {
    if protocol != CallerProtocol::Anthropic {
        return error_response(
            StatusCode::BAD_REQUEST,
            "ZCode 订阅使用 Anthropic Messages 协议",
        );
    }
    // ③5 free_first：开启且本机有登录会话 JWT 时，先试免费的 Start Plan 通道
    // （额度在 zcode.z.ai 自己的端点上）；JWT 缺失时直接走下面的现有顺序，不报错。
    // 落回判定取「非 2xx 即落回」的粗口径：Start Plan 的拒绝语义（忙碌 429、额度
    // 用尽 1113/1005、风控文案）上游随时会变，细分类容易在语义漂移时漏落回；
    // 保守判定的代价只是多付一次已完成的请求，不会把可用的 Coding Plan 拒之门外。
    if free_first() && super::zcode_start::jwt_at().is_some() {
        let attempt =
            super::zcode_start::forward(deps.clone(), entry, protocol, body.clone(), session).await;
        if attempt.status().is_success() {
            return attempt;
        }
    }
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
    target.base_url = Some(base.into());
    target.protocol = "anthropic".into();
    // 请求体是 JSON：必须带上 content-type，否则上游按非对象解析，
    // 返回「Invalid request body: Input should be a valid dictionary」（400）。
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let response = super::direct::forward(
        deps.clone(),
        &target,
        model.into(),
        Method::POST,
        "/v1/messages",
        &headers,
        body.clone(),
    )
    .await;
    // Start Plan 兜底：Coding Plan Key 认证通过但账号没有对应资源包（429/1113、402），
    // 而登录会话的 JWT 在——免费 Start Plan 的额度在 zcode.z.ai 自己的端点上
    // （用会话 JWT + ZCode 同款着装请求）。有 Coding Plan 的账号不受影响。
    let billing = response.status() == StatusCode::TOO_MANY_REQUESTS
        || response.status() == StatusCode::PAYMENT_REQUIRED;
    if billing && super::zcode_start::jwt_at().is_some() {
        return super::zcode_start::forward(deps, entry, protocol, body, session).await;
    }
    response
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

    #[tokio::test]
    async fn personal_project_token_uses_only_selected_account() {
        let deps = crate::proxy::test_support::bare_deps();
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
