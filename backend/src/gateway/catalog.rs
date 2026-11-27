//! 模型清单（只读）：把模型池里每个来源展开成 `<slug>/<model>`。
//! 只读数据库缓存，不打上游；拉取新清单仍由 `/v1/models` 负责。

use rusqlite::Connection;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub source: String,
    pub slug: String,
    pub model: String,
    pub kind: String,
    /// 所属平台；来源级上限覆盖（model_overrides）按它 + model 匹配。
    /// 目录页不展示，序列化省略；旧消费方不受影响。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub status: String,
    /// models.dev 元数据（P4.2）：缓存缺失时为 None，前端不展示该列
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<CatalogModel>> {
    let order = crate::gateway::groups::source_order(conn);
    let mut stmt = conn.prepare(
        "SELECT id, slug, name, platform, kind, model, status, models_json
         FROM connections WHERE serve_enabled = 1 AND slug IS NOT NULL AND slug != ''
         ORDER BY created_ms ASC",
    )?;
    let mut rows: Vec<(String, String, String, String, String, Option<String>, String, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.sort_by_key(|row| order.get(&row.0).copied().unwrap_or((i64::MAX, i64::MAX)));
    let mut out = Vec::new();
    for (id, slug, source, platform, kind, default_model, status, models_json) in rows {
        let models: Vec<String> = if kind == "api" {
            let cached = models_json
                .as_deref()
                .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
                .unwrap_or_default();
            if cached.is_empty() {
                default_model.into_iter().collect()
            } else {
                cached
            }
        } else if platform == "workbuddy" {
            crate::gateway::adapter::workbuddy::cached_models(conn, &id).unwrap_or_default()
        } else if platform == "qoder" {
            crate::gateway::adapter::qoder::cached_models(conn, &id).unwrap_or_default()
        } else if platform == "trae" {
            crate::gateway::adapter::trae_models::cached_models(conn, &id).unwrap_or_default()
        } else if platform == "zcode" || platform == "grok" || platform == "gemini" {
            // 这三个订阅平台的清单由「拉取模型清单 / 上游 /v1/models」写成纯数组
            // （刷新命令与各自适配器都不自管缓存形状）；没有就如实为空，不拿内置清单冒充
            models_json
                .as_deref()
                .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
                .unwrap_or_default()
        } else {
            crate::gateway::server::builtin_subscription_models(&platform)
                .iter()
                .map(|model| model.to_string())
                .collect()
        };
        // Qoder 的对外模型 id 就是官方 `display_name`（`cached_models` 已返回人读名），
        // 转发层把 display_name 归一回内部 key——catalog 不再需要 key→名字的 label 映射。
        for model in models {
            out.push(CatalogModel {
                id: format!("{slug}/{model}"),
                source: source.clone(),
                slug: slug.clone(),
                model,
                kind: kind.clone(),
                platform: Some(platform.clone()),
                status: status.clone(),
                context_window: None,
                max_output: None,
                label: None,
            });
        }
    }
    Ok(out)
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn trae_catalog_reads_fresh_bound_models_without_using_legacy_cache() {
        use sha2::{Digest, Sha256};
        let deps = crate::proxy::test_support::bare_deps();
        let secret = crate::provider_browser_auth::seal("trae-cache", &crate::provider_browser_auth::Credential::Trae {
            edition: "cn".into(), session: crate::trae_browser_auth::Session {
                token: "fixture".into(), host: "https://api.trae.cn".into(), expires_at: "2099-01-01T00:00:00Z".into(), account: None, user_id: "cache-user".into(),
            },
        }).unwrap();
        let cache = serde_json::json!({"credential_hash":format!("{:x}",Sha256::digest(secret.as_bytes())),"edition":"cn","fetched_ms":chrono::Utc::now().timestamp_millis(),"models":["account-model"]}).to_string();
        let conn = deps.db.lock().unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled,models_json) VALUES('trae-cache','trae','auth','Trae',?1,1,'trae-cache',1,?2)", rusqlite::params![secret,cache]).unwrap();
        assert_eq!(super::list(&conn).unwrap().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["trae-cache/account-model"]);
        conn.execute("UPDATE connections SET models_json='[\"unverified\"]' WHERE id='trae-cache'", []).unwrap();
        assert!(super::list(&conn).unwrap().is_empty());
    }
    #[test]
    fn subscription_catalog_uses_qoder_builtin_but_never_unverified_trae_model() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        let secret = crate::provider_browser_auth::seal("qoder-source", &crate::provider_browser_auth::Credential::Qoder {
            edition: "cn".into(),
            session: crate::qoder_desktop::Session {
                token: "fixture-token".into(), refresh_token: None,
                expires_at: Some("2099-01-01T00:00:00Z".into()),
                account: None, user_id: Some("fixture-user".into()), machine_id: None,
            },
        }).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled) VALUES('qoder-source','qoder','auth','Qoder',?1,1,'qoder-source',1)", [secret]).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,created_ms,slug,serve_enabled,models_json) VALUES('trae-source','trae','auth','Trae',2,'trae-source',1,'[\"unverified-cache\"]')", []).unwrap();
        let list = super::list(&conn).unwrap();
        // 从未读到账号列表：Qoder 国内版内置清单（2026-11-14 同步的官方阵容快照）
        let qoder_ids: Vec<String> = list.iter().filter(|v| v.slug == "qoder-source").map(|v| v.id.clone()).collect();
        assert_eq!(qoder_ids, vec![
            "qoder-source/fast", "qoder-source/qmodel_latest", "qoder-source/qmodel_38max", "qoder-source/qfmodel",
            "qoder-source/dmodel", "qoder-source/dfmodel", "qoder-source/gmodel", "qoder-source/gfmodel",
            "qoder-source/kmodel_latest", "qoder-source/kmodel", "qoder-source/jmodel", "qoder-source/mmodel",
        ]);
        conn.execute("UPDATE connections SET model='explicit-model' WHERE id='trae-source'", []).unwrap();
        let list = super::list(&conn).unwrap();
        assert!(!list.iter().any(|v| v.id == "trae-source/explicit-model"), "无授权不能退回手填模型冒充可用来源");
        let trae_secret = crate::provider_browser_auth::seal("trae-source", &crate::provider_browser_auth::Credential::Trae {
            edition: "cn".into(), session: crate::trae_browser_auth::Session {
                token: "fixture".into(), host: "https://api.trae.cn".into(), expires_at: "2099-01-01T00:00:00Z".into(), account: None, user_id: "fixture-user".into(),
            },
        }).unwrap();
        conn.execute("UPDATE connections SET secret=?1 WHERE id='trae-source'", [trae_secret]).unwrap();
        let list = super::list(&conn).unwrap();
        assert!(!list.iter().any(|v| v.id == "trae-source/explicit-model"), "仅有授权和手填 ID 不能替代账号目录路由");
        assert!(!list.iter().any(|v| v.model == "unverified-cache"));
    }
}

/// 合并用的元数据查询：给模型名，返回可选元数据。
pub type MetaLookup<'a> = dyn Fn(&str) -> Option<crate::gateway::models_dev::ModelMeta> + 'a;

/// 列出模型目录，并并入 `models.dev` 元数据与来源级上限覆盖。
pub fn list_with_meta(conn: &Connection, meta: &MetaLookup<'_>) -> Result<Vec<CatalogModel>, String> {
    let mut out = list(conn).map_err(|error| error.to_string())?;
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
        if let Some(platform) = entry.platform.as_deref() {
            if let Some(over) = crate::gateway::model_overrides::lookup(platform, &entry.model) {
                crate::gateway::model_overrides::apply(entry, over);
            }
        }
    }
    Ok(out)
}

fn merge_meta(entry: &mut CatalogModel, meta: &crate::gateway::models_dev::ModelMeta) {
    entry.context_window = Some(meta.context_window).filter(|value| *value > 0);
    entry.max_output = Some(meta.max_output).filter(|value| *value > 0);
    entry.label = Some(meta.label.clone()).filter(|label| !label.is_empty());
}

#[cfg(test)]
mod meta_merge_tests {
    use super::*;
    use crate::gateway::models_dev::ModelMeta;

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
        let without = list_with_meta(&conn, &|_| None).unwrap();
        assert_eq!(without.len(), 0, "空库没有模型");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn meta_merge_skips_zero_and_empty_values() {
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
            context_window: 0,
            max_output: 0,
            label: String::new(),
            description: String::new(),
            ..Default::default()
        };
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
            ..Default::default()
        };
        merge_meta(&mut entry, &meta);
        assert_eq!(entry.context_window, Some(128_000));
        assert_eq!(entry.max_output, Some(8_192));
        assert_eq!(entry.label.as_deref(), Some("GLM-5.3"));
    }
}
