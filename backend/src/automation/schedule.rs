use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::quota::QuotaState;
use super::store::{add_event, load_run, Schedule};

/* ─────────────────────────── 定时 ─────────────────────────── */

/// 下一次触发时间（毫秒）；`after_ms` 之后最近的一次，单次任务已过去时返回 None
pub fn next_fire<Tz: TimeZone>(tz: &Tz, s: &Schedule, after_ms: i64) -> Option<i64> {
    let time = NaiveTime::parse_from_str(&s.time, "%H:%M").ok()?;
    let at = |date: NaiveDate| tz.from_local_datetime(&date.and_time(time)).earliest().map(|d| d.timestamp_millis());
    match s.freq.as_str() {
        "once" => {
            let date = NaiveDate::parse_from_str(s.date.as_deref()?, "%Y-%m-%d").ok()?;
            at(date).filter(|ms| *ms > after_ms)
        }
        "daily" | "weekly" => {
            let start = tz.timestamp_millis_opt(after_ms).single()?.date_naive();
            (0..=8u64)
                .filter_map(|d| start.checked_add_days(chrono::Days::new(d)))
                .filter(|d| s.freq == "daily" || Some(d.weekday().number_from_monday()) == s.weekday)
                .filter_map(at)
                .find(|ms| *ms > after_ms)
        }
        _ => None,
    }
}


/* ─────────────────────────── 额度判定 ─────────────────────────── */

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowInfo {
    pub key: String,
    pub name: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QuotaVerdict {
    /// available / blocked / unknown / reauth / unsupported
    pub state: String,
    pub reason: Option<String>,
    /// 全部阻塞窗口都恢复的预计时间；未知时为 None
    pub resets_at: Option<i64>,
    pub windows: Vec<WindowInfo>,
}

impl QuotaVerdict {
    pub(crate) fn simple(state: &str, reason: impl Into<String>) -> Self {
        Self { state: state.into(), reason: Some(reason.into()), resets_at: None, windows: vec![] }
    }
}

pub fn parse_reset(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(text) { return Some(dt.timestamp_millis()); }
    let n: i64 = text.parse().ok()?;
    Some(if n < 100_000_000_000 { n * 1000 } else { n })
}

pub fn verdict(state: &QuotaState) -> QuotaVerdict {
    match state {
        QuotaState::Ok { windows, .. } => {
            let windows: Vec<WindowInfo> = windows.iter().map(|w| WindowInfo {
                key: w.key.clone(),
                name: w.window_name.clone(),
                used_percent: w.used_percent,
                resets_at: w.resets_at.as_deref().and_then(parse_reset),
                blocking: w.used_percent.is_some_and(|p| p >= 100.0),
            }).collect();
            let blocking: Vec<&WindowInfo> = windows.iter().filter(|w| w.blocking).collect();
            if blocking.is_empty() {
                return QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows };
            }
            let resets_at = if blocking.iter().all(|w| w.resets_at.is_some()) { blocking.iter().filter_map(|w| w.resets_at).max() } else { None };
            let names: Vec<&str> = blocking.iter().map(|w| w.name.as_str()).collect();
            QuotaVerdict { state: "blocked".into(), reason: Some(format!("{}已用完", names.join("、"))), resets_at, windows }
        }
        QuotaState::Unauthorized { reason } => QuotaVerdict::simple("reauth", reason.clone()),
        QuotaState::Unsupported { reason } | QuotaState::Forbidden { reason } => QuotaVerdict::simple("unsupported", reason.clone()),
        QuotaState::RateLimited { reason } | QuotaState::Failed { reason } => QuotaVerdict::simple("unknown", reason.clone()),
        // 限流头观测只是「上次请求时刻」的估算，不能据此判定额度可用或用完
        QuotaState::Observed { reason, .. } => QuotaVerdict::simple("unknown", format!("{reason}（只有上次请求的限流头观测，不作为额度判断依据）")),
    }
}

/// 恢复时间未知或检查失败：5、10、20、40 分钟，之后每 60 分钟
pub(crate) fn backoff(failures: i64) -> i64 {
    let minutes = match failures { ..=1 => 5, 2 => 10, 3 => 20, 4 => 40, _ => 60 };
    minutes * 60_000
}

/// 把一次额度检查的结果写入等待中的运行；返回是否转入队列
pub fn apply_check(conn: &Connection, run_id: i64, v: &QuotaVerdict, now: i64) -> rusqlite::Result<bool> {
    let Some(run) = load_run(conn, run_id)? else { return Ok(false) };
    if run.status != "waiting" { return Ok(false); }
    let waited = now - run.wait_started_at.unwrap_or(now);
    let windows = v.windows.iter()
        .map(|w| format!("{} {}", w.name, w.used_percent.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "未知".into())))
        .collect::<Vec<_>>().join(" · ");
    let set_wait = |kind: &str, reason: &str, resets: Option<i64>, next: i64, failures: i64| -> rusqlite::Result<()> {
        if run.block_kind.as_deref() != Some(kind) || run.reason.as_deref() != Some(reason) {
            add_event(conn, run_id, now, "check", "检查额度", Some(reason))?;
        }
        conn.execute(
            "UPDATE automation_runs SET block_kind=?2, reason=?3, resets_at=?4, last_check_at=?5, next_check_at=?6, check_failures=?7 WHERE id=?1",
            params![run_id, kind, reason, resets, now, next, failures],
        )?;
        Ok(())
    };
    match v.state.as_str() {
        "available" => {
            conn.execute(
                "UPDATE automation_runs SET status='queued', block_kind=NULL, reason=NULL, last_check_at=?2, next_check_at=NULL, check_failures=0, wait_ms=wait_ms+?3, wait_started_at=NULL, resume_confirmed=resume_pending WHERE id=?1",
                params![run_id, now, waited],
            )?;
            let detail = if windows.is_empty() { "额度已可用".to_string() } else { windows };
            add_event(conn, run_id, now, "check", "检查额度恢复", Some(&detail))?;
            return Ok(true);
        }
        "blocked" => match v.resets_at {
            Some(at) => {
                let next = if at > now { at + 30_000 } else { now + backoff(1) };
                set_wait("quota", v.reason.as_deref().unwrap_or("额度暂不可用"), Some(at), next, 0)?;
            }
            None => {
                let failures = run.check_failures + 1;
                set_wait("quota_unknown", "恢复时间未知", None, now + backoff(failures), failures)?;
            }
        },
        "unknown" => {
            let failures = run.check_failures + 1;
            let reason = format!("检查失败 · {}", v.reason.as_deref().unwrap_or("未知原因"));
            set_wait("check_failed", &reason, run.resets_at, now + backoff(failures), failures)?;
        }
        "reauth" => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='reauth', reason=?2, last_check_at=?3, next_check_at=NULL, wait_ms=wait_ms+?4, wait_started_at=NULL WHERE id=?1",
                params![run_id, "需要重新登录：停止额度检查，保留任务；重新认证并检查账号后才能继续。", now, waited],
            )?;
            add_event(conn, run_id, now, "attention", "需要重新登录", v.reason.as_deref())?;
        }
        _ => {
            // 不支持读取额度：执行中耗尽的运行在执行器给出的时间（或间隔重试）直接尝试继续；
            // 额度恢复后启动的任务无法判断，转为需要处理
            if run.resume_pending {
                match run.resets_at {
                    Some(at) if now >= at => {
                        conn.execute(
                            "UPDATE automation_runs SET status='queued', block_kind=NULL, reason=NULL, last_check_at=?2, next_check_at=NULL, wait_ms=wait_ms+?3, wait_started_at=NULL, resume_confirmed=1 WHERE id=?1",
                            params![run_id, now, waited],
                        )?;
                        add_event(conn, run_id, now, "check", "到达预计恢复时间", Some("该账号不支持读取额度，直接尝试继续"))?;
                        return Ok(true);
                    }
                    Some(at) => set_wait("quota_unknown", "该账号不支持读取额度，将在预计时间尝试继续", Some(at), at + 30_000, run.check_failures)?,
                    None => {
                        conn.execute(
                            "UPDATE automation_runs SET resets_at=?2 WHERE id=?1",
                            params![run_id, now + 30 * 60_000],
                        )?;
                        set_wait("quota_unknown", "恢复时间未知 · 30 分钟后尝试继续", Some(now + 30 * 60_000), now + 30 * 60_000 + 30_000, run.check_failures + 1)?;
                    }
                }
            } else {
                conn.execute(
                    "UPDATE automation_runs SET status='attention', block_kind='unsupported', reason=?2, last_check_at=?3, next_check_at=NULL, wait_ms=wait_ms+?4, wait_started_at=NULL WHERE id=?1",
                    params![run_id, format!("该账号不支持读取额度：{}", v.reason.as_deref().unwrap_or("")), now, waited],
                )?;
                add_event(conn, run_id, now, "attention", "无法读取额度", v.reason.as_deref())?;
            }
        }
    }
    Ok(false)
}
