//! 路由选路与分组展开：模型池快照、单模型亲和、`group/<slug>` 展开与候选去重过滤。

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::response::Response;

use super::egress::record_route;
use super::ingress::PreparedRequest;
use super::{error_response, error_response_with_retry};
use crate::gateway::activity::RouteMeta;
use crate::gateway::route::{self, RouteDecision};
use crate::proxy::ProxyDeps;

pub(super) struct CandidatePlan {
    pub candidates: Vec<(route::PoolEntry, String)>,
    pub meta: RouteMeta,
}

/// 网关运行态（亲和 + 冷却）的借用引用，避免在 async 里长期持锁。
struct GatewayState<'a> {
    affinity: &'a Mutex<route::Affinity>,
    cooldowns: &'a Mutex<route::Cooldowns>,
}

/// 带来源前缀的请求被「额度用尽 / 登录失效」拒绝时的替补：找到那个来源，取它的同平台替补。
fn fallback_for_rejected(
    deps: &Arc<ProxyDeps>,
    pool: &[route::PoolEntry],
    model: &str,
) -> Option<Vec<(route::PoolEntry, String)>> {
    let (slug, name) = route::split_model_id(model)?;
    let primary = pool.iter().find(|entry| entry.slug == slug)?;
    let alternates = alternates_for(deps, pool, primary, name);
    (!alternates.is_empty()).then(|| alternates.into_iter().map(|alt| (alt, name.to_string())).collect())
}

/// 普通来源失败时的替补候选（最多 2 个，加主选共 3 个；见 `route::same_model_alternates`）。
fn alternates_for(
    deps: &Arc<ProxyDeps>,
    pool: &[route::PoolEntry],
    primary: &route::PoolEntry,
    model_name: &str,
) -> Vec<route::PoolEntry> {
    let catalog = deps
        .db
        .try_lock()
        .ok()
        .and_then(|conn| crate::gateway::catalog::list(&conn).ok())
        .unwrap_or_default();
    let serves = |slug: &str| catalog.iter().any(|entry| entry.slug == slug && entry.model == model_name);
    route::same_model_alternates(pool, primary, &serves, 2)
}

async fn expand_routing_group(
    deps: &Arc<ProxyDeps>,
    prepared: &PreparedRequest,
    pool: &[route::PoolEntry],
    state: &GatewayState<'_>,
    group_slug: &str,
    meta: &mut RouteMeta,
) -> Result<(Vec<(route::PoolEntry, String)>, Option<String>), Response> {
    let group = deps
        .db
        .lock()
        .ok()
        .and_then(|conn| crate::gateway::routing_groups::get(&conn, group_slug).ok().flatten());
    let Some(group) = group else {
        record_route(deps, prepared.request_id, meta, None, None, Some(404), "rejected", prepared.started);
        return Err(error_response(StatusCode::NOT_FOUND, &format!("没有名为「{group_slug}」的路由分组")));
    };
    let intent_verdict = if group.strategy == "intent" {
        let verdict = crate::gateway::intent::decide(
            deps.clone(),
            &group.slug,
            group.intent.as_ref(),
            prepared.protocol,
            &prepared.body,
            &prepared.session,
        )
        .await;
        eprintln!("[路由分组] {}：{}", group.slug, verdict.note);
        Some(verdict)
    } else {
        None
    };
    let bound = state.affinity.lock().ok().and_then(|affinity| {
        let connection_id = affinity.get(&prepared.session)?;
        pool.iter().find(|e| e.connection_id == connection_id).map(|e| e.slug.clone())
    });
    let usage_of = |slug: &str| -> i64 {
        deps.db
            .try_lock()
            .ok()
            .and_then(|conn| route::member_usage(&conn, slug).ok())
            .unwrap_or(0)
    };
    let (mut ordered, note) = {
        let resets_of = |slug: &str| -> Option<i64> {
            let connection_id = pool.iter().find(|e| e.slug == slug)?.connection_id.clone();
            crate::gateway::quota_snapshot::resets_of(&connection_id)
        };
        let remaining_of = |slug: &str| -> Option<f64> {
            let connection_id = pool.iter().find(|e| e.slug == slug)?.connection_id.clone();
            crate::gateway::quota_snapshot::remaining_percent(&connection_id)
        };
        let mut rotate = crate::gateway::routing_groups::rotate_state();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let exhausted_of = |slug: &str| -> bool {
            pool.iter()
                .find(|e| e.slug == slug)
                .map(|e| e.connection_id.as_str())
                .is_some_and(|id| crate::gateway::quota_snapshot::is_exhausted(id, now_ms))
        };
        let ordered = crate::gateway::routing_groups::ordered_members(
            &group,
            &mut rotate,
            &usage_of,
            &resets_of,
            &remaining_of,
            &exhausted_of,
            now_ms,
            bound.as_deref(),
        );
        if group.strategy == "pace" {
            let candidates: Vec<(&str, bool, Option<f64>)> = group
                .members
                .iter()
                .filter_map(|member| {
                    let entry = pool.iter().find(|e| e.slug == member.slug)?;
                    let supports = crate::platforms::supports(&entry.platform, "subscription_quota");
                    Some((
                        entry.connection_id.as_str(),
                        supports,
                        crate::gateway::quota_snapshot::remaining_percent(&entry.connection_id),
                    ))
                })
                .collect();
            let stale = crate::gateway::quota_snapshot::connections_needing_refresh(
                candidates,
                &crate::gateway::quota_snapshot::stale_trigger_state(),
                now_ms,
            );
            for connection_id in stale {
                crate::gateway::quota_snapshot::note_stale_trigger(&connection_id, now_ms);
                (deps.on_quota_stale)(&connection_id);
            }
        }
        ordered
    };
    if let Some(verdict) = &intent_verdict {
        crate::gateway::routing_groups::prefer_tier(&mut ordered, verdict.tier == crate::gateway::intent::Tier::Light);
    }
    let group_note = match (note, intent_verdict.map(|verdict| verdict.note)) {
        (Some(note), Some(intent)) => Some(format!("{intent}；{note}")),
        (note, intent) => note.or(intent),
    };
    meta.decision = Some(match group_note.as_deref() {
        Some(note) => format!("分组 {} · 策略 {} · {note}", group.slug, group.strategy),
        None => format!("分组 {} · 策略 {}", group.slug, group.strategy),
    });
    let (expanded, skipped) = crate::gateway::routing_groups::expand(&ordered, 0, &|slug| {
        deps.db
            .lock()
            .ok()
            .and_then(|conn| crate::gateway::routing_groups::get(&conn, slug).ok().flatten())
            .map(|group| group.members)
    });
    for reason in &skipped {
        eprintln!("[路由分组] 跳过成员：{reason}");
    }
    let list = expanded
        .into_iter()
        .filter_map(|member| {
            let entry = pool.iter().find(|e| e.slug == member.slug)?.clone();
            Some((entry, member.model))
        })
        .collect();
    Ok((list, group_note))
}

fn resolve_single_model(
    deps: &Arc<ProxyDeps>,
    prepared: &PreparedRequest,
    pool: &[route::PoolEntry],
    state: &GatewayState<'_>,
    meta: &RouteMeta,
) -> Result<Vec<(route::PoolEntry, String)>, Response> {
    let settings_default = deps.gateway_default_connection();
    let decision = {
        let history = |slug: &str| -> Option<String> {
            deps.db
                .try_lock()
                .ok()
                .and_then(|conn| route::historical_slug_message(&conn, slug))
        };
        let bare = |name: &str| -> Option<String> {
            let conn = deps.db.lock().ok()?;
            let catalog = crate::gateway::catalog::list(&conn).ok()?;
            route::pick_bare(
                catalog.iter().map(|entry| {
                    (entry.slug.as_str(), entry.platform.as_deref(), entry.kind.as_str(), entry.model.as_str())
                }),
                name,
                meta.agent.as_deref(),
            )
        };
        let mut affinity = match state.affinity.lock() {
            Ok(g) => g,
            Err(_) => return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试")),
        };
        let cooldowns = match state.cooldowns.lock() {
            Ok(g) => g,
            Err(_) => return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试")),
        };
        route::decide_with_affinity(
            pool,
            &prepared.model,
            settings_default.as_deref(),
            Some(&prepared.session),
            &mut affinity,
            &cooldowns,
            &history,
            &bare,
        )
    };

    match decision {
        RouteDecision::Direct(entry, upstream_model) => {
            let bare = upstream_model.clone();
            let mut list = vec![(entry.clone(), upstream_model)];
            list.extend(alternates_for(deps, pool, entry, &bare).into_iter().map(|alt| (alt, bare.clone())));
            Ok(list)
        }
        RouteDecision::Subscription(entry) => {
            let bare = route::split_model_id(&prepared.model)
                .map(|(_, name)| name.to_string())
                .unwrap_or_else(|| prepared.model.clone());
            let mut list = vec![(entry.clone(), String::new())];
            list.extend(alternates_for(deps, pool, entry, &bare).into_iter().map(|alt| (alt, bare.clone())));
            Ok(list)
        }
        RouteDecision::Reject {
            kind: route::RejectKind::PaymentRequired | route::RejectKind::Unauthorized,
            ..
        } if fallback_for_rejected(deps, pool, &prepared.model).is_some() => {
            Ok(fallback_for_rejected(deps, pool, &prepared.model).unwrap_or_default())
        }
        RouteDecision::Reject { kind, message, retry_after } => {
            let status = match kind {
                route::RejectKind::BadRequest => StatusCode::BAD_REQUEST,
                route::RejectKind::NotFound => StatusCode::NOT_FOUND,
                route::RejectKind::Unauthorized => StatusCode::UNAUTHORIZED,
                route::RejectKind::PaymentRequired => StatusCode::PAYMENT_REQUIRED,
                route::RejectKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
                route::RejectKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            };
            record_route(deps, prepared.request_id, meta, None, None, Some(status.as_u16()), "rejected", prepared.started);
            Err(match retry_after {
                Some(seconds) => error_response_with_retry(status, &message, seconds),
                None => error_response(status, &message),
            })
        }
    }
}

pub(super) async fn resolve_candidates(
    deps: &Arc<ProxyDeps>,
    prepared: &PreparedRequest,
) -> Result<CandidatePlan, Response> {
    let mut meta = prepared.meta.clone();
    let pool = {
        let guard = match deps.pool_index() {
            Ok(g) => g,
            Err(_) => return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试")),
        };
        route::pool_snapshot(&deps.db, guard)
    };
    let state = match (deps.affinity.as_ref(), deps.cooldowns.as_ref()) {
        (Some(a), Some(c)) => GatewayState { affinity: a, cooldowns: c },
        _ => return Err(error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试")),
    };

    let (candidates, group_note) = if let Some(group_slug) = prepared.model.strip_prefix("group/") {
        expand_routing_group(deps, prepared, &pool, &state, group_slug, &mut meta).await?
    } else {
        (resolve_single_model(deps, prepared, &pool, &state, &meta)?, None)
    };

    if candidates.is_empty() {
        record_route(deps, prepared.request_id, &meta, None, None, Some(503), "rejected", prepared.started);
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            group_note.as_deref().unwrap_or("路由分组的成员都不在模型池中：请检查分组配置"),
        ));
    }

    let mut tried_accounts: Vec<String> = Vec::new();
    let candidates: Vec<(route::PoolEntry, String)> = candidates
        .into_iter()
        .filter(|(entry, _)| match entry.account_fp.as_deref() {
            Some(fp) if tried_accounts.iter().any(|seen| seen == fp) => {
                eprintln!("[路由] 成员「{}」与前面的候选是同一账号，本次不再重复尝试", entry.slug);
                false
            }
            Some(fp) => {
                tried_accounts.push(fp.to_string());
                true
            }
            None => true,
        })
        .collect();

    let candidates = match prepared.allowed_accounts.as_ref() {
        Some(allowed) => {
            let before: Vec<String> = candidates.iter().map(|(entry, _)| entry.slug.clone()).collect();
            let kept: Vec<_> = candidates
                .into_iter()
                .filter(|(entry, _)| allowed.contains(&entry.connection_id))
                .collect();
            if kept.is_empty() {
                record_route(deps, prepared.request_id, &meta, None, None, Some(403), "rejected", prepared.started);
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    &format!("这把客户端 Key 只能使用指定的账号，本次要用的来源（{}）不在范围内", before.join("、")),
                ));
            }
            kept
        }
        None => candidates,
    };

    Ok(CandidatePlan { candidates, meta })
}
