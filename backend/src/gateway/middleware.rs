//! 网关中间件链（P6.1，对齐 magpie 的 gateway middleware 语义）：
//! 按配置顺序对请求体做改写，失败隔离——单个中间件出错只跳过自己，请求原样继续。
//!
//! 首发两个内置：
//! - `model-map`：模型改名映射（New API 同款语义）——`{"mapping": {"fast": "来源/模型"}}`，
//!   精确命中（忽略大小写）即改写请求体 model；这是「任何模型名映射」的正解。
//! - `param-override`：顶层参数覆盖——`{"set": {字段: 值}, "delete": [字段]}`。
//!
//! 与路由的先后：中间件在**选路之前**执行（agent 发的原始名字先被映射，再进入
//! P1.1 的解析链），映射后的 id 就是路由、流水与用量记账的 id（magpie 同款：
//! 「the one id its routing, stats and usage rows know it by」）。
//!
//! word-guard / system-prompt / think-tags 为后续内置（本模块留扩展位）：
//! 它们触碰消息内容，与 P5 隐私脱敏的执行顺序要先定义（脱敏最先）。

use rusqlite::Connection;
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Middleware {
    pub name: String,
    pub kind: String,
    pub config: Value,
    pub enabled: bool,
    pub sort: i64,
}

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Middleware>> {
    let mut stmt = conn.prepare(
        "SELECT name, kind, config_json, enabled, sort FROM gateway_middleware ORDER BY sort ASC, name ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (name, kind, config_json, enabled, sort) = row?;
        match serde_json::from_str::<Value>(&config_json) {
            Ok(config) => out.push(Middleware { name, kind, config, enabled: enabled != 0, sort }),
            Err(error) => eprintln!("[中间件] 「{name}」的配置无效，已跳过：{error}"),
        }
    }
    Ok(out)
}

/// 新增 / 覆盖一个中间件（按 name 幂等）。
pub(crate) fn set(conn: &Connection, middleware: &Middleware) -> Result<(), String> {
    conn.execute(
        "INSERT INTO gateway_middleware(name, kind, config_json, enabled, sort) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(name) DO UPDATE SET kind = excluded.kind, config_json = excluded.config_json, enabled = excluded.enabled, sort = excluded.sort",
        rusqlite::params![middleware.name, middleware.kind, middleware.config.to_string(), middleware.enabled as i64, middleware.sort],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn delete(conn: &Connection, name: &str) -> Result<bool, String> {
    let changed = conn
        .execute("DELETE FROM gateway_middleware WHERE name = ?1", [name])
        .map_err(|error| error.to_string())?;
    Ok(changed > 0)
}

/// 中间件链的执行结果：映射前后的模型名（流水与用量记账用）与跳过原因。
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    /// model-map 命中时的「原始名 → 映射名」；未命中为 None
    pub mapped_from: Option<String>,
    /// 被隔离跳过的中间件（配置无效 / 执行出错），trace 用
    pub skipped: Vec<String>,
    /// word-guard 命中阻断词时的拒绝原因；`Some` 表示请求应当被拒发
    pub blocked: Option<String>,
    /// system-prompt 是否真的追加了内容（trace 用）
    pub injected_prompt: bool,
}

/// 按顺序执行启用的中间件，原地改写请求体。
///
/// 纪律（magpie 同款）：单个中间件失败**只跳过自己**，链上其余照常；model-map 只改
/// `model` 字段，不碰工具声明与 stream（客户端按它读回复）。
pub(crate) fn apply_chain(middlewares: &[Middleware], body: &mut Value) -> Outcome {
    let mut outcome = Outcome::default();
    for middleware in middlewares {
        if !middleware.enabled {
            continue;
        }
        let result = match middleware.kind.as_str() {
            "model-map" => apply_model_map(&middleware.config, body, &mut outcome),
            "param-override" => apply_param_override(&middleware.config, body),
            "system-prompt" => apply_system_prompt(&middleware.config, body),
            "word-guard" => apply_word_guard(&middleware.config, body, &mut outcome),
            "think-tags" => Ok(()), // 响应侧中间件：请求阶段无事可做（见 restore_chain）
            "js-hook" => Ok(()),    // JS 钩子在 server.rs 里另走异步宿主（js_hooks.rs）
            _ => Err(format!("未知中间件类型「{}」", middleware.kind)),
        };
        match result {
            Ok(()) => {}
            Err(error) => outcome.skipped.push(format!("{}（{error}）", middleware.name)),
        }
        // word-guard 的 block 判定：命中即拒发，后续中间件不再执行
        if outcome.blocked.is_some() {
            break;
        }
    }
    outcome
}

/// model-map：`{"mapping": {"请求名": "真实名"}}`，忽略大小写精确命中。
fn apply_model_map(config: &Value, body: &mut Value, outcome: &mut Outcome) -> Result<(), String> {
    let Some(mapping) = config.get("mapping").and_then(Value::as_object) else {
        return Err("model-map 配置缺少 mapping 表".into());
    };
    let Some(current) = body.get("model").and_then(Value::as_str) else {
        return Ok(()); // 请求没有 model 字段：本中间件无事可做，不算失败
    };
    let Some(target) = mapping
        .iter()
        .find(|(from, _)| from.as_str().eq_ignore_ascii_case(current))
        .and_then(|(_, to)| to.as_str())
    else {
        return Ok(()); // 映射表没这个名字：原样放行（裸名解析与默认来源接着处理）
    };
    if target.trim().is_empty() {
        return Err("model-map 的目标模型名为空".into());
    }
    outcome.mapped_from = Some(current.to_string());
    body["model"] = Value::String(target.trim().to_string());
    Ok(())
}

/// param-override：`{"set": {字段: 值}, "delete": [字段]}`，只动顶层字段。
fn apply_param_override(config: &Value, body: &mut Value) -> Result<(), String> {
    let object = body.as_object_mut().ok_or("请求体顶层不是对象")?;
    // set / delete 键存在但类型不对 = 配置错误（静默当 no-op 会掩盖写错的配置）
    if let Some(set) = config.get("set") {
        let Some(set) = set.as_object() else { return Err("param-override 的 set 不是对象".into()) };
        for (field, value) in set {
            object.insert(field.clone(), value.clone());
        }
    }
    if let Some(delete) = config.get("delete") {
        let Some(delete) = delete.as_array() else { return Err("param-override 的 delete 不是数组".into()) };
        for field in delete {
            if let Some(name) = field.as_str() {
                object.remove(name);
            }
        }
    }
    Ok(())
}

/// system-prompt：`{"prompt": "...", "agents": ["claude-code"], "models": []}`。
///
/// 命中条件：`agents` / `models` 为空表示不限，否则请求的 agent / model 要在其中。
/// 形态兼容：Anthropic 的 `system` 是字符串或块数组；OpenAI 的 system 是 messages 里的
/// `role: "system"`。两种都追加，且**不替换**原有系统提示（网关不该抹掉 agent 自己的指令）。
fn apply_system_prompt(config: &Value, body: &mut Value) -> Result<(), String> {
    let prompt = config
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or("system-prompt 的 prompt 为空")?;
    // Anthropic 形态：顶层 system
    match body.get_mut("system") {
        Some(Value::String(text)) => {
            *text = format!("{text}

{prompt}");
            return Ok(());
        }
        Some(Value::Array(blocks)) => {
            blocks.push(serde_json::json!({"type": "text", "text": prompt}));
            return Ok(());
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err("system 字段类型不支持（既不是字符串也不是数组）".into()),
    }
    // OpenAI 形态：messages 里的 system 消息
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        if let Some(system) = messages.iter_mut().find(|m| m["role"] == "system") {
            match system.get_mut("content") {
                Some(Value::String(text)) => {
                    *text = format!("{text}

{prompt}");
                    return Ok(());
                }
                Some(Value::Array(blocks)) => {
                    blocks.push(serde_json::json!({"type": "text", "text": prompt}));
                    return Ok(());
                }
                _ => {}
            }
        }
        // 没有任何 system 消息：插一条在最前
        messages.insert(0, serde_json::json!({"role": "system", "content": prompt}));
    }
    Ok(())
}

/// word-guard：`{"block": ["词"], "mask": ["词"]}`。
///
/// - `block` 命中的词出现时**整条请求拒发**（`Outcome.blocked`）；
/// - `mask` 命中的词在请求体里替换为 `***`（只处理字符串值，等价于 P5 脱敏的粗粒度版）。
/// 两者都只作用于请求侧；响应侧不处理（magpie 的 word-guard 会处理回复，Jolu 暂不跟进——
/// 响应改写会破坏流式增量语义，见模块注释）。
fn apply_word_guard(config: &Value, body: &mut Value, outcome: &mut Outcome) -> Result<(), String> {
    let block: Vec<&str> = config
        .get("block")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).filter(|w| !w.is_empty()).collect())
        .unwrap_or_default();
    let mask: Vec<&str> = config
        .get("mask")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).filter(|w| !w.is_empty()).collect())
        .unwrap_or_default();
    if block.is_empty() && mask.is_empty() {
        return Err("word-guard 的 block / mask 都为空".into());
    }
    // 先扫 block：命中就拒发，不再做 mask
    if !block.is_empty() {
        let hit = scan_strings(body, &block);
        if let Some(word) = hit {
            outcome.blocked = Some(format!("请求包含被拦截的内容（「{word}」）"));
            return Ok(());
        }
    }
    if !mask.is_empty() {
        mask_strings(body, &mask);
    }
    Ok(())
}

/// 在请求体的所有字符串值里找第一个命中词（读到即返回，不做全量替换）。
fn scan_strings(value: &Value, words: &[&str]) -> Option<String> {
    match value {
        Value::String(text) => words.iter().find(|w| text.contains(**w)).map(|w| w.to_string()),
        Value::Array(items) => items.iter().find_map(|item| scan_strings(item, words)),
        Value::Object(map) => map.values().find_map(|item| scan_strings(item, words)),
        _ => None,
    }
}

/// 把请求体所有字符串值里的命中词替换为 `***`。
fn mask_strings(value: &mut Value, words: &[&str]) {
    match value {
        Value::String(text) => {
            for word in words {
                if text.contains(word) {
                    *text = text.replace(word, "***");
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| mask_strings(item, words)),
        Value::Object(map) => map.values_mut().for_each(|item| mask_strings(item, words)),
        _ => {}
    }
}

/// 响应侧中间件链（目前只有 think-tags）：
/// `{"mode": "strip"}` 剥掉 `<think>…</think>`；`{"mode": "wrap"}` 把独立的思考字段包进 `<think>` 放到正文前。
///
/// 只用于**非流式**响应体；流式回复走 [`EventChain`]（onEvent，按结构化的思考字段 / 块逐事件
/// 处理，不在增量文本里找标签，所以跨块的标签不会被拆坏）。
pub(crate) fn restore_chain(middlewares: &[Middleware], text: &str) -> String {
    let mut out = text.to_string();
    for middleware in middlewares {
        if !middleware.enabled || middleware.kind != "think-tags" {
            continue;
        }
        let mode = middleware.config.get("mode").and_then(Value::as_str).unwrap_or("strip");
        match mode {
            "strip" => out = strip_think_tags(&out),
            "wrap" => out = wrap_reasoning(&out),
            _ => {}
        }
    }
    out
}

/// wrap：把独立字段里的思考内容包进 `<think>…</think>`，放到正文之前——给只认
/// 正文、不认 `reasoning_content` / thinking 块的客户端看思考过程。
/// - OpenAI Chat：`choices[].message.reasoning_content` 并入 `content` 并移除原字段；
/// - Anthropic Messages：`thinking` 块改成同位置的文本块（签名随块丢弃，客户端不再回传）。
/// 不是 JSON 或没有思考内容时原样返回（保持字节不变）。
fn wrap_reasoning(text: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return text.to_string();
    };
    let mut changed = false;
    if let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) {
        for choice in choices {
            let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) else { continue };
            let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string) else {
                continue;
            };
            let content = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
            message.insert("content".into(), Value::String(format!("<think>{reasoning}</think>{content}")));
            message.remove("reasoning_content");
            changed = true;
        }
    }
    if let Some(blocks) = value.get_mut("content").and_then(Value::as_array_mut) {
        for block in blocks.iter_mut() {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                continue;
            }
            let thinking = block.get("thinking").and_then(Value::as_str).unwrap_or("").to_string();
            *block = serde_json::json!({"type": "text", "text": format!("<think>{thinking}</think>")});
            changed = true;
        }
    }
    if changed { value.to_string() } else { text.to_string() }
}

/// 剥掉 `<think>…</think>` 段（含标签），忽略大小写与空白差异。
fn strip_think_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find("<think>") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after_open = start + "<think>".len();
        match lower[after_open..].find("</think>") {
            Some(end) => rest = &rest[after_open + end + "</think>".len()..],
            // 没有闭合标签：把剩下全丢掉（未闭合的思考段不该进用户可见文本）
            None => break,
        }
    }
    out
}

/* ───────────────────── 流式事件钩子 onEvent（3.7） ───────────────────── */

/// 一条流式回复的事件链：对每个 SSE 事件（已解析的 JSON）调用启用中间件的 onEvent，
/// 一个事件可以变成零个（丢掉）或多个（拆开）。目前实现 onEvent 的内置只有 think-tags：
/// - strip：Chat 片段去掉 `reasoning_content` / `reasoning`；Anthropic 的 thinking /
///   redacted_thinking 块整块丢掉，后面块的 index 往前补齐（SDK 按 index 拼块）；
/// - wrap：Chat 的思考增量改成正文，首段前加 `<think>`、转入正文或结束时补 `</think>`；
///   Anthropic 的 thinking 块改成同位置的文本块，开头 `<think>`、结尾 `</think>`，签名丢弃。
pub(crate) struct EventChain {
    wrap: bool,
    /// Anthropic：被丢掉的块 index
    dropped: Vec<u64>,
    /// Anthropic wrap：改成文本块的思考块 index
    wrapping: Vec<u64>,
    /// Chat wrap：正在输出思考（已经发了 `<think>`）
    in_reasoning: bool,
}

impl EventChain {
    /// 没有需要逐事件处理的中间件时返回 None（调用方原样转发）。
    pub(crate) fn new(middlewares: &[Middleware]) -> Option<Self> {
        let think = middlewares.iter().find(|m| m.enabled && m.kind == "think-tags")?;
        let wrap = think.config.get("mode").and_then(Value::as_str) == Some("wrap");
        Some(Self { wrap, dropped: Vec::new(), wrapping: Vec::new(), in_reasoning: false })
    }

    pub(crate) fn on_event(&mut self, mut event: Value) -> Vec<Value> {
        if event.get("choices").is_some() {
            self.on_chat(&mut event);
            return vec![event];
        }
        let Some(kind) = event.get("type").and_then(Value::as_str).map(str::to_owned) else {
            return vec![event];
        };
        let index = event.get("index").and_then(Value::as_u64);
        match (kind.as_str(), index) {
            ("content_block_start", Some(index)) => {
                let block = event["content_block"]["type"].as_str().unwrap_or_default();
                if block == "redacted_thinking" || (block == "thinking" && !self.wrap) {
                    self.dropped.push(index);
                    return Vec::new();
                }
                if block == "thinking" {
                    self.wrapping.push(index);
                    event["content_block"] = serde_json::json!({"type": "text", "text": ""});
                    let at = self.shift(index);
                    event["index"] = at.into();
                    let open = serde_json::json!({"type": "content_block_delta", "index": at, "delta": {"type": "text_delta", "text": "<think>"}});
                    return vec![event, open];
                }
                event["index"] = self.shift(index).into();
                vec![event]
            }
            ("content_block_delta" | "content_block_stop", Some(index)) => {
                if self.dropped.contains(&index) {
                    return Vec::new();
                }
                let at = self.shift(index);
                event["index"] = at.into();
                if self.wrapping.contains(&index) {
                    if kind == "content_block_stop" {
                        let close = serde_json::json!({"type": "content_block_delta", "index": at, "delta": {"type": "text_delta", "text": "</think>"}});
                        return vec![close, event];
                    }
                    match event["delta"]["type"].as_str() {
                        Some("thinking_delta") => {
                            let text = event["delta"]["thinking"].clone();
                            event["delta"] = serde_json::json!({"type": "text_delta", "text": text});
                        }
                        // 签名只对思考块有意义，改成文本块后不再回传
                        _ => return Vec::new(),
                    }
                }
                vec![event]
            }
            _ => vec![event],
        }
    }

    /// 丢掉的块在它前面的个数（index 往前补齐）。
    fn shift(&self, index: u64) -> u64 {
        index - self.dropped.iter().filter(|d| **d < index).count() as u64
    }

    fn on_chat(&mut self, event: &mut Value) {
        let Some(choices) = event.get_mut("choices").and_then(Value::as_array_mut) else { return };
        for choice in choices {
            let finished = choice.get("finish_reason").is_some_and(|f| !f.is_null());
            let Some(delta) = choice.get_mut("delta").and_then(Value::as_object_mut) else { continue };
            let reasoning = ["reasoning_content", "reasoning"]
                .iter()
                .filter_map(|k| delta.remove(*k))
                .filter_map(|v| v.as_str().map(str::to_owned))
                .find(|r| !r.is_empty());
            delta.remove("reasoning_details");
            if !self.wrap {
                continue;
            }
            let content = delta.get("content").and_then(Value::as_str).unwrap_or_default().to_string();
            let mut text = String::new();
            if let Some(reasoning) = reasoning {
                if !self.in_reasoning {
                    text.push_str("<think>");
                    self.in_reasoning = true;
                }
                text.push_str(&reasoning);
            }
            if self.in_reasoning && (!content.is_empty() || finished) {
                text.push_str("</think>");
                self.in_reasoning = false;
            }
            text.push_str(&content);
            if !text.is_empty() {
                delta.insert("content".into(), Value::String(text));
            }
        }
    }
}

/// 用事件链改写一条 SSE 流：按空行切事件，`data:` 是 JSON 的交给链处理，其余（`[DONE]`、
/// 注释、非 JSON）原样转发。原来带 `event:` 行的，改写后的事件按其 `type` 写 `event:`。
pub(crate) fn transform_sse<S, E>(upstream: S, chain: EventChain) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>>
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Send + 'static,
{
    use futures_util::StreamExt;
    futures_util::stream::unfold(
        (Box::pin(upstream), chain, String::new(), false),
        |(mut upstream, mut chain, mut buffer, done)| async move {
            if done {
                return None;
            }
            loop {
                let (ended, chunk) = match upstream.next().await {
                    Some(Ok(bytes)) => (false, String::from_utf8_lossy(&bytes).replace("\r\n", "\n")),
                    _ => (true, String::new()),
                };
                buffer.push_str(&chunk);
                let mut out = String::new();
                while let Some(at) = buffer.find("\n\n") {
                    let block: String = buffer.drain(..at + 2).collect();
                    out.push_str(&rewrite_block(&mut chain, &block));
                }
                if ended {
                    if !buffer.trim().is_empty() {
                        let rest = std::mem::take(&mut buffer) + "\n\n";
                        out.push_str(&rewrite_block(&mut chain, &rest));
                    }
                    return Some((Ok(axum::body::Bytes::from(out)), (upstream, chain, buffer, true)));
                }
                if !out.is_empty() {
                    return Some((Ok(axum::body::Bytes::from(out)), (upstream, chain, buffer, false)));
                }
            }
        },
    )
}

fn rewrite_block(chain: &mut EventChain, block: &str) -> String {
    let mut named = false;
    let mut data = Vec::new();
    for line in block.lines() {
        if line.starts_with("event:") {
            named = true;
        } else if let Some(d) = line.strip_prefix("data:") {
            data.push(d.trim_start());
        }
    }
    let joined = data.join("\n");
    let Ok(value) = serde_json::from_str::<Value>(&joined) else {
        return block.to_string();
    };
    let mut out = String::new();
    for event in chain.on_event(value) {
        if named {
            out.push_str(&format!("event: {}\n", event.get("type").and_then(Value::as_str).unwrap_or("message")));
        }
        out.push_str(&format!("data: {event}\n\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_strip_drops_thinking_blocks_and_renumbers_the_rest() {
        let mut chain = EventChain::new(&[middleware("think-tags", serde_json::json!({"mode": "strip"}))]).unwrap();
        let start = chain.on_event(serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}));
        assert!(start.is_empty());
        assert!(chain.on_event(serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "嗯"}})).is_empty());
        let text = chain.on_event(serde_json::json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}));
        assert_eq!(text[0]["index"], 0, "后面的块往前补齐");
        let chat = chain.on_event(serde_json::json!({"choices": [{"delta": {"reasoning_content": "想", "content": "答"}}]}));
        assert_eq!(chat[0]["choices"][0]["delta"], serde_json::json!({"content": "答"}));
    }

    #[test]
    fn stream_wrap_turns_reasoning_into_think_tagged_text() {
        let mut chain = EventChain::new(&[middleware("think-tags", serde_json::json!({"mode": "wrap"}))]).unwrap();
        let a = chain.on_event(serde_json::json!({"choices": [{"delta": {"reasoning_content": "想"}}]}));
        let b = chain.on_event(serde_json::json!({"choices": [{"delta": {"reasoning_content": "好了"}}]}));
        let c = chain.on_event(serde_json::json!({"choices": [{"delta": {"content": "答"}}]}));
        assert_eq!(a[0]["choices"][0]["delta"]["content"], "<think>想");
        assert_eq!(b[0]["choices"][0]["delta"]["content"], "好了");
        assert_eq!(c[0]["choices"][0]["delta"]["content"], "</think>答");
        let mut anthropic = EventChain::new(&[middleware("think-tags", serde_json::json!({"mode": "wrap"}))]).unwrap();
        let start = anthropic.on_event(serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}));
        assert_eq!(start[0]["content_block"]["type"], "text");
        assert_eq!(start[1]["delta"]["text"], "<think>");
        assert!(anthropic.on_event(serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "s"}})).is_empty());
        let stop = anthropic.on_event(serde_json::json!({"type": "content_block_stop", "index": 0}));
        assert_eq!(stop[0]["delta"]["text"], "</think>");
        assert_eq!(stop[1]["type"], "content_block_stop");
    }

    #[tokio::test]
    async fn sse_streams_are_rewritten_event_by_event() {
        let chain = EventChain::new(&[middleware("think-tags", serde_json::json!({"mode": "strip"}))]).unwrap();
        let chunks: Vec<Result<axum::body::Bytes, std::convert::Infallible>> = vec![
            Ok("event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\nevent: content_block_start\nda".into()),
            Ok("ta: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\"}}\r\n\r\ndata: [DONE]\n\n".into()),
        ];
        use futures_util::StreamExt;
        let out: Vec<_> = transform_sse(futures_util::stream::iter(chunks), chain).collect().await;
        let text: String = out.into_iter().map(|b| String::from_utf8(b.unwrap().to_vec()).unwrap()).collect();
        assert!(!text.contains("thinking"), "{text}");
        assert!(text.contains("event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"},\"index\":0"), "{text}");
        assert!(text.ends_with("data: [DONE]\n\n"), "{text}");
    }
    use serde_json::json;

    fn middleware(kind: &str, config: Value) -> Middleware {
        Middleware { name: kind.into(), kind: kind.into(), config, enabled: true, sort: 0 }
    }

    #[test]
    fn model_map_rewrites_exact_case_insensitive_and_records_from() {
        let m = middleware("model-map", json!({"mapping": {"fast": "zcode/glm-5.3-flash"}}));
        let mut body = json!({"model": "Fast", "max_tokens": 8});
        let outcome = apply_chain(&[m.clone()], &mut body);
        assert_eq!(body["model"], "zcode/glm-5.3-flash");
        assert_eq!(outcome.mapped_from.as_deref(), Some("Fast"));
        // 未映射的名字原样放行
        let mut body = json!({"model": "other", "max_tokens": 8});
        let outcome = apply_chain(&[m], &mut body);
        assert_eq!(body["model"], "other");
        assert!(outcome.mapped_from.is_none());
    }

    #[test]
    fn model_map_rejects_empty_target_without_touching_body() {
        let m = middleware("model-map", json!({"mapping": {"fast": "  "}}));
        let mut body = json!({"model": "fast"});
        let outcome = apply_chain(&[m], &mut body);
        assert_eq!(body["model"], "fast", "失败的中间件不改写请求");
        assert!(outcome.skipped.len() == 1, "失败要进 skipped：{:?}", outcome.skipped);
    }

    #[test]
    fn param_override_sets_and_deletes_top_level_fields() {
        let m = middleware(
            "param-override",
            json!({"set": {"max_tokens": 512, "temperature": 0.2}, "delete": ["top_p"]}),
        );
        let mut body = json!({"model": "m", "top_p": 0.9});
        apply_chain(&[m], &mut body);
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["temperature"], 0.2);
        assert!(body.get("top_p").is_none());
    }

    #[test]
    fn chain_applies_in_order_and_failure_isolates() {
        // 坏中间件夹在中间：它被跳过，后面的照常执行
        let chain = [
            middleware("model-map", json!({"mapping": {"fast": "zcode/glm-5.3-flash"}})),
            middleware("param-override", json!({"set": 1})), // set 不是对象 → 失败
            middleware("param-override", json!({"set": {"max_tokens": 32}})),
        ];
        let mut body = json!({"model": "fast"});
        let outcome = apply_chain(&chain, &mut body);
        assert_eq!(body["model"], "zcode/glm-5.3-flash");
        assert_eq!(body["max_tokens"], 32, "失败之后的中间件照常执行");
        assert_eq!(outcome.skipped.len(), 1);
    }

    #[test]
    fn disabled_and_unknown_middlewares_are_skipped() {
        let mut disabled = middleware("model-map", json!({"mapping": {"fast": "x/y"}}));
        disabled.enabled = false;
        let unknown = middleware("what", json!({}));
        let mut body = json!({"model": "fast"});
        let outcome = apply_chain(&[disabled, unknown], &mut body);
        assert_eq!(body["model"], "fast", "禁用的不执行");
        assert_eq!(outcome.skipped.len(), 1, "未知类型进 skipped");
    }

    #[test]
    fn system_prompt_appends_to_anthropic_string_and_array() {
        // 字符串形态：追加，不替换
        let m = middleware("system-prompt", json!({"prompt": "始终用中文回答"}));
        let mut body = json!({"model": "m", "system": "原有指令"});
        apply_chain(&[m.clone()], &mut body);
        assert_eq!(body["system"], "原有指令

始终用中文回答", "追加而不是替换");

        // 块数组形态：push 一个新 text 块
        let mut body = json!({"model": "m", "system": [{"type": "text", "text": "原有"}]});
        apply_chain(&[m], &mut body);
        let blocks = body["system"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["text"], "始终用中文回答");
    }

    #[test]
    fn system_prompt_falls_back_to_openai_system_message() {
        // OpenAI 形态：没有顶层 system，messages 里也没有 system 时插入一条
        let m = middleware("system-prompt", json!({"prompt": "注入"}));
        let mut body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        apply_chain(&[m], &mut body);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "注入");
        assert_eq!(messages[1]["role"], "user", "原消息保留在后面");
    }

    #[test]
    fn system_prompt_rejects_empty_prompt() {
        let m = middleware("system-prompt", json!({"prompt": "   "}));
        let mut body = json!({"model": "m", "system": "原有"});
        let outcome = apply_chain(&[m], &mut body);
        assert_eq!(body["system"], "原有", "配置无效时不改请求");
        assert_eq!(outcome.skipped.len(), 1);
    }

    #[test]
    fn word_guard_blocks_on_hit_and_masks_otherwise() {
        // block 命中：拒发，且不继续执行后面的中间件
        let chain = [
            middleware("word-guard", json!({"block": ["机密项目"]})),
            middleware("param-override", json!({"set": {"max_tokens": 999}})),
        ];
        let mut body = json!({"model": "m", "messages": [{"role": "user", "content": "谈谈机密项目"}]});
        let outcome = apply_chain(&chain, &mut body);
        assert!(outcome.blocked.is_some(), "命中阻断词要拒发");
        assert_eq!(body["max_tokens"], json!(null), "拒发后不再执行后续中间件");

        // mask：替换为 ***
        let m = middleware("word-guard", json!({"mask": ["1234"]}));
        let mut body = json!({"model": "m", "messages": [{"role": "user", "content": "卡号 1234 结尾"}]});
        let outcome = apply_chain(&[m], &mut body);
        assert!(outcome.blocked.is_none());
        assert_eq!(body["messages"][0]["content"], "卡号 *** 结尾");
    }

    #[test]
    fn word_guard_rejects_empty_config() {
        let m = middleware("word-guard", json!({}));
        let mut body = json!({"model": "m"});
        let outcome = apply_chain(&[m], &mut body);
        assert_eq!(outcome.skipped.len(), 1, "空配置要报错而不是静默放行");
    }

    #[test]
    fn think_tags_strips_thinking_blocks() {
        assert_eq!(strip_think_tags("前<think>思考中</think>后"), "前后");
        assert_eq!(strip_think_tags("<THINK>大写</THINK>ok"), "ok");
        assert_eq!(strip_think_tags("没有标签"), "没有标签");
        // 未闭合：剩下全丢（未闭合的思考段不该进用户可见文本）
        assert_eq!(strip_think_tags("前<think>没闭合"), "前");
        // 多个段
        assert_eq!(strip_think_tags("a<think>1</think>b<think>2</think>c"), "abc");
    }

    #[test]
    fn wrap_mode_moves_reasoning_into_think_tags_for_both_protocols() {
        let chain = [middleware("think-tags", json!({"mode": "wrap"}))];
        // OpenAI Chat：reasoning_content 包进 <think> 放到正文前，原字段移除
        let openai = json!({"choices": [{"message": {"role": "assistant", "reasoning_content": "先想", "content": "答案"}}]}).to_string();
        let wrapped: Value = serde_json::from_str(&restore_chain(&chain, &openai)).unwrap();
        assert_eq!(wrapped["choices"][0]["message"]["content"], "<think>先想</think>答案");
        assert!(wrapped["choices"][0]["message"].get("reasoning_content").is_none());
        // Anthropic Messages：thinking 块改成文本块里的 <think>
        let anthropic = json!({"content": [{"type": "thinking", "thinking": "先想", "signature": "s"}, {"type": "text", "text": "答案"}]}).to_string();
        let wrapped: Value = serde_json::from_str(&restore_chain(&chain, &anthropic)).unwrap();
        assert_eq!(wrapped["content"], json!([{"type": "text", "text": "<think>先想</think>"}, {"type": "text", "text": "答案"}]));
        // 没有思考内容 / 不是 JSON：原样返回
        let plain = json!({"choices": [{"message": {"content": "答案"}}]}).to_string();
        assert_eq!(restore_chain(&chain, &plain), plain);
        assert_eq!(restore_chain(&chain, "not json"), "not json");
    }

    #[test]
    fn restore_chain_only_touches_think_tags() {
        let chain = [
            middleware("think-tags", json!({"mode": "strip"})),
            middleware("model-map", json!({"mapping": {}})),
        ];
        assert_eq!(restore_chain(&chain, "答<think>x</think>案"), "答案");
        // 禁用的不生效
        let mut disabled = middleware("think-tags", json!({"mode": "strip"}));
        disabled.enabled = false;
        assert_eq!(restore_chain(&[disabled], "答<think>x</think>案"), "答<think>x</think>案");
    }

    #[test]
    fn store_round_trip_keeps_config_and_order() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS gateway_middleware (
                name TEXT PRIMARY KEY, kind TEXT NOT NULL, config_json TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1, sort INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        let mut first = middleware("model-map", json!({"mapping": {}}));
        first.sort = 0;
        set(&conn, &first).unwrap();
        let mut second = middleware("param-override", json!({"set": {}}));
        second.sort = 1;
        set(&conn, &second).unwrap();
        let listed = list(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "model-map");
        assert!(listed[0].enabled);
        delete(&conn, "model-map").unwrap();
        assert!(list(&conn).unwrap().len() == 1);
    }
}
