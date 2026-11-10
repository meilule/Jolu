//! 搜索 MCP 协议处理（能力汇聚第 2 层，批 5）：stdio 上说 MCP 协议，
//! 把批 3 的搜索能力以 `web_search` 工具暴露给 agent。
//!
//! 形态：`jolu.exe --mcp-search`，由 agent 自己拉起（agent 的 mcpServers 配置
//! 指向本可执行文件），stdin/stdout 即 MCP 通道，一行一个 JSON-RPC 消息。
//! agent 退出，本进程随之退出——无守护、无端口、无监听。stdio 循环在
//! `lib.rs::run_mcp_search`。
//!
//! 管什么：
//! - `initialize` / `tools/list` / `tools/call`（`web_search`）三个方法；
//! - 搜索配置从桌面端数据目录的 `settings.json`（`search_apis`）读取，
//!   与 CLI `jolu search`、未来 UI 同一份数据源；
//! - Key 轮转的冷却状态只活在本进程内（agent 会话生命周期）。
//!
//! 不管什么：
//! - 不做 `web_fetch`（结果 URL 的抓取是后续项）；
//! - 不读网关数据库——MCP server 与网关监听是完全独立的两条进程，
//!   互不依赖对方存活。
//!
//! Key 只进内存与设置文件；本进程的 stdout 是 MCP 通道，任何日志走 stderr。

use crate::gateway::search::{self, SearchApiConfig};

/// MCP 协议版本（agent 侧可能更高，协商取交集由调用方保证；这里固定声明）。
const PROTOCOL_VERSION: &str = "2024-11-05";

/// 处理一行 JSON-RPC；通知类消息返回 None（无响应）。
pub(crate) async fn handle_line(line: &str) -> Option<String> {
    let request: serde_json::Value = serde_json::from_str(line).ok()?;
    let id = request.get("id")?.clone();
    let method = request["method"].as_str()?.to_string();
    let params = request.get("params").cloned().unwrap_or(serde_json::json!({}));

    let result: Result<serde_json::Value, String> = match method.as_str() {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "jolu-search", "version": env!("CARGO_PKG_VERSION") },
        })),
        "notifications/initialized" | "notifications/cancelled" => return None,
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(serde_json::json!({
            "tools": [{
                "name": "web_search",
                "description": "联网搜索：按用户配置的搜索后端（Tavily / SearXNG / 豆包）返回网页结果列表。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "搜索关键词" },
                        "count": { "type": "integer", "description": "结果条数（默认 5，最多 20）" },
                    },
                    "required": ["query"],
                },
            }],
        })),
        "tools/call" => handle_tools_call(&params).await,
        other => Err(format!("未知方法 {other}")),
    };

    let response = match result {
        Ok(value) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err(reason) => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32603, "message": reason },
        }),
    };
    Some(response.to_string())
}

/// `tools/call`：目前只有 `web_search`。
async fn handle_tools_call(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let name = params["name"].as_str().unwrap_or_default();
    if name != "web_search" {
        return Err(format!("未知工具 {name}"));
    }
    let arguments = &params["arguments"];
    let query = arguments["query"].as_str().unwrap_or_default().to_string();
    let count = arguments["count"].as_u64().unwrap_or(5).clamp(1, 20) as usize;

    let config = search_config().ok_or("还没有配置搜索后端：在桌面端 settings.json 的 search_apis 里配置（vendor + keys）")?;
    let mut refused_tail: Vec<(String, i64)> = Vec::new();
    let now = chrono::Utc::now().timestamp_millis();
    let hits = search::run_search(&config, &query, count, &mut refused_tail, now)
        .await
        .map_err(|error| error.to_string())?;

    // 文本化输出：agent 读的是工具结果文本，列表逐条给标题 / 链接 / 摘要
    let mut text = if hits.is_empty() { "没有搜索结果。".to_string() } else { String::new() };
    for (index, hit) in hits.iter().enumerate() {
        text.push_str(&format!("{}. {}\n   {}\n", index + 1, hit.title, hit.url));
        if !hit.snippet.is_empty() {
            text.push_str(&format!("   {}\n", hit.snippet));
        }
    }
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
    }))
}

/// 读搜索配置：第一个未暂停的后端。与 CLI `jolu search` 同一份数据源。
fn search_config() -> Option<SearchApiConfig> {
    let directory = crate::service::paths::config_directory().ok()?;
    let settings = crate::service::settings::read_settings(&directory);
    settings.search_apis.into_iter().find(|api| !api.off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialize_and_tools_list_shape() {
        let response = handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .await
            .expect("initialize 有响应");
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["serverInfo"]["name"], "jolu-search");

        let response = handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .await
            .expect("tools/list 有响应");
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        let tools = value["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "web_search");
        assert_eq!(tools[0]["inputSchema"]["required"][0], "query");
    }

    #[tokio::test]
    async fn notifications_are_silent_and_unknown_tools_error() {
        // 通知无响应
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await.is_none());
        // 未知工具：报错但带 id
        let response = handle_line(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"web_fetch","arguments":{}}}"#,
        )
        .await
        .expect("未知工具也有响应");
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["id"], 7);
        assert!(value["error"]["message"].as_str().unwrap().contains("web_fetch"));
    }

    #[tokio::test]
    async fn web_search_without_config_reports_actionable_error() {
        // 用 JOLU_DATA_DIR 指向空目录：配置缺失时报「还没配置」，不是崩溃
        let previous = std::env::var_os("JOLU_DATA_DIR");
        let dir = std::env::temp_dir().join(format!("jolu-mcp-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("JOLU_DATA_DIR", &dir);
        let response = handle_line(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"web_search","arguments":{"query":"rust"}}}"#,
        )
        .await;
        match previous {
            Some(value) => std::env::set_var("JOLU_DATA_DIR", value),
            None => std::env::remove_var("JOLU_DATA_DIR"),
        }
        let value: serde_json::Value =
            serde_json::from_str(&response.expect("缺配置也有响应")).unwrap();
        assert!(value["error"]["message"].as_str().unwrap().contains("search_apis"));
    }
}
