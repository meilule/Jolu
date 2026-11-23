//! 额度快照注册表（路由分组 smart / pace / cap hold 的数据源）。
//!
//! 每次额度查询成功（[`crate::quota`]）后，把该连接的**额度事实**记在这里；
//! 路由分组据此排序成员，cap hold 据此把已用满的成员降到末尾。
//!
//! ## 记什么
//!
//! | 字段 | 谁在用 | 为什么需要 |
//! |---|---|---|
//! | `resets_at_ms` | `smart`（重置最近先用） | 额度马上回来的账号先花掉 |
//! | `remaining_percent` | `pace`（剩余最多先用） | 剩余量排序的唯一依据 |
//! | `exhausted` | cap hold | 已用满的成员降到末尾 |
//! | `updated_ms` | 快照新鲜度 | 过期快照不能继续影响选路 |
//!
//! ## 为什么 `pace` 早先是近似
//!
//! 早先这里只存重置时间，于是 `pace`（语义是「剩余最多先用」）只能退化成
//! 「重置时间最晚先用」——两者在「刚重置完」时恰好一致，但成员消耗速度不同就会分歧：
//! A 刚重置但已烧掉 90%，B 两天前重置但只用 30%，按重置时间会先选 A，按剩余量应该选 B。
//!
//! `QuotaWindow.used_percent` 一直是有的，只是没往这里存。现在存「**所有窗口里最紧的那个**
//! 的剩余百分比」（`100 - max(used_percent)`）：额度是木桶，最紧的窗口决定这个账号还能用多久。
//!
//! ## 快照新鲜度
//!
//! `updated_ms` 超过 [`STALE_AFTER_MS`] 视为过期，`pace` 不再拿它排序（如实降级为 order，
//! 并在 note 里说明）。宁可退回用户配置的顺序，也不拿一小时的旧数据假装知道剩余量。
//!
//! 快照只在内存：重启后第一次额度查询会重新填上——落盘反而要处理过期语义，不值得。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// 快照有效期。额度窗口最短常见是 5 小时，取 15 分钟：比任何窗口都短得多，
/// 又比一次查询的典型周期长。
pub(crate) const STALE_AFTER_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Clone, Copy)]
struct Snapshot {
    /// 最早的下一次重置（毫秒时间戳）
    resets_at_ms: i64,
    /// 各窗口里最紧的那个的剩余百分比（`0.0..=100.0`）；来源没给 `used_percent` 时为 None。
    ///
    /// 「最紧」= `used_percent` 最大的那个窗口。取最紧而非平均：额度是木桶效应，
    /// 5h 窗口用到 95% 而周窗口只用 10% 时，这个账号实际上快不能用了。
    remaining_percent: Option<f64>,
    /// 查询时是否已有窗口用到 100%（cap hold 依据）
    exhausted: bool,
    /// 记录时间（毫秒）。用于判过期，也让诊断能看出数据有多旧。
    updated_ms: i64,
}

static SNAPSHOT: LazyLock<Mutex<HashMap<String, Snapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 记录一个连接的额度事实。
pub(crate) fn record(
    connection_id: &str,
    resets_at_ms: i64,
    remaining_percent: Option<f64>,
    exhausted: bool,
) {
    if let Ok(mut guard) = SNAPSHOT.lock() {
        guard.insert(
            connection_id.to_string(),
            Snapshot {
                resets_at_ms,
                // 夹到 0..=100：来源给 105% 或 -3% 都只可能是坏数据或超支，按边界处理
                remaining_percent: remaining_percent.map(|p| if p.is_finite() { p.clamp(0.0, 100.0) } else { 0.0 }),
                exhausted,
                updated_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
    }
}

/// 查一个连接的下一次重置时间；没有快照时 None。
pub(crate) fn resets_of(connection_id: &str) -> Option<i64> {
    SNAPSHOT.lock().ok()?.get(connection_id).map(|s| s.resets_at_ms)
}

/// 查一个连接的剩余百分比（`0.0..=100.0`）。没有快照、或快照已过期时 None。
///
/// **过期返回 None 而不是旧值**：`pace` 拿它当排序依据，用一小时的旧数据排序
/// 比按用户配置的顺序更糟——那是用一个假事实替换一个已知的未知。
pub(crate) fn remaining_percent(connection_id: &str) -> Option<f64> {
    remaining_percent_at(connection_id, chrono::Utc::now().timestamp_millis())
}

/// [`remaining_percent`] 的可注入时间版本（测试用）。
pub(crate) fn remaining_percent_at(connection_id: &str, now_ms: i64) -> Option<f64> {
    let snapshot = SNAPSHOT.lock().ok()?.get(connection_id).copied()?;
    if now_ms.saturating_sub(snapshot.updated_ms) > STALE_AFTER_MS {
        return None;
    }
    snapshot.remaining_percent
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

/// 从额度窗口列表里取「剩余百分比」：所有给了 `used_percent` 的窗口里最紧的那个。
///
/// 一个窗口都没给 `used_percent` 时返回 None（不推断）。
pub(crate) fn remaining_from_windows(windows: &[crate::quota::QuotaWindow]) -> Option<f64> {
    let tightest = windows
        .iter()
        .filter_map(|w| w.used_percent)
        .fold(None::<f64>, |acc: Option<f64>, used| Some(acc.map_or(used, |a| a.max(used))))?;
    Some((100.0 - tightest).clamp(0.0, 100.0))
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

    fn window_with_used(used: Option<f64>) -> QuotaWindow {
        QuotaWindow { used_percent: used, ..window("2026-11-09T12:00:00Z") }
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

    /// `pace` 的核心依据：取**最紧**的窗口，不是平均值。
    ///
    /// 木桶效应：5h 用到 95%、周窗口只用 10% 时，这个账号实际上快不能用了，
    /// 取平均值（剩余 47.5%）会把它误判成很宽裕。
    #[test]
    fn remaining_takes_the_tightest_window_not_the_average() {
        let windows = [window_with_used(Some(95.0)), window_with_used(Some(10.0))];
        assert_eq!(remaining_from_windows(&windows), Some(5.0), "95% 已用 → 剩 5%");
    }

    #[test]
    fn remaining_is_none_when_no_window_reports_usage() {
        let windows = [window_with_used(None), window_with_used(None)];
        assert_eq!(remaining_from_windows(&windows), None, "来源没给就不能推断");
        assert_eq!(remaining_from_windows(&[]), None);
    }

    #[test]
    fn remaining_is_clamped_for_out_of_range_sources() {
        // 超支（105%）→ 剩 0% 而不是 -5%
        assert_eq!(remaining_from_windows(&[window_with_used(Some(105.0))]), Some(0.0));
        // 负数（坏数据）→ 剩 100% 而不是 103%
        assert_eq!(remaining_from_windows(&[window_with_used(Some(-3.0))]), Some(100.0));
    }

    #[test]
    fn record_and_resets_round_trip() {
        record("conn-a", 1_000, Some(42.5), false);
        assert_eq!(resets_of("conn-a"), Some(1_000));
        record("conn-a", 2_000, Some(30.0), false);
        assert_eq!(resets_of("conn-a"), Some(2_000), "同连接覆盖旧快照");
        assert_eq!(remaining_percent("conn-a"), Some(30.0));
        assert_eq!(resets_of("conn-b"), None);
        assert_eq!(remaining_percent("conn-b"), None);
    }

    /// 过期快照不能继续参与 `pace` 排序。
    #[test]
    fn a_stale_snapshot_stops_reporting_remaining() {
        record("conn-stale", 9_999, Some(10.0), false);
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(remaining_percent_at("conn-stale", now), Some(10.0), "刚记录时有效");
        assert_eq!(
            remaining_percent_at("conn-stale", now + STALE_AFTER_MS + 1),
            None,
            "超过有效期要如实返回未知，不能拿旧值排序",
        );
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
        record("conn-cap", now + 60_000, None, true);
        assert!(is_exhausted("conn-cap", now), "用满且重置在未来");
        record("conn-cap", now - 60_000, None, true);
        assert!(!is_exhausted("conn-cap", now), "重置时间已过，快照失效");
        record("conn-cap", now + 60_000, None, false);
        assert!(!is_exhausted("conn-cap", now), "记录为未用满");
    }
}
