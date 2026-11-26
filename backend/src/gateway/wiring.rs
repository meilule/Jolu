//! 接线层：把各 agent 的配置指向本机网关。
//!
//! 纪律：
//! 1. 写前把原文件备份到应用数据目录
//! 2. 临时文件 + rename 原子写
//! 3. 还原只清理自己的写入：当前值不再是网关地址时不覆盖（外部意图优先）
//! 4. 所有入口共用一把锁（[`crate::cli_switch::with_switch`]，进程内 + 跨进程）
//! 5. 接线动作先记 `agent_wiring` 的 pending，写完文件再标 wired
//! 6. 原值快照**过滤一切网关值**：重新接线不能把网关地址当成用户原来的上游
//!
//! 文件读写拆成纯函数（`*_wire` / `*_read`），路径解析依赖真实用户目录、单独一层，
//! 测试用临时文本验证保留性与边界（与 `proxy_config.rs` 同款做法）。

pub(crate) mod adapters;

use std::path::{Path, PathBuf};

use crate::gateway::wiring_store::{Applied, TierModels, WiringRecord};

/// 支持的 agent 标识（与 `providerCatalog` 的平台 id 对齐）。
/// 后四家（Pencil / Qoder / Qoder CN / Crush）的适配在 `wiring_extra.rs`（5.11）
// dsh 的适配在 `wiring_dsh.rs`（多 profile patch + .env + credentials，magpie dsh.go 裁剪版）
/// 可接线工具清单。**从注册表派生**（`wiring_specs::SPECS`），不再手写——
/// 加一家只需在注册表加一行，这里与 `db.rs` 种子（同样从注册表派生）自动跟上。
pub fn agents() -> &'static [&'static str] {
    super::wiring_specs::ids()
}

pub fn is_agent(agent: &str) -> bool {
    super::wiring_specs::spec(agent).is_some()
}

/// 某 agent 的接入状态。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WiringStatus {
    pub agent: String,
    /// 是否已接入网关
    pub wired: bool,
    /// 配置文件路径（展示用；不存在时为 None）
    pub config_path: Option<String>,
    /// 需要重启 agent 才生效时给出提示
    pub notice: Option<String>,
    /// 无法接线的原因（已安装但没有可写配置等）
    pub reason: Option<String>,
    /// 接线记录的阶段：idle / pending / wired；没有记录时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// 漂移：pending / removed / changed / foreign；正常或被接受后为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<String>,
    /// Codex 检测到的登录方式：oauth / provider；其他 agent 为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_mode: Option<String>,
    /// B8a：Claude Code 配置里**现在**的主模型（回读 settings.json，`/model` 改的就是它）；
    /// 其他 agent 或读不到时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    /// B8b：`current_model` 是档位别名（opus / sonnet / haiku / fable / best / opusplan）时解析出的真实模型
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_resolved: Option<String>,
}

/* ─────────────────────────── 纯函数：各 agent 的配置读写 ─────────────────────────── */

/// 网关写入的地址 / token 特征：任何 127.0.0.1 / localhost 地址或 `jolu-*` 令牌。
pub(crate) fn is_gateway_value(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("jolu-") || value.contains("127.0.0.1") || value.contains("localhost")
}

/// Claude Code 的模型档位（magpie `claudeTiers` 同款）。
/// 接线时各档**默认跟随主模型**（magpie 的 `Follows: "model"` 语义）：选一次主模型，
/// opus/sonnet/haiku/fable 与 small-fast 全部写成它；用户在 /model 里的切换不再被覆盖——
/// 因为 `ANTHROPIC_MODEL` 会被删掉（它压过 /model 的选择，写了它切换只在本会话生效）。
const CLAUDE_TIERS: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];

fn claude_tier_env(tier: &str) -> String {
    format!("ANTHROPIC_DEFAULT_{}_MODEL", tier.to_ascii_uppercase())
}

/// 每档的「关于」键：`/model` 菜单据此显示档位模型的名称与描述。
/// 档位换模型时旧值会张冠李戴（magpie #1227），所以接线时一并清掉、断开时还原。
const CLAUDE_ABOUT: [&str; 3] = ["_NAME", "_DESCRIPTION", "_SUPPORTED_CAPABILITIES"];

/// 云端开关（magpie `claudeClouds` 同款，6 个）：它们会把请求引去 Bedrock / Vertex /
/// Foundry / Anthropic on AWS / Anthropic on GCP / Bedrock Mantle，绕过 ANTHROPIC_BASE_URL，
/// 接线期间必须移除（原值暂存快照、断开还原）；云专属变量（AWS_*、ANTHROPIC_VERTEX_PROJECT_ID
/// 等）不动——开关移除后 Claude Code 不读它们，断开时自然恢复生效。
const CLAUDE_CLOUD_SWITCHES: [&str; 6] = [
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_MANTLE",
];

/// 接线时按主模型派生的 env（P0-1 / B5 / B6 / B7，magpie `writeTiers` 第 3–7 条）。
pub(crate) const CLAUDE_MAX_OUTPUT: &str = "CLAUDE_CODE_MAX_OUTPUT_TOKENS";
pub(crate) const CLAUDE_MAX_CONTEXT: &str = "CLAUDE_CODE_MAX_CONTEXT_TOKENS";
pub(crate) const CLAUDE_AUTO_MODE_SERVER: &str = "CLAUDE_CODE_AUTO_MODE_SERVER";
pub(crate) const CLAUDE_COMPACT_WINDOW: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";
pub(crate) const CLAUDE_MODEL_CAPABILITIES: &str = "CLAUDE_CODE_MODEL_CAPABILITIES";
const CLAUDE_EXTRA_KEYS: [&str; 5] = [CLAUDE_MAX_OUTPUT, CLAUDE_MAX_CONTEXT, CLAUDE_AUTO_MODE_SERVER, CLAUDE_COMPACT_WINDOW, CLAUDE_MODEL_CAPABILITIES];

/// 派生 env 的输入。数值来自 models.dev（缓存 → 内置快照）；缺失就是 None，**不写也不猜**。
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaudeExtras {
    /// 主模型的最大输出（B1）：不写时 Claude Code 默认 32000，第三方长回复会被截断
    pub max_output: Option<i64>,
    /// 主模型的上下文窗口（B2）：不写时 Claude Code 把大窗口模型按 200K 处理
    pub context_window: Option<i64>,
    /// 长对话压缩阈值（B5）：已按窗口裁好的最终值
    pub compact_window: Option<i64>,
    /// 全局能力声明（B6）：`模型=effort,xhigh_effort;…`
    pub capabilities: Option<String>,
    /// 上次接线写下的派生值：这次算不出时，文件里仍等于它的键删掉（不留指错模型的旧值）
    pub previous: std::collections::BTreeMap<String, String>,
}

/// 主模型是不是 Claude 家族（剥掉来源前缀、`[1m]` 与 `:effort`）。
fn is_claude_family(model: &str) -> bool {
    let (bare, _) = crate::gateway::wiring_store::split_tier_value(model);
    bare.rsplit('/').next().unwrap_or(bare).to_ascii_lowercase().contains("claude")
}

/// 主模型是不是 Fable 档家族（B3：只有它才配写进 Fable 档）。
fn is_fable_family(model: &str) -> bool {
    let (bare, _) = crate::gateway::wiring_store::split_tier_value(model);
    bare.rsplit('/').next().unwrap_or(bare).to_ascii_lowercase().contains("fable")
}

/// 这次接线要写的派生 env（纯函数；`claude_wire` 写文件、`Applied.extra_env` 记账都用它）。
///
/// B7 的判据：auto 模式的服务端复核是 Anthropic 自家能力，主模型不是 Claude 家族时
/// 关掉（写 `"0"`），否则 Claude Code 会对上游发它不认识的复核请求。这是按 magpie
/// 「上游不支持时关掉」的口径做的推断，Claude 家族不写、交给 Claude Code 自己决定。
pub(crate) fn claude_extra_env(model: &str, extras: &ClaudeExtras) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let positive = |value: Option<i64>| value.filter(|v| *v > 0).map(|v| v.to_string());
    if let Some(value) = positive(extras.max_output) {
        out.insert(CLAUDE_MAX_OUTPUT.to_string(), value);
    }
    if let Some(value) = positive(extras.context_window) {
        out.insert(CLAUDE_MAX_CONTEXT.to_string(), value);
    }
    if let Some(value) = positive(extras.compact_window) {
        out.insert(CLAUDE_COMPACT_WINDOW.to_string(), value);
    }
    if let Some(value) = extras.capabilities.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        out.insert(CLAUDE_MODEL_CAPABILITIES.to_string(), value.to_string());
    }
    if !is_claude_family(model) {
        out.insert(CLAUDE_AUTO_MODE_SERVER.to_string(), "0".to_string());
    }
    out
}

/// 快照里去掉上次接线自己写的派生值（按键比对：`"0"`、窗口数这类值不能跨键判断）。
fn drop_our_extras(mut saved: serde_json::Value, previous: &std::collections::BTreeMap<String, String>) -> Option<serde_json::Value> {
    if let Some(env) = saved.get_mut("env").and_then(|e| e.as_object_mut()) {
        env.retain(|key, value| !previous.get(key).is_some_and(|ours| value.as_str() == Some(ours.as_str())));
        if env.is_empty() {
            saved.as_object_mut()?.remove("env");
        }
    }
    let empty = saved.as_object().is_some_and(|o| o.is_empty());
    (!empty).then_some(saved)
}

/// 接线涉及的全部 env 键（快照与还原的清单）。
fn claude_env_keys() -> Vec<String> {
    let mut keys = vec![
        "ANTHROPIC_BASE_URL".to_string(),
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        "ANTHROPIC_MODEL".to_string(),
        "ANTHROPIC_SMALL_FAST_MODEL".to_string(),
        "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
        // 档位自选的 Auto 判别（批 4）：写了就要能还原，否则断开后留在用户配置里
        "CLAUDE_CODE_AUTO_MODE_MODEL".to_string(),
    ];
    // 派生 env：用户原值进快照，断开时逐键还原
    keys.extend(CLAUDE_EXTRA_KEYS.map(str::to_string));
    for tier in CLAUDE_TIERS {
        keys.push(claude_tier_env(tier));
        for about in CLAUDE_ABOUT {
            keys.push(format!("{}{about}", claude_tier_env(tier)));
        }
    }
    keys.extend(CLAUDE_CLOUD_SWITCHES.map(str::to_string));
    keys
}

/// 一个模型的 `_SUPPORTED_CAPABILITIES`：`effort`，收 xhigh / max 时再加对应项；不收 effort 为空串。
pub(crate) fn claude_capabilities(model: &str) -> String {
    let levels = crate::gateway::wiring_store::tier_efforts(model).unwrap_or(crate::gateway::wiring_store::CLAUDE_EFFORTS);
    if levels.is_empty() {
        return String::new();
    }
    let mut caps = vec!["effort"];
    if levels.contains(&"xhigh") {
        caps.push("xhigh_effort");
    }
    if levels.contains(&"max") {
        caps.push("max_effort");
    }
    caps.join(",")
}

/// D2：把用户的 effortLevel 调到新模型收的档。None = 不用动；Some(None) = 模型不收 effort，删掉；
/// Some(Some(档)) = 换成不高于原值的最高一档（原值比所有可选都低时取最低一档）。
fn fit_effort(model: &str, effort: &str) -> Option<Option<String>> {
    const ORDER: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
    let levels = crate::gateway::wiring_store::tier_efforts(model).unwrap_or(crate::gateway::wiring_store::CLAUDE_EFFORTS);
    let wanted = effort.trim().to_ascii_lowercase();
    if levels.contains(&wanted.as_str()) {
        return None;
    }
    if levels.is_empty() {
        return Some(None);
    }
    let rank = ORDER.iter().position(|level| *level == wanted)?;
    let fitted = ORDER[..=rank].iter().rev().find(|level| levels.contains(level)).or_else(|| levels.first());
    Some(fitted.map(|level| level.to_string()))
}

/// Claude Code 接线写 `settings.json`。
///
/// `model` 是主模型：写了它，就同时写顶层 `model`、`ANTHROPIC_SMALL_FAST_MODEL` 与全部档位 env
/// （默认跟随主模型）、删掉 `ANTHROPIC_MODEL` 与各档的旧「关于」键；没写它，就只接地址和令牌，
/// 用户的模型配置原样保留。
///
/// `tiers` 是批 4 的档位自选（magpie tier_own 同款）：某档有值时写该值（可带 `:effort`
/// 与 `[1m]`），「关于」（`_NAME`/`_DESCRIPTION`）按**该档自己的模型**取——`/model` 菜单
/// 里那行要显示档位实际用的模型；`tier_about` 给「档位写入值」返回它的显示名与描述。
/// 没自选的档位照旧写主模型、用主模型的「关于」。
#[allow(clippy::too_many_arguments)]
pub(super) fn claude_wire(
    text: &str,
    base: &str,
    token: &str,
    model: Option<&str>,
    label: Option<&str>,
    description: Option<&str>,
    subagent_model: Option<&str>,
    tiers: Option<&TierModels>,
    tier_about: &dyn Fn(&str) -> (Option<String>, Option<String>),
    extras: &ClaudeExtras,
) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("settings.json 不是有效 JSON：{error}"))?;
    let object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    let env = object.entry("env").or_insert_with(|| serde_json::json!({}));
    let env = env.as_object_mut().ok_or("settings.json 的 env 段不是对象")?;
    env.insert("ANTHROPIC_BASE_URL".into(), serde_json::Value::String(base.into()));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), serde_json::Value::String(token.into()));
    // 云开关把请求引去 Bedrock/Vertex、绕过 ANTHROPIC_BASE_URL：接线期间移除（原值在快照里，断开还原）
    for switch in CLAUDE_CLOUD_SWITCHES {
        env.remove(switch);
    }
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        // 主模型的 env 覆盖必须移除：它压过 /model 的选择（magpie writeTiers 同款动作）
        env.remove("ANTHROPIC_MODEL");
        // 显示名 / 描述：models.dev 的 label 优先，没有就退回模型 id 本身
        let name = label.map(str::trim).filter(|l| !l.is_empty()).unwrap_or(model);
        let description = description.map(str::trim).filter(|d| !d.is_empty());
        for tier in CLAUDE_TIERS {
            // B3（magpie #2090）：Fable 档跟随主模型时**留空**。把别家主模型写进
            // ANTHROPIC_DEFAULT_FABLE_MODEL，Claude Code 会把它当 Fable：Pro 账号先要 usage
            // credits、Opus 掉回 Haiku、别家模型直接不跑。用户自选的 Fable 档照写。
            if tier == "fable" && tiers.and_then(|t| t.tier(tier)).is_none() && !is_fable_family(model) {
                env.remove(&claude_tier_env(tier));
                for about in CLAUDE_ABOUT {
                    env.remove(&format!("{}{about}", claude_tier_env(tier)));
                }
                continue;
            }
            // 批 4：档位自选优先，None 跟随主模型（tier_own 语义——自选值已在快照里，换主模型不重算）
            let (tier_value, tier_name, tier_desc) = match tiers.and_then(|t| t.tier(tier)) {
                Some(own) => {
                    let (own_name, own_desc) = tier_about(own);
                    (
                        own.to_string(),
                        Some(own_name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| own.to_string())),
                        own_desc.filter(|d| !d.trim().is_empty()).or_else(|| description.map(str::to_string)),
                    )
                }
                None => (
                    model.to_string(),
                    Some(name.to_string()),
                    description.map(str::to_string),
                ),
            };
            env.insert(claude_tier_env(tier).into(), serde_json::Value::String(tier_value.clone()));
            for about in CLAUDE_ABOUT {
                // 档位换模型后旧的「关于」值会指错模型，先清掉；断开时按快照还原用户的
                env.remove(&format!("{}{about}", claude_tier_env(tier)));
            }
            // /model 菜单的档位标签与描述（magpie claudeAbout 同款）；能力键不猜
            env.insert(
                format!("{}{}", claude_tier_env(tier), "_NAME"),
                serde_json::Value::String(tier_name.unwrap_or_else(|| tier_value.clone())),
            );
            if let Some(tier_description) = tier_desc {
                env.insert(format!("{}{}", claude_tier_env(tier), "_DESCRIPTION"), serde_json::Value::String(tier_description));
            }
            // D4：这一档模型能收的 effort 档告诉 Claude Code（magpie claudeCaps 同款），
            // 否则 /model 里看不到 effort 选项；不收 effort 的模型不写
            let (tier_model, _) = crate::gateway::wiring_store::split_tier_value(&tier_value);
            let capabilities = claude_capabilities(tier_model);
            if !capabilities.is_empty() {
                env.insert(format!("{}{}", claude_tier_env(tier), "_SUPPORTED_CAPABILITIES"), serde_json::Value::String(capabilities));
            }
        }
        // D2：用户设的 effortLevel 新主模型不收时，换成它收的、不高于原值的最高一档
        // （Claude Code 对不收的档会静默降级或报错）；原值在快照里，断开还原
        let fitted = object.get("effortLevel").and_then(|v| v.as_str()).and_then(|effort| fit_effort(model, effort));
        match fitted {
            Some(Some(level)) => {
                object.insert("effortLevel".into(), serde_json::Value::String(level));
            }
            Some(None) => {
                object.remove("effortLevel");
            }
            None => {}
        }
        let env = object.get_mut("env").and_then(|e| e.as_object_mut()).ok_or("settings.json 的 env 段不是对象")?;
        env.insert("ANTHROPIC_SMALL_FAST_MODEL".into(), serde_json::Value::String(model.into()));
        // 派生 env（B1 / B2 / B5 / B6 / B7）：算得出才写；算不出时只删上次我们写的旧值，用户自己的值不动
        let extra = claude_extra_env(model, extras);
        for key in CLAUDE_EXTRA_KEYS {
            match extra.get(key) {
                Some(value) => {
                    env.insert(key.into(), serde_json::Value::String(value.clone()));
                }
                None => {
                    let stale = env
                        .get(key)
                        .and_then(|v| v.as_str())
                        .is_some_and(|current| extras.previous.get(key).is_some_and(|prev| prev == current));
                    if stale {
                        env.remove(key);
                    }
                }
            }
        }
        // 主模型放 settings.json 顶层：Claude Code 自己的 /model 也改这里
        object.insert("model".into(), serde_json::Value::String(model.into()));
        // subagent 独立模型（magpie `CLAUDE_CODE_SUBAGENT_MODEL` 同款）：给了才写，没给就删键
        // ——删键等价于「跟随主模型」，那正是 Claude Code 的缺省行为。
        if let Some(env) = object.get_mut("env").and_then(|e| e.as_object_mut()) {
            match subagent_model.map(str::trim).filter(|m| !m.is_empty()) {
                Some(subagent) => {
                    env.insert("CLAUDE_CODE_SUBAGENT_MODEL".into(), serde_json::Value::String(subagent.into()));
                }
                None => {
                    env.remove("CLAUDE_CODE_SUBAGENT_MODEL");
                }
            }
            // Auto 判别模型（CLAUDE_CODE_AUTO_MODE_MODEL，magpie 2.1.302 同款）：
            // Claude Code 每个动作先跑一遍分类器，高频轻调用。「跟随子代理」由前端
            // 把子代理值填进 auto_mode——后端 None = 删键 = Claude Code 自己挑。
            match tiers.and_then(|t| t.auto_mode.as_deref()).map(str::trim).filter(|m| !m.is_empty()) {
                Some(auto) => {
                    env.insert("CLAUDE_CODE_AUTO_MODE_MODEL".into(), serde_json::Value::String(auto.into()));
                }
                None => {
                    env.remove("CLAUDE_CODE_AUTO_MODE_MODEL");
                }
            }
        }
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("settings.json 序列化失败：{error}"))
}

/// 这个值是不是上次接线写进去的。
///
/// 「关于」键（`_NAME` / `_DESCRIPTION` / `_SUPPORTED_CAPABILITIES`）：写了主模型的接线会把各档的
/// 这三个键全部清掉重写，所以只要上次写过主模型，文件里的这些键就都是 Jolu 写的（用户原值在
/// 当时的快照里）。自选档位的显示名按该档模型取，不等于主模型的 `prior_label`，只比 label 会漏。
fn written_by_us(value: &str, key: &str, prior_model: Option<&str>, prior_label: Option<&str>, prior_values: &[String]) -> bool {
    let value = value.trim();
    let about = CLAUDE_ABOUT.iter().any(|suffix| key.ends_with(suffix));
    prior_model.is_some_and(|model| value == model.trim())
        || (about && (prior_model.is_some() || prior_label.is_some_and(|label| value == label.trim())))
        || prior_values.iter().any(|written| value == written.trim())
}

/// 快照的结构：新版带 `env` 子对象与顶层 `model`；旧版是 env 键的平面表（升级兼容）。
fn claude_snapshot_is_structured(saved: &serde_json::Value) -> bool {
    // 新版快照只在用户有原值时才带对应部分：只有顶层 model / effortLevel、没有 env 键的
    // 也是新版（旧版平面表的键全是 env 变量名，不会叫 model / effortLevel）
    saved.get("env").is_some() || saved.get("model").is_some() || saved.get("effortLevel").is_some()
}

/// 还原 Claude：只删自己写的键。`saved` 是接线前读到的原值快照（新旧两种结构都认）。
///
/// `prior_model` 是上一次接线写下的主模型（取自接线记录的 applied）：没有快照时，
/// 顶层 `model` 与各档位键若等于它，就是我们写的，要删——不能把网关来源名留给用户。
#[allow(clippy::too_many_arguments)]
pub(super) fn claude_restore(
    text: &str,
    saved: Option<&serde_json::Value>,
    prior_model: Option<&str>,
    prior_label: Option<&str>,
    prior_values: &[String],
) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("settings.json 不是有效 JSON：{error}"))?;
    let Some(object) = root.as_object_mut() else { return Ok(text.to_string()) };
    let structured = saved.filter(|s| claude_snapshot_is_structured(s));
    // 平面旧快照：只有 env 键；新快照：env 子对象 + 顶层 model
    let legacy_env = saved.filter(|s| !claude_snapshot_is_structured(s));
    let env_saved = structured
        .and_then(|s| s.get("env"))
        .or(legacy_env);
    let ours = |value: &str, key: &str| -> bool {
        is_gateway_value(value) || written_by_us(value, key, prior_model, prior_label, prior_values)
    };
    let restore_env_key = |env: &mut serde_json::Map<String, serde_json::Value>, key: &str| {
        match env_saved.and_then(|s| s.get(key)).and_then(|v| v.as_str()) {
            Some(original) => {
                env.insert(key.into(), serde_json::Value::String(original.into()));
            }
            None => {
                env.remove(key);
            }
        }
    };
    if let Some(env) = object.get_mut("env").and_then(|e| e.as_object_mut()) {
        for key in claude_env_keys() {
            // 没有快照（旧版记录）时，档位键若等于我们写的主模型也删掉
            let ours_value = env_saved.is_none()
                && key != "ANTHROPIC_BASE_URL"
                && key != "ANTHROPIC_AUTH_TOKEN"
                && env.get(key.as_str()).and_then(|v| v.as_str()).is_some_and(|v| ours(v, &key));
            if ours_value {
                env.remove(key.as_str());
                continue;
            }
            restore_env_key(env, &key);
        }
        if env.is_empty() {
            object.remove("env");
        }
    }
    // 顶层 effortLevel：快照里有原值就还原（接线可能按主模型调整过它，D2）
    if let Some(effort) = structured.and_then(|s| s.get("effortLevel")).and_then(|v| v.as_str()) {
        object.insert("effortLevel".into(), serde_json::Value::String(effort.into()));
    }
    // 顶层 model：新快照按原值还原/删除；没有快照但等于我们写的主模型时删掉；
    // 旧版快照从没写过顶层 model，保持用户现状不动。
    if let Some(model_saved) = structured.and_then(|s| s.get("model")).and_then(|v| v.as_str()) {
        object.insert("model".into(), serde_json::Value::String(model_saved.into()));
    } else if structured.is_some() {
        object.remove("model");
    } else if let Some(current) = object.get("model").and_then(|v| v.as_str()) {
        if ours(current, "model") {
            object.remove("model");
        }
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("settings.json 序列化失败：{error}"))
}

fn claude_env(text: &str, key: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("env")?.get(key)?.as_str().map(str::to_string)
}

pub(crate) fn claude_current_base(text: &str) -> Option<String> {
    claude_env(text, "ANTHROPIC_BASE_URL")
}

pub(crate) fn claude_current_token(text: &str) -> Option<String> {
    claude_env(text, "ANTHROPIC_AUTH_TOKEN")
}

/// 读 Claude 接线涉及的全部键作为原值快照；网关写下的值一律不收。
///
/// `prior_model` 是上一次接线写下的主模型（取自接线记录的 applied）：重新接线时文件里的
/// 档位键 / 顶层 model 都等于它，但那不是用户的原值——收进快照会让「断开接线」把用户的
/// 模型名还原成网关来源名。
///
/// 快照结构：`{"env": {键: 原值}, "model": 顶层model原值}`；`model` 只在用户本来就有时出现。
pub(super) fn claude_snapshot(
    text: &str,
    prior_model: Option<&str>,
    prior_label: Option<&str>,
    prior_values: &[String],
) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut saved = serde_json::Map::new();
    let mut env_saved = serde_json::Map::new();
    if let Some(env) = root.get("env").and_then(|e| e.as_object()) {
        for key in claude_env_keys() {
            if let Some(value) = env.get(&key).and_then(|v| v.as_str()) {
                // 上次接线写下的值不是用户原值（主模型、子代理、各档自选、Auto、「关于」键）
                let ours = written_by_us(value, &key, prior_model, prior_label, prior_values);
                if !is_gateway_value(value) && !ours {
                    env_saved.insert(key, serde_json::Value::String(value.to_string()));
                }
            }
        }
    }
    if !env_saved.is_empty() {
        saved.insert("env".into(), serde_json::Value::Object(env_saved));
    }
    // 顶层 effortLevel：接线可能按新主模型调整它（D2），原值要能还原
    if let Some(value) = root.get("effortLevel").and_then(|v| v.as_str()) {
        saved.insert("effortLevel".into(), serde_json::Value::String(value.to_string()));
    }
    // 顶层 model：等于我们上次写的主模型时不算用户原值
    if let Some(value) = root.get("model").and_then(|v| v.as_str()) {
        let ours = prior_model.is_some_and(|model| value.trim() == model.trim());
        if !is_gateway_value(value) && !ours {
            saved.insert("model".into(), serde_json::Value::String(value.to_string()));
        }
    }
    if saved.is_empty() { None } else { Some(serde_json::Value::Object(saved)) }
}

/// Codex 有两种接法：
/// - 已登录 ChatGPT（`auth.json` 有 tokens）：保留内置 openai provider，只写顶层
///   `openai_base_url = <base>/backend-api/codex`，OAuth 头原样发到网关。
/// - 未登录 ChatGPT：写自定义 provider + `experimental_bearer_token`。
///
/// `wire_api` 必须是 `responses`：当前 Codex 只接受这个值，写 `chat` 会让它拒绝加载整份配置。
pub(super) fn codex_wire(
    text: &str,
    base: &str,
    provider: &str,
    oauth: bool,
    token: Option<&str>,
) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    if oauth {
        // 内置 provider 优先：删掉 Jolu 自己的 provider 段与 model_provider 覆盖。
        if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
            providers.remove(provider);
        }
        document.remove("model_provider");
        document["openai_base_url"] = toml_edit::value(format!("{base}/backend-api/codex"));
        return Ok(document.to_string());
    }
    if !document.contains_key("model_providers") {
        document["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 的 model_providers 不是表")?;
    let mut entry = toml_edit::Table::new();
    entry["name"] = toml_edit::value(provider);
    entry["base_url"] = toml_edit::value(base);
    entry["wire_api"] = toml_edit::value("responses");
    if let Some(token) = token {
        entry["experimental_bearer_token"] = toml_edit::value(token);
    }
    providers[provider] = toml_edit::Item::Table(entry);
    document["model_provider"] = toml_edit::value(provider);
    Ok(document.to_string())
}

/// 还原 Codex：删掉 Jolu 建的 provider 段，恢复 `openai_base_url` 与原 `model_provider`。
/// 外部改过的值不动（当前 provider 是别人的、current base 不是网关地址时跳过对应字段）。
pub(super) fn codex_restore(text: &str, saved: Option<&serde_json::Value>, provider: &str) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    let current_provider = codex_current_provider(text);
    let current_base = document.get("openai_base_url").and_then(|v| v.as_str()).map(str::to_string);
    if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
        providers.remove(provider);
    }
    // 空表头要一起清掉：`codex_wire` 在文件原本没有 `model_providers` 时是新建的，
    // 只删子项会留下一个空的 `[model_providers]`，用户断开后配置里凭空多一截。
    prune_empty_table(&mut document, "model_providers");
    // openai_base_url：只有当前仍指向本机网关（或为空）时才还原
    if current_base.as_deref().is_none_or(is_gateway_value) {
        match saved.and_then(|s| s.get("openai_base_url")).and_then(|v| v.as_str()) {
            Some(original) => {
                document["openai_base_url"] = toml_edit::value(original);
            }
            None => {
                document.remove("openai_base_url");
            }
        }
    }
    // model_provider：只有当前是我们删掉后的空值、或仍指向我们时才还原
    if current_provider.as_deref().is_none() || current_provider.as_deref() == Some(provider) {
        match saved.and_then(|s| s.get("model_provider")).and_then(|v| v.as_str()) {
            Some(original) => {
                document["model_provider"] = toml_edit::value(original);
            }
            None => {
                document.remove("model_provider");
            }
        }
    }
    Ok(document.to_string())
}

/// Codex 实际生效的 base：顶层 openai_base_url 优先，其次当前 provider 的 base_url。
pub(crate) fn codex_effective_base(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    if let Some(base) = document.get("openai_base_url").and_then(|v| v.as_str()) {
        let base = base.trim();
        if !base.is_empty() {
            return Some(base.to_string());
        }
    }
    let provider = document.get("model_provider")?.as_str()?;
    codex_provider_base(text, provider)
}

/// 是否已登录 ChatGPT（`auth.json` 里有 OAuth tokens，而不是只有 API Key）。
fn codex_oauth_logged_in() -> bool {
    let Some(dir) = crate::creds::codex_dir() else { return false };
    let Ok(text) = crate::steady::read_to_string(dir.join("auth.json")) else { return false };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    value
        .pointer("/tokens/access_token")
        .and_then(|v| v.as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

fn codex_current_provider(text: &str) -> Option<String> {
    text.parse::<toml_edit::DocumentMut>()
        .ok()?
        .get("model_provider")?
        .as_str()
        .map(str::to_string)
}

fn codex_provider_base(text: &str, provider: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    document
        .get("model_providers")?
        .get(provider)?
        .get("base_url")?
        .as_str()
        .map(str::to_string)
}

/// 读 Codex 的 `openai_base_url` 与当前 provider 作为原值快照；网关值一律不收。
pub(super) fn codex_snapshot(text: &str) -> Option<serde_json::Value> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let mut saved = serde_json::Map::new();
    if let Some(base) = document.get("openai_base_url").and_then(|v| v.as_str()) {
        if !is_gateway_value(base) {
            saved.insert("openai_base_url".into(), serde_json::Value::String(base.to_string()));
        }
    }
    if let Some(current) = document.get("model_provider").and_then(|v| v.as_str()) {
        if current != CODEX_PROVIDER
            && !codex_provider_base(text, current).is_some_and(|base| is_gateway_value(&base))
        {
            saved.insert("model_provider".into(), serde_json::Value::String(current.to_string()));
        }
    }
    if saved.is_empty() { None } else { Some(serde_json::Value::Object(saved)) }
}

/// OpenCode：`opencode.json` 的 `provider` 段。JSON 往返保留全部字段。
pub(super) fn opencode_wire(text: &str, base: &str, token: &str) -> Result<String, String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?
    };
    let object = root.as_object_mut().ok_or("opencode 配置顶层不是对象")?;
    let providers = object.entry("provider").or_insert_with(|| serde_json::json!({}));
    let providers = providers.as_object_mut().ok_or("provider 段不是对象")?;
    providers.insert(
        "jolu".into(),
        serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "Jolu 网关",
            "options": {"baseURL": base, "apiKey": token},
        }),
    );
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

pub(super) fn opencode_restore(text: &str, _saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove("jolu");
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

pub(crate) fn opencode_current_base(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("baseURL")?.as_str().map(str::to_string)
}

pub(crate) fn opencode_current_token(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("apiKey")?.as_str().map(str::to_string)
}

/// Gemini CLI：交互式按 `settings.json` 的 `security.auth.selectedType` 选鉴权。
/// 只写 base URL 对 Google 登录用户无效，所以这里做事务式接管：
/// `.env` 写地址 + 网关 Key，`settings.json` 临时把 selectedType 切成 `gemini-api-key`；
/// 断开时按快照逐项还原。OAuth 凭据文件本身不碰。
#[derive(Debug, Clone)]
struct GeminiPlan {
    env: String,
    settings: String,
    snapshot: serde_json::Value,
}

fn gemini_settings_path() -> Option<PathBuf> {
    Some(crate::creds::home()?.join(".gemini").join("settings.json"))
}

pub(crate) fn gemini_selected_type(settings_text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(settings_text)
        .ok()?
        .pointer("/security/auth/selectedType")?
        .as_str()
        .map(str::to_string)
}

fn gemini_plan(env_text: &str, settings_text: &str, base: &str, token: &str) -> Result<GeminiPlan, String> {
    let mut env_saved = serde_json::Map::new();
    for key in ["GOOGLE_GEMINI_BASE_URL", "GEMINI_API_KEY"] {
        if let Some(value) = dotenv_get(env_text, key).filter(|value| !is_gateway_value(value)) {
            env_saved.insert(key.into(), serde_json::Value::String(value));
        }
    }
    let mut settings_saved = serde_json::Map::new();
    if let Some(selected) = gemini_selected_type(settings_text) {
        settings_saved.insert("selectedType".into(), serde_json::Value::String(selected));
    }
    // 即使原来什么都没有也要记快照：断开时才知道「这两个键是我们加的，要删掉」。
    let snapshot = serde_json::json!({
        "env": serde_json::Value::Object(env_saved),
        "settings": serde_json::Value::Object(settings_saved),
    });
    let env = wire_dotenv(
        &wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", Some(base)),
        "GEMINI_API_KEY",
        Some(token),
    );
    let mut settings: serde_json::Value = if settings_text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(settings_text)
            .map_err(|error| format!("~/.gemini/settings.json 不是有效 JSON，未改动：{error}"))?
    };
    let object = settings.as_object_mut().ok_or("~/.gemini/settings.json 顶层不是对象，未改动")?;
    let security = object.entry("security").or_insert_with(|| serde_json::json!({}));
    let security = security
        .as_object_mut()
        .ok_or("~/.gemini/settings.json 的 security 段不是对象，未改动")?;
    let auth = security.entry("auth").or_insert_with(|| serde_json::json!({}));
    let auth = auth
        .as_object_mut()
        .ok_or("~/.gemini/settings.json 的 security.auth 段不是对象，未改动")?;
    auth.insert("selectedType".into(), serde_json::Value::String("gemini-api-key".into()));
    Ok(GeminiPlan {
        env,
        settings: serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings_text.to_string()),
        snapshot,
    })
}

/// 还原 `.env` 与 settings.json；旧版快照（只有 `value`）只还原地址，不碰 settings。
fn gemini_restore(env_text: &str, settings_text: &str, saved: Option<&serde_json::Value>) -> (String, String) {
    let legacy_base = saved.and_then(|s| s.get("value")).and_then(|v| v.as_str());
    let saved_env = saved.and_then(|s| s.get("env"));
    let saved_settings = saved.and_then(|s| s.get("settings"));
    let env = if let Some(env) = saved_env {
        let base = env.get("GOOGLE_GEMINI_BASE_URL").and_then(|v| v.as_str());
        let key = env.get("GEMINI_API_KEY").and_then(|v| v.as_str());
        let next = wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", base);
        wire_dotenv(&next, "GEMINI_API_KEY", key)
    } else {
        wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", legacy_base)
    };
    let settings = match saved_settings {
        Some(saved_settings) => {
            let mut settings: serde_json::Value = if settings_text.trim().is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(settings_text) {
                    Ok(value) => value,
                    // 文件坏了就不写回，交给用户自己修
                    Err(_) => return (env, settings_text.to_string()),
                }
            };
            let selected = saved_settings.get("selectedType").and_then(|v| v.as_str());
            let auth = settings
                .pointer_mut("/security/auth")
                .and_then(|v| v.as_object_mut());
            if let Some(auth) = auth {
                match selected {
                    Some(value) => {
                        auth.insert("selectedType".into(), serde_json::Value::String(value.to_string()));
                    }
                    None => {
                        auth.remove("selectedType");
                    }
                }
            }
            serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings_text.to_string())
        }
        None => settings_text.to_string(),
    };
    (env, settings)
}

pub(crate) fn gemini_current_base(text: &str) -> Option<String> {
    dotenv_get(text, "GOOGLE_GEMINI_BASE_URL")
}

pub(crate) fn gemini_current_token(text: &str) -> Option<String> {
    dotenv_get(text, "GEMINI_API_KEY")
}

fn gemini_settings_text() -> Option<String> {
    crate::steady::read_to_string(gemini_settings_path()?).ok()
}

/// ZCode：`provider` JSON 的 `options.baseURL` + `apiKey`（结构对齐 magpie zcode.go 的字段名）。
/// 接给 ZCode 的一个网关模型（2.9）。
#[derive(Debug, Clone, PartialEq)]
pub struct ZcodeModel {
    /// 网关模型 id（`来源/模型` 或 `group/分组`）
    pub id: String,
    pub name: String,
    /// 上下文窗口；0 = 不知道
    pub context: i64,
    /// 最大输出；0 = 不知道（不写，ZCode 按自己的 32000 截断）
    pub output: i64,
    pub image: bool,
}

/// ZCode 规则与 config.json 里 Jolu 的 provider id。
const ZCODE_PROVIDER: &str = "jolu";
/// 上下文不知道时写的值：ZCode 要一个数，magpie 同样退回 200000。
const ZCODE_DEFAULT_CONTEXT: i64 = 200_000;

/// ZCode 的数据目录：与 Agent 探测同源（`creds::zcode_data_dir`：`ZCODE_HOME` 覆盖 →
/// ZCode 主目录 → 设置里改过的「数据存储路径」`dataBaseDir`），保证探测到的就是写的。
pub(crate) fn zcode_dir_public(home: &Path) -> PathBuf { zcode_dir(home) }

fn zcode_dir(home: &Path) -> PathBuf {
    crate::creds::zcode_data_dir(home)
}

fn parse_object(text: &str, what: &str) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| format!("{what}不是有效 JSON：{error}"))?;
    if !value.is_object() {
        return Err(format!("{what}顶层不是对象"));
    }
    Ok(value)
}

/// 在对象里按路径取子对象，沿途缺的补空对象；路径上有非对象值时报错（不覆盖用户数据）。
fn object_path<'a>(value: &'a mut serde_json::Value, path: &[&str], what: &str) -> Result<&'a mut serde_json::Map<String, serde_json::Value>, String> {
    let mut current = value;
    for key in path {
        let object = current.as_object_mut().ok_or_else(|| format!("{what}里「{key}」的上级不是对象"))?;
        current = object.entry(key.to_string()).or_insert_with(|| serde_json::json!({}));
    }
    current.as_object_mut().ok_or_else(|| format!("{what}里「{}」不是对象", path.join(".")))
}

fn array_at<'a>(object: &'a mut serde_json::Map<String, serde_json::Value>, key: &str, what: &str) -> Result<&'a mut Vec<serde_json::Value>, String> {
    object
        .entry(key.to_string())
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("{what}里「{key}」不是数组"))
}

fn is_ours(rule: &serde_json::Value) -> bool {
    rule.get("providerId").and_then(|v| v.as_str()) == Some(ZCODE_PROVIDER)
}

/// 新版 ZCode（3.14 起）的 `v2/provider_config.json`：provider 规则 + 每个模型的规则。
///
/// 为什么每个模型都要规则：ZCode 不知道模型的输出上限时把每次回复截到 32000 token，
/// 长输出会在中途断掉；上下文窗口与是否收图片也靠模型规则告诉它。用户在 ZCode 里手动设过
/// 的模型（`manualProviderModelRules`）不动。参照 magpie `internal/agent/zcode.go`。
fn zcode_rules_wire(text: &str, base: &str, token: &str, models: &[ZcodeModel]) -> Result<String, String> {
    let what = "ZCode 的 provider_config.json ";
    let mut root = parse_object(text, what)?;
    if root.get("schemaVersion").is_none() {
        root["schemaVersion"] = serde_json::json!(1);
    }
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    {
        let config = object_path(&mut root, &["config"], what)?;
        let order = array_at(config, "providerOrder", what)?;
        if !order.iter().any(|v| v.as_str() == Some(ZCODE_PROVIDER)) {
            order.push(serde_json::json!(ZCODE_PROVIDER));
        }
    }
    {
        let rules = object_path(&mut root, &["config", "providerConfigRules"], what)?;
        let list = array_at(rules, "providerRules", what)?;
        list.retain(|rule| !is_ours(rule));
        list.push(serde_json::json!({
            "providerId": ZCODE_PROVIDER,
            "providerName": "Jolu",
            "enabled": true,
            "config": {
                "group": "standard-personal",
                "access": {"type": "api-key", "apiKey": token},
                "api": {"type": "anthropic-messages", "baseUrl": base},
                "personalModelIds": ids,
                "modelOrder": ids,
            },
        }));
    }
    {
        let rules = object_path(&mut root, &["config", "modelConfigRules"], what)?;
        array_at(rules, "manualProviderModelRules", what)?;
        let list = array_at(rules, "providerModelRules", what)?;
        list.retain(|rule| !is_ours(rule));
        for model in models {
            let mut config = serde_json::json!({
                "properties": {
                    "contextWindow": if model.context > 0 { model.context } else { ZCODE_DEFAULT_CONTEXT },
                    "inputFormat": {"supportsImage": model.image},
                },
            });
            if model.output > 0 {
                config["optionSpecs"] = serde_json::json!({"maxOutputTokens": {"max": model.output}});
            }
            list.push(serde_json::json!({"providerId": ZCODE_PROVIDER, "modelId": model.id, "config": config}));
        }
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("{what}序列化失败：{error}"))
}

/// 旧版 ZCode 读的 `v2/config.json`：OpenCode 形状的 provider。
fn zcode_config_wire(text: &str, base: &str, token: &str, models: &[ZcodeModel]) -> Result<String, String> {
    let what = "ZCode 的 config.json ";
    let mut root = parse_object(text, what)?;
    let mut entries = serde_json::Map::new();
    for model in models {
        let mut limit = serde_json::json!({"context": if model.context > 0 { model.context } else { ZCODE_DEFAULT_CONTEXT }});
        if model.output > 0 {
            limit["output"] = serde_json::json!(model.output);
        }
        let input = if model.image { serde_json::json!(["text", "image"]) } else { serde_json::json!(["text"]) };
        entries.insert(model.id.clone(), serde_json::json!({
            "name": model.name,
            "limit": limit,
            "modalities": {"input": input, "output": ["text"]},
        }));
    }
    let providers = object_path(&mut root, &["provider"], what)?;
    // 用户在 ZCode 里关掉过的保持关闭
    let enabled = providers.get(ZCODE_PROVIDER).and_then(|p| p.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(true);
    providers.insert(ZCODE_PROVIDER.into(), serde_json::json!({
        "name": "Jolu",
        "kind": "anthropic",
        "enabled": enabled,
        "source": "custom",
        "options": {"apiKey": token, "baseURL": base},
        "models": entries,
    }));
    serde_json::to_string_pretty(&root).map_err(|error| format!("{what}序列化失败：{error}"))
}

/// ZCode 规则文件的快照：记下接线前**哪几处结构本来就有**。
///
/// 不存整份原文是因为 restore 仍要在「用户后来自己改过」的文件上操作（外部意图优先），
/// 只需要知道「这些壳是不是我们建的」——三处新建 + schemaVersion 一个布尔就够了。
fn zcode_rules_snapshot(text: &str) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(serde_json::json!({
        "schemaVersion": root.get("schemaVersion").is_some(),
        "providerOrder": root.pointer("/config/providerOrder").is_some(),
        "providerRules": root.pointer("/config/providerConfigRules/providerRules").is_some(),
        "modelConfigRules": root.pointer("/config/modelConfigRules").is_some(),
    }))
}

fn zcode_rules_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root = parse_object(text, "ZCode 的 provider_config.json ")?;
    // `saved` 里是接线前「这几处是否存在」的布尔；没有快照（旧版本接的线）时按
    // 「都存在」处理——宁可留着空壳，也不擅自删用户的键。
    let had = |key: &str| saved.and_then(|saved| saved.get(key)).and_then(|v| v.as_bool()).unwrap_or(true);
    if let Some(order) = root.pointer_mut("/config/providerOrder").and_then(|o| o.as_array_mut()) {
        order.retain(|v| v.as_str() != Some(ZCODE_PROVIDER));
    }
    if let Some(list) = root.pointer_mut("/config/providerConfigRules/providerRules").and_then(|l| l.as_array_mut()) {
        list.retain(|rule| !is_ours(rule));
    }
    if let Some(list) = root.pointer_mut("/config/modelConfigRules/providerModelRules").and_then(|l| l.as_array_mut()) {
        list.retain(|rule| !is_ours(rule));
    }
    // 接线时新建的空壳要收掉，理由见 `zcode_rules_snapshot`。
    if !had("providerOrder") {
        prune_empty_json_at(&mut root, &["config", "providerOrder"]);
    }
    if !had("providerRules") {
        prune_empty_json_at(&mut root, &["config", "providerConfigRules", "providerRules"]);
    }
    if !had("modelConfigRules") {
        prune_empty_json_at(&mut root, &["config", "modelConfigRules", "manualProviderModelRules"]);
        prune_empty_json_at(&mut root, &["config", "modelConfigRules", "providerModelRules"]);
        prune_empty_json_object(&mut root, &["config", "modelConfigRules"]);
    }
    if !had("schemaVersion") && root.get("schemaVersion").and_then(|v| v.as_i64()) == Some(1) {
        if let Some(object) = root.as_object_mut() {
            object.remove("schemaVersion");
        }
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

/// 删掉一个空数组键（路径不存在或不是空数组则不动）。
fn prune_empty_json_at(root: &mut serde_json::Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else { return };
    let Some(parent) = parents.iter().try_fold(root, |node, key| node.get_mut(*key)) else { return };
    let empty = parent.get(*last).and_then(|v| v.as_array()).is_some_and(|a| a.is_empty());
    if empty {
        if let Some(object) = parent.as_object_mut() {
            object.remove(*last);
        }
    }
}

/// 删掉一个空对象键（路径不存在或不是空对象则不动）。
fn prune_empty_json_object(root: &mut serde_json::Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else { return };
    let Some(parent) = parents.iter().try_fold(root, |node, key| node.get_mut(*key)) else { return };
    let empty = parent.get(*last).and_then(|v| v.as_object()).is_some_and(|o| o.is_empty());
    if empty {
        if let Some(object) = parent.as_object_mut() {
            object.remove(*last);
        }
    }
}

fn zcode_config_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root = parse_object(text, "ZCode 的 config.json ")?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove(ZCODE_PROVIDER);
    }
    if saved.and_then(|value| value.get("configProvider")).and_then(|value| value.as_bool()) == Some(false) {
        prune_empty_json_object(&mut root, &["provider"]);
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

/// 旧版 Jolu 在 `~/.zcode/provider.json` 顶层写过 `jolu` 键（ZCode 从不读这个文件）：断开时顺带删掉。
pub(crate) fn zcode_clean_legacy(home: &Path) {
    // 旧版 Jolu 写在原位置（`ZCODE_HOME` 或 `~/.zcode`），不随 dataBaseDir 找
    let legacy = std::env::var_os("ZCODE_HOME").filter(|value| !value.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".zcode")).join("provider.json");
    let Ok(Some(text)) = crate::steady::read_optional(&legacy) else { return };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let Some(object) = root.as_object_mut() else { return };
    if object.remove(ZCODE_PROVIDER).is_none() {
        return;
    }
    if object.is_empty() {
        let _ = std::fs::remove_file(&legacy);
    } else if let Ok(next) = serde_json::to_string_pretty(&root) {
        let _ = write_atomic(&legacy, next.into_bytes());
    }
}

fn zcode_rule(text: &str) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.pointer("/config/providerConfigRules/providerRules")?
        .as_array()?
        .iter()
        .find(|rule| is_ours(rule))
        .cloned()
}

pub(crate) fn zcode_current_base(text: &str) -> Option<String> {
    zcode_rule(text)?.pointer("/config/api/baseUrl")?.as_str().map(str::to_string)
}

pub(crate) fn zcode_current_token(text: &str) -> Option<String> {
    zcode_rule(text)?.pointer("/config/access/apiKey")?.as_str().map(str::to_string)
}

/* ─────────────────────────── Kimi CLI / Kimi Code（config.toml） ───────────────────────────
 * 参照 magpie `internal/agent/kimi.go`。Kimi 的 config.toml 里：
 *   - `providers.<名字>`：上游供应商（base_url / api_key / type 等）；
 *   - `models."<本地名>"`：模型表——`provider` 指到上面的供应商、`model` 是发给上游的模型名、
 *     `max_context_size` 告诉 Kimi 什么时候压缩上下文；`/model` 选择器与状态栏显示的是本地名。
 * 接线 = 写 `providers.jolu` + 一组 `models."jolu/<网关模型 id>"` 表 + `default_model`。
 * 本地名带 `jolu/` 前缀只是为了断开时能按前缀找到我们写的表（用户不会起这种名字），
 * 发给上游的 `model` 用的是表里的 `model` 字段（网关可路由的 `来源/模型`），前缀不出本机。
 */

/// Kimi 的 provider 段名与模型表前缀（同 magpie 的 magpieID）。
const KIMI_PROVIDER: &str = "jolu";
/// 上下文不知道时写的值：Kimi 必须要一个数，超了它会压缩会话（magpie kimiContext）。
const KIMI_DEFAULT_CONTEXT: i64 = 128_000;

/// Kimi 的配置目录，附带是否旧版：新版 Kimi Code（TypeScript 2.x）读 `~/.kimi-code`
/// （`KIMI_CODE_HOME` 覆盖），旧版 Python kimi-cli 读 `~/.kimi`（`KIMI_SHARE_DIR` 覆盖）；
/// 新版目录存在时旧的不再被读（magpie #290）。两处都不存在时默认写到 `~/.kimi-code`
/// （首次接线时代码会建目录）；探测侧 [`crate::agent_discovery`] 的同名逻辑保持一致。
/// 与 [`zcode_dir`] 同理：接线需要「legacy」这个探测规格表没有的信息（旧版 kimi-cli
/// 不认 `capabilities` 里的 `tool_use`，写了会拒绝启动），所以单独再算一遍目录。
pub(crate) fn kimi_dir_public(home: &Path) -> (PathBuf, bool) { kimi_dir(home) }

fn kimi_dir(home: &Path) -> (PathBuf, bool) {
    if let Some(dir) = std::env::var_os("KIMI_CODE_HOME").filter(|value| !value.is_empty()) {
        return (PathBuf::from(dir), false);
    }
    let code = home.join(".kimi-code");
    if code.is_dir() {
        return (code, false);
    }
    if let Some(dir) = std::env::var_os("KIMI_SHARE_DIR").filter(|value| !value.is_empty()) {
        return (PathBuf::from(dir), true);
    }
    (home.join(".kimi"), true)
}

/// 读 `providers.jolu` 表（只读；状态判定用）；不存在或不是表时返回 None。
fn kimi_provider_read<'a>(document: &'a toml_edit::DocumentMut) -> Option<&'a toml_edit::Table> {
    document.get("providers")?.as_table()?.get(KIMI_PROVIDER)?.as_table()
}

pub(crate) fn kimi_current_base(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let base = kimi_provider_read(&document)?.get("base_url")?.as_str()?.trim().to_string();
    (!base.is_empty()).then_some(base)
}

pub(crate) fn kimi_current_token(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let token = kimi_provider_read(&document)?.get("api_key")?.as_str()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// 顶层 default_model 是否指向我们的模型表（`jolu/` 前缀）。
/// 文本版的「默认模型指向我们的表」判定（注册表用，避免调用方各自解析 TOML）。
pub(crate) fn kimi_default_model_ours(text: &str) -> bool {
    text.parse::<toml_edit::DocumentMut>().map(|document| kimi_default_ours(&document)).unwrap_or(false)
}

pub(crate) fn kimi_default_ours(document: &toml_edit::DocumentMut) -> bool {
    document
        .get("default_model")
        .and_then(|value| value.as_str())
        .is_some_and(|model| model.trim().starts_with(&format!("{KIMI_PROVIDER}/")))
}

/// 接线：写 providers.jolu（就地更新，保留用户自加的 custom_headers 等）+ 重建全部
/// `models."jolu/..."` 表 + 设 default_model。返回新文件内容；快照见 [`kimi_snapshot`]。
/// 参照 magpie `kimiIn` 的 writeMagpie / kimiModelTables / setDefault。
/// 与 magpie 的差异：magpie 用 stash 记住「上次写过的值」来区分用户手改（kimiOwn），
/// Jolu 第一版简化为「表里已有的 display_name / max_output_size 一律保留」——
/// 效果是用户手改不丢，代价是目录改名后旧显示名会留着，直到断开重接。
fn kimi_wire(
    text: &str,
    base: &str,
    token: &str,
    model: Option<&str>,
    models: &[ZcodeModel],
    legacy: bool,
) -> Result<String, String> {
    if models.is_empty() {
        return Err("网关模型清单为空；先在「模型池」里启用要提供给 Kimi 的模型".into());
    }
    let mut document = parse_toml_document(text, "config.toml")?;
    // 快照（断开时还原的原值）由 [`kimi_snapshot`] 单独算，这里只管写。
    // providers.jolu：段是表时就地更新（magpie kimiProvider：整表重写会把
    // 用户设的 custom_headers、env 以及挂在 provider 上的 default_model 冲掉，magpie #2030）；
    // jolu 键本身不是表（配置写坏）则换成表；providers 段整个不在（首次接线）才重建段。
    // 写坏的场景文件在磁盘上有备份（commit_plan 的 backup），不会丢数据
    if document.get("providers").map(|value| value.is_table()).unwrap_or(false) {
        let providers = document.get_mut("providers").and_then(|value| value.as_table_mut()).expect("上面判定过是表");
        if !providers.get(KIMI_PROVIDER).map(|value| value.is_table()).unwrap_or(false) {
            providers.insert(KIMI_PROVIDER, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let table = providers.get_mut(KIMI_PROVIDER).and_then(|value| value.as_table_mut()).expect("上一步刚换成表");
        table["type"] = toml_edit::value("kimi");
        table["base_url"] = toml_edit::value(base);
        table["api_key"] = toml_edit::value(token);
    } else {
        let mut table = toml_edit::Table::new();
        table["type"] = toml_edit::value("kimi");
        table["base_url"] = toml_edit::value(base);
        table["api_key"] = toml_edit::value(token);
        let mut providers = toml_edit::Table::new();
        providers.insert(KIMI_PROVIDER, toml_edit::Item::Table(table));
        document.insert("providers", toml_edit::Item::Table(providers));
    }
    let prefix = format!("{KIMI_PROVIDER}/");
    // 旧 `jolu/` 表里用户保留值（display_name / max_output_size）先收走——下面要整表删掉重写
    let mut kept: std::collections::HashMap<String, (Option<String>, Option<i64>)> = std::collections::HashMap::new();
    if let Some(models_table) = document.get_mut("models").and_then(|value| value.as_table_mut()) {
        for (name, item) in models_table.iter() {
            if !name.starts_with(&prefix) {
                continue;
            }
            let display = item.get("display_name").and_then(|value| value.as_str()).map(str::to_string).filter(|value| !value.is_empty());
            let output = item.get("max_output_size").and_then(|value| value.as_str()).and_then(|value| value.trim().replace('_', "").parse::<i64>().ok()).filter(|value| *value > 0);
            kept.insert(name.to_string(), (display, output));
        }
    }
    // 模型表：删掉全部旧 `jolu/` 表再重写——目录里模型改名或停用后，旧表不能残留
    if let Some(models_table) = document.get_mut("models").and_then(|value| value.as_table_mut()) {
        let ours: Vec<String> = models_table
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with(&prefix))
            .collect();
        for name in ours {
            models_table.remove(&name);
        }
    }
    // `models` 段不存在就建；是别的类型（配置写坏）时整体换成表
    if !document.get("models").map(|value| value.is_table()).unwrap_or(false) {
        document.insert("models", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let models_table = document.get_mut("models").and_then(|value| value.as_table_mut()).expect("上一步刚建成表");
    for entry in models {
        // 本地名 = `jolu/<网关模型 id>`；发给上游的 model 字段 = 网关模型 id 本身，
        // Kimi 按 model 字段发请求，网关按 `来源/模型` 选路。表键的引号交给 toml_edit 处理
        let name = format!("{prefix}{}", entry.id);
        let old = kept.get(&name).cloned().unwrap_or((None, None));
        let mut table = toml_edit::Table::new();
        table["provider"] = toml_edit::value(KIMI_PROVIDER);
        table["model"] = toml_edit::value(&entry.id);
        table["max_context_size"] = toml_edit::value(if entry.context > 0 { entry.context } else { KIMI_DEFAULT_CONTEXT });
        // /model 选择器与状态栏显示的名字：目录给了别称才写（等于 id 时 Kimi 自己会显示 id）；
        // 旧表里已有值（用户手改或上次接线写的）优先保留，见函数头注释的 magpie 差异
        if entry.name != entry.id {
            let display = old.0.unwrap_or_else(|| entry.name.clone());
            table["display_name"] = toml_edit::value(display);
        }
        // capabilities：旧版 kimi-cli 不认 tool_use（写了拒绝启动）；Jolu 目录暂无 thinking
        // 档数据，所以不写 thinking（magpie 会在模型有推理档位时加）
        let mut capabilities: Vec<String> = Vec::new();
        if entry.image {
            capabilities.push("image_in".into());
        }
        if !legacy {
            capabilities.push("tool_use".into());
        }
        if !capabilities.is_empty() {
            let mut array = toml_edit::Array::new();
            for item in &capabilities {
                array.push(item.as_str());
            }
            table["capabilities"] = toml_edit::Item::Value(toml_edit::Value::Array(array));
        }
        // 输出上限：新版 Kimi Code 会拿它发 max_completion_tokens（kimi-cli 没有这个键）。
        // 旧表里已有合法整数就保留；别的值（字符串等）按没设处理，写我们的（magpie kimiInt）
        if !legacy && entry.output > 0 {
            table["max_output_size"] = toml_edit::value(old.1.unwrap_or(entry.output));
        }
        models_table.insert(&name, toml_edit::Item::Table(table));
    }
    // default_model：UI 指定的优先（`jolu/<id>`）；其次保留仍在我们清单里的旧值；
    // 都没有（首次接线、或用户原来指着自己的供应商）就用第一个——接线就是把流量引到网关
    let chosen = model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| format!("{prefix}{model}"))
        .or_else(|| {
            let current = document.get("default_model").and_then(|value| value.as_str()).map(str::trim).map(str::to_string);
            current.filter(|value| value.starts_with(&prefix) && models.iter().any(|entry| format!("{prefix}{}", entry.id) == *value))
        })
        .unwrap_or_else(|| format!("{prefix}{}", models[0].id));
    document["default_model"] = toml_edit::value(chosen);
    Ok(document.to_string())
}

/// 断开：删全部 `jolu/` 模型表与 providers.jolu；default_model 是我们的才还原成
/// 快照里的原值，且原模型表还在才写回（表没了会变成悬空引用，magpie ownModel 同款判断）。
/// 当前 default_model 已被用户改成别的（非 `jolu/`）时不动——外部意图优先。
fn kimi_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut document = parse_toml_document(text, "config.toml")?;
    if let Some(models_table) = document.get_mut("models").and_then(|value| value.as_table_mut()) {
        let ours: Vec<String> = models_table
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with(&format!("{KIMI_PROVIDER}/")))
            .collect();
        for name in ours {
            models_table.remove(&name);
        }
    }
    if let Some(providers) = document.get_mut("providers").and_then(|value| value.as_table_mut()) {
        providers.remove(KIMI_PROVIDER);
    }
    if kimi_default_ours(&document) {
        let original = saved.and_then(|saved| saved.get("default_model")).and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty());
        // 原来的默认模型得还有表可指（用户可能已把那张表删了）
        let exists = original
            .map(|model| document.get("models").and_then(|value| value.get(model)).is_some())
            .unwrap_or(false);
        match (original, exists) {
            (Some(model), true) => document["default_model"] = toml_edit::value(model),
            _ => {
                document.remove("default_model");
            }
        }
    }
    // 空表头一起清掉，理由同 `codex_restore`：kimi_wire 在文件原本没有这两张表时会新建。
    prune_empty_table(&mut document, "models");
    prune_empty_table(&mut document, "providers");
    Ok(document.to_string())
}

/// 快照：接线前用户自己的 default_model（是我们的值则不用存，断开时按「原来没有」处理）。
fn kimi_snapshot(text: &str) -> Option<serde_json::Value> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    if kimi_default_ours(&document) {
        return None;
    }
    let model = document.get("default_model").and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty())?;
    Some(serde_json::json!({ "default_model": model }))
}

/// TOML 解析的公共入口：空文件按空文档处理（首次接线还没生成 config.toml）。
fn parse_toml_document(text: &str, what: &str) -> Result<toml_edit::DocumentMut, String> {
    if text.trim().is_empty() {
        return Ok(toml_edit::DocumentMut::new());
    }
    text.parse::<toml_edit::DocumentMut>().map_err(|error| format!("{what} 格式无效：{error}"))
}

/// 删掉一个已经空掉的顶层表（只删表头本身，不留 `[name]` 空壳）。
///
/// 为什么需要：`codex_wire` / `kimi_wire` 在文件原本没有目标表时会**新建**一张，
/// 断开时只删我们写进去的子项，表就空着了。用户看到的是一段凭空多出来的
/// `[model_providers]` / `[models]`——语义无害但明显是「没还原干净」。
/// 只处理顶层表：子表（`[a.b]`）由它自己的写入方负责。
fn prune_empty_table(document: &mut toml_edit::DocumentMut, key: &str) {
    let empty = document
        .get(key)
        .and_then(|item| item.as_table())
        .is_some_and(|table| table.is_empty());
    if empty {
        document.remove(key);
    }
}

/// dotenv 键值写入：保留其它行与注释，只替换目标键（不存在则追加）。
pub(crate) fn wire_dotenv(text: &str, key: &str, value: Option<&str>) -> String {
    let prefix = format!("{key}=");
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        if line.trim_start().starts_with(&prefix) {
            if let Some(value) = value {
                out.push(format!("{prefix}{value}"));
            }
            replaced = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !replaced {
        if let Some(value) = value {
            out.push(format!("{prefix}{value}"));
        }
    }
    let mut joined = out.join("\n");
    if !joined.is_empty() && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

pub(crate) fn dotenv_get(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key}=")))
        .map(|v| v.trim().trim_matches('"').to_string())
}

/// 判断当前配置值是否指向本机网关端口（用于「外部已改就不覆盖」）。
pub(crate) fn points_at_gateway(value: &str, port: u16) -> bool {
    let value = value.trim().trim_end_matches('/');
    value.starts_with(&format!("http://127.0.0.1:{port}"))
        || value.starts_with(&format!("http://localhost:{port}"))
}

/* ─────────────────────────── 文件层：路径、备份、原子写 ─────────────────────────── */

/// Jolu 建的 Codex provider 段名。
pub(super) const CODEX_PROVIDER: &str = "jolu-gateway";

/// `jolu-<agent>.<实例前 8 位>`：一键接线的归属标记，独立于可刷新的手动 Key。
pub(crate) fn token_for(agent: &str, instance_short: &str) -> String {
    format!("jolu-{agent}.{instance_short}")
}

/// 各 agent 的配置文件路径（探测与写入共用）。
/// library（批 5）与卸载钩子用的公开包装：同 [`config_path`]。
pub fn config_path_for(agent: &str) -> Option<PathBuf> {
    config_path(agent)
}

/// library（批 5）用的公开原子写：临时文件 + rename（同 [`write_atomic`]）。
pub fn write_json_atomic_public(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    write_atomic(path, bytes)
}

/// 配置路径：查注册表（一家一行声明在 `wiring_specs::SPECS`）。
fn config_path(agent: &str) -> Option<PathBuf> {
    let home = crate::creds::home()?;
    (super::wiring_specs::spec(agent)?.path)(&home)
}

fn write_atomic(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension("jolu-wiring.tmp");
    let previous = crate::steady::read(path).ok();
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("写入 {} 失败：{error}", path.display())
    })?;
    // 2.10 写后校验：读回来要与写入的一致、且仍是合法的 JSON / TOML；不对就放回原内容并报错。
    // 为什么：rename 成功不代表 agent 读到的是我们写的（同时有别的程序改写、杀毒软件拦截等），
    // 写坏的配置会让工具整个起不来，必须当场发现
    if let Err(reason) = verify_written(path, &bytes) {
        match previous {
            Some(previous) => {
                let _ = std::fs::write(path, previous);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        return Err(format!("写入 {} 后校验失败，已放回原内容：{reason}", path.display()));
    }
    Ok(())
}

/// 读回刚写的文件：内容要一致，按扩展名重新解析一遍。
fn verify_written(path: &Path, expected: &[u8]) -> Result<(), String> {
    let actual = crate::steady::read(path).map_err(|error| format!("读回失败：{error}"))?;
    if actual != expected {
        return Err("读回的内容与写入的不一致（可能有别的程序同时改写）".into());
    }
    let text = String::from_utf8_lossy(&actual);
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") if !text.trim().is_empty() => {
            serde_json::from_str::<serde_json::Value>(&text).map_err(|error| format!("不是有效 JSON：{error}"))?;
        }
        Some("toml") => {
            text.parse::<toml_edit::DocumentMut>().map_err(|error| format!("不是有效 TOML：{error}"))?;
        }
        _ => {}
    }
    Ok(())
}

/// 写前备份到应用数据目录（沿用 cli_apply 的纪律：备份失败则不写）。
pub(crate) fn backup(path: &Path, backup_dir: &Path) -> Result<(), String> {
    if !crate::steady::exists(&path) {
        return Ok(());
    }
    std::fs::create_dir_all(backup_dir).map_err(|error| format!("创建备份目录失败：{error}"))?;
    let name = format!(
        "{}-{}.bak",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("config"),
        chrono::Local::now().format("%Y%m%d%H%M%S")
    );
    std::fs::copy(path, backup_dir.join(name)).map_err(|error| format!("备份 {path:?} 失败：{error}"))?;
    Ok(())
}

/// 别的 Jolu 实例写下的标记：token 的实例后缀与当前实例不同。
fn foreign_marker(agent: &str, text: &str, instance_short: &str) -> Option<String> {
    let spec = super::wiring_specs::spec(agent)?;
    let home = crate::creds::home()?;
    let token = (spec.token)(&home, text)?;
    if !token.starts_with("jolu-") {
        return None;
    }
    let (_, suffix) = token.rsplit_once('.')?;
    (suffix != instance_short).then(|| suffix.to_string())
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
/// 当前配置是否由本实例写入（状态判断用）。
fn ours(agent: &str, text: &str, port: u16, instance_short: &str) -> bool {
    ours_with(agent, text, port, instance_short, None)
}

/// `kept` 是接线时保留下来的调用方 Key（keyHolds）：文件里是它也算我们的接线。
fn ours_with(agent: &str, text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
    let Some(spec) = super::wiring_specs::spec(agent) else { return false };
    let Some(home) = crate::creds::home() else { return false };
    spec.is_wired(&home, text, port, instance_short, kept)
}

/// 探测某 agent 的当前接入状态（只看文件）。
pub fn status(agent: &str, port: u16) -> WiringStatus {
    status_with(agent, port, None)
}

/// 带接线记录的探测：pending 要单独说明，不能当成「已接线」。
pub fn status_with(agent: &str, port: u16, record: Option<&WiringRecord>) -> WiringStatus {
    let codex_mode = (agent == "codex").then(|| {
        if codex_oauth_logged_in() { "oauth" } else { "provider" }.to_string()
    });
    if !is_agent(agent) {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: None,
            notice: None,
            reason: Some("该工具不在接线范围".into()),
            state: record.map(|r| r.state.clone()),
            drift: None,
            codex_mode,
            current_model: None,
            current_resolved: None,
        };
    }
    let Some(path) = config_path(agent) else {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: None,
            notice: None,
            reason: Some("未找到用户目录".into()),
            state: record.map(|r| r.state.clone()),
            drift: None,
            codex_mode,
            current_model: None,
            current_resolved: None,
        };
    };
    let display = path.display().to_string();
    let installed = path.exists()
        || (super::wiring_specs::spec(agent)
            .is_some_and(|spec| spec.extra == super::wiring_specs::WiredExtra::GeminiApiKey)
            && gemini_settings_path().is_some_and(|p| p.exists()))
        || (super::wiring_specs::spec(agent)
            .is_some_and(|spec| spec.extra == super::wiring_specs::WiredExtra::AntigravityModel)
            && crate::creds::home().is_some_and(|home| crate::gateway::wiring_antigravity::installed(&home)));
    if !installed {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: Some(display),
            notice: None,
            reason: Some("尚未安装或未生成配置文件".into()),
            state: record.map(|r| r.state.clone()),
            drift: None,
            codex_mode,
            current_model: None,
            current_resolved: None,
        };
    }
    let text = crate::steady::read_to_string(&path).unwrap_or_default();
    if record.is_some_and(|r| r.state == "pending") {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: Some(display),
            notice: Some(restart_notice(agent).to_string()),
            reason: Some("上次接线未完成；启动时会按配置文件的实际内容修复".into()),
            state: Some("pending".into()),
            drift: Some("pending".into()),
            codex_mode,
            current_model: None,
            current_resolved: None,
        };
    }
    // 状态以文件里实际写下的值为准；旧版记录（无实例后缀）按本机网关端口判断。
    let instance = record.map(|r| r.instance_id.chars().take(8).collect::<String>()).unwrap_or_default();
    let kept = record.and_then(|r| r.applied()).and_then(|a| a.token);
    let wired = ours_with(agent, &text, port, &instance, kept.as_deref());
    let drift = if !wired && record.is_some_and(|r| r.state != "idle") {
        Some(
            if foreign_marker(agent, &text, &instance).is_some() {
                "foreign"
            } else if text.trim().is_empty() {
                "removed"
            } else {
                "changed"
            }
            .to_string(),
        )
    } else {
        None
    };
    // B8a：回读 Claude Code 现在的主模型；和上次接线写的不一致 = 用户在 Claude Code 里 /model 换过
    let (current_model, current_resolved) = match (agent, wired) {
        ("claude-code", true) => claude_read_model(&text).map_or((None, None), |(raw, resolved)| (Some(raw), resolved)),
        // Antigravity 的模型选择在 settings.json 的 `model`（`jolu/<模型>`），没有别的回读入口。
        ("antigravity", true) => crate::gateway::wiring_antigravity::current_model(&text).map_or((None, None), |model| (Some(model), None)),
        _ => (None, None),
    };
    let applied_model = record.and_then(|r| r.applied()).and_then(|a| a.model);
    let drift = drift.or_else(|| {
        let now = current_resolved.as_deref().or(current_model.as_deref())?;
        let written = applied_model.as_deref()?;
        (model_key(now) != model_key(written)).then(|| "model_changed".to_string())
    });
    WiringStatus {
        agent: agent.into(),
        wired,
        config_path: Some(display),
        notice: Some(restart_notice(agent).to_string()),
        reason: None,
        state: record.map(|r| r.state.clone()),
        drift,
        codex_mode,
        current_model,
        current_resolved,
    }
}

/// 比较两个模型名时忽略 `[1m]` 标记与 `:effort` 后缀和大小写。
fn model_key(model: &str) -> String {
    let (bare, _) = crate::gateway::wiring_store::split_tier_value(model.trim());
    bare.trim().trim_end_matches("[1m]").trim_end_matches("[1M]").to_ascii_lowercase()
}

/// 回读 Claude Code 现在用的主模型（magpie `claudeMain` 的优先级链）：
/// 顶层 `model`（`/model` 写这里）→ `env.ANTHROPIC_MODEL` → `env.ANTHROPIC_DEFAULT_SONNET_MODEL`（默认档）。
/// 返回（原值，别名解析结果）：原值是档位别名时按对应档位 env 解析成真实模型（B8b），
/// `best` 按 opus、`opusplan` 按 opus 解析；档位 env 没写就没有解析结果。
pub(crate) fn claude_read_model(text: &str) -> Option<(String, Option<String>)> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let env = root.get("env");
    let env_str = |key: &str| env.and_then(|e| e.get(key)).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let raw = root
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| env_str("ANTHROPIC_MODEL"))
        .or_else(|| env_str("ANTHROPIC_DEFAULT_SONNET_MODEL"))?;
    let alias = model_key(&raw);
    let tier = match alias.as_str() {
        "opus" | "best" | "opusplan" => Some("opus"),
        "sonnet" => Some("sonnet"),
        "haiku" => Some("haiku"),
        "fable" => Some("fable"),
        _ => None,
    };
    let resolved = tier.and_then(|tier| env_str(&claude_tier_env(tier)));
    Some((raw, resolved))
}

/// 重启提示：全部可从注册表读（`wiring_specs::AgentSpec.notice`），未注册的 id 给通用兜底。
///
/// 这里原来还有一段手写 `match`（claude / gemini / codex / opencode / zcode / kimi 各一条
/// 文案 + dsh / pi / extra 三个转发）。注册表接管后它已成**不可达死代码**——各家在注册表
/// 里都有非空 notice，短路永远先命中。2026-11-20 删除，文案只维护注册表那一份。
fn restart_notice(agent: &str) -> &'static str {
    super::wiring_specs::spec(agent)
        .map(|spec| spec.notice)
        .unwrap_or("需重启后生效")
}

/// 重新应用时优先沿用旧快照；只有第一次接线才使用当前文件算出的值。
fn effective_snapshot(prior: Option<&str>, computed: Option<serde_json::Value>) -> Option<String> {
    prior.map(str::to_string).or_else(|| computed.map(|s| s.to_string()))
}

/// 落盘一份接线计划：备份 → `before_write`（调用方先写 pending 记录）→ 多文件原子写（失败回滚）。
pub(crate) fn commit_plan(
    plan: WirePlan,
    backup_dir: &Path,
    before_write: impl FnOnce(Option<&str>, &Applied) -> Result<(), String>,
) -> Result<(Option<String>, Applied), String> {
    for (file, _) in &plan.files {
        backup(file, backup_dir)?;
    }
    before_write(plan.snapshot.as_deref(), &plan.applied)?;
    write_all_with_rollback(&plan.files)?;
    Ok((plan.snapshot, plan.applied))
}

/// 接线要写的全部文件与记账（只算不写）：[`wire`] 与接入前的「写入预览」共用。
pub(crate) struct WirePlan {
    pub files: Vec<(PathBuf, Vec<u8>)>,
    pub snapshot: Option<String>,
    pub applied: Applied,
}

/// 一次接线请求的全部输入。字段按「谁用」分组，每家工具只读自己需要的：
/// 通用字段在 `WireContext` 本身，选模型相关在 [`ModelChoice`]，Claude Code 专属在
/// [`ClaudeInput`]。
///
/// 为什么不再用位置参数：原来 16 个参数里每家只用到 3-4 个，而第 9 与第 12 格都是
/// `Option<&str>`——填错位置编译器不报错，运行时才发现「Claude 的档位写进了 Kimi」。
/// 详情见 `docs/plans/2026-11-20-agent-wiring-refactor-plan.md`。
pub(crate) struct WireContext<'a> {
    pub agent: &'a str,
    /// 网关根地址（`http://127.0.0.1:<port>`）。**拥有所有权**而不是 `&str`：
    /// 调用方原本写 `base: &format!(...)`，靠临时值生命周期延长才能编译，
    /// 一旦 ctx 需要跨语句存活就失效——这类脆弱性不值得省一次分配。
    pub base: String,
    /// 归属令牌（`jolu-<agent>.<实例8位>`）。同样拥有所有权（理由同 `base`）。
    /// Claude Code 的 keyHolds 会在 `plan_wire` 里把它换成保留的命名 Key。
    pub token: String,
    pub instance_short: &'a str,
    /// 网关模型目录：写进对方配置的模型清单来自这里（zcode / kimi / dsh / pi / extra 用）
    pub catalog: &'a [ZcodeModel],
    /// 上一次接线保存的原值：重新应用时用它，不能拿当前文件重新快照——
    /// 当前文件里可能已经是我们写进去的网关地址，重算会把用户的真实上游弄丢
    pub prior_snapshot: Option<&'a str>,
    pub chosen: ModelChoice<'a>,
    /// 只有 Claude Code 会读；其它家传 `None`
    pub claude: Option<ClaudeInput<'a>>,
}

/// 「选哪个模型」相关的输入。
/// `model` 只对 Claude Code 生效：网关按 `<来源>/<模型>` 选路，不写模型映射工具就会发自带别名；
/// 其它家用 `catalog` 的第一个或工具自己上次的选择。
pub(crate) struct ModelChoice<'a> {
    pub model: Option<&'a str>,
    pub label: Option<&'a str>,
    pub description: Option<&'a str>,
    /// 上一次接线写下的模型映射（取自接线记录的 applied），重新接线时用来把它
    /// 从「用户原值」里排除——否则断开接线会把用户的模型名还原成网关来源名
    pub prior_model: Option<&'a str>,
    pub prior_label: Option<&'a str>,
    pub prior_values: &'a [String],
    pub subagent_model: Option<&'a str>,
    pub tiers: Option<&'a TierModels>,
}

/// Claude Code 专属输入。
pub(crate) struct ClaudeInput<'a> {
    /// 档位「关于」：按写入值查 models.dev，给 /model 菜单用
    pub tier_about: &'a dyn Fn(&str) -> (Option<String>, Option<String>),
    pub extras: &'a ClaudeExtras,
    /// keyHolds：Claude Code 里已是网关认的命名客户端 Key 时保留它——那把 Key
    /// 带着用户设的每日上限与并发，换成 Jolu 的令牌就把这些约束丢了
    pub holds: &'a dyn Fn(&str) -> bool,
}

/// 接入的计划：读原值 → 拒绝别的实例 → 算出要写的文件与记账（只算不写；落盘见 [`commit_plan`]）。
pub(crate) fn plan_wire(ctx: &WireContext) -> Result<WirePlan, String> {
    let spec = super::wiring_specs::spec(ctx.agent).ok_or("该工具不在接线范围")?;
    let path = config_path(ctx.agent).ok_or("未找到用户目录")?;
    let original = if crate::steady::exists(&path) {
        crate::steady::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    if let Some(suffix) = foreign_marker(ctx.agent, &original, ctx.instance_short) {
        return Err(format!("该工具已被另一个 Jolu 实例接管（尾号 {suffix}）；请先在那个实例里断开，或手动改回直连"));
    }
    let edits = (spec.wire)(ctx, &original)?;
    let mut files = edits.extra_files;
    files.push((path, edits.next.into_bytes()));
    Ok(WirePlan { files, snapshot: effective_snapshot(ctx.prior_snapshot, edits.snapshot), applied: edits.applied })
}

/// 接线前要让用户知道的事（只读，不改文件）。目前：Claude Code 的顶层 `model` 与
/// `env.ANTHROPIC_MODEL` 同时设了——接线会移除后者（它压过 /model 的选择），断开时还原。
pub fn preflight_notices(agent: &str) -> Vec<String> {
    if agent != "claude-code" {
        return Vec::new();
    }
    let Some(text) = config_path(agent).and_then(|path| crate::steady::read_to_string(path).ok()) else { return Vec::new() };
    claude_conflicts(&text)
}

fn claude_conflicts(text: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let top = root.get("model").and_then(|v| v.as_str()).filter(|m| !m.trim().is_empty() && !is_gateway_value(m));
    let env = root.pointer("/env/ANTHROPIC_MODEL").and_then(|v| v.as_str()).filter(|m| !m.trim().is_empty() && !is_gateway_value(m));
    match (top, env) {
        (Some(top), Some(env)) => vec![format!(
            "你的 settings.json 同时设置了顶层 model（{top}）与 env.ANTHROPIC_MODEL（{env}）：后者会压过 /model 的选择，接线时会移除它，断开接线时原样还原。"
        )],
        _ => Vec::new(),
    }
}

/// 多个文件按顺序写；中途失败就把已经写的文件还原，避免 `.env` 改了、settings 没改的半接管。
fn write_all_with_rollback(files: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    let originals: Vec<Option<Vec<u8>>> = files.iter().map(|(path, _)| crate::steady::read(path).ok()).collect();
    for (index, (path, bytes)) in files.iter().enumerate() {
        if let Err(error) = write_atomic(path, bytes.clone()) {
            for (done, (done_path, _)) in files[..index].iter().enumerate() {
                match &originals[done] {
                    Some(previous) => {
                        let _ = write_atomic(done_path, previous.clone());
                    }
                    None => {
                        let _ = std::fs::remove_file(done_path);
                    }
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

/// 还原：只清理自己的写入。当前值已不指向本服务（外部改过 / 别的实例接管）则不覆盖。
pub(crate) fn unwire(agent: &str, port: u16, record: Option<&WiringRecord>) -> Result<bool, String> {
    let Some(files) = plan_unwire(agent, port, record)? else { return Ok(false) };
    write_all_with_rollback(&files)?;
    if let (Some(cleanup), Some(home)) = (super::wiring_specs::spec(agent).and_then(|spec| spec.after_unwire), crate::creds::home()) {
        cleanup(&home);
    }
    Ok(true)
}

/// 断开要写回的文件（只算不写）。None = 不是我们的配置，什么都不做。
/// 已接入时的「写入的改动」预览用它：还原后的内容 → 现在的内容 = Jolu 改了什么。
pub(crate) fn plan_unwire(agent: &str, port: u16, record: Option<&WiringRecord>) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    let spec = super::wiring_specs::spec(agent).ok_or("该工具不在接线范围")?;
    if !crate::steady::exists(&path) && !spec.unwire_without_main {
        return Ok(None);
    }
    let original = if crate::steady::exists(&path) {
        crate::steady::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    let instance = record.map(|r| r.instance_id.chars().take(8).collect::<String>()).unwrap_or_default();
    // 别的实例写的配置不能由我们还原；没有记录（旧版升级）时不做这层判断。
    if !instance.is_empty() && foreign_marker(agent, &original, &instance).is_some() {
        return Ok(None);
    }
    // 记录里存的是接线前的原值；没有记录时按「原来没有该键」处理。
    let saved: Option<serde_json::Value> = record
        .and_then(|r| r.original_json.as_deref())
        .and_then(|s| serde_json::from_str(s).ok());
    let input = adapters::UnwireInput { path, original, saved, port, record };
    (spec.unwire)(agent, &input)
}

/// 已接入时「写入的改动」：每个文件按「断开会还原成的内容 → 现在的内容」对比，即 Jolu 改了什么。
/// 不是我们的配置时返回空列表。
pub(crate) fn preview_wired(agent: &str, port: u16, record: Option<&WiringRecord>) -> Result<Vec<crate::gateway::wiring_diff::FileDiff>, String> {
    let Some(files) = plan_unwire(agent, port, record)? else { return Ok(Vec::new()) };
    Ok(files
        .iter()
        .map(|(path, restored)| {
            let current = crate::steady::read_optional(path).ok().flatten().unwrap_or_default();
            let restored = String::from_utf8_lossy(restored);
            crate::gateway::wiring_diff::diff_file(path, Some(&restored), &current)
        })
        .collect())
}

/// 接入前「写入预览」：现在的内容 → 将要写成的内容。
pub(crate) fn preview_plan(plan: &WirePlan) -> Vec<crate::gateway::wiring_diff::FileDiff> {
    plan.files
        .iter()
        .map(|(path, next)| {
            let current = crate::steady::read_optional(path).ok().flatten();
            crate::gateway::wiring_diff::diff_file(path, current.as_deref(), &String::from_utf8_lossy(next))
        })
        .collect()
}

/// 启动时修复 pending：文件里确实是我们写下的值 → 直接标 wired；否则标 idle。
/// 不在启动时重写配置文件：用户可能正是在两个版本之间手动改回了直连。
pub(crate) fn reconcile_pending(agent: &str, port: u16, record: &WiringRecord) -> &'static str {
    if record.state != "pending" {
        return "unchanged";
    }
    let Some(path) = config_path(agent) else { return "idle" };
    let Ok(text) = crate::steady::read_to_string(&path) else { return "idle" };
    let applied = record.applied();
    let wrote = applied
        .as_ref()
        .map(|applied| {
            let base = applied.base.trim();
            let expected_port = base.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(port);
            ours_with(agent, &text, expected_port, &record.instance_id.chars().take(8).collect::<String>(), applied.token.as_deref())
        })
        .unwrap_or(false);
    if wrote { "wired" } else { "idle" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::wiring_store::TierModels;

    // 不注入档位「关于」的测试桩：档位 about 断言在专项测试里做
    fn no_about(_: &str) -> (Option<String>, Option<String>) { (None, None) }

    #[test]
    fn claude_wire_and_restore_round_trip_preserves_other_fields() {
        let original = r#"{"env":{"SOME_KEY":"keep","ANTHROPIC_BASE_URL":"https://old"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["SOME_KEY"], "keep");
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:12731");
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "jolu-claude-code");
        // 还原回原值
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://old");
        assert_eq!(value["env"]["SOME_KEY"], "keep");
        assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(), "原本没有的键还原后要删掉");
    }

    #[test]
    fn claude_wire_writes_main_model_into_tiers_and_top_level() {
        // magpie writeTiers 语义：主模型写顶层 model、四档 env 与 small-fast 全部跟随；
        // ANTHROPIC_MODEL 必须删除——它压过 /model 的选择，写了切换只在本会话生效。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["model"], "zcode/glm-4.6", "主模型在 settings 顶层");
        assert!(value["env"].get("ANTHROPIC_MODEL").is_none(), "ANTHROPIC_MODEL 要删掉");
        for tier in ["opus", "sonnet", "haiku"] {
            assert_eq!(value["env"][claude_tier_env(tier)], "zcode/glm-4.6", "{tier} 档跟随主模型");
        }
        // B3：Fable 档跟随别家主模型时留空（magpie #2090）
        assert!(value["env"].get(claude_tier_env("fable")).is_none(), "Fable 档不写别家模型");
        assert!(value["env"].get("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME").is_none());
        assert_eq!(value["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "zcode/glm-4.6", "后台小任务也要能路由");
        // 每档写入显示名与描述（/model 菜单的档位标签），旧「关于」值被覆盖不再张冠李戴
        for tier in ["opus", "sonnet", "haiku"] {
            assert_eq!(value["env"][format!("{}{}", claude_tier_env(tier), "_NAME")], "zcode/glm-4.6 显示名");
            assert_eq!(value["env"][format!("{}{}", claude_tier_env(tier), "_DESCRIPTION")], "zcode/glm-4.6 的描述，用于 /model 菜单");
        }
    }

    #[test]
    fn claude_wire_stales_tier_about_keys_and_restores_them() {
        // 用户原来给 opus 档写过 _NAME：接线换模型后要清掉（不然 /model 显示错模型），
        // 断开时还原成用户写的。
        let name_key = format!("{}{}", claude_tier_env("opus"), "_NAME");
        let original = serde_json::json!({"env": {&name_key: "我的自定义 Opus"}});
        let snapshot = claude_snapshot(serde_json::to_string(&original).unwrap().as_str(), None, None, &[]).unwrap();
        // 无显示名（label=None）：旧「关于」值清掉，/model 退回显示模型 id
        let wired = claude_wire(
            &serde_json::to_string(&original).unwrap(),
            "http://127.0.0.1:12731",
            "tok",
            Some("zcode/glm-4.6"),
            None,
            None,
            None,
        None, &no_about, &ClaudeExtras::default())
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        // label 缺省时退回模型 id 本身——旧「关于」值照样被覆盖，不再指错模型
        assert_eq!(value["env"][&name_key], "zcode/glm-4.6");
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"][&name_key], "我的自定义 Opus");
    }

    #[test]
    fn claude_subagent_model_three_states() {
        // 给了 subagent 模型：写进 env
        let wired = claude_wire(
            "{}", "http://127.0.0.1:12731", "tok",
            Some("zcode/glm-4.6"), None, None,
            Some("kimi/kimi-k2.5"),
        None, &no_about, &ClaudeExtras::default())
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "kimi/kimi-k2.5");

        // 没给：删键（跟随主模型，Claude Code 的缺省）
        let original = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"old/model"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none(), "不给就删键＝跟随主模型");

        // 断开还原：用户原来的 subagent 模型要回来
        let original = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"user/own-sub"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, Some("zcode/glm-4.6"), None, &no_about, &ClaudeExtras::default()).unwrap();
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "user/own-sub");
    }

    #[test]
    fn claude_tiers_own_model_writes_tier_env_with_effort_and_own_about() {
        // 批 4 档位自选（magpie tier_own + tierWith 同款）：
        // 自选档写 `model[:effort]`（可带 [1m]），「关于」按该档自己的模型取；
        // 未自选的档照旧跟随主模型。
        let tiers = TierModels {
            opus: None,
            sonnet: None,
            haiku: Some("zcode/glm-5.3-flash:low".into()),
            fable: Some("zcode/glm-5.3-flash".into()),
            auto_mode: Some("kimi/kimi-k2.5".into()),
        };
        let tier_about = |value: &str| -> (Option<String>, Option<String>) {
            // 模拟调用方按 models.dev 查档位模型的显示名
            let name = if value.starts_with("zcode/glm-5.3-flash") { "GLM-5.3-Flash" } else { "Kimi K2.5" };
            (Some(name.to_string()), Some(format!("{name} 的描述")))
        };
        let wired = claude_wire(
            "{}", "http://127.0.0.1:12731", "tok",
            Some("zcode/glm-4.6"), Some("GLM-4.6 主显示名"), Some("主模型描述"),
            Some("zcode/glm-5.3-flash"), Some(&tiers), &tier_about, &ClaudeExtras::default(),
        ).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        // 自选档：写入值带 effort 编码，「关于」用档位模型自己的名字
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "zcode/glm-5.3-flash:low");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"], "GLM-5.3-Flash", "/model 菜单显示档位实际用的模型");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "zcode/glm-5.3-flash", "不带 effort 就裸模型名");
        // 未自选档：照旧跟随主模型，「关于」也用主模型的
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "zcode/glm-4.6");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"], "GLM-4.6 主显示名");
        // Auto 判别：写了 CLAUDE_CODE_AUTO_MODE_MODEL
        assert_eq!(value["env"]["CLAUDE_CODE_AUTO_MODE_MODEL"], "kimi/kimi-k2.5");
        // 子代理照常
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "zcode/glm-5.3-flash");
    }

    #[test]
    fn claude_tiers_none_keeps_follow_behavior_and_clears_auto_mode() {
        // 全跟随（tiers = None，等价旧快照升级）：行为与批 4 之前完全一致，
        // 且 CLAUDE_CODE_AUTO_MODE_MODEL 要删键——Claude Code 自己挑分类器模型
        let original = r#"{"env":{"CLAUDE_CODE_AUTO_MODE_MODEL":"stale/model","CLAUDE_CODE_SUBAGENT_MODEL":"zcode/glm-5.3-flash"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, Some("zcode/glm-5.3-flash"), None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        for tier in ["opus", "sonnet", "haiku"] {
            assert_eq!(value["env"][claude_tier_env(tier)], "zcode/glm-4.6", "{tier} 跟随主模型");
        }
        assert!(value["env"].get(claude_tier_env("fable")).is_none(), "Fable 跟随别家主模型时留空");
        assert!(value["env"].get("CLAUDE_CODE_AUTO_MODE_MODEL").is_none(), "Auto 判别没自选就删键");
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "zcode/glm-5.3-flash");
    }

    #[test]
    fn claude_read_model_follows_magpie_priority_and_resolves_aliases() {
        // 顶层 model 优先（/model 写这里）
        let text = r#"{"model":"kimi/kimi-k2.5","env":{"ANTHROPIC_MODEL":"x/y","ANTHROPIC_DEFAULT_SONNET_MODEL":"z/w"}}"#;
        assert_eq!(claude_read_model(text), Some(("kimi/kimi-k2.5".into(), None)));
        // 别名：按档位 env 解析成真实模型；best / opusplan 按 opus
        let text = r#"{"model":"opus","env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"zcode/glm-5.3[1m]"}}"#;
        assert_eq!(claude_read_model(text), Some(("opus".into(), Some("zcode/glm-5.3[1m]".into()))));
        let text = r#"{"model":"best","env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"a/b"}}"#;
        assert_eq!(claude_read_model(text).unwrap().1.as_deref(), Some("a/b"));
        // 没有顶层 model：ANTHROPIC_MODEL → 默认档
        assert_eq!(claude_read_model(r#"{"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"z/w"}}"#).unwrap().0, "z/w");
        assert_eq!(claude_read_model(r#"{"env":{}}"#), None);
        // 比较忽略 [1m] / :effort
        assert_eq!(model_key("zcode/GLM-5.3[1m]:high"), model_key("zcode/glm-5.3"));
    }

    #[test]
    fn claude_fable_follows_only_fable_family_or_own_pick() {
        // 主模型本身是 Fable 家族：跟随照写
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("claude/claude-fable-5-2"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "claude/claude-fable-5-2");
        // 用户自选 Fable 档：照写
        let tiers = TierModels { opus: None, sonnet: None, haiku: None, fable: Some("kimi/kimi-k2.5".into()), auto_mode: None };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("zcode/glm-5.3"), None, None, None, Some(&tiers), &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "kimi/kimi-k2.5", "自选档照写");
        // 用户原来有 Fable 档：接线期间移除，断开时还原
        let original = r#"{"env":{"ANTHROPIC_DEFAULT_FABLE_MODEL":"claude-fable-5-2"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-5.3"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&wired).unwrap()["env"].get("ANTHROPIC_DEFAULT_FABLE_MODEL").is_none());
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&restored).unwrap()["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"], "claude-fable-5-2");
    }

    #[test]
    fn claude_extras_write_read_back_and_restore() {
        // 用户原有自己的最大输出；上下文窗口原本没有
        let original = r#"{"env":{"CLAUDE_CODE_MAX_OUTPUT_TOKENS":"64000","OTHER":"keep"}}"#;
        let extras = ClaudeExtras {
            max_output: Some(131_072),
            context_window: Some(1_000_000),
            compact_window: Some(272_000),
            capabilities: Some("zcode/glm-5.3=effort,xhigh_effort,max_effort".into()),
            previous: Default::default(),
        };
        let snapshot = claude_snapshot(original, None, None, &[]).and_then(|s| drop_our_extras(s, &extras.previous)).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-5.3"), None, None, None, None, &no_about, &extras).unwrap();
        // 写入 → 读回
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"][CLAUDE_MAX_OUTPUT], "131072", "B1：绕开 32000 默认");
        assert_eq!(value["env"][CLAUDE_MAX_CONTEXT], "1000000", "B2：大窗口不再按 200K");
        assert_eq!(value["env"][CLAUDE_COMPACT_WINDOW], "272000", "B5");
        assert_eq!(value["env"][CLAUDE_MODEL_CAPABILITIES], "zcode/glm-5.3=effort,xhigh_effort,max_effort", "B6");
        assert_eq!(value["env"][CLAUDE_AUTO_MODE_SERVER], "0", "B7：别家主模型关掉 auto review");
        // 断开还原：用户的 64000 回来，我们加的键删干净，其它键不动
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"][CLAUDE_MAX_OUTPUT], "64000");
        assert_eq!(value["env"]["OTHER"], "keep");
        for key in [CLAUDE_MAX_CONTEXT, CLAUDE_COMPACT_WINDOW, CLAUDE_MODEL_CAPABILITIES, CLAUDE_AUTO_MODE_SERVER] {
            assert!(value["env"].get(key).is_none(), "{key} 原本没有，还原后删掉");
        }
        // 重新接线：上次写下的派生值不被当成用户原值
        let applied = claude_extra_env("zcode/glm-5.3", &extras);
        let resnap = claude_snapshot(&wired, Some("zcode/glm-5.3"), None, &[]).and_then(|s| drop_our_extras(s, &applied));
        assert!(resnap.as_ref().and_then(|s| s.get("env")).map_or(true, |env| env.get(CLAUDE_MAX_CONTEXT).is_none() && env.get(CLAUDE_AUTO_MODE_SERVER).is_none()), "{resnap:?}");
    }

    #[test]
    fn claude_extras_unknown_values_are_not_guessed_and_stale_ones_are_removed() {
        // Claude 家族主模型：不关 auto review；元数据缺失：窗口与输出都不写
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("claude/claude-sonnet-5[1m]"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        for key in CLAUDE_EXTRA_KEYS {
            assert!(value["env"].get(key).is_none(), "{key} 不猜");
        }
        // 换成元数据缺失的模型：上次我们写的窗口删掉，用户自己改过的值不动
        let previous: std::collections::BTreeMap<String, String> =
            [(CLAUDE_MAX_CONTEXT.to_string(), "1000000".to_string()), (CLAUDE_MAX_OUTPUT.to_string(), "131072".to_string())].into();
        let current = r#"{"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"1000000","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"50000"}}"#;
        let extras = ClaudeExtras { previous, ..Default::default() };
        let wired = claude_wire(current, "http://127.0.0.1:12731", "tok", Some("other/unknown-model"), None, None, None, None, &no_about, &extras).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get(CLAUDE_MAX_CONTEXT).is_none(), "旧模型的窗口不能留给新模型");
        assert_eq!(value["env"][CLAUDE_MAX_OUTPUT], "50000", "用户自己改过的值不动");
    }

    #[test]
    fn tier_models_serde_round_trip_and_legacy_default() {
        // 旧快照（无 tiers 字段）反序列化为 None = 全跟随；新快照往返一致
        let legacy: Applied = serde_json::from_str(
            r#"{"base":"http://127.0.0.1:12731","model":"zcode/glm-4.6"}"#,
        ).unwrap();
        assert!(legacy.tiers.is_none(), "旧快照没有档位自选");
        let applied = Applied {
            base: "http://127.0.0.1:12731".into(), token: None, provider: None, mode: None,
            model: Some("zcode/glm-4.6".into()), label: None, subagent_model: None,
            tiers: Some(TierModels {
                opus: None, sonnet: None,
                haiku: Some("zcode/glm-5.3-flash:low".into()), fable: None, auto_mode: None,
            }),
            extra_env: Default::default(),
        };
        let json = serde_json::to_string(&applied).unwrap();
        let back: Applied = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tiers.as_ref().and_then(|t| t.haiku.as_deref()), Some("zcode/glm-5.3-flash:low"));
        assert_eq!(back.tiers.as_ref().and_then(|t| t.opus.as_deref()), None);
    }

    #[test]
    fn rewire_snapshot_does_not_capture_our_own_tier_and_auto_values() {
        // 用户原本没有任何模型配置 → 没有存快照；重新接线时文件里全是我们写的值
        let tiers = TierModels { opus: Some("claude-me/claude-opus-4-5:high".into()), sonnet: None, haiku: None, fable: None, auto_mode: Some("zcode/glm-5.3-flash".into()) };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code.abcd1234", Some("zcode/glm-5.3"), Some("GLM 5.3"), None, None, Some(&tiers), &|v: &str| (Some(format!("{v} 名")), None), &ClaudeExtras::default()).unwrap();
        let applied = Applied { tiers: Some(tiers.clone()), extra_env: claude_extra_env("zcode/glm-5.3", &ClaudeExtras::default()), ..Default::default() };
        let snapshot = claude_snapshot(&wired, Some("zcode/glm-5.3"), Some("GLM 5.3"), &applied.written_values())
            .and_then(|saved| drop_our_extras(saved, &applied.extra_env));
        assert!(snapshot.is_none(), "自选档位值、Auto 值、自选档位的显示名都是我们写的：{snapshot:?}");
        // 断开：没有快照时这些键也要删干净
        let restored = claude_restore(&wired, None, Some("zcode/glm-5.3"), Some("GLM 5.3"), &applied.written_values()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").map_or(true, |env| env.get("CLAUDE_CODE_AUTO_MODE_MODEL").is_none() && env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none()), "{restored}");
    }

    #[test]
    fn tiers_declare_their_effort_capabilities() {
        let tiers = TierModels { opus: Some("c/claude-opus-4-5".into()), sonnet: None, haiku: Some("c/claude-haiku-4-5".into()), fable: None, auto_mode: None };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-5.3"), None, None, None, Some(&tiers), &no_about, &ClaudeExtras::default()).unwrap();
        let env = &serde_json::from_str::<serde_json::Value>(&wired).unwrap()["env"];
        assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES"], "effort", "Opus 4.5 只到 high");
        assert!(env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES").is_none(), "Haiku 不收 effort，不写");
        assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES"], "effort,xhigh_effort,max_effort", "跟随主模型（别家模型）给全集");
    }

    #[test]
    fn effort_level_is_fitted_to_the_new_main_model_and_restored_on_unwire() {
        assert_eq!(fit_effort("c/claude-opus-4-5", "max"), Some(Some("high".into())), "不收 max 时降到 high");
        assert_eq!(fit_effort("c/claude-opus-4-6", "xhigh"), Some(Some("high".into())), "跳过不收的 xhigh");
        assert_eq!(fit_effort("c/claude-haiku-4-5", "high"), Some(None), "不收 effort 就删掉");
        assert_eq!(fit_effort("zcode/glm-5.3", "max"), None, "收的不动");
        let original = r#"{"effortLevel":"max","env":{"KEEP":"1"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", Some("c/claude-opus-4-5"), None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&wired).unwrap()["effortLevel"], "high");
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&restored).unwrap()["effortLevel"], "max", "断开还原用户原值");
    }

    #[test]
    fn a_kept_client_key_still_counts_as_our_wiring() {
        let text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-ck-laptop-key"}}"#;
        assert!(!ours("claude-code", text, 12731, "abcd1234"), "不是 Jolu 令牌，单看文件不算");
        assert!(ours_with("claude-code", text, 12731, "abcd1234", Some("jolu-ck-laptop-key")), "接线记录里保留的就是它");
        assert!(!ours_with("claude-code", text, 12731, "abcd1234", Some("")), "空值不算");
    }

    #[test]
    fn top_level_model_and_env_model_together_raise_a_notice() {
        assert_eq!(claude_conflicts(r#"{"model":"opus","env":{"ANTHROPIC_MODEL":"sonnet"}}"#).len(), 1);
        assert!(claude_conflicts(r#"{"model":"opus"}"#).is_empty());
        assert!(claude_conflicts(r#"{"env":{"ANTHROPIC_MODEL":"sonnet"}}"#).is_empty());
        assert!(claude_conflicts("not json").is_empty());
    }

    #[test]
    fn claude_snapshot_excludes_our_own_subagent_write() {
        // 重新接线时，文件里的 subagent 模型是我们上次写的，不是用户原值
        let rewire = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"zcode/glm-4.6"}}"#;
        let snapshot = claude_snapshot(rewire, None, None, &["zcode/glm-4.6".to_string()]);
        assert!(snapshot.is_none(), "我们写的 subagent 模型不该进快照");
    }

    #[test]
    fn claude_wire_stashes_and_removes_cloud_switches() {
        // 云开关会把请求引去 Bedrock/Vertex、绕过 ANTHROPIC_BASE_URL：接线期间移除，断开还原
        let original = r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1","AWS_REGION":"us-east-1"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get("CLAUDE_CODE_USE_BEDROCK").is_none(), "云开关接线期间要移除");
        assert_eq!(value["env"]["AWS_REGION"], "us-east-1", "云专属变量不动");
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_USE_BEDROCK"], "1");
    }

    #[test]
    fn claude_wire_without_model_leaves_existing_mapping_untouched() {
        // 不给模型时不写也不删：用户可能已经把映射配在别处（环境变量 / 命令行）。
        let original = r#"{"env":{"ANTHROPIC_MODEL":"already/set"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "already/set");
        assert!(value["env"].get("ANTHROPIC_SMALL_FAST_MODEL").is_none(), "没给模型就不该凭空加键");
        assert!(value.get("model").is_none(), "没给模型就不动顶层 model");
    }

    #[test]
    fn claude_wire_ignores_blank_model() {
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("   "), Some("    显示名"), Some("    的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value.get("model").is_none(), "空白模型名不应写成空字符串");
    }

    #[test]
    fn claude_restore_puts_back_the_original_model_mapping() {
        // 用户本来就有自己的档位映射：断开接线必须还原成他的，而不是删掉。
        let original = r#"{"env":{"ANTHROPIC_MODEL":"user/own","ANTHROPIC_SMALL_FAST_MODEL":"user/fast","ANTHROPIC_DEFAULT_OPUS_MODEL":"user/opus"},"model":"user-main"}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["model"], "zcode/glm-4.6", "顶层 model 换成主模型");
        assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "user/own");
        assert_eq!(value["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "user/fast");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "user/opus");
        assert_eq!(value["model"], "user-main", "顶层 model 还原");
    }

    #[test]
    fn claude_restore_removes_model_keys_we_added() {
        // 用户原本没有模型映射：断开后要删掉，不能留下指向网关来源的模型名。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about, &ClaudeExtras::default()).unwrap();
        // prior_model 来自接线记录的 applied：断开时靠它识别「顶层 model / 档位键是我们写的」
        let restored = claude_restore(&wired, None, Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").is_none(), "全部是网关写入的键，env 段要清空");
        assert!(value.get("model").is_none(), "顶层 model 也要清掉");
    }

    #[test]
    fn claude_restore_accepts_legacy_flat_snapshot() {
        // 升级兼容：旧版接线记录的快照是 env 键的平面表，没有 env 子对象。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        let legacy = serde_json::json!({"ANTHROPIC_BASE_URL": "https://old.example"});
        let restored = claude_restore(&wired, Some(&legacy), None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://old.example");
        assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn claude_snapshot_records_top_level_model() {
        let original = r#"{"model":"user-main","env":{"ANTHROPIC_BASE_URL":"https://old"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        assert_eq!(snapshot["model"], "user-main");
        assert_eq!(snapshot["env"]["ANTHROPIC_BASE_URL"], "https://old");
    }

    #[test]
    fn claude_restore_cleans_empty_env() {
        let wired = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        let restored = claude_restore(wired, None, None, None, &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").is_none(), "只装了网关配置的 env 段要整个清掉");
    }

    #[test]
    fn snapshot_filters_any_gateway_value() {
        // 重新接线时，上一次写进去的网关地址 / 令牌 / 主模型不能被当成用户原来的上游
        let rewire = r#"{"model":"zcode/glm-4.6","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcdef12","ANTHROPIC_DEFAULT_OPUS_MODEL":"zcode/glm-4.6","KEEP":"1"}}"#;
        // 快照只收接线键清单里的用户原值；KEEP 这种接线不触碰的键不在清单里，本就不收
        let snapshot = claude_snapshot(rewire, Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), &[]);
        assert!(snapshot.is_none(), "管理范围内的键全是网关值时应视为没有原值");
        let mixed = r#"{"env":{"ANTHROPIC_BASE_URL":"https://real.example","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        let snapshot = claude_snapshot(mixed, None, None, &[]).unwrap();
        assert_eq!(snapshot["env"]["ANTHROPIC_BASE_URL"], "https://real.example");
        assert!(snapshot["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn codex_provider_mode_round_trip_keeps_comments_and_writes_token() {
        let original = "# 注释要保留\nmodel_provider = \"openai\"\n\n[model_providers.openai]\nname = \"OpenAI\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        assert_eq!(snapshot["model_provider"], "openai");
        let wired = codex_wire(
            original,
            "http://127.0.0.1:12731",
            "jolu-gateway",
            false,
            Some("jolu-codex.abcd1234"),
        )
        .unwrap();
        assert!(wired.contains("# 注释要保留"), "{wired}");
        assert!(wired.contains("jolu-gateway"), "{wired}");
        assert!(wired.contains("wire_api = \"responses\""), "{wired}");
        assert!(wired.contains("experimental_bearer_token = \"jolu-codex.abcd1234\""), "{wired}");
        assert!(ours("codex", &wired, 12731, "abcd1234"));
        // 别的实例的 token 要能认出来
        assert_eq!(foreign_marker("codex", &wired, "ffffffff").as_deref(), Some("abcd1234"));
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("# 注释要保留"), "{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        assert!(!restored.contains("jolu-gateway"), "还原要删掉 Jolu 的 provider：{restored}");
        assert!(!restored.contains("experimental_bearer_token"), "{restored}");
        // 重新接线不能把 jolu-gateway 当原值
        assert_eq!(codex_snapshot(&wired), None);
    }

    #[test]
    fn codex_oauth_mode_keeps_builtin_provider_and_restores_it() {
        // 已登录 ChatGPT：只写顶层 openai_base_url，不覆盖 model_provider
        let original = "model_provider = \"openai\"\nmodel = \"gpt-5.5\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", true, None).unwrap();
        assert!(wired.contains("openai_base_url = \"http://127.0.0.1:12731/backend-api/codex\""), "{wired}");
        assert!(!wired.contains("model_provider = \"jolu-gateway\""), "{wired}");
        assert!(!wired.contains("[model_providers.jolu-gateway]"), "{wired}");
        assert!(ours("codex", &wired, 12731, "abcd1234"));
        assert!(!wired.contains("model_provider ="), "OAuth 模式要用内置 provider：{wired}");
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(!restored.contains("openai_base_url"), "原本没有该键，还原时要删掉：{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        // 原本有 openai_base_url 的用户要能原样拿回
        let original = "model_provider = \"openai\"\nopenai_base_url = \"https://relay.example\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", true, None).unwrap();
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("openai_base_url = \"https://relay.example\""), "{restored}");
    }

    #[test]
    fn codex_restore_keeps_provider_switched_by_user() {
        let wired = "model_provider = \"jolu-gateway\"\n[model_providers.jolu-gateway]\nbase_url = \"http://127.0.0.1:12731\"\n";
        let switched = wired.replace("model_provider = \"jolu-gateway\"", "model_provider = \"openai\"");
        let restored = codex_restore(&switched, Some(&serde_json::json!({"model_provider":"zhipu"})), "jolu-gateway").unwrap();
        assert!(restored.contains("model_provider = \"openai\""), "用户切换后的 provider 不能被改回去：{restored}");
        assert!(!restored.contains("jolu-gateway"));
    }

    #[test]
    fn opencode_and_zcode_add_and_remove_own_entry_only() {
        let base = r#"{"provider":{"existing":{"options":{"baseURL":"https://keep"}}}}"#;
        let wired = opencode_wire(base, "http://127.0.0.1:12731", "jolu-opencode").unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["provider"]["existing"]["options"]["baseURL"], "https://keep");
        assert_eq!(value["provider"]["jolu"]["options"]["baseURL"], "http://127.0.0.1:12731");
        assert_eq!(opencode_current_base(&wired).as_deref(), Some("http://127.0.0.1:12731"));
        assert_eq!(opencode_current_token(&wired).as_deref(), Some("jolu-opencode"));
        let restored = opencode_restore(&wired, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value["provider"].get("jolu").is_none());
        assert!(value["provider"].get("existing").is_some(), "别人的 provider 不能动");

        let wired = zcode_rules_wire("{}", "http://127.0.0.1:12731", "jolu-zcode", &[]).unwrap();
        assert!(wired.contains("127.0.0.1:12731"));
        assert_eq!(zcode_current_base(&wired).as_deref(), Some("http://127.0.0.1:12731"));
        let restored = zcode_rules_restore(&wired, None).unwrap();
        assert!(!restored.contains("jolu"));
    }

    #[test]
    fn zcode_rules_add_our_provider_and_model_limits_and_restore_leaves_the_rest() {
        let existing = r#"{"schemaVersion":1,"config":{"providerOrder":["builtin:bigmodel"],
            "providerConfigRules":{"providerRules":[{"providerId":"builtin:bigmodel","providerName":"BigModel","config":{"group":"g"}}]},
            "modelConfigRules":{"providerModelRules":[{"providerId":"builtin:bigmodel","modelId":"GLM-5.3"}],
              "manualProviderModelRules":[{"providerId":"jolu","modelId":"kimi/k2","config":{"mine":true}}]}}}"#;
        let models = vec![
            ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 256_000, output: 32_768, image: false },
            ZcodeModel { id: "group/main".into(), name: "group/main".into(), context: 0, output: 0, image: true },
        ];
        let wired = zcode_rules_wire(existing, "http://127.0.0.1:12731", "jolu-zcode.abcd1234", &models).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["config"]["providerOrder"], serde_json::json!(["builtin:bigmodel", "jolu"]));
        let rules = value["config"]["providerConfigRules"]["providerRules"].as_array().unwrap();
        assert_eq!(rules.len(), 2, "别人的 provider 规则保留");
        assert_eq!(rules[1]["config"]["api"]["baseUrl"], "http://127.0.0.1:12731");
        assert_eq!(rules[1]["config"]["personalModelIds"], serde_json::json!(["kimi/k2", "group/main"]));
        let model_rules = value["config"]["modelConfigRules"]["providerModelRules"].as_array().unwrap();
        assert_eq!(model_rules.len(), 3);
        assert_eq!(model_rules[1]["config"]["optionSpecs"]["maxOutputTokens"]["max"], 32_768, "知道输出上限才写，防 32000 截断");
        assert!(model_rules[2]["config"].get("optionSpecs").is_none(), "不知道就不编");
        assert_eq!(model_rules[2]["config"]["properties"]["contextWindow"], 200_000);
        assert_eq!(model_rules[2]["config"]["properties"]["inputFormat"]["supportsImage"], true);
        assert_eq!(value["config"]["modelConfigRules"]["manualProviderModelRules"][0]["config"]["mine"], true, "用户手动设的不动");
        assert_eq!(zcode_current_token(&wired).as_deref(), Some("jolu-zcode.abcd1234"));
        // 重复接线不重复添加
        let again: serde_json::Value = serde_json::from_str(&zcode_rules_wire(&wired, "http://127.0.0.1:12731", "t", &models).unwrap()).unwrap();
        assert_eq!(again["config"]["providerConfigRules"]["providerRules"].as_array().unwrap().len(), 2);
        assert_eq!(again["config"]["providerOrder"].as_array().unwrap().len(), 2);
        // 断开：只去掉自己的
        let restored: serde_json::Value = serde_json::from_str(&zcode_rules_restore(&wired, None).unwrap()).unwrap();
        assert_eq!(restored["config"]["providerOrder"], serde_json::json!(["builtin:bigmodel"]));
        assert_eq!(restored["config"]["providerConfigRules"]["providerRules"].as_array().unwrap().len(), 1);
        assert_eq!(restored["config"]["modelConfigRules"]["providerModelRules"].as_array().unwrap().len(), 1);
        assert_eq!(restored["config"]["modelConfigRules"]["manualProviderModelRules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn zcode_config_json_keeps_a_provider_the_user_turned_off() {
        let existing = r#"{"provider":{"builtin:zai":{"name":"Z.ai"},"jolu":{"enabled":false}}}"#;
        let models = vec![ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 0, output: 8_192, image: false }];
        let wired: serde_json::Value = serde_json::from_str(&zcode_config_wire(existing, "http://127.0.0.1:12731", "t", &models).unwrap()).unwrap();
        assert_eq!(wired["provider"]["jolu"]["enabled"], false);
        assert_eq!(wired["provider"]["jolu"]["models"]["kimi/k2"]["limit"]["output"], 8_192);
        assert!(wired["provider"].get("builtin:zai").is_some());
        let restored: serde_json::Value = serde_json::from_str(&zcode_config_restore(&wired.to_string(), None).unwrap()).unwrap();
        assert!(restored["provider"].get("jolu").is_none() && restored["provider"].get("builtin:zai").is_some());
    }

    // ---------- Kimi（config.toml；参照 magpie kimi.go 的行为） ----------

    fn kimi_models() -> Vec<ZcodeModel> {
        vec![
            ZcodeModel { id: "zcode/GLM-5.3".into(), name: "GLM-5.3".into(), context: 200_000, output: 32_768, image: true },
            ZcodeModel { id: "grok/grok-4".into(), name: "grok/grok-4".into(), context: 0, output: 0, image: false },
        ]
    }

    /// 读接线后 config.toml 里 `models."jolu/<id>"` 表（不存在时 panic，便于断言）。
    fn kimi_table(text: &str, id: &str) -> toml_edit::Table {
        text.parse::<toml_edit::DocumentMut>()
            .unwrap()
            .get("models")
            .and_then(|models| models.get(&format!("jolu/{id}")))
            .and_then(|item| item.as_table())
            .cloned()
            .unwrap_or_else(|| panic!("缺少 models.\"jolu/{id}\" 表：{text}"))
    }

    fn kimi_str(table: &toml_edit::Table, key: &str) -> String {
        table.get(key).and_then(|value| value.as_str()).unwrap_or_default().to_string()
    }

    #[test]
    fn kimi_wire_writes_provider_tables_default_and_round_trips() {
        let models = kimi_models();
        // 空配置首次接线：目录不存在也要能写成完整 config.toml
        let (wired, snapshot) = (kimi_wire("", "http://127.0.0.1:12731", "jolu-kimi.abcd1234", None, &models, false).unwrap(), kimi_snapshot(""));
        assert!(snapshot.is_none(), "原来没有 default_model 就不用存");
        let document = wired.parse::<toml_edit::DocumentMut>().unwrap();
        let provider = kimi_provider_read(&document).unwrap();
        assert_eq!(kimi_str(provider, "type"), "kimi");
        assert_eq!(kimi_str(provider, "base_url"), "http://127.0.0.1:12731");
        assert_eq!(kimi_str(provider, "api_key"), "jolu-kimi.abcd1234");
        // 第一个模型：全字段；上下文/输出/图片都来自目录
        let first = kimi_table(&wired, "zcode/GLM-5.3");
        assert_eq!(kimi_str(&first, "provider"), "jolu");
        assert_eq!(kimi_str(&first, "model"), "zcode/GLM-5.3", "发给上游的是网关模型 id，网关按它选路");
        assert_eq!(kimi_str(&first, "display_name"), "GLM-5.3", "目录给了别称");
        assert_eq!(first.get("max_context_size").and_then(|v| v.as_integer()), Some(200_000));
        assert_eq!(first.get("max_output_size").and_then(|v| v.as_integer()), Some(32_768));
        let caps: Vec<String> = first.get("capabilities").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        assert!(caps.contains(&"image_in".to_string()) && caps.contains(&"tool_use".to_string()), "{caps:?}");
        // 第二个模型：目录缺上下文退 128000（magpie kimiContext）、无输出上限、名字等于 id 就不写 display_name
        let second = kimi_table(&wired, "grok/grok-4");
        assert_eq!(second.get("max_context_size").and_then(|v| v.as_integer()), Some(128_000));
        assert!(second.get("display_name").is_none());
        assert!(second.get("max_output_size").is_none());
        let caps: Vec<String> = second.get("capabilities").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        assert_eq!(caps, vec!["tool_use".to_string()], "{caps:?}");
        // 默认模型 = 清单第一个
        assert_eq!(document.get("default_model").and_then(|v| v.as_str()), Some("jolu/zcode/GLM-5.3"));
        // 状态判定：base + token + default_model 都是才算已接入
        assert!(ours_with("kimi", &wired, 12731, "abcd1234", None));
        // 断开：原来没有 default_model → 连键一起删；我们的表与 provider 不留残余
        let restored = kimi_restore(&wired, None).unwrap();
        let document = restored.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(kimi_provider_read(&document).is_none());
        assert!(document.get("default_model").is_none());
        let leftover = document.get("models").map(|models| models.as_table().map(|t| t.iter().any(|(name, _)| name.starts_with("jolu/"))).unwrap_or(false)).unwrap_or(false);
        assert!(!leftover, "不能留下 jolu 模型表：{restored}");
    }

    #[test]
    fn kimi_wire_keeps_user_keys_and_restore_brings_back_their_default() {
        let original = "default_model = \"mine/m1\"\n\
            \n\
            [providers.jolu]\n\
            base_url = \"https://direct\"\n\
            api_key = \"user-key\"\n\
            custom_headers = { X-Flag = \"1\" }\n\
            \n\
            [models.\"mine/m1\"]\n\
            provider = \"mine\"\n\
            model = \"m1\"\n";
        let snapshot = kimi_snapshot(original).unwrap();
        assert_eq!(snapshot["default_model"], "mine/m1", "用户自己的 default_model 要存快照");
        let wired = kimi_wire(original, "http://127.0.0.1:12731", "jolu-kimi.abcd1234", None, &kimi_models(), false).unwrap();
        let document = wired.parse::<toml_edit::DocumentMut>().unwrap();
        let provider = kimi_provider_read(&document).unwrap();
        assert_eq!(kimi_str(provider, "base_url"), "http://127.0.0.1:12731");
        assert_eq!(kimi_str(provider, "api_key"), "jolu-kimi.abcd1234");
        assert_eq!(provider.get("custom_headers").and_then(|h| h.get("X-Flag")).and_then(|v| v.as_str()), Some("1"), "用户自加的键不能丢（magpie #2030）");
        assert!(document.get("models").and_then(|models| models.get("mine/m1")).is_some(), "用户自己的模型表不能动");
        assert_eq!(document.get("default_model").and_then(|v| v.as_str()), Some("jolu/zcode/GLM-5.3"), "接线就把默认模型引到网关");
        // 断开：default_model 还原回用户原值（那张表还在），其余同上
        let restored = kimi_restore(&wired, Some(&snapshot)).unwrap();
        let document = restored.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document.get("default_model").and_then(|v| v.as_str()), Some("mine/m1"));
        assert!(kimi_provider_read(&document).is_none());
        assert!(document.get("models").and_then(|models| models.get("mine/m1")).is_some());
    }

    #[test]
    fn kimi_wire_legacy_omits_the_keys_old_kimi_cli_cannot_read() {
        let wired = kimi_wire("", "http://127.0.0.1:12731", "t", None, &kimi_models(), true).unwrap();
        // 旧版仍认 image_in（magpie 同款），但不写 tool_use；max_output_size 是新版键
        let first = kimi_table(&wired, "zcode/GLM-5.3");
        let caps: Vec<String> = first.get("capabilities").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        assert_eq!(caps, vec!["image_in".to_string()], "{caps:?}");
        assert!(first.get("max_output_size").is_none(), "旧版没有这个键");
        // 不带图片的模型在旧版下连 capabilities 都不写
        let second = kimi_table(&wired, "grok/grok-4");
        assert!(second.get("capabilities").is_none());
    }

    #[test]
    fn kimi_restore_leaves_a_default_the_user_switched_to() {
        let wired = kimi_wire("", "http://127.0.0.1:12731", "t", None, &kimi_models(), false).unwrap();
        // 接线后用户在 Kimi 里把默认模型换成自己的：断开不能改回去（外部意图优先）
        let user_changed = wired.replace("default_model = \"jolu/zcode/GLM-5.3\"", "default_model = \"mine/m1\"");
        let restored = kimi_restore(&user_changed, Some(&serde_json::json!({"default_model": "mine/m1"}))).unwrap();
        assert_eq!(restored.parse::<toml_edit::DocumentMut>().unwrap().get("default_model").and_then(|v| v.as_str()), Some("mine/m1"));
    }

    #[test]
    fn kimi_restore_drops_a_default_whose_table_is_gone() {
        // 快照里的默认模型指向一张已经不存在的表：不能写出悬空引用（magpie ownModel 判断）
        let wired = kimi_wire("", "http://127.0.0.1:12731", "t", None, &kimi_models(), false).unwrap();
        let restored = kimi_restore(&wired, Some(&serde_json::json!({"default_model": "mine/gone"}))).unwrap();
        assert!(restored.parse::<toml_edit::DocumentMut>().unwrap().get("default_model").is_none());
    }

    #[test]
    fn a_write_that_does_not_parse_is_rolled_back_to_the_previous_content() {
        let dir = std::env::temp_dir().join(format!("jolu-write-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"keep":true}"#).unwrap();
        let error = write_atomic(&path, b"{ broken".to_vec()).unwrap_err();
        assert!(error.contains("校验失败") && error.contains("JSON"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"keep":true}"#, "放回原内容");
        write_atomic(&path, br#"{"ok":1}"#.to_vec()).unwrap();
        let toml = dir.join("config.toml");
        assert!(write_atomic(&toml, b"a = [".to_vec()).unwrap_err().contains("TOML"));
        assert!(!toml.exists(), "原来没有的文件写坏了就删掉");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn gemini_takeover_switches_auth_and_restores_it() {
        let env = "# gemini 配置\nOTHER=1\nGOOGLE_GEMINI_BASE_URL=https://old\nGEMINI_API_KEY=user-key\n";
        let settings =
            r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"model":{"name":"gemini-2.5-pro"}}"#;
        let plan = gemini_plan(env, settings, "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        assert!(plan.env.contains("# gemini 配置"), "{}", plan.env);
        assert!(plan.env.contains("OTHER=1"), "{}", plan.env);
        assert!(plan.env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731"), "{}", plan.env);
        assert!(plan.env.contains("GEMINI_API_KEY=jolu-gemini.abcd1234"), "{}", plan.env);
        let wired_settings: serde_json::Value = serde_json::from_str(&plan.settings).unwrap();
        assert_eq!(wired_settings["security"]["auth"]["selectedType"], "gemini-api-key");
        assert_eq!(wired_settings["model"]["name"], "gemini-2.5-pro", "其他设置不能动");

        let snapshot = plan.snapshot.clone();
        assert_eq!(snapshot["env"]["GEMINI_API_KEY"], "user-key");
        assert_eq!(snapshot["settings"]["selectedType"], "oauth-personal");
        assert_eq!(snapshot["env"]["GOOGLE_GEMINI_BASE_URL"], "https://old");

        let (env, settings) = gemini_restore(&plan.env, &plan.settings, Some(&snapshot));
        assert!(env.contains("GOOGLE_GEMINI_BASE_URL=https://old"), "{env}");
        assert!(env.contains("GEMINI_API_KEY=user-key"), "{env}");
        assert!(env.contains("OTHER=1"), "{env}");
        let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(settings["security"]["auth"]["selectedType"], "oauth-personal");
        assert_eq!(settings["model"]["name"], "gemini-2.5-pro");

        // 原本没有这些键：还原时删除，不能留下 jolu 令牌
        let plan = gemini_plan("OTHER=1\n", "", "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        let (env, settings) = gemini_restore(&plan.env, &plan.settings, Some(&plan.snapshot));
        assert!(!env.contains("GOOGLE_GEMINI_BASE_URL"), "{env}");
        assert!(!env.contains("GEMINI_API_KEY"), "{env}");
        let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert!(settings.pointer("/security/auth/selectedType").is_none(), "{settings}");

        // 旧版快照只有 value：只还原地址，不改 settings
        let plan = gemini_plan(env.as_str(), settings.to_string().as_str(), "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        let (_, legacy_settings) = gemini_restore(&plan.env, &plan.settings, Some(&serde_json::json!({"value":"https://legacy"})));
        assert_eq!(legacy_settings, plan.settings, "旧快照不能改 auth");
    }

    #[test]
    fn gemini_refuses_invalid_settings_instead_of_wiping_it() {
        assert!(gemini_plan("A=1\n", "{not json", "http://127.0.0.1:12731", "jolu-gemini.abcd1234").is_err());
        let (_, settings) = gemini_restore(
            "A=1\n",
            "{not json",
            Some(&serde_json::json!({"env": {}, "settings": {"selectedType": "oauth-personal"}})),
        );
        assert_eq!(settings, "{not json", "坏文件不能被覆盖");
    }

    #[test]
    fn gateway_pointing_check_matches_only_our_port() {
        assert!(points_at_gateway("http://127.0.0.1:12731", 12731));
        assert!(points_at_gateway("http://127.0.0.1:12731/", 12731));
        assert!(points_at_gateway("http://localhost:12731", 12731));
        assert!(!points_at_gateway("http://127.0.0.1:12732", 12731));
        assert!(!points_at_gateway("https://api.anthropic.com", 12731));
    }

    #[test]
    fn token_carries_instance_suffix() {
        assert_eq!(token_for("claude-code", "abcd1234"), "jolu-claude-code.abcd1234");
        assert_eq!(token_for("codex", "abcd1234"), "jolu-codex.abcd1234");
    }

    #[test]
    fn rewiring_keeps_the_original_snapshot() {
        let computed = Some(serde_json::json!({"ANTHROPIC_BASE_URL":"https://current"}));
        assert_eq!(
            effective_snapshot(Some(r#"{"ANTHROPIC_BASE_URL":"https://original"}"#), computed.clone()).as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://original"}"#),
            "重新应用不能拿当前文件重新快照"
        );
        assert_eq!(
            effective_snapshot(None, computed).as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://current"}"#)
        );
        assert_eq!(effective_snapshot(None, None), None);
    }

    #[test]
    fn foreign_marker_rejects_another_instance() {
        let text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.ffffffff"}}"#;
        assert_eq!(foreign_marker("claude-code", text, "abcd1234").as_deref(), Some("ffffffff"));
        assert_eq!(foreign_marker("claude-code", text, "ffffffff"), None);
        // 旧版没有实例后缀：按本实例处理，允许覆盖
        let legacy = r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        assert_eq!(foreign_marker("claude-code", legacy, "abcd1234"), None);
    }

    #[test]
    fn agent_whitelist_covers_catalog_platforms() {
        // 全部 14 家（含 antigravity / qwen / workbuddy），一家都不能漏
        for agent in [
            "claude-code", "codex", "zcode", "gemini", "opencode", "kimi", "dsh", "pi",
            "pencil", "qoder", "qoder-cn", "qwen", "antigravity", "workbuddy",
        ] {
            assert!(is_agent(agent), "{agent} 应可接线");
        }
        // 5.11：Qoder CLI 接入的是 Jolu 网关令牌（写进它的 settings.json），不代管 Qoder 账号的密钥。
        // 2026-11-21：WorkBuddy 的 models.json 是顶层数组，已按 magpie `buddyWrite` 实现逐项插入
        // 与 disabled 状态保留（见 `wiring_workbuddy.rs`），它现在可接线；改用 Cline 锁「不接」。
        assert!(!is_agent("cline"), "Cline 要写系统凭据库，仍不接");
    }

    /// 文件级链路：接线 → 状态探测为已接线 → 还原 → 状态回到未接线。
    /// 用临时目录注入配置路径，不碰真实用户配置。
    #[test]
    fn file_level_wire_status_unwire_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-wiring-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("settings.json");
        std::fs::write(&config, r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"},"keep":1}"#).unwrap();
        let backups = dir.join("backups");

        // 直接调纯函数模拟 wire/unwire（真实 wire() 依赖用户目录，测不到临时路径）
        let original = std::fs::read_to_string(&config).unwrap();
        let snapshot = claude_snapshot(&original, None, None, &[]).unwrap();
        let wired = claude_wire(&original, "http://127.0.0.1:12731", "jolu-claude-code.abcd1234", None, None, None, None, None, &no_about, &ClaudeExtras::default()).unwrap();
        std::fs::write(&config, &wired).unwrap();
        assert!(ours("claude-code", &wired, 12731, "abcd1234"));
        assert!(!ours("claude-code", &wired, 12732, "abcd1234"));

        // 还原
        let restored = claude_restore(&wired, Some(&snapshot), None, None, &[]).unwrap();
        std::fs::write(&config, &restored).unwrap();
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://api.anthropic.com", "还原回原上游");
        assert_eq!(value["keep"], 1, "无关字段必须保留");

        // 备份目录可创建（真实 wire 会用到）
        std::fs::create_dir_all(&backups).unwrap();
        assert!(backups.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 外部已改：还原前发现当前值不是网关地址 → 返回 false 且不改文件。
    #[test]
    fn unwire_skips_when_external_value_took_over() {
        let external = r#"{"env":{"ANTHROPIC_BASE_URL":"https://custom.example"}}"#;
        let current = claude_current_base(external);
        assert!(
            current.is_some_and(|c| !points_at_gateway(&c, 12731) && !is_gateway_value(&c)),
            "外部值不应被判定为网关地址，因此还原会跳过"
        );
        let ours_text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcd1234"}}"#;
        assert!(ours("claude-code", ours_text, 12731, "abcd1234"));
    }

    // ─────────────────── 全部可接线工具的全量往返（子进程 + 临时 home） ───────────────────
    //
    // 为什么必须是子进程：`plan_wire` / `plan_unwire` 走 `creds::home()`（读 `USERPROFILE`
    // / `HOME`），直接在本进程里改这些环境变量会污染并行运行的其他测试（进程级状态，
    // 且 `set_var` 在多线程下未定义行为）。所以父测试只负责起子进程，子进程把 home 指到
    // 临时目录后再跑生产函数，桌面入口和真实工具仍需另行验证。
    //
    // 为什么只读真机、不点桌面：真机桌面实例没有重定向 home，点接入会写用户的真实配置。
    // 现有 `agent-icons-kimi-preview.e2e.mjs` 因此只做「写入预览」不做往返。这里补的正是
    // 那条缺口：覆盖注册表里**每一家**的写入 → 状态 → 还原闭环（家数由 `agents()` 决定，
    // 新增一家自动纳入——没有 `seed_original` 样本时测试会 panic 提示补上）。

    /// 父：给每家起一个干净的子进程（各自临时 home），子进程失败时把 stderr 带出来。
    #[test]
    fn every_agent_round_trips_through_production_plan() {
        if std::env::var_os("JOLU_WIRING_ROUNDTRIP_CHILD").is_some() {
            return;
        }
        let mut failed = Vec::new();
        for agent in agents() {
            let root = std::env::temp_dir().join(format!(
                "jolu-roundtrip-{}-{}",
                agent,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let home = root.join("home");
            std::fs::create_dir_all(&home).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "gateway::wiring::tests::round_trip_one_agent_child",
                    "--nocapture",
                ])
                .env("JOLU_WIRING_ROUNDTRIP_CHILD", "1")
                .env("JOLU_ROUNDTRIP_AGENT", agent)
                .env("JOLU_ROUNDTRIP_HOME", &home)
                // 把 home 与所有能改配置落点的变量一起指进临时目录，避免任何一家
                // 顺着真实环境变量写出去（各家覆盖变量见 `wiring_specs` 的 path_* 与各 dir 函数）
                .env("USERPROFILE", &home)
                .env("HOME", &home)
                .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
                .env("CODEX_HOME", home.join(".codex"))
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("KIMI_CODE_HOME", home.join(".kimi-code"))
                .env("KIMI_SHARE_DIR", home.join(".kimi-share"))
                .env("ZCODE_HOME", home.join(".zcode"))
                .env("DSH_HOME", home.join(".dsh"))
                .env("PI_CODING_AGENT_DIR", home.join(".pi/agent"))
                .env("QODER_CONFIG_DIR", home.join(".qoder"))
                .env("QODERCN_CONFIG_DIR", home.join(".qoder-cn"))
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !output.status.success() {
                failed.push(format!("{agent}：\n{stderr}"));
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(failed.is_empty(), "以下工具的往返失败：\n{}", failed.join("\n\n"));
    }

    /// 子进程：对一家工具跑接线、状态检查和配置还原。
    ///
    /// 使用实际保存的接线记录，并检查计划涉及的所有文件。
    #[test]
    fn round_trip_one_agent_child() {
        let Ok(agent) = std::env::var("JOLU_ROUNDTRIP_AGENT") else { return };
        let Ok(home) = std::env::var("JOLU_ROUNDTRIP_HOME").map(std::path::PathBuf::from) else { return };
        let agent = agent.as_str();
        let port = 12731u16;
        let instance = "abcd1234";
        let base = format!("http://127.0.0.1:{port}");

        // 1. 造一份包含用户原有配置的样本。
        let path = config_path(agent).unwrap_or_else(|| panic!("{agent} 算不出配置路径"));
        seed_original(agent, &home, &path);
        // dsh 的主文件会随已有 profile 改变，准备样本后重新取得。
        let path = config_path(agent).unwrap();

        // 2. 走生产 plan_wire
        let catalog = vec![
            ZcodeModel { id: "zcode/glm-5.3".into(), name: "GLM-5.3".into(), context: 200_000, output: 32_768, image: true },
            ZcodeModel { id: "kimi/k2".into(), name: "Kimi K2".into(), context: 256_000, output: 32_768, image: false },
        ];
        let chosen = ModelChoice {
            model: Some("zcode/glm-5.3"),
            label: Some("GLM-5.3"),
            description: None,
            prior_model: None,
            prior_label: None,
            prior_values: &[],
            subagent_model: None,
            tiers: None,
        };
        let ctx = WireContext {
            agent,
            base: base.clone(),
            token: token_for(agent, instance),
            instance_short: instance,
            catalog: &catalog,
            prior_snapshot: None,
            chosen,
            claude: None,
        };
        let plan = plan_wire(&ctx).unwrap_or_else(|error| panic!("{agent} 生成接线计划失败：{error}"));
        assert!(!plan.files.is_empty(), "{agent} 的计划里没有文件");
        let originals: Vec<_> = plan.files.iter().map(|(file, _)| (file.clone(), std::fs::read(file).ok())).collect();

        // 3. 落盘（与 wire_agents 同一条 commit_plan）
        let backup_dir = home.join("gateway-backups");
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        commit_plan(plan, &backup_dir, |snapshot, applied| {
            super::super::wiring_store::mark_pending(&conn, agent, instance, "relay", snapshot, applied, applied.token.as_deref())
                .map_err(|error| error.to_string())
        }).unwrap_or_else(|error| panic!("{agent} 写盘失败：{error}"));
        super::super::wiring_store::mark_wired(&conn, agent, 42).unwrap();

        // 4. 状态必须是「已接线」（走注册表的 is_wired，这条同时验 token_match / extra）
        let recorded = super::super::wiring_store::load(&conn, agent).unwrap();
        let status = status_with(agent, port, Some(&recorded));
        assert!(status.wired, "{agent} 写完配置文件后 status 仍报未接线：{:?}", status.reason);
        let wired_text = std::fs::read_to_string(&path).unwrap();
        if (super::super::wiring_specs::spec(agent).unwrap().token)(&home, &wired_text).is_some() {
            assert_eq!(foreign_marker(agent, &wired_text, "ffffffff").as_deref(), Some(instance));
            assert_eq!(foreign_marker(agent, &wired_text, instance), None);
        }

        // 5. 断开：用真实记录（含快照）走 plan_unwire，再逐文件应用
        assert!(unwire(agent, port, Some(&recorded)).unwrap_or_else(|error| panic!("{agent} 还原失败：{error}")));
        super::super::wiring_store::mark_idle(&conn, agent).unwrap();

        // 6. 还原后必须把用户的配置还回来
        //
        // 判据是**语义等价**，不是字节相等：生产实现走 `serde_json::Value` / `toml_edit`
        // 解析后重新序列化，键序与缩进会规范化（原始 `{"a":1,"b":2}` 会变成 pretty 排版
        // 且按字母序）。这是既有行为、不是本次重构引入，也不影响任何工具读取。
        // 所以这里比「解析后的值相等」+「用户原始键都在」+「我们的痕迹走干净」三件事。
        for (file, original) in originals {
            let restored = std::fs::read(&file).ok();
            match (original, restored) {
                (Some(before), Some(after)) => {
                    let before_text = String::from_utf8_lossy(&before).to_string();
                    let after_text = String::from_utf8_lossy(&after).to_string();
                    if file.extension().is_some_and(|ext| ext == "yml" || ext == "yaml") {
                        assert_eq!(before_text, after_text, "{agent} 文件 {} 未还原", file.display());
                    } else {
                        assert_semantically_restored(agent, &before_text, &after_text);
                    }
                }
                (None, after) => {
                    // 原本没有这个文件：还原后也不该留下我们的东西
                    if let Some(after) = after {
                        let text = String::from_utf8_lossy(&after);
                        assert!(
                            !text.contains("12731") && !text.contains("jolu-"),
                            "{agent} 原本没有配置文件，还原后却残留了网关内容：{text}"
                        );
                    }
                }
                (Some(_), None) => panic!("{agent} 原本有配置文件，还原后文件被删了"),
            }
        }

        // 7. 还原后状态回到未接线
        let after_status = status_with(agent, port, None);
        assert!(!after_status.wired, "{agent} 还原后 status 仍报已接线");
        if agent == "dsh" {
            let plan = plan_wire(&ctx).unwrap();
            commit_plan(plan, &backup_dir, |snapshot, applied| {
                super::super::wiring_store::mark_pending(&conn, agent, instance, "relay", snapshot, applied, applied.token.as_deref())
                    .map_err(|error| error.to_string())
            }).unwrap();
            let dsh_home = super::super::wiring_dsh::dsh_dir_public(&home);
            std::fs::write(dsh_home.join(".env"), "JOLU_API_KEY=changed-env\n").unwrap();
            std::fs::write(dsh_home.join(".credentials.yaml"), "version: 1\nrefs:\n  JOLU_API_KEY: changed-creds\n").unwrap();
            let record = super::super::wiring_store::load(&conn, agent).unwrap();
            assert!(unwire(agent, port, Some(&record)).unwrap());
            assert!(std::fs::read_to_string(dsh_home.join(".env")).unwrap().contains("JOLU_API_KEY=changed-env"));
            assert!(std::fs::read_to_string(dsh_home.join(".credentials.yaml")).unwrap().contains("JOLU_API_KEY: changed-creds"));
        }
    }

    /// 还原是否「把用户的东西还回来了」：按格式解析后比较值，并额外确认
    /// 网关地址 / Jolu 令牌 / 我们建的 provider 段都不在了。
    ///
    /// 为什么分格式：JSON 与 TOML 的解析器都在生产依赖里（`serde_json` / `toml_edit`），
    /// 直接用它们比较就等价于「工具读到的内容一致」，比字符串比对更贴近真实语义。
    /// `.env` 没有解析库也不需要：按行取键值比就行（注释行与空行不参与比较）。
    fn assert_semantically_restored(agent: &str, before: &str, after: &str) {
        let trimmed = before.trim_start();
        // 判格式：以 { / [ 开头是 JSON；否则看是不是「每行 KEY=VALUE」的 dotenv
        // （gemini 的 .env）；剩下按 TOML 处理。
        let format = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            "json"
        } else if is_dotenv(before) {
            "dotenv"
        } else {
            "toml"
        };
        match format {
            "json" => {
                let expected: serde_json::Value = serde_json::from_str(before)
                    .unwrap_or_else(|error| panic!("{agent} 原始配置不是有效 JSON：{error}"));
                let actual: serde_json::Value = serde_json::from_str(after)
                    .unwrap_or_else(|error| panic!("{agent} 还原结果不是有效 JSON：{error}\n{after}"));
                assert_eq!(expected, actual, "{agent} 还原后的值与原始配置不一致\n原始：{before}\n还原：{after}");
            }
            "dotenv" => {
                // .env 按行取键值比较：注释与空行不参与（生产写入只动目标键，注释原样保留）
                let parse = |text: &str| -> std::collections::BTreeMap<String, String> {
                    text.lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty() && !line.starts_with('#'))
                        .filter_map(|line| line.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                        .collect()
                };
                assert_eq!(parse(before), parse(after), "{agent} 还原后 .env 键值不一致\n原始：{before}\n还原：{after}");
            }
            _ => {
                let expected: toml_edit::DocumentMut = before
                    .parse()
                    .unwrap_or_else(|error| panic!("{agent} 原始配置不是有效 TOML：{error}"));
                let actual: toml_edit::DocumentMut = after
                    .parse()
                    .unwrap_or_else(|error| panic!("{agent} 还原结果不是有效 TOML：{error}\n{after}"));
                assert_eq!(
                    expected.to_string().trim(),
                    actual.to_string().trim(),
                    "{agent} 还原后的值与原始配置不一致\n原始：{before}\n还原：{after}"
                );
            }
        }
        // 不管哪种格式，网关痕迹都必须走干净
        assert!(!after.contains("127.0.0.1:12731"), "{agent} 还原后仍指向网关：{after}");
        assert!(!after.contains("jolu-"), "{agent} 还原后仍留着 Jolu 令牌：{after}");
    }

    /// 子进程用：判断一段文本是不是 dotenv（每行 `KEY=VALUE` 或注释）。
    ///
    /// 只看第一行有效内容：TOML 的键值行不会出现「裸 `=` 而无引号且键名是大写常量」的形态，
    /// 而 `.env` 全是这个形态，足够区分。
    fn is_dotenv(text: &str) -> bool {
        let Some(first) = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')) else {
            return false;
        };
        let Some((key, _)) = first.split_once('=') else { return false };
        !key.is_empty()
            && key.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            && !first.contains('"')
            && !first.contains('\'')
    }

    /// 子进程用：给一家工具写一份「用户原有配置」（含干扰项），让还原有东西可还。
    fn seed_original(agent: &str, home: &Path, path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let text = match agent {
            "claude-code" => r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-mine"},"theme":"dark"}"#,
            "codex" => "model = \"gpt-5\"\nmodel_provider = \"openai\"\n",
            "zcode" => {
                std::fs::write(path.parent().unwrap().join("config.json"), r#"{"keep":1}"#).unwrap();
                r#"{"config":{"providerConfigRules":{"providerRules":[]}},"keep":1}"#
            }
            "gemini" => {
                std::fs::write(path.parent().unwrap().join("settings.json"), r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"keep":true}"#).unwrap();
                "GOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\nGEMINI_API_KEY=my-own-key\n"
            }
            "opencode" => r#"{"theme":"x","provider":{"mine":{"options":{"baseURL":"https://a"}}}}"#,
            "kimi" => {
                // default_model 必须指向一个**存在**的表：生产语义是「原表还在才还原默认」，
                // 指向不存在的表时它会按兜底删掉 default_model（那是另一个分支，另有用例覆盖）。
                "default_model = \"moonshot/kimi-k2\"\n[providers.moonshot]\nbase_url = \"https://api.moonshot.cn/v1\"\napi_key = \"sk-mine\"\n[models.\"moonshot/kimi-k2\"]\nmodel = \"kimi-k2\"\nmax_context_size = 256000\n"
            }
            "pi" => {
                // 双文件：models.json 与 settings.json 都要有原值
                std::fs::write(
                    crate::gateway::wiring_pi::settings_path(home),
                    r#"{"defaultProvider":"anthropic","defaultModel":"claude-sonnet-4"}"#,
                ).unwrap();
                r#"{"providers":{"anthropic":{"baseUrl":"https://api.anthropic.com/v1","apiKey":"sk-mine"}}}"#
            }
            "pencil" | "qoder" | "qoder-cn" => r#"{"providers":{"mine":{"baseUrl":"https://a"}},"model":{"name":"qwen3-coder","reasoningEffort":"high"}}"#,
            "dsh" => {
                // 多文件：一个 profile 的 patch（带注释，验行级编辑不丢注释）
                let dsh_home = crate::gateway::wiring_dsh::dsh_dir_public(home);
                let patch_dir = dsh_home.join("profiles").join("web");
                std::fs::create_dir_all(&patch_dir).unwrap();
                std::fs::write(
                    patch_dir.join("cordis.patch.yml"),
                    "# 用户自己的注释要保住\n- id: my-own-entry\n  name: \"@x/y\"\n  config:\n    foo: 1\n",
                ).unwrap();
                // .env 也放一个用户自己的变量
                std::fs::write(dsh_home.join(".env"), "MY_OWN_KEY=abc\nJOLU_API_KEY=my-env-key\n").unwrap();
                std::fs::write(dsh_home.join(".credentials.yaml"), "version: 1\nrefs:\n  JOLU_API_KEY: \"my: creds key\"\n  OTHER: keep\nrecords:\n  - id: 1\n").unwrap();
                let second = dsh_home.join("profiles").join("other");
                std::fs::create_dir_all(&second).unwrap();
                std::fs::write(second.join("cordis.patch.yml"), "# 另一个 profile\n- id: keep\n  name: user\n").unwrap();
                return;
            }
            "antigravity" => {
                // 用户自己的自定义模型 + 非 gemini 的 modelProvider；断开要原样还回来
                r#"{"customModelsConfig":{"customModels":{"mine":{"apiProvider":"API_PROVIDER_GOOGLE_GEMINI","modelName":"gemini-3-pro"}}},"model":"mine","modelProvider":"vertex","trustedWorkspaces":["/w"]}"#
            }
            "qwen" => {
                // 用户自己的一条 provider + 自己的 env 键 + 自己的默认模型
                r#"{"modelProviders":{"openai":[{"baseUrl":"https://my-own","envKey":"MY_KEY","id":"my-model","name":"My Model"}]},"env":{"MY_KEY":"sk-mine"},"model":{"name":"my-model","baseUrl":"https://my-own"},"keep":1}"#
            }
            "workbuddy" => {
                // 顶层数组：用户自己的一条 + 一个关掉的开关
                r#"[{"id":"my-own","name":"My Own","vendor":"Custom","apiKey":"sk-mine","url":"https://my-own/v1/chat/completions"},{"id":"off-one","name":"Off","vendor":"Custom","apiKey":"sk-2","url":"https://off","disabled":true}]"#
            }
            _ => panic!("{agent} 没有原始配置样本：给 `seed_original` 补一份"),
        };
        std::fs::write(path, text).unwrap();
    }

}
