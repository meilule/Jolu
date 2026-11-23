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
    /// 最早的下一次重置（毫秒时间戳）。来源只给剩余量不给重置时间时为 None——
    /// 这类来源仍要写快照（否则 `pace` 永远拿不到它的数据），只是 `smart` 用不了。
    resets_at_ms: Option<i64>,
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
///
/// **重置时间与剩余量任一有值就写快照。** 只给剩余量不给重置时间的来源（例如某些
/// 中转商的 `/usage` 端点）也要让 `pace` 用上——早先要求必须有重置时间才写，
/// 这类来源的剩余量被整个丢掉，`pace` 永远拿不到它们的数据。
pub(crate) fn record(
    connection_id: &str,
    resets_at_ms: Option<i64>,
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

/// 查一个连接的下一次重置时间；没有快照、快照已过期、或来源没给重置时间时 None。
///
/// **过期就返回 None**，与 `remaining_percent` 同一口径：`smart` 拿它排序，
/// 用一个小时的旧重置时间排序比按用户配置的顺序更糟。
pub(crate) fn resets_of(connection_id: &str) -> Option<i64> {
    resets_of_at(connection_id, chrono::Utc::now().timestamp_millis())
}

/// [`resets_of`] 的可注入时间版本（测试用）。
pub(crate) fn resets_of_at(connection_id: &str, now_ms: i64) -> Option<i64> {
    let snapshot = SNAPSHOT.lock().ok()?.get(connection_id).copied()?;
    if now_ms.saturating_sub(snapshot.updated_ms) > STALE_AFTER_MS {
        return None;
    }
    snapshot.resets_at_ms
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

    // 过期快照不再把成员按额度用满降权——授权失效的账号以前会一直残留到重置时间。
    if now_ms.saturating_sub(snapshot.updated_ms) > STALE_AFTER_MS {
        return false;
    }
    // 没有重置时间就无法判断何时恢复：不知道不等于用满。
    snapshot.exhausted && snapshot.resets_at_ms.is_some_and(|resets| resets > now_ms)
}

/// 一次额度查询的结果该怎么落到快照上（纯判定，不做 IO）。
///
/// | 查询结果 | 处置 | 理由 |
/// |---|---|---|
/// | `Ok` 且至少一个窗口给了重置时间或剩余量 | 写入 | 有新事实 |
/// | `Ok` 但一个窗口都没有 | 清除 | 来源没给任何可用事实，旧快照不再代表现实 |
/// | `Unauthorized` / `Unsupported` / `Forbidden` | 清除 | 结构性结论：这个连接不再有额度可查 |
/// | `Failed` / `RateLimited` | 保留 | 临时失败，旧数据可能仍然有效（新鲜度会兜住） |
/// | `Observed` | 不写不清 | 网关限流头推断，不是官方额度，不作为路由依据 |
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SnapshotAction {
    /// 写入快照（带下面的值）
    Write { resets_at_ms: Option<i64>, remaining_percent: Option<f64>, exhausted: bool },
    /// 清除快照
    Clear,
    /// 不动快照
    Keep,
}

/// 按查询结果决定快照处置。
///
/// 抽成纯函数是为了能脱离 `QuotaState` 的构造与真实查询单独测这五种分支——
/// 这些判断直接决定「授权失效的账号会不会继续被当作可用」。
pub(crate) fn action_for_ok(windows: &[crate::quota::QuotaWindow], now_ms: i64) -> SnapshotAction {
    let resets = earliest_reset_from_windows(windows);
    let remaining = remaining_from_windows(windows);
    // 两个都没有：这个 `Ok` 没给出任何可用于选路的事实
    if resets.is_none() && remaining.is_none() {
        return SnapshotAction::Clear;
    }
    SnapshotAction::Write {
        resets_at_ms: resets,
        remaining_percent: remaining,
        exhausted: exhausted_from_windows(windows, now_ms),
    }
}

/// 清除一个连接的快照。
///
/// 用于「查询给出了结构性结论但不再有可用的额度事实」：授权失效、平台不支持、
/// 或成功查询里一个窗口都没有。这些情况下旧快照不再代表现实，继续留着只会让
/// cap hold 与 `smart` 拿过期事实做决策。
pub(crate) fn clear(connection_id: &str) {
    if let Ok(mut guard) = SNAPSHOT.lock() {
        guard.remove(connection_id);
    }
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

/// 补查去重的记忆：连接 id → 上次触发补查的时刻（毫秒）。
///
/// 只在请求路径上用一把极短的锁（读一次 Hashmap），不碰数据库锁。
/// 进程内即可：补查本身是后台动作，重启后重来一遍无害。
static LAST_STALE_TRIGGER: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 同一连接两次补查的最小间隔。额度缓存本身是 5 分钟，1 分钟足够避免
/// 连续请求把同一个连接打进补查队列，也不会让补查拖太久。
pub(crate) const STALE_TRIGGER_COOLDOWN_MS: i64 = 60 * 1_000;

/// 从候选里挑出「需要后台补查额度」的连接（纯判定，不做 IO）。
///
/// 输入是 `(连接 id, 是否支持额度查询, 剩余量)`；输出是按输入顺序去重后的连接 id。
///
/// 判定：**剩余量为 None**（缺失或过期）**且**该连接支持额度查询**且**距上次补查
/// 已超过冷却期。不支持额度的连接不触发——那是已知没有这个能力，不是「缺数据」。
///
/// 抽成纯函数是为了能单独测这三种情况（过期 / 冷却中 / 不支持），
/// 不必启动网关。
pub(crate) fn connections_needing_refresh<'a>(
    candidates: impl IntoIterator<Item = (&'a str, bool, Option<f64>)>,
    last_triggered: &HashMap<String, i64>,
    now_ms: i64,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (connection_id, supports_quota, remaining) in candidates {
        if !supports_quota || remaining.is_some() {
            continue;
        }
        if out.iter().any(|seen| seen == connection_id) {
            continue;
        }
        let cooling = last_triggered
            .get(connection_id)
            .is_some_and(|last| now_ms.saturating_sub(*last) < STALE_TRIGGER_COOLDOWN_MS);
        if cooling {
            continue;
        }

        out.push(connection_id.to_string());
    }
    out
}

/// 记下「刚为这个连接触发过补查」。
pub(crate) fn note_stale_trigger(connection_id: &str, now_ms: i64) {
    if let Ok(mut guard) = LAST_STALE_TRIGGER.lock() {
        guard.insert(connection_id.to_string(), now_ms);
    }
}

/// 读一份补查记忆的快照（测试与判定用）。
pub(crate) fn stale_trigger_state() -> HashMap<String, i64> {
    LAST_STALE_TRIGGER.lock().map(|guard| guard.clone()).unwrap_or_default()
}

/// 清空补查记忆（测试隔离用）。
#[cfg(test)]
pub(crate) fn forget_stale_triggers() {
    if let Ok(mut guard) = LAST_STALE_TRIGGER.lock() {
        guard.clear();
    }
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
        record("conn-a", Some(1_000), Some(42.5), false);
        assert_eq!(resets_of_at("conn-a", chrono::Utc::now().timestamp_millis()), Some(1_000));
        record("conn-a", Some(2_000), Some(30.0), false);
        assert_eq!(resets_of_at("conn-a", chrono::Utc::now().timestamp_millis()), Some(2_000), "同连接覆盖旧快照");
        assert_eq!(remaining_percent("conn-a"), Some(30.0));
        assert_eq!(resets_of_at("conn-b", chrono::Utc::now().timestamp_millis()), None);
        assert_eq!(remaining_percent("conn-b"), None);
    }

    /// 过期快照不能继续参与 `pace` 排序。
    #[test]
    fn a_stale_snapshot_stops_reporting_remaining() {
        record("conn-stale", Some(9_999), Some(10.0), false);
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(remaining_percent_at("conn-stale", now), Some(10.0), "刚记录时有效");
        assert_eq!(
            remaining_percent_at("conn-stale", now + STALE_AFTER_MS + 1),
            None,
            "超过有效期要如实返回未知，不能拿旧值排序",
        );
    }

    /// F-1：补查选择的三条分支——过期会被选中、冷却中不会、不支持额度不会。
    ///
    /// 这三条决定「后台补查会不会把同一个连接打爆」与「会不会去查一个根本没有
    /// 额度的连接」。抽成纯函数就是为了能这样直接测。
    #[test]
    fn refresh_selection_covers_stale_cooling_and_unsupported() {
        let now = 1_000_000;
        let mut last = HashMap::new();
        // 过期的（剩余量 None）+ 支持额度 → 选中
        let picked = connections_needing_refresh(
            [("conn-stale", true, None), ("conn-fresh", true, Some(50.0))],
            &last,
            now,
        );
        assert_eq!(picked, vec!["conn-stale".to_string()], "只补查缺数据的");

        // 冷却中 → 不再选中
        last.insert("conn-stale".to_string(), now - 30_000);
        let picked = connections_needing_refresh([("conn-stale", true, None)], &last, now);
        assert!(picked.is_empty(), "1 分钟内补查过的不要再触发");

        // 冷却已过 → 重新选中
        last.insert("conn-stale".to_string(), now - STALE_TRIGGER_COOLDOWN_MS - 1);
        let picked = connections_needing_refresh([("conn-stale", true, None)], &last, now);
        assert_eq!(picked, vec!["conn-stale".to_string()], "冷却结束后允许再次补查");

        // 不支持额度 → 不选中（custom API Key 本来就没有官方额度）
        let picked = connections_needing_refresh([("conn-custom", false, None)], &HashMap::new(), now);
        assert!(picked.is_empty(), "不支持额度的连接不触发补查");

        // 同一连接在候选里出现两次 → 只输出一次
        let picked = connections_needing_refresh(
            [("conn-dup", true, None), ("conn-dup", true, None)],
            &HashMap::new(),
            now,
        );
        assert_eq!(picked.len(), 1, "同一连接只补查一次");
    }

    #[test]
    fn noting_a_trigger_makes_the_next_selection_skip_it() {
        forget_stale_triggers();
        let now = chrono::Utc::now().timestamp_millis();
        let picked = connections_needing_refresh([("conn-note", true, None)], &stale_trigger_state(), now);
        assert_eq!(picked.len(), 1);
        note_stale_trigger("conn-note", now);
        let picked = connections_needing_refresh([("conn-note", true, None)], &stale_trigger_state(), now);
        assert!(picked.is_empty(), "记下之后立刻再选应当跳过");
        forget_stale_triggers();
    }

    /// F-2：只给剩余量、不给重置时间的来源，剩余量也要留下来。
    ///
    /// 早先要求「有可解析的重置时间」才写快照，这类来源的剩余量被整个丢掉，
    /// `pace` 永远拿不到它们的数据——即使上游明确告诉了剩余 30%。
    #[test]
    fn a_source_without_reset_time_still_keeps_its_remaining_percent() {
        let now = chrono::Utc::now().timestamp_millis();
        record("conn-no-reset", None, Some(30.0), false);
        assert_eq!(remaining_percent_at("conn-no-reset", now), Some(30.0), "剩余量要留下来");
        assert_eq!(resets_of_at("conn-no-reset", now), None, "没给重置时间就是未知");
        assert!(!is_exhausted("conn-no-reset", now), "没有重置时间就无法判断何时恢复，不知道不等于用满");
    }

    /// F-3：`clear` 之后三个读取函数都按「不知道」处理。
    #[test]
    fn clear_makes_every_reader_report_unknown() {
        let now = chrono::Utc::now().timestamp_millis();
        record("conn-clear", Some(now + 60_000), Some(10.0), true);
        assert!(resets_of_at("conn-clear", now).is_some());
        assert_eq!(remaining_percent_at("conn-clear", now), Some(10.0));
        assert!(is_exhausted("conn-clear", now));
        clear("conn-clear");
        assert_eq!(resets_of_at("conn-clear", now), None, "清除后重置时间未知");
        assert_eq!(remaining_percent_at("conn-clear", now), None, "清除后剩余量未知");
        assert!(!is_exhausted("conn-clear", now), "清除后不再按用满降权");
        // 清除不存在的快照不报错
        clear("conn-never-existed");
    }

    /// F-3：`is_exhausted` 与 `resets_of` 也要检查新鲜度。
    ///
    /// 以前只有 `remaining_percent` 判过期，于是授权已失效的账号会一直残留到重置时间：
    /// cap hold 继续把它按「已用满」降权，`smart` 继续按旧的重置时间排序。
    #[test]
    fn reset_and_exhausted_stop_being_reported_once_the_snapshot_is_stale() {
        let now = chrono::Utc::now().timestamp_millis();
        record("conn-stale-all", Some(now + 3_600_000), Some(0.0), true);
        assert!(is_exhausted("conn-stale-all", now), "刚记录时生效");
        assert!(resets_of_at("conn-stale-all", now).is_some());
        let later = now + STALE_AFTER_MS + 1;
        assert!(!is_exhausted("conn-stale-all", later), "过期后不再按已用满降权");
        assert_eq!(resets_of_at("conn-stale-all", later), None, "过期后重置时间也算未知");
    }

    /// F-3：查询结果 → 快照处置的映射。
    ///
    /// 这五条分支直接决定「授权失效的账号会不会继续被当作可用」，必须逐条锁住。
    #[test]
    fn the_action_follows_the_query_outcome() {
        let now = chrono::Utc::now().timestamp_millis();
        let future = "2099-01-01T00:00:00Z";

        // Ok 且有重置时间 → 写入
        let with_reset = QuotaWindow {
            used_percent: Some(50.0),
            resets_at: Some(future.into()),
            ..window("5h")
        };
        match action_for_ok(&[with_reset], now) {
            SnapshotAction::Write { resets_at_ms, remaining_percent, exhausted } => {
                assert!(resets_at_ms.is_some(), "有重置时间要带上");
                assert_eq!(remaining_percent, Some(50.0));
                assert!(!exhausted);
            }
            other => panic!("Ok 有事实应写入，实际 {other:?}"),
        }

        // Ok 且只有剩余量没有重置时间 → 仍要写入（F-2）
        let only_remaining = QuotaWindow { used_percent: Some(70.0), resets_at: None, ..window("x") };
        match action_for_ok(&[only_remaining], now) {
            SnapshotAction::Write { resets_at_ms, remaining_percent, .. } => {
                assert_eq!(resets_at_ms, None);
                assert_eq!(remaining_percent, Some(30.0));
            }
            other => panic!("只有剩余量也应写入，实际 {other:?}"),
        }

        // Ok 但一个窗口都没有 → 清除
        assert_eq!(action_for_ok(&[], now), SnapshotAction::Clear, "没有任何事实应清除旧快照");

        // Ok 但窗口里既没重置时间也没剩余量 → 清除
        let nothing = QuotaWindow { used_percent: None, resets_at: None, ..window("x") };
        assert_eq!(action_for_ok(&[nothing], now), SnapshotAction::Clear);
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
        record("conn-cap", Some(now + 60_000), None, true);
        assert!(is_exhausted("conn-cap", now), "用满且重置在未来");
        record("conn-cap", Some(now - 60_000), None, true);
        assert!(!is_exhausted("conn-cap", now), "重置时间已过，快照失效");
        record("conn-cap", Some(now + 60_000), None, false);
        assert!(!is_exhausted("conn-cap", now), "记录为未用满");
    }
}
