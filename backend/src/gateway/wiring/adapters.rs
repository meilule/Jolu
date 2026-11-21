//! 各工具的接入与断开计划。公共层负责读取主文件、备份、记录与写盘。
use super::*;

pub(crate) struct WireEdits {
    pub next: String,
    pub snapshot: Option<serde_json::Value>,
    pub extra_files: Vec<(PathBuf, Vec<u8>)>,
    pub applied: Applied,
}

pub(crate) struct UnwireInput<'a> {
    pub path: PathBuf,
    pub original: String,
    pub saved: Option<serde_json::Value>,
    pub port: u16,
    pub record: Option<&'a WiringRecord>,
}

pub(crate) fn wire_claude(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let mut token = ctx.token.clone();
    let extra_files = Vec::new();
    let model = ctx.chosen.model;
    let label = ctx.chosen.label;
    let description = ctx.chosen.description;
    let prior_model = ctx.chosen.prior_model;
    let prior_label = ctx.chosen.prior_label;
    let prior_values = ctx.chosen.prior_values;
    let subagent_model = ctx.chosen.subagent_model;
    let tiers = ctx.chosen.tiers;
    let stubbed_extras = ClaudeExtras::default();
    let stubbed_holds = |_: &str| false;
    let stubbed_about = |_: &str| (None, None);
    let (extras, holds, tier_about): (
        &ClaudeExtras,
        &dyn Fn(&str) -> bool,
        &dyn Fn(&str) -> (Option<String>, Option<String>),
    ) = match &ctx.claude {
        Some(input) => (input.extras, input.holds, input.tier_about),
        None => (&stubbed_extras, &stubbed_holds, &stubbed_about),
    };
    if let Some(existing) =
        claude_current_token(&original).filter(|t| !t.starts_with("jolu-") && holds(t))
    {
        token = existing;
    }
    let (next, snapshot) = {
        let snapshot = claude_snapshot(&original, prior_model, prior_label, prior_values)
            .and_then(|saved| drop_our_extras(saved, &extras.previous));
        let text = if original.trim().is_empty() {
            "{}"
        } else {
            original.as_str()
        };
        (
            claude_wire(
                text,
                &base,
                &token,
                model,
                label,
                description,
                subagent_model,
                tiers,
                tier_about,
                extras,
            )?,
            snapshot,
        )
    };
    let mut applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    applied.model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    applied.label = label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    applied.subagent_model = subagent_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    applied.tiers = tiers.cloned();
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        applied.extra_env = claude_extra_env(model, extras);
    }
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_codex(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let extra_files = Vec::new();
    let codex_oauth = codex_oauth_logged_in();
    let (next, snapshot) = {
        let snapshot = codex_snapshot(&original);
        let token = (!codex_oauth).then_some(token.as_str());
        (
            codex_wire(&original, &base, CODEX_PROVIDER, codex_oauth, token)?,
            snapshot,
        )
    };
    let mut applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    if codex_oauth {
        applied.base = format!("{base}/backend-api/codex");
        applied.token = None;
    } else {
        applied.provider = Some(CODEX_PROVIDER.to_string());
    }
    applied.mode = Some(if codex_oauth { "oauth" } else { "provider" }.to_string());
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_gemini(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let mut extra_files = Vec::new();
    let (next, snapshot) = {
        let settings_path = gemini_settings_path().ok_or("未找到用户目录")?;
        let settings_text = gemini_settings_text().unwrap_or_default();
        let plan = gemini_plan(&original, &settings_text, &base, &token)?;
        extra_files.push((settings_path, plan.settings.into_bytes()));
        (plan.env, Some(plan.snapshot))
    };
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_opencode(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let extra_files = Vec::new();
    let (next, snapshot) = (opencode_wire(&original, &base, &token)?, None);
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_zcode(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let mut extra_files = Vec::new();
    let catalog_models = ctx.catalog;
    let (next, snapshot) = {
        // 两份都写：新版 ZCode 读规则文件，旧版读 config.json（magpie 同款）
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let config_path = zcode_dir(&home).join("v2").join("config.json");
        let config_text = crate::steady::read_optional(&config_path)
            .map_err(|error| format!("读取 {} 失败：{error}", config_path.display()))?
            .unwrap_or_default();
        extra_files.push((
            config_path,
            zcode_config_wire(&config_text, &base, &token, catalog_models)?.into_bytes(),
        ));
        // 记录接线前哪些结构存在，断开时才知道哪些空壳是本次新建的。
        let mut snapshot = zcode_rules_snapshot(if original.trim().is_empty() {
            "{}"
        } else {
            &original
        });
        if let Some(value) = snapshot.as_mut() {
            let config: serde_json::Value = serde_json::from_str(&config_text).unwrap_or_default();
            value["configProvider"] = serde_json::json!(config.get("provider").is_some());
        }
        (
            zcode_rules_wire(&original, &base, &token, catalog_models)?,
            snapshot,
        )
    };
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_kimi(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let extra_files = Vec::new();
    let model = ctx.chosen.model;
    let catalog_models = ctx.catalog;
    let (next, snapshot) = {
        // 旧版 kimi-cli（~/.kimi）不认 tool_use 等新键；写哪个目录按 kimi_dir 的判定
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let legacy = kimi_dir(&home).1;
        (
            kimi_wire(&original, &base, &token, model, catalog_models, legacy)?,
            kimi_snapshot(&original),
        )
    };
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_qwen(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let model = ctx.chosen.model;
    let catalog_models = ctx.catalog;
    let (next, snapshot) = (
        crate::gateway::wiring_qwen::wire(&original, &base, &token, model, catalog_models)?,
        crate::gateway::wiring_qwen::snapshot(&original),
    );
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files: Vec::new(),
        applied,
    })
}

pub(crate) fn wire_workbuddy(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let catalog_models = ctx.catalog;
    let (next, snapshot) = (
        crate::gateway::wiring_workbuddy::wire(&original, &base, &token, catalog_models)?,
        crate::gateway::wiring_workbuddy::snapshot(&original),
    );
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files: Vec::new(),
        applied,
    })
}

pub(crate) fn wire_pi(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let mut extra_files = Vec::new();
    let model = ctx.chosen.model;
    let catalog_models = ctx.catalog;
    let (next, snapshot) = {
        // 双文件：models.json（provider + 模型清单）+ settings.json（默认 provider/model）
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let settings_path = crate::gateway::wiring_pi::settings_path(&home);
        let settings_text = crate::steady::read_optional(&settings_path)
            .map_err(|e| format!("读取 {} 失败：{e}", settings_path.display()))?
            .unwrap_or_default();
        let (models_next, settings_next, snapshot) = crate::gateway::wiring_pi::wire(
            &original,
            &settings_text,
            &base,
            &token,
            model,
            catalog_models,
        )?;
        extra_files.push((settings_path, settings_next.into_bytes()));
        (models_next, snapshot)
    };
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_dsh(ctx: &WireContext, _original: &str) -> Result<WireEdits, String> {
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let mut extra_files = Vec::new();
    let model = ctx.chosen.model;
    let catalog_models = ctx.catalog;
    let (next, snapshot) = {
        // 多文件：每个 profile 的 patch + .env + credentials refs（wiring_dsh 组装）
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let plan = crate::gateway::wiring_dsh::plan_wire_files(
            &home,
            &base,
            &token,
            model,
            catalog_models,
        )?;
        for (path, text) in plan.extra {
            extra_files.push((path, text.into_bytes()));
        }
        (plan.main_text, plan.snapshot)
    };
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn wire_extra(ctx: &WireContext, original: &str) -> Result<WireEdits, String> {
    let original = original.to_string();
    let base: &str = &ctx.base;
    let token = ctx.token.clone();
    let extra_files = Vec::new();
    let model = ctx.chosen.model;
    let catalog_models = ctx.catalog;
    let other = ctx.agent;
    let (next, snapshot) = (
        crate::gateway::wiring_extra::wire(other, &original, &base, &token, model, catalog_models)?,
        crate::gateway::wiring_extra::snapshot(other, &original),
    );
    let applied = Applied {
        base: ctx.base.clone(),
        token: Some(token),
        ..Default::default()
    };
    Ok(WireEdits {
        next,
        snapshot,
        extra_files,
        applied,
    })
}

pub(crate) fn unwire_claude(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    let record = input.record;
    let port = input.port;
    let next = {
        // 外部意图优先：当前 base 已不是网关地址、且 token 不是 Jolu 写的，就不动
        let base = claude_current_base(&original).unwrap_or_default();
        let token = claude_current_token(&original).unwrap_or_default();
        if !points_at_gateway(&base, port)
            && !is_gateway_value(&base)
            && !token.starts_with("jolu-")
        {
            return Ok(None);
        }
        // 上次接线写下的主模型与显示名：没有快照时靠它们识别「哪些键是我们写的」
        let applied = record.and_then(|r| r.applied());
        let prior_model = applied.as_ref().and_then(|a| a.model.clone());
        let prior_label = applied.as_ref().and_then(|a| a.label.clone());
        let prior_values = applied
            .as_ref()
            .map(Applied::written_values)
            .unwrap_or_default();
        claude_restore(
            &original,
            saved.as_ref(),
            prior_model.as_deref(),
            prior_label.as_deref(),
            &prior_values,
        )?
    };
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_codex(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    let next = {
        let points_at_us = codex_effective_base(&original)
            .is_some_and(|base| is_gateway_value(&base))
            || codex_provider_base(&original, CODEX_PROVIDER)
                .is_some_and(|base| is_gateway_value(&base));
        if !points_at_us {
            return Ok(None);
        }
        codex_restore(&original, saved.as_ref(), CODEX_PROVIDER)?
    };
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_opencode(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    let next = {
        let base_ours =
            opencode_current_base(&original).is_some_and(|base| is_gateway_value(&base));
        let token_ours =
            opencode_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
        if !base_ours && !token_ours {
            return Ok(None);
        }
        opencode_restore(&original, saved.as_ref())?
    };
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_kimi(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    let next = {
        let base_ours = kimi_current_base(&original).is_some_and(|base| is_gateway_value(&base));
        let token_ours =
            kimi_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
        if !base_ours && !token_ours {
            return Ok(None);
        }
        kimi_restore(&original, saved.as_ref())?
    };
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_pi(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let path = &input.path;
    let original = &input.original;
    let saved = &input.saved;
    {
        // 双文件还原：models.json 删 provider；settings.json 还原默认（用户改过就不动）
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let settings_path = crate::gateway::wiring_pi::settings_path(&home);
        let settings_text = crate::steady::read_optional(&settings_path)
            .map_err(|e| format!("读取 {} 失败：{e}", settings_path.display()))?
            .unwrap_or_default();
        let base_ours = crate::gateway::wiring_pi::current_base(&original)
            .is_some_and(|base| is_gateway_value(&base));
        let token_ours = crate::gateway::wiring_pi::current_token(&original)
            .is_some_and(|token| token.starts_with("jolu-"));
        if !base_ours && !token_ours {
            return Ok(None);
        }
        let (models_next, settings_next) =
            crate::gateway::wiring_pi::restore(&original, &settings_text, saved.as_ref())?;
        let mut files = vec![(path.clone(), models_next.into_bytes())];
        if settings_next != settings_text {
            files.push((settings_path, settings_next.into_bytes()));
        }
        return Ok(Some(files));
    }
}

pub(crate) fn unwire_qwen(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    let base_ours = crate::gateway::wiring_qwen::current_base(original).is_some_and(|base| is_gateway_value(&base));
    let token_ours = crate::gateway::wiring_qwen::current_token(original).is_some_and(|token| token.starts_with("jolu-"));
    if !base_ours && !token_ours {
        return Ok(None);
    }
    let next = crate::gateway::wiring_qwen::restore(original, saved.as_ref())?;
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_workbuddy(
    agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    // 令牌按实例算：`token_for(agent, instance)` 才是我们写进去的那把
    let instance: String = input
        .record
        .map(|record| record.instance_id.chars().take(8).collect())
        .unwrap_or_default();
    let token = crate::gateway::wiring::token_for(agent, &instance);
    let base_ours = crate::gateway::wiring_workbuddy::current_base(original).is_some_and(|base| is_gateway_value(&base));
    let token_ours = crate::gateway::wiring_workbuddy::current_token(original).is_some_and(|found| found.starts_with("jolu-"));
    if !base_ours && !token_ours {
        return Ok(None);
    }
    let next = crate::gateway::wiring_workbuddy::restore(original, &token, saved.as_ref())?;
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_dsh(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let path = &input.path;
    let original = &input.original;
    let saved = &input.saved;
    {
        // 多文件还原：每个 profile 的 patch + .env + credentials（快照按文件分组）。
        // 主文件（第一个 profile）放列表末尾，与 plan_wire 的主文件保持同路径。
        let home = crate::creds::home().ok_or("未找到用户目录")?;
        let base_ours = crate::gateway::wiring_dsh::current_base(&original)
            .is_some_and(|base| is_gateway_value(&base));
        let token_ours = crate::creds::home()
            .and_then(|home| crate::gateway::wiring_dsh::current_token(&home))
            .is_some_and(|token| token.starts_with("jolu-"));
        if !base_ours && !token_ours {
            return Ok(None);
        }
        let mut files = crate::gateway::wiring_dsh::plan_unwire_files(&home, saved.as_ref())?;
        if let Some(at) = files.iter().position(|(p, _)| p == path) {
            let (main_path, main_text) = files.remove(at);
            files.push((main_path, main_text));
        }
        return Ok(Some(
            files
                .into_iter()
                .map(|(p, t)| (p, t.into_bytes()))
                .collect(),
        ));
    }
}

pub(crate) fn unwire_extra(
    agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let original = &input.original;
    let saved = &input.saved;
    let other = agent;
    let next = {
        let base_ours = crate::gateway::wiring_extra::current_base(other, &original)
            .is_some_and(|base| is_gateway_value(&base));
        let token_ours = crate::gateway::wiring_extra::current_token(other, &original)
            .is_some_and(|token| token.starts_with("jolu-"));
        if !base_ours && !token_ours {
            return Ok(None);
        }
        crate::gateway::wiring_extra::restore(other, &original, saved.as_ref())?
    };
    Ok(Some(vec![(input.path.clone(), next.into_bytes())]))
}

pub(crate) fn unwire_zcode(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let path = &input.path;
    let original = &input.original;
    let saved = &input.saved;
    let base_ours = zcode_current_base(&original).is_some_and(|base| is_gateway_value(&base));
    let token_ours = zcode_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
    if !base_ours && !token_ours {
        return Ok(None);
    }
    let home = crate::creds::home().ok_or("未找到用户目录")?;
    let config_path = zcode_dir(&home).join("v2").join("config.json");
    let mut files = vec![(
        path.clone(),
        zcode_rules_restore(&original, saved.as_ref())?.into_bytes(),
    )];
    if let Ok(Some(config_text)) = crate::steady::read_optional(&config_path) {
        files.push((
            config_path,
            zcode_config_restore(&config_text, saved.as_ref())?.into_bytes(),
        ));
    }
    return Ok(Some(files));
}

pub(crate) fn unwire_gemini(
    _agent: &str,
    input: &UnwireInput<'_>,
) -> Result<Option<Vec<(PathBuf, Vec<u8>)>>, String> {
    let path = &input.path;
    let original = &input.original;
    let saved = &input.saved;
    let record = input.record;
    let base_ours = gemini_current_base(&original).is_some_and(|base| is_gateway_value(&base));
    let token_ours =
        gemini_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
    let settings_ours = record.is_some()
        && gemini_settings_text()
            .as_deref()
            .and_then(gemini_selected_type)
            .as_deref()
            == Some("gemini-api-key");
    if !base_ours && !token_ours && !settings_ours {
        return Ok(None);
    }
    let settings_path = gemini_settings_path().ok_or("未找到用户目录")?;
    let settings_text = gemini_settings_text().unwrap_or_default();
    let (env, settings) = gemini_restore(&original, &settings_text, saved.as_ref());
    return Ok(Some(vec![
        (settings_path, settings.into_bytes()),
        (path.clone(), env.into_bytes()),
    ]));
}
