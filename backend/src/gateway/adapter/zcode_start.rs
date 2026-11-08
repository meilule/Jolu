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
const APP_VERSION: &str = "3.17.2";
const UA: &str = "ZCode/3.17.2 ai-sdk/anthropic/3.0.81";

/// Start Plan 供给的模型（magpie 插件 START_MODELS 同款：全部除 GLM-5.3）。
pub(crate) const START_MODELS: [&str; 3] = ["GLM-5.3-Flash", "GLM-5.2", "GLM-5-Turbo"];

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
    let mut lines = vec![
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

/// Start Plan 的 messages 转发。`session` 是调用方会话键（cache 与 x-session-id 用）。
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
    let Some(jwt) = jwt_at() else {
        return error_response(StatusCode::UNAUTHORIZED, "ZCode 会话凭据缺失：请在原工具重新登录");
    };
    let device = device_of(&jwt);
    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求格式无效"),
    };
    // Start Plan 不提供 GLM-5.3（magpie START_MODELS 同款）；未知模型如实交给上游判定
    dress(&mut value, &device, session);
    let dressed = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("请求体着装失败：{error}")),
    };
    let mut target = entry.clone();
    target.secret = Some(jwt);
    target.base_url = Some(START_BASE.into());
    target.protocol = "anthropic".into();
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
    // Start Plan 通道不吃路径查询（Claude Code 的 ?beta=true 剥掉——magpie startURL 同款）
    direct::forward(
        deps,
        &target,
        value["model"].as_str().unwrap_or_default().to_string(),
        Method::POST,
        "/v1/messages",
        &headers,
        dressed.into(),
    )
    .await
}

/// Start Plan 的模型清单（静态：免费套餐的四个模型少 GLM-5.3）。
pub(crate) fn models() -> Vec<String> {
    START_MODELS.iter().map(|m| m.to_string()).collect()
}
