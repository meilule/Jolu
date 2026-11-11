//! 把客户端发来的 URL 安全 / 无填充 base64 统一成标准 base64（带 `=` 填充）。
//!
//! 为什么：Google 的 Python SDK（google-genai）用 `urlsafe_b64encode` 写 `Part.from_bytes`
//! 的字节，`inlineData.data` 里带 `-` 和 `_`。Google 自己的 API 两种都认，但经代理的
//! Gemini 端点、以及多数 OpenAI / Anthropic 兼容上游只认标准字母表，回 400「Failed to
//! base64-decode image data」。同一请求换成 `+` `/` 就能过（magpie #5697 的现场）。
//!
//! 管什么：所有协议里的内联字节——Anthropic 的 `{"type":"base64","data":…}`（图片、文档、
//! 工具结果里的图片），Chat / Responses 的 `data:…;base64,…` 地址，Gemini 的
//! `inlineData` / `inline_data`（任何类型：图片、PDF、录音，含 systemInstruction 与函数回复）。
//!
//! 不管什么：已经是标准且带填充的原样不动（整个请求体逐字节不变）；同时含两套字母表的、
//! 不是 base64 的字符串也不动——宁可让上游报错，不猜。
//!
//! 参照 magpie `internal/gateway/base64_input.go::stdBase64Body`（`b657a21`）；差异：magpie
//! 原位拼接保留其余字节，Jolu 只在确实改了时重新序列化整个 JSON。

use serde_json::Value;

/// 请求体可能带内联字节时才解析（避免对每个请求做一次完整的 JSON 往返）。
fn might_carry_bytes(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    text.contains("base64") || text.contains("inlineData") || text.contains("inline_data")
}

/// 一段 base64 若是 URL 安全或无填充的，返回标准写法；已标准、混用两套字母表、或不是 base64 时返回 None。
pub(crate) fn standard(text: &str) -> Option<String> {
    let trimmed = text.trim_end_matches('=');
    if trimmed.is_empty() {
        return None;
    }
    let url_safe = trimmed.contains('-') || trimmed.contains('_');
    let standard_only = trimmed.contains('+') || trimmed.contains('/');
    if url_safe && standard_only {
        return None;
    }
    if !trimmed.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'/')) {
        return None;
    }
    // 长度余 1 的不可能是合法 base64
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let padded_len = trimmed.len().div_ceil(4) * 4;
    if !url_safe && text.len() == padded_len {
        return None;
    }
    let mut out: String = trimmed.chars().map(|c| match c { '-' => '+', '_' => '/', other => other }).collect();
    while out.len() < padded_len {
        out.push('=');
    }
    (out != text).then_some(out)
}

/// `data:<type>;base64,<payload>` 里的 payload 标准化。
fn standard_data_url(url: &str) -> Option<String> {
    if !url.starts_with("data:") {
        return None;
    }
    let (head, payload) = url.split_once(";base64,")?;
    standard(payload).map(|fixed| format!("{head};base64,{fixed}"))
}

/// 递归走一遍，按上面三种位置改写。返回是否改过。
fn walk(value: &mut Value, inline_parent: bool) -> bool {
    match value {
        Value::Object(object) => {
            let mut changed = false;
            let base64_source = object.get("type").and_then(Value::as_str) == Some("base64");
            for (key, child) in object.iter_mut() {
                let is_bytes = key == "data" && (base64_source || inline_parent);
                if is_bytes {
                    if let Some(fixed) = child.as_str().and_then(standard) {
                        *child = Value::String(fixed);
                        changed = true;
                    }
                    continue;
                }
                changed |= walk(child, key == "inlineData" || key == "inline_data");
            }
            changed
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| walk(item, false) | changed),
        Value::String(text) => match standard_data_url(text) {
            Some(fixed) => {
                *text = fixed;
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// 整个请求体：没改动时原样返回同一份字节。
pub(crate) fn standardize_body(body: axum::body::Bytes) -> axum::body::Bytes {
    if !might_carry_bytes(&body) {
        return body;
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else { return body };
    if !walk(&mut value, false) {
        return body;
    }
    serde_json::to_vec(&value).map(Into::into).unwrap_or(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_safe_and_unpadded_become_standard_padded() {
        assert_eq!(standard("ab-_").as_deref(), Some("ab+/"));
        assert_eq!(standard("YWJj-w").as_deref(), Some("YWJj+w=="), "补齐填充");
        assert_eq!(standard("YWJjZA").as_deref(), Some("YWJjZA=="), "只缺填充也改");
        assert_eq!(standard("YWJjZA=="), None, "已是标准的不动");
        assert_eq!(standard("ab+/"), None);
        assert_eq!(standard("a-b+"), None, "两套字母表混用不猜");
        assert_eq!(standard("不是 base64"), None);
        assert_eq!(standard("YWJjZ"), None, "长度余 1 不是 base64");
    }

    #[test]
    fn every_protocol_position_is_standardized_and_nothing_else() {
        let body = json!({
            "messages": [{"role": "user", "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBO-w"}},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBO_w"}},
                {"type": "text", "text": "data-like_text-should-stay"}
            ]}],
            "contents": [{"parts": [{"inlineData": {"mimeType": "application/pdf", "data": "JVBE-w"}}]}],
            "systemInstruction": {"parts": [{"inline_data": {"mime_type": "audio/wav", "data": "UklG_g"}}]},
            "metadata": {"data": "keep-me_please"}
        });
        let out: Value = serde_json::from_slice(&standardize_body(body.to_string().into())).unwrap();
        assert_eq!(out["messages"][0]["content"][0]["source"]["data"], "iVBO+w==");
        assert_eq!(out["messages"][0]["content"][1]["image_url"]["url"], "data:image/png;base64,iVBO/w==");
        assert_eq!(out["messages"][0]["content"][2]["text"], "data-like_text-should-stay", "普通文字不动");
        assert_eq!(out["contents"][0]["parts"][0]["inlineData"]["data"], "JVBE+w==");
        assert_eq!(out["systemInstruction"]["parts"][0]["inline_data"]["data"], "UklG/g==");
        assert_eq!(out["metadata"]["data"], "keep-me_please", "不是内联字节位置的 data 不动");
    }

    #[test]
    fn untouched_bodies_keep_their_exact_bytes() {
        let standard_body = axum::body::Bytes::from(r#"{"a" : 1, "source":{"type":"base64","data":"YWJjZA=="}}"#);
        assert_eq!(standardize_body(standard_body.clone()), standard_body, "已标准的请求体逐字节不变");
        let plain = axum::body::Bytes::from(r#"{"model":"m","messages":[]}"#);
        assert_eq!(standardize_body(plain.clone()), plain);
    }
}
