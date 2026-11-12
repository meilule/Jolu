//! 意图路由（3.9 / magpie E3）：`intent` 策略的路由分组在每个新的人工回合用一个小模型判一次意图，
//! 强档成员服务需要动脑的回合（写代码、调试、测试、多步规划、长上下文），轻档成员服务快问题。
//!
//! - **默认不启用**：只有用户专门建「按意图」分组并用 `group/<slug>` 调用时才会多这一次判定调用。
//! - **判不出就走强档**：判定模型超时、报错、回答看不懂时宁可多花钱，不拿便宜模型去答难题；
//!   原因写进日志与选路说明，不静默。
//! - **工具循环中途不重判**：最后一条是工具结果说明这还是同一个回合，沿用该会话上次的判定，
//!   不为每一步工具调用多付一次判定。网关重启后没有记忆的继续回合按强档走。
//! - 判定调用走网关自己的 Chat 入口（同一套选路、协议转换与流水记录），所以它的用量与费用
//!   照常出现在路由流水里；它不计入发起请求的命名客户端 Key 的预算。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use serde_json::{json, Value};

use crate::gateway::routing_groups::IntentConfig;
use crate::gateway::wire::CallerProtocol;
use crate::proxy::ProxyDeps;

pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 4_000;
pub(crate) const MIN_TIMEOUT_MS: u64 = 500;
pub(crate) const MAX_TIMEOUT_MS: u64 = 15_000;
/// 交给判定模型的文字上限（字符）：够判断意图，不把整段长上下文再付一遍钱。
const MAX_PROMPT_CHARS: usize = 4_000;
/// 会话判定的记忆时长与条数上限（只服务于工具循环中途的沿用）。
const REMEMBER_MS: i64 = 6 * 60 * 60 * 1000;
const REMEMBER_LIMIT: usize = 512;
/// 判定回复读取上限：判定只要一个词，回复异常地大时不往内存里读。
const REPLY_LIMIT: usize = 256 * 1024;

const PROMPT: &str = "You are a request router. Decide which model tier should answer the user's latest message.\n\
Reply with exactly one word:\n\
STRONG - writing or changing code, debugging, running or fixing tests, refactoring, multi-step planning, \
analysing long documents or logs, careful reasoning or math.\n\
LIGHT - quick factual questions, small talk, greetings, short explanations, translation, formatting, \
renaming, tiny edits, yes/no confirmations.\n\
When unsure, reply STRONG.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    Strong,
    Light,
}

impl Tier {
    fn text(self) -> &'static str {
        match self {
            Tier::Strong => "强档",
            Tier::Light => "轻档",
        }
    }
}

/// 调用方最后一条输入的性质。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    /// 新的人工输入（带文字）。
    Fresh(String),
    /// 工具结果：同一回合的继续。
    Continuing,
    /// 看不出来（空请求、助手预填等）。
    Unknown,
}

/// 判定结果：档位 + 给日志与选路说明用的一句话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub tier: Tier,
    pub note: String,
}

static REMEMBERED: LazyLock<Mutex<HashMap<String, (Tier, i64)>>> = LazyLock::new(Default::default);

fn remember(key: &str, tier: Tier, now_ms: i64) {
    if let Ok(mut map) = REMEMBERED.lock() {
        map.retain(|_, (_, at)| now_ms - *at < REMEMBER_MS);
        if map.len() >= REMEMBER_LIMIT {
            // 满了丢最旧的一条：记忆只是省钱，丢了最多多判一次
            if let Some(oldest) = map.iter().min_by_key(|(_, (_, at))| *at).map(|(k, _)| k.clone()) {
                map.remove(&oldest);
            }
        }
        map.insert(key.to_string(), (tier, now_ms));
    }
}

fn recall(key: &str, now_ms: i64) -> Option<Tier> {
    REMEMBERED
        .lock()
        .ok()?
        .get(key)
        .filter(|(_, at)| now_ms - *at < REMEMBER_MS)
        .map(|(tier, _)| *tier)
}

/// 把一段内容（字符串或内容块数组）里的文字拼起来；`kinds` 是算作文字的块类型。
fn text_of(content: &Value, kinds: &[&str]) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str).is_some_and(|kind| kinds.contains(&kind)))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn fresh(text: String) -> Turn {
    if text.trim().is_empty() {
        Turn::Unknown
    } else {
        Turn::Fresh(text)
    }
}

/// 读出调用方最后一条输入（三种协议）。
pub(crate) fn latest_turn(protocol: CallerProtocol, body: &Value) -> Turn {
    match protocol {
        CallerProtocol::Anthropic => {
            let Some(last) = body.get("messages").and_then(Value::as_array).and_then(|m| m.last()) else {
                return Turn::Unknown;
            };
            if last.get("role").and_then(Value::as_str) != Some("user") {
                return Turn::Unknown;
            }
            let content = last.get("content").unwrap_or(&Value::Null);
            let has_tool_result = content
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")));
            if has_tool_result {
                return Turn::Continuing;
            }
            fresh(text_of(content, &["text"]))
        }
        CallerProtocol::Chat => {
            let Some(last) = body.get("messages").and_then(Value::as_array).and_then(|m| m.last()) else {
                return Turn::Unknown;
            };
            match last.get("role").and_then(Value::as_str) {
                Some("tool") | Some("function") => Turn::Continuing,
                Some("user") => fresh(text_of(last.get("content").unwrap_or(&Value::Null), &["text"])),
                _ => Turn::Unknown,
            }
        }
        CallerProtocol::Responses => match body.get("input") {
            Some(Value::String(text)) => fresh(text.clone()),
            Some(Value::Array(items)) => {
                let Some(last) = items.last() else { return Turn::Unknown };
                let kind = last.get("type").and_then(Value::as_str).unwrap_or("message");
                if kind.ends_with("_output") {
                    return Turn::Continuing;
                }
                if kind == "message" && last.get("role").and_then(Value::as_str) == Some("user") {
                    return fresh(text_of(last.get("content").unwrap_or(&Value::Null), &["input_text", "text"]));
                }
                Turn::Unknown
            }
            _ => Turn::Unknown,
        },
    }
}

/// 截到上限：保留开头与结尾（意图通常在头尾），中间用省略号接上。
fn clip(text: &str) -> String {
    let count = text.chars().count();
    if count <= MAX_PROMPT_CHARS {
        return text.to_string();
    }
    let half = MAX_PROMPT_CHARS / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(count - half).collect();
    format!("{head}\n…\n{tail}")
}

/// 判定请求体（Chat 协议，非流式，输出很短）。
pub(crate) fn classifier_body(classifier: &str, text: &str) -> Value {
    json!({
        "model": classifier,
        "stream": false,
        "max_tokens": 16,
        "temperature": 0,
        "messages": [
            { "role": "system", "content": PROMPT },
            { "role": "user", "content": clip(text) }
        ]
    })
}

/// 读判定回复：去掉思考段后找第一个 STRONG / LIGHT（不分大小写）；都没有返回 None。
pub(crate) fn parse_reply(reply: &Value) -> Option<Tier> {
    let content = reply.pointer("/choices/0/message/content")?;
    let text = text_of(content, &["text"]);
    let text = match text.rfind("</think>") {
        Some(end) => &text[end + "</think>".len()..],
        None => text.as_str(),
    };
    let upper = text.to_ascii_uppercase();
    match (upper.find("STRONG"), upper.find("LIGHT")) {
        (Some(strong), Some(light)) => Some(if light < strong { Tier::Light } else { Tier::Strong }),
        (Some(_), None) => Some(Tier::Strong),
        (None, Some(_)) => Some(Tier::Light),
        (None, None) => None,
    }
}

/// 经网关自己的 Chat 入口调用判定模型。显式装箱：判定在 `wire::handle` 里发起、又调回
/// `wire::handle`，必须把 future 类型擦掉才能递归。
fn call(deps: Arc<ProxyDeps>, body: axum::body::Bytes) -> Pin<Box<dyn Future<Output = Response> + Send>> {
    let mut headers = HeaderMap::new();
    headers.insert("user-agent", HeaderValue::from_static("jolu-intent-router"));
    Box::pin(crate::gateway::wire::handle(CallerProtocol::Chat, deps, headers, body, "/v1/chat/completions", None))
}

async fn classify(deps: Arc<ProxyDeps>, config: &IntentConfig, text: &str) -> Result<Tier, String> {
    let body = serde_json::to_vec(&classifier_body(config.classifier.trim(), text)).map_err(|error| error.to_string())?;
    let timeout = Duration::from_millis(config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS));
    let attempt = async {
        let response = call(deps, body.into()).await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), REPLY_LIMIT)
            .await
            .map_err(|_| "判定回复读取失败".to_string())?;
        if !status.is_success() {
            return Err(format!("判定模型返回 {}", status.as_u16()));
        }
        let reply: Value = serde_json::from_slice(&bytes).map_err(|_| "判定回复不是 JSON".to_string())?;
        parse_reply(&reply).ok_or_else(|| "判定回复里没有 STRONG / LIGHT".to_string())
    };
    match tokio::time::timeout(timeout, attempt).await {
        Ok(result) => result,
        Err(_) => Err(format!("判定超过 {} 毫秒", timeout.as_millis())),
    }
}

/// 选档：新的人工输入 → 调判定模型；工具结果 → 沿用该会话上次判定；其余与失败 → 强档。
/// `config` 缺失（库里的意图配置坏了）时直接按强档并说明。
pub(crate) async fn decide(
    deps: Arc<ProxyDeps>,
    group: &str,
    config: Option<&IntentConfig>,
    protocol: CallerProtocol,
    body: &[u8],
    session: &str,
) -> Verdict {
    let strong = |note: String| Verdict { tier: Tier::Strong, note };
    let Some(config) = config else {
        return strong("分组缺少意图判定配置，按强档".into());
    };
    let key = format!("{group}\0{session}");
    let now = chrono::Utc::now().timestamp_millis();
    let parsed = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    match latest_turn(protocol, &parsed) {
        Turn::Fresh(text) => match classify(deps, config, &text).await {
            Ok(tier) => {
                remember(&key, tier, now);
                Verdict { tier, note: format!("意图判定：{}（{}）", tier.text(), config.classifier.trim()) }
            }
            Err(reason) => {
                // 记成强档：同一回合后续的工具步骤沿用强档，不再反复撞失败的判定模型
                remember(&key, Tier::Strong, now);
                strong(format!("意图判定失败（{reason}），按强档"))
            }
        },
        Turn::Continuing | Turn::Unknown => match recall(&key, now) {
            Some(tier) => Verdict { tier, note: format!("沿用本会话上次的意图判定：{}", tier.text()) },
            None => strong("没有新的输入可判定，也没有本会话的上次判定，按强档".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_turn_reads_each_protocol() {
        let anthropic = json!({"messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "t", "name": "x", "input": {}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t", "content": "ok"}]}
        ]});
        assert_eq!(latest_turn(CallerProtocol::Anthropic, &anthropic), Turn::Continuing);
        let anthropic = json!({"messages": [{"role": "user", "content": [{"type": "text", "text": "修一下这个测试"}, {"type": "image", "source": {}}]}]});
        assert_eq!(latest_turn(CallerProtocol::Anthropic, &anthropic), Turn::Fresh("修一下这个测试".into()));
        let prefill = json!({"messages": [{"role": "user", "content": "x"}, {"role": "assistant", "content": "{"}]});
        assert_eq!(latest_turn(CallerProtocol::Anthropic, &prefill), Turn::Unknown);

        let chat = json!({"messages": [{"role": "system", "content": "s"}, {"role": "user", "content": [{"type": "text", "text": "几点了"}]}]});
        assert_eq!(latest_turn(CallerProtocol::Chat, &chat), Turn::Fresh("几点了".into()));
        let chat = json!({"messages": [{"role": "user", "content": "q"}, {"role": "tool", "tool_call_id": "c", "content": "r"}]});
        assert_eq!(latest_turn(CallerProtocol::Chat, &chat), Turn::Continuing);

        assert_eq!(latest_turn(CallerProtocol::Responses, &json!({"input": "写个脚本"})), Turn::Fresh("写个脚本".into()));
        let responses = json!({"input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "a"}]},
            {"type": "function_call", "call_id": "c", "name": "f", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c", "output": "r"}
        ]});
        assert_eq!(latest_turn(CallerProtocol::Responses, &responses), Turn::Continuing);
        let responses = json!({"input": [{"role": "user", "content": [{"type": "input_text", "text": "解释一下"}]}]});
        assert_eq!(latest_turn(CallerProtocol::Responses, &responses), Turn::Fresh("解释一下".into()));
        assert_eq!(latest_turn(CallerProtocol::Chat, &json!({"messages": [{"role": "user", "content": "  "}]})), Turn::Unknown);
    }

    #[test]
    fn reply_parsing_takes_the_first_word_after_thinking() {
        let reply = |text: &str| json!({"choices": [{"message": {"role": "assistant", "content": text}}]});
        assert_eq!(parse_reply(&reply("LIGHT")), Some(Tier::Light));
        assert_eq!(parse_reply(&reply("strong.")), Some(Tier::Strong));
        assert_eq!(parse_reply(&reply("<think>maybe STRONG?</think>\nLight")), Some(Tier::Light));
        assert_eq!(parse_reply(&reply("LIGHT, not STRONG")), Some(Tier::Light));
        assert_eq!(parse_reply(&reply("我不知道")), None);
        assert_eq!(parse_reply(&json!({"error": {}})), None);
    }

    #[test]
    fn long_input_is_clipped_head_and_tail() {
        let text = format!("开头{}结尾", "中".repeat(10_000));
        let body = classifier_body("deepseek/deepseek-chat", &text);
        let sent = body.pointer("/messages/1/content").and_then(Value::as_str).unwrap();
        assert!(sent.starts_with("开头") && sent.ends_with("结尾"), "{}", &sent[..20]);
        assert!(sent.chars().count() <= MAX_PROMPT_CHARS + 3);
        assert_eq!(body["stream"], false);
        assert_eq!(body["model"], "deepseek/deepseek-chat");
    }

    /// 端到端：模拟上游同时充当判定模型、强档与轻档。快问题走轻档、难题走强档；
    /// 同一会话的工具结果回合不再判定、沿用轻档；判定模型不存在时按强档并照常答复。
    #[tokio::test]
    async fn intent_group_routes_each_turn_to_its_tier() {
        use crate::gateway::routing_groups::{create, GroupMember};
        use std::sync::atomic::{AtomicU32, Ordering};
        let judged = Arc::new(AtomicU32::new(0));
        let counter = judged.clone();
        let app = axum::Router::new().fallback(move |body: axum::body::Bytes| {
            let counter = counter.clone();
            async move {
                let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let model = request["model"].as_str().unwrap_or("").to_string();
                let first = request.pointer("/messages/0/content").and_then(Value::as_str).unwrap_or("");
                let answer = if first.contains("request router") {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let question = request.pointer("/messages/1/content").and_then(Value::as_str).unwrap_or("");
                    if question.contains("几点") { "LIGHT" } else { "STRONG" }.to_string()
                } else {
                    format!("answered-by:{model}")
                };
                ([("content-type", "application/json")], json!({
                    "id": "c", "object": "chat.completion", "model": model,
                    "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": answer}}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                }).to_string())
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let deps = crate::proxy::test_support::bare_deps();
        {
            let conn = deps.db.lock().unwrap();
            for slug in ["judge", "big", "small"] {
                conn.execute(
                    "INSERT INTO connections(id,platform,kind,name,secret,base_url,protocol,status,serve_enabled,created_ms,slug) VALUES(?1,'custom','api',?1,?2,?3,'openai','connected',1,1,?1)",
                    rusqlite::params![slug, format!("token-{slug}"), base],
                )
                .unwrap();
            }
            let member = |slug: &str, model: &str, tier: Option<&str>| GroupMember {
                slug: slug.into(),
                model: model.into(),
                paused_until_ms: None,
                weight: None,
                tier: tier.map(str::to_string),
            };
            let members = [member("big", "big-model", None), member("small", "small-model", Some("light"))];
            let config = IntentConfig { classifier: "judge/judge-model".into(), timeout_ms: None };
            create(&conn, "auto", "自动", "intent", &members, Some(&config)).unwrap();
            let missing = IntentConfig { classifier: "nowhere/judge".into(), timeout_ms: Some(1_000) };
            create(&conn, "auto-broken", "判定失效", "intent", &members, Some(&missing)).unwrap();
        }
        let ask = |group: &str, session: &str, messages: Value| {
            let deps = deps.clone();
            let body = json!({"model": format!("group/{group}"), "messages": messages}).to_string();
            let mut headers = HeaderMap::new();
            headers.insert("x-jolu-session", HeaderValue::from_str(session).unwrap());
            async move {
                let response = crate::gateway::wire::handle(CallerProtocol::Chat, deps, headers, body.into(), "/v1/chat/completions", None).await;
                assert!(response.status().is_success(), "{}", response.status());
                let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
                let reply: Value = serde_json::from_slice(&bytes).unwrap();
                reply.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or("").to_string()
            }
        };

        let quick = json!([{"role": "user", "content": "现在几点"}]);
        assert_eq!(ask("auto", "s-quick", quick).await, "answered-by:small-model");
        assert_eq!(judged.load(Ordering::SeqCst), 1);
        let tool_step = json!([
            {"role": "user", "content": "现在几点"},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "t1", "type": "function", "function": {"name": "clock", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "t1", "content": "10:00"}
        ]);
        assert_eq!(ask("auto", "s-quick", tool_step).await, "answered-by:small-model", "工具结果回合沿用轻档");
        assert_eq!(judged.load(Ordering::SeqCst), 1, "工具结果回合不再判定");

        let hard = json!([{"role": "user", "content": "重构这个模块并补上单元测试"}]);
        assert_eq!(ask("auto", "s-hard", hard).await, "answered-by:big-model");
        assert_eq!(judged.load(Ordering::SeqCst), 2);

        let quick = json!([{"role": "user", "content": "现在几点"}]);
        assert_eq!(ask("auto-broken", "s-broken", quick).await, "answered-by:big-model", "判定失败按强档");
        server.abort();
    }

    #[test]
    fn remembered_verdicts_expire() {
        remember("g\0s-expire", Tier::Light, 1_000);
        assert_eq!(recall("g\0s-expire", 2_000), Some(Tier::Light));
        assert_eq!(recall("g\0s-expire", 1_000 + REMEMBER_MS), None);
    }
}
