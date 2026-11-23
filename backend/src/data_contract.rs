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

#[cfg(test)]
mod tests {
    use super::*;

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
