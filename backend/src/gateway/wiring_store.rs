//! agent 接线记录（`agent_wiring` 表）。
//!
//! 记录三件事：谁在接管（owner）、写到哪一步（state）、写下了什么（applied）。
//! 先记 pending 再写文件，写完改成 wired；启动时按文件里的实际值修复 pending。
//! `settings.gateway_wiring` 在过渡版本里继续双写，方便回退到旧版。

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, serde::Serialize)]
pub struct WiringRecord {
    pub agent: String,
    pub instance_id: String,
    /// none | observe | relay
    pub owner: String,
    /// idle | pending | wired
    pub state: String,
    pub original_json: Option<String>,
    pub applied_json: Option<String>,
    pub token: Option<String>,
    pub wired_ms: Option<i64>,
}

impl WiringRecord {
    pub fn applied(&self) -> Option<Applied> {
        serde_json::from_str(self.applied_json.as_deref()?).ok()
    }
}

/// 我们实际写进配置的值。状态与断开都以它为准，不按 settings 猜。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct Applied {
    pub base: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// 接线方式：oauth（保留登录，写 openai_base_url）或 provider（自定义 provider + 静态令牌）。
    #[serde(default)]
    pub mode: Option<String>,
    /// 只对 Claude Code：写进 `ANTHROPIC_MODEL` / `ANTHROPIC_SMALL_FAST_MODEL` 的 `<来源>/<模型>`。
    /// 重新接线算原值快照时要拿它判断「这个模型名是我们写的」，不能当成用户原来的设置。
    #[serde(default)]
    pub model: Option<String>,
    /// 只对 Claude Code：写进各档 `_NAME` / `_DESCRIPTION` 的显示名（models.dev label）。
    /// 快照排除逻辑同 model：我们写下的显示名不是用户原值。
    #[serde(default)]
    pub label: Option<String>,
    /// 只对 Claude Code：写进 `CLAUDE_CODE_SUBAGENT_MODEL` 的 subagent 独立模型；None = 跟随主模型。
    #[serde(default)]
    pub subagent_model: Option<String>,
    /// 只对 Claude Code：四个档位与 Auto 判别的独立模型（批 4 档位深耕）。
    /// 值是**最终写入值**（自选 = `model[:effort]`，可带 [1m] 后缀）；None = 跟随主模型。
    /// 旧快照没有此字段 → 反序列化为 None = 全跟随，行为与升级前一致。
    #[serde(default)]
    pub tiers: Option<TierModels>,
}

/// Claude Code 的档位自选集合。字段与 `ANTHROPIC_DEFAULT_<TIER>_MODEL` 一一对应，
/// `auto_mode` 对应 `CLAUDE_CODE_AUTO_MODE_MODEL`（Claude Code 每个动作先跑的分类器）。
///
/// 「自选保持」语义（magpie `tier_own` 同款）：这里存的是绝对值——用户给某档选过
/// 模型后，更换主模型时该档的值不重算；只有仍为 None 的档位在重接线时跟随新主模型。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TierModels {
    #[serde(default)]
    pub opus: Option<String>,
    #[serde(default)]
    pub sonnet: Option<String>,
    #[serde(default)]
    pub haiku: Option<String>,
    #[serde(default)]
    pub fable: Option<String>,
    /// 前端类型是 camelCase 的 `autoMode`：序列化与之一致，旧快照里的 `auto_mode` 照常读回
    #[serde(default, rename = "autoMode", alias = "auto_mode")]
    pub auto_mode: Option<String>,
}

/// Claude Code 的 effort 全集（2.1.285 起），也是它对未知模型（别家经网关的模型）发的范围。
pub(crate) const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// 写在档位值里会被当成 effort 的词。冒号后不是这些词的（`qwen3:32b`、`llama3:latest`）属于模型名本身，
/// 所以拼错的 effort 无法在这里识别，会原样当模型名写入。
const EFFORT_WORDS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// 档位值拆成（模型，effort）：`<来源>/<模型>[1m]:<effort>`，effort 可省。
pub(crate) fn split_tier_value(value: &str) -> (&str, Option<&str>) {
    match value.rsplit_once(':') {
        Some((model, effort)) if EFFORT_WORDS.contains(&effort.trim().to_ascii_lowercase().as_str()) => (model, Some(effort.trim())),
        _ => (value, None),
    }
}

/// 某个模型在 Claude Code 里可设的 effort：认得的 Claude 模型按 Claude Code 实际发送的范围，
/// Haiku 与更早的模型没有（空），新模型与别家模型是全集。
///
/// 为什么：effort 填了模型不收的值，Claude Code 会静默按更低一档发或直接报错，用户看不出
/// 档位没按设定生效。参照 magpie `internal/agent/claude.go::claudeEffortsFor`（克隆 `0ee424e`）；
/// 差异：magpie 先查供应商目录里的 efforts，Jolu 的 models.dev 元数据没有这一项，只用内置表。
pub(crate) fn tier_efforts(model: &str) -> Option<&'static [&'static str]> {
    let id = model.rsplit('/').next().unwrap_or(model).trim_end_matches("[1m]").to_ascii_lowercase().replace('.', "-");
    let Some(at) = id.find("claude-") else { return Some(CLAUDE_EFFORTS) };
    let rest = &id[at + "claude-".len()..];
    if rest.starts_with('3') || rest.starts_with("instant") || rest.starts_with('2') {
        return Some(&[]);
    }
    let mut parts = rest.split('-');
    let family = parts.next().unwrap_or_default();
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().filter(|p| p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit())).unwrap_or_default();
    let name = if minor.is_empty() { format!("{family}-{major}") } else { format!("{family}-{major}-{minor}") };
    Some(match name.as_str() {
        "opus-4-5" => &["low", "medium", "high"],
        "opus-4-6" | "sonnet-4-6" => &["low", "medium", "high", "max"],
        "haiku-4-5" | "sonnet-4-5" | "opus-4" | "sonnet-4" | "opus-4-1" | "opus-4-0" | "sonnet-4-0" => &[],
        _ => CLAUDE_EFFORTS,
    })
}

impl TierModels {
    /// 写入前校验每档的 effort：拼错的词、模型不收的档位都拒绝，并说出可选值。
    pub fn validate(&self) -> Result<(), String> {
        self.validate_with(&|_| None)
    }

    /// 同 [`validate`]，`efforts_of` 给出 models.dev 记录的该模型档位（有就以它为准，
    /// 只保留 Claude Code 认得的档；没有或为空时按内置表 [`tier_efforts`]）。
    pub fn validate_with(&self, efforts_of: &dyn Fn(&str) -> Option<Vec<String>>) -> Result<(), String> {
        for (tier, value) in [
            ("opus", &self.opus), ("sonnet", &self.sonnet), ("haiku", &self.haiku), ("fable", &self.fable), ("auto", &self.auto_mode),
        ] {
            let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else { continue };
            let (model, effort) = split_tier_value(value);
            let Some(effort) = effort else { continue };
            let known: Option<Vec<&'static str>> = efforts_of(model)
                .map(|list| CLAUDE_EFFORTS.iter().copied().filter(|level| list.iter().any(|l| l == level)).collect::<Vec<_>>())
                .filter(|list| !list.is_empty());
            let allowed: Vec<&'static str> = known.unwrap_or_else(|| tier_efforts(model).unwrap_or(CLAUDE_EFFORTS).to_vec());
            if allowed.is_empty() {
                return Err(format!("{tier} 档的 {model} 不支持 effort，请去掉「:{effort}」"));
            }
            if !allowed.contains(&effort.to_ascii_lowercase().as_str()) {
                return Err(format!("{tier} 档的 {model} 不能设 effort「{effort}」，可选：{}", allowed.join(" / ")));
            }
        }
        Ok(())
    }
    /// 按 tier 名取值（"opus" / "sonnet" / "haiku" / "fable"；其他名字返回 None）。
    pub fn tier(&self, tier: &str) -> Option<&str> {
        match tier {
            "opus" => self.opus.as_deref(),
            "sonnet" => self.sonnet.as_deref(),
            "haiku" => self.haiku.as_deref(),
            "fable" => self.fable.as_deref(),
            _ => None,
        }
    }
}

/// 来源改 id（slug）后，接线里写过的 `旧id/模型` 都要换成 `新id/模型`，否则工具用旧 id 请求只会拿到 404。
/// 返回（主模型，子代理，档位）的新值；没有任何一项用到旧 id 时返回 None（不用重写）。
pub fn renamed_slug(applied: &Applied, old: &str, new: &str) -> Option<(Option<String>, Option<String>, Option<TierModels>)> {
    let prefix = format!("{old}/");
    let swap = |value: &Option<String>| -> (Option<String>, bool) {
        match value {
            Some(v) if v.starts_with(&prefix) => (Some(format!("{new}/{}", &v[prefix.len()..])), true),
            other => (other.clone(), false),
        }
    };
    let (model, a) = swap(&applied.model);
    let (subagent, b) = swap(&applied.subagent_model);
    let mut changed = a || b;
    let tiers = applied.tiers.as_ref().map(|tiers| {
        let mut out = tiers.clone();
        for slot in [&mut out.opus, &mut out.sonnet, &mut out.haiku, &mut out.fable, &mut out.auto_mode] {
            let (next, hit) = swap(slot);
            changed |= hit;
            *slot = next;
        }
        out
    });
    changed.then_some((model, subagent, tiers))
}

impl Applied {
    /// 上次接线写进配置的模型类值（子代理、各档自选、Auto 判别）：重新快照与还原时
    /// 据此认出「这是 Jolu 写的」，不能当用户原值收进快照、断开时再写回去。
    pub fn written_values(&self) -> Vec<String> {
        let mut values: Vec<String> = self.subagent_model.iter().cloned().collect();
        if let Some(tiers) = &self.tiers {
            values.extend([&tiers.opus, &tiers.sonnet, &tiers.haiku, &tiers.fable, &tiers.auto_mode].into_iter().flatten().cloned());
        }
        values.retain(|v| !v.trim().is_empty());
        values
    }
}

pub fn load(conn: &Connection, agent: &str) -> Option<WiringRecord> {
    conn.query_row(
        "SELECT agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms
         FROM agent_wiring WHERE agent_id = ?1",
        [agent],
        |r| {
            Ok(WiringRecord {
                agent: r.get(0)?,
                instance_id: r.get(1)?,
                owner: r.get(2)?,
                state: r.get(3)?,
                original_json: r.get(4)?,
                applied_json: r.get(5)?,
                token: r.get(6)?,
                wired_ms: r.get(7)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

pub fn all(conn: &Connection) -> Vec<WiringRecord> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms
         FROM agent_wiring",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(WiringRecord {
            agent: r.get(0)?,
            instance_id: r.get(1)?,
            owner: r.get(2)?,
            state: r.get(3)?,
            original_json: r.get(4)?,
            applied_json: r.get(5)?,
            token: r.get(6)?,
            wired_ms: r.get(7)?,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok).collect()
}

/// 写文件之前先落 pending：崩溃后能看出「打算写、可能写了一半」。
pub fn mark_pending(
    conn: &Connection,
    agent: &str,
    instance_id: &str,
    owner: &str,
    original_json: Option<&str>,
    applied: &Applied,
    token: Option<&str>,
) -> rusqlite::Result<()> {
    let applied_json = serde_json::to_string(applied).unwrap_or_default();
    conn.execute(
        "INSERT INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
         VALUES(?1, ?2, ?3, 'pending', ?4, ?5, ?6, NULL)
         ON CONFLICT(agent_id) DO UPDATE SET
             instance_id = excluded.instance_id,
             owner = excluded.owner,
             state = 'pending',
             original_json = excluded.original_json,
             applied_json = excluded.applied_json,
             token = excluded.token,
             wired_ms = NULL",
        rusqlite::params![agent, instance_id, owner, original_json, applied_json, token],
    )?;
    Ok(())
}

pub fn mark_wired(conn: &Connection, agent: &str, wired_ms: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE agent_wiring SET state = 'wired', wired_ms = ?2 WHERE agent_id = ?1",
        rusqlite::params![agent, wired_ms],
    )?;
    Ok(())
}

/// 断开后回到 idle；保留 owner 便于界面说明「这个工具现在归直连」。
pub fn mark_idle(conn: &Connection, agent: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE agent_wiring SET state = 'idle', applied_json = NULL, original_json = NULL, wired_ms = NULL
         WHERE agent_id = ?1",
        [agent],
    )?;
    Ok(())
}

pub fn set_owner_all(conn: &Connection, owner: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE agent_wiring SET owner = ?1", [owner])?;
    Ok(())
}

/// 从旧设置导入接线记录（只导入不存在的 agent；旧版回退后仍可读 settings）。
pub fn import_legacy(conn: &Connection, snapshots: &std::collections::BTreeMap<String, String>, instance_id: &str) -> usize {
    let mut imported = 0;
    for (agent, snapshot) in snapshots {
        if load(conn, agent).is_some() {
            continue;
        }
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
             VALUES(?1, ?2, 'relay', 'wired', ?3, NULL, NULL, NULL)",
            rusqlite::params![agent, instance_id, snapshot],
        );
        if inserted.unwrap_or(0) > 0 {
            imported += 1;
        }
    }
    imported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_a_source_rewrites_every_value_that_named_it() {
        let applied = Applied {
            model: Some("kimi/kimi-k2[1m]".into()),
            subagent_model: Some("zcode/glm".into()),
            tiers: Some(TierModels { opus: Some("kimi/kimi-k2:high".into()), sonnet: None, haiku: Some("kimi-old/x".into()), fable: None, auto_mode: Some("kimi/k".into()) }),
            ..Default::default()
        };
        let (model, subagent, tiers) = renamed_slug(&applied, "kimi", "moonshot").expect("用到了旧 id");
        assert_eq!(model.as_deref(), Some("moonshot/kimi-k2[1m]"));
        assert_eq!(subagent.as_deref(), Some("zcode/glm"), "别的来源不动");
        let tiers = tiers.unwrap();
        assert_eq!(tiers.opus.as_deref(), Some("moonshot/kimi-k2:high"));
        assert_eq!(tiers.haiku.as_deref(), Some("kimi-old/x"), "前缀要整段匹配，kimi-old 不是 kimi");
        assert_eq!(tiers.auto_mode.as_deref(), Some("moonshot/k"));
        assert!(renamed_slug(&applied, "nobody", "x").is_none());
    }

    #[test]
    fn auto_mode_accepts_the_frontend_camel_case_name() {
        let tiers: TierModels = serde_json::from_str(r#"{"opus":null,"autoMode":"zcode/glm-5.3-flash"}"#).unwrap();
        assert_eq!(tiers.auto_mode.as_deref(), Some("zcode/glm-5.3-flash"));
        let old: TierModels = serde_json::from_str(r#"{"auto_mode":"zcode/glm-5.3"}"#).unwrap();
        assert_eq!(old.auto_mode.as_deref(), Some("zcode/glm-5.3"), "旧快照照常读回");
        assert!(serde_json::to_string(&old).unwrap().contains("\"autoMode\""), "给前端的名字与其类型一致");
    }

    #[test]
    fn tier_efforts_follow_what_claude_code_sends_each_model() {
        assert_eq!(tier_efforts("claude/claude-opus-4-5"), Some(&["low", "medium", "high"][..]));
        assert_eq!(tier_efforts("anthropic/claude-sonnet-4.6"), Some(&["low", "medium", "high", "max"][..]), "点号写法同样识别");
        assert_eq!(tier_efforts("claude-haiku-4-5"), Some(&[][..]), "Haiku 不收 effort");
        assert_eq!(tier_efforts("claude-3-5-sonnet-latest"), Some(&[][..]));
        assert_eq!(tier_efforts("claude-opus-5-5[1m]"), Some(CLAUDE_EFFORTS), "新模型全套");
        assert_eq!(tier_efforts("zcode/glm-5.3-flash"), Some(CLAUDE_EFFORTS), "别家模型按 Claude Code 的全套");
    }

    #[test]
    fn validate_rejects_efforts_a_model_does_not_take() {
        let ok = TierModels {
            opus: Some("claude/claude-opus-4-6:max".into()),
            sonnet: Some("zcode/glm-5.3-flash:low".into()),
            haiku: None,
            fable: Some("claude/claude-fable-5-1[1m]".into()),
            auto_mode: Some("zcode/glm-5.3-flash".into()),
        };
        assert!(ok.validate().is_ok());
        let xhigh_on_46 = TierModels { opus: Some("claude/claude-opus-4-6:xhigh".into()), ..ok.clone() };
        let error = xhigh_on_46.validate().unwrap_err();
        assert!(error.contains("opus") && error.contains("xhigh") && error.contains("low / medium / high / max"), "{error}");
        let haiku_effort = TierModels { haiku: Some("claude/claude-haiku-4-5:low".into()), ..ok.clone() };
        assert!(haiku_effort.validate().unwrap_err().contains("不支持 effort"));
        let none_on_claude = TierModels { sonnet: Some("claude/claude-sonnet-5:minimal".into()), ..ok.clone() };
        assert!(none_on_claude.validate().unwrap_err().contains("minimal"), "minimal 不是 Claude Code 的档位");
        // models.dev 有档位数据时以它为准
        let glm = TierModels { sonnet: Some("zcode/glm-5.3:max".into()), ..ok.clone() };
        let only_high = |model: &str| model.ends_with("glm-5.3").then(|| vec!["low".to_string(), "high".to_string()]);
        assert!(glm.validate_with(&only_high).unwrap_err().contains("low / high"));
        assert!(glm.validate().is_ok(), "没有 models.dev 数据时别家模型按全集");
        // 冒号后不是 effort 词的（Ollama 标签、参数量）属于模型名，不当 effort 看
        for model in ["local/qwen3:32b", "ollama/llama3:latest"] {
            let tag = TierModels { sonnet: Some(model.into()), ..ok.clone() };
            assert!(tag.validate().is_ok(), "{model}");
        }
    }

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn pending_then_wired_then_idle_round_trip() {
        let conn = db();
        let applied = Applied { base: "http://127.0.0.1:12731".into(), token: Some("jolu-codex.abcd1234".into()), provider: None, mode: Some("provider".into()), model: None, label: None, subagent_model: None, tiers: None };
        mark_pending(&conn, "codex", "instance-1", "relay", Some(r#"{"model_provider":"openai"}"#), &applied, Some("jolu-codex.abcd1234")).unwrap();
        let record = load(&conn, "codex").unwrap();
        assert_eq!(record.state, "pending");
        assert_eq!(record.applied().unwrap().base, "http://127.0.0.1:12731");
        assert_eq!(record.original_json.as_deref(), Some(r#"{"model_provider":"openai"}"#));

        mark_wired(&conn, "codex", 42).unwrap();
        assert_eq!(load(&conn, "codex").unwrap().state, "wired");

        mark_idle(&conn, "codex").unwrap();
        let record = load(&conn, "codex").unwrap();
        assert_eq!(record.state, "idle");
        assert!(record.applied_json.is_none());
        assert!(record.original_json.is_none());
    }

    #[test]
    fn legacy_snapshots_import_once() {
        let conn = db();
        let mut map = std::collections::BTreeMap::new();
        map.insert("claude-code".to_string(), r#"{"ANTHROPIC_BASE_URL":"https://old"}"#.to_string());
        assert_eq!(import_legacy(&conn, &map, "inst"), 1);
        assert_eq!(import_legacy(&conn, &map, "inst"), 0, "重复导入不能覆盖已有记录");
        let record = load(&conn, "claude-code").unwrap();
        assert_eq!(record.state, "wired");
        assert_eq!(record.owner, "relay");
    }
}
