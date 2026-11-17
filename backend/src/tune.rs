//! 成本调优建议（5.10 / N9，移植 magpie `internal/tune/tune.go`）：拿用户自己这段时间的每次调用
//! 逐条回放，算出「在哪个上下文大小压缩」「提示词缓存留 5 分钟还是 1 小时」最省加权 Token。
//! 结论只对这个用户、这个工具成立。
//!
//! 加权：按厂商对各类 Token 相对普通输入的收费——缓存读 0.1、缓存写 1.25（1 小时 2）、输出 5
//! （Claude）；OpenAI 缓存写按输入价、读 0.1、输出 8。订阅额度与 API 账单都按这个比例算。
//!
//! 与 magpie 的差异：Jolu 的 `requests` 表没有「1 小时缓存写入」的拆分、没有子代理标记、也没有
//! 压缩耗时，所以不算等待时长；当前缓存时长不知道时按 5 分钟推断并在结果里写明。

use std::collections::BTreeMap;

use serde::Serialize;

/// 回放用的一次调用
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub time_ms: i64,
    /// 所在会话
    pub stream: String,
    /// 去掉 `[1m]`、小写
    pub model: String,
    /// 带 `[1m]` 请求的
    pub long: bool,
    /// 提示词总量：输入 + 缓存读 + 缓存写
    pub prompt: i64,
    pub out: i64,
    /// 提示词里写进缓存的
    pub wrote: i64,
    /// 其中留 1 小时的（Jolu 没有拆分时为 0）
    pub wrote_1h: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub read: f64,
    pub write: f64,
    pub write_1h: f64,
    pub out: f64,
}

/// 按模型名判厂商；`来源/模型` 的前缀先去掉
pub fn weights_for(model: &str) -> Weights {
    let bare = model.rsplit('/').next().unwrap_or(model);
    if bare.starts_with("claude") {
        Weights { read: 0.1, write: 1.25, write_1h: 2.0, out: 5.0 }
    } else {
        Weights { read: 0.1, write: 1.0, write_1h: 1.0, out: 8.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Point {
    pub value: i64,
    pub cost: f64,
    pub compacts: i64,
}

/// 从调用里读出的习惯，和建议一起给用户看
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Facts {
    pub calls: i64,
    pub streams: i64,
    /// 这段时间里看到的压缩次数
    pub compacts: i64,
    /// 压缩前提示词的中位数
    pub peak: i64,
    /// 压缩后提示词的中位数
    pub after: i64,
    /// 相邻两次调用提示词增长的中位数
    pub growth: i64,
    /// 压缩后十次调用里多读回来的量
    pub rework: i64,
    /// 调用间隔：5 分钟内 / 5 分钟到 1 小时 / 超过 1 小时 的占比
    pub quick: f64,
    pub pause: f64,
    pub away: f64,
    /// 缓存写入里留 1 小时的占比
    pub hour: f64,
    /// 到过压缩的会话数
    pub sessions: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Compaction {
    pub model: String,
    /// 现在按多少压缩（推断或读到的设置）
    pub current: i64,
    /// 模型窗口（设置不能超过它）；0 = 不知道
    pub window: i64,
    pub best: i64,
    /// 用 current 的加权 Token
    pub cost: f64,
    /// 用 best 的
    pub low: f64,
    pub curve: Vec<Point>,
    pub facts: Facts,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompactOpts {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    /// 工具在窗口下留的余量：Claude Code 在窗口减输出余量处压缩（200K 的 167K），Codex 就在设置处
    pub reserve: i64,
    /// 现在的设置；0 = 没设，按压缩点推断
    pub current: i64,
    /// 模型整窗；0 = 不知道
    pub window: i64,
}

/// 压缩前后掉到之前的这么多以下，且之前至少这么长，算一次压缩
const DROP_SHARE: f64 = 0.6;
const DROP_FROM: i64 = 20_000;
const REWORK_CALLS: usize = 10;
/// 省不到 3% 不建议改
const SAME_COST: f64 = 0.03;
/// 少于这么多调用不给建议
pub const MIN_CALLS: i64 = 50;

fn compacted(prev: i64, cur: i64) -> bool {
    prev >= DROP_FROM && (cur as f64) < DROP_SHARE * prev as f64
}

fn median(xs: &[i64]) -> i64 {
    if xs.is_empty() {
        return 0;
    }
    let mut s = xs.to_vec();
    s.sort_unstable();
    s[s.len() / 2]
}

/// 按会话分组，组内按时间排序，组按会话名排序
fn streams(calls: &[Call]) -> Vec<Vec<Call>> {
    let mut by: BTreeMap<&str, Vec<Call>> = BTreeMap::new();
    for call in calls {
        by.entry(call.stream.as_str()).or_default().push(call.clone());
    }
    by.into_values()
        .map(|mut s| {
            s.sort_by_key(|c| c.time_ms);
            s
        })
        .collect()
}

fn observe(ss: &[Vec<Call>]) -> Facts {
    let mut f = Facts::default();
    let (mut peaks, mut afters, mut growths, mut reworks) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for s in ss {
        f.streams += 1;
        f.calls += s.len() as i64;
        let mut ds = Vec::new();
        let mut cuts = Vec::new();
        for i in 1..s.len() {
            if compacted(s[i - 1].prompt, s[i].prompt) {
                peaks.push(s[i - 1].prompt);
                afters.push(s[i].prompt);
                cuts.push(i);
                continue;
            }
            let d = s[i].prompt - s[i - 1].prompt;
            if d >= 0 {
                ds.push(d);
            }
        }
        if !cuts.is_empty() {
            f.sessions += 1;
        }
        let g = median(&ds);
        growths.extend(ds);
        for at in cuts {
            let end = at + REWORK_CALLS;
            if end >= s.len() {
                continue;
            }
            let clean = (at + 1..=end).all(|j| !compacted(s[j - 1].prompt, s[j].prompt));
            if clean {
                reworks.push((s[end].prompt - s[at].prompt - REWORK_CALLS as i64 * g).max(0));
            }
        }
    }
    f.compacts = peaks.len() as i64;
    f.peak = median(&peaks);
    f.after = median(&afters);
    f.growth = median(&growths);
    f.rework = median(&reworks);
    f
}

fn base(ss: &[Vec<Call>]) -> i64 {
    median(&ss.iter().filter_map(|s| s.first().map(|c| c.prompt)).collect::<Vec<_>>())
}

/// 按阈值压缩时这段时间要花多少（加权），返回（成本，压缩次数）
#[allow(clippy::too_many_arguments)]
fn replay_compact(ss: &[Vec<Call>], threshold: i64, after: i64, b: i64, rework: i64, w: Weights, write: f64) -> (f64, i64) {
    let (mut cost, mut n) = (0.0, 0);
    let summary = (after - b).max(0);
    for s in ss {
        let Some(first) = s.first() else { continue };
        let mut ctx = first.prompt;
        cost += ctx as f64 * write + first.out as f64 * w.out;
        for i in 1..s.len() {
            let mut d = s[i].prompt - s[i - 1].prompt;
            if d < 0 || compacted(s[i - 1].prompt, s[i].prompt) {
                d = 0;
            }
            if ctx + d > threshold && ctx > after {
                cost += ctx as f64 * w.read + summary as f64 * w.out + (summary + rework) as f64 * write;
                ctx = after + rework;
                n += 1;
            }
            cost += ctx as f64 * w.read + d as f64 * write + s[i].out as f64 * w.out;
            ctx += d;
        }
    }
    (cost, n)
}

fn round_to(v: i64, step: i64) -> i64 {
    if step <= 0 { v } else { (v + step / 2) / step * step }
}

fn round_up(v: i64, step: i64) -> i64 {
    if step <= 0 { v } else { (v + step - 1) / step * step }
}

/// 一个模型的压缩窗口建议；调用太少或从没接近窗口时返回 None
pub fn compact(calls: &[Call], o: CompactOpts) -> Option<Compaction> {
    let ss = streams(calls);
    let mut f = observe(&ss);
    if f.calls < MIN_CALLS || f.streams == 0 || o.step <= 0 {
        return None;
    }
    let model = calls.first()?.model.clone();
    let w = weights_for(&model);
    let mut write = w.write;
    let wrote: i64 = calls.iter().map(|c| c.wrote).sum();
    let wrote_1h: i64 = calls.iter().map(|c| c.wrote_1h).sum();
    if wrote > 0 {
        f.hour = wrote_1h as f64 / wrote as f64;
        if f.hour > 0.5 {
            write = w.write_1h;
        }
    }
    let b = base(&ss);
    // 没见过压缩：Claude Code 与 Codex 的摘要大约比会话开头多 10K
    let after = if f.after == 0 { b + 10_000 } else { f.after };
    let rework = if f.compacts < 5 { f.rework.max(10_000) } else { f.rework };
    let mut cur = o.current;
    if cur == 0 {
        cur = if f.compacts >= 3 { round_to(f.peak + o.reserve, o.step) } else if o.window > 0 { o.window } else { o.max };
    }
    let mut top = o.max;
    if o.window > 0 {
        cur = cur.min(o.window);
        top = o.max.min(o.window);
    }
    // 窗口要在摘要上方留出余地，否则会一遍遍压缩
    let floor = o.min.max(round_up(after + (after / 2).max(20 * f.growth) + o.reserve, o.step));
    let run = |v: i64| {
        let (cost, compacts) = replay_compact(&ss, (after + 1).max(v - o.reserve), after, b, rework, w, write);
        Point { value: v, cost, compacts }
    };
    let now = run(cur);
    let mut curve = Vec::new();
    let mut v = floor;
    while v <= top {
        curve.push(run(v));
        v += o.step;
    }
    let low = *curve.iter().min_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap_or(std::cmp::Ordering::Equal))?;
    // 比最省的贵不到 1% 的里取最大的：压缩越少，会话丢的越少
    let best = curve.iter().filter(|p| p.cost <= low.cost * 1.01).max_by_key(|p| p.value).copied().unwrap_or(low);
    let (mut best_value, mut best_cost) = (best.value, best.cost);
    if now.cost <= 0.0 || (now.cost - best_cost) / now.cost < SAME_COST {
        best_value = cur;
        best_cost = now.cost;
    }
    Some(Compaction { model, current: cur, window: o.window, best: best_value, cost: now.cost, low: best_cost, curve, facts: f })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ttl {
    /// 5m / 1h
    pub current: String,
    /// current 是推断出来的（Jolu 记录里没有 1 小时写入的拆分）
    pub current_inferred: bool,
    pub best: String,
    pub cost: f64,
    pub low: f64,
    pub short: f64,
    pub hour: f64,
    pub facts: Facts,
}

fn replay_ttl(ss: &[Vec<Call>], ttl_ms: i64, hour: bool) -> f64 {
    let mut cost = 0.0;
    for s in ss {
        let Some(first) = s.first() else { continue };
        let w = weights_for(&first.model);
        let write = if hour { w.write_1h } else { w.write };
        cost += first.prompt as f64 * write;
        for i in 1..s.len() {
            let (p, c) = (s[i - 1].prompt, s[i].prompt);
            cost += if s[i].time_ms - s[i - 1].time_ms > ttl_ms || compacted(p, c) {
                c as f64 * write
            } else if c >= p {
                p as f64 * w.read + (c - p) as f64 * write
            } else {
                c as f64 * w.read
            };
        }
    }
    cost
}

/// Claude Code 提示词缓存留多久的建议；`current` 为空时按写入推断
pub fn cache_ttl(calls: &[Call], current: Option<&str>) -> Option<Ttl> {
    let ss = streams(calls);
    let mut f = observe(&ss);
    if f.calls < MIN_CALLS {
        return None;
    }
    let (mut wrote, mut wrote_1h, mut gaps, mut quick, mut pause) = (0i64, 0i64, 0i64, 0i64, 0i64);
    for s in &ss {
        for (i, c) in s.iter().enumerate() {
            wrote += c.wrote;
            wrote_1h += c.wrote_1h;
            if i == 0 {
                continue;
            }
            gaps += 1;
            let gap = c.time_ms - s[i - 1].time_ms;
            if gap <= 5 * 60_000 {
                quick += 1;
            } else if gap <= 3_600_000 {
                pause += 1;
            }
        }
    }
    if gaps > 0 {
        f.quick = quick as f64 / gaps as f64;
        f.pause = pause as f64 / gaps as f64;
        f.away = 1.0 - f.quick - f.pause;
    }
    if wrote > 0 {
        f.hour = wrote_1h as f64 / wrote as f64;
    }
    let (current, inferred) = match current {
        Some(c) => (c.to_string(), false),
        None => ((if f.hour > 0.5 { "1h" } else { "5m" }).to_string(), wrote_1h == 0),
    };
    let short = replay_ttl(&ss, 5 * 60_000, false);
    let hour = replay_ttl(&ss, 3_600_000, true);
    let cost = if current == "1h" { hour } else { short };
    let (mut best, mut low) = if hour < short { ("1h".to_string(), hour) } else { ("5m".to_string(), short) };
    if cost <= 0.0 || (cost - low) / cost < SAME_COST {
        best = current.clone();
        low = cost;
    }
    Some(Ttl { current, current_inferred: inferred, best, cost, low, short, hour, facts: f })
}

/* ─────────────────────────── 从 Jolu 的记录取数 ─────────────────────────── */

/// 读一个平台这段时间成功的调用（只要四类用量都已知、提示词大于 0、有会话标识的）
pub fn calls_from_db(conn: &rusqlite::Connection, platform: &str, since_ms: i64) -> rusqlite::Result<Vec<Call>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, ts, COALESCE(model, ''), input_tokens, output_tokens, cache_read_tokens, cache_write_tokens
         FROM requests
         WHERE platform = ?1 AND ts >= ?2 AND session_id IS NOT NULL AND session_id <> ''
           AND input_known = 1 AND output_known = 1 AND cache_read_known = 1 AND cache_write_known = 1
           AND (status_code IS NULL OR status_code < 400)",
    )?;
    let rows = stmt.query_map(rusqlite::params![platform, since_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?))
    })?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (session, ts, model, input, output, read, write) = row;
        let prompt = input + read + write;
        if prompt <= 0 {
            continue;
        }
        let lower = model.trim().to_ascii_lowercase();
        let (model, long) = match lower.strip_suffix("[1m]") {
            Some(bare) => (bare.to_string(), true),
            None => (lower, false),
        };
        out.push(Call { time_ms: ts, stream: format!("{platform}/{session}"), model, long, prompt, out: output, wrote: write, wrote_1h: 0 });
    }
    Ok(out)
}

/// 一个工具的建议
#[derive(Debug, Clone, Serialize)]
pub struct AgentAdvice {
    /// claude / codex
    pub platform: String,
    pub calls: i64,
    /// 读到的当前压缩设置（Claude Code 的 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`、Codex 的
    /// `model_auto_compact_token_limit`）；None = 没设，按压缩点推断
    pub compact_setting: Option<i64>,
    pub compactions: Vec<Compaction>,
    pub cache: Option<Ttl>,
}

/// 生成两家的建议。`claude_setting` / `codex_setting` 是读到的当前压缩设置；
/// `window_of` 查模型的上下文窗口（models.dev 元数据，不知道返回 None）。
pub fn advise(by_platform: &BTreeMap<String, Vec<Call>>, claude_setting: Option<i64>, codex_setting: Option<i64>, window_of: &dyn Fn(&str) -> Option<i64>) -> Vec<AgentAdvice> {
    let mut out = Vec::new();
    for platform in ["claude", "codex"] {
        let Some(calls) = by_platform.get(platform).filter(|c| !c.is_empty()) else { continue };
        let setting = if platform == "claude" { claude_setting } else { codex_setting };
        let mut models: BTreeMap<&str, Vec<Call>> = BTreeMap::new();
        for call in calls {
            models.entry(call.model.as_str()).or_default().push(call.clone());
        }
        let mut compactions: Vec<Compaction> = Vec::new();
        if platform == "claude" {
            for model_calls in models.values() {
                // 元数据知道窗口就用它；否则带 [1m] 或见过超过 200K 的按 1M，其余按 Claude Code 默认的 200K
                let seen_long = model_calls.iter().any(|c| c.long || c.prompt > 200_000);
                let window = match model_calls.first().and_then(|c| window_of(&c.model)) {
                    // 带 [1m] 或超过 200K 的说明用的是长上下文：元数据给的是标准窗口时按 1M
                    Some(known) if seen_long => known.max(1_000_000),
                    Some(known) => known,
                    None if seen_long => 1_000_000,
                    None => 200_000,
                };
                let o = CompactOpts { min: 100_000, max: 1_000_000, step: 10_000, reserve: 33_000, window, current: setting.unwrap_or(0) };
                compactions.extend(compact(model_calls, o));
            }
            compactions.sort_by(|a, b| b.facts.calls.cmp(&a.facts.calls));
        } else {
            // Codex 按调用最多的模型查窗口；查不到就不设上限之外的限制，界面上写明「窗口未知」
            let current = setting.unwrap_or(0);
            let main = models.iter().max_by_key(|(_, c)| c.len()).map(|(m, _)| *m).unwrap_or_default();
            let window = window_of(main).unwrap_or(0);
            let o = CompactOpts { min: 50_000, max: 1_000_000.max(current), step: 10_000, reserve: 0, window, current };
            compactions.extend(compact(calls, o));
        }
        let cache = if platform == "claude" { cache_ttl(calls, None) } else { None };
        out.push(AgentAdvice { platform: platform.into(), calls: calls.len() as i64, compact_setting: setting, compactions, cache });
    }
    out
}

/// 读 Claude Code 现在的自动压缩窗口（settings.json 的 env）
pub fn claude_compact_setting(settings_json: &str) -> Option<i64> {
    let root: serde_json::Value = serde_json::from_str(settings_json).ok()?;
    let value = root.pointer("/env/CLAUDE_CODE_AUTO_COMPACT_WINDOW")?;
    value.as_i64().or_else(|| value.as_str()?.trim().parse().ok())
}

/// 读 Codex 现在的压缩阈值（config.toml 顶层 `model_auto_compact_token_limit`）
pub fn codex_compact_setting(config_toml: &str) -> Option<i64> {
    let doc = config_toml.parse::<toml_edit::DocumentMut>().ok()?;
    doc.get("model_auto_compact_token_limit")?.as_integer()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个会话：从 start 起每次长 grow，到 at 就压缩回 after，间隔 gap
    #[allow(clippy::too_many_arguments)]
    fn session(id: &str, t0: i64, n: usize, start: i64, grow: i64, at: i64, after: i64, gap_ms: i64, wrote_1h: bool) -> Vec<Call> {
        let mut out = Vec::new();
        let mut p = start;
        for i in 0..n {
            out.push(Call { time_ms: t0 + i as i64 * gap_ms, stream: id.into(), model: "claude-opus-5-5".into(), long: false, prompt: p, out: 800, wrote: grow, wrote_1h: if wrote_1h { grow } else { 0 } });
            p += grow;
            if at > 0 && p >= at {
                p = after;
            }
        }
        out
    }

    fn many(k: usize, f: impl Fn(&str, i64) -> Vec<Call>) -> Vec<Call> {
        (0..k).flat_map(|i| f(&format!("s{i}"), 1_790_000_000_000 + i as i64 * 86_400_000)).collect()
    }

    const CLAUDE: CompactOpts = CompactOpts { min: 100_000, max: 1_000_000, step: 10_000, reserve: 33_000, current: 0, window: 0 };

    #[test]
    fn long_sessions_go_smaller() {
        let cs = many(3, |id, t0| session(id, t0, 600, 30_000, 5_000, 967_000, 60_000, 30_000, false));
        let r = compact(&cs, CompactOpts { window: 1_000_000, current: 1_000_000, ..CLAUDE }).unwrap();
        assert!(r.best < 1_000_000 && (r.cost - r.low) / r.cost >= 0.2, "best {} 省 {:.0} / {:.0}", r.best, r.cost - r.low, r.cost);
        assert!(r.facts.compacts > 0);
        assert_eq!(r.facts.growth, 5_000);
    }

    #[test]
    fn short_sessions_keep_current() {
        let cs = many(5, |id, t0| session(id, t0, 30, 20_000, 2_000, 0, 0, 60_000, false));
        let r = compact(&cs, CompactOpts { window: 200_000, ..CLAUDE }).unwrap();
        assert_eq!((r.current, r.best), (200_000, 200_000));
        assert_eq!(r.low, r.cost);
    }

    #[test]
    fn needs_enough_calls() {
        assert!(compact(&session("s", 0, 20, 20_000, 2_000, 0, 0, 60_000, false), CLAUDE).is_none());
    }

    #[test]
    fn pauses_want_an_hour_and_quick_calls_five_minutes() {
        let paused = many(4, |id, t0| session(id, t0, 40, 30_000, 3_000, 0, 0, 20 * 60_000, false));
        let r = cache_ttl(&paused, None).unwrap();
        assert_eq!((r.current.as_str(), r.best.as_str()), ("5m", "1h"));
        assert!(r.low < r.cost && r.facts.pause > 0.99);
        assert!(r.current_inferred, "没有 1 小时写入的拆分，当前值是推断的");
        let quick = many(4, |id, t0| session(id, t0, 40, 30_000, 3_000, 0, 0, 20_000, true));
        let r = cache_ttl(&quick, None).unwrap();
        assert_eq!((r.current.as_str(), r.best.as_str()), ("1h", "5m"));
        assert!(!r.current_inferred);
    }

    #[test]
    fn setting_capped_by_window() {
        let cs = many(3, |id, t0| session(id, t0, 400, 30_000, 1_000, 167_000, 57_000, 30_000, false));
        let r = compact(&cs, CompactOpts { window: 200_000, current: 500_000, ..CLAUDE }).unwrap();
        assert_eq!((r.current, r.window), (200_000, 200_000));
        assert!(r.curve.last().unwrap().value <= 200_000);
    }

    #[test]
    fn curve_counts_fewer_compactions_as_the_window_grows() {
        let cs = many(3, |id, t0| session(id, t0, 600, 30_000, 5_000, 967_000, 60_000, 30_000, false));
        let r = compact(&cs, CompactOpts { window: 1_000_000, current: 1_000_000, ..CLAUDE }).unwrap();
        let first = r.curve.first().unwrap().compacts;
        let last = r.curve.last().unwrap().compacts;
        assert!(first > last, "{first} > {last}");
    }

    #[test]
    fn unknown_window_caps_nothing() {
        let cs = many(2, |id, t0| session(id, t0, 80, 20_000, 5_000, 0, 0, 20_000, false));
        let r = compact(&cs, CompactOpts { min: 50_000, max: 1_000_000, step: 10_000, ..Default::default() }).unwrap();
        assert_eq!((r.curve.last().unwrap().value, r.current, r.window), (1_000_000, 1_000_000, 0));
    }

    #[test]
    fn weights_and_settings_readers() {
        assert_eq!(weights_for("anthropic/claude-sonnet-4").out, 5.0);
        assert_eq!(weights_for("gpt-5").out, 8.0);
        assert_eq!(claude_compact_setting(r#"{"env":{"CLAUDE_CODE_AUTO_COMPACT_WINDOW":"400000"}}"#), Some(400_000));
        assert_eq!(claude_compact_setting(r#"{"env":{}}"#), None);
        assert_eq!(codex_compact_setting("model_auto_compact_token_limit = 180000\n"), Some(180_000));
        assert_eq!(codex_compact_setting("model = \"x\"\n"), None);
    }

    #[test]
    fn calls_come_from_known_successful_rows_with_a_session() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE requests (platform TEXT, session_id TEXT, ts INTEGER, model TEXT, input_tokens INTEGER, output_tokens INTEGER,
               cache_read_tokens INTEGER, cache_write_tokens INTEGER, input_known INTEGER DEFAULT 1, output_known INTEGER DEFAULT 1,
               cache_read_known INTEGER DEFAULT 1, cache_write_known INTEGER DEFAULT 1, status_code INTEGER);
             INSERT INTO requests(platform, session_id, ts, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, status_code) VALUES
               ('claude','a',10,'claude-opus-5-5[1m]',100,10,1000,50,200),
               ('claude','a',20,'claude-opus-5-5',0,0,0,0,200),
               ('claude',NULL,30,'x',100,1,1,1,NULL),
               ('claude','b',40,'x',100,1,1,1,500),
               ('codex','c',50,'gpt-5',100,1,1,1,NULL);
             INSERT INTO requests(platform, session_id, ts, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, input_known) VALUES ('claude','d',60,'x',0,1,1,1,0);",
        )
        .unwrap();
        let calls = calls_from_db(&conn, "claude", 0).unwrap();
        assert_eq!(calls.len(), 1, "提示词为 0、没会话、失败、未知用量都不要");
        assert_eq!((calls[0].prompt, calls[0].model.as_str(), calls[0].long, calls[0].stream.as_str()), (1_150, "claude-opus-5-5", true, "claude/a"));
    }

    #[test]
    fn advise_covers_both_agents_and_skips_thin_ones() {
        let mut by = BTreeMap::new();
        by.insert("claude".to_string(), many(3, |id, t0| session(id, t0, 600, 30_000, 5_000, 967_000, 60_000, 30_000, false)));
        by.insert("codex".to_string(), session("x", 0, 10, 20_000, 1_000, 0, 0, 1_000, false));
        let advice = advise(&by, None, None, &|_| None);
        assert_eq!(advice.len(), 2);
        assert_eq!(advice[0].compactions.len(), 1);
        assert!(advice[0].cache.is_some());
        assert!(advice[1].compactions.is_empty(), "Codex 只有 10 次调用，不给建议");
    }
}
