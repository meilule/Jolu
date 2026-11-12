//! 额度查询的后端注册表（4.3，见 `docs/specs/2026-11-05-provider-quota-descriptor.md` §6.5 第 4 步）。
//!
//! 每个（平台, 接入方式）走哪条查询路线集中在 [`route`] 一处，`connection_quota` 只按结果分派，
//! 不再在命令里散写 `match (platform, kind)`。平台能力仍以前后端共用的 `platformCatalog.json`
//! 为准（`platforms.rs`），这里的一致性测试保证「目录声明了订阅额度的平台都有一条查询路线」。
//!
//! 浏览器授权的连接不在表里：它由凭据本身（`provider_browser_auth::is_browser`）决定，
//! 与平台无关，命令里先于本表判断。

use crate::quota::QuotaState;

/// 查询路线。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotaRoute {
    /// 原工具的本机来源，按来源 id 与身份摘要缓存（Trae / WorkBuddy / ZCode 账号来源）
    NativeSource,
    /// Qoder 本机来源（只有国内版能取到身份摘要，按来源 id 分别处理）
    QoderSource,
    /// ZCode 本机登录的普通连接（`cached_zcode_connection_quota`）
    ZcodeLocal,
    /// 只读原工具登录状态的本机查询，不缓存（见 [`local_login`]）
    LocalLogin,
    /// 保存凭据的官方订阅接口
    ClaudeOauth,
    CodexOauth,
    /// API Key 且配了服务地址：套餐域名走 Coding Plan，否则只向该地址查 sub2api
    ApiBaseUrl,
    /// 官方 API Key：没有订阅额度（界面改看组织用量或本机统计）
    ApiKeyNoPlan,
    /// 原工具来源平台的其他接入方式：按目录里的限制说明
    NativeUnsupported,
    /// 其余：暂未接入
    Unsupported,
}

/// 选查询路线。`connection_id` 只用于识别 ZCode 的账号来源连接；
/// `has_base_url` 是连接是否配了非空服务地址。
pub(crate) fn route(platform: &str, kind: &str, connection_id: &str, has_base_url: bool) -> QuotaRoute {
    if crate::platforms::uses_native_quota(platform, kind) {
        return match platform {
            "trae" | "workbuddy" => QuotaRoute::NativeSource,
            "qoder" => QuotaRoute::QoderSource,
            "zcode" if connection_id == ZCODE_ACCOUNT_SOURCE => QuotaRoute::NativeSource,
            "zcode" => QuotaRoute::ZcodeLocal,
            _ if local_login(platform).is_some() => QuotaRoute::LocalLogin,
            _ => QuotaRoute::NativeUnsupported,
        };
    }
    if kind == "api" && has_base_url {
        return QuotaRoute::ApiBaseUrl;
    }
    match (platform, kind) {
        ("claude", "auth") => QuotaRoute::ClaudeOauth,
        ("codex", "auth") => QuotaRoute::CodexOauth,
        ("claude" | "codex", "api") => QuotaRoute::ApiKeyNoPlan,
        _ => QuotaRoute::Unsupported,
    }
}

/// ZCode 账号来源连接的固定 id（`native_sources` 生成）。
pub(crate) const ZCODE_ACCOUNT_SOURCE: &str = "monitor:native:zcode:account";

/// `LocalLogin` 路线的查询函数（只读原工具的登录状态，不写回凭据）。
pub(crate) fn local_login(platform: &str) -> Option<fn() -> QuotaState> {
    match platform {
        "grok" => Some(crate::grok_quota::local_grok_quota),
        "gemini" => Some(crate::gemini_quota::local_gemini_quota),
        "opencode" => Some(crate::opencode::local_openai_quota),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目录（platformCatalog.json）声明了订阅额度的平台，订阅登录都要有一条查询路线；
    /// 新增平台时漏登记会在这里失败，而不是在界面上悄悄显示「暂未接入」。
    #[test]
    fn every_catalog_platform_with_subscription_quota_has_an_auth_route() {
        for platform in crate::platforms::ids() {
            if !crate::platforms::supports(&platform, "subscription_quota") || !crate::platforms::supports(&platform, "auth_connection") {
                continue;
            }
            let routed = route(&platform, "auth", "conn", false);
            assert!(
                !matches!(routed, QuotaRoute::Unsupported | QuotaRoute::NativeUnsupported),
                "{platform} 声明了订阅额度却没有查询路线"
            );
        }
    }

    #[test]
    fn routes_match_the_previous_dispatch() {
        assert_eq!(route("trae", "auth", "x", false), QuotaRoute::NativeSource);
        assert_eq!(route("workbuddy", "auth", "x", false), QuotaRoute::NativeSource);
        assert_eq!(route("qoder", "auth", "x", false), QuotaRoute::QoderSource);
        assert_eq!(route("zcode", "auth", ZCODE_ACCOUNT_SOURCE, false), QuotaRoute::NativeSource);
        assert_eq!(route("zcode", "auth", "monitor:native:zcode:main", false), QuotaRoute::ZcodeLocal);
        for platform in ["grok", "gemini", "opencode"] {
            assert_eq!(route(platform, "auth", "x", false), QuotaRoute::LocalLogin, "{platform}");
        }
        assert_eq!(route("claude", "auth", "x", false), QuotaRoute::ClaudeOauth);
        assert_eq!(route("codex", "auth", "x", false), QuotaRoute::CodexOauth);
        assert_eq!(route("claude", "api", "x", false), QuotaRoute::ApiKeyNoPlan);
        assert_eq!(route("claude", "api", "x", true), QuotaRoute::ApiBaseUrl, "配了地址的 API Key 先看套餐 / 网关");
        // 原工具来源平台的 API Key 不走本机来源（与 platforms.rs 既有断言同口径）
        assert_eq!(route("zcode", "api", "x", true), QuotaRoute::ApiBaseUrl);
        assert_eq!(route("custom", "api", "x", true), QuotaRoute::ApiBaseUrl);
        assert_eq!(route("custom", "api", "x", false), QuotaRoute::Unsupported);
    }
}
