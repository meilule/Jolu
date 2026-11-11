//! 路由分组成员的固定 effort：成员写成 `来源/模型:high` 时，转发给这个成员的请求按这个
//! 推理档位问上游，不管 agent 自己要的是多少（magpie #189）。
//!
//! 管什么：拆出成员模型名里的 effort（只认档位词：`qwen:7b`、`deepseek-r1:free` 这类模型名里的
//! 冒号不当 effort）；按调用方协议改请求体——Chat 的 `reasoning_effort`、Responses 的
//! `reasoning.effort`、Anthropic 的 `output_config.effort`（自适应思考）或思考预算（开启思考）。
//!
//! 不管什么：请求本来没要推理（如会话标题这种小请求）时不加——硬加会让便宜请求变贵；
//! magpie 的「多档 `low+high` 跟随 agent」暂不支持，只有单个固定档。
//!
//! 参照 magpie `internal/gateway/decide.go::withEffort` 与 `ir.go::budgetOf`（克隆 `0ee424e`）。

use serde_json::Value;

use crate::gateway::wire::CallerProtocol;

const LEVELS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// `模型:档位` → (模型, 档位)；冒号后不是档位词的整体当模型名。
pub(crate) fn split(model: &str) -> (String, Option<String>) {
    match model.rsplit_once(':') {
        Some((name, effort)) if !name.is_empty() && LEVELS.contains(&effort.trim().to_ascii_lowercase().as_str()) => {
            (name.to_string(), Some(effort.trim().to_ascii_lowercase()))
        }
        _ => (model.to_string(), None),
    }
}

/// 开启思考时各档的思考预算（magpie `budgetOf` 同值）。
fn budget_of(effort: &str) -> i64 {
    match effort {
        "low" | "minimal" => 4096,
        "medium" => 10_000,
        "high" => 24_000,
        "xhigh" | "max" => 32_000,
        _ => 10_000,
    }
}

/// 按协议把 effort 写进请求体；请求本来没要推理、或读不懂时原样返回。
pub(crate) fn apply(protocol: CallerProtocol, body: &axum::body::Bytes, effort: &str) -> axum::body::Bytes {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else { return body.clone() };
    let changed = match protocol {
        CallerProtocol::Chat => {
            let asked = value.get("reasoning_effort").and_then(Value::as_str).is_some_and(|e| e != "none");
            if asked {
                value["reasoning_effort"] = Value::String(effort.into());
            }
            asked
        }
        CallerProtocol::Responses => {
            let asked = value.pointer("/reasoning/effort").and_then(Value::as_str).is_some_and(|e| !e.is_empty() && e != "none");
            if asked {
                value["reasoning"]["effort"] = Value::String(effort.into());
            }
            asked
        }
        CallerProtocol::Anthropic => match value.pointer("/thinking/type").and_then(Value::as_str) {
            Some("adaptive") => {
                if !value.get("output_config").is_some_and(Value::is_object) {
                    value["output_config"] = serde_json::json!({});
                }
                value["output_config"]["effort"] = Value::String(effort.into());
                true
            }
            Some("enabled") => {
                // 预算要给回答留出空间
                let mut budget = budget_of(effort);
                if let Some(max) = value.get("max_tokens").and_then(Value::as_i64).filter(|m| *m > 0) {
                    budget = budget.min(max - 1);
                }
                if budget < 1024 {
                    false
                } else {
                    value["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": budget});
                    if value.pointer("/output_config/effort").is_some() {
                        value["output_config"]["effort"] = Value::String(effort.into());
                    }
                    true
                }
            }
            _ => false,
        },
    };
    if !changed {
        return body.clone();
    }
    serde_json::to_vec(&value).map(Into::into).unwrap_or_else(|_| body.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(protocol: CallerProtocol, body: Value, effort: &str) -> Value {
        serde_json::from_slice(&apply(protocol, &body.to_string().into(), effort)).unwrap()
    }

    #[test]
    fn only_level_words_after_the_last_colon_are_an_effort() {
        assert_eq!(split("glm-5.3:high"), ("glm-5.3".to_string(), Some("high".to_string())));
        assert_eq!(split("qwen:7b"), ("qwen:7b".to_string(), None));
        assert_eq!(split("deepseek/deepseek-r1:free"), ("deepseek/deepseek-r1:free".to_string(), None));
        assert_eq!(split("m:HIGH").1.as_deref(), Some("high"));
    }

    #[test]
    fn effort_is_written_in_each_protocols_own_words_only_when_reasoning_was_asked() {
        assert_eq!(run(CallerProtocol::Chat, json!({"reasoning_effort": "low"}), "high")["reasoning_effort"], "high");
        assert!(run(CallerProtocol::Chat, json!({"messages": []}), "high").get("reasoning_effort").is_none(), "没要推理不加");
        assert_eq!(run(CallerProtocol::Responses, json!({"reasoning": {"effort": "low", "summary": "auto"}}), "max")["reasoning"], json!({"effort": "max", "summary": "auto"}));
        assert_eq!(run(CallerProtocol::Anthropic, json!({"thinking": {"type": "adaptive"}}), "low")["output_config"]["effort"], "low");
        let enabled = run(CallerProtocol::Anthropic, json!({"thinking": {"type": "enabled", "budget_tokens": 2048}, "max_tokens": 20000}), "high");
        assert_eq!(enabled["thinking"]["budget_tokens"], 19_999, "预算给回答留空间");
        let tiny = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 800});
        assert_eq!(run(CallerProtocol::Anthropic, tiny.clone(), "high"), tiny, "留不出 1024 的预算就不改");
        assert!(run(CallerProtocol::Anthropic, json!({"messages": []}), "high").get("thinking").is_none());
    }
}
