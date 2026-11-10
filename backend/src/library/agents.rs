//! 各 agent 配置文件的 MCP 段读写（library 的适配层）。
//!
//! 四个 agent，三种形状（magpie `mcp.go::key()` 的键名分派）：
//!
//! | agent | 文件 | 格式 | 键 | 条目形状 |
//! |---|---|---|---|---|
//! | claude-code | `~/.claude/settings.json` | JSON | `mcpServers` | `{command, args}` |
//! | gemini | `~/.gemini/settings.json` | JSON | `mcpServers` | `{command, args}` |
//! | codex | `~/.codex/config.toml` | TOML | `mcp_servers` | `command`、`args`（toml_edit 保注释） |
//! | opencode | `opencode.json` | JSON | `mcp` | `{type:"local", command:[cmd,...args], enabled:true}` |
//!
//! ## 为什么只有 4 个，而 magpie 覆盖 25 种格式
//!
//! magpie `library/mcp.go` 用**声明式表**驱动：`owned` 表为每种格式列出「这个
//! server 条目里哪些字段是 magpie 写的」，配一个通用引擎读写。实测那 25 种格式
//! 归并后只有 **6 个 MCP 键名**（`mcpServers` / `mcp_servers` / `mcp` /
//! `extensions` / `context_servers` / `mcp.servers`）与 **13 种字段集合**，其中
//! 7 种格式共用最主流的 `{type,url,headers,command,args,env}`。
//!
//! Jolu 这里用 `match agent` 而不是表，是**刻意的范围取舍**：
//! - Jolu 的 `wiring::AGENTS` 只有 5 个 agent，其中 zcode 的 `provider.json`
//!   用未实测的 `mcp.servers` 嵌套形状，按「不猜」原则暂不支持；
//! - magpie 那 25 种里，其余 20 种（Antigravity / Cursor / Zed / Goose / Crush /
//!   Kiro / Devin / Cline / Copilot / Pi / Hermes / Alma / Kimi / Grok / Dsh /
//!   LiveAgent / CommandCode / Desktop / Omp / PiNative）Jolu 连**接线都不支持**，
//!   装 MCP 也无处可用。
//!
//! ## 将来要加 agent 时怎么改
//!
//! 把 `match agent` 换成语义等价的格式描述表（约半天改造，之后每加一个 agent
//! 约 30 分钟 + 一个往返测试）：
//!
//! ```ignore
//! struct McpFormat {
//!     key: &'static str,              // "mcpServers" / "mcp_servers" / ...
//!     style: FieldStyle,              // Claude式 / Codex式 / OpenCode式 / Goose式 / Zed式
//!     owned: &'static [&'static str], // 我写哪些字段（对应 magpie 的 owned 表）
//! }
//! enum FieldStyle { Claude, Codex, OpenCode, Goose, Zed }
//! ```
//!
//! 只有 5 个 `FieldStyle` 变体：三处差异分别是 **`type` vs `transport`**、
//! **`env` vs `environment` vs `envs`**、**`command` 字符串 vs 数组**。
//! 现在不提前改，是因为 zcode 的格式未实测——空表项既写不出正确配置，
//! 也让「支持了哪些 agent」变得含混。等到真要接第 6 个 agent 时再改不亏。
//!
//! ## 纪律
//!
//! - **原子写**（批 2 steady）：先写临时文件再改名，中途失败不留半截配置；
//! - **保留注释与格式**：TOML 走 `toml_edit`；JSON 重排键序无妨（这些文件本就机器管理），
//!   但 Claude Code 的 settings.json 用户可能手动加注释（JSONC）——读取时容忍尾逗号
//!   与注释（宽松解析），写出为标准 JSON；
//! - 删除不存在的条目返回 false，不算错误；
//! - **别在这里判「是不是 Jolu 装的」**：那是 `library::is_ours` 的事，本模块只管
//!   按格式读写；冲突判定收在上层是为了让「拒绝覆盖第三方条目」这条纪律只有一个
//!   判定点。

use std::path::Path;

use super::McpServerItem;

/// 读该 agent 配置里现有的 server 名字（用于同名冲突判定）。
pub(super) fn read_server_names(path: &Path, agent: &str, key: &str) -> Result<Vec<String>, String> {
    if !crate::steady::exists(path) {
        return Ok(Vec::new());
    }
    let text = crate::steady::read_to_string(path).map_err(|error| error.to_string())?;
    match agent {
        "codex" => {
            let document = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|error| format!("{} 格式无效：{error}", path.display()))?;
            Ok(document
                .get(key)
                .and_then(|item| item.as_table())
                .map(|table| table.iter().map(|(name, _)| name.to_string()).collect())
                .unwrap_or_default())
        }
        _ => {
            let value = parse_lenient_json(&text, path)?;
            Ok(value
                .get(key)
                .and_then(|v| v.as_object())
                .map(|object| object.keys().cloned().collect())
                .unwrap_or_default())
        }
    }
}

/// 把一个 server 写进配置（新增或更新——更新只对 applied 表里登记过的名字发生，
/// 冲突判定在 `library::install`）。
pub(super) fn write_server(path: &Path, agent: &str, key: &str, server: &McpServerItem) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建 {} 失败：{error}", parent.display()))?;
    }
    let text = if crate::steady::exists(path) {
        crate::steady::read_to_string(path).map_err(|error| error.to_string())?
    } else {
        String::new()
    };
    match agent {
        "codex" => write_toml_server(path, &text, key, server),
        "opencode" => {
            let mut value = parse_lenient_json_or_empty(&text, path)?;
            let mut command = vec![server.command.clone()];
            command.extend(server.args.iter().cloned());
            let entry = serde_json::json!({
                "type": "local",
                "command": command,
                "enabled": true,
            });
            value[key][server.name.clone()] = entry;
            write_json_atomic(path, &value)
        }
        // claude-code 与 gemini 同形状：mcpServers.<name> = {command, args}
        _ => {
            let mut value = parse_lenient_json_or_empty(&text, path)?;
            value[key][server.name.clone()] =
                serde_json::json!({"command": server.command, "args": server.args});
            write_json_atomic(path, &value)
        }
    }
}

/// 从配置里移除一个 server。返回是否真的删了（文件里本来没有 = false）。
pub(super) fn remove_server(path: &Path, agent: &str, key: &str, name: &str) -> Result<bool, String> {
    if !crate::steady::exists(path) {
        return Ok(false);
    }
    let text = crate::steady::read_to_string(path).map_err(|error| error.to_string())?;
    match agent {
        "codex" => {
            let mut document = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|error| format!("{} 格式无效：{error}", path.display()))?;
            let removed = document
                .get_mut(key)
                .and_then(|item| item.as_table_mut())
                .is_some_and(|table| table.remove(name).is_some());
            if removed {
                write_text_atomic(path, &document.to_string())?;
            }
            Ok(removed)
        }
        _ => {
            let mut value = parse_lenient_json(&text, path)?;
            let removed = value
                .get_mut(key)
                .and_then(|v| v.as_object_mut())
                .is_some_and(|object| object.remove(name).is_some());
            // 键空了就整段移除，避免留下 `"mcpServers": {}` 的空壳
            if removed {
                if let Some(object) = value.as_object_mut() {
                    if object.get(key).and_then(|v| v.as_object()).is_some_and(|empty| empty.is_empty()) {
                        object.remove(key);
                    }
                }
                write_json_atomic(path, &value)?;
            }
            Ok(removed)
        }
    }
}

// ===== 内部：解析与落盘 =====

/// 宽松 JSON 解析：容忍用户手写的注释与尾逗号（Claude Code 的 settings.json
/// 常被手动编辑成 JSONC）。`serde_json` 严格解析失败时降级去注释 / 尾逗号再试。
fn parse_lenient_json(text: &str, path: &Path) -> Result<serde_json::Value, String> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => Ok(value),
        Err(strict_error) => {
            let stripped = strip_jsonc(text);
            serde_json::from_str::<serde_json::Value>(&stripped)
                .map_err(|error| format!("{} 解析失败（严格：{strict_error}；宽松：{error}）", path.display()))
        }
    }
}

fn parse_lenient_json_or_empty(text: &str, path: &Path) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    parse_lenient_json(text, path)
}

/// 去 `//` 行注释与 `/* */` 块注释与尾逗号（字符串字面量里的不动）。
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(current) = chars.next() {
        if in_string {
            out.push(current);
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == '"' {
                in_string = false;
            }
            continue;
        }
        match current {
            '"' => {
                in_string = true;
                out.push(current);
            }
            '/' if chars.peek() == Some(&'/') => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                while let Some(skipped) = chars.next() {
                    if skipped == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
            }
            ',' => {
                // 尾逗号：跳过空白与注释后的 `}` 或 `]`
                let mut lookahead = chars.clone();
                let mut trimmed = String::new();
                let mut closes = None;
                while let Some(next) = lookahead.next() {
                    if next.is_whitespace() {
                        trimmed.push(next);
                        continue;
                    }
                    if next == '/' && lookahead.peek() == Some(&'/') {
                        for skipped in lookahead.by_ref() {
                            if skipped == '\n' {
                                trimmed.push('\n');
                                break;
                            }
                        }
                        continue;
                    }
                    closes = Some(next);
                    break;
                }
                if matches!(closes, Some('}') | Some(']')) {
                    out.push_str(&trimmed);
                    out.push(closes.unwrap());
                    chars = lookahead;
                    chars.next();
                } else {
                    out.push(current);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut pretty = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    pretty.push('\n');
    write_text_atomic(path, &pretty)
}

fn write_text_atomic(path: &Path, text: &str) -> Result<(), String> {
    // 复用 wiring 的原子写（临时文件 + rename，批 2 steady 语义）
    crate::gateway::wiring::write_json_atomic_public(path, text.as_bytes().to_vec())
}

// ===== TOML（codex）=====

fn write_toml_server(path: &Path, text: &str, key: &str, server: &McpServerItem) -> Result<(), String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("{} 格式无效：{error}", path.display()))?;
    if !document.contains_key(key) {
        document[key] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let table = document[key]
        .as_table_mut()
        .ok_or_else(|| format!("{} 的 [{key}] 不是表", path.display()))?;
    let mut entry = toml_edit::Table::new();
    entry["command"] = toml_edit::value(server.command.clone());
    let mut args = toml_edit::Array::new();
    for arg in &server.args {
        args.push(arg.clone());
    }
    entry["args"] = toml_edit::value(args);
    table.insert(&server.name, toml_edit::Item::Table(entry));
    write_text_atomic(path, &document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jolu-lib-agents-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }

    fn server() -> McpServerItem {
        McpServerItem { name: "jolu-search".into(), command: "C:\\jolu\\jolu.exe".into(), args: vec!["--mcp-search".into()] }
    }

    #[test]
    fn claude_code_round_trip() {
        let path = temp_path("claude.json");
        write_server(&path, "claude-code", "mcpServers", &server()).unwrap();
        assert!(read_server_names(&path, "claude-code", "mcpServers").unwrap().contains(&"jolu-search".to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["mcpServers"]["jolu-search"]["command"], "C:\\jolu\\jolu.exe");
        assert_eq!(value["mcpServers"]["jolu-search"]["args"][0], "--mcp-search");
        // 用户的其他键保留
        assert!(value.get("model").is_none() || value["model"].is_null());
        assert!(remove_server(&path, "claude-code", "mcpServers", "jolu-search").unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("jolu-search"), "卸载后不残留：{text}");
        assert!(!text.contains("mcpServers"), "空段不残留空壳：{text}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn claude_code_lenient_jsonc_user_file_survives() {
        // 用户手动编辑过的 JSONC（注释 + 尾逗号）：装得进去，其他键不丢
        let path = temp_path("claude-jsonc.json");
        std::fs::write(
            &path,
            r#"{
  // 我的注释
  "model": "opus",
  "mcpServers": {
    "user-own": {"command": "C:/user/tool.exe"},
  },
}"#,
        )
        .unwrap();
        write_server(&path, "claude-code", "mcpServers", &server()).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["model"], "opus", "用户的顶层键保留");
        assert_eq!(value["mcpServers"]["user-own"]["command"], "C:/user/tool.exe", "用户自己的 server 保留");
        assert_eq!(value["mcpServers"]["jolu-search"]["args"][0], "--mcp-search");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn codex_toml_round_trip_keeps_comments() {
        let path = temp_path("codex.toml");
        std::fs::write(
            &path,
            "# 我的配置\nmodel = \"gpt-5.2\"\n\n[mcp_servers.user_own]\ncommand = \"C:/user/tool.exe\"\n",
        )
        .unwrap();
        write_server(&path, "codex", "mcp_servers", &server()).unwrap();
        assert!(read_server_names(&path, "codex", "mcp_servers").unwrap().contains(&"jolu-search".to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# 我的配置"), "注释必须保留（toml_edit 的价值）");
        assert!(text.contains("model = \"gpt-5.2\""), "既有键保留");
        assert!(text.contains("[mcp_servers.jolu-search]"), "条目按子表写入");
        assert!(text.contains("user_own"), "用户的 server 保留");
        assert!(remove_server(&path, "codex", "mcp_servers", "jolu-search").unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("jolu-search"));
        assert!(text.contains("user_own"), "卸载只删自己的");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opencode_uses_local_array_command() {
        let path = temp_path("opencode.json");
        write_server(&path, "opencode", "mcp", &server()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["mcp"]["jolu-search"]["type"], "local");
        assert_eq!(value["mcp"]["jolu-search"]["command"][0], "C:\\jolu\\jolu.exe");
        assert_eq!(value["mcp"]["jolu-search"]["command"][1], "--mcp-search");
        assert_eq!(value["mcp"]["jolu-search"]["enabled"], true);
        assert!(remove_server(&path, "opencode", "mcp", "jolu-search").unwrap());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn remove_missing_file_is_false_not_error() {
        let path = temp_path("missing.json");
        assert!(!remove_server(&path, "claude-code", "mcpServers", "jolu-search").unwrap());
    }
}
