//! 来源级模型上限覆盖表：来源自己实测的上限优先于 models.dev 的记录。
//!
//! 管什么：models.dev 与来源实际行为不一致、且不一致会影响用户判断（上下文
//! 标注、200K 判定、接线提示）的模型。
//!
//! 不管什么：
//! - 没有实测依据的偏差不登记——宁可显示 models.dev 的值，不按猜测改；
//! - 来源动态自报的上限不走这里（那类走 `models_json` / `cached_models` 的正常链路）；
//! - 连接级别的一次性偏差不改全局表（那是该修数据源的问题）。
//!
//! 为什么按 `(platform, model)` 双键：同一个模型名在不同来源上限可能真的不同
//! （models.dev 记录的是官方 API 的值，订阅来源可能是自己的裁剪版）；
//! 只按 model 名覆盖会把官方 API 的真值也改掉。
//!
//! 参照 magpie：WorkBuddy 插件 0.2.26（magpie-community/plugins `7e01fa81`）
//! 自报 `deepseek-v4.1-flash` 回复上限 128000，修正 models.dev 的 384000
//! （magpie commit `0f04dca`，用户 sayagoodbyeplz 报告）。

use crate::gateway::catalog::CatalogModel;

/// 一条覆盖：`None` 表示「这一维以 models.dev 为准」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelOverride {
    pub context_window: Option<i64>,
    pub max_output: Option<i64>,
}

/// 覆盖表。键：`(platform, model)`（model 忽略大小写比较）。
///
/// 每条都要有实测依据；登记格式：`(platform, model, 覆盖, 依据一句话)`。
const OVERRIDES: &[(&str, &str, ModelOverride, &str)] = &[
    (
        "workbuddy",
        "deepseek-v4.1-flash",
        ModelOverride { context_window: None, max_output: Some(128_000) },
        "WorkBuddy 实际回复 128000；models.dev 的 384000 是 DeepSeek 官方 API 的值（magpie 0f04dca）",
    ),
];

/// 查一条覆盖。model 忽略大小写；platform 精确匹配（平台 id 是小写约定值）。
pub(crate) fn lookup(platform: &str, model: &str) -> Option<ModelOverride> {
    let wanted = model.trim();
    OVERRIDES
        .iter()
        .find(|(entry_platform, entry_model, _, _)| *entry_platform == platform && entry_model.eq_ignore_ascii_case(wanted))
        .map(|(_, _, over, _)| *over)
}

/// 把覆盖应用到一个目录条目。
///
/// 与 models.dev 元数据的合并顺序：先并 models.dev（`service::gateway::catalog`），
/// 再应用本表——覆盖优先。`None` 的维度不动。
pub(crate) fn apply(entry: &mut CatalogModel, over: ModelOverride) {
    if let Some(window) = over.context_window {
        entry.context_window = Some(window);
    }
    if let Some(output) = over.max_output {
        entry.max_output = Some(output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbuddy_deepseek_is_capped_at_128k_and_others_are_not() {
        let over = lookup("workbuddy", "deepseek-v4.1-flash").expect("登记过的组合必须命中");
        assert_eq!(over.max_output, Some(128_000));
        assert_eq!(over.context_window, None, "上下文这一维没有实测偏差，不动");

        // 大小写不敏感
        assert!(lookup("workbuddy", "DeepSeek-V4.1-Flash").is_some());
        // 同名模型在其他平台不覆盖：models.dev 对官方 API 的记录可能是真值
        assert!(lookup("custom", "deepseek-v4.1-flash").is_none());
        assert!(lookup("zcode", "deepseek-v4.1-flash").is_none());
        assert!(lookup("workbuddy", "glm-5.3").is_none(), "没登记的模型不命中");
    }

    #[test]
    fn apply_touches_only_the_declared_dimension() {
        let mut entry = CatalogModel {
            id: "src/deepseek-v4.1-flash".into(),
            source: "来源".into(),
            slug: "src".into(),
            model: "deepseek-v4.1-flash".into(),
            kind: "api".into(),
            platform: Some("workbuddy".into()),
            status: "connected".into(),
            context_window: Some(384_000),
            max_output: Some(384_000),
            label: None,
        };
        apply(&mut entry, lookup("workbuddy", "deepseek-v4.1-flash").unwrap());
        assert_eq!(entry.max_output, Some(128_000));
        assert_eq!(entry.context_window, Some(384_000), "None 的维度保持 models.dev 的值");
    }
}
