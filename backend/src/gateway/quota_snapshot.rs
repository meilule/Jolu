//! 额度快照注册表（路由分组 smart / pace / cap hold 的数据源）。
//!
//! 每次额度查询成功（[`crate::quota`]）后，把该连接**最早的下一次重置时间**与
//! **是否已用满**记在这里；路由分组的 smart（重置最近先用）与 pace（剩余最多先用）
//! 按重置时间排序，cap hold 把已用满的成员降到末尾。
//!
//! - 只在内存：重置时间是时点数据，重启后第一次额度查询会重新填上——
//!   落盘反而要处理过期语义，不值得。
//! - 只有来源给出可解析的重置时间才有值；没给的成员在 smart / pace 里排最后，
//!   全员都没给时策略如实降级为 order（调用方看返回的 note）。
//! - 「已用满」只在**重置时间仍在未来**时才算数：重置时间一过，窗口就刷新了，
//!   旧快照不能继续把成员按额度用满降权。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, Copy)]
struct Snapshot {
    /// 最早的下一次重置（毫秒时间戳）
    resets_at_ms: i64,
    /// 查询时是否已有窗口用到 100%（cap hold 依据）
    exhausted: bool,
    /// 记录时间（不用，留作诊断）
    #[allow(dead_code)]
    updated_ms: i64,
}

static SNAPSHOT: LazyLock<Mutex<HashMap<String, Snapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 记录一个连接的下一次重置时间与是否已用满。
pub(crate) fn record(connection_id: &str, resets_at_ms: i64, exhausted: bool) {
    if let Ok(mut guard) = SNAPSHOT.lock() {
        guard.insert(
            connection_id.to_string(),
            Snapshot { resets_at_ms, exhausted, updated_ms: chrono::Utc::now().timestamp_millis() },
        );
    }
}

/// 查一个连接的下一次重置时间；没有快照时 None。
pub(crate) fn resets_of(connection_id: &str) -> Option<i64> {
    SNAPSHOT.lock().ok()?.get(connection_id).map(|s| s.resets_at_ms)
}

/// 该连接此刻是否处于「额度已用满」：需要快照记录为用满，且重置时间仍在未来。
///
/// 没有快照、或重置时间已过，都返回 false——不知道不等于用满，
/// 宁可让成员正常参与选路，也不能凭过期数据把它降权。
pub(crate) fn is_exhausted(connection_id: &str, now_ms: i64) -> bool {
    let Some(snapshot) = SNAPSHOT.lock().ok().and_then(|guard| guard.get(connection_id).copied()) else {
        return false;
    };
    snapshot.exhausted && snapshot.resets_at_ms > now_ms
}

/// 从额度窗口列表里取「最早的下一次重置」（毫秒）。解析不了的窗口跳过；
/// 全部解析失败返回 None——调用方不记录，路由层如实降级。
pub(crate) fn earliest_reset_from_windows(windows: &[crate::quota::QuotaWindow]) -> Option<i64> {
    windows
        .iter()
        .filter_map(|w| w.resets_at.as_deref())
        .filter_map(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|time| time.timestamp_millis())
        .min()
}

/// 是否已有窗口用满且还没到重置时间（cap hold 依据）。
///
/// 只认**同时**给出 `used_percent >= 100` 与可解析重置时间的窗口：
/// 缺任一项都不判定为用满——来源没说清楚时不能替它下结论。
pub(crate) fn exhausted_from_windows(
    windows: &[crate::quota::QuotaWindow], now_ms: i64,
) -> bool {
    windows.iter().any(|w| {
        let used_up = w.used_percent.is_some_and(|percent| percent >= 100.0);
        let pending_reset = w
            .resets_at
            .as_deref()
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .is_some_and(|time| time.timestamp_millis() > now_ms);
        used_up && pending_reset
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::QuotaWindow;

    fn window(resets: &str) -> QuotaWindow {
        QuotaWindow {
            key: "5h".into(),
            window_name: "5 小时窗口".into(),
            used_percent: None,
            amount_text: None,
            remaining_text: None,
            resets_at: Some(resets.into()),
        }
    }

    #[test]
    fn earliest_reset_picks_min_and_skips_unparsable() {
        let windows = [
            window("2026-11-09T18:00:00Z"),
            window("not-a-date"),
            window("2026-11-09T12:00:00Z"),
            QuotaWindow { resets_at: None, ..window("x") },
        ];
        let parsed = earliest_reset_from_windows(&windows).unwrap();
        let expected = chrono::DateTime::parse_from_rfc3339("2026-11-09T12:00:00Z").unwrap().timestamp_millis();
        assert_eq!(parsed, expected, "取最早的重置时间");
        assert!(earliest_reset_from_windows(&[QuotaWindow { resets_at: None, ..window("x") }]).is_none());
    }

    #[test]
    fn record_and_resets_round_trip() {
        record("conn-a", 1_000, false);
        assert_eq!(resets_of("conn-a"), Some(1_000));
        record("conn-a", 2_000, false);
        assert_eq!(resets_of("conn-a"), Some(2_000), "同连接覆盖旧快照");
        assert_eq!(resets_of("conn-b"), None);
    }

    #[test]
    fn exhausted_from_windows_needs_both_full_window_and_pending_reset() {
        let now = 1_000_000;
        let full_pending = QuotaWindow {
            used_percent: Some(100.0),
            resets_at: Some("1970-01-01T00:20:00Z".into()), // 1_200_000 ms，晚于 now
            ..window("5h")
        };
        assert!(exhausted_from_windows(&[full_pending], now), "用满且未到重置时间");

        let full_expired = QuotaWindow {
            used_percent: Some(100.0),
            resets_at: Some("1970-01-01T00:10:00Z".into()), // 600_000 ms，早于 now
            ..window("5h")
        };
        assert!(!exhausted_from_windows(&[full_expired], now), "重置时间已过：窗口刷新了，不能继续降权");

        let partial = QuotaWindow {
            used_percent: Some(99.9),
            resets_at: Some("1970-01-01T00:20:00Z".into()),
            ..window("5h")
        };
        assert!(!exhausted_from_windows(&[partial], now), "没到 100% 不算用满");

        let no_percent = QuotaWindow { used_percent: None, resets_at: Some("1970-01-01T00:20:00Z".into()), ..window("5h") };
        assert!(!exhausted_from_windows(&[no_percent], now), "来源没给用量百分比：不替它下结论");

        let no_reset = QuotaWindow { used_percent: Some(100.0), resets_at: None, ..window("5h") };
        assert!(!exhausted_from_windows(&[no_reset], now), "没有重置时间就不知道会不会自己恢复");
    }

    #[test]
    fn is_exhausted_ignores_unknown_and_expired_snapshots() {
        let now = 1_000_000;
        assert!(!is_exhausted("conn-never-seen", now), "没有快照：不知道不等于用满");
        record("conn-cap", now + 60_000, true);
        assert!(is_exhausted("conn-cap", now), "用满且重置在未来");
        record("conn-cap", now - 60_000, true);
        assert!(!is_exhausted("conn-cap", now), "重置时间已过，快照失效");
        record("conn-cap", now + 60_000, false);
        assert!(!is_exhausted("conn-cap", now), "记录为未用满");
    }
}
