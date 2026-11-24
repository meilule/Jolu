//! 共享数据契约（P1-4）。
//!
//! ## 为什么需要这个模块
//!
//! 用量、会话、路由、灵动岛四个入口各自定义了一套「数值 + 未知」的表达：
//!
//! | 入口 | 结构 | 未知怎么表达 |
//! |---|---|---|
//! | 用量 | `TokenSplit` / `TokenCoverage` / `ProviderRow.known_tokens` | `Option<i64>` + `*_unknown` 计数 |
//! | 会话 | `SessionTokens` | `Option<i64>` + `state: "known"\|"partial"\|"missing"` |
//! | 路由 | `GatewaySession.tokens` | 复用 `SessionTokens` |
//! | 灵动岛 | 前端各自算 | 混用 `null` / `0` |
//!
//! 同一个语义（「这个数的来源是什么、覆盖了多少条、可信到什么程度」）写了四遍，
//! 结果就是对同一个请求，四个页面可能给出不同的判断。这个模块把那份语义定成一份。
//!
//! ## 设计约束
//!
//! **不改既有序列化形状**。四处的 DTO 已经在前端用着，贸然换结构会让所有页面同时炸。
//! 所以这里只提供**计算与判定函数**（纯逻辑，零 DTO 改动），各处按需调用；
//! 等将来某个入口重做时再考虑把字段直接换成这个类型。
//!
//! 这也是计划 §5「依赖和可并行关系」的要求：P1-4 应先于 P4 的跨组件联动，
//! 但不能把 P2/P3 的既有界面推翻重来。

use serde::Serialize;

/// 一份数值的覆盖情况：已知多少、未知多少、总量是多少。
///
/// 关键约束：`known` 为 0 时 `total` 必须是 `None`。**未知不等于零**——
/// 这是四个入口最容易出分歧的地方（灵动岛曾经把「拿不到」画成 0）。
///
/// 字段私有，只能经 [`Coverage::of`] / [`Coverage::from_counts`] 构造，两者都保证这条约束；
/// 读取用同名方法。序列化形状不变（`total` / `known` / `unknown`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coverage {
    /// 已知部分的合计；一条都没有时为 None
    total: Option<i64>,
    /// 有值的条数
    known: i64,
    /// 缺值的条数
    unknown: i64,
}

impl Coverage {
    /// 由已经数好的条数与另算的总量构造（覆盖与总量来自不同查询时用）。
    ///
    /// 违反约束的输入会被纠正而不是原样保存：负的条数按 0 计；`known` 为 0 时总量一律未知；
    /// 负的总量当未知。
    pub fn from_counts(known: i64, unknown: i64, total: Option<i64>) -> Self {
        let known = known.max(0);
        Self {
            total: total.filter(|value| known > 0 && *value >= 0),
            known,
            unknown: unknown.max(0),
        }
    }

    /// 已知部分的合计；一条都没有时为 None
    pub fn total(&self) -> Option<i64> {
        self.total
    }

    /// 有值的条数
    pub fn known(&self) -> i64 {
        self.known
    }

    /// 缺值的条数
    pub fn unknown(&self) -> i64 {
        self.unknown
    }

    /// 从一批 `Option<i64>` 累加。
    ///
    /// 全部为 None 时 `total` 是 None（不是 0）——这正是四处需要统一的那条规则。
    pub fn of(values: impl IntoIterator<Item = Option<i64>>) -> Self {
        let mut total = 0i64;
        let mut known = 0i64;
        let mut unknown = 0i64;
        for value in values {
            match value {
                Some(v) => {
                    // 负数在用量里没有意义，当未知处理而不是让合计变小
                    if v < 0 {
                        unknown += 1;
                    } else {
                        total += v;
                        known += 1;
                    }
                }
                None => unknown += 1,
            }
        }
        Self {
            total: (known > 0).then_some(total),
            known,
            unknown,
        }
    }

    /// 是否全部都有值
    pub fn complete(&self) -> bool {
        self.state() == "known"
    }

    /// 展示用的状态：与 `SessionTokens.state` / `TokenCoverage` 的三态对齐。
    ///
    /// - `known`：全部有值
    /// - `partial`：有值但缺一部分
    /// - `missing`：一条都没有
    pub fn state(&self) -> &'static str {
        match (self.known > 0, self.unknown == 0) {
            (true, true) => "known",
            (true, false) => "partial",
            (false, _) => "missing",
        }
    }
}

/// 数据从哪来（P1-1 / P1-4 的「来源」）。
///
/// 三种来源的口径不同，不能互相冒充：
/// - `official`：平台官方接口返回（额度、余额）；
/// - `local_record`：Jolu 从本机工具的日志 / 数据库采到的记录；
/// - `gateway_log`：Jolu 自己的网关在转发时写下的记录。
///
/// 一份统计同时用到本地记录和网关记录时是 `mixed`（用量表里两种来源的行共存，见
/// `requests.source`：以 `gateway` 开头的是网关写的，其余都是本机采集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Official,
    LocalRecord,
    GatewayLog,
    Mixed,
}

/// 数值的单位。同名指标在不同入口单位不同（请求数 vs Token 数 vs 百分比），显式写出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Tokens,
    Requests,
    Percent,
}

/// 覆盖程度：所声明的时间范围是否被完整覆盖。
///
/// - `full`：能证明范围内没有缺数据（例如流水没有被滚动删除过这一段）；
/// - `partial`：已知有缺口（流水被截断、记录早于保留范围）；
/// - `as_recorded`：只代表 Jolu 已经采到的部分，无法对照源端证明完整（本机采集都属于这一档——
///   源端可能还没被扫到，也可能已经清理了历史）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Full,
    Partial,
    AsRecorded,
}

/// 一份统计的元数据：来源、采集时间、数据时间、覆盖范围、单位、覆盖程度。
///
/// 这是 P1-1 原设计里 `DataOrigin` / `Measured` 的落地形态。作为**附加字段**挂到各入口的
/// 既有 DTO 上（不替换原字段），前端可以逐步改用，旧前端忽略它也不会出错。
///
/// 时间全部是 UTC 毫秒；「今天」这类自然日边界由调用方按本地时区算好再传进来，
/// 这里不再做时区换算——四个入口共用同一组边界，才不会出现「同一个今天两种算法」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Provenance {
    pub origin: Origin,
    pub unit: Unit,
    /// Jolu 生成这份统计的时刻
    pub collected_at: i64,
    /// 数据本身最新到哪个时刻：范围内最后一条记录的时间；范围内没有记录时为 None
    pub data_at: Option<i64>,
    /// 声明覆盖的范围起点（含）
    pub window_start: i64,
    /// 声明覆盖的范围终点（不含）
    pub window_end: i64,
    /// 现存最早一条可用记录的时间；比 `window_start` 晚时说明范围前段可能已被清理或从未采到
    pub earliest_available: Option<i64>,
    pub completeness: Completeness,
}

impl Provenance {
    /// 按「来源中是否有本机记录 / 网关记录」判定来源。两者都没有（范围内无记录）时按
    /// `fallback` 处理——空范围仍然要说明「本来该从哪取」，不能写成 mixed。
    pub fn origin_of(has_local: bool, has_gateway: bool, fallback: Origin) -> Origin {
        match (has_local, has_gateway) {
            (true, true) => Origin::Mixed,
            (true, false) => Origin::LocalRecord,
            (false, true) => Origin::GatewayLog,
            (false, false) => fallback,
        }
    }

    /// 本机记录类统计的覆盖程度：只能是 `as_recorded`，除非已知有缺口。
    ///
    /// `earliest_available` 比范围起点晚**不**单独判为缺口：那也可能只是那段时间确实没用。
    /// 只有调用方明确知道被截断（`truncated`）时才是 `partial`。
    pub fn recorded_completeness(truncated: bool) -> Completeness {
        if truncated { Completeness::Partial } else { Completeness::AsRecorded }
    }
}

/// 用量表（`requests`，别名 `r`）上一段范围的来源元数据。
///
/// - `range_where` / `range_args`：这份统计实际使用的过滤条件（含时间范围），用来取数据时间与来源；
/// - `scope_where` / `scope_args`：同样的过滤条件但**不含时间范围**，用来取「现存最早一条」。
///
/// 两组条件都由调用方给出，保证元数据与统计本身用的是同一批行，不另起一套口径。
#[allow(clippy::too_many_arguments)]
pub fn requests_provenance(
    conn: &rusqlite::Connection,
    window_start: i64,
    window_end: i64,
    range_where: &str,
    range_args: &[rusqlite::types::Value],
    scope_where: &str,
    scope_args: &[rusqlite::types::Value],
    collected_at: i64,
) -> rusqlite::Result<Provenance> {
    let (data_at, has_local, has_gateway): (Option<i64>, i64, i64) = conn.query_row(
        &format!(
            "SELECT MAX(r.ts),
                    COALESCE(MAX(CASE WHEN r.source LIKE 'gateway%' THEN 0 ELSE 1 END), 0),
                    COALESCE(MAX(CASE WHEN r.source LIKE 'gateway%' THEN 1 ELSE 0 END), 0)
             FROM requests r WHERE {range_where}"
        ),
        rusqlite::params_from_iter(range_args.iter()),
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let earliest_available: Option<i64> = conn.query_row(
        &format!("SELECT MIN(r.ts) FROM requests r WHERE {scope_where}"),
        rusqlite::params_from_iter(scope_args.iter()),
        |row| row.get(0),
    )?;
    Ok(Provenance {
        origin: Provenance::origin_of(has_local == 1, has_gateway == 1, Origin::LocalRecord),
        unit: Unit::Tokens,
        collected_at,
        data_at,
        window_start,
        window_end,
        earliest_available,
        completeness: Provenance::recorded_completeness(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requests_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE requests (ts INTEGER, source TEXT, platform TEXT);").unwrap();
        conn
    }

    fn provenance_for(conn: &rusqlite::Connection, start: i64, end: i64, platform: Option<&str>, at: i64) -> Provenance {
        use rusqlite::types::Value;
        let (scope, scope_args) = match platform {
            Some(p) => ("r.platform = ?".to_string(), vec![Value::Text(p.into())]),
            None => ("1 = 1".to_string(), vec![]),
        };
        let mut range_args = vec![Value::Integer(start), Value::Integer(end)];
        range_args.extend(scope_args.clone());
        let range = format!("r.ts >= ? AND r.ts < ? AND {scope}");
        requests_provenance(conn, start, end, &range, &range_args, &scope, &scope_args, at).unwrap()
    }

    #[test]
    fn provenance_of_an_empty_window_keeps_the_fallback_origin_and_no_data_time() {
        let conn = requests_db();
        let p = provenance_for(&conn, 100, 200, None, 999);
        assert_eq!(p.origin, Origin::LocalRecord, "空范围也要说明本该从哪取，不能写成 mixed");
        assert_eq!(p.data_at, None, "没有记录时数据时间未知，不能拿采集时间冒充");
        assert_eq!(p.earliest_available, None);
        assert_eq!((p.window_start, p.window_end, p.collected_at), (100, 200, 999));
        assert_eq!(p.completeness, Completeness::AsRecorded);
    }

    #[test]
    fn provenance_separates_local_gateway_and_mixed_records() {
        let conn = requests_db();
        conn.execute_batch(
            "INSERT INTO requests VALUES (50, 'native:zcode:main', 'zcode');
             INSERT INTO requests VALUES (120, 'native:zcode:main', 'zcode');
             INSERT INTO requests VALUES (150, 'gateway-subscription', 'qoder');
             INSERT INTO requests VALUES (180, 'claude-native', 'claude');",
        )
        .unwrap();
        let zcode = provenance_for(&conn, 100, 200, Some("zcode"), 1);
        assert_eq!(zcode.origin, Origin::LocalRecord);
        assert_eq!(zcode.data_at, Some(120), "数据时间是范围内最后一条，不是范围外的");
        assert_eq!(zcode.earliest_available, Some(50), "最早可用记录不受范围限制");

        let gateway = provenance_for(&conn, 100, 200, Some("qoder"), 1);
        assert_eq!(gateway.origin, Origin::GatewayLog);

        let all = provenance_for(&conn, 100, 200, None, 1);
        assert_eq!(all.origin, Origin::Mixed, "本机记录与网关记录共存时不能只报一种");
        assert_eq!(all.data_at, Some(180));
    }

    #[test]
    fn provenance_serializes_snake_case_tags() {
        let p = Provenance {
            origin: Origin::GatewayLog,
            unit: Unit::Requests,
            collected_at: 3,
            data_at: None,
            window_start: 1,
            window_end: 2,
            earliest_available: Some(0),
            completeness: Completeness::Partial,
        };
        let json = serde_json::to_value(p).unwrap();
        assert_eq!(json["origin"], "gateway_log");
        assert_eq!(json["unit"], "requests");
        assert_eq!(json["completeness"], "partial");
        assert!(json["data_at"].is_null());
    }

    #[test]
    fn all_unknown_reports_missing_not_zero() {
        let c = Coverage::of([None, None, None]);
        assert_eq!(c.total(), None, "全部未知时不能给 0");
        assert_eq!((c.known(), c.unknown()), (0, 3));
        assert!(!c.complete());
        assert_eq!(c.state(), "missing");
    }

    #[test]
    fn partial_coverage_keeps_the_known_subtotal() {
        // 现场：ZCode 模型有 4987 条记录，29 条取消/失败没有 Token
        let c = Coverage::of([Some(1000), Some(2000), None]);
        assert_eq!(c.total(), Some(3000), "已知部分是真的，不能因一条未知就丢");
        assert_eq!((c.known(), c.unknown()), (2, 1));
        assert!(!c.complete());
        assert_eq!(c.state(), "partial");
    }

    #[test]
    fn full_coverage_has_no_ratio_note() {
        let c = Coverage::of([Some(10), Some(20)]);
        assert_eq!(c.total(), Some(30));
        assert!(c.complete());
        assert_eq!(c.state(), "known");
    }

    #[test]
    fn negative_values_are_unknown_not_subtracted() {
        // 用量里的负数只可能是坏数据；当未知处理，不能让合计变小
        let c = Coverage::of([Some(100), Some(-50)]);
        assert_eq!(c.total(), Some(100));
        assert_eq!((c.known(), c.unknown()), (1, 1));
        assert_eq!(c.state(), "partial");
    }

    #[test]
    fn from_counts_keeps_a_separately_computed_total() {
        // 覆盖与总量分开算时（灵动岛就是：`today_fresh_tokens` 有各平台口径）
        let c = Coverage::from_counts(1, 1, Some(10));
        assert_eq!(c.total(), Some(10));
        assert_eq!((c.known(), c.unknown()), (1, 1));
        assert_eq!(c.state(), "partial");
    }

    /// C-6：违反约束的输入在构造时就被纠正，不能造出「零条已知却有总量」。
    #[test]
    fn from_counts_cannot_build_a_total_without_known_rows() {
        let c = Coverage::from_counts(0, 3, Some(500));
        assert_eq!(c.total(), None, "一条已知都没有，总量不能是数");
        assert_eq!(c.state(), "missing");

        let c = Coverage::from_counts(2, 0, Some(-1));
        assert_eq!(c.total(), None, "负的总量当未知");

        let c = Coverage::from_counts(-1, -1, None);
        assert_eq!((c.known(), c.unknown()), (0, 0), "负的条数按 0 计");
    }

    #[test]
    fn serialized_shape_is_unchanged() {
        // 字段私有化不能改前端读到的形状
        let json = serde_json::to_value(Coverage::from_counts(1, 2, Some(7))).unwrap();
        assert_eq!(json, serde_json::json!({"total": 7, "known": 1, "unknown": 2}));
    }

    #[test]
    fn empty_input_is_missing() {
        let c = Coverage::of(std::iter::empty());
        assert_eq!(c.total(), None);
        assert_eq!(c.state(), "missing");
    }

}
