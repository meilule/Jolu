//! Agent 接线注册表。每家工具在这里声明路径、状态判断、接入和断开处理。
//! 公共编排层负责备份、写入和接线记录，各工具适配器负责生成各自的配置改动。
//!
//! ## 一个必须保留的区分：两套「是不是我们的」谓词
//!
//! - **状态判定**（`Wired::base_only` 等）用 `points_at_gateway`：**精确到本机端口**。
//!   用户把配置指到别的本地服务时不该显示「已接入本网关」。
//! - **还原守卫**（各 `plan_unwire` 分支）用 `is_gateway_value`：**任意 127.0.0.1 / localhost**。
//!   用户换过端口、或旧版本写的地址，也要能还原。
//!
//! 合并这两个谓词会造成：变严 → 换端口后还原不掉；变松 → 误删用户指向其它本地服务的配置。

use std::path::{Path, PathBuf};

/// 令牌归属的判定宽严（各家现状不同，必须逐家显式声明，不能统一）。
///
/// 为什么不能统一：变严 → 老实例写的配置显示「未接入」；变松 → 两个 Jolu 实例互认，
/// 多实例保护失效（`foreign_marker` 永远返回 None）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenMatch {
    /// 精确等于 `token_for(agent, instance)`（或接线记录里保留的调用方 Key）。
    /// 用于 pencil / qoder / qoder-cn / kimi。
    Exact,
    /// 精确匹配，或任意 `jolu-` 前缀（升级兼容：早期版本的令牌不带实例后缀）。
    /// 用于 pi / dsh。
    Prefixed,
    /// claude-code 特例：还要接受历史字面量 `jolu-claude-code`（无实例后缀）。
    LegacyLiteral,
    /// 完全不看令牌，只看地址（codex / gemini / opencode / zcode）。
    Ignored,
}

/// 状态判定「还要额外满足什么」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WiredExtra {
    /// 只要地址对就算（codex / opencode / zcode）。
    None,
    /// Gemini 的 settings.json 必须选择网关使用的 API Key 登录。
    GeminiApiKey,
    /// 还要 `default_model` 指向我们的模型表（kimi：否则流量不经过我们）。
    KimiDefaultModel,
    /// 还要 `defaultProvider == "jolu"`（pi：设置在另一个文件里）。
    PiDefaultProvider,
    /// 还要工具当前选中的是我们的模型（pencil / qoder / qoder-cn：看 `providers.jolu` 或 `model.name`）。
    SelectionOurs,
    /// 由 adapter 自己判断（dsh：需要读 `.env` 与多 profile patch，逻辑在 `wiring_dsh::wired`）。
    Adapter,
    /// 还要 Antigravity 的启动脚本仍带着本机网关地址与令牌（地址/Key 只在环境变量里）。
    AntigravityModel,
}

/// 一家工具的接线声明。
pub(crate) struct AgentSpec {
    pub id: &'static str,
    pub wire: fn(&crate::gateway::wiring::WireContext, &str) -> Result<crate::gateway::wiring::adapters::WireEdits, String>,
    pub unwire: fn(&str, &crate::gateway::wiring::adapters::UnwireInput) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String>,
    /// 主文件缺失时仍可还原其他文件。
    pub unwire_without_main: bool,
    /// 成功写入还原结果后执行的兼容清理。
    pub after_unwire: Option<fn(&Path)>,
    /// 配置路径。`home` 由编排层解析一次传入（避免每家各自调 `creds::home()`，
    /// gemini 曾经主文件与 extra 文件落在不同 home）。
    pub path: fn(&Path) -> Option<PathBuf>,
    /// 从主文件文本里读当前 base URL
    pub base: fn(&str) -> Option<String>,
    /// 读取当前令牌，dsh 从 home 下的 .env 读取。
    pub token: fn(&Path, &str) -> Option<String>,
    pub token_match: TokenMatch,
    pub extra: WiredExtra,
    pub notice: &'static str,
    /// 该工具写进配置的模型清单是否来自网关目录（zcode / kimi / dsh / pi / extra 五组）
    pub uses_catalog: bool,
    /// User-Agent 前缀（小写比较），用来在**没带 Jolu 令牌**时认出请求来自哪个工具。
    ///
    /// 为什么需要：接线写进对方配置的是 `jolu-<agent>.<实例>`，但工具不一定把它放在
    /// `Authorization`/`x-api-key` 里（有的走自己的登录态、有的改写头）。本机实测
    /// `gateway_route_log` 里有 103 条 `agent` 为 NULL，其中 ZCode 42 条、WorkBuddy 53 条
    /// —— 全是「接线成功了但流水认不出是谁」。UA 是不需要配置就会带的信号。
    ///
    /// 取值来自 magpie 各家的 `UA:` 声明（`internal/agent/*.go`）。空数组表示 magpie
    /// 也没给这家声明 UA（Pi、Qoder），那它只能靠令牌前缀识别，这里不猜。
    pub ua: &'static [&'static str],
}

impl AgentSpec {
    /// 状态判定：地址精确指向本机端口 + 令牌归属 + 额外条件。
    ///
    /// `text` 是主文件文本；`kept` 是接线记录里保留的调用方 Key（claude 的 keyHolds）；
    /// `home` 供需要读第二个文件或自己判断的实现使用（pi 读 settings.json、dsh 读 .env）。
    pub(crate) fn is_wired(&self, home: &Path, text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
        // dsh 必须**先**短路：它把令牌放在 `$DSH_HOME/.env` 而不是主文件里，所以
        // adapter 自己检查所有条件，包括读取 .env 和多个 profile。
        // `WiredExtra::Adapter` 的语义就是「整个判定交给 adapter 自己」，包括读盘取令牌。
        if self.extra == WiredExtra::Adapter {
            return crate::gateway::wiring_dsh::wired(home, text, port, instance_short, kept);
        }
        if self.extra == WiredExtra::AntigravityModel {
            return crate::gateway::wiring_antigravity::wired(home, text, port, instance_short, kept);
        }
        let base = (self.base)(text).unwrap_or_default();
        if !crate::gateway::wiring::points_at_gateway(&base, port) {
            return false;
        }
        let token = (self.token)(home, text).unwrap_or_default();
        let token_ok = match self.token_match {
            TokenMatch::Ignored => true,
            TokenMatch::Exact => token == crate::gateway::wiring::token_for(self.id, instance_short)
                || kept.is_some_and(|kept| !kept.is_empty() && token == kept),
            TokenMatch::Prefixed => token == crate::gateway::wiring::token_for(self.id, instance_short)
                || token.starts_with("jolu-")
                || kept.is_some_and(|kept| !kept.is_empty() && token == kept),
            TokenMatch::LegacyLiteral => {
                token == "jolu-claude-code"
                    || token == crate::gateway::wiring::token_for(self.id, instance_short)
                    || kept.is_some_and(|kept| !kept.is_empty() && token == kept)
            }
        };
        if !token_ok {
            return false;
        }
        match self.extra {
            // 上面已短路
            WiredExtra::Adapter | WiredExtra::AntigravityModel => false,
            WiredExtra::None => true,
            WiredExtra::GeminiApiKey => {
                let settings = crate::steady::read_optional(&home.join(".gemini").join("settings.json"))
                    .ok().flatten().unwrap_or_default();
                crate::gateway::wiring::gemini_selected_type(&settings).as_deref() == Some("gemini-api-key")
            },
            WiredExtra::KimiDefaultModel => crate::gateway::wiring::kimi_default_model_ours(text),
            WiredExtra::PiDefaultProvider => {
                let settings = crate::steady::read_optional(&crate::gateway::wiring_pi::settings_path(home))
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                crate::gateway::wiring_pi::selected_ours(&settings)
            }
            WiredExtra::SelectionOurs => crate::gateway::wiring_extra::selected_ours(self.id, text),
        }
    }
}

// ─────────────────────────── 各家路径（与探测 `agent_discovery` 同源） ───────────────────────────

fn path_claude(home: &Path) -> Option<PathBuf> {
    // claude_dir 支持 CLAUDE_CONFIG_DIR 覆盖，不走 home
    let _ = home;
    Some(crate::creds::claude_dir()?.join("settings.json"))
}

fn path_codex(home: &Path) -> Option<PathBuf> {
    let _ = home;
    Some(crate::creds::codex_dir()?.join("config.toml"))
}

fn path_zcode(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring::zcode_dir_public(home).join("v2").join("provider_config.json"))
}

fn path_gemini(home: &Path) -> Option<PathBuf> {
    Some(home.join(".gemini").join(".env"))
}

fn path_opencode(home: &Path) -> Option<PathBuf> {
    Some(
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode")
            .join("opencode.json"),
    )
}

fn path_kimi(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring::kimi_dir_public(home).0.join("config.toml"))
}

fn path_dsh(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring_dsh::config_path(home))
}

fn path_pi(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring_pi::config_path(home))
}

// 三家的路径逻辑在 `wiring_extra::config_path(agent, home)`，这里各包一层单参函数
// （闭包不能出现在 `const` 里，所以写成具名函数而不是 `path_extra("pencil")`）
fn path_pencil(home: &Path) -> Option<PathBuf> {
    crate::gateway::wiring_extra::config_path("pencil", home)
}
fn path_qoder(home: &Path) -> Option<PathBuf> {
    crate::gateway::wiring_extra::config_path("qoder", home)
}
fn path_qoder_cn(home: &Path) -> Option<PathBuf> {
    crate::gateway::wiring_extra::config_path("qoder-cn", home)
}
fn path_qwen(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring_qwen::config_path(home))
}
fn path_workbuddy(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring_workbuddy::config_path(home))
}
fn path_antigravity(home: &Path) -> Option<PathBuf> {
    Some(crate::gateway::wiring_antigravity::config_path(home))
}
fn antigravity_no_base(_: &str) -> Option<String> {
    None
}

// ─────────────────────────── 注册表：加一家 = 加一行 ───────────────────────────

/// 注册顺序同时用于前端工具卡片的默认顺序。
pub(crate) const SPECS: &[AgentSpec] = &[
    AgentSpec {
        id: "claude-code",
        wire: crate::gateway::wiring::adapters::wire_claude,
        unwire: crate::gateway::wiring::adapters::unwire_claude,
        unwire_without_main: false,
        after_unwire: None,
        path: path_claude,
        base: crate::gateway::wiring::claude_current_base,
        token: |_, text| crate::gateway::wiring::claude_current_token(text),
        token_match: TokenMatch::LegacyLiteral,
        extra: WiredExtra::None,
        notice: "新会话生效；已在运行的会话需重开",
        uses_catalog: false,
        ua: &["claude-cli", "claude-code"],
    },
    AgentSpec {
        id: "codex",
        wire: crate::gateway::wiring::adapters::wire_codex,
        unwire: crate::gateway::wiring::adapters::unwire_codex,
        unwire_without_main: false,
        after_unwire: None,
        path: path_codex,
        base: crate::gateway::wiring::codex_effective_base,
        token: |_, text| text.parse::<toml_edit::DocumentMut>().ok().and_then(|doc| {
            doc.get("model_providers")?.get(crate::gateway::wiring::CODEX_PROVIDER)?.get("experimental_bearer_token")?.as_str().map(str::to_string)
        }),
        token_match: TokenMatch::Ignored,
        extra: WiredExtra::None,
        notice: "保留 ChatGPT 登录时只改请求地址；需重启 Codex 后生效",
        uses_catalog: false,
        // codex_cli_rs 是 Codex CLI 实际发的产品名（实测 UA `codex_cli_rs/0.1`）；
        // magpie 声明的是 codex / acp-extension-codex（ACP 适配器与桌面版）。三套都要认。
        ua: &["codex", "codex_cli_rs", "acp-extension-codex"],
    },
    AgentSpec {
        id: "gemini",
        wire: crate::gateway::wiring::adapters::wire_gemini,
        unwire: crate::gateway::wiring::adapters::unwire_gemini,
        unwire_without_main: true,
        after_unwire: None,
        path: path_gemini,
        base: crate::gateway::wiring::gemini_current_base,
        token: |_, text| crate::gateway::wiring::gemini_current_token(text),
        token_match: TokenMatch::Ignored,
        extra: WiredExtra::GeminiApiKey,
        notice: "接线期间改用网关 Key，断开时恢复原登录；需重启 Gemini 后生效",
        uses_catalog: false,
        ua: &["geminicli", "gemini-cli"],
    },
    AgentSpec {
        id: "opencode",
        wire: crate::gateway::wiring::adapters::wire_opencode,
        unwire: crate::gateway::wiring::adapters::unwire_opencode,
        unwire_without_main: false,
        after_unwire: None,
        path: path_opencode,
        base: crate::gateway::wiring::opencode_current_base,
        token: |_, text| crate::gateway::wiring::opencode_current_token(text),
        token_match: TokenMatch::Ignored,
        extra: WiredExtra::None,
        notice: "需重启该工具后生效",
        uses_catalog: false,
        ua: &["opencode"],
    },
    AgentSpec {
        id: "zcode",
        wire: crate::gateway::wiring::adapters::wire_zcode,
        unwire: crate::gateway::wiring::adapters::unwire_zcode,
        unwire_without_main: false,
        after_unwire: Some(crate::gateway::wiring::zcode_clean_legacy),
        path: path_zcode,
        base: crate::gateway::wiring::zcode_current_base,
        token: |_, text| crate::gateway::wiring::zcode_current_token(text),
        token_match: TokenMatch::Ignored,
        extra: WiredExtra::None,
        notice: "需重启 ZCode 后生效",
        uses_catalog: true,
        ua: &["zcode"],
    },
    AgentSpec {
        id: "kimi",
        wire: crate::gateway::wiring::adapters::wire_kimi,
        unwire: crate::gateway::wiring::adapters::unwire_kimi,
        unwire_without_main: false,
        after_unwire: None,
        path: path_kimi,
        base: crate::gateway::wiring::kimi_current_base,
        token: |_, text| crate::gateway::wiring::kimi_current_token(text),
        token_match: TokenMatch::Exact,
        extra: WiredExtra::KimiDefaultModel,
        notice: "需重启 Kimi 后生效",
        uses_catalog: true,
        ua: &["kimicli"],
    },
    AgentSpec {
        id: "dsh",
        wire: crate::gateway::wiring::adapters::wire_dsh,
        unwire: crate::gateway::wiring::adapters::unwire_dsh,
        unwire_without_main: false,
        after_unwire: None,
        path: path_dsh,
        base: crate::gateway::wiring_dsh::current_base,
        token: |home, _| crate::gateway::wiring_dsh::current_token(home),
        token_match: TokenMatch::Prefixed,
        extra: WiredExtra::Adapter,
        notice: "新会话生效；已打开的会话保持原模型",
        uses_catalog: true,
        ua: &["deepseek-harness"],
    },
    AgentSpec {
        id: "pi",
        wire: crate::gateway::wiring::adapters::wire_pi,
        unwire: crate::gateway::wiring::adapters::unwire_pi,
        unwire_without_main: false,
        after_unwire: None,
        path: path_pi,
        base: crate::gateway::wiring_pi::current_base,
        token: |_, text| crate::gateway::wiring_pi::current_token(text),
        token_match: TokenMatch::Prefixed,
        extra: WiredExtra::PiDefaultProvider,
        notice: "新会话生效；已打开的会话保持原模型",
        uses_catalog: true,
        ua: &[],
    },
    AgentSpec {
        id: "pencil",
        wire: crate::gateway::wiring::adapters::wire_extra,
        unwire: crate::gateway::wiring::adapters::unwire_extra,
        unwire_without_main: false,
        after_unwire: None,
        path: path_pencil,
        base: |text| crate::gateway::wiring_extra::current_base("pencil", text),
        token: |_, text| crate::gateway::wiring_extra::current_token("pencil", text),
        token_match: TokenMatch::Exact,
        extra: WiredExtra::SelectionOurs,
        notice: "Pencil 启动时读自定义模型：重启 Pencil 后在模型选择器里选 Jolu 的模型",
        uses_catalog: true,
        ua: &[],
    },
    AgentSpec {
        id: "qoder",
        wire: crate::gateway::wiring::adapters::wire_extra,
        unwire: crate::gateway::wiring::adapters::unwire_extra,
        unwire_without_main: false,
        after_unwire: None,
        path: path_qoder,
        base: |text| crate::gateway::wiring_extra::current_base("qoder", text),
        token: |_, text| crate::gateway::wiring_extra::current_token("qoder", text),
        token_match: TokenMatch::Exact,
        extra: WiredExtra::SelectionOurs,
        notice: "新会话生效；已打开的会话保持原模型",
        uses_catalog: true,
        ua: &[],
    },
    AgentSpec {
        id: "qoder-cn",
        wire: crate::gateway::wiring::adapters::wire_extra,
        unwire: crate::gateway::wiring::adapters::unwire_extra,
        unwire_without_main: false,
        after_unwire: None,
        path: path_qoder_cn,
        base: |text| crate::gateway::wiring_extra::current_base("qoder-cn", text),
        token: |_, text| crate::gateway::wiring_extra::current_token("qoder-cn", text),
        token_match: TokenMatch::Exact,
        extra: WiredExtra::SelectionOurs,
        notice: "新会话生效；已打开的会话保持原模型",
        uses_catalog: true,
        ua: &[],
    },
    AgentSpec {
        id: "qwen",
        wire: crate::gateway::wiring::adapters::wire_qwen,
        unwire: crate::gateway::wiring::adapters::unwire_qwen,
        unwire_without_main: false,
        after_unwire: None,
        path: path_qwen,
        base: crate::gateway::wiring_qwen::current_base,
        token: |_, text| crate::gateway::wiring_qwen::current_token(text),
        // 令牌写在 `env.JOLU_QWEN_API_KEY`（不在 modelProviders 数组里），
        // 所以读 token 与读 base 看的是文件里两个不同位置，`Exact` 按整串比
        token_match: TokenMatch::Exact,
        extra: WiredExtra::None,
        notice: "新会话生效；已打开的会话保持原模型",
        uses_catalog: true,
        ua: &["qwencode"],
    },
    AgentSpec {
        id: "antigravity",
        wire: crate::gateway::wiring_antigravity::wire,
        unwire: crate::gateway::wiring_antigravity::unwire,
        unwire_without_main: false,
        after_unwire: Some(crate::gateway::wiring_antigravity::cleanup_launcher),
        path: path_antigravity,
        // 地址与 Key 只在 agy 的环境变量里（由 jolu-agy.cmd 注入）；令牌从启动脚本回读以支持多实例检测。
        base: antigravity_no_base,
        token: |home, _| crate::gateway::wiring_antigravity::current_token(home),
        token_match: TokenMatch::Exact,
        extra: WiredExtra::AntigravityModel,
        notice: "Antigravity CLI 只从环境变量读取网关地址与 Key：用同目录生成的 jolu-agy.cmd 启动；直接运行 agy 不经过模型路由",
        uses_catalog: true,
        ua: &[],
    },
    AgentSpec {
        id: "workbuddy",
        wire: crate::gateway::wiring::adapters::wire_workbuddy,
        unwire: crate::gateway::wiring::adapters::unwire_workbuddy,
        unwire_without_main: false,
        after_unwire: None,
        path: path_workbuddy,
        // WorkBuddy 的 base 要从我们条目的 url 里剥掉 /chat/completions，需要令牌判归属
        base: crate::gateway::wiring_workbuddy::current_base,
        token: |_, text| crate::gateway::wiring_workbuddy::current_token(text),
        token_match: TokenMatch::Prefixed,
        extra: WiredExtra::None,
        // 5.6 的 app.asar 只在启动时监视 models.json，重写后要重启才认
        notice: "重启 WorkBuddy 后生效（它只在启动时读这个文件）",
        uses_catalog: true,
        ua: &["workbuddy"],
    },
];

/// 按 id 查声明。
pub(crate) fn spec(agent: &str) -> Option<&'static AgentSpec> {
    SPECS.iter().find(|spec| spec.id == agent)
}

/// 全部可接线工具的 id（替代手写的 `AGENTS` 常量）。
/// 返回 `&'static [&'static str]` 以便调用方直接 `for agent in agents()` 迭代。
pub(crate) fn ids() -> &'static [&'static str] {
    static IDS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| SPECS.iter().map(|spec| spec.id).collect());
    &IDS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表覆盖全部 AGENTS，且无重复——这条防止「加了 adapter 忘了注册」。
    #[test]
    fn every_agent_has_exactly_one_spec() {
        for agent in crate::gateway::wiring::agents() {
            let hits = SPECS.iter().filter(|spec| spec.id == *agent).count();
            assert_eq!(hits, 1, "{agent} 在注册表里出现 {hits} 次");
            assert!(spec(agent).is_some(), "{agent} 没有注册声明");
        }
        assert_eq!(SPECS.len(), crate::gateway::wiring::agents().len(), "注册表与 agents() 家数不一致");
    }

    /// 写进配置的模型清单：只有这些家从网关目录取（其余家不看 catalog）。
    /// 漏标的后果：接入时写的模型清单为空，工具里看不到任何模型。
    #[test]
    fn catalog_users_are_declared() {
        let expected: Vec<&str> = SPECS.iter().filter(|spec| spec.uses_catalog).map(|spec| spec.id).collect();
        assert_eq!(
            expected,
            vec!["zcode", "kimi", "dsh", "pi", "pencil", "qoder", "qoder-cn", "qwen", "antigravity", "workbuddy"]
        );
    }

    /// 每家都要有非空提示文案（漏写会让界面显示空字符串）。
    #[test]
    fn every_spec_has_a_notice() {
        for spec in SPECS {
            assert!(!spec.notice.trim().is_empty(), "{} 的提示文案是空的", spec.id);
        }
    }

    /// 注册表与探测表必须能对上：接线清单里的每家都应被探测覆盖
    /// （探测到才能显示卡片；注册了却探测不到 = 用户永远看不到这张卡）。
    #[test]
    fn every_wirable_agent_is_also_detected() {
        let detected = crate::agent_discovery::spec_ids();
        for agent in crate::gateway::wiring::agents() {
            assert!(detected.contains(agent), "{agent} 可接线但探测表里没有，用户看不到这张卡");
        }
    }

    /// 路径函数对每个已注册的 agent 都要能算出一个路径（`None` 只在用户目录缺失时出现，
    /// 测试环境里 home 一定有）。
    #[test]
    fn every_spec_resolves_a_config_path() {
        let home = std::env::temp_dir();
        for spec in SPECS {
            assert!((spec.path)(&home).is_some(), "{} 算不出配置路径", spec.id);
        }
    }

    /// 每家声明「已接线」的样本都必须能让 `is_wired` 返回 true。
    ///
    /// 这条锁住一个真出过的 bug（2026-11-20 审查工作流发现，high）：dsh 的令牌在
    /// `$DSH_HOME/.env` 而不是主文件里，旧版 `token` 写成 `|_| None`，而 `token_match`
    /// 写成了 `Prefixed` —— `is_wired` 的令牌前置门因此恒假，dsh 接上过也永远显示
    /// 「未接入」，断开与启动自愈都会跳过它、原值快照还会被清掉。
    ///
    /// 为什么不能用「静态声明检查」代替：那种写法（`token_match != Ignored` 时必须能读
    /// 到令牌，除非 extra == Adapter）会被 Adapter 这条豁免无条件放行，测的是标记而不是
    /// 真实行为——上面的 bug 正是这样漏掉的。这里给每家造一个临时 home 与**已接线**的
    /// 配置文件，断言 `is_wired` 真的返回 true；前置门挡死时会立刻红。
    #[test]
    fn a_wired_sample_is_recognised_as_wired() {
        let root = match std::env::var_os("JOLU_WIRED_SAMPLE_ROOT") {
            Some(root) => std::path::PathBuf::from(root),
            None => {
                // 各工具会读取进程环境变量。用子进程隔离这些变量，避免覆盖真实配置，
                // 也避免与并行测试互相修改同一份环境。
                let root = std::env::temp_dir().join(format!("jolu-spec-wired-{}", std::process::id()));
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", &format!("{}::a_wired_sample_is_recognised_as_wired", module_path!().trim_start_matches("jolu_lib::")), "--nocapture"])
                    .env("JOLU_WIRED_SAMPLE_ROOT", &root)
                    .env("USERPROFILE", &root)
                    .env("HOME", &root)
                    .env("CLAUDE_CONFIG_DIR", root.join("claude"))
                    .env("CODEX_HOME", root.join("codex"))
                    .env("XDG_CONFIG_HOME", root.join("config"))
                    .env("KIMI_CODE_HOME", root.join("kimi-code"))
                    .env("KIMI_SHARE_DIR", root.join("kimi"))
                    .env("ZCODE_HOME", root.join("zcode"))
                    .env("DSH_HOME", root.join("dsh"))
                    .env("PI_CODING_AGENT_DIR", root.join("pi"))
                    .env("QODER_CONFIG_DIR", root.join("qoder"))
                    .env("QODERCN_CONFIG_DIR", root.join("qoder-cn"))
                    .output().unwrap();
                let _ = std::fs::remove_dir_all(&root);
                assert!(output.status.success(), "隔离样本测试失败：\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
                return;
            }
        };
        let port = 12731u16;
        let instance = "abcd1234";
        let mut checked = 0;
        for spec in SPECS {
            let Some(home) = make_wired_home(&root, spec.id, instance) else {
                panic!("{} 没有接线样本：给 `make_wired_home` 补一份，否则这条测试覆盖不到它", spec.id);
            };
            let text = std::fs::read_to_string((spec.path)(&home).expect("样本 home 要能算出路径")).unwrap_or_default();
            let base = (spec.base)(&text);
            let token = (spec.token)(&home, &text);
            assert!(
                spec.is_wired(&home, &text, port, instance, None),
                "{} 的已接线样本被判成「未接入」（base={:?} token={:?} 文本前 80 字={:?}）",
                spec.id,
                base,
                token,
                &text.chars().take(80).collect::<String>(),
            );
            checked += 1;
        }
        assert_eq!(checked, SPECS.len(), "每家都要真的被验一次");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 在 `root/<agent>` 下造一个「已接线」的最小 home，返回它；
    /// 认不出的 agent 返回 `None`（新增一家忘了补样本时测试会红）。
    fn make_wired_home(root: &std::path::Path, agent: &str, instance: &str) -> Option<std::path::PathBuf> {
        let home = root.join(agent);
        let path = (spec(agent)?.path)(&home)?;
        assert!(path.starts_with(root), "样本路径超出临时目录：{}", path.display());
        std::fs::create_dir_all(path.parent()?).ok()?;
        let body = match agent {
            "claude-code" => r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcd1234"}}"#.to_string(),
            "codex" => "openai_base_url = \"http://127.0.0.1:12731/v1\"
".to_string(),
            "zcode" => r#"{"config":{"providerConfigRules":{"providerRules":[{"providerId":"jolu","config":{"api":{"baseUrl":"http://127.0.0.1:12731"}}}]}}}"#.to_string(),
            "gemini" => {
                std::fs::write(home.join(".gemini").join("settings.json"), r#"{"security":{"auth":{"selectedType":"gemini-api-key"}}}"#).ok()?;
                "GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731
GEMINI_API_KEY=jolu-gemini.abcd1234
".to_string()
            },
            "opencode" => r#"{"provider":{"jolu":{"options":{"baseURL":"http://127.0.0.1:12731","apiKey":"jolu-opencode.abcd1234"}}}}"#.to_string(),
            "kimi" => "default_model = \"jolu/zcode/glm-5.3\"
[providers.jolu]
base_url = \"http://127.0.0.1:12731\"
api_key = \"jolu-kimi.abcd1234\"
".to_string(),
            "pi" => {
                // pi 的默认 provider 在**另一个文件**（settings.json），与主文件同目录
                std::fs::write(
                    crate::gateway::wiring_pi::settings_path(&home),
                    r#"{"defaultProvider":"jolu","defaultModel":"zcode/glm-5.3"}"#,
                ).ok()?;
                r#"{"providers":{"jolu":{"baseUrl":"http://127.0.0.1:12731/v1","apiKey":"jolu-pi.abcd1234"}}}"#.to_string()
            }
            "pencil" | "qoder" | "qoder-cn" => {
                // 这三家的 token_match 是 Exact：令牌必须精确等于 token_for(agent, instance)
                format!(
                    r#"{{"providers":{{"jolu":{{"baseUrl":"http://127.0.0.1:12731/v1","apiKey":"jolu-{agent}.{instance}","model":"jolu/zcode/glm-5.3"}}}},"model":{{"name":"jolu/zcode/glm-5.3"}}}}"#
                )
            }
            "dsh" => {
                // dsh 的令牌在 $DSH_HOME/.env，patch 里只放引用；这里造一份最小 patch + .env。
                // wiring_dsh::wired 会自己读 .env —— 这条路径正是被令牌前置门挡死的那条。
                let dsh_home = crate::gateway::wiring_dsh::dsh_dir_public(&home);
                std::fs::create_dir_all(dsh_home.join("profiles").join("web")).ok()?;
                std::fs::write(
                    dsh_home.join("profiles").join("web").join("cordis.patch.yml"),
                    [
                        "- id: llm-pi-ai",
                        "  name: \"@deepseek-ai/dsh-llm-pi-ai\"",
                        "  config:",
                        "    providers:",
                        "      jolu:",
                        "        baseURL: http://127.0.0.1:12731/v1",
                        "- id: agent-default-model",
                        "  name: \"@deepseek-ai/dsh-agent-default-model\"",
                        "  config:",
                        "    provider: jolu",
                        "    model: zcode/glm-5.3",
                    ].join("
"),
                ).ok()?;
                std::fs::write(dsh_home.join(".env"), format!("JOLU_API_KEY=jolu-{agent}.{instance}
")).ok()?;
                // 主文件 = 第一个 profile 的 patch（config_path 已指向它），与上面写的同一个
                return Some(home);
            }
            "antigravity" => {
                // 地址与 Key 只在启动脚本里；状态判定会读同目录的 jolu-agy.cmd。
                std::fs::write(
                    crate::gateway::wiring_antigravity::launcher_path(&path),
                    format!("set \"GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731\"\r\nset \"GEMINI_API_KEY=jolu-{agent}.{instance}\"\r\n"),
                ).ok()?;
                format!(
                    r#"{{"model":"jolu/zcode/glm-5.3","modelProvider":"gemini","customModelsConfig":{{"customModels":{{"jolu/zcode/glm-5.3":{{"apiProvider":"API_PROVIDER_GOOGLE_GEMINI","modelName":"zcode/glm-5.3"}}}}}}}}"#
                )
            }
            "qwen" => {
                // 令牌在 env.JOLU_QWEN_API_KEY，地址在 modelProviders.openai 的条目里
                format!(
                    r#"{{"modelProviders":{{"openai":[{{"baseUrl":"http://127.0.0.1:12731/v1","envKey":"JOLU_QWEN_API_KEY","id":"zcode/glm-5.3","name":"GLM-5.3"}}]}},"env":{{"JOLU_QWEN_API_KEY":"jolu-{agent}.{instance}"}},"security":{{"auth":{{"selectedType":"openai"}}}},"model":{{"name":"zcode/glm-5.3","baseUrl":"http://127.0.0.1:12731/v1"}}}}"#
                )
            }
            "workbuddy" => {
                // 顶层数组；判定只看 apiKey（`jolu-workbuddy.<实例>`）
                format!(
                    r#"[{{"id":"zcode/glm-5.3","name":"GLM-5.3","vendor":"jolu","apiKey":"jolu-{agent}.{instance}","url":"http://127.0.0.1:12731/v1/chat/completions","maxInputTokens":200000,"supportsToolCall":true,"supportsImages":false,"supportsReasoning":false}}]"#
                )
            }
            _ => return None,
        };
        std::fs::write(&path, body).ok()?;
        Some(home)
    }
}
