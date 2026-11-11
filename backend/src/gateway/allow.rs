//! 来源地址白名单（P2-9，对齐 magpie `internal/gateway/allow.go`）：局域网共享时，只有列表里的
//! 地址能连上网关，其余在看 Key 之前就 403——Key 泄露了，在别处也用不了。
//!
//! 管什么：
//! - 列表是 IP 或 CIDR 段（`192.168.1.20`、`10.0.0.0/8`、`fd00::/8`），逗号 / 空白 / 换行分隔；
//! - `JOLU_ALLOW_IPS` 环境变量优先于设置（容器、无界面部署用），设了就以它为准；
//! - 本机回环永远放行；列表为空 = 不限（与以前一样）；
//! - 判的是 TCP 连接的对端地址，**不信** X-Forwarded-For 之类的转发头——谁都能写。
//!
//! 不管什么：只在局域网共享时有意义；不共享时网关只监听回环，本来就连不进来。

use std::net::IpAddr;
use std::sync::{LazyLock, RwLock};

/// 环境变量名（与 magpie 的 `MAGPIE_ALLOW_IPS` 同义）。
pub const ENV: &str = "JOLU_ALLOW_IPS";

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    network: IpAddr,
    prefix: u8,
}

impl Rule {
    fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - self.prefix) };
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 { 0 } else { u128::MAX << (128 - self.prefix) };
                u128::from(net) & mask == u128::from(ip) & mask
            }
            // IPv4 映射成 IPv6 的地址（::ffff:a.b.c.d）按 IPv4 判
            (IpAddr::V4(_), IpAddr::V6(ip)) => ip.to_ipv4_mapped().is_some_and(|v4| self.contains(IpAddr::V4(v4))),
            _ => false,
        }
    }
}

/// 拆分并解析列表；有一项写错就整体报错并说出是哪一项（不悄悄忽略——忽略等于放宽）。
pub fn parse(text: &str) -> Result<Vec<Rule>, String> {
    text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|item| !item.is_empty())
        .map(|item| {
            let (address, prefix) = match item.split_once('/') {
                Some((address, prefix)) => (address, Some(prefix)),
                None => (item, None),
            };
            let network: IpAddr = address.parse().map_err(|_| format!("「{item}」不是 IP 地址或 CIDR 段"))?;
            let max = if network.is_ipv4() { 32 } else { 128 };
            let prefix = match prefix {
                Some(text) => text.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("「{item}」的前缀长度无效"))?,
                None => max,
            };
            Ok(Rule { network, prefix })
        })
        .collect()
}

/// 生效中的列表：网关启动时按环境变量 / 设置装入；保存设置时更新。
static ACTIVE: LazyLock<RwLock<Vec<Rule>>> = LazyLock::new(Default::default);

/// 装入列表：环境变量设了就用它，否则用设置里的。返回生效的条数与来源（给日志）。
pub fn load(settings_list: &[String]) -> Result<(usize, &'static str), String> {
    let (rules, source) = match std::env::var(ENV).ok().filter(|v| !v.trim().is_empty()) {
        Some(value) => (parse(&value)?, "环境变量 JOLU_ALLOW_IPS"),
        None => (parse(&settings_list.join(","))?, "设置"),
    };
    let count = rules.len();
    if let Ok(mut active) = ACTIVE.write() {
        *active = rules;
    }
    Ok((count, source))
}

/// 环境变量是否接管了列表（界面据此提示「设置里改了不生效」）。
pub fn from_env() -> bool {
    std::env::var(ENV).ok().is_some_and(|v| !v.trim().is_empty())
}

/// 这个对端地址能不能进来：回环永远可以；列表为空不限；否则要在列表里。
pub fn allowed(ip: IpAddr) -> bool {
    if ip.is_loopback() || matches!(ip, IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())) {
        return true;
    }
    let Ok(rules) = ACTIVE.read() else { return false };
    rules.is_empty() || rules.iter().any(|rule| rule.contains(ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ips_and_ranges_match_and_bad_entries_are_named() {
        let rules = parse("192.168.1.20, 10.0.0.0/8\nfd00::/8").unwrap();
        let matches = |ip: &str| rules.iter().any(|rule| rule.contains(ip.parse().unwrap()));
        assert!(matches("192.168.1.20"));
        assert!(!matches("192.168.1.21"));
        assert!(matches("10.200.3.4"));
        assert!(matches("fd12::1"));
        assert!(matches("::ffff:10.1.1.1"), "IPv4 映射地址按 IPv4 判");
        assert!(!matches("fe80::1"));
        assert!(parse("10.0.0.0/33").unwrap_err().contains("前缀"));
        assert!(parse("example.com").unwrap_err().contains("example.com"));
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn loopback_always_and_empty_list_lets_everyone_in() {
        load(&[]).unwrap();
        assert!(allowed("192.168.5.5".parse().unwrap()), "没有列表不限");
        load(&["192.168.1.0/24".to_string()]).unwrap();
        assert!(allowed("192.168.1.9".parse().unwrap()));
        assert!(!allowed("192.168.5.5".parse().unwrap()));
        assert!(allowed("127.0.0.1".parse().unwrap()) && allowed("::1".parse().unwrap()), "本机永远放行");
        load(&[]).unwrap();
    }
}
