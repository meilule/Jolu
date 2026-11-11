//! 网页搜索（能力汇聚第 1 层）：给不带搜索能力的模型补 `web_search`。
//!
//! 管什么：按用户配置的搜索后端（Key + 可选自建地址）发一次搜索，返回
//! 标题 / 链接 / 摘要列表。多 Key 轮转：依次用，被拒的排到最后（可能只是限流）。
//!
//! 不管什么：
//! - **不接管 agent 的工具调用**——那是批 4（替身工具拦截）的事，本模块只提供能力；
//! - 不做网页抓取（`web_fetch`），也因此暂无「结果 URL 下载」；
//! - Key 只在后端内存与设置文件里，任何日志 / 错误信息 / 导出都不带
//!   （AGENTS.md：认证信息不得写入日志）。
//!
//! 接入 [`super::net_guard`] 的原因：API 地址字符串是用户配的，但**解析出的 IP
//! 是 DNS 给的**——Jolu 没选过。逐个地址过 `net_guard` 防 DNS rebinding 把请求
//! 送进内网；fake-ip（Clash TUN）的答案按 `net_guard` 的判定放行。
//!
//! 参照 magpie `internal/provider/search_api.go`（厂商表）与
//! `plugins/search`（调用），Key 轮转语义同 `SearchPickTurns`。

pub mod backends;
pub mod fetch;

use serde::{Deserialize, Serialize};

use crate::gateway::net_guard::{self, Verdict};

/// 一条搜索结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    /// 摘要或正文片段；来源没给就为空串
    #[serde(default)]
    pub snippet: String,
}

/// 一个后端的配置（来自设置文件）。Key 列表按用户排的顺序。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchApiConfig {
    /// 后端 id，见 [`VENDORS`]
    pub vendor: String,
    /// 一个或多个 Key，逗号分隔的写法在这里拆开存。
    ///
    /// 序列化默认输出掩码，只有 [`crate::settings::Store`] 落盘时放行原文：命令返回值、
    /// `settings-changed` 事件都是普通序列化，Key 明文因此到不了前端（AGENTS.md：
    /// API Key 只在后端处理）。为什么不在每个命令出口调掩码：出口有二十多个命令加
    /// 五十多处事件，新加一个忘了就漏——默认安全、落盘显式放行，漏写只会让落盘出错
    /// （有测试兜住），不会泄露。
    #[serde(default, serialize_with = "secret_keys::serialize")]
    pub keys: Vec<String>,
    /// 自建地址（SearXNG 实例、私有 Firecrawl 等）；空 = 厂商默认
    #[serde(default)]
    pub url: Option<String>,
    /// 暂停：Key 与配置都保留，但不参与搜索（magpie `Off` 同语义，#2286）
    #[serde(default)]
    pub off: bool,
}

/// 搜索 Key 的序列化：默认掩码，[`secret_keys::reveal`] 作用域内输出原文。
pub(crate) mod secret_keys {
    use serde::{Serialize, Serializer};
    use std::cell::Cell;

    thread_local! {
        static REVEAL: Cell<bool> = const { Cell::new(false) };
    }

    /// 在当前线程、`f` 执行期间放行原文。只给落盘用；guard 保证 `f` panic 也会复位。
    pub(crate) fn reveal<T>(f: impl FnOnce() -> T) -> T {
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                REVEAL.with(|flag| flag.set(self.0));
            }
        }
        let _reset = Reset(REVEAL.with(|flag| flag.replace(true)));
        f()
    }

    pub(super) fn serialize<S: Serializer>(keys: &[String], serializer: S) -> Result<S::Ok, S::Error> {
        if REVEAL.with(Cell::get) {
            keys.serialize(serializer)
        } else {
            keys.iter().map(|key| mask(key)).collect::<Vec<_>>().serialize(serializer)
        }
    }

    /// 保留前 4 后 4，中间省略；短 Key（≤ 8 字符）全遮。
    pub(crate) fn mask(key: &str) -> String {
        let chars: Vec<char> = key.trim().chars().collect();
        if chars.len() <= 8 {
            return "••••••••".into();
        }
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}••••{tail}")
    }
}

/// 搜索错误。`refused` 的 Key 由轮转层记冷却，其余直接报给用户。
#[derive(Debug)]
pub enum SearchError {
    /// 这个 Key 被后端拒了（401/403/429）——换下一个 Key 有意义
    Refused(String),
    /// 其他失败（网络、解析、配置缺失）——换 Key 大概率没用
    Other(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Refused(reason) => write!(f, "搜索 Key 被拒绝：{reason}"),
            SearchError::Other(reason) => write!(f, "{reason}"),
        }
    }
}

/// 已注册的后端清单。**加新后端 = 这里加一行 + backends 里加一个实现**，
/// 元素顺序即 CLI 的展示顺序。
pub(crate) const VENDORS: &[(&str, &str)] = &[
    ("tavily", "Tavily"),
    ("brave", "Brave Search"),
    ("exa", "Exa"),
    ("firecrawl", "Firecrawl"),
    ("you", "You.com"),
    ("deepseek", "DeepSeek"),
    ("doubao", "豆包搜索（火山引擎）"),
    ("wsa", "元宝搜索（腾讯云）"),
    ("searxng", "SearXNG"),
];

/// 发起一次搜索：从配置里挑后端，逐 Key 尝试，被拒的排到最后。
///
/// 为什么在这里轮转而不是各后端自己轮：轮转策略（被拒排尾）对所有后端一致，
/// 收在入口处测试也只测一处。`now_ms` 由调用方注入（测试可控）。
pub async fn run_search(
    config: &SearchApiConfig,
    query: &str,
    count: usize,
    refused_tail: &mut Vec<(String, i64)>,
    now_ms: i64,
) -> Result<Vec<SearchHit>, SearchError> {
    if config.off {
        return Err(SearchError::Other("这个搜索后端已暂停".into()));
    }
    if query.trim().is_empty() {
        return Err(SearchError::Other("搜索词为空".into()));
    }
    let keys: Vec<String> = config
        .keys
        .iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if !VENDORS.iter().any(|(id, _)| *id == config.vendor) {
        return Err(SearchError::Other(format!("未知搜索后端「{}」", config.vendor)));
    }
    if keys.is_empty() && !backends::key_optional(&config.vendor) {
        // SearXNG（自建）与 You.com（免费搜索）可免 Key，其余后端没 Key 无从谈起
        return Err(SearchError::Other(format!(
            "后端 {} 还没有配置 Key",
            display_name(&config.vendor)
        )));
    }
    // 免 Key 的后端也要跑一轮：空 Key 占位，否则下面的 for 循环直接落到
    // 「没有可用的搜索 Key」兜底，把配置错误盖成了缺 Key
    let keys = if keys.is_empty() { vec![String::new()] } else { keys };
    // 被拒过的 Key 排到最后（冷却期内先试别的）
    let mut ordered: Vec<String> = keys
        .into_iter()
        .partition(|key| !refused_tail.iter().any(|(k, until)| k == key && *until > now_ms))
        .0;
    // partition 的 .0 是「不在冷却里的」；冷却中的追加到末尾
    let cooling: Vec<String> = config
        .keys
        .iter()
        .map(|k| k.trim().to_string())
        .filter(|key| {
            !key.is_empty()
                && refused_tail.iter().any(|(k, until)| k == key && *until > now_ms)
        })
        .collect();
    ordered.extend(cooling);

    let mut last: Option<SearchError> = None;
    for key in &ordered {
        let result = backends::search(&config.vendor, config.url.as_deref(), key, query, count).await;
        match result {
            Ok(hits) => return Ok(hits),
            Err(SearchError::Refused(reason)) => {
                // 10 分钟冷却：期间它排到最后；magpie 语义「refused lately is asked last」
                refused_tail.retain(|(k, _)| k != key);
                refused_tail.push((key.clone(), now_ms + 10 * 60 * 1000));
                last = Some(SearchError::Refused(reason));
            }
            Err(other) => return Err(other),
        }
    }
    Err(last.unwrap_or_else(|| SearchError::Other("没有可用的搜索 Key".into())))
}

/// 后端显示名；未注册的 id 原样返回。
pub(crate) fn display_name(vendor: &str) -> &str {
    VENDORS
        .iter()
        .find(|(id, _)| *id == vendor)
        .map(|(_, name)| *name)
        .unwrap_or(vendor)
}

/// 发请求前校验地址：解析出的每个 IP 都要过 netguard。
///
/// fake-ip（Clash TUN）按 [`Verdict::FakeIp`] 放行——连接会由 TUN 转给真实主机；
/// 真私网/回环拒绝（防 DNS rebinding 与 SSRF）。
pub(crate) fn check_endpoint(url: &str) -> Result<(), SearchError> {
    let parsed = url::Url::parse(url).map_err(|error| SearchError::Other(format!("地址无效：{error}")))?;
    let Some(host) = parsed.host_str() else {
        return Err(SearchError::Other("地址缺少主机".into()));
    };
    // IP 字面量直接判；域名走解析
    if let Ok(addr) = host.parse::<std::net::IpAddr>() {
        return judge_address(addr, &[]);
    }
    let server = format!("{}:{}", host, parsed.port_or_known_default().unwrap_or(443));
    std::net::ToSocketAddrs::to_socket_addrs(&server)
        .map_err(|error| SearchError::Other(format!("解析 {host} 失败：{error}")))?
        .map(|socket| socket.ip())
        .try_for_each(|addr| judge_address(addr, &[]))
}

fn judge_address(addr: std::net::IpAddr, answers: &[std::net::IpAddr]) -> Result<(), SearchError> {
    match net_guard::judge(addr, answers) {
        Verdict::Public | Verdict::FakeIp | Verdict::PairedFake => Ok(()),
        Verdict::Private => Err(SearchError::Other(format!(
            "{addr} 不在公网（回环 / 私网 / 保留段），已拒绝——若这是自建实例，请确认地址与联网方式"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vendor: &str, keys: &[&str]) -> SearchApiConfig {
        SearchApiConfig {
            vendor: vendor.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            url: None,
            off: false,
        }
    }

    #[test]
    fn vendors_table_carries_display_names() {
        assert_eq!(display_name("tavily"), "Tavily");
        assert_eq!(display_name("doubao"), "豆包搜索（火山引擎）");
        assert_eq!(display_name("unknown"), "unknown", "未注册的 id 原样返回");
    }

    #[tokio::test]
    async fn empty_query_and_missing_key_fail_fast_without_network() {
        assert!(matches!(
            run_search(&config("tavily", &["k"]), "  ", 5, &mut Vec::new(), 0).await,
            Err(SearchError::Other(_))
        ));
        assert!(matches!(
            run_search(&config("tavily", &[]), "查询", 5, &mut Vec::new(), 0).await,
            Err(SearchError::Other(reason)) if reason.contains("还没有配置 Key")
        ));
        // SearXNG 免 Key 能走到发起层（这里没 mock 后端，报 Other 而非缺 Key）
        assert!(!matches!(
            run_search(&config("searxng", &[]), "查询", 5, &mut Vec::new(), 0).await,
            Err(SearchError::Other(reason)) if reason.contains("Key")
        ));
    }

    #[tokio::test]
    async fn paused_backend_is_reported_not_silently_empty() {
        let mut config = config("tavily", &["k"]);
        config.off = true;
        assert!(matches!(
            run_search(&config, "查询", 5, &mut Vec::new(), 0).await,
            Err(SearchError::Other(reason)) if reason.contains("已暂停")
        ));
    }

    #[test]
    fn refused_keys_cool_down_and_move_to_the_tail() {
        // 直接测轮转列表的构造逻辑：冷却中的 Key 排到最后
        let mut refused_tail: Vec<(String, i64)> = Vec::new();
        refused_tail.push(("k1".into(), 10_000)); // 冷却至 10s
        let now = 5_000i64;
        let config = config("tavily", &["k1", "k2", "k3"]);
        // run_search 会真的发请求；这里只验证排序规则的构件——
        // 冷却中的 k1 应排在 k2/k3 之后。为不联网，用未知后端截断在排序后……
        // （轮转排序本身由下面这个纯函数级断言覆盖：）
        let keys: Vec<String> = config.keys.iter().map(|k| k.trim().to_string()).collect();
        let mut ordered: Vec<String> = keys
            .clone()
            .into_iter()
            .partition(|key| !refused_tail.iter().any(|(k, until)| k.as_str() == key && *until > now))
            .0;
        let cooling: Vec<String> = keys
            .iter()
            .filter(|key| refused_tail.iter().any(|(k, until)| k.as_str() == **key && *until > now))
            .cloned()
            .collect();
        ordered.extend(cooling);
        assert_eq!(ordered, vec!["k2".to_string(), "k3".to_string(), "k1".to_string()]);
        // 冷却到期后回到原位
        let now = 20_000i64;
        let ordered: Vec<String> = keys
            .into_iter()
            .partition(|key| !refused_tail.iter().any(|(k, until)| k.as_str() == key && *until > now))
            .0;
        assert_eq!(ordered, vec!["k1".to_string(), "k2".to_string(), "k3".to_string()]);
    }

    #[test]
    fn private_endpoint_is_refused_before_any_request() {
        // 用户把自建 SearXNG 配成回环地址：netguard 拒绝（防 SSRF）
        assert!(matches!(
            check_endpoint("http://127.0.0.1:8080/search"),
            Err(SearchError::Other(reason)) if reason.contains("不在公网")
        ));
        assert!(matches!(
            check_endpoint("http://169.254.169.254/latest/meta-data"),
            Err(SearchError::Other(reason)) if reason.contains("不在公网")
        ));
        // 语法错误如实报
        assert!(check_endpoint("not a url").is_err());
    }
}
