//! 额度快照注册表（路由分组 smart / pace 策略的数据源）。
//!
//! 每次额度查询成功（[`crate::quota`]）后，把该连接**最早的下一次重置时间**记在这里；
//! 路由分组的 smart（重置最近先用）与 pace（剩余最多先用）按它排序。
//!
//! - 只在内存：重置时间是时点数据，重启后第一次额度查询会重新填上——
//!   落盘反而要处理过期语义，不值得。
//! - 只有来源给出可解析的重置时间才有值；没给的成员在 smart / pace 里排最后，
//!   全员都没给时策略如实降级为 order（调用方看返回的 note）。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, Copy)]
struct Snapshot {
    /// 最早的下一次重置（毫秒时间戳）
    resets_at_ms: i64,
    /// 记录时间（不用，留作诊断）
    #[allow(dead_code)]
    updated_ms: i64,
}

static SNAPSHOT: LazyLock<Mutex<HashMap<String, Snapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 记录一个连接的下一次重置时间。
pub(crate) fn record(connection_id: &str, resets_at_ms: i64) {
    if let Ok(mut guard) = SNAPSHOT.lock() {
        guard.insert(
            connection_id.to_string(),
            Snapshot { resets_at_ms, updated_ms: chrono::Utc::now().timestamp_millis() },
        );
    }
}

/// 查一个连接的下一次重置时间；没有快照时 None。
pub(crate) fn resets_of(connection_id: &str) -> Option<i64> {
    SNAPSHOT.lock().ok()?.get(connection_id).map(|s| s.resets_at_ms)
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
        record("conn-a", 1_000);
        assert_eq!(resets_of("conn-a"), Some(1_000));
        record("conn-a", 2_000);
        assert_eq!(resets_of("conn-a"), Some(2_000), "同连接覆盖旧快照");
        assert_eq!(resets_of("conn-b"), None);
    }
}
