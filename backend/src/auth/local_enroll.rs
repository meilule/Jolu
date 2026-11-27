//! 灵动岛未收录本机凭证「原地一键收录」入库编排。
//!
//! 安全边界（AGENTS.md §4）：
//! - 前端点击「一键收录」时只传 `platform` 与可选 `source_agent`，**绝不接收前端传入的明文密钥**；
//! - 后端从本机配置文件即时重读当前生效凭证，走正规 `connections::add`（含 slug、指纹、脱敏掩码与快照保全）；
//! - 若配置带上游网关地址，顺带尝试一次模型列表拉取写入 `models_json`（失败不回滚入库，仅附带温和提示）。

use rusqlite::Connection;
use serde::Serialize;

use crate::connections::{self, NewConnection};
use crate::current_connection::LocalConnection;

#[derive(Debug, Clone, Serialize)]
pub struct EnrolledLocalAccountDto {
    pub connection_id: String,
    pub platform: String,
    pub kind: String,
    pub name: String,
    pub models_fetched: usize,
    pub warning: Option<String>,
}

struct ApiSavePlan {
    platform: String,
    service: Option<String>,
    base_url: Option<String>,
    protocol: Option<String>,
    name: String,
    slug_base: String,
}

fn host_label(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

fn default_agent_label(local: &LocalConnection) -> String {
    if let Some(agent) = local.source_agent.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return agent.to_string();
    }
    crate::platforms::get(&local.platform)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| local.platform.clone())
}

fn build_api_save_plan(local: &LocalConnection) -> Result<ApiSavePlan, String> {
    let raw_base = local
        .base_url
        .as_deref()
        .map(|b| b.trim().trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty());

    if let Some(base) = raw_base.as_deref() {
        let parsed = reqwest::Url::parse(base).map_err(|_| "本机配置中的网关地址格式无效，请前往供应商页检查录入".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("本机配置中的网关地址必须使用 HTTP 或 HTTPS".into());
        }
    }

    let agent_label = default_agent_label(local);
    let zcode_official = [
        "https://open.bigmodel.cn/api/anthropic",
        "https://open.bigmodel.cn/api/coding/paas/v4",
        "https://api.z.ai/api/anthropic",
        "https://api.z.ai/api/coding/paas/v4",
    ];

    if matches!(local.platform.as_str(), "claude" | "codex") {
        let normalized = connections::normalized_base(&local.platform, "api", raw_base);
        let name = match normalized.as_deref().and_then(host_label) {
            Some(host) => format!("{agent_label} · {host}"),
            None => format!("{agent_label} · 本机 Key"),
        };
        return Ok(ApiSavePlan {
            platform: local.platform.clone(),
            service: None,
            base_url: normalized,
            protocol: None,
            slug_base: agent_label,
            name,
        });
    }

    if crate::provider_key::supported(&local.platform)
        && crate::provider_key::check_base(&local.platform, raw_base.as_deref()).is_ok()
    {
        return Ok(ApiSavePlan {
            platform: local.platform.clone(),
            service: None,
            base_url: None,
            protocol: None,
            name: format!("{agent_label} · 官方 Key"),
            slug_base: agent_label,
        });
    }

    if local.platform == "zcode" {
        if let Some(base) = raw_base.as_deref().filter(|b| zcode_official.contains(b)) {
            let proto = if base.ends_with("/anthropic") { "anthropic" } else { "openai" };
            let host = host_label(base).unwrap_or_else(|| "官方".into());
            return Ok(ApiSavePlan {
                platform: "zcode".into(),
                service: None,
                base_url: Some(base.to_string()),
                protocol: Some(proto.into()),
                name: format!("ZCode · {host}"),
                slug_base: "zcode-api".into(),
            });
        }
    }

    if local.platform == "opencode" && raw_base.as_deref() == Some("https://opencode.ai/zen/go/v1") {
        return Ok(ApiSavePlan {
            platform: "opencode".into(),
            service: Some("opencode-go".into()),
            base_url: raw_base,
            protocol: Some("openai".into()),
            name: "OpenCode · Go 官方".into(),
            slug_base: "opencode-go".into(),
        });
    }

    let Some(base) = raw_base else {
        return Err("该本机凭证未包含接口地址（Base URL），请前往供应商页手动补全录入".into());
    };
    let proto = match local.protocol.as_deref() {
        Some("anthropic") => "anthropic",
        Some("openai") => "openai",
        _ if local.secret.trim().starts_with("sk-ant-") => "anthropic",
        _ => "openai",
    };
    let host = host_label(&base).unwrap_or_else(|| "本机直连".into());
    Ok(ApiSavePlan {
        platform: "custom".into(),
        service: None,
        base_url: Some(base),
        protocol: Some(proto.into()),
        name: format!("{agent_label} · {host}"),
        slug_base: host_label_slug(&host, &agent_label),
    })
}

fn host_label_slug(host: &str, fallback: &str) -> String {
    let head = host.split(':').next().unwrap_or(host).split('.').next().unwrap_or("");
    if head.len() >= 2 && !matches!(head, "api" | "open" | "www" | "gateway" | "llm") {
        head.to_string()
    } else {
        fallback.to_string()
    }
}

fn enroll_native_local_oauth(
    conn: &mut Connection,
    local: &LocalConnection,
    now_ms: i64,
) -> Result<EnrolledLocalAccountDto, String> {
    let (id, default_name, label, service) = match local.platform.as_str() {
        "zcode" => ("monitor:native:zcode:account", "ZCode 账号", "本机登录", None),
        "gemini" => ("monitor:native:gemini:cli", "Gemini CLI", "Google 登录", None),
        "opencode" => ("monitor:native:opencode:openai", "OpenCode · OpenAI", "OpenAI 订阅", Some("openai")),
        "grok" => ("monitor:native:grok:cli", "Grok CLI", "本机登录", None),
        other => return Err(format!("{other} 的本地登录请前往供应商页添加")),
    };
    let display = local
        .detected_email
        .as_deref()
        .or(local.masked_override.as_deref())
        .unwrap_or(default_name);
    let saved_name = connections::validate_name(&display.chars().take(80).collect::<String>())?;
    let slug = crate::gateway::identity::allocate_slug(conn, id, &local.platform, &saved_name, None)
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO connections(id, platform, kind, name, label, masked, status, created_ms, account_email, service, slug)
         VALUES(?1, ?2, 'auth', ?3, ?4, '', 'connected', ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           status = 'connected',
           account_email = COALESCE(excluded.account_email, connections.account_email)",
        rusqlite::params![id, local.platform, saved_name, label, now_ms, local.detected_email, service, slug],
    )
    .map_err(|e| e.to_string())?;
    let _ = crate::db::stamp_active_config_attribution(conn, &local.platform, now_ms - 90_000, id);
    let _ = crate::local_active::refresh_and_diff(conn);
    Ok(EnrolledLocalAccountDto {
        connection_id: id.into(),
        platform: local.platform.clone(),
        kind: "auth".into(),
        name: saved_name,
        models_fetched: 0,
        warning: None,
    })
}

/// 在已重读出的本机凭证集合中挑选目标条目并完成入库（纯同步函数，供阻塞线程池与单测调用）。
pub(crate) fn enroll_from_locals(
    conn: &mut Connection,
    locals: Vec<LocalConnection>,
    target_platform: &str,
    source_agent: Option<&str>,
    claude_prepared: Option<crate::claude_auth::PreparedClaude>,
    probe_models: bool,
) -> Result<EnrolledLocalAccountDto, String> {
    let wanted_agent = source_agent.map(str::trim).filter(|s| !s.is_empty());
    let local = locals
        .into_iter()
        .filter(|item| {
            !crate::gateway::wiring::is_gateway_value(&item.secret)
                && !item
                    .base_url
                    .as_deref()
                    .is_some_and(crate::gateway::wiring::is_gateway_value)
        })
        .find(|item| match wanted_agent {
            Some(agent) => item.source_agent.as_deref() == Some(agent) || item.platform == target_platform,
            None => item.platform == target_platform,
        })
        .ok_or_else(|| "未在本机配置中检测到待收录的有效凭证，可能已被切换或移除".to_string())?;

    // 幂等防连点：若库里已经存在匹配的连接，直接返回既有记录
    if let Some((existing_id, existing_platform, existing_name)) =
        crate::local_active::find_matching_saved_connection(conn, &local)
    {
        let _ = crate::local_active::refresh_and_diff(conn);
        return Ok(EnrolledLocalAccountDto {
            connection_id: existing_id,
            platform: existing_platform,
            kind: local.kind,
            name: existing_name,
            models_fetched: 0,
            warning: None,
        });
    }

    let now_ms = chrono::Local::now().timestamp_millis();

    if local.kind == "auth" {
        if local.secret.starts_with(crate::agent_configs::LOCAL_OAUTH_PREFIX) {
            return enroll_native_local_oauth(conn, &local, now_ms);
        }
        if !matches!(local.platform.as_str(), "claude" | "codex") {
            return Err("该平台的订阅登录请前往供应商页通过授权或本机来源获取".into());
        }
        let default_name = if local.platform == "claude" {
            "Claude 官方订阅"
        } else {
            "Codex 官方订阅"
        };
        let inputs = connections::with_custom_name(
            vec![NewConnection {
                service: None,
                platform: local.platform.clone(),
                kind: "auth".into(),
                name: default_name.into(),
                secret: Some(local.secret.clone()),
                base_url: None,
                model: None,
                effort: None,
                context_1m: None,
                protocol: None,
                slug_base: None,
            }],
            None,
        )?;
        let input = inputs.into_iter().next().ok_or("无法构建订阅连接")?;
        let saved_name = input.name.clone();
        let saved_platform = input.platform.clone();

        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let id = connections::add(&tx, input)?;
        if let Some(prepared) = &claude_prepared {
            prepared.sync(&tx, &id)?;
        }
        let _ = crate::db::stamp_active_config_attribution(&tx, &local.platform, now_ms - 90_000, &id);
        tx.commit().map_err(|e| e.to_string())?;
        let _ = crate::local_active::refresh_and_diff(conn);

        return Ok(EnrolledLocalAccountDto {
            connection_id: id,
            platform: saved_platform,
            kind: "auth".into(),
            name: saved_name,
            models_fetched: 0,
            warning: None,
        });
    }

    // API Key 分支：构建保存方案并按需探测协议与模型列表
    let mut plan = build_api_save_plan(&local)?;
    let mut fetched_models: Vec<String> = Vec::new();
    let mut warning: Option<String> = None;

    if probe_models {
        if let Some(base) = plan.base_url.as_deref() {
            let primary_proto = plan.protocol.clone();
            match connections::fetch_models(&plan.platform, primary_proto.as_deref(), base, &local.secret) {
                Ok(list) if !list.is_empty() => fetched_models = list,
                Err(first_err) => {
                    // 自定义供应商若初次协议猜测不通，自动尝试另一协议（如 OpenAI <-> Anthropic 兼容网关）
                    if plan.platform == "custom" {
                        let alt = if plan.protocol.as_deref() == Some("anthropic") {
                            "openai"
                        } else {
                            "anthropic"
                        };
                        if let Ok(list) = connections::fetch_models("custom", Some(alt), base, &local.secret) {
                            if !list.is_empty() {
                                plan.protocol = Some(alt.into());
                                fetched_models = list;
                            }
                        }
                    }
                    if fetched_models.is_empty() {
                        warning = Some(format!(
                            "已收录连接；自动拉取模型清单未成功（{first_err}），如需接入模型路由可在供应商页补充"
                        ));
                    }
                }
                Ok(_) => {}
            }
        }
    }

    let saved_name = connections::validate_name(&plan.name)?;
    let saved_platform = plan.platform.clone();
    let input = NewConnection {
        service: plan.service,
        platform: plan.platform,
        kind: "api".into(),
        name: saved_name.clone(),
        secret: Some(local.secret.clone()),
        base_url: plan.base_url,
        model: fetched_models.first().cloned(),
        effort: None,
        context_1m: None,
        protocol: plan.protocol,
        slug_base: Some(plan.slug_base),
    };

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let id = connections::add(&tx, input)?;
    if !fetched_models.is_empty() {
        if let Ok(json) = serde_json::to_string(&fetched_models) {
            let _ = tx.execute(
                "UPDATE connections SET models_json = ?2 WHERE id = ?1",
                rusqlite::params![id, json],
            );
        }
    }
    let _ = crate::db::stamp_active_config_attribution(&tx, &local.platform, now_ms - 90_000, &id);
    tx.commit().map_err(|e| e.to_string())?;
    let _ = crate::local_active::refresh_and_diff(conn);

    Ok(EnrolledLocalAccountDto {
        connection_id: id,
        platform: saved_platform,
        kind: "api".into(),
        name: saved_name,
        models_fetched: fetched_models.len(),
        warning,
    })
}

/// 生产环境入口：重读本机全量生效配置并执行一键收录。
pub(crate) fn enroll_current_local(
    pool: &std::sync::Mutex<Connection>,
    target_platform: &str,
    source_agent: Option<&str>,
) -> Result<EnrolledLocalAccountDto, String> {
    if crate::validation::isolated() {
        return Err("隔离验收环境禁止读取真实凭证".into());
    }
    let customs = {
        let conn = pool.lock().map_err(|_| "数据库不可用".to_string())?;
        crate::custom_agents::list(&conn).unwrap_or_default()
    };
    let locals = crate::current_connection::read_all_local(&customs);

    let needs_claude_auth = locals.iter().any(|item| {
        item.platform == "claude"
            && item.kind == "auth"
            && (target_platform == "claude" || source_agent == item.source_agent.as_deref())
    });
    let claude_prepared = if needs_claude_auth {
        let prepared = crate::claude_auth::prepare_current()?;
        if let Some(c) = locals.iter().find(|i| i.platform == "claude" && i.kind == "auth") {
            prepared.require(Some(&c.secret))?;
        }
        Some(prepared)
    } else {
        None
    };

    let mut conn = pool.lock().map_err(|_| "数据库不可用".to_string())?;
    enroll_from_locals(&mut conn, locals, target_platform, source_agent, claude_prepared, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mem_db() -> Connection {
        crate::db::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn enrolls_third_party_cli_key_as_custom_and_is_idempotent() {
        let mut conn = mem_db();
        let locals = vec![LocalConnection {
            platform: "opencode".into(),
            kind: "api".into(),
            secret: "sk-or-v1-enroll-test-8899".into(),
            base_url: Some("https://openrouter.ai/api/v1".into()),
            protocol: Some("openai".into()),
            source_agent: Some("OpenCode".into()),
            masked_override: None,
            detected_email: None,
        }];

        let first = enroll_from_locals(&mut conn, locals.clone(), "opencode", Some("OpenCode"), None, false)
            .expect("first enroll");
        assert_eq!(first.platform, "custom");
        assert_eq!(first.kind, "api");
        assert_eq!(first.name, "OpenCode · openrouter.ai");

        // 再次点击幂等返回同一条连接，不重复建卡
        let second = enroll_from_locals(&mut conn, locals.clone(), "opencode", Some("OpenCode"), None, false)
            .expect("idempotent second enroll");
        assert_eq!(second.connection_id, first.connection_id);

        // 且跨平台匹配立刻识别 opencode 已命中刚收录的 custom 连接
        let resolved = crate::local_active::resolve_with_lookup(&conn, locals, 1_000, |_, _| None);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].state, "matched");
        assert_eq!(resolved[0].connection_id.as_deref(), Some(first.connection_id.as_str()));
    }

    #[test]
    fn enrolls_claude_relay_on_claude_platform_and_rejects_gateway_token() {
        let mut conn = mem_db();
        let gateway_only = vec![LocalConnection {
            platform: "claude".into(),
            kind: "api".into(),
            secret: "jolu-claude-code.12345678".into(),
            base_url: Some("http://127.0.0.1:12731".into()),
            protocol: Some("anthropic".into()),
            source_agent: Some("Claude Code".into()),
            masked_override: None,
            detected_email: None,
        }];
        assert!(enroll_from_locals(&mut conn, gateway_only, "claude", None, None, false).is_err());

        let claude_relay = vec![LocalConnection {
            platform: "claude".into(),
            kind: "api".into(),
            secret: "sk-ant-relay-test-7766".into(),
            base_url: Some("https://sub2api.example.com/".into()),
            protocol: Some("anthropic".into()),
            source_agent: Some("Claude Code".into()),
            masked_override: None,
            detected_email: None,
        }];
        let enrolled = enroll_from_locals(&mut conn, claude_relay, "claude", None, None, false).unwrap();
        assert_eq!(enrolled.platform, "claude");
        assert_eq!(enrolled.name, "Claude Code · sub2api.example.com");
    }
}
