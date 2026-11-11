//! OTLP 用量导出（批次 G §3.12）。
//!
//! 把本机用量**按平台 + 模型聚合**成 OTLP 指标，POST 到用户配置的 OTLP HTTP 端点
//! （`<endpoint>/v1/metrics`）。设计取舍：
//!
//! - **只导聚合值**：没有会话 ID、没有请求正文、没有 Key 原文、没有连接标识。
//!   唯一例外是命名客户端 Key 的**名字**（用户起的设备标签），见 [`KeyPoint`]。
//!   逐请求导出会把「谁在什么时候用什么模型聊了什么」的时序轮廓暴露出去，
//!   而运维真正需要的是「某模型某天用了多少 token」。脱敏靠**不采集**完成，
//!   比事后过滤可靠（见 AGENTS.md：认证信息不得写入统计导出）。
//! - **默认关闭**：端点为空即不导。
//! - **失败如实回报**：HTTP 非 2xx 与网络错误都当失败，不静默吞掉。
//! - 走 `net::async_builder()`，与其它出站请求同一套联网方式（含回环排除）。

use serde::Serialize;
use std::time::Duration;

/// 一条聚合后的用量指标：某个平台某个模型在该时间窗口内的合计。
#[derive(Debug, Clone, Serialize)]
pub struct UsagePoint {
    pub platform: String,
    /// 模型名；来源没给时为 None，导出时标成 `unknown`
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub requests: i64,
}

/// 导出结果。成功与失败都带回可核对的计数。
#[derive(Debug, Clone, Serialize)]
pub struct OtlpExportResult {
    pub endpoint: String,
    pub points: usize,
    /// 实际发出的 HTTP 状态码（网络层失败时为 None）
    pub status: Option<u16>,
    /// 服务端返回的正文片段（最多 400 字符，便于对照 400 的原因）
    pub response: Option<String>,
}

/// OTLP 的 metric 名。用 OTel 语义约定里的 token 用量命名，便于对接现成看板。
const METRIC_TOKENS: &str = "gen_ai.client.token.usage";
const METRIC_REQUESTS: &str = "gen_ai.client.request.count";
/// 经统一网关、带命名客户端 Key 的请求数（局域网共享的远端设备）。
const METRIC_KEY_REQUESTS: &str = "jolu.gateway.key.request.count";

/// 一把命名客户端 Key 在窗口内某模型、某结果的请求数。
///
/// 为什么带 Key 名：局域网共享时多台设备各用一把 Key，排查「哪台设备一直失败」
/// 只能靠它（参照 magpie `4314f4b`，OTLP 属性 `magpie.gateway_key.name`）。Key 名是
/// 用户起的标签（如 `laptop`），不是 Key 本身；Key 原文从不进导出。
#[derive(Debug, Clone, Serialize)]
pub struct KeyPoint {
    pub name: String,
    pub model: Option<String>,
    /// `ok` / `failed`
    pub result: String,
    pub requests: i64,
}

/// 端点规范化：允许用户只填到根（`http://host:4318`），自动补 `/v1/metrics`；
/// 已经写了 `/v1/metrics` 的原样使用。只接受 http/https。
pub(crate) fn metrics_url(endpoint: &str) -> Result<String, String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() { return Err("请先填写 OTLP 端点".into()); }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("OTLP 端点必须以 http:// 或 https:// 开头".into());
    }
    if trimmed.ends_with("/v1/metrics") { Ok(trimmed.to_string()) }
    else { Ok(format!("{trimmed}/v1/metrics")) }
}

/// 组装 OTLP/HTTP JSON（`resourceMetrics` 结构，protocol 1.0）。
///
/// 时间单位为纳秒。每个平台+模型组合一条 Gauge 数据点，属性里只放
/// `gen_ai.system`（平台）与 `gen_ai.request.model`（模型）——没有账号、会话、Key。
pub(crate) fn build_payload(points: &[UsagePoint], keys: &[KeyPoint], now_ms: i64) -> serde_json::Value {
    let time_nano = (now_ms as i128 * 1_000_000).to_string();
    let attributes = |point: &UsagePoint| -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({ "key": "gen_ai.system", "value": { "stringValue": point.platform } }),
            serde_json::json!({
                "key": "gen_ai.request.model",
                "value": { "stringValue": point.model.clone().unwrap_or_else(|| "unknown".into()) }
            }),
        ]
    };
    let mut token_points = Vec::new();
    let mut request_points = Vec::new();
    for point in points {
        // 四类 token 各出一个数据点：input / output / cache_read / cache_write
        for (kind, value) in [
            ("input", point.input_tokens),
            ("output", point.output_tokens),
            ("cache_read", point.cache_read_tokens),
            ("cache_write", point.cache_write_tokens),
        ] {
            if value == 0 { continue; }
            let mut attrs = attributes(point);
            attrs.push(serde_json::json!({ "key": "gen_ai.token.type", "value": { "stringValue": kind } }));
            token_points.push(serde_json::json!({
                "attributes": attrs,
                "startTimeUnixNano": time_nano,
                "timeUnixNano": time_nano,
                "asInt": value.to_string(),
            }));
        }
        if point.requests > 0 {
            request_points.push(serde_json::json!({
                "attributes": attributes(point),
                "startTimeUnixNano": time_nano,
                "timeUnixNano": time_nano,
                "asInt": point.requests.to_string(),
            }));
        }
    }
    let string_attr = |key: &str, value: &str| serde_json::json!({ "key": key, "value": { "stringValue": value } });
    let key_points: Vec<serde_json::Value> = keys
        .iter()
        .filter(|key| key.requests > 0)
        .map(|key| {
            serde_json::json!({
                "attributes": [
                    string_attr("jolu.gateway_key.name", &key.name),
                    string_attr("gen_ai.request.model", key.model.as_deref().unwrap_or("unknown")),
                    string_attr("jolu.result", &key.result),
                ],
                "startTimeUnixNano": time_nano,
                "timeUnixNano": time_nano,
                "asInt": key.requests.to_string(),
            })
        })
        .collect();
    let gauge = |name: &str, unit: &str, data: Vec<serde_json::Value>| -> serde_json::Value {
        if data.is_empty() { return serde_json::Value::Null; }
        serde_json::json!({
            "name": name,
            "unit": unit,
            "gauge": { "dataPoints": data }
        })
    };
    let metrics: Vec<serde_json::Value> = vec![
        gauge(METRIC_TOKENS, "{token}", token_points),
        gauge(METRIC_REQUESTS, "{request}", request_points),
        gauge(METRIC_KEY_REQUESTS, "{request}", key_points),
    ]
    .into_iter()
    .filter(|value| !value.is_null())
    .collect();
    serde_json::json!({
        "resourceMetrics": [{
            "resource": {
                "attributes": [
                    { "key": "service.name", "value": { "stringValue": "jolu" } },
                    { "key": "service.version", "value": { "stringValue": env!("CARGO_PKG_VERSION") } }
                ]
            },
            "scopeMetrics": [{
                "scope": { "name": "jolu.usage", "version": env!("CARGO_PKG_VERSION") },
                "metrics": metrics
            }]
        }]
    })
}

/// 从连接里读聚合用量。
///
/// 只按 `platform` + `model` 分组，**不 select** `session_id`、`source`、`dedup_key`——
/// 这几个字段会暴露具体会话或连接，导出不需要。
pub(crate) fn aggregate(
    conn: &rusqlite::Connection, since_ms: i64, until_ms: i64,
) -> Result<Vec<UsagePoint>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT platform, model,
                    COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0), COALESCE(SUM(cache_write_tokens), 0),
                    COALESCE(SUM(total_tokens), 0), COUNT(*)
               FROM requests
              WHERE ts >= ?1 AND ts < ?2
              GROUP BY platform, model
              ORDER BY platform, model",
        )
        .map_err(|error| format!("准备用量查询失败：{error}"))?;
    let rows = stmt
        .query_map([since_ms, until_ms], |row| {
            Ok(UsagePoint {
                platform: row.get(0)?,
                model: row.get(1)?,
                input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                cache_read_tokens: row.get(4)?,
                cache_write_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                requests: row.get(7)?,
            })
        })
        .map_err(|error| format!("查询用量失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("读取用量失败：{error}"))
}

/// 从统一网关流水读按命名 Key 的请求数：只选 Key 名、请求模型、结果三列，
/// 没有 Key 名的（本机自己的请求）不进这项指标。
pub(crate) fn aggregate_keys(
    conn: &rusqlite::Connection, since_ms: i64, until_ms: i64,
) -> Result<Vec<KeyPoint>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT client_key, requested_model, result, COUNT(*)
               FROM gateway_route_log
              WHERE ts >= ?1 AND ts < ?2 AND client_key IS NOT NULL AND client_key <> ''
              GROUP BY client_key, requested_model, result
              ORDER BY client_key, requested_model, result",
        )
        .map_err(|error| format!("准备按 Key 的用量查询失败：{error}"))?;
    let rows = stmt
        .query_map([since_ms, until_ms], |row| {
            Ok(KeyPoint { name: row.get(0)?, model: row.get(1)?, result: row.get(2)?, requests: row.get(3)? })
        })
        .map_err(|error| format!("查询按 Key 的用量失败：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("读取按 Key 的用量失败：{error}"))
}

/// POST 到 OTLP 端点。`headers` 用于认证（如 `Authorization`），只在本函数内使用，不回显。
pub async fn export(
    endpoint: &str, headers: &[(String, String)], points: &[UsagePoint], keys: &[KeyPoint], now_ms: i64,
) -> Result<OtlpExportResult, String> {
    let url = metrics_url(endpoint)?;
    if points.is_empty() && keys.is_empty() { return Err("这段时间没有可导出的用量".into()); }
    let payload = build_payload(points, keys, now_ms);
    let body = serde_json::to_vec(&payload).map_err(|error| format!("序列化 OTLP 载荷失败：{error}"))?;
    let client = crate::net::async_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("创建 OTLP 客户端失败：{error}"))?;
    let mut request = client
        .post(&url)
        .header("content-type", "application/json")
        .body(body);
    for (key, value) in headers {
        request = request.header(key.as_str(), value.as_str());
    }
    let response = request.send().await.map_err(|error| format!("连接 OTLP 端点失败：{error}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let snippet = if text.trim().is_empty() { None } else { Some(text.chars().take(400).collect::<String>()) };
    if !status.is_success() {
        return Err(format!(
            "OTLP 端点返回 {}{}",
            status.as_u16(),
            snippet.as_deref().map(|s| format!("：{s}")).unwrap_or_default()
        ));
    }
    Ok(OtlpExportResult {
        endpoint: url,
        points: points.len() + keys.len(),
        status: Some(status.as_u16()),
        response: snippet,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point() -> UsagePoint {
        UsagePoint {
            platform: "claude".into(),
            model: Some("claude-sonnet-4".into()),
            input_tokens: 120,
            output_tokens: 80,
            cache_read_tokens: 0,
            cache_write_tokens: 10,
            total_tokens: 210,
            requests: 3,
        }
    }

    #[test]
    fn endpoint_gets_metrics_path_once() {
        assert_eq!(metrics_url("http://localhost:4318").unwrap(), "http://localhost:4318/v1/metrics");
        assert_eq!(metrics_url("http://localhost:4318/").unwrap(), "http://localhost:4318/v1/metrics");
        assert_eq!(
            metrics_url("http://localhost:4318/v1/metrics").unwrap(),
            "http://localhost:4318/v1/metrics",
            "已经写了完整路径不能重复拼"
        );
        assert!(metrics_url("").is_err());
        assert!(metrics_url("localhost:4318").is_err(), "缺协议必须报错");
        assert!(metrics_url("ftp://host").is_err());
    }

    #[test]
    fn payload_has_no_identifying_fields() {
        let payload = build_payload(&[point()], &[], 1_700_000_000_000);
        let text = payload.to_string();
        for forbidden in ["session", "dedup", "api_key", "apikey", "authorization", "token="] {
            assert!(
                !text.to_lowercase().contains(forbidden),
                "导出载荷不能出现 {forbidden}：{text}"
            );
        }
    }

    #[test]
    fn payload_carries_platform_model_and_token_kinds() {
        let payload = build_payload(&[point()], &[], 1_700_000_000_000);
        let metrics = payload["resourceMetrics"][0]["scopeMetrics"][0]["metrics"].as_array().unwrap();
        let names: Vec<&str> = metrics.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert!(names.contains(&METRIC_TOKENS));
        assert!(names.contains(&METRIC_REQUESTS));
        let token_metric = metrics.iter().find(|m| m["name"] == METRIC_TOKENS).unwrap();
        let points = token_metric["gauge"]["dataPoints"].as_array().unwrap();
        // input / output / cache_write 有用量，cache_read 为 0 应跳过
        assert_eq!(points.len(), 3, "零值数据点不导出");
        let kinds: Vec<&str> = points
            .iter()
            .map(|p| p["attributes"][2]["value"]["stringValue"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["input", "output", "cache_write"]);
    }

    #[test]
    fn missing_model_is_marked_unknown_not_omitted() {
        let mut point = point();
        point.model = None;
        let payload = build_payload(&[point], &[], 1_700_000_000_000);
        assert!(payload.to_string().contains("unknown"));
    }

    #[test]
    fn timestamps_are_nanoseconds() {
        let payload = build_payload(&[point()], &[], 1_700_000_000_000);
        let metric = &payload["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0];
        let stamp = metric["gauge"]["dataPoints"][0]["timeUnixNano"].as_str().unwrap();
        assert_eq!(stamp, "1700000000000000000", "毫秒要换成纳秒");
    }

    #[test]
    fn aggregation_selects_no_identifying_columns() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE requests (
                platform TEXT, model TEXT, ts INTEGER,
                input_tokens INTEGER, output_tokens INTEGER,
                cache_read_tokens INTEGER, cache_write_tokens INTEGER, total_tokens INTEGER
            );
             INSERT INTO requests VALUES ('claude','m1',100,10,20,0,0,30);
             INSERT INTO requests VALUES ('claude','m1',200,5,5,0,0,10);
             INSERT INTO requests VALUES ('codex',NULL,150,7,3,0,0,10);
             INSERT INTO requests VALUES ('claude','m1',9999,1,1,0,0,2);",
        )
        .unwrap();
        let points = aggregate(&conn, 0, 1000).unwrap();
        assert_eq!(points.len(), 2, "按 platform + model 分组，区间外的排除");
        let claude = points.iter().find(|p| p.platform == "claude").unwrap();
        assert_eq!((claude.input_tokens, claude.output_tokens, claude.requests), (15, 25, 2));
        assert_eq!(claude.total_tokens, 40, "total 取和而不是重算");
        let codex = points.iter().find(|p| p.platform == "codex").unwrap();
        assert_eq!(codex.model, None, "来源没给模型时保留 NULL，导出才写 unknown");
    }

    #[test]
    fn named_key_requests_carry_the_key_name_attribute() {
        // 局域网共享多台设备各用一把 Key：排查「哪台设备在失败」要看到 Key 名（magpie `4314f4b`）
        let keys = [KeyPoint { name: "laptop".into(), model: Some("claude-opus-5-5".into()), result: "ok".into(), requests: 4 }];
        let payload = build_payload(&[], &keys, 1_700_000_000_000);
        let metrics = payload["resourceMetrics"][0]["scopeMetrics"][0]["metrics"].as_array().unwrap();
        let metric = metrics.iter().find(|m| m["name"] == METRIC_KEY_REQUESTS).expect("按 Key 的请求数指标");
        let point = &metric["gauge"]["dataPoints"][0];
        let attrs = point["attributes"].as_array().unwrap();
        let get = |key: &str| attrs.iter().find(|a| a["key"] == key).map(|a| a["value"]["stringValue"].clone());
        assert_eq!(get("jolu.gateway_key.name").unwrap(), "laptop");
        assert_eq!(get("gen_ai.request.model").unwrap(), "claude-opus-5-5");
        assert_eq!(get("jolu.result").unwrap(), "ok");
        assert_eq!(point["asInt"], "4");
    }

    #[test]
    fn key_aggregation_reads_only_named_key_rows_in_the_window() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE gateway_route_log (ts INTEGER, requested_model TEXT, result TEXT, client_key TEXT, session_id TEXT);
             INSERT INTO gateway_route_log VALUES (100,'m1','ok','laptop','s1');
             INSERT INTO gateway_route_log VALUES (200,'m1','ok','laptop','s2');
             INSERT INTO gateway_route_log VALUES (300,'m1','failed','laptop','s3');
             INSERT INTO gateway_route_log VALUES (300,'m1','ok',NULL,'s4');
             INSERT INTO gateway_route_log VALUES (5000,'m1','ok','laptop','s5');",
        )
        .unwrap();
        let keys = aggregate_keys(&conn, 0, 1000).unwrap();
        assert_eq!(keys.len(), 2, "本机自己的请求（没有 Key 名）不进按 Key 指标");
        let ok = keys.iter().find(|k| k.result == "ok").unwrap();
        assert_eq!((ok.name.as_str(), ok.requests), ("laptop", 2));
        assert!(!serde_json::to_string(&keys).unwrap().contains("s1"), "不带会话");
    }
}
