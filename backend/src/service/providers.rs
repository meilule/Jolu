//! `jolu provider list|add|rm`（5.5）：命令行加 API Key 连接。
//!
//! 与桌面端「添加连接」同一条路：`connections::prepare_new` 归一与校验 → `quota::validate_connection`
//! 真的拿 Key 连一次上游 → 事务里 `connections::add`。只收 API Key：官方订阅的登录凭据要从本机
//! 读取或走浏览器授权，命令行不碰。Key 由调用方从标准输入读（不放进命令行参数，免得留在 shell 历史里），
//! 这里不打印、不记日志。桌面端每 2 秒看连接表签名，变了就刷新界面。

use rusqlite::Connection;

pub struct ProviderView {
    pub id: String,
    pub platform: String,
    pub kind: String,
    pub name: String,
    pub masked: String,
    pub status: String,
    pub base_url: Option<String>,
}

pub fn list(conn: &Connection) -> Result<Vec<ProviderView>, String> {
    Ok(crate::connections::list(conn)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|c| ProviderView { id: c.id, platform: c.platform, kind: c.kind, name: c.name, masked: c.masked, status: c.status, base_url: c.base_url })
        .collect())
}

pub struct ApiKeyInput {
    pub platform: String,
    pub name: String,
    pub secret: String,
    pub base_url: Option<String>,
    pub protocol: Option<String>,
    pub model: Option<String>,
    pub service: Option<String>,
}

/// 归一与校验（不联网）；单独拆出来便于测试和先报参数错误
pub fn prepare(input: ApiKeyInput) -> Result<crate::connections::NewConnection, String> {
    if input.secret.trim().is_empty() {
        return Err("API Key 为空：从标准输入传入，例如 `echo sk-... | jolu provider add ...`".into());
    }
    crate::connections::prepare_new(crate::connections::NewConnection {
        service: input.service,
        platform: input.platform,
        kind: "api".into(),
        name: input.name,
        secret: Some(input.secret),
        base_url: input.base_url,
        model: input.model,
        effort: None,
        context_1m: None,
        protocol: input.protocol,
        slug_base: None,
    })
}

/// 校验、联网验证并保存；返回新连接 id。会发网络请求，调用方不要在异步运行时线程里直接调。
pub fn add_api_key(conn: &mut Connection, input: ApiKeyInput) -> Result<String, String> {
    let prepared = prepare(input)?;
    let secret = prepared.secret.clone().ok_or("连接尚未准备好凭证")?;
    crate::quota::validate_connection(&prepared.platform, &prepared.kind, &secret, prepared.base_url.as_deref(), prepared.model.as_deref())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let id = crate::connections::add(&tx, prepared)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

/// 进出模型池（同桌面端「管理来源」的开关）：网关每次请求都从库里读，写入即生效
pub fn set_serve(conn: &Connection, id: &str, on: bool) -> Result<(), String> {
    if !list(conn)?.iter().any(|p| p.id == id) {
        return Err(format!("没有 id 为「{id}」的连接"));
    }
    crate::connections::set_serve(conn, id, on)
}

/// 删除连接：只删凭据与配置，历史统计保留（同桌面端）
pub fn remove(conn: &Connection, id: &str) -> Result<(), String> {
    if !list(conn)?.iter().any(|p| p.id == id) {
        return Err(format!("没有 id 为「{id}」的连接"));
    }
    crate::connections::remove(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(platform: &str) -> ApiKeyInput {
        ApiKeyInput { platform: platform.into(), name: "我的网关".into(), secret: "sk-test-1234567890".into(), base_url: Some("https://example.com/v1/".into()), protocol: Some("openai".into()), model: None, service: None }
    }

    #[test]
    fn prepare_uses_desktop_rules() {
        let ok = prepare(input("custom")).unwrap();
        assert_eq!((ok.kind.as_str(), ok.base_url.as_deref()), ("api", Some("https://example.com/v1")));
        assert!(prepare(ApiKeyInput { secret: " ".into(), ..input("custom") }).unwrap_err().contains("标准输入"));
        assert!(prepare(ApiKeyInput { protocol: None, ..input("custom") }).is_err(), "自定义供应商要选协议");
        assert!(prepare(ApiKeyInput { base_url: Some("ftp://x".into()), ..input("custom") }).is_err());
        assert!(prepare(input("no-such-platform")).is_err());
    }

    #[test]
    fn list_and_remove_on_an_empty_database() {
        let path = std::env::temp_dir().join(format!("jolu-providers-{}-{}.db", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let conn = crate::db::open(&path).unwrap();
        assert!(list(&conn).unwrap().is_empty());
        assert!(remove(&conn, "nope").is_err());
    }
}
