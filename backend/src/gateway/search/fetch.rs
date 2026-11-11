//! `web_fetch`：读一个网页的正文给模型（能力汇聚第 1 层的第二个工具）。
//!
//! 管什么：
//! - 只抓 http / https；每一跳（含重定向目标）解析出的每个地址都过 netguard，
//!   并把连接**钉在检查过的地址上**——否则检查后再次解析时 DNS 可以换成内网地址
//!   （DNS rebinding），检查就白做了；
//! - 正文最多读 [`BODY_MAX`] 字节，HTML 转成纯文本（去掉脚本、样式与标签），
//!   给模型的文本最多 `max_chars` 字，截断时如实标注。
//!
//! 不管什么：不执行脚本（要渲染的单页应用读不到正文，如实返回空文本）；不带 Cookie；
//! 走代理时由代理解析域名，钉地址无效——这时只校验用户给的地址本身。
//!
//! 参照 magpie `internal/gateway/search.go` 的页面读取与 `netguard`（克隆 `0ee424e`）；
//! 差异：magpie 先尝试 Jina Reader 等外部转写服务，Jolu 不把用户要读的地址发给第三方。

use std::net::{IpAddr, SocketAddr};

use crate::gateway::net_guard::{self, Verdict};

/// 最多读的正文字节数。
const BODY_MAX: usize = 2 * 1024 * 1024;
/// 最多跟随的重定向次数。
const HOPS_MAX: usize = 5;
/// 默认给模型的字符数。
pub(crate) const TEXT_DEFAULT: usize = 20_000;

/// 读到的页面。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Page {
    /// 最终地址（跟随重定向之后）
    pub url: String,
    pub title: String,
    pub text: String,
    /// 正文超过 `max_chars` 被截断
    pub truncated: bool,
}

/// 地址是否允许抓取，允许时给出要钉住的地址（IP 字面量或解析结果）。
fn vet(url: &url::Url) -> Result<Vec<SocketAddr>, String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("只能读取 http / https 地址，「{}」不行", url.scheme()));
    }
    let host = url.host_str().ok_or("地址缺少主机")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
            .map_err(|error| format!("解析 {host} 失败：{error}"))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(format!("{host} 没有解析出地址"));
    }
    let answers: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
    for addr in &answers {
        if net_guard::judge(*addr, &answers) == Verdict::Private {
            return Err(format!("{host} 指向 {addr}，不在公网（回环 / 私网 / 保留段），不读取"));
        }
    }
    Ok(addrs)
}

/// 读取一个网页。重定向手动跟随，每一跳重新校验并钉地址。
pub async fn fetch(raw_url: &str, max_chars: usize) -> Result<Page, String> {
    let mut url = url::Url::parse(raw_url.trim()).map_err(|error| format!("地址无效：{error}"))?;
    for _ in 0..=HOPS_MAX {
        let addrs = tokio::task::spawn_blocking({
            let url = url.clone();
            move || vet(&url)
        })
        .await
        .map_err(|error| format!("地址校验失败：{error}"))??;
        let host = url.host_str().unwrap_or_default().to_string();
        let mut builder = crate::net::async_builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none());
        if host.parse::<IpAddr>().is_err() {
            builder = builder.resolve_to_addrs(&host, &addrs);
        }
        let client = builder.build().map_err(|error| format!("无法创建客户端：{error}"))?;
        let mut response = client
            .get(url.clone())
            .header("accept", "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5")
            .header("user-agent", concat!("jolu-fetch/", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(|error| format!("读取 {url} 失败：{error}"))?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("{url} 返回 {status} 但没有给出跳转地址"))?;
            url = url.join(location).map_err(|error| format!("跳转地址无效：{error}"))?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("{url} 返回 {status}"));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let html = content_type.contains("html") || content_type.is_empty();
        let textual = html || content_type.starts_with("text/") || content_type.contains("json") || content_type.contains("xml");
        if !textual {
            return Err(format!("{url} 不是文本页面（{content_type}）"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| format!("读取正文失败：{error}"))? {
            let room = BODY_MAX.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if body.len() >= BODY_MAX {
                break;
            }
        }
        let raw = String::from_utf8_lossy(&body);
        let (title, text) = if html { html_to_text(&raw) } else { (String::new(), raw.trim().to_string()) };
        let truncated = text.chars().count() > max_chars;
        let text = if truncated { text.chars().take(max_chars).collect() } else { text };
        return Ok(Page { url: url.to_string(), title, text, truncated });
    }
    Err(format!("跳转超过 {HOPS_MAX} 次，已停止"))
}

/// 不进正文的元素：内容整段丢弃。
const DROPPED: &[&str] = &["script", "style", "noscript", "svg", "template", "head", "iframe"];
/// 块级元素：前后换行，保持段落。
const BLOCKS: &[&str] = &[
    "p", "div", "br", "li", "ul", "ol", "tr", "table", "section", "article", "header", "footer", "h1", "h2", "h3", "h4",
    "h5", "h6", "pre", "blockquote", "hr", "main", "nav", "aside", "dd", "dt", "figcaption",
];

/// HTML 转纯文本，返回（标题，正文）。不求完美排版，只求模型读得到正文、读不到脚本。
pub(crate) fn html_to_text(html: &str) -> (String, String) {
    let lower = html.to_ascii_lowercase();
    let title = lower
        .find("<title")
        .and_then(|start| {
            let open_end = start + lower[start..].find('>')? + 1;
            let close = open_end + lower[open_end..].find("</title")?;
            Some(unescape(html[open_end..close].trim()))
        })
        .unwrap_or_default();
    let mut out = String::with_capacity(html.len() / 2);
    let mut index = 0;
    while index < html.len() {
        let Some(offset) = html[index..].find('<') else {
            out.push_str(&html[index..]);
            break;
        };
        out.push_str(&html[index..index + offset]);
        let tag_start = index + offset;
        if lower[tag_start..].starts_with("<!--") {
            index = lower[tag_start..].find("-->").map(|end| tag_start + end + 3).unwrap_or(html.len());
            continue;
        }
        let Some(tag_len) = html[tag_start..].find('>') else { break };
        let tag_end = tag_start + tag_len + 1;
        let name: String = lower[tag_start + 1..tag_end - 1]
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let closing = lower[tag_start + 1..].starts_with('/');
        if !closing && DROPPED.contains(&name.as_str()) {
            let close = format!("</{name}");
            index = lower[tag_end..].find(&close).map(|at| tag_end + at).and_then(|at| {
                lower[at..].find('>').map(|end| at + end + 1)
            }).unwrap_or(html.len());
            continue;
        }
        if BLOCKS.contains(&name.as_str()) {
            out.push('\n');
        } else {
            out.push(' ');
        }
        index = tag_end;
    }
    let text = unescape(&out);
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() {
            lines.push(line);
        }
    }
    (title, lines.join("\n"))
}

fn unescape(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_readable_text_without_scripts_or_styles() {
        let html = r#"<!doctype html><html><head><title>Rust &amp; 发布</title><style>p{color:red}</style>
            <script>alert("x")</script></head><body><!-- 注释 --><h1>标题</h1><p>第一段 <b>加粗</b></p>
            <ul><li>一</li><li>二</li></ul><SCRIPT type="module">evil()</SCRIPT><p>结尾&nbsp;段</p></body></html>"#;
        let (title, text) = html_to_text(html);
        assert_eq!(title, "Rust & 发布");
        assert_eq!(text, "标题\n第一段 加粗\n一\n二\n结尾 段");
        assert!(!text.contains("alert") && !text.contains("evil") && !text.contains("color"));
    }

    #[test]
    fn unterminated_markup_does_not_panic() {
        assert_eq!(html_to_text("<p>半截 <b").1, "半截");
        assert_eq!(html_to_text("<script>没有结束").1, "");
        assert_eq!(html_to_text("纯文本").1, "纯文本");
    }

    #[test]
    fn only_public_http_addresses_are_fetched() {
        let vet_url = |raw: &str| vet(&url::Url::parse(raw).unwrap());
        assert!(vet_url("file:///etc/passwd").unwrap_err().contains("http"));
        assert!(vet_url("ftp://example.com/x").is_err());
        for private in ["http://127.0.0.1/", "http://169.254.169.254/latest/meta-data", "http://192.168.1.1/", "http://[::1]/"] {
            assert!(vet_url(private).unwrap_err().contains("不在公网"), "{private}");
        }
        assert_eq!(vet_url("https://1.1.1.1/").unwrap(), vec!["1.1.1.1:443".parse().unwrap()]);
    }

    #[tokio::test]
    async fn fetch_refuses_private_targets_before_connecting() {
        let error = fetch("http://127.0.0.1:9/", 100).await.unwrap_err();
        assert!(error.contains("不在公网"), "{error}");
    }
}
