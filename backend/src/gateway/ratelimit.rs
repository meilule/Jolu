//! 转发响应里的限流头（4.3，见 `docs/specs/2026-11-05-provider-quota-descriptor.md` §3.1 / §4.4）。
//!
//! 网关替用户向上游转发时，从**自己发出的那一次响应**里读限流头，按连接记下「最近一次观测」。
//! 它只是官方额度接口缺失时的降级来源（`confidence = derived`）：
//! - 没有请求就没有观测；空闲时界面拿到的永远是「上次请求时刻」的值，并写明观测时刻；
//! - 三类头分别解析：Anthropic `anthropic-ratelimit-*`（重置是绝对时间）、OpenAI 式
//!   `x-ratelimit-*-{requests,tokens}`（重置是时长）、IETF 草案 `RateLimit` / `RateLimit-Policy`；
//! - **相对时长不换算成绝对重置时间**（草案 `t` 与 OpenAI 的 reset 时长都受代理与时钟影响），
//!   只在文字里写「观测时约 N 后重置」；不带单位的 `x-ratelimit-reset` 含义不统一，直接忽略。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use axum::http::HeaderMap;

/// 一个限流维度的观测值。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Observation {
    /// 维度：requests / tokens / input-tokens / output-tokens / IETF 策略名
    pub name: String,
    pub limit: Option<f64>,
    pub remaining: f64,
    /// 绝对重置时刻（只有来源给的是绝对时间时才有）
    pub resets_at: Option<String>,
    /// 观测时距重置的秒数（来源给的是时长时）；不换算成绝对时间
    pub resets_in_s: Option<f64>,
}

/// 某条连接最近一次观测。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub observed_at_ms: i64,
    pub items: Vec<Observation>,
}

/// 观测超过这么久不再拿来显示（一天前的剩余量对现在没有意义）。
pub(crate) const MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

static LAST: LazyLock<Mutex<HashMap<String, Snapshot>>> = LazyLock::new(Default::default);

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|value| value.to_str().ok()).map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn number(headers: &HeaderMap, name: &str) -> Option<f64> {
    header(headers, name)?.parse::<f64>().ok().filter(|value| value.is_finite() && *value >= 0.0)
}

/// OpenAI 式时长：`1s` / `6m0s` / `20ms` / `1h2m3.5s`。没有单位的纯数字含义不统一，返回 None。
fn duration_seconds(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || text.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let mut total = 0.0;
    let mut digits = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            digits.push(c);
            continue;
        }
        let value: f64 = digits.parse().ok()?;
        digits.clear();
        let factor = match c {
            'h' => 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                0.001
            }
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        total += value * factor;
    }
    digits.is_empty().then_some(total)
}

/// IETF 草案的字典成员：`"name";k=v;k=v, "other";k=v` → [(name, {k: v})]。
fn ietf_items(text: &str) -> Vec<(String, HashMap<String, f64>)> {
    text.split(',')
        .filter_map(|item| {
            let mut parts = item.split(';');
            let name = parts.next()?.trim().trim_matches('"').to_string();
            if name.is_empty() {
                return None;
            }
            let params = parts
                .filter_map(|part| {
                    let (key, value) = part.split_once('=')?;
                    Some((key.trim().to_string(), value.trim().trim_matches('"').parse::<f64>().ok()?))
                })
                .collect();
            Some((name, params))
        })
        .collect()
}

/// 从一次响应的头里解析出全部维度（没有任何限流头时为空）。
pub(crate) fn parse(headers: &HeaderMap) -> Vec<Observation> {
    let mut out = Vec::new();
    for dimension in ["requests", "tokens", "input-tokens", "output-tokens"] {
        if let Some(remaining) = number(headers, &format!("anthropic-ratelimit-{dimension}-remaining")) {
            out.push(Observation {
                name: dimension.to_string(),
                limit: number(headers, &format!("anthropic-ratelimit-{dimension}-limit")),
                remaining,
                resets_at: header(headers, &format!("anthropic-ratelimit-{dimension}-reset"))
                    .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok()),
                resets_in_s: None,
            });
        }
    }
    if out.is_empty() {
        for dimension in ["requests", "tokens"] {
            if let Some(remaining) = number(headers, &format!("x-ratelimit-remaining-{dimension}")) {
                out.push(Observation {
                    name: dimension.to_string(),
                    limit: number(headers, &format!("x-ratelimit-limit-{dimension}")),
                    remaining,
                    resets_at: None,
                    resets_in_s: header(headers, &format!("x-ratelimit-reset-{dimension}")).as_deref().and_then(duration_seconds),
                });
            }
        }
    }
    if out.is_empty() {
        if let Some(state) = header(headers, "ratelimit") {
            let policies: HashMap<String, HashMap<String, f64>> =
                header(headers, "ratelimit-policy").map(|text| ietf_items(&text).into_iter().collect()).unwrap_or_default();
            for (name, params) in ietf_items(&state) {
                let Some(remaining) = params.get("r").copied() else { continue };
                out.push(Observation {
                    limit: policies.get(&name).and_then(|policy| policy.get("q").copied()),
                    name,
                    remaining,
                    resets_at: None,
                    resets_in_s: params.get("t").copied(),
                });
            }
        }
    }
    if out.is_empty() {
        // 不带维度后缀的通用写法：只认剩余与上限，重置含义不统一，不读
        if let Some(remaining) = number(headers, "x-ratelimit-remaining") {
            out.push(Observation { name: "requests".into(), limit: number(headers, "x-ratelimit-limit"), remaining, resets_at: None, resets_in_s: None });
        }
    }
    out
}

/// 记下一次响应的观测（没有限流头时不覆盖上一次）。
pub(crate) fn observe(connection_id: &str, headers: &HeaderMap, now_ms: i64) {
    let items = parse(headers);
    if items.is_empty() {
        return;
    }
    if let Ok(mut map) = LAST.lock() {
        map.insert(connection_id.to_string(), Snapshot { observed_at_ms: now_ms, items });
    }
}

/// 某条连接最近一次、且不超过 [`MAX_AGE_MS`] 的观测。
pub(crate) fn latest(connection_id: &str, now_ms: i64) -> Option<Snapshot> {
    LAST.lock()
        .ok()?
        .get(connection_id)
        .filter(|snapshot| now_ms - snapshot.observed_at_ms <= MAX_AGE_MS)
        .cloned()
}

fn dimension_label(name: &str) -> String {
    match name {
        "requests" => "请求数".into(),
        "tokens" => "Token".into(),
        "input-tokens" => "输入 Token".into(),
        "output-tokens" => "输出 Token".into(),
        other => format!("限额「{other}」"),
    }
}

fn amount(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value}") }
}

fn span(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as i64;
    match seconds {
        0..=59 => format!("{seconds} 秒"),
        60..=3599 => format!("{} 分钟", (seconds + 30) / 60),
        _ => format!("{:.1} 小时", seconds as f64 / 3600.0),
    }
}

/// 观测 → 额度窗口。窗口名写明「上次请求观测 HH:MM」，任何界面都能和官方值区分开；
/// 有上限才算已用百分比，没有上限只写剩余量，不编分母。
pub(crate) fn windows(snapshot: &Snapshot) -> Vec<crate::quota::QuotaWindow> {
    let at = chrono::DateTime::from_timestamp_millis(snapshot.observed_at_ms)
        .map(|time| time.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default();
    snapshot
        .items
        .iter()
        .map(|item| {
            let used_percent = item
                .limit
                .filter(|limit| *limit > 0.0)
                .map(|limit| ((limit - item.remaining) / limit * 100.0).clamp(0.0, 100.0));
            let reset_hint = item.resets_in_s.map(|seconds| format!("，观测时约 {}后重置", span(seconds))).unwrap_or_default();
            crate::quota::QuotaWindow {
                key: format!("rl-{}", item.name),
                window_name: format!("{} · 上次请求观测 {at}", dimension_label(&item.name)),
                used_percent,
                amount_text: item.limit.map(|limit| format!("剩余 {} / {}{reset_hint}", amount(item.remaining), amount(limit))),
                remaining_text: item.limit.is_none().then(|| format!("剩余 {}{reset_hint}", amount(item.remaining))),
                resets_at: item.resets_at.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), value.parse().unwrap());
        }
        map
    }

    #[test]
    fn anthropic_headers_keep_absolute_resets() {
        let found = parse(&headers(&[
            ("anthropic-ratelimit-requests-limit", "50"),
            ("anthropic-ratelimit-requests-remaining", "49"),
            ("anthropic-ratelimit-requests-reset", "2026-11-12T10:00:00Z"),
            ("anthropic-ratelimit-tokens-remaining", "70000"),
            ("anthropic-ratelimit-tokens-reset", "not-a-date"),
        ]));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], Observation { name: "requests".into(), limit: Some(50.0), remaining: 49.0, resets_at: Some("2026-11-12T10:00:00Z".into()), resets_in_s: None });
        assert_eq!(found[1].limit, None);
        assert_eq!(found[1].resets_at, None, "不是 RFC3339 的重置时间不要");
    }

    #[test]
    fn openai_durations_stay_relative_and_bare_numbers_are_ignored() {
        let found = parse(&headers(&[
            ("x-ratelimit-limit-requests", "500"),
            ("x-ratelimit-remaining-requests", "499"),
            ("x-ratelimit-reset-requests", "120ms"),
            ("x-ratelimit-limit-tokens", "30000"),
            ("x-ratelimit-remaining-tokens", "29000"),
            ("x-ratelimit-reset-tokens", "6m0s"),
        ]));
        assert_eq!(found[0].resets_in_s, Some(0.12));
        assert_eq!(found[1].resets_in_s, Some(360.0));
        assert!(found.iter().all(|item| item.resets_at.is_none()), "时长不换算成绝对时间");
        assert_eq!(duration_seconds("1h2m3.5s"), Some(3723.5));
        assert_eq!(duration_seconds("30"), None, "纯数字含义不统一");
        let generic = parse(&headers(&[("x-ratelimit-limit", "100"), ("x-ratelimit-remaining", "7"), ("x-ratelimit-reset", "1731400000")]));
        assert_eq!(generic, vec![Observation { name: "requests".into(), limit: Some(100.0), remaining: 7.0, resets_at: None, resets_in_s: None }]);
    }

    #[test]
    fn ietf_draft_headers_pair_policy_and_state_by_name() {
        let found = parse(&headers(&[
            ("ratelimit-policy", "\"burst\";q=100;w=60, \"day\";q=1000;w=86400"),
            ("ratelimit", "\"burst\";r=40;t=30, \"day\";r=900;t=3600"),
        ]));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], Observation { name: "burst".into(), limit: Some(100.0), remaining: 40.0, resets_at: None, resets_in_s: Some(30.0) });
        assert_eq!(found[1].limit, Some(1000.0));
        assert!(parse(&headers(&[("content-type", "application/json")])).is_empty());
    }

    #[test]
    fn observations_become_labelled_windows_and_expire() {
        let snapshot = Snapshot {
            observed_at_ms: 1_000,
            items: vec![
                Observation { name: "requests".into(), limit: Some(50.0), remaining: 40.0, resets_at: None, resets_in_s: Some(90.0) },
                Observation { name: "tokens".into(), limit: None, remaining: 7000.0, resets_at: None, resets_in_s: None },
            ],
        };
        let windows = windows(&snapshot);
        assert_eq!(windows[0].key, "rl-requests");
        assert!(windows[0].window_name.contains("上次请求观测"));
        assert_eq!(windows[0].used_percent, Some(20.0));
        assert_eq!(windows[0].amount_text.as_deref(), Some("剩余 40 / 50，观测时约 2 分钟后重置"));
        assert_eq!(windows[1].used_percent, None, "没有上限不编百分比");
        assert_eq!(windows[1].remaining_text.as_deref(), Some("剩余 7000"));

        observe("conn-rl", &headers(&[("x-ratelimit-remaining", "3")]), 5_000);
        observe("conn-rl", &headers(&[("content-type", "text/plain")]), 6_000);
        assert_eq!(latest("conn-rl", 6_000).map(|s| s.observed_at_ms), Some(5_000), "没有限流头的响应不覆盖上一次");
        assert!(latest("conn-rl", 5_000 + MAX_AGE_MS + 1).is_none());
    }
}
