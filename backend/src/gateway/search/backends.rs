//! 九个搜索后端的请求构造与响应解析。
//!
//! 分两层，便于不联网测试：
//! - [`request`]：按后端拼出要发的请求（方法、地址、查询参数、JSON 体、鉴权头）；
//! - [`parse`]：按后端读出结果，并把「HTTP 200 但正文里带错误」的后端（豆包、元宝）
//!   映射回状态码，交给 [`map_status`] 判定是否「Key 被拒」。
//!
//! [`search`] 只负责把两者接起来发出去。所有请求都走 [`crate::net::async_builder`]，
//! 尊重联网方式；**不跟随跨主机重定向**——Brave / Exa / You.com 的 Key 在自定义头里，
//! 跨主机重定向时 reqwest 只会剥掉 `Authorization`，自定义头会被带去别的主机。
//!
//! 参照 magpie `internal/gateway/search_api.go::askSearchAPI` 与 `readSearchAPI`
//! （克隆 `0ee424e`）。差异：magpie 的豆包 id 是 `volcengine`，Jolu 保留已发布的
//! `doubao`；magpie 每次固定取 6 条，Jolu 用调用方给的条数（上限 20）。

use super::{SearchError, SearchHit};

/// 摘要最多保留的字符数（magpie `pageText` 同值）。
const SNIPPET_MAX: usize = 1500;
/// DeepSeek 搜索的查询上限（超出回 400，api-docs.deepseek.com/guides/search）。
const DEEPSEEK_QUERY_MAX: usize = 128;
/// 豆包搜索的查询上限。
const DOUBAO_QUERY_MAX: usize = 100;
/// 元宝搜索的 `Cnt` 只收 10/20/30/40/50，取最小的再截断。
const WSA_COUNT: usize = 10;
/// You.com 免 Key 的公共搜索（每个地址每天 100 次，之后 429）。
const YOU_FREE_SEARCH: &str = "https://api.you.com/v1/agents/search";

/// 后端默认地址；SearXNG 没有（必须自建）。
pub(crate) fn default_base(vendor: &str) -> Option<&'static str> {
    Some(match vendor {
        "tavily" => "https://api.tavily.com",
        "brave" => "https://api.search.brave.com",
        "exa" => "https://api.exa.ai",
        "firecrawl" => "https://api.firecrawl.dev",
        "you" => "https://ydc-index.io",
        "deepseek" => "https://api.deepseek.com",
        "doubao" => "https://open.feedcoopapi.com",
        "wsa" => "https://api.wsa.cloud.tencent.com",
        _ => return None,
    })
}

/// 可以不填 Key 的后端：SearXNG（自建）与 You.com（免费搜索）。
pub(crate) fn key_optional(vendor: &str) -> bool {
    matches!(vendor, "searxng" | "you")
}

/// 后端的搜索路径。用户填的地址已经以它结尾时不再重复拼（magpie #509 `/search/search`）。
fn paths(vendor: &str) -> &'static [&'static str] {
    match vendor {
        "tavily" | "exa" | "searxng" => &["/search"],
        "brave" => &["/res/v1/web/search"],
        "firecrawl" => &["/v2/search"],
        "you" => &["/v1/search", "/v1/agents/search"],
        "deepseek" => &["/v1/tools/search"],
        "doubao" => &["/search_api/web_search"],
        "wsa" => &["/SearchPro"],
        _ => &[],
    }
}

/// 实际地址：用户填的（去掉末尾斜杠与已带的搜索路径）或默认地址。
pub(crate) fn base(vendor: &str, url: Option<&str>) -> Result<String, SearchError> {
    let given = url.map(str::trim).filter(|s| !s.is_empty());
    let Some(given) = given else {
        return default_base(vendor)
            .map(str::to_string)
            .ok_or_else(|| SearchError::Other(format!("{} 需要填写实例地址", super::display_name(vendor))));
    };
    let mut base = given.trim_end_matches('/').to_string();
    for path in paths(vendor) {
        if let Some(stripped) = base.strip_suffix(path) {
            base = stripped.to_string();
            break;
        }
    }
    Ok(base)
}

/// 一次要发的请求。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Request {
    pub post: bool,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
    pub headers: Vec<(String, String)>,
}

fn first_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 按后端拼请求。`count` 已由调用方限定在 1–20。
pub(crate) fn request(vendor: &str, url: Option<&str>, key: &str, query: &str, count: usize) -> Result<Request, SearchError> {
    use serde_json::json;
    let base = base(vendor, url)?;
    let post = |path: &str, body: serde_json::Value| Request {
        post: true, url: format!("{base}{path}"), query: Vec::new(), body: Some(body), headers: Vec::new(),
    };
    let get = |at: String, query: Vec<(&str, String)>| Request {
        post: false, url: at, query: query.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), body: None, headers: Vec::new(),
    };
    let n = count.to_string();
    let mut request = match vendor {
        "tavily" => post("/search", json!({ "query": query, "max_results": count, "search_depth": "basic" })),
        "brave" => get(format!("{base}/res/v1/web/search"), vec![("q", query.into()), ("count", n)]),
        "exa" => post("/search", json!({ "query": query, "numResults": count, "contents": { "text": { "maxCharacters": SNIPPET_MAX } } })),
        "firecrawl" => post("/v2/search", json!({ "query": query, "limit": count })),
        // DeepSeek 的摘要就是给模型的正文，need_content 保持默认 false
        "deepseek" => post("/v1/tools/search", json!({ "query": first_chars(query, DEEPSEEK_QUERY_MAX), "count": count })),
        // 豆包搜索 Custom 版：只要网页、只要带 URL 的（NeedUrl 排除没有链接的卡片）
        "doubao" => post("/search_api/web_search", json!({
            "Query": first_chars(query, DOUBAO_QUERY_MAX), "SearchType": "web", "Count": count, "Filter": { "NeedUrl": true },
        })),
        "wsa" => post("/SearchPro", json!({ "Query": query, "Cnt": WSA_COUNT })),
        "searxng" => get(format!("{base}/search"), vec![("q", query.into()), ("format", "json".into())]),
        "you" => {
            if !key.is_empty() {
                post("/v1/search", json!({ "query": query, "count": count }))
            } else if url.map(str::trim).is_some_and(|u| !u.is_empty()) {
                get(format!("{base}/v1/agents/search"), vec![("query", query.into()), ("count", n)])
            } else {
                get(YOU_FREE_SEARCH.to_string(), vec![("query", query.into()), ("count", n)])
            }
        }
        other => return Err(SearchError::Other(format!("未知搜索后端「{other}」"))),
    };
    request.headers.push(("accept".into(), "application/json".into()));
    if !key.is_empty() {
        let header = match vendor {
            "brave" => ("x-subscription-token".to_string(), key.to_string()),
            "exa" | "you" => ("x-api-key".to_string(), key.to_string()),
            _ => ("authorization".to_string(), format!("Bearer {key}")),
        };
        request.headers.push(header);
    }
    Ok(request)
}

/// 把后端 HTTP 状态映射成 [`SearchError`]：401/402/403/429 是「Key 被拒」（轮转层会换下一个
/// Key；402 是这把 Key 的额度用完，换一把可能就好），其余是「Other」（换 Key 没用）。
/// 错误文案里出现 Key 原文时替换成掩码——有的后端会把 Key 回显在错误里。
fn map_status(vendor: &str, status: u16, message: &str, key: &str) -> SearchError {
    let mut snippet: String = message.chars().take(200).collect();
    if !key.is_empty() {
        snippet = snippet.replace(key, &super::secret_keys::mask(key));
    }
    let text = format!("{} 返回 {status}：{snippet}", super::display_name(vendor));
    match status {
        401 | 402 | 403 | 429 => SearchError::Refused(text),
        _ => SearchError::Other(text),
    }
}

/// 豆包：错误在 `ResponseMetadata.Error`，可能伴随 200。按错误码映射回状态。
fn doubao_error(value: &serde_json::Value, status: u16) -> Option<(u16, String)> {
    let error = value.pointer("/ResponseMetadata/Error").filter(|e| e.is_object())?;
    let code = error
        .get("CodeN")
        .and_then(serde_json::Value::as_i64)
        .filter(|c| *c != 0)
        .or_else(|| error.get("Code").and_then(serde_json::Value::as_str).and_then(|c| c.parse().ok()))
        .unwrap_or(0);
    let message = format!("{code} {}", error.get("Message").and_then(serde_json::Value::as_str).unwrap_or_default());
    let mapped = match code {
        10401 | 10403 | 10409 => 403, // Key 不认识、服务未开通、套餐 Key 不能这样搜
        10406 | 10408 | 10410 | 10412 => 402, // 免费额度用完、无套餐、套餐用完
        700429 => 429,
        _ if status < 300 => 502,
        _ => status,
    };
    Some((mapped, message.trim().to_string()))
}

/// 元宝（腾讯云）：请求被处理就回 200，错误在 `Response.Error {Code, Message}`。
fn wsa_error(value: &serde_json::Value, status: u16) -> Option<(u16, String)> {
    let error = value.pointer("/Response/Error").filter(|e| e.is_object())?;
    let code = error.get("Code").and_then(serde_json::Value::as_str).filter(|c| !c.is_empty())?;
    let message = format!("{code} {}", error.get("Message").and_then(serde_json::Value::as_str).unwrap_or_default());
    let mapped = match code.split('.').next().unwrap_or(code) {
        "AuthFailure" | "UnauthorizedOperation" | "ResourceNotFound" => 403,
        "ResourceUnavailable" | "LimitExceeded" | "ResourceInsufficient" => 402,
        "RequestLimitExceeded" => 429,
        _ if status < 300 => 502,
        _ => status,
    };
    Some((mapped, message.trim().to_string()))
}

/// 搜索结果里的文字：去掉标记命中词的标签（Brave 的 `<strong>`）、还原常见实体、压成一行。
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hit(title: &str, url: &str, text: &str) -> Option<SearchHit> {
    if url.trim().is_empty() {
        return None;
    }
    let title = plain(title);
    Some(SearchHit {
        title: if title.is_empty() { url.to_string() } else { title },
        url: url.to_string(),
        snippet: first_chars(&plain(text), SNIPPET_MAX),
    })
}

fn str_at<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value.get(key).and_then(serde_json::Value::as_str).unwrap_or_default()
}

/// 读一次响应：先查正文里的错误（豆包、元宝），再按状态判错，最后按后端形状取结果。
pub(crate) fn parse(vendor: &str, status: u16, body: &str, key: &str, count: usize) -> Result<Vec<SearchHit>, SearchError> {
    let value: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let embedded = value.as_ref().and_then(|v| match vendor {
        "doubao" => doubao_error(v, status),
        "wsa" => wsa_error(v, status),
        _ => None,
    });
    if let Some((code, message)) = embedded {
        return Err(map_status(vendor, code, &message, key));
    }
    if !(200..300).contains(&status) {
        // You.com 的错误是代码（payment_required），文字在 message
        let message = value
            .as_ref()
            .and_then(|v| v.get("message").or_else(|| v.pointer("/error/message")))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| body.to_string());
        return Err(map_status(vendor, status, &message, key));
    }
    let value = value.ok_or_else(|| SearchError::Other(format!("{} 响应不是 JSON", super::display_name(vendor))))?;
    let shape = || SearchError::Other(format!("{} 响应形状不符：缺少结果", super::display_name(vendor)));
    let array = |pointer: &str| value.pointer(pointer).and_then(serde_json::Value::as_array);
    let hits: Vec<SearchHit> = match vendor {
        "doubao" => {
            let list = array("/Result/WebResults").ok_or_else(shape)?;
            list.iter()
                .filter_map(|h| {
                    let summary = str_at(h, "Summary");
                    let text = if summary.trim().is_empty() { str_at(h, "Snippet") } else { summary };
                    hit(str_at(h, "Title"), str_at(h, "Url"), text)
                })
                .collect()
        }
        "wsa" => {
            // 每页是一个「JSON 字符串」
            let list = array("/Response/Pages").ok_or_else(shape)?;
            list.iter()
                .filter_map(|page| serde_json::from_str::<serde_json::Value>(page.as_str()?).ok())
                .filter_map(|h| {
                    let passage = str_at(&h, "passage");
                    let title = str_at(&h, "title");
                    // 摘要只是标题时，用 content（极速版没有 content）
                    let text = if plain(passage).is_empty() || plain(passage) == plain(title) {
                        Some(str_at(&h, "content")).filter(|c| !plain(c).is_empty()).unwrap_or(passage)
                    } else {
                        passage
                    };
                    hit(title, str_at(&h, "url"), text)
                })
                .collect()
        }
        "you" => {
            let results = value.get("results").filter(|r| r.is_object()).ok_or_else(shape)?;
            ["web", "news"]
                .iter()
                .filter_map(|kind| results.get(*kind).and_then(serde_json::Value::as_array))
                .flatten()
                .filter_map(|h| {
                    let snippets: Vec<&str> = h
                        .get("snippets")
                        .and_then(serde_json::Value::as_array)
                        .map(|s| s.iter().filter_map(serde_json::Value::as_str).collect())
                        .unwrap_or_default();
                    let joined = snippets.join(" ");
                    let text = if joined.is_empty() { str_at(h, "description").to_string() } else { joined };
                    hit(str_at(h, "title"), str_at(h, "url"), &text)
                })
                .collect()
        }
        _ => {
            let list = match vendor {
                "brave" => array("/web/results"),
                // v2 是 data.web，v1 是 data 裸数组
                "firecrawl" => array("/data/web").or_else(|| array("/data")),
                _ => array("/results"),
            }
            .ok_or_else(shape)?;
            list.iter()
                .filter_map(|h| {
                    let text = ["content", "summary", "text", "description", "markdown", "snippet"]
                        .iter()
                        .map(|field| str_at(h, field))
                        .find(|t| !t.is_empty())
                        .unwrap_or_default();
                    hit(str_at(h, "title"), str_at(h, "url"), text)
                })
                .collect()
        }
    };
    Ok(hits.into_iter().take(count.max(1)).collect())
}

/// 统一的异步客户端：只跟随同主机重定向（见模块头）。
fn client() -> Result<reqwest::Client, SearchError> {
    crate::net::async_builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let same = attempt.previous().first().is_some_and(|first| {
                first.scheme() == attempt.url().scheme() && first.host_str() == attempt.url().host_str()
            });
            if attempt.previous().len() >= 10 || !same {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| SearchError::Other(format!("无法创建搜索客户端：{error}")))
}

/// 发一次搜索。地址先过 netguard（防 DNS 把请求送进内网）。
pub(crate) async fn search(vendor: &str, url: Option<&str>, key: &str, query: &str, count: usize) -> Result<Vec<SearchHit>, SearchError> {
    let count = count.clamp(1, 20);
    let spec = request(vendor, url, key, query, count)?;
    super::check_endpoint(&spec.url)?;
    let client = client()?;
    let mut builder = if spec.post { client.post(&spec.url) } else { client.get(&spec.url) };
    if !spec.query.is_empty() {
        builder = builder.query(&spec.query);
    }
    if let Some(body) = &spec.body {
        builder = builder.header("content-type", "application/json").body(body.to_string());
    }
    for (name, value) in &spec.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let name = super::display_name(vendor);
    let response = builder.send().await.map_err(|error| SearchError::Other(format!("连接{name}失败：{error}")))?;
    let status = response.status();
    if status.is_redirection() {
        let to = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("另一个地址").to_string();
        return Err(SearchError::Other(format!("{name} 把搜索转到了 {to}，Key 不会带过去；请直接把地址设为它")));
    }
    let text = response.text().await.unwrap_or_default();
    parse(vendor, status.as_u16(), &text, key, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
        request.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn each_vendor_puts_its_key_in_its_own_header() {
        let brave = request("brave", None, "k1", "rust", 5).unwrap();
        assert_eq!(header(&brave, "x-subscription-token"), Some("k1"));
        assert!(header(&brave, "authorization").is_none());
        assert_eq!(brave.url, "https://api.search.brave.com/res/v1/web/search");
        assert!(!brave.post && brave.query.contains(&("q".into(), "rust".into())));
        for vendor in ["exa", "you"] {
            assert_eq!(header(&request(vendor, None, "k2", "q", 5).unwrap(), "x-api-key"), Some("k2"), "{vendor}");
        }
        for vendor in ["tavily", "firecrawl", "deepseek", "doubao", "wsa"] {
            assert_eq!(header(&request(vendor, None, "k3", "q", 5).unwrap(), "authorization"), Some("Bearer k3"), "{vendor}");
        }
    }

    #[test]
    fn request_bodies_follow_each_vendor_contract() {
        let doubao = request("doubao", None, "k", &"长".repeat(150), 5).unwrap();
        let body = doubao.body.unwrap();
        assert_eq!(body["Query"].as_str().unwrap().chars().count(), DOUBAO_QUERY_MAX, "豆包查询截到 100 字");
        assert_eq!(body["Filter"]["NeedUrl"], true);
        let deepseek = request("deepseek", None, "k", &"x".repeat(300), 5).unwrap().body.unwrap();
        assert_eq!(deepseek["query"].as_str().unwrap().len(), DEEPSEEK_QUERY_MAX);
        assert_eq!(request("wsa", None, "k", "q", 3).unwrap().body.unwrap()["Cnt"], 10, "元宝 Cnt 只收 10 起");
        assert_eq!(request("exa", None, "k", "q", 4).unwrap().body.unwrap()["numResults"], 4);
        assert_eq!(request("firecrawl", None, "k", "q", 4).unwrap().url, "https://api.firecrawl.dev/v2/search");
    }

    #[test]
    fn you_without_a_key_uses_the_free_search_and_sends_no_key_header() {
        let free = request("you", None, "", "q", 5).unwrap();
        assert_eq!(free.url, YOU_FREE_SEARCH);
        assert!(header(&free, "x-api-key").is_none());
        let own = request("you", Some("https://my.you/"), "", "q", 5).unwrap();
        assert_eq!(own.url, "https://my.you/v1/agents/search", "免 Key 但填了自己的地址");
        assert_eq!(request("you", None, "k", "q", 5).unwrap().url, "https://ydc-index.io/v1/search");
    }

    #[test]
    fn a_base_given_with_the_search_path_is_not_doubled() {
        assert_eq!(base("searxng", Some("https://sx.example/search/")).unwrap(), "https://sx.example");
        assert_eq!(base("tavily", Some("https://my.proxy/")).unwrap(), "https://my.proxy");
        assert_eq!(base("tavily", None).unwrap(), "https://api.tavily.com");
        assert!(matches!(base("searxng", None), Err(SearchError::Other(m)) if m.contains("实例地址")));
    }

    #[test]
    fn generic_shapes_parse_and_skip_entries_without_url() {
        let tavily = json!({"results": [{"title": "Rust 1.90", "url": "https://r/1.90", "content": "发布说明"}, {"title": "缺 url"}]});
        let hits = parse("tavily", 200, &tavily.to_string(), "", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].title.as_str(), hits[0].snippet.as_str()), ("Rust 1.90", "发布说明"));
        let brave = json!({"web": {"results": [{"title": "<strong>Rust</strong> &amp; Go", "url": "https://b", "description": "d"}]}});
        let hits = parse("brave", 200, &brave.to_string(), "", 5).unwrap();
        assert_eq!(hits[0].title, "Rust & Go", "去标签、还原实体");
        let firecrawl_v2 = json!({"data": {"web": [{"title": "", "url": "https://f", "markdown": "md"}]}});
        let hits = parse("firecrawl", 200, &firecrawl_v2.to_string(), "", 5).unwrap();
        assert_eq!(hits[0].title, "https://f", "没有标题用链接");
        let firecrawl_v1 = json!({"data": [{"title": "t", "url": "https://f1"}]});
        assert_eq!(parse("firecrawl", 200, &firecrawl_v1.to_string(), "", 5).unwrap().len(), 1);
        let deepseek = json!({"results": [{"title": "t", "url": "https://d", "summary": "长摘要", "snippet": "短"}]});
        assert_eq!(parse("deepseek", 200, &deepseek.to_string(), "", 5).unwrap()[0].snippet, "长摘要");
        assert!(parse("exa", 200, r#"{"error":"x"}"#, "", 5).is_err(), "缺结果如实报错");
    }

    #[test]
    fn you_doubao_and_wsa_shapes_parse() {
        let you = json!({"results": {"web": [{"title": "w", "url": "https://w", "snippets": ["a", "b"]}], "news": [{"title": "n", "url": "https://n", "description": "d"}]}});
        let hits = parse("you", 200, &you.to_string(), "", 5).unwrap();
        assert_eq!((hits.len(), hits[0].snippet.as_str(), hits[1].snippet.as_str()), (2, "a b", "d"));
        let doubao = json!({"Result": {"WebResults": [{"Title": "t", "Url": "https://v", "Summary": " ", "Snippet": "s"}, {"Title": "无链接"}]}});
        let hits = parse("doubao", 200, &doubao.to_string(), "", 5).unwrap();
        assert_eq!((hits.len(), hits[0].snippet.as_str()), (1, "s"), "Summary 为空用 Snippet，无链接的跳过");
        let page = json!({"title": "标题", "url": "https://t", "passage": "标题", "content": "正文"}).to_string();
        let wsa = json!({"Response": {"Pages": [page, "不是 JSON"]}});
        let hits = parse("wsa", 200, &wsa.to_string(), "", 5).unwrap();
        assert_eq!((hits.len(), hits[0].snippet.as_str()), (1, "正文"), "摘要只是标题时用 content");
    }

    #[test]
    fn errors_inside_a_200_are_mapped_and_keys_are_masked_in_messages() {
        let spent = json!({"ResponseMetadata": {"Error": {"CodeN": 10410, "Message": "套餐用完"}}});
        assert!(matches!(parse("doubao", 200, &spent.to_string(), "", 5), Err(SearchError::Refused(m)) if m.contains("402")));
        let other = json!({"ResponseMetadata": {"Error": {"Code": "12345", "Message": "未知"}}});
        assert!(matches!(parse("doubao", 200, &other.to_string(), "", 5), Err(SearchError::Other(m)) if m.contains("502")));
        let auth = json!({"Response": {"Error": {"Code": "AuthFailure.InvalidKey", "Message": "请检查服务密钥"}}});
        assert!(matches!(parse("wsa", 200, &auth.to_string(), "", 5), Err(SearchError::Refused(_))));
        let limited = json!({"Response": {"Error": {"Code": "RequestLimitExceeded", "Message": "频率"}}});
        assert!(matches!(parse("wsa", 200, &limited.to_string(), "", 5), Err(SearchError::Refused(m)) if m.contains("429")));
        let echoed = parse("brave", 401, "bad key tvly-abcdefgh12345678", "tvly-abcdefgh12345678", 5);
        assert!(matches!(echoed, Err(SearchError::Refused(m)) if !m.contains("tvly-abcdefgh12345678") && m.contains("tvly••••5678")));
        let you = parse("you", 402, r#"{"error":"payment_required","message":"额度用完"}"#, "", 5);
        assert!(matches!(you, Err(SearchError::Refused(m)) if m.contains("额度用完")));
        assert!(matches!(parse("exa", 500, "boom", "", 5), Err(SearchError::Other(_))), "5xx 换 Key 没用");
    }
}
