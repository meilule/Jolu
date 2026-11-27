//! 跨协议双向转换器：统一封装 API 直连与 Chat-only 订阅源共享的 `Anthropic / Responses <-> Chat` 转换。

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::Value;

use super::{error_response, CallerProtocol};
use crate::gateway::route;

/// 直连来源的协议转换方式（3.3）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Conversion {
    None,
    ResponsesToChat,
    AnthropicToChat,
    /// Chat / Responses 调用方 → anthropic 协议的 API 来源。
    ChatToAnthropic,
    ResponsesToAnthropic,
}

/// 原始 Responses 请求里的自由格式工具名。
pub(crate) fn original_custom_tools(body: &Bytes) -> Vec<String> {
    serde_json::from_slice::<Value>(body)
        .map(|value| crate::gateway::responses_chat::custom_tool_names(&value))
        .unwrap_or_default()
}

pub(crate) fn wire_label(protocol: CallerProtocol) -> &'static str {
    match protocol {
        CallerProtocol::Anthropic => "Anthropic Messages",
        CallerProtocol::Chat => "OpenAI Chat",
        CallerProtocol::Responses => "OpenAI Responses",
    }
}

/// 这次转发经协议转换会省略哪些服务端工具；不经「转 Chat」的路径返回 None。
pub(crate) fn dropped_tools_note(
    protocol: CallerProtocol,
    entry: &route::PoolEntry,
    body: &Bytes,
) -> Option<String> {
    let converts_to_chat = match entry.kind.as_str() {
        "api" => match (protocol, entry.protocol.as_str()) {
            (CallerProtocol::Anthropic, "openai") | (CallerProtocol::Responses, "anthropic") => true,
            (CallerProtocol::Responses, "openai") => {
                !crate::gateway::responses_chat::serves_responses(entry.base_url.as_deref())
            }
            _ => false,
        },
        _ => matches!(entry.platform.as_str(), "gemini" | "workbuddy" | "qoder" | "trae")
            && protocol != CallerProtocol::Chat,
    };
    if !converts_to_chat {
        return None;
    }
    let value: Value = serde_json::from_slice(body).ok()?;
    let dropped = match protocol {
        CallerProtocol::Anthropic => crate::gateway::translate::anthropic_tools_dropped_for_chat(&value),
        CallerProtocol::Responses => crate::gateway::responses_chat::builtin_tools_dropped_for_chat(&value),
        CallerProtocol::Chat => return None,
    };
    (!dropped.is_empty()).then(|| format!("来源不支持，已省略服务端工具：{}", dropped.join("、")))
}

/// 将任意调用方协议请求体转换为 OpenAI Chat Completions 请求体（API 直连与 Chat-only 订阅共用）。
pub(crate) fn to_chat_body(protocol: CallerProtocol, body: &Bytes, prefix_err: bool) -> Result<Bytes, Response> {
    match protocol {
        CallerProtocol::Chat => Ok(body.clone()),
        CallerProtocol::Anthropic => crate::gateway::translate::anthropic_request(body).map_err(|reason| {
            let msg = if prefix_err {
                format!("Anthropic 请求转 Chat 失败：{reason}")
            } else {
                reason
            };
            error_response(StatusCode::BAD_REQUEST, &msg)
        }),
        CallerProtocol::Responses => serde_json::from_slice::<Value>(body)
            .map_err(|error| error.to_string())
            .and_then(|value| crate::gateway::responses_chat::request(&value))
            .map(|chat| Bytes::from(chat.to_string()))
            .map_err(|reason| {
                let msg = if prefix_err {
                    format!("Responses 请求转 Chat 失败：{reason}")
                } else {
                    reason
                };
                error_response(StatusCode::BAD_REQUEST, &msg)
            }),
    }
}

/// 将 OpenAI Chat Completions 响应还原为调用方协议响应（API 直连与 Chat-only 订阅共用）。
pub(crate) async fn from_chat_response(
    protocol: CallerProtocol,
    response: Response,
    requested_model: &str,
    original_body: &Bytes,
) -> Response {
    match protocol {
        CallerProtocol::Chat => response,
        CallerProtocol::Anthropic => crate::gateway::translate::anthropic_response(response, requested_model).await,
        CallerProtocol::Responses => {
            crate::gateway::responses_chat::response_body(response, original_custom_tools(original_body)).await
        }
    }
}

pub(crate) fn decide_api_conversion(
    protocol: CallerProtocol,
    entry: &route::PoolEntry,
) -> Result<Conversion, Response> {
    match (protocol, entry.protocol.as_str()) {
        (CallerProtocol::Anthropic, "anthropic") | (CallerProtocol::Chat, "openai") => Ok(Conversion::None),
        (CallerProtocol::Responses, "openai")
            if crate::gateway::responses_chat::serves_responses(entry.base_url.as_deref()) =>
        {
            Ok(Conversion::None)
        }
        (CallerProtocol::Responses, "openai") => Ok(Conversion::ResponsesToChat),
        (CallerProtocol::Anthropic, "openai") => Ok(Conversion::AnthropicToChat),
        (CallerProtocol::Chat, "anthropic") => Ok(Conversion::ChatToAnthropic),
        (CallerProtocol::Responses, "anthropic") => Ok(Conversion::ResponsesToAnthropic),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "连接「{}」只提供 {} 协议端点，不能服务 {} 请求；请为该连接选择 {} 兼容地址",
                entry.slug,
                entry.protocol,
                wire_label(protocol),
                wire_label(protocol),
            ),
        )),
    }
}

pub(crate) fn encode_api_request(
    conversion: Conversion,
    protocol: CallerProtocol,
    body: &Bytes,
    path_and_query: &str,
) -> Result<(Bytes, String), Response> {
    match conversion {
        Conversion::None => Ok((body.clone(), path_and_query.to_string())),
        Conversion::ResponsesToChat | Conversion::AnthropicToChat => {
            let chat = to_chat_body(protocol, body, true)?;
            Ok((chat, crate::gateway::responses_chat::chat_path(path_and_query)))
        }
        Conversion::ChatToAnthropic | Conversion::ResponsesToAnthropic => {
            let chat = match protocol {
                CallerProtocol::Responses => serde_json::from_slice::<Value>(body)
                    .map_err(|error| error.to_string())
                    .and_then(|value| crate::gateway::responses_chat::request(&value).map(|chat| chat.to_string())),
                _ => serde_json::from_slice::<Value>(body)
                    .map_err(|error| error.to_string())
                    .map(|value| value.to_string()),
            };
            let anthropic = chat.and_then(|text| crate::gateway::translate::chat_to_anthropic_request(text.as_bytes()));
            match anthropic {
                Ok(bytes) => Ok((bytes, crate::gateway::translate::messages_path().to_string())),
                Err(reason) => Err(error_response(StatusCode::BAD_REQUEST, &format!("请求转 Anthropic Messages 失败：{reason}"))),
            }
        }
    }
}

pub(crate) async fn decode_api_response(
    conversion: Conversion,
    protocol: CallerProtocol,
    response: Response,
    requested_model: &str,
    original_body: &Bytes,
) -> Response {
    match conversion {
        Conversion::None => response,
        Conversion::ResponsesToChat | Conversion::AnthropicToChat => {
            from_chat_response(protocol, response, requested_model, original_body).await
        }
        Conversion::ChatToAnthropic => crate::gateway::translate::chat_response(response, requested_model).await,
        Conversion::ResponsesToAnthropic => {
            let chat_form = crate::gateway::translate::chat_response(response, requested_model).await;
            crate::gateway::responses_chat::response_body(chat_form, original_custom_tools(original_body)).await
        }
    }
}
