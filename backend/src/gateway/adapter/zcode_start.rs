//! ZCode 的 **Start Plan** 通道（免费套餐）：额度只由 zcode.z.ai 自己的端点供应，
//! 凭据是登录会话的 JWT（`zcodejwttoken`），**不是** Coding Plan Key——后者打
//! api.z.ai 会返回 429/1113「余额不足或无资源包」（个人 Coding Plan 才有资源包）。
//!
//! ⚠ 供应商把 Start Plan 限制为「长得像 ZCode 客户端自己的请求」：请求体要按
//! ZCode 3.17.2 的样式「着装」（[`dress`]——系统提示三块逐字节一致、metadata.user_id、
//! cache_control 位置），请求头要带 ZCode 的 UA / App 版本 / 设备号。这是与厂商风控的
//! 军备竞赛：ZCode 升级换文案后 [`zcode_start_prompt`] 的常量要同步更新（来源与核对
//! 方式见该模块头注释）。magpie 为此把 ZCode 退役出主程序交给社区插件维护——Jolu
//! 保留内置实现时承担同样的跟进义务（见 docs/plans 路线图 P2.1 风险表）。

use crate::gateway::zcode_start_prompt as prompt;
use crate::gateway::adapter::direct;
use crate::gateway::route::PoolEntry;
use crate::gateway::wire::error_response;
use axum::response::Response;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde_json::Value;

pub(crate) const START_BASE: &str = "https://zcode.z.ai/api/v1/zcode-plan/anthropic";
const ZCODE_SITE: &str = "https://zcode.z.ai";
// 着装请求与余额查询共用同一个 ZCode 版本（升级时只改 zcode_quota 那一处；
// 下面的 UA 字面量由测试保证与它一致）。
use crate::zcode_quota::ZCODE_APP_VERSION as APP_VERSION;
const UA: &str = "ZCode/3.17.2 ai-sdk/anthropic/3.0.81";

/// ZCode 客户端注册表里的模型 id 写法（magpie 插件 `MODELS`）。余额桶把模型写成小写
/// （`glm-5.3-flash`），请求要用这里的写法；不认识的名字按桶里原样保留。
const KNOWN_MODELS: [&str; 4] = ["GLM-5.3", "GLM-5.3-Flash", "GLM-5.2", "GLM-5-Turbo"];

/// 会话 JWT（`zcodejwttoken` 解密）。Start Plan 的凭据。
pub(crate) fn jwt_at() -> Option<String> {
    let home = crate::creds::home()?;
    let jwt = crate::zcode_quota::coding_plan_jwt_at(&home)?;
    (!jwt.trim().is_empty()).then_some(jwt)
}

/// Start Plan 的设备号：从 JWT 派生稳定 UUID（同账号每次一致，prompt cache 按
/// metadata.user_id 的设备落地——随机设备会让缓存命中率趋近于零）。
fn device_of(jwt: &str) -> String {
    let hash = sha256_hex(&format!("zcode-device\0{jwt}"));
    format!(
        "{}-{}-4{}-{}{}-{}",
        &hash[0..8],
        &hash[8..12],
        &hash[13..16],
        ((hash[16..17].parse::<u8>().unwrap_or(0) & 3) | 8),
        &hash[17..20],
        &hash[20..32]
    )
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn today() -> String {
    let now = chrono::Local::now();
    format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
}

use chrono::Datelike;

/// 第三块的动态段（magpie `environment` 同款；place 为空 = 桌面端无目录分支）。
fn environment() -> String {
    let lines = vec![
        prompt::ENV_HEADING.to_string(),
        prompt::ENV_INVOKED_LINE.to_string(),
        // 本机 OS 版本：取不到精确构建号时不影响（该行在第三块，风控重点在前两块）
        format!(" - {}: {}", prompt::ENV_PLATFORM_LABEL, "win32"),
        format!(" - {}: 10.0", prompt::ENV_OS_VERSION_LABEL),
        format!(" - {}", prompt::ENV_UNTRUSTED_NOTE),
    ];
    lines.join("\n")
}

/// 把 Messages 请求体「着装」成 ZCode 客户端自己的样子（magpie `dress` 同款移植）。
///
/// 关键点：system 必须恰好三块（cliPrefix / 稳定段 / 动态段）——zcode.z.ai 对第四块
/// 返回 405/3012；agent 自己的系统文本并入第三块；metadata.user_id 承载设备号与
/// 会话 id（额度缓存按它落地）；工具与消息内容里的 cache_control 位置重排。
fn dress(body: &mut Value, device: &str, session: &str) {
    let cached = |t: String| {
        serde_json::json!({"type": "text", "text": t, "cache_control": {"type": "ephemeral"}})
    };
    // agent 自己的系统文本（字符串或块数组）收进第三块的末尾
    let mut own: Vec<String> = Vec::new();
    match body.get("system") {
        Some(Value::String(text)) => {
            if !text.trim().is_empty() {
                own.push(text.clone());
            }
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                if block["type"] == "text" {
                    if let Some(text) = block["text"].as_str().filter(|t| !t.trim().is_empty()) {
                        own.push(text.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    let skill = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(|t| t["name"] == "Skill"));
    let model = body["model"].as_str().unwrap_or("").to_string();
    let mut dynamic = vec![prompt::DYNAMIC_BEHAVIOR.to_string()];
    if skill {
        dynamic.push(prompt::SESSION_GUIDANCE.to_string());
    }
    dynamic.push(environment());
    if !model.is_empty() {
        dynamic.push(prompt::POWERED_BY.replace("{provider}", "zai-api").replace("{model}", &model));
    }
    dynamic.push(prompt::CONTEXT_MANAGEMENT.to_string());
    dynamic.extend(own);
    let dynamic_text = format!("\n\n{}", dynamic.join("\n\n"));
    body["system"] = serde_json::json!([
        cached(prompt::CLI_PREFIX.to_string()),
        cached(prompt::STABLE_SECTIONS.to_string()),
        cached(dynamic_text),
    ]);
    // ZCode 在 agent 消息前放一条日期 user 消息
    let date_line = prompt::CONTEXT_DATE_LINE.replace("{date}", &today());
    let prefix = serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": format!("<system-reminder>{date_line}</system-reminder>")}]
    });
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        messages.insert(0, prefix);
        let mut last_index = None;
        for (index, message) in messages.iter_mut().enumerate() {
            if message["role"] == "system" {
                continue;
            }
            last_index = Some(index);
            if let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) {
                for block in content.iter_mut() {
                    if let Some(object) = block.as_object_mut() {
                        object.remove("cache_control");
                    }
                }
            }
        }
        if let Some(index) = last_index {
            let message = &mut messages[index];
            match message.get_mut("content") {
                Some(Value::String(text)) => {
                    let text = text.clone();
                    message["content"] = serde_json::json!([cached(text)]);
                }
                Some(Value::Array(content)) if !content.is_empty() => {
                    if let Some(block) = content.last_mut().and_then(|b| b.as_object_mut()) {
                        block.entry("cache_control").or_insert_with(|| serde_json::json!({"type": "ephemeral"}));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools.iter_mut() {
            if let Some(object) = tool.as_object_mut() {
                object.remove("cache_control");
            }
        }
    }
    let meta = body.get("metadata").and_then(Value::as_object).cloned().unwrap_or_default();
    let user_id = serde_json::json!({"device_id": device, "account_uuid": "", "session_id": session});
    let mut metadata = meta;
    metadata.insert("user_id".into(), user_id);
    body["metadata"] = Value::Object(metadata);
}

/* ───────────────────────── ③3 忙碌重试（magpie `askBusy` 同款） ───────────────────────── */

/// 429 后每次重问的等待秒数（magpie `BUSY_WAITS` = [1000, 2000] 同款：ZCode 自己的
/// 客户端等 1 秒、2 秒各重问一次，之后才认输）。
const BUSY_WAITS_SECS: [u64; 2] = [1, 2];

/// 一次忙碌等待的实际时长：响应带可解析为秒数的 Retry-After 时取
/// min(Retry-After, 对应间隔)，否则等间隔本身（HTTP 日期形式不解析——上游实际
/// 返回的都是秒数形式，解析不了就按 ZCode 客户端自己的节奏等）。
fn busy_wait(response: &Response, wait_secs: u64) -> std::time::Duration {
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    std::time::Duration::from_secs(retry_after.unwrap_or(wait_secs).min(wait_secs))
}

/// 忙碌重试的骨架：先问一次；429 时按 [`BUSY_WAITS_SECS`] 的间隔重问，最多两次
/// （`send` 每次发出**同一个**请求——同一着装后的请求体与同一头，见 [`forward`]）；
/// 两次后仍 429 就把最后一次响应交回调用方。任务规格对任意 429 都重问（magpie
/// 只重问 3008/3009/3010 忙碌码）：Start Plan 的 429 兼有额度语义，多问两次的
/// 代价是固定的 3 秒上限，换来的是并发偶发上限不把请求打回给 agent。
async fn ask_busy<F, Fut>(mut send: F) -> Response
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Response>,
{
    let mut response = send().await;
    for wait in BUSY_WAITS_SECS {
        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return response;
        }
        tokio::time::sleep(busy_wait(&response, wait)).await;
        response = send().await;
    }
    response
}

/// Start Plan 的 messages 转发。`session` 是调用方会话键（cache 与 x-session-id 用）。
/* ───────────────────────── C4 每账号同时请求数（magpie 插件 `startGate` 同款） ───────────────────────── */

/// 一个账号的 Start Plan 同时在途请求数（插件实测：12 个并发里服务了 3–4 个）。
const START_INFLIGHT: u32 = 3;
/// 排队最多等这么久，到时本地回 429，不发出去被拒。
const START_QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/* ───────────────────────── P2-8 -Trial 模型按模型休息（插件 `blocked` 同款） ───────────────────────── */

/// 一个模型的礼包被拒（额度用完）后，这个账号的这个模型休息多久。
const MODEL_REST_MS: i64 = 60_000;

static RESTED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> =
    std::sync::LazyLock::new(Default::default);

fn rest_key(device: &str, model: &str) -> String {
    format!("{device}\0{}", model.to_ascii_lowercase())
}

fn rested_until(device: &str, model: &str, now: i64) -> Option<i64> {
    let mut map = RESTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    map.retain(|_, until| *until > now);
    map.get(&rest_key(device, model)).copied()
}

fn rest_model(device: &str, model: &str, now: i64) {
    RESTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(rest_key(device, model), now + MODEL_REST_MS);
}

/// 上游的拒绝是不是「这个模型的礼包额度用完」：402，或 429 带余额不足类的业务码（1113 / 1005）。
/// 忙碌（3008–3010）不算——那是排队问题，由 [`ask_busy`] 重问。
fn quota_refusal(status: StatusCode, body: &str) -> bool {
    if status == StatusCode::PAYMENT_REQUIRED {
        return true;
    }
    status == StatusCode::TOO_MANY_REQUESTS
        && ["\"1113\"", ":1113", "\"1005\"", ":1005", "余额不足", "insufficient balance"].iter().any(|code| body.contains(code))
}

pub(crate) async fn forward(
    deps: std::sync::Arc<crate::proxy::ProxyDeps>,
    entry: &PoolEntry,
    protocol: crate::gateway::wire::CallerProtocol,
    body: Bytes,
    session: &str,
) -> Response {
    if protocol != crate::gateway::wire::CallerProtocol::Anthropic {
        return error_response(StatusCode::BAD_REQUEST, "ZCode Start Plan 使用 Anthropic Messages 协议");
    }
    // C5：边缘保持期内不再敲 zcode.z.ai（必然失败，且会延长防火墙的封禁）
    if let Some(until) = EDGE.until(chrono::Utc::now().timestamp_millis()) {
        return refused_answer(Refusal::Edge, until);
    }
    let Some(jwt) = jwt_at() else {
        return error_response(StatusCode::UNAUTHORIZED, "ZCode 会话凭据缺失：请在原工具重新登录");
    };
    let device = device_of(&jwt);
    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求格式无效"),
    };
    // 礼包外的模型如实交给上游判定（礼包清单见 [`gift_models`]）
    dress(&mut value, &device, session);
    let dressed = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("请求体着装失败：{error}")),
    };
    let mut target = entry.clone();
    target.secret = Some(jwt);
    target.base_url = Some(START_BASE.into());
    // Start Plan 端点要 Authorization: Bearer <JWT>（anthropic 的 x-api-key 会被 401 拒）；
    // 借 openai 协议的鉴权形态，anthropic-version 由下面的调用方头自带。
    target.protocol = "openai".into();
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("HTTP-Referer", HeaderValue::from_static(ZCODE_SITE));
    headers.insert("user-agent", HeaderValue::from_static(UA));
    headers.insert("X-ZCode-App-Version", HeaderValue::from_static(APP_VERSION));
    headers.insert("X-Device-Mid", HeaderValue::from_str(&device).unwrap_or(HeaderValue::from_static("")));
    headers.insert(
        "x-session-id",
        HeaderValue::from_str(session).unwrap_or(HeaderValue::from_static("")),
    );
    // Start Plan 通道不吃路径查询（Claude Code 的 ?beta=true 剥掉——magpie startURL 同款）。
    // 发出走忙碌重试（③3）：着装只做一次，闭包每次重用同一份着装后的请求体与同一头——
    // 重复着装会重排 cache_control 并让 prompt cache 失效。
    let model = value["model"].as_str().unwrap_or_default().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    if let Some(until) = rested_until(&device, &model, now) {
        let seconds = ((until - now) / 1000).max(1) as u64;
        return crate::gateway::wire::error_response_with_retry(
            StatusCode::TOO_MANY_REQUESTS,
            &format!("ZCode Start Plan 的「{model}」礼包额度刚被拒（用完或暂不可用），这个模型休息 {seconds} 秒；其他模型不受影响"),
            seconds,
        );
    }
    // C4：同一账号最多 3 个在途，其余按到达顺序排队；客户端断开即离队
    let slot = match tokio::time::timeout(START_QUEUE_WAIT, crate::gateway::key_slots::acquire(&format!("zcode-start\0{device}"), Some(START_INFLIGHT))).await {
        Ok(slot) => slot,
        Err(_) => {
            return crate::gateway::wire::error_response_with_retry(
                StatusCode::TOO_MANY_REQUESTS,
                "ZCode Start Plan 同一账号同时只服务几个请求，排队超过 2 分钟仍没轮到：请减少同时运行的 agent 或子代理",
                30,
            );
        }
    };
    let rest_model_name = model.clone();
    let rest_device = device.clone();
    let response = ask_busy(move || {
        let deps = deps.clone();
        let target = target.clone();
        let headers = headers.clone();
        let dressed = dressed.clone();
        let model = model.clone();
        async move {
            direct::forward(
                deps,
                &target,
                model,
                Method::POST,
                "/v1/messages",
                &headers,
                dressed.into(),
            )
            .await
        }
    })
    .await;
    let response = settle(response, &rest_device, &rest_model_name).await;
    crate::gateway::key_slots::hold(response, slot)
}

/* ───────────────────────── C1 礼包按余额过滤（magpie `startPlan` 同款） ───────────────────────── */

/// 余额查询结果的缓存时长：余额有网络成本，礼包模型一天内基本不变。
const GIFT_TTL_MS: i64 = 5 * 60_000;

fn num(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("")
}

/// 桶属于哪个套餐：两边都有 `user_plan_id` 时按它对，否则按 `plan_id`。
fn same_plan(bucket: &Value, plan: &Value) -> bool {
    let (bucket_user, plan_user) = (text(bucket, "user_plan_id"), text(plan, "user_plan_id"));
    if !bucket_user.is_empty() && !plan_user.is_empty() {
        return bucket_user == plan_user;
    }
    bucket.get("plan_id").map(Value::to_string) == plan.get("plan_id").map(Value::to_string)
}

/// 桶已用完：有 `remaining_units` 看它，没有时看 used ≥ total；都没有不算用完。
fn bucket_spent(bucket: &Value) -> bool {
    if let Some(left) = num(bucket.get("remaining_units")) {
        return left <= 0.0;
    }
    matches!(
        (num(bucket.get("total_units")), num(bucket.get("used_units"))),
        (Some(total), Some(used)) if used >= total
    )
}

/// 桶声明的模型：`capabilities` 里 `model:` 开头或不带冒号的条目（`tool:` 之类不是模型）。
fn bucket_models(bucket: &Value) -> Vec<String> {
    bucket
        .get("capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|c| c.starts_with("model:") || !c.contains(':'))
        .map(|c| c.trim_start_matches("model:").trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

fn wire_model(name: &str) -> String {
    KNOWN_MODELS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .map_or_else(|| name.to_string(), |known| known.to_string())
}

/// 余额 `data` → 现在能花的礼包模型。`None` = 没有生效中的套餐（含读不出 `plans`）；
/// `Some(空)` = 有生效套餐但桶都用完 / 过期——两者分开，不把「没有」说成「不知道」。
pub(crate) fn gift_models_from_balance(data: &Value) -> Option<Vec<String>> {
    let now = num(data.get("server_time"))
        .filter(|n| *n > 0.0)
        .unwrap_or_else(|| chrono::Utc::now().timestamp() as f64);
    let plans = data.get("plans").and_then(Value::as_array)?;
    let balances = data.get("balances").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let past = |v: Option<f64>| v.is_some_and(|t| t > 0.0 && t <= now);
    let mut live_plan = false;
    let mut models: Vec<String> = Vec::new();
    for plan in plans {
        let started = num(plan.get("starts_at")).is_none_or(|t| t <= now);
        if !text(plan, "status").eq_ignore_ascii_case("active") || past(num(plan.get("ends_at"))) || !started {
            continue;
        }
        live_plan = true;
        for bucket in balances.iter().filter(|b| same_plan(b, plan)) {
            if past(num(bucket.get("expires_at"))) || bucket_spent(bucket) {
                continue;
            }
            for name in bucket_models(bucket) {
                let name = wire_model(&name);
                if !models.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
                    models.push(name);
                }
            }
        }
    }
    live_plan.then_some(models)
}

/// 按会话 JWT 的哈希缓存余额结论（只缓存读成功的结果；读失败下次再问）。
#[derive(Default)]
struct GiftCache(std::sync::Mutex<std::collections::HashMap<String, (i64, Option<Vec<String>>)>>);

impl GiftCache {
    fn get(&self, jwt: &str, now_ms: i64) -> Option<Option<Vec<String>>> {
        let map = self.0.lock().ok()?;
        let (at, value) = map.get(&sha256_hex(jwt))?;
        (now_ms - at < GIFT_TTL_MS).then(|| value.clone())
    }

    fn put(&self, jwt: &str, value: Option<Vec<String>>, now_ms: i64) {
        if let Ok(mut map) = self.0.lock() {
            map.retain(|_, (at, _)| now_ms - *at < GIFT_TTL_MS);
            map.insert(sha256_hex(jwt), (now_ms, value));
        }
    }
}

static GIFTS: std::sync::LazyLock<GiftCache> = std::sync::LazyLock::new(GiftCache::default);

/// 本机 ZCode 会话现在能花的 Start Plan 礼包模型（读 `billing/balance`，5 分钟缓存）。
/// `Ok(None)` = 没有生效套餐；`Err` = 读不到（会话失效时给可操作的说明）。
pub(crate) async fn gift_models() -> Result<Option<Vec<String>>, String> {
    let jwt = jwt_at().ok_or("ZCode 会话凭据缺失：请在原工具重新登录")?;
    let now = chrono::Utc::now().timestamp_millis();
    if let Some(hit) = GIFTS.get(&jwt, now) {
        return Ok(hit);
    }
    let (token, device) = (jwt.clone(), device_of(&jwt));
    let body = tokio::task::spawn_blocking(move || crate::zcode_quota::plan_balance_body(&token, &device))
        .await
        .map_err(|_| "ZCode 余额查询失败".to_string())??;
    let gift = gift_models_from_balance(body.get("data").unwrap_or(&Value::Null));
    GIFTS.put(&jwt, gift.clone(), now);
    Ok(gift)
}

/* ───────────────────────── C5 边缘防火墙保持（magpie `edge` 同款） ───────────────────────── */

/// 被边缘防火墙拦下后，本进程暂停请求 zcode.z.ai 的时长（magpie `EDGE_HOLD` = 3 分钟：
/// 防火墙按 IP 计数，期间每个账号、每个会话再敲都只会延长封禁；时长短到换网络 / 代理的
/// 用户也不必久等）。
const EDGE_HOLD_MS: i64 = 3 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// 阿里云边缘防火墙的拦截页：本机网络地址被拦，与账号无关
    Edge,
    /// ZCode 风控（405 / 3012「unusual activity」）：账号被限制
    Restricted,
}

/// Start Plan 的回应是否是拒绝，是哪一种（magpie `startRefused` / `EDGE_PAGE` 同款判定）。
fn refusal_of(status: StatusCode, body: &str) -> Option<Refusal> {
    if status.as_u16() < 400 {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    let code_3012 = serde_json::from_str::<Value>(body).ok().is_some_and(|v| {
        [v.get("code"), v.pointer("/error/code")]
            .into_iter()
            .flatten()
            .any(|c| c.as_i64() == Some(3012) || c.as_str() == Some("3012"))
    });
    let refused = status == StatusCode::METHOD_NOT_ALLOWED
        || lower.contains("unusual activity")
        || lower.contains("request has been blocked")
        || code_3012;
    if !refused {
        return None;
    }
    if lower.contains("errors.aliyun.com") || lower.contains("<html") {
        Some(Refusal::Edge)
    } else {
        Some(Refusal::Restricted)
    }
}

/// 进程级的边缘保持：所有账号都从同一个出口地址出去，所以保持属于进程而不是账号。
#[derive(Default)]
struct EdgeHold(std::sync::Mutex<Option<i64>>);

impl EdgeHold {
    fn until(&self, now_ms: i64) -> Option<i64> {
        let until = (*self.0.lock().ok()?)?;
        (now_ms < until).then_some(until)
    }

    fn hold(&self, now_ms: i64) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(now_ms + EDGE_HOLD_MS);
        }
    }

    fn clear(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }
}

static EDGE: std::sync::LazyLock<EdgeHold> = std::sync::LazyLock::new(EdgeHold::default);

/// 拒绝的回应：都落在网关的可转移错误里（503 / 403），路由分组会换下一个成员。
fn refused_answer(kind: Refusal, until_ms: i64) -> Response {
    match kind {
        Refusal::Edge => {
            let until = chrono::DateTime::from_timestamp_millis(until_ms)
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                .unwrap_or_default();
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("ZCode Start Plan 被服务商的网络防火墙拦截（本机网络地址，与账号无关）：{until} 前不再请求；可稍后重试，或切换网络 / 代理"),
            )
        }
        Refusal::Restricted => error_response(
            StatusCode::FORBIDDEN,
            "ZCode 风控限制了该账号（405 / code 3012「unusual activity」）：ZCode 客户端在该账号上也会失败，只能联系 ZCode 客服解除",
        ),
    }
}

/// 把 Start Plan 的上游回应按边缘保持规则收尾：成功清除保持；拒绝改写成可读的
/// 可转移错误（拦截页同时开始保持）；其他失败原样交回。
async fn settle(response: Response, device: &str, model: &str) -> Response {
    if response.status().is_success() {
        EDGE.clear();
        return response;
    }
    if response.status().as_u16() < 400 {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, 256 * 1024).await.unwrap_or_default();
    if quota_refusal(parts.status, &String::from_utf8_lossy(&bytes)) {
        // 只让这个账号的这个模型休息，不动整条连接（P2-8）
        rest_model(device, model, chrono::Utc::now().timestamp_millis());
    }
    match refusal_of(parts.status, &String::from_utf8_lossy(&bytes)) {
        Some(Refusal::Edge) => {
            let now = chrono::Utc::now().timestamp_millis();
            EDGE.hold(now);
            refused_answer(Refusal::Edge, now + EDGE_HOLD_MS)
        }
        Some(Refusal::Restricted) => refused_answer(Refusal::Restricted, 0),
        None => Response::from_parts(parts, axum::body::Body::from(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zcode_quota::balance_url;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// canned 响应的请求骨架：第 n 次调用返回 `sequence[n]`，序号越界后重复最后一个；
    /// `hits` 记录请求次数。Retry-After 给 "0" 让忙碌等待即时完成，测试不必真睡。
    fn canned_sequence(
        sequence: Vec<(StatusCode, Option<&'static str>)>,
        hits: Arc<AtomicUsize>,
    ) -> impl FnMut() -> std::future::Ready<Response> + Send {
        let sequence = Arc::new(Mutex::new(sequence));
        move || {
            hits.fetch_add(1, Ordering::SeqCst);
            let (status, retry_after) = {
                let mut slot = sequence.lock().unwrap();
                if slot.len() > 1 { slot.remove(0) } else { slot[0] }
            };
            let mut builder = Response::builder().status(status);
            if let Some(value) = retry_after {
                builder = builder.header("retry-after", value);
            }
            std::future::ready(builder.body(axum::body::Body::empty()).unwrap())
        }
    }

    #[test]
    fn a_trial_models_quota_refusal_rests_only_that_model() {
        assert!(quota_refusal(StatusCode::PAYMENT_REQUIRED, ""));
        assert!(quota_refusal(StatusCode::TOO_MANY_REQUESTS, r#"{"error":{"code":"1113","message":"余额不足"}}"#));
        assert!(!quota_refusal(StatusCode::TOO_MANY_REQUESTS, r#"{"error":{"code":"3008","message":"user concurrency limit exceeded"}}"#), "忙碌不是用完");
        assert!(!quota_refusal(StatusCode::BAD_REQUEST, "1113"));
        let now = 1_000_000;
        rest_model("dev-rest", "GLM-5.3-Trial", now);
        assert!(rested_until("dev-rest", "glm-5.3-trial", now + 1).is_some(), "大小写不敏感");
        assert!(rested_until("dev-rest", "GLM-4.6-Trial", now + 1).is_none(), "别的模型不受影响");
        assert!(rested_until("dev-other", "GLM-5.3-Trial", now + 1).is_none(), "别的账号不受影响");
        assert!(rested_until("dev-rest", "GLM-5.3-Trial", now + MODEL_REST_MS + 1).is_none(), "到时恢复");
    }

    #[test]
    fn busy_wait_prefers_retry_after_but_caps_at_interval() {
        let with_header = |value: &'static str| {
            Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("retry-after", value)
                .body(axum::body::Body::empty())
                .unwrap()
        };
        // Retry-After 比间隔大：取间隔
        assert_eq!(busy_wait(&with_header("7"), 2), std::time::Duration::from_secs(2));
        // Retry-After 比间隔小：等它说的
        assert_eq!(busy_wait(&with_header("1"), 2), std::time::Duration::from_secs(1));
        // 解析不了（HTTP 日期形式）：按间隔等
        assert_eq!(
            busy_wait(&with_header("Fri, 31 Dec 2021 23:59:59 GMT"), 2),
            std::time::Duration::from_secs(2)
        );
        // 没带 Retry-After：按间隔等
        let bare = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(busy_wait(&bare, 1), std::time::Duration::from_secs(1));
    }

    #[tokio::test]
    async fn ask_busy_retries_429_twice_and_returns_first_success() {
        // 429 → 429 → 200：第三次成功，共发 3 次请求；Retry-After 给 0 让等待即时完成
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let send = canned_sequence(
            vec![
                (StatusCode::TOO_MANY_REQUESTS, Some("0")),
                (StatusCode::TOO_MANY_REQUESTS, Some("0")),
                (StatusCode::OK, None),
            ],
            counter,
        );
        let response = ask_busy(send).await;
        assert_eq!(hits.load(Ordering::SeqCst), 3, "两次重问后应该有第三次请求");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ask_busy_gives_up_after_two_retries_and_returns_last_429() {
        // 一直 429：最多 3 次请求（首发 + 两次重问），返回最后一次 429
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let send = canned_sequence(vec![(StatusCode::TOO_MANY_REQUESTS, Some("0"))], counter);
        let response = ask_busy(send).await;
        assert_eq!(hits.load(Ordering::SeqCst), 3, "首发之外最多重问两次");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn ask_busy_passes_non_429_through_without_retry() {
        // 非 429（如 401）一次都不重问
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let send = canned_sequence(vec![(StatusCode::UNAUTHORIZED, None)], counter);
        let response = ask_busy(send).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1, "非 429 不重问");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ask_busy_returns_200_immediately_when_first_answer_succeeds() {
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let send = canned_sequence(vec![(StatusCode::OK, None)], counter);
        let response = ask_busy(send).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(response.status(), StatusCode::OK);
    }

    /* ───── C1 礼包按余额过滤 ───── */

    fn balance(plans: Value, balances: Value) -> Value {
        serde_json::json!({ "server_time": 1_000_000i64, "plans": plans, "balances": balances })
    }

    #[test]
    fn gift_models_lists_live_bucket_models_in_wire_casing() {
        let body = balance(
            serde_json::json!([{ "user_plan_id": "u1", "plan_id": "start", "status": "active", "starts_at": 900_000i64, "ends_at": 2_000_000i64 }]),
            serde_json::json!([
                { "user_plan_id": "u1", "capabilities": ["model:glm-5.3-flash"], "remaining_units": 10 },
                { "user_plan_id": "u1", "capabilities": ["model:glm-5.2", "tool:web-search", "glm-5-turbo"], "remaining_units": 5 },
                { "user_plan_id": "u1", "capabilities": ["model:glm-future-preview"], "remaining_units": 5 }
            ]),
        );
        // 已知模型还原成 ZCode 请求里的大小写，未知模型按桶里写的原样保留；tool: 能力不算模型
        assert_eq!(
            gift_models_from_balance(&body),
            Some(vec!["GLM-5.3-Flash".to_string(), "GLM-5.2".into(), "GLM-5-Turbo".into(), "glm-future-preview".into()])
        );
    }

    #[test]
    fn gift_models_skip_spent_expired_and_foreign_buckets() {
        let body = balance(
            serde_json::json!([{ "user_plan_id": "u1", "plan_id": "start", "status": "active", "ends_at": 2_000_000i64 }]),
            serde_json::json!([
                { "user_plan_id": "u1", "capabilities": ["model:glm-5.3"], "remaining_units": 0 },
                { "user_plan_id": "u1", "capabilities": ["model:glm-5.2"], "used_units": 100, "total_units": 100 },
                { "user_plan_id": "u1", "capabilities": ["model:glm-5-turbo"], "remaining_units": 5, "expires_at": 999_999i64 },
                { "user_plan_id": "other", "capabilities": ["model:glm-5.3-flash"], "remaining_units": 5 }
            ]),
        );
        // 有生效套餐但桶都用完 / 过期 / 不属于它：Some(空)，与「读不到余额」区分
        assert_eq!(gift_models_from_balance(&body), Some(vec![]));
    }

    #[test]
    fn gift_models_are_none_without_a_live_plan() {
        let bucket = serde_json::json!([{ "user_plan_id": "u1", "capabilities": ["model:glm-5.2"], "remaining_units": 5 }]);
        for plan in [
            serde_json::json!({ "user_plan_id": "u1", "status": "expired" }),
            serde_json::json!({ "user_plan_id": "u1", "status": "active", "ends_at": 999_999i64 }),
            serde_json::json!({ "user_plan_id": "u1", "status": "active", "starts_at": 1_000_001i64 }),
        ] {
            assert_eq!(gift_models_from_balance(&balance(serde_json::json!([plan]), bucket.clone())), None);
        }
        assert_eq!(gift_models_from_balance(&serde_json::json!({})), None);
    }

    #[test]
    fn gift_cache_expires_after_ttl_and_is_keyed_by_session() {
        let cache = GiftCache::default();
        cache.put("jwt-a", Some(vec!["GLM-5.2".into()]), 1_000);
        assert_eq!(cache.get("jwt-a", 1_000 + GIFT_TTL_MS - 1), Some(Some(vec!["GLM-5.2".to_string()])));
        assert_eq!(cache.get("jwt-b", 1_000), None);
        assert_eq!(cache.get("jwt-a", 1_000 + GIFT_TTL_MS), None);
    }

    #[test]
    fn balance_url_carries_app_version_and_ua_matches_it() {
        assert!(balance_url().ends_with(&format!("?app_version={APP_VERSION}")));
        assert!(UA.starts_with(&format!("ZCode/{APP_VERSION} ")));
    }

    /* ───── C5 边缘防火墙保持 ───── */

    #[test]
    fn refusal_kind_tells_edge_page_from_account_restriction() {
        let page = "<html><body>blocked, see errors.aliyun.com</body></html>";
        assert_eq!(refusal_of(StatusCode::METHOD_NOT_ALLOWED, page), Some(Refusal::Edge));
        // 不是拒绝（405 / 风控文案）的 HTML 错误页——例如代理的 502——不算边缘拦截
        assert_eq!(refusal_of(StatusCode::BAD_GATEWAY, "<html>bad gateway</html>"), None);
        let restricted = r#"{"code":3012,"msg":"request has been blocked due to unusual activity"}"#;
        assert_eq!(refusal_of(StatusCode::METHOD_NOT_ALLOWED, restricted), Some(Refusal::Restricted));
        assert_eq!(refusal_of(StatusCode::METHOD_NOT_ALLOWED, "{}"), Some(Refusal::Restricted));
        assert_eq!(refusal_of(StatusCode::TOO_MANY_REQUESTS, r#"{"code":1113}"#), None);
        assert_eq!(refusal_of(StatusCode::OK, page), None);
    }

    #[test]
    fn edge_hold_lasts_its_window_and_clears_on_success() {
        let hold = EdgeHold::default();
        assert_eq!(hold.until(0), None);
        hold.hold(1_000);
        assert_eq!(hold.until(1_000 + EDGE_HOLD_MS - 1), Some(1_000 + EDGE_HOLD_MS));
        assert_eq!(hold.until(1_000 + EDGE_HOLD_MS), None);
        hold.hold(5_000);
        hold.clear();
        assert_eq!(hold.until(5_001), None);
    }

    #[tokio::test]
    async fn refusal_answers_are_transferable_and_readable() {
        // 503 / 403 都在网关的可转移错误里，路由分组会换下一个成员
        let edge = refused_answer(Refusal::Edge, 1_000 + EDGE_HOLD_MS);
        assert_eq!(edge.status(), StatusCode::SERVICE_UNAVAILABLE);
        let text = String::from_utf8(axum::body::to_bytes(edge.into_body(), 4096).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("网络防火墙"), "{text}");
        let restricted = refused_answer(Refusal::Restricted, 0);
        assert_eq!(restricted.status(), StatusCode::FORBIDDEN);
        let text = String::from_utf8(axum::body::to_bytes(restricted.into_body(), 4096).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("3012"), "{text}");
    }
}

#[cfg(test)]
mod local {
    /// 真机只读：用本机 ZCode 会话读一次余额，打印礼包模型（模型名不是敏感值，令牌不打印）。
    #[tokio::test]
    #[ignore = "需要本机 ZCode 登录与网络；仅显式只读执行"]
    async fn local_start_plan_gift_readonly_smoke() {
        let present = super::jwt_at().is_some();
        println!("ZCode session jwt present: {present}");
        let Some(jwt) = super::jwt_at() else { return };
        println!("Start Plan gift: {:?}", super::gift_models().await);
        // 结构摘要：套餐名 / 状态 / 起止，桶的归属与能力——都不含令牌与用户标识
        let device = super::device_of(&jwt);
        let body = tokio::task::spawn_blocking(move || crate::zcode_quota::plan_balance_body(&jwt, &device)).await.unwrap();
        match body {
            Ok(body) => {
                let data = &body["data"];
                println!("server_time={} plans={}", data["server_time"], data["plans"].as_array().map_or(0, Vec::len));
                for plan in data["plans"].as_array().into_iter().flatten() {
                    println!("  plan name={} status={} starts={} ends={}", plan["name"], plan["status"], plan["starts_at"], plan["ends_at"]);
                }
                for bucket in data["balances"].as_array().into_iter().flatten() {
                    println!("  bucket show={} caps={} remaining={} expires={}", bucket["show_name"], bucket["capabilities"], bucket["remaining_units"], bucket["expires_at"]);
                }
            }
            Err(error) => println!("balance error: {error}"),
        }
    }
}
