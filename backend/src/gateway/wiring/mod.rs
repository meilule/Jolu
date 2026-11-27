#![allow(unused_imports)]
//! 接线层：把各 agent 的配置指向本机网关。

use std::path::{Path, PathBuf};

pub(crate) mod adapters;
pub(crate) mod antigravity;
pub(crate) mod claude;
pub(crate) mod codex;
pub mod diff;
pub(crate) mod dsh;
pub(crate) mod extra;
pub(crate) mod files;
pub(crate) mod gemini_opencode;
pub(crate) mod kimi;
pub(crate) mod pi;
pub(crate) mod qwen;
pub(crate) mod specs;
pub mod store;
#[cfg(test)]
mod tests;
pub(crate) mod workbuddy;
pub(crate) mod wsl;
pub(crate) mod zcode;

use store::{Applied, TierModels, WiringRecord};

pub(crate) use claude::{
    claude_capabilities, claude_current_base, claude_current_token, claude_extra_env,
    claude_read_model, ClaudeExtras, CLAUDE_AUTO_MODE_SERVER, CLAUDE_COMPACT_WINDOW,
    CLAUDE_MAX_CONTEXT, CLAUDE_MAX_OUTPUT, CLAUDE_MODEL_CAPABILITIES,
};
pub(crate) use claude::{
    claude_conflicts, claude_env, claude_env_keys, claude_restore, claude_snapshot,
    claude_snapshot_is_structured, claude_tier_env, claude_wire, drop_our_extras, fit_effort,
    is_claude_family, is_fable_family, written_by_us, CLAUDE_ABOUT, CLAUDE_CLOUD_SWITCHES,
    CLAUDE_EXTRA_KEYS, CLAUDE_TIERS,
};
pub(crate) use codex::{
    codex_effective_base, codex_oauth_logged_in, codex_provider_base,
    codex_restore, codex_snapshot, codex_wire, CODEX_PROVIDER,
};
pub use files::{config_path_for, write_json_atomic_public};
pub(crate) use files::{
    backup, config_path, dotenv_get, foreign_marker, ours_with, parse_toml_document,
    points_at_gateway, prune_empty_table, token_for, verify_written, wire_dotenv,
    write_all_with_rollback, write_atomic,
};
#[cfg(test)]
pub(crate) use files::ours;
pub(crate) use gemini_opencode::{
    gemini_current_base, gemini_current_token, gemini_plan, gemini_restore, gemini_selected_type,
    gemini_settings_path, gemini_settings_text, opencode_current_base, opencode_current_token,
    opencode_restore, opencode_wire,
};
pub(crate) use kimi::{
    kimi_current_base, kimi_current_token, kimi_default_model_ours, kimi_default_ours, kimi_dir,
    kimi_dir_public, kimi_provider_read, kimi_restore, kimi_snapshot, kimi_wire,
    KIMI_DEFAULT_CONTEXT, KIMI_PROVIDER,
};
pub use zcode::ZcodeModel;
pub(crate) use zcode::{
    array_at, is_ours, object_path, parse_object, prune_empty_json_at, prune_empty_json_object,
    zcode_clean_legacy, zcode_config_restore, zcode_config_wire, zcode_current_base,
    zcode_current_token, zcode_dir, zcode_dir_public, zcode_rule, zcode_rules_restore,
    zcode_rules_snapshot, zcode_rules_wire, ZCODE_DEFAULT_CONTEXT, ZCODE_PROVIDER,
};

pub fn agents() -> &'static [&'static str] {
    specs::ids()
}

pub fn is_agent(agent: &str) -> bool {
    specs::spec(agent).is_some()
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
        || (specs::spec(agent)
            .is_some_and(|spec| spec.extra == specs::WiredExtra::GeminiApiKey)
            && gemini_settings_path().is_some_and(|p| p.exists()))
        || (specs::spec(agent)
            .is_some_and(|spec| spec.extra == specs::WiredExtra::AntigravityModel)
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
pub(crate) fn model_key(model: &str) -> String {
    let (bare, _) = crate::gateway::wiring_store::split_tier_value(model.trim());
    bare.trim().trim_end_matches("[1m]").trim_end_matches("[1M]").to_ascii_lowercase()
}

/// 回读 Claude Code 现在用的主模型（magpie `claudeMain` 的优先级链）：
/// 顶层 `model`（`/model` 写这里）→ `env.ANTHROPIC_MODEL` → `env.ANTHROPIC_DEFAULT_SONNET_MODEL`（默认档）。
/// 返回（原值，别名解析结果）：原值是档位别名时按对应档位 env 解析成真实模型（B8b），
/// `best` 按 opus、`opusplan` 按 opus 解析；档位 env 没写就没有解析结果。

pub(crate) fn restart_notice(agent: &str) -> &'static str {
    specs::spec(agent)
        .map(|spec| spec.notice)
        .unwrap_or("需重启后生效")
}

/// 重新应用时优先沿用旧快照；只有第一次接线才使用当前文件算出的值。
pub(crate) fn effective_snapshot(prior: Option<&str>, computed: Option<serde_json::Value>) -> Option<String> {
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
    let spec = specs::spec(ctx.agent).ok_or("该工具不在接线范围")?;
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


pub(crate) fn unwire(agent: &str, port: u16, record: Option<&WiringRecord>) -> Result<bool, String> {
    let Some(files) = plan_unwire(agent, port, record)? else { return Ok(false) };
    write_all_with_rollback(&files)?;
    if let (Some(cleanup), Some(home)) = (specs::spec(agent).and_then(|spec| spec.after_unwire), crate::creds::home()) {
        cleanup(&home);
    }
    Ok(true)
}

/// 断开要写回的文件（只算不写）。None = 不是我们的配置，什么都不做。
/// 已接入时的「写入的改动」预览用它：还原后的内容 → 现在的内容 = Jolu 改了什么。
pub(crate) fn plan_unwire(agent: &str, port: u16, record: Option<&WiringRecord>) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    let spec = specs::spec(agent).ok_or("该工具不在接线范围")?;
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
