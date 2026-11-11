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

use std::path::{Path, PathBuf};

use crate::gateway::wiring_store::{Applied, TierModels, WiringRecord};

/// 支持的 agent 标识（与 `providerCatalog` 的平台 id 对齐）。
pub const AGENTS: [&str; 5] = ["claude-code", "codex", "zcode", "gemini", "opencode"];

pub fn is_agent(agent: &str) -> bool {
    AGENTS.contains(&agent)
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
fn claude_capabilities(model: &str) -> String {
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
fn claude_wire(
    text: &str,
    base: &str,
    token: &str,
    model: Option<&str>,
    label: Option<&str>,
    description: Option<&str>,
    subagent_model: Option<&str>,
    tiers: Option<&TierModels>,
    tier_about: &dyn Fn(&str) -> (Option<String>, Option<String>),
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
fn claude_restore(
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

fn claude_current_base(text: &str) -> Option<String> {
    claude_env(text, "ANTHROPIC_BASE_URL")
}

fn claude_current_token(text: &str) -> Option<String> {
    claude_env(text, "ANTHROPIC_AUTH_TOKEN")
}

/// 读 Claude 接线涉及的全部键作为原值快照；网关写下的值一律不收。
///
/// `prior_model` 是上一次接线写下的主模型（取自接线记录的 applied）：重新接线时文件里的
/// 档位键 / 顶层 model 都等于它，但那不是用户的原值——收进快照会让「断开接线」把用户的
/// 模型名还原成网关来源名。
///
/// 快照结构：`{"env": {键: 原值}, "model": 顶层model原值}`；`model` 只在用户本来就有时出现。
fn claude_snapshot(
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
fn codex_wire(
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
fn codex_restore(text: &str, saved: Option<&serde_json::Value>, provider: &str) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    let current_provider = codex_current_provider(text);
    let current_base = document.get("openai_base_url").and_then(|v| v.as_str()).map(str::to_string);
    if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
        providers.remove(provider);
    }
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
fn codex_effective_base(text: &str) -> Option<String> {
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
fn codex_snapshot(text: &str) -> Option<serde_json::Value> {
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
fn opencode_wire(text: &str, base: &str, token: &str) -> Result<String, String> {
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

fn opencode_restore(text: &str, _saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove("jolu");
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

fn opencode_current_base(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("baseURL")?.as_str().map(str::to_string)
}

fn opencode_current_token(text: &str) -> Option<String> {
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

fn gemini_selected_type(settings_text: &str) -> Option<String> {
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

fn gemini_current_base(text: &str) -> Option<String> {
    dotenv_get(text, "GOOGLE_GEMINI_BASE_URL")
}

fn gemini_current_token(text: &str) -> Option<String> {
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

/// ZCode 的数据目录：`ZCODE_HOME` 指定的，否则 `~/.zcode`。
fn zcode_dir(home: &Path) -> PathBuf {
    std::env::var_os("ZCODE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".zcode"))
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

fn zcode_rules_restore(text: &str) -> Result<String, String> {
    let mut root = parse_object(text, "ZCode 的 provider_config.json ")?;
    if let Some(order) = root.pointer_mut("/config/providerOrder").and_then(|o| o.as_array_mut()) {
        order.retain(|v| v.as_str() != Some(ZCODE_PROVIDER));
    }
    if let Some(list) = root.pointer_mut("/config/providerConfigRules/providerRules").and_then(|l| l.as_array_mut()) {
        list.retain(|rule| !is_ours(rule));
    }
    if let Some(list) = root.pointer_mut("/config/modelConfigRules/providerModelRules").and_then(|l| l.as_array_mut()) {
        list.retain(|rule| !is_ours(rule));
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

fn zcode_config_restore(text: &str) -> Result<String, String> {
    let mut root = parse_object(text, "ZCode 的 config.json ")?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove(ZCODE_PROVIDER);
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

/// 旧版 Jolu 在 `~/.zcode/provider.json` 顶层写过 `jolu` 键（ZCode 从不读这个文件）：断开时顺带删掉。
fn zcode_clean_legacy(home: &Path) {
    let legacy = zcode_dir(home).join("provider.json");
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

fn zcode_current_base(text: &str) -> Option<String> {
    zcode_rule(text)?.pointer("/config/api/baseUrl")?.as_str().map(str::to_string)
}

fn zcode_current_token(text: &str) -> Option<String> {
    zcode_rule(text)?.pointer("/config/access/apiKey")?.as_str().map(str::to_string)
}

/// dotenv 键值写入：保留其它行与注释，只替换目标键（不存在则追加）。
fn wire_dotenv(text: &str, key: &str, value: Option<&str>) -> String {
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

fn dotenv_get(text: &str, key: &str) -> Option<String> {
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
const CODEX_PROVIDER: &str = "jolu-gateway";

/// `jolu-<agent>.<实例前 8 位>`：一键接线的归属标记，独立于可刷新的手动 Key。
fn token_for(agent: &str, instance_short: &str) -> String {
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

fn config_path(agent: &str) -> Option<PathBuf> {
    let home = crate::creds::home()?;
    Some(match agent {
        "claude-code" => crate::creds::claude_dir()?.join("settings.json"),
        "codex" => crate::creds::codex_dir()?.join("config.toml"),
        // ZCode 3.14 起从 v2/provider_config.json 的规则读 provider（2.9；此前写的 ~/.zcode/provider.json ZCode 不读）
        "zcode" => zcode_dir(&home).join("v2").join("provider_config.json"),
        "gemini" => home.join(".gemini").join(".env"),
        "opencode" => std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode")
            .join("opencode.json"),
        _ => return None,
    })
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
fn backup(path: &Path, backup_dir: &Path) -> Result<(), String> {
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
    let token = match agent {
        "claude-code" => claude_current_token(text),
        "codex" => text
            .parse::<toml_edit::DocumentMut>()
            .ok()
            .and_then(|doc| {
                doc.get("model_providers")?
                    .get(CODEX_PROVIDER)?
                    .get("experimental_bearer_token")?
                    .as_str()
                    .map(str::to_string)
            }),
        "gemini" => gemini_current_token(text),
        "opencode" => opencode_current_token(text),
        "zcode" => zcode_current_token(text),
        _ => None,
    }?;
    if !token.starts_with("jolu-") {
        return None;
    }
    let (_, suffix) = token.rsplit_once('.')?;
    (suffix != instance_short).then(|| suffix.to_string())
}

/// 当前配置是否由本实例写入（状态判断用）。
fn ours(agent: &str, text: &str, port: u16, instance_short: &str) -> bool {
    ours_with(agent, text, port, instance_short, None)
}

/// `kept` 是接线时保留下来的调用方 Key（keyHolds）：文件里是它也算我们的接线。
fn ours_with(agent: &str, text: &str, port: u16, instance_short: &str, kept: Option<&str>) -> bool {
    match agent {
        "claude-code" => {
            let base = claude_current_base(text).unwrap_or_default();
            let token = claude_current_token(text).unwrap_or_default();
            points_at_gateway(&base, port)
                && (token == format!("jolu-claude-code")
                    || token == token_for("claude-code", instance_short)
                    || kept.is_some_and(|kept| !kept.is_empty() && token == kept))
        }
        "codex" => codex_effective_base(text).is_some_and(|base| points_at_gateway(&base, port)),
        "gemini" => {
            let base_ok = gemini_current_base(text).is_some_and(|base| points_at_gateway(&base, port));
            let selected = gemini_settings_text()
                .as_deref()
                .and_then(gemini_selected_type);
            base_ok && selected.as_deref() == Some("gemini-api-key")
        }
        "opencode" => opencode_current_base(text).is_some_and(|base| points_at_gateway(&base, port)),
        "zcode" => zcode_current_base(text).is_some_and(|base| points_at_gateway(&base, port)),
        _ => false,
    }
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
        };
    };
    let display = path.display().to_string();
    let installed = path.exists() || (agent == "gemini" && gemini_settings_path().is_some_and(|p| p.exists()));
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
    WiringStatus {
        agent: agent.into(),
        wired,
        config_path: Some(display),
        notice: Some(restart_notice(agent).to_string()),
        reason: None,
        state: record.map(|r| r.state.clone()),
        drift,
        codex_mode,
    }
}

fn restart_notice(agent: &str) -> &'static str {
    match agent {
        "claude-code" => "新会话生效；已在运行的会话需重开",
        // 这句话必须让用户知道登录方式被临时切走了
        "gemini" => "接线期间改用网关 Key，断开时恢复原登录；需重启 Gemini 后生效",
        "codex" => "保留 ChatGPT 登录时只改请求地址；需重启 Codex 后生效",
        "opencode" => "需重启该工具后生效",
        "zcode" => "需重启 ZCode 后生效",
        _ => "需重启后生效",
    }
}

/// 重新应用时优先沿用旧快照；只有第一次接线才使用当前文件算出的值。
fn effective_snapshot(prior: Option<&str>, computed: Option<serde_json::Value>) -> Option<String> {
    prior.map(str::to_string).or_else(|| computed.map(|s| s.to_string()))
}

/// 接入：读原值 → 拒绝别的实例 → 备份 → 原子写。返回（原值快照，实际写入的值）。
/// `before_write` 在文件落盘前调用：调用方用它先写 pending 记录，崩溃时才看得出写到哪一步。
/// `prior_snapshot` 是上一次接线保存的原值：重新应用时用它，不能拿当前文件重新快照——
/// 当前文件里可能已经是我们写进去的网关地址，重算会把用户的真实上游弄丢。
/// `model` 只对 Claude Code 生效：网关按 `<来源>/<模型>` 选路，不写模型映射工具就会发自带别名。
/// `prior_model` 是上一次接线写下的模型映射（取自接线记录的 applied），重新接线时用来把它
/// 从「用户原值」里排除——否则断开接线会把用户的模型名还原成网关来源名。
pub(crate) fn wire(
    agent: &str,
    port: u16,
    instance_short: &str,
    backup_dir: &Path,
    prior_snapshot: Option<&str>,
    model: Option<&str>,
    label: Option<&str>,
    description: Option<&str>,
    prior_model: Option<&str>,
    prior_label: Option<&str>,
    subagent_model: Option<&str>,
    prior_values: &[String],
    tiers: Option<&TierModels>,
    tier_about: &dyn Fn(&str) -> (Option<String>, Option<String>),
    holds: &dyn Fn(&str) -> bool,
    zcode_models: &[ZcodeModel],
    before_write: impl FnOnce(Option<&str>, &Applied) -> Result<(), String>,
) -> Result<(Option<String>, Applied), String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    let base = format!("http://127.0.0.1:{port}");
    let mut token = token_for(agent, instance_short);
    let original = if crate::steady::exists(&path) {
        crate::steady::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    if let Some(suffix) = foreign_marker(agent, &original, instance_short) {
        return Err(format!(
            "该工具已被另一个 Jolu 实例接管（尾号 {suffix}）；请先在那个实例里断开，或手动改回直连"
        ));
    }
    // keyHolds（magpie 同款）：Claude Code 里已经是网关认的命名客户端 Key 时保留它——那把 Key
    // 带着用户设的每日上限与并发，换成 Jolu 的令牌就把这些约束丢了
    if agent == "claude-code" {
        if let Some(existing) = claude_current_token(&original).filter(|t| !t.starts_with("jolu-") && holds(t)) {
            token = existing;
        }
    }
    // Codex 已登录 ChatGPT 时保留 OAuth；否则才写自定义 provider + 静态令牌。
    let codex_oauth = agent == "codex" && codex_oauth_logged_in();
    let mut extra_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let (next, snapshot) = match agent {
        "claude-code" => {
            let snapshot = claude_snapshot(&original, prior_model, prior_label, prior_values);
            let text = if original.trim().is_empty() { "{}" } else { original.as_str() };
            (claude_wire(text, &base, &token, model, label, description, subagent_model, tiers, tier_about)?, snapshot)
        }
        "codex" => {
            let snapshot = codex_snapshot(&original);
            let token = (!codex_oauth).then_some(token.as_str());
            (codex_wire(&original, &base, CODEX_PROVIDER, codex_oauth, token)?, snapshot)
        }
        "gemini" => {
            let settings_path = gemini_settings_path().ok_or("未找到用户目录")?;
            let settings_text = gemini_settings_text().unwrap_or_default();
            let plan = gemini_plan(&original, &settings_text, &base, &token)?;
            extra_files.push((settings_path, plan.settings.into_bytes()));
            (plan.env, Some(plan.snapshot))
        }
        "opencode" => (opencode_wire(&original, &base, &token)?, None),
        "zcode" => {
            // 两份都写：新版 ZCode 读规则文件，旧版读 config.json（magpie 同款）
            let home = crate::creds::home().ok_or("未找到用户目录")?;
            let config_path = zcode_dir(&home).join("v2").join("config.json");
            let config_text = crate::steady::read_optional(&config_path).map_err(|error| format!("读取 {} 失败：{error}", config_path.display()))?.unwrap_or_default();
            extra_files.push((config_path, zcode_config_wire(&config_text, &base, &token, zcode_models)?.into_bytes()));
            (zcode_rules_wire(&original, &base, &token, zcode_models)?, None)
        }
        _ => return Err("该工具不在接线范围".into()),
    };
    let mut files = extra_files;
    files.push((path, next.into_bytes()));
    for (file, _) in &files {
        backup(file, backup_dir)?;
    }
    let applied = Applied {
        base: if codex_oauth { format!("{base}/backend-api/codex") } else { base.clone() },
        token: if agent == "codex" {
            (!codex_oauth).then(|| token.clone())
        } else {
            matches!(agent, "claude-code" | "gemini" | "opencode" | "zcode").then(|| token.clone())
        },
        provider: (agent == "codex" && !codex_oauth).then(|| CODEX_PROVIDER.to_string()),
        mode: (agent == "codex").then(|| if codex_oauth { "oauth" } else { "provider" }.to_string()),
        model: (agent == "claude-code")
            .then(|| model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string))
            .flatten(),
        label: (agent == "claude-code")
            .then(|| label.map(str::trim).filter(|l| !l.is_empty()).map(str::to_string))
            .flatten(),
        subagent_model: (agent == "claude-code")
            .then(|| subagent_model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string))
            .flatten(),
        tiers: (agent == "claude-code").then(|| tiers.cloned()).flatten(),
    };
    let snapshot = effective_snapshot(prior_snapshot, snapshot);
    before_write(snapshot.as_deref(), &applied)?;
    write_all_with_rollback(&files)?;
    Ok((snapshot, applied))
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
pub(crate) fn unwire(
    agent: &str,
    port: u16,
    record: Option<&WiringRecord>,
) -> Result<bool, String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    // Gemini 可能只剩 settings.json 被改过（.env 被外部删了），仍然要还原 auth。
    if !crate::steady::exists(&path) && agent != "gemini" {
        return Ok(false);
    }
    let original = if crate::steady::exists(&path) {
        crate::steady::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    let instance = record.map(|r| r.instance_id.chars().take(8).collect::<String>()).unwrap_or_default();
    // 别的实例写的配置不能由我们还原；没有记录（旧版升级）时不做这层判断。
    if !instance.is_empty() && foreign_marker(agent, &original, &instance).is_some() {
        return Ok(false);
    }
    // 记录里存的是接线前的原值；没有记录时按「原来没有该键」处理。
    let saved: Option<serde_json::Value> = record
        .and_then(|r| r.original_json.as_deref())
        .and_then(|s| serde_json::from_str(s).ok());
    // ZCode 要同时还原规则文件与 config.json，并清掉旧版 Jolu 写在 provider.json 里的键
    if agent == "zcode" {
        let base_ours = zcode_current_base(&original).is_some_and(|base| is_gateway_value(&base));
        let token_ours = zcode_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
        if !base_ours && !token_ours {
            return Ok(false);
        }
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let config_path = zcode_dir(&home).join("v2").join("config.json");
        let mut files = vec![(path.clone(), zcode_rules_restore(&original)?.into_bytes())];
        if let Ok(Some(config_text)) = crate::steady::read_optional(&config_path) {
            files.push((config_path, zcode_config_restore(&config_text)?.into_bytes()));
        }
        write_all_with_rollback(&files)?;
        zcode_clean_legacy(&home);
        return Ok(true);
    }
    // Gemini 要同时还原 `.env` 与 settings.json，单独走多文件路径。
    if agent == "gemini" {
        let base_ours = gemini_current_base(&original).is_some_and(|base| is_gateway_value(&base));
        let token_ours = gemini_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
        let settings_ours = record.is_some()
            && gemini_settings_text()
                .as_deref()
                .and_then(gemini_selected_type)
                .as_deref()
                == Some("gemini-api-key");
        if !base_ours && !token_ours && !settings_ours {
            return Ok(false);
        }
        let settings_path = gemini_settings_path().ok_or("未找到用户目录")?;
        let settings_text = gemini_settings_text().unwrap_or_default();
        let (env, settings) = gemini_restore(&original, &settings_text, saved.as_ref());
        write_all_with_rollback(&[
            (settings_path, settings.into_bytes()),
            (path, env.into_bytes()),
        ])?;
        return Ok(true);
    }
    let next = match agent {
        "claude-code" => {
            // 外部意图优先：当前 base 已不是网关地址、且 token 不是 Jolu 写的，就不动
            let base = claude_current_base(&original).unwrap_or_default();
            let token = claude_current_token(&original).unwrap_or_default();
            if !points_at_gateway(&base, port) && !is_gateway_value(&base) && !token.starts_with("jolu-") {
                return Ok(false);
            }
            // 上次接线写下的主模型与显示名：没有快照时靠它们识别「哪些键是我们写的」
            let applied = record.and_then(|r| r.applied());
            let prior_model = applied.as_ref().and_then(|a| a.model.clone());
            let prior_label = applied.as_ref().and_then(|a| a.label.clone());
            let prior_values = applied.as_ref().map(Applied::written_values).unwrap_or_default();
            claude_restore(
                &original,
                saved.as_ref(),
                prior_model.as_deref(),
                prior_label.as_deref(),
                &prior_values,
            )?
        }
        "codex" => {
            let points_at_us = codex_effective_base(&original).is_some_and(|base| is_gateway_value(&base))
                || codex_provider_base(&original, CODEX_PROVIDER).is_some_and(|base| is_gateway_value(&base));
            if !points_at_us {
                return Ok(false);
            }
            codex_restore(&original, saved.as_ref(), CODEX_PROVIDER)?
        }
        "opencode" => {
            let base_ours = opencode_current_base(&original).is_some_and(|base| is_gateway_value(&base));
            let token_ours = opencode_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
            if !base_ours && !token_ours {
                return Ok(false);
            }
            opencode_restore(&original, saved.as_ref())?
        }
        _ => return Err("该工具不在接线范围".into()),
    };
    write_atomic(&path, next.into_bytes())?;
    Ok(true)
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
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", None, None, None, None, None, &no_about).unwrap();
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
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["model"], "zcode/glm-4.6", "主模型在 settings 顶层");
        assert!(value["env"].get("ANTHROPIC_MODEL").is_none(), "ANTHROPIC_MODEL 要删掉");
        for tier in CLAUDE_TIERS {
            assert_eq!(value["env"][claude_tier_env(tier)], "zcode/glm-4.6", "{tier} 档跟随主模型");
        }
        assert_eq!(value["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "zcode/glm-4.6", "后台小任务也要能路由");
        // 每档写入显示名与描述（/model 菜单的档位标签），旧「关于」值被覆盖不再张冠李戴
        for tier in CLAUDE_TIERS {
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
        None, &no_about)
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
        None, &no_about)
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "kimi/kimi-k2.5");

        // 没给：删键（跟随主模型，Claude Code 的缺省）
        let original = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"old/model"}}"#;
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, None, None, &no_about).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none(), "不给就删键＝跟随主模型");

        // 断开还原：用户原来的 subagent 模型要回来
        let original = r#"{"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"user/own-sub"}}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, Some("zcode/glm-4.6"), None, &no_about).unwrap();
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
            Some("zcode/glm-5.3-flash"), Some(&tiers), &tier_about,
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
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), None, None, Some("zcode/glm-5.3-flash"), None, &no_about).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        for tier in CLAUDE_TIERS {
            assert_eq!(value["env"][claude_tier_env(tier)], "zcode/glm-4.6", "{tier} 跟随主模型");
        }
        assert!(value["env"].get("CLAUDE_CODE_AUTO_MODE_MODEL").is_none(), "Auto 判别没自选就删键");
        assert_eq!(value["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "zcode/glm-5.3-flash");
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
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code.abcd1234", Some("zcode/glm-5.3"), Some("GLM 5.3"), None, None, Some(&tiers), &|v: &str| (Some(format!("{v} 名")), None)).unwrap();
        let applied = Applied { tiers: Some(tiers.clone()), ..Default::default() };
        let snapshot = claude_snapshot(&wired, Some("zcode/glm-5.3"), Some("GLM 5.3"), &applied.written_values());
        assert!(snapshot.is_none(), "自选档位值、Auto 值、自选档位的显示名都是我们写的：{snapshot:?}");
        // 断开：没有快照时这些键也要删干净
        let restored = claude_restore(&wired, None, Some("zcode/glm-5.3"), Some("GLM 5.3"), &applied.written_values()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").map_or(true, |env| env.get("CLAUDE_CODE_AUTO_MODE_MODEL").is_none() && env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none()), "{restored}");
    }

    #[test]
    fn tiers_declare_their_effort_capabilities() {
        let tiers = TierModels { opus: Some("c/claude-opus-4-5".into()), sonnet: None, haiku: Some("c/claude-haiku-4-5".into()), fable: None, auto_mode: None };
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-5.3"), None, None, None, Some(&tiers), &no_about).unwrap();
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
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", Some("c/claude-opus-4-5"), None, None, None, None, &no_about).unwrap();
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
        let wired = claude_wire(original, "http://127.0.0.1:12731", "tok", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about).unwrap();
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
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", None, None, None, None, None, &no_about).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "already/set");
        assert!(value["env"].get("ANTHROPIC_SMALL_FAST_MODEL").is_none(), "没给模型就不该凭空加键");
        assert!(value.get("model").is_none(), "没给模型就不动顶层 model");
    }

    #[test]
    fn claude_wire_ignores_blank_model() {
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", Some("   "), Some("    显示名"), Some("    的描述，用于 /model 菜单"), None, None, &no_about).unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert!(value.get("model").is_none(), "空白模型名不应写成空字符串");
    }

    #[test]
    fn claude_restore_puts_back_the_original_model_mapping() {
        // 用户本来就有自己的档位映射：断开接线必须还原成他的，而不是删掉。
        let original = r#"{"env":{"ANTHROPIC_MODEL":"user/own","ANTHROPIC_SMALL_FAST_MODEL":"user/fast","ANTHROPIC_DEFAULT_OPUS_MODEL":"user/opus"},"model":"user-main"}"#;
        let snapshot = claude_snapshot(original, None, None, &[]).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about).unwrap();
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
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "jolu-claude-code", Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), Some("zcode/glm-4.6 的描述，用于 /model 菜单"), None, None, &no_about).unwrap();
        // prior_model 来自接线记录的 applied：断开时靠它识别「顶层 model / 档位键是我们写的」
        let restored = claude_restore(&wired, None, Some("zcode/glm-4.6"), Some("zcode/glm-4.6 显示名"), &[]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").is_none(), "全部是网关写入的键，env 段要清空");
        assert!(value.get("model").is_none(), "顶层 model 也要清掉");
    }

    #[test]
    fn claude_restore_accepts_legacy_flat_snapshot() {
        // 升级兼容：旧版接线记录的快照是 env 键的平面表，没有 env 子对象。
        let wired = claude_wire("{}", "http://127.0.0.1:12731", "tok", None, None, None, None, None, &no_about).unwrap();
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
        let restored = zcode_rules_restore(&wired).unwrap();
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
        let restored: serde_json::Value = serde_json::from_str(&zcode_rules_restore(&wired).unwrap()).unwrap();
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
        let restored: serde_json::Value = serde_json::from_str(&zcode_config_restore(&wired.to_string()).unwrap()).unwrap();
        assert!(restored["provider"].get("jolu").is_none() && restored["provider"].get("builtin:zai").is_some());
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
        for agent in ["claude-code", "codex", "zcode", "gemini", "opencode"] {
            assert!(is_agent(agent), "{agent}");
        }
        assert!(!is_agent("qoder"), "Qoder 不在接线范围（不代管密钥）");
        assert!(!is_agent("workbuddy"));
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
        let wired = claude_wire(&original, "http://127.0.0.1:12731", "jolu-claude-code.abcd1234", None, None, None, None, None, &no_about).unwrap();
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
}
