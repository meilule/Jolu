//! 地址安全校验（netguard）：Jolu 没选过的 URL，解析出的地址能不能连。
//!
//! 管什么：网关替别人发的下载——搜索结果的页面抓取（未来的 `web_fetch`）、
//! 厂商图片、搜索 API 解析后的地址。**只判地址，不做决定**：拒绝、放行还是
//! 交给代理，由调用方决定。
//!
//! 不管什么：
//! - 用户在 Jolu 里显式配置的上游 base_url 不在这里拦（那是联网方式与代理的职责）；
//! - 不做 DNS 解析——调用方拿到解析结果后逐个地址来问（防 DNS rebinding 靠
//!   「每个答案都过一遍」而不是这里代查）。
//!
//! 关键取舍：fake-ip（Clash TUN 增强模式）的答案是**假 IP 但合法**，一律拒绝会
//! 把 Clash Verge 用户全部拦死（magpie 现场：jiezhengj，11-08，TUN 下所有
//! web_fetch 全拒）。所以 fake-ip 的 A 记录（198.18.0.0/15）单独放行判断，
//! 它旁边的 AAAA（mihomo 的 fake-ip-range6，默认 `fdfe:dcba:9876::1/64`，
//! 属 fc00::/7；或 2001:2::/48）**不单独拨号**，用同一答案里配对的 fake A
//! 代替判定——见 [`fake_paired`]。
//!
//! 参照 magpie `internal/netguard/netguard.go`（97 行；IPv6 fake-ip 修正
//! commit `c6bdc17`）。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// 需要拒绝的「特殊用途」段。每个都标了它是干什么的——加新段前先看
/// IANA 的特殊用途注册表，别凭感觉加。
/// 参照 magpie `netguard.go::specialNets`。
const SPECIAL_V4: &[(Ipv4Addr, u8, &str)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), 8, "本网络"),
    (Ipv4Addr::new(100, 64, 0, 0), 10, "CGNAT、Tailscale"),
    (Ipv4Addr::new(192, 0, 0, 0), 24, "IETF 协议分配"),
    (Ipv4Addr::new(192, 0, 2, 0), 24, "文档示例（TEST-NET-1）"),
    (Ipv4Addr::new(198, 18, 0, 0), 15, "基准测试段；Clash 等 fake-ip 解析器用它（FakeIP 单独放行判断）"),
    (Ipv4Addr::new(198, 51, 100, 0), 24, "文档示例（TEST-NET-2）"),
    (Ipv4Addr::new(203, 0, 113, 0), 24, "文档示例（TEST-NET-3）"),
    (Ipv4Addr::new(240, 0, 0, 0), 4, "保留、广播"),
];

const SPECIAL_V6: &[(Ipv6Addr, u8, &str)] = &[
    (Ipv6Addr::new(0x100, 0, 0, 0, 0, 0, 0, 0), 64, "丢弃段（discard-only）"),
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 32, "Teredo"),
    (Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32, "文档示例"),
    (Ipv6Addr::new(0x2001, 0x0002, 0, 0, 0, 0, 0, 0), 48, "基准测试（RFC 5180）；fake-ip 解析器的 AAAA（FakePaired 单独判断）"),
    (Ipv6Addr::new(0xfec0, 0, 0, 0, 0, 0, 0, 0), 10, "站点本地（已废弃）"),
    (Ipv6Addr::new(0x0064, 0xff9b, 0x0001, 0, 0, 0, 0, 0), 48, "本地用途 NAT64"),
];

const NAT64: (Ipv6Addr, u8) = (Ipv6Addr::new(0x0064, 0xff9b, 0, 0, 0, 0, 0, 0), 96);
const FAKE_NET: (Ipv4Addr, u8) = (Ipv4Addr::new(198, 18, 0, 0), 15);
/// Loon 给 fake A 配对合成的 AAAA 段。
const LOON_NET: (Ipv6Addr, u8) = (Ipv6Addr::new(0xfd27, 0x0712, 0, 0, 0, 0, 0, 0), 96);
const ULA: (Ipv6Addr, u8) = (Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7);

/// IPv4 的 `bits` 位前缀包含判定：用 u32 位宽——把 v4 当 u128 的低 32 位比短前缀
/// 时，掩码会跑到 v4 位之外（`0 == 0` 恒真），8.8.8.8 会被误判成任何段。
fn v4_prefix_contains(addr: Ipv4Addr, prefix: (Ipv4Addr, u8)) -> bool {
    let bits = prefix.1;
    if bits == 0 {
        return true;
    }
    let mask = u32::MAX << (32 - bits as u32);
    (u32::from(addr) & mask) == (u32::from(prefix.0) & mask)
}

fn v6_prefix_contains(addr: Ipv6Addr, prefix: (Ipv6Addr, u8)) -> bool {
    let bits = prefix.1;
    if bits == 0 {
        return true;
    }
    let mask = u128::MAX << (128 - bits as u32);
    (u128::from_be_bytes(addr.octets()) & mask) == (u128::from_be_bytes(prefix.0.octets()) & mask)
}

/// 地址是否在公网：不是回环、RFC 1918、fc00::/7、链路本地（含云元数据
/// 169.254.169.254）、CGNAT、未指定、组播、文档、保留、基准测试、Teredo，
/// 也不是 IPv4-mapped、NAT64 或 6to4 形式的上述地址。
///
/// fake-ip 解析器的答案**不是**公网——它们走 [`fake_ip`] / [`fake_paired`]
/// 单独判断，调用方决定放行策略。
pub(crate) fn is_public(addr: IpAddr) -> bool {
    // ::ffff:127.0.0.1 就是 127.0.0.1：先解包 IPv4-mapped
    let addr = match addr {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
    };
    match addr {
        IpAddr::V4(v4) => {
            if v4.is_loopback() || v4.is_private() || v4.is_unspecified() || v4.is_link_local() || v4.is_broadcast() {
                return false;
            }
            // 组播 224.0.0.0/4 std 没有 V4 的稳定 is_multicast？有：Ipv4Addr::is_multicast 稳定
            if v4.is_multicast() {
                return false;
            }
            !SPECIAL_V4.iter().any(|(network, bits, _)| v4_prefix_contains(v4, (*network, *bits)))
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return false;
            }
            // 手写 std 不稳定的判定：fc00::/7 唯一本地、fe80::/10 链路本地、ff00::/8 组播
            if v6_prefix_contains(v6, ULA)
                || v6_prefix_contains(v6, (Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10))
                || v6_prefix_contains(v6, (Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8))
            {
                return false;
            }
            if SPECIAL_V6.iter().any(|(network, bits, _)| v6_prefix_contains(v6, (*network, *bits))) {
                return false;
            }
            // IPv4 藏在 IPv6 里：NAT64（64:ff9b::/96）与 6to4（2002::/16）都通到内嵌的 v4
            if v6_prefix_contains(v6, NAT64) {
                let bytes = v6.octets();
                let v4 = Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]);
                return is_public(IpAddr::V4(v4));
            }
            let bytes = v6.octets();
            if bytes[0] == 0x20 && bytes[1] == 0x02 {
                return is_public(IpAddr::V4(Ipv4Addr::new(bytes[2], bytes[3], bytes[4], bytes[5])));
            }
            true
        }
    }
}

/// fake-ip 的 A 记录：198.18.0.0/15（RFC 2544 基准测试段）。
///
/// Clash 的 TUN fake-ip、Clash Verge、Surge 增强模式与 Loon 在这里应答每个域名，
/// 并把连接送到域名对应的真实主机（magpie #252、#2593）。用户网络上没有机器坐在这个段，
/// 所以连过去要么到达真实主机、要么什么都没有；但到底是哪台，由 fake-ip 解析器决定，
/// Jolu 看不见。调用方据此选择「放行连接」或「要求用户改真实解析」。
pub(crate) fn fake_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4_prefix_contains(v4, FAKE_NET),
        // IPv4-mapped 形式先解包
        IpAddr::V6(v6) => v6.to_ipv4_mapped().is_some_and(|v4| v4_prefix_contains(v4, FAKE_NET)),
    }
}

/// fake-ip 解析器在 fake A 旁边给的 AAAA：fc00::/7 或 2001:2::/48 里的地址，
/// 且 `answers` 里有一个落在 198.18.0.0/15 的 IPv4。这样的 a **不拨号**：
/// 用配对的 fake A 代替判定（它连到什么，fake A 就连到什么）。
/// 没有配对 fake A 的 ULA，照旧是私网。
///
/// 为什么需要它：mihomo 在 IPv6 开启时从 fake-ip-range6 应答 AAAA（Clash Verge Rev
/// 默认开 IPv6，填 `fdfe:dcba:9876::1/64`），没有这条放行，Clash Verge Rev 的
/// TUN 用户所有抓取全被拒。
pub(crate) fn fake_paired(addr: IpAddr, answers: &[IpAddr]) -> bool {
    let IpAddr::V6(v6) = addr else { return false };
    if !v6_prefix_contains(v6, ULA)
        && !v6_prefix_contains(v6, (Ipv6Addr::new(0x2001, 0x0002, 0, 0, 0, 0, 0, 0), 48))
    {
        return false;
    }
    answers.iter().any(|other| fake_ip(*other))
}

/// Loon 给 fake A 合成的 AAAA：fd27:712::/96 内嵌 IPv4，且该 IPv4 在 `answers` 里。
/// 这样的 a 代表那个 IPv4 地址（调用方自己判它），a 本身不拨号。
pub(crate) fn loon_paired(addr: IpAddr, answers: &[IpAddr]) -> bool {
    let IpAddr::V6(v6) = addr else { return false };
    if !v6_prefix_contains(v6, LOON_NET) {
        return false;
    }
    let bytes = v6.octets();
    let v4 = Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]);
    answers.iter().any(|other| matches!(other, IpAddr::V4(o) if *o == v4))
}

/// 一个地址的完整判定结果，调用方按此决定连不连。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// 公网地址：直接连
    Public,
    /// fake-ip 的 A 记录：连接会由 TUN 转给真实主机，放行与否由调用方策略决定
    FakeIp,
    /// fake-ip 旁的 AAAA / Loon 配对 AAAA：不拨这个，用配对的 fake A 判定
    PairedFake,
    /// 其余一切（回环、私网、保留段……）：拒绝
    Private,
}

/// 对一个地址给出判定。`answers` 是同一次 DNS 查询的其他答案（配对判定用）。
pub(crate) fn judge(addr: IpAddr, answers: &[IpAddr]) -> Verdict {
    if is_public(addr) {
        return Verdict::Public;
    }
    if fake_ip(addr) {
        return Verdict::FakeIp;
    }
    if fake_paired(addr, answers) || loon_paired(addr, answers) {
        return Verdict::PairedFake;
    }
    Verdict::Private
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn v4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    fn v6(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses_pass_and_private_fail() {
        for public in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "2001:4860:4860::8888"] {
            assert_eq!(judge(public.parse().unwrap(), &[]), Verdict::Public, "{public} 应是公网");
        }
        for private in [
            "127.0.0.1", "10.0.0.5", "172.16.0.9", "192.168.1.1", "169.254.169.254", "0.0.0.0",
            "100.64.0.10", "240.0.0.1", "224.0.0.1", "::1", "fe80::1", "fc00::1", "fd12:3456::1",
            "fdfe:dcba:9876::1", "2001:db8::1", "64:ff9b:1::10",
        ] {
            assert_eq!(judge(private.parse().unwrap(), &[]), Verdict::Private, "{private} 应拒");
        }
    }

    #[test]
    fn ipv4_mapped_and_nat64_and_6to4_unwrap_to_the_inner_verdict() {
        // ::ffff:127.0.0.1 就是 127.0.0.1
        assert_eq!(judge(v6("::ffff:127.0.0.1"), &[]), Verdict::Private);
        // ::ffff:8.8.8.8 是公网
        assert_eq!(judge(v6("::ffff:8.8.8.8"), &[]), Verdict::Public);
        // NAT64 64:ff9b::7f00:1 → 内嵌 127.0.0.1，拒
        assert_eq!(judge(v6("64:ff9b::127.0.0.1"), &[]), Verdict::Private);
        // NAT64 内嵌公网 → 公网
        assert_eq!(judge(v6("64:ff9b::808:808"), &[]), Verdict::Public);
        // 6to4 2002::7f00:1 → 内嵌 127.0.0.1，拒
        assert_eq!(judge(v6("2002::127.0.0.1"), &[]), Verdict::Private);
    }

    #[test]
    fn fake_ip_a_records_are_reported_not_refused() {
        assert_eq!(judge(v4("198.18.0.7"), &[]), Verdict::FakeIp);
        assert_eq!(judge(v4("198.19.255.1"), &[]), Verdict::FakeIp, "198.18/15 整段");
        // 段外紧邻地址是普通公网（198.18/15 只覆盖 198.18.0.0–198.19.255.255）
        assert_eq!(judge(v4("198.17.255.255"), &[]), Verdict::Public);
        assert_eq!(judge(v4("198.20.0.1"), &[]), Verdict::Public);
        // IPv4-mapped 的 fake-ip 也认
        assert_eq!(judge(v6("::ffff:198.18.0.7"), &[]), Verdict::FakeIp);
    }

    #[test]
    fn fake_ip_aaaa_is_paired_with_a_fake_a_in_the_same_answer() {
        let answers = [v4("198.18.0.7")];
        // mihomo fake-ip-range6 默认段（Clash Verge Rev）
        assert_eq!(judge(v6("fdfe:dcba:9876::1"), &answers), Verdict::PairedFake);
        // RFC 5180 基准测试段
        assert_eq!(judge(v6("2001:2::9"), &answers), Verdict::PairedFake);
        // 没有配对 fake A 的 ULA：照旧拒
        assert_eq!(judge(v6("fdfe:dcba:9876::1"), &[]), Verdict::Private);
        // 配对答案里是真实私网而不是 fake A：不配对，拒
        assert_eq!(judge(v6("fdfe:dcba:9876::1"), &[v4("192.168.1.1")]), Verdict::Private);
    }

    #[test]
    fn loon_paired_aaaa_is_recognized() {
        // fd27:712:: 内嵌 198.18.0.7（前 96 位是段前缀，后 32 位是 IPv4）
        let loon = v6("fd27:712::198.18.0.7");
        assert_eq!(judge(loon, &[v4("198.18.0.7")]), Verdict::PairedFake);
        // 内嵌的不是 fake A：Loon 配对也不成立，拒
        assert_eq!(judge(loon, &[v4("8.8.8.8")]), Verdict::Private);
    }

    #[test]
    fn every_special_v4_entry_has_a_reason_label() {
        // 注释规范：每个段都标它是什么——表里任何一条没有标签说明都是漏登记
        for (_, _, label) in SPECIAL_V4 {
            assert!(!label.trim().is_empty());
        }
        for (_, _, label) in SPECIAL_V6 {
            assert!(!label.trim().is_empty());
        }
    }
}
