//! 额度查询的路由决策（从 `lib.rs::connection_quota` 抽出）。
//!
//! `connection_quota` 命令本身有大量副作用（缓存、发事件、更新托盘），不适合整体搬进
//! service 层。真正值得抽、也最容易出错的是**「这个连接该走哪条查询链」**这个判断：
//! 它决定了用户看到的是配额、还是「不支持」、还是「凭证失效」。
//!
//! 这个决策以前埋在 100 多行命令体里，靠顺序与 `return` 表达，抽出来后可以逐条测。
//! 判断顺序必须与命令侧保持一致，改这里等于改命令的行为。

use crate::connections::Creds;

/// 一个连接该走哪条额度查询链。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaRoute {
    /// 浏览器授权账号（WorkBuddy / Qoder / Trae 等通过浏览器登录的连接）
    BrowserAuth,
    /// 原工具登录的本机来源，按平台分派
    NativeLocal(NativeSource),
    /// 保存了凭证的普通连接：有 `base_url` 时按域名识别套餐供应商或自建网关
    SavedCredential,
    /// 该平台没有接入官方额度查询
    Unsupported(String),
}

/// 本机来源的细分。与命令侧原先的 `match` 分支一一对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeSource {
    /// Qoder 桌面端（区分国内源与其他源）
    Qoder { cn: bool },
    /// Trae
    Trae,
    /// WorkBuddy
    WorkBuddy,
    /// ZCode 账号级（`monitor:native:zcode:account`）
    ZcodeAccount,
    /// ZCode 本机登录（与上面的账号级是两套缓存语义）
    ZcodeLocal,
    /// 其余按平台查本机登录（grok / gemini / opencode）
    Platform,
}

/// 决定一个连接走哪条链。`connection_id` 参与两处特例判断
/// （`monitor:native:zcode:account` 与 qoder 国内源），因此一并传入。
///
/// 顺序与 `connection_quota` 原有分支严格一致：
/// 1. 浏览器授权 → 2. 本机来源（含 zcode 两级） → 3. 有凭证的普通连接 → 4. 不支持。
pub fn route_for(connection_id: &str, creds: &Creds) -> QuotaRoute {
    // 1. 浏览器授权账号：凭证在浏览器侧的加密存储里，不在连接表
    if creds.kind == "auth" && crate::provider_browser_auth::is_browser(creds.secret.as_deref()) {
        return QuotaRoute::BrowserAuth;
    }
    // 2. 原工具登录走本机来源；自定义及各平台 API 连接按其实际地址查询
    if crate::platforms::uses_native_quota(&creds.platform, &creds.kind) {
        if creds.kind == "auth" {
            let special = matches!(creds.platform.as_str(), "trae" | "workbuddy" | "qoder")
                || connection_id == "monitor:native:zcode:account";
            if special {
                return QuotaRoute::NativeLocal(match creds.platform.as_str() {
                    "qoder" => NativeSource::Qoder {
                        cn: crate::native_sources::monitor_id(connection_id).as_deref() == Some("native:qoder:cn"),
                    },
                    "trae" => NativeSource::Trae,
                    "workbuddy" => NativeSource::WorkBuddy,
                    // 走到这里是 `monitor:native:zcode:account`
                    _ => NativeSource::ZcodeAccount,
                });
            }
            // ZCode 本机登录（`monitor:native:zcode:main` 等普通连接）
            if creds.platform == "zcode" {
                return QuotaRoute::NativeLocal(NativeSource::ZcodeLocal);
            }
            // 其余本机登录的订阅账号
            return QuotaRoute::NativeLocal(NativeSource::Platform);
        }
        // API Key 连接不查官方额度端点
        return QuotaRoute::Unsupported(crate::platforms::limitation(&creds.platform));
    }
    // 3. 有凭证的普通连接
    if creds.secret.as_ref().is_some_and(|secret| !secret.trim().is_empty()) {
        return QuotaRoute::SavedCredential;
    }
    // 4. 没凭证：命令侧会报「凭证未保存」，这里单独成一类便于 CLI 给同样的提示
    QuotaRoute::Unsupported("该连接尚未保存凭证".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(platform: &str, kind: &str, secret: Option<&str>) -> Creds {
        Creds {
            paused: false,
            name: "测试".into(),
            platform: platform.into(),
            kind: kind.into(),
            secret: secret.map(str::to_string),
            base_url: None,
            model: None,
        }
    }

    #[test]
    fn browser_auth_wins_over_everything_else() {
        let c = creds("workbuddy", "auth", Some("jolu-browser:v1:sealed"));
        assert_eq!(route_for("conn-1", &c), QuotaRoute::BrowserAuth);
    }

    #[test]
    fn zcode_account_and_local_are_different_routes() {
        let account = creds("zcode", "auth", Some("token"));
        assert_eq!(
            route_for("monitor:native:zcode:account", &account),
            QuotaRoute::NativeLocal(NativeSource::ZcodeAccount),
            "账号级连接有独立缓存语义"
        );
        assert_eq!(
            route_for("monitor:native:zcode:main", &account),
            QuotaRoute::NativeLocal(NativeSource::ZcodeLocal),
            "普通本机登录走另一套缓存"
        );
    }

    #[test]
    fn qoder_cn_is_distinguished_from_other_sources() {
        let qoder = creds("qoder", "auth", Some("token"));
        assert_eq!(
            route_for("monitor:native:qoder:cn", &qoder),
            QuotaRoute::NativeLocal(NativeSource::Qoder { cn: true })
        );
        assert_eq!(
            route_for("monitor:native:qoder:global", &qoder),
            QuotaRoute::NativeLocal(NativeSource::Qoder { cn: false })
        );
    }

    #[test]
    fn trae_and_workbuddy_have_their_own_sources() {
        assert_eq!(
            route_for("c", &creds("trae", "auth", Some("t"))),
            QuotaRoute::NativeLocal(NativeSource::Trae)
        );
        assert_eq!(
            route_for("c", &creds("workbuddy", "auth", Some("t"))),
            QuotaRoute::NativeLocal(NativeSource::WorkBuddy)
        );
    }

    #[test]
    fn other_native_logins_fall_to_platform_dispatch() {
        for platform in ["grok", "gemini", "opencode"] {
            assert_eq!(
                route_for("c", &creds(platform, "auth", Some("t"))),
                QuotaRoute::NativeLocal(NativeSource::Platform),
                "{platform} 应按平台分派"
            );
        }
    }

    #[test]
    fn api_key_never_takes_the_native_local_route() {
        // `uses_native_quota` 要求 kind == "auth"：API Key 连接不查官方订阅额度，
        // 一律落到普通凭证链，由那条链按 platform+kind 决定给「不支持」还是组织用量。
        for platform in ["zcode", "opencode", "gemini", "workbuddy", "trae", "qoder"] {
            assert_eq!(
                route_for("c", &creds(platform, "api", Some("key"))),
                QuotaRoute::SavedCredential,
                "{platform} 的 API Key 应走普通凭证链"
            );
        }
    }

    #[test]
    fn native_auth_without_saved_secret_is_routed_by_platform_not_rejected_early() {
        // 本机登录的 auth 连接即使连接表里没有 secret，也走本机来源分支
        // （凭证由原工具自己管理），不能提前判成「未保存凭证」
        assert_eq!(
            route_for("monitor:native:zcode:main", &creds("zcode", "auth", None)),
            QuotaRoute::NativeLocal(NativeSource::ZcodeLocal)
        );
    }

    #[test]
    fn claude_api_key_goes_through_the_saved_credential_path() {
        // claude 不是 native 平台（它有官方组织用量接口），所以 API Key 走普通凭证链，
        // 由那条链按 platform+kind 决定给「不支持」还是组织用量
        assert_eq!(
            route_for("c", &creds("claude", "api", Some("sk-ant-xxx"))),
            QuotaRoute::SavedCredential
        );
    }

    #[test]
    fn saved_credential_requires_a_non_empty_secret() {
        assert_eq!(
            route_for("c", &creds("custom", "api", Some("sk-xxx"))),
            QuotaRoute::SavedCredential
        );
        // 只有空白也算没保存
        assert!(matches!(
            route_for("c", &creds("custom", "api", Some("   "))),
            QuotaRoute::Unsupported(_)
        ));
        assert!(matches!(
            route_for("c", &creds("custom", "api", None)),
            QuotaRoute::Unsupported(_)
        ));
    }
}
