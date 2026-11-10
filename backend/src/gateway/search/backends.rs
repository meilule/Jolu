//! 三个搜索后端的请求构造与响应解析。
//!
//! 每个后端一个文件级的 `search` 函数，签名统一：
//! `(自建地址, Key, 查询, 条数) -> Result<Vec<SearchHit>, SearchError>`。
//!
//! 解析只认**自己构造的请求**对应的响应形状；真实响应与假设不符时如实报
//! 「响应形状不符」，不猜字段。豆包（火山引擎）的响应形状未经真实账号验证——
//! 标注见 `doubao` 模块头，接入真实 Key 后按实测修订。
//!
//! 所有请求都走 [`crate::net::async_builder`]：尊重联网方式，不裸走系统代理。

use super::{SearchError, SearchHit};

/// 统一的异步客户端（每次新建：搜索是低频操作，连接池收益为零）。
fn client() -> Result<reqwest::Client, SearchError> {
    crate::net::async_builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| SearchError::Other(format!("无法创建搜索客户端：{error}")))
}

/// 把后端 HTTP 状态映射成 [`SearchError`]：401/403/429 是「Key 被拒」（轮转层
/// 会换下一个 Key），5xx 与其他状态是「Other」（换 Key 没用）。
fn map_status(vendor: &str, status: reqwest::StatusCode, body: &str) -> SearchError {
    let snippet: String = body.chars().take(200).collect();
    match status.as_u16() {
        401 | 403 | 429 => SearchError::Refused(format!("{vendor} 返回 {status}：{snippet}")),
        _ => SearchError::Other(format!("{vendor} 返回 {status}：{snippet}")),
    }
}

pub(crate) mod tavily {
    //! Tavily：`POST {base}/search`，`Authorization: Bearer <key>`。
    //! 响应 `{ results: [{ title, url, content }], answer? }`。

    use super::*;

    const DEFAULT_BASE: &str = "https://api.tavily.com";

    pub(crate) fn base(url: Option<&str>) -> String {
        let base = url.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(DEFAULT_BASE);
        base.trim_end_matches('/').to_string()
    }

    pub(crate) async fn search(
        url: Option<&str>, key: &str, query: &str, count: usize,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let endpoint = format!("{}/search", base(url));
        super::super::check_endpoint(&endpoint)?;
        let body = serde_json::json!({ "query": query, "max_results": count.clamp(1, 20) });
        let response = client()?
            .post(&endpoint)
            .header("authorization", format!("Bearer {key}"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| SearchError::Other(format!("连接 Tavily 失败：{error}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_status("Tavily", status, &text));
        }
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::Other(format!("Tavily 响应解析失败：{error}")))?;
        parse_results(&value, "results")
    }
}

pub(crate) mod searxng {
    //! SearXNG（自建）：`GET {instance}/search?q=...&format=json`，免 Key。
    //! 实例必须在设置里允许 JSON 输出（`formats: [html, json]`），否则返回 403。

    use super::*;

    pub(crate) async fn search(
        url: Option<&str>, _key: &str, query: &str, count: usize,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let base = url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| SearchError::Other("SearXNG 需要填写实例地址".into()))?;
        let endpoint = format!("{}/search", base.trim_end_matches('/'));
        super::super::check_endpoint(&endpoint)?;
        let response = client()?
            .get(&endpoint)
            .query(&[("q", query), ("format", "json")])
            .send()
            .await
            .map_err(|error| SearchError::Other(format!("连接 SearXNG 实例失败：{error}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_status("SearXNG", status, &text));
        }
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::Other(format!("SearXNG 响应解析失败（确认实例已开放 json 格式）：{error}")))?;
        let hits = parse_results(&value, "results")?;
        Ok(hits.into_iter().take(count.max(1)).collect())
    }
}

pub(crate) mod doubao {
    //! 豆包搜索（火山引擎）：`POST {base}/search_api/web_search`，`Bearer <key>`。
    //!
    //! **响应形状未经真实账号验证**：火山文档的响应包一层 `data`，字段名以接入
    //! 实测为准——当前按 `data[i].{title,url,content}` 解析，实测不符时改这里，
    //! 单测同步改（mock 即合同）。

    use super::*;

    const DEFAULT_BASE: &str = "https://open.feedcoopapi.com";

    pub(crate) fn base(url: Option<&str>) -> String {
        let base = url.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(DEFAULT_BASE);
        base.trim_end_matches('/').to_string()
    }

    pub(crate) async fn search(
        url: Option<&str>, key: &str, query: &str, count: usize,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let endpoint = format!("{}/search_api/web_search", base(url));
        super::super::check_endpoint(&endpoint)?;
        let body = serde_json::json!({ "query": query, "count": count.clamp(1, 20) });
        let response = client()?
            .post(&endpoint)
            .header("authorization", format!("Bearer {key}"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| SearchError::Other(format!("连接豆包搜索失败：{error}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_status("豆包搜索", status, &text));
        }
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| SearchError::Other(format!("豆包搜索响应解析失败：{error}")))?;
        // 文档把列表包在 data 里（裸数组）；没有 data 就按顶层 results 兜底
        let source = value.get("data").filter(|d| d.is_array()).unwrap_or(&value);
        parse_results(source, "results")
    }
}

/// 通用的结果数组解析：`value[key] = [{ title, url, content|snippet }]`；
/// `value` 本身是数组时直接用（豆包的 `data` 就是裸数组）。
fn parse_results(value: &serde_json::Value, key: &str) -> Result<Vec<SearchHit>, SearchError> {
    let list = match value.as_array() {
        Some(list) => list,
        None => value.get(key).and_then(serde_json::Value::as_array).ok_or_else(|| {
            SearchError::Other("响应形状不符：缺少结果数组".into())
        })?,
    };
    Ok(list
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.to_string();
            let url = item.get("url").or_else(|| item.get("link"))?.as_str()?.to_string();
            let snippet = item
                .get("content")
                .or_else(|| item.get("snippet"))
                .or_else(|| item.get("digest"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(SearchHit { title, url, snippet })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tavily_shape_parses_results_and_skips_incomplete_entries() {
        let response = json!({
            "results": [
                {"title": "Rust 1.90", "url": "https://blog.rust-lang.org/1.90", "content": "发布说明"},
                {"title": "缺 url 的条目会被跳过"}
            ]
        });
        let hits = parse_results(&response, "results").unwrap();
        assert_eq!(hits.len(), 1, "缺 url 的条目跳过");
        assert_eq!(hits[0].title, "Rust 1.90");
        assert_eq!(hits[0].snippet, "发布说明");
    }

    #[test]
    fn status_mapping_refuses_only_401_403_429() {
        for code in [401u16, 403, 429] {
            let status = reqwest::StatusCode::from_u16(code).unwrap();
            assert!(matches!(map_status("X", status, ""), SearchError::Refused(_)), "{code} 应算被拒");
        }
        for code in [400u16, 404, 500, 502, 503] {
            let status = reqwest::StatusCode::from_u16(code).unwrap();
            assert!(matches!(map_status("X", status, ""), SearchError::Other(_)), "{code} 不该算被拒");
        }
        // 5xx 不该算被拒：换 Key 没用，是后端的问题
    }

    #[test]
    fn tavily_base_strips_trailing_slash_and_defaults() {
        assert_eq!(tavily::base(None), "https://api.tavily.com");
        assert_eq!(tavily::base(Some("https://my.proxy/")), "https://my.proxy");
    }

    #[test]
    fn doubao_data_array_and_digest_field_are_recognized() {
        // data 是裸数组：直接解析
        let hits = parse_results(&json!([{"title": "t", "url": "u", "digest": "d"}]), "results").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "d", "digest 作为摘要字段被识别");
        // 顶层 results 形状也认（与 Tavily 一致的兜底）
        let wrapped = parse_results(&json!({"results": [{"title": "t", "url": "u"}]}), "results").unwrap();
        assert_eq!(wrapped.len(), 1);
        // 缺结果的响应如实报错
        assert!(parse_results(&json!({"error": "x"}), "results").is_err());
    }

    #[tokio::test]
    async fn searxng_without_instance_url_is_a_config_error() {
        // SearXNG 没填实例地址：不联网直接报配置错误
        let result = searxng::search(None, "", "查询", 5).await;
        assert!(matches!(result, Err(SearchError::Other(reason)) if reason.contains("实例地址")));
    }
}
