//! 模型目录（从 `lib.rs::gateway_catalog` 抽出）。
//!
//! 目录 = 连接表中已接入的模型 + `group/<slug>` 路由分组，再并入 models.dev 的
//! 上下文窗口 / 最大输出 / 显示名。
//!
//! 抽层时把「models.dev 元数据从哪来」变成参数：命令侧负责触发后台刷新并取缓存，
//! 这里只管合并。这样 CLI 也能用同一套合并逻辑，而不用启动 Tauri。

use rusqlite::Connection;

use crate::gateway::catalog::CatalogModel;
use crate::gateway::models_dev::ModelMeta;

/// 合并用的元数据查询：给模型名，返回可选元数据。
pub type MetaLookup<'a> = dyn Fn(&str) -> Option<ModelMeta> + 'a;

/// 列出模型目录，并并入元数据。
///
/// - 分组以 `group/<分组slug>` 进目录供接线与 Profile 选择；
/// - 元数据按模型 id 并入（models.dev 的 id 带厂商前缀，`lookup_meta` 做末段忽略大小写匹配）；
/// - 元数据里 0 或空串视为「没给」，不覆盖成 0 上下文窗口；
/// - **来源级上限覆盖最后应用**（`model_overrides`）：来源实测值优先于 models.dev——
///   覆盖只对带 platform 的连接条目生效，路由分组（无平台归属）不参与。
pub fn list(conn: &Connection, meta: &MetaLookup<'_>) -> Result<Vec<CatalogModel>, String> {
    let mut out = crate::gateway::catalog::list(conn).map_err(|error| error.to_string())?;
    // 路由分组也是可选的模型 id（`group/<分组slug>`），进目录供接线与 Profile 选择
    if let Ok(groups) = crate::gateway::routing_groups::list(conn) {
        for group in groups {
            out.push(CatalogModel {
                id: format!("group/{}", group.slug),
                source: group.name,
                slug: "group".into(),
                model: group.slug,
                kind: "group".into(),
                platform: None,
                status: "connected".into(),
                context_window: None,
                max_output: None,
                label: None,
            });
        }
    }
    for entry in out.iter_mut() {
        if let Some(meta) = meta(&entry.model) {
            merge_meta(entry, &meta);
        }
        // models.dev 合并完再应用来源覆盖：覆盖优先（WorkBuddy deepseek-v4.1-flash 案例）
        if let Some(platform) = entry.platform.as_deref() {
            if let Some(over) = crate::gateway::model_overrides::lookup(platform, &entry.model) {
                crate::gateway::model_overrides::apply(entry, over);
            }
        }
    }
    Ok(out)
}

/// 把元数据并进目录条目。
///
/// 0 与空串表示「来源没给」：不能当成真实值覆盖（显示成 0 上下文窗口比留空更误导）。
fn merge_meta(entry: &mut CatalogModel, meta: &ModelMeta) {
    entry.context_window = Some(meta.context_window).filter(|value| *value > 0);
    entry.max_output = Some(meta.max_output).filter(|value| *value > 0);
    entry.label = Some(meta.label.clone()).filter(|label| !label.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (Connection, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "jolu-catalog-svc-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        (crate::db::open(&path).unwrap(), path)
    }

    #[test]
    fn no_meta_lookup_leaves_catalog_untouched() {
        let (conn, path) = database();
        let without = list(&conn, &|_| None).unwrap();
        assert_eq!(without.len(), 0, "空库没有模型");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn meta_merge_skips_zero_and_empty_values() {
        // 纯逻辑验证：把 0 / 空串当作「没给」，不能覆盖成 0 上下文窗口
        let mut entry = CatalogModel {
            id: "src/glm-5.3".into(),
            source: "来源".into(),
            slug: "src".into(),
            model: "glm-5.3".into(),
            kind: "api".into(),
            platform: Some("workbuddy".into()),
            status: "connected".into(),
            context_window: None,
            max_output: None,
            label: None,
        };
        let meta = ModelMeta { context_window: 0, max_output: 0, label: String::new(), description: String::new() };
        merge_meta(&mut entry, &meta);
        assert_eq!(entry.context_window, None, "0 当作没给");
        assert_eq!(entry.max_output, None);
        assert_eq!(entry.label, None, "空串当作没给");
    }

    #[test]
    fn meta_merge_applies_real_values() {
        let mut entry = CatalogModel {
            id: "src/glm-5.3".into(),
            source: "来源".into(),
            slug: "src".into(),
            model: "glm-5.3".into(),
            kind: "api".into(),
            platform: Some("workbuddy".into()),
            status: "connected".into(),
            context_window: None,
            max_output: None,
            label: None,
        };
        let meta = ModelMeta {
            context_window: 128_000,
            max_output: 8_192,
            label: "GLM-5.3".into(),
            description: String::new(),
        };
        merge_meta(&mut entry, &meta);
        assert_eq!(entry.context_window, Some(128_000));
        assert_eq!(entry.max_output, Some(8_192));
        assert_eq!(entry.label.as_deref(), Some("GLM-5.3"));
    }
}
