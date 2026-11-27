use std::path::Path;
use crate::gateway::wiring_store::{TierModels, CLAUDE_EFFORTS, split_tier_value, tier_efforts};
use super::{is_gateway_value, model_key};

pub(crate) const CLAUDE_TIERS: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];

pub(crate) fn claude_tier_env(tier: &str) -> String {
    format!("ANTHROPIC_DEFAULT_{}_MODEL", tier.to_ascii_uppercase())
}

/// 每档的「关于」键：`/model` 菜单据此显示档位模型的名称与描述。
/// 档位换模型时旧值会张冠李戴（magpie #1227），所以接线时一并清掉、断开时还原。
pub(crate) const CLAUDE_ABOUT: [&str; 3] = ["_NAME", "_DESCRIPTION", "_SUPPORTED_CAPABILITIES"];

/// 云端开关（magpie `claudeClouds` 同款，6 个）：它们会把请求引去 Bedrock / Vertex /
/// Foundry / Anthropic on AWS / Anthropic on GCP / Bedrock Mantle，绕过 ANTHROPIC_BASE_URL，
/// 接线期间必须移除（原值暂存快照、断开还原）；云专属变量（AWS_*、ANTHROPIC_VERTEX_PROJECT_ID
/// 等）不动——开关移除后 Claude Code 不读它们，断开时自然恢复生效。
pub(crate) const CLAUDE_CLOUD_SWITCHES: [&str; 6] = [
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
pub(crate) const CLAUDE_EXTRA_KEYS: [&str; 5] = [CLAUDE_MAX_OUTPUT, CLAUDE_MAX_CONTEXT, CLAUDE_AUTO_MODE_SERVER, CLAUDE_COMPACT_WINDOW, CLAUDE_MODEL_CAPABILITIES];

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
pub(crate) fn is_claude_family(model: &str) -> bool {
    let (bare, _) = crate::gateway::wiring_store::split_tier_value(model);
    bare.rsplit('/').next().unwrap_or(bare).to_ascii_lowercase().contains("claude")
}

/// 主模型是不是 Fable 档家族（B3：只有它才配写进 Fable 档）。
pub(crate) fn is_fable_family(model: &str) -> bool {
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
pub(crate) fn drop_our_extras(mut saved: serde_json::Value, previous: &std::collections::BTreeMap<String, String>) -> Option<serde_json::Value> {
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
pub(crate) fn claude_env_keys() -> Vec<String> {
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
pub(crate) fn fit_effort(model: &str, effort: &str) -> Option<Option<String>> {
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
pub(crate) fn claude_wire(
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
pub(crate) fn written_by_us(value: &str, key: &str, prior_model: Option<&str>, prior_label: Option<&str>, prior_values: &[String]) -> bool {
    let value = value.trim();
    let about = CLAUDE_ABOUT.iter().any(|suffix| key.ends_with(suffix));
    prior_model.is_some_and(|model| value == model.trim())
        || (about && (prior_model.is_some() || prior_label.is_some_and(|label| value == label.trim())))
        || prior_values.iter().any(|written| value == written.trim())
}

/// 快照的结构：新版带 `env` 子对象与顶层 `model`；旧版是 env 键的平面表（升级兼容）。
pub(crate) fn claude_snapshot_is_structured(saved: &serde_json::Value) -> bool {
    // 新版快照只在用户有原值时才带对应部分：只有顶层 model / effortLevel、没有 env 键的
    // 也是新版（旧版平面表的键全是 env 变量名，不会叫 model / effortLevel）
    saved.get("env").is_some() || saved.get("model").is_some() || saved.get("effortLevel").is_some()
}

/// 还原 Claude：只删自己写的键。`saved` 是接线前读到的原值快照（新旧两种结构都认）。
///
/// `prior_model` 是上一次接线写下的主模型（取自接线记录的 applied）：没有快照时，
/// 顶层 `model` 与各档位键若等于它，就是我们写的，要删——不能把网关来源名留给用户。
#[allow(clippy::too_many_arguments)]
pub(crate) fn claude_restore(
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

pub(crate) fn claude_env(text: &str, key: &str) -> Option<String> {
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
pub(crate) fn claude_snapshot(
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

pub(crate) fn claude_conflicts(text: &str) -> Vec<String> {
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
