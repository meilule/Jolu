//! 从本机既有配置导入供应商连接（P4.3，对齐 magpie 的 import apps）。
//!
//! 三个来源：CC Switch（`~/.cc-switch/cc-switch.db` 或旧版 config.json）、
//! Claude Code（`~/.claude/settings.json` 的 env）、Codex（`~/.codex/config.toml` + auth.json）。
//!
//! 纪律（AGENTS.md §4 + magpie 同款）：
//! - **键不出后端**：`scan` 给前端的草稿只带掩码；`apply` 由后端重读原文建连接，前端只回传 ref；
//! - **自己的接线不算**：Claude Code 的 env 若指向本机网关（Jolu 接线的产物），跳过——
//!   那是 Jolu 写进去的，不是用户的上游（magpie 同款判断）；
//! - **只读原文**：导入绝不改写来源文件；落库由 `apply` 走既有连接创建路径。

use serde_json::{json, Value};
use std::path::PathBuf;

fn home() -> Option<PathBuf> {
    crate::creds::home()
}

fn mask_key(key: &str) -> String {
    let key = key.trim();
    if key.len() <= 8 {
        "••••".into()
    } else {
        format!("••••••••{}", &key[key.len() - 4..])
    }
}

/// 网关地址判定：Jolu 接线写进去的 base 不是用户上游，跳过。
fn points_at_local_gateway(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    lower.contains("127.0.0.1") || lower.contains("localhost")
}

fn draft(source: &'static str, from: String, reference: String, name: String, base_url: String, key: String, protocol: &'static str) -> Draft {
    Draft { reference, from, source, name, base_url, key, protocol }
}

/// 扫描全部来源。单个来源失败不拖垮其它（错误进该来源的条目，UI 能展示原因）。
pub(crate) fn scan() -> Result<Value, String> {
    let mut out: Vec<Value> = Vec::new();
    for (source, result) in [
        ("claude-code", scan_claude_code()),
        ("codex", scan_codex()),
        ("cc-switch", scan_cc_switch()),
    ] {
        match result {
            Ok(drafts) => {
                for d in drafts {
                    out.push(json!({"reference": d.reference, "source": d.source, "from": d.from, "name": d.name, "baseUrl": d.base_url, "protocol": d.protocol, "keyMasked": mask_key(&d.key)}));
                }
            }
            Err(error) => out.push(json!({"source": source, "error": error})),
        }
    }
    Ok(json!({ "drafts": out }))
}

/// 按引用落库：重读原文取真 Key，走既有 `connections::add`（加密、slug 分配、掩码都在正路里）。
/// 只做「新增」；前端先行 diff，apply 不做去重以外的写操作。
///
/// 两处与「显示名」解耦的地方：
///
/// - **slug 用草稿自己的名字**（`slug_base`），不用拼了来源标签的显示名，否则会得到
///   `claude-code-claude-code` 这种重复词；
/// - **导入后顺带拉一次模型清单**：这里只有 Key 与地址、没有模型，不拉的话这个来源在
///   模型路由里永远是「0 个模型」。拉取失败**不阻断导入**——用户至少先拿到连接，可以
///   在账号卡上再点拉取；失败原因随返回值带出，由调用方决定怎么提示。
pub(crate) fn apply(conn: &rusqlite::Connection, references: &[String]) -> Result<Vec<String>, String> {
    let mut created = Vec::new();
    for reference in references {
        let draft = match reference.as_str() {
            r if r.starts_with("claude-code:") => read_claude_code_one().ok_or("Claude Code 的来源已不可读")?,
            r if r.starts_with("cc-switch:") => read_cc_switch_one(&r["cc-switch:".len()..])
                .ok_or("CC Switch 的该条目已不存在")?,
            other => return Err(format!("不认识的导入引用：{other}")),
        };
        let name = format!("{} · {}", source_label(&draft.source), draft.name);
        let id = crate::connections::add(
            conn,
            crate::connections::NewConnection {
                service: None,
                platform: "custom".into(),
                kind: "api".into(),
                name: name.clone(),
                secret: Some(draft.key.clone()),
                base_url: Some(draft.base_url.clone()),
                model: None,
                effort: None,
                context_1m: None,
                protocol: Some(draft.protocol.into()),
                // 「Claude Code 上游」→ `claude-code-upstream`；纯中文名仍按兜底规则走
                slug_base: Some(draft.name.clone()),
            },
        )?;
        // 清单落库：成功就把模型一次带出来，失败只记原因、不回滚这次导入
        let models = crate::connections::fetch_models("custom", Some(draft.protocol), &draft.base_url, &draft.key)
            .ok()
            .filter(|list| !list.is_empty());
        if let Some(list) = &models {
            if let Ok(json) = serde_json::to_string(list) {
                let _ = conn.execute("UPDATE connections SET models_json = ?2 WHERE id = ?1", rusqlite::params![id, json]);
            }
        }
        created.push(format!("{name}（{id}）"));
    }
    Ok(created)
}

fn source_label(source: &str) -> String {
    match source {
        "claude-code" => "Claude Code".into(),
        "cc-switch" => "CC Switch".into(),
        other => other.into(),
    }
}

// ── Claude Code ──

struct Draft {
    reference: String,
    from: String,
    source: &'static str,
    name: String,
    base_url: String,
    key: String,
    protocol: &'static str,
}

fn read_claude_code_one() -> Option<Draft> {
    scan_claude_code().ok().and_then(|mut drafts| drafts.pop())
}

fn scan_claude_code() -> Result<Vec<Draft>, String> {
    let path = home().map(|h| h.join(".claude").join("settings.json")).ok_or("无法定位用户目录")?;
    let text = std::fs::read_to_string(&path).map_err(|error| format!("Claude Code settings.json：{error}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| format!("Claude Code settings.json 解析失败：{error}"))?;
    let base = value["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap_or("").trim().to_string();
    let key = value["env"]["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap_or("").trim().to_string();
    if base.is_empty() || key.is_empty() {
        return Err("Claude Code 的 env 里没有可导入的地址与令牌".into());
    }
    // Jolu 自己接线写进去的不算用户上游
    if points_at_local_gateway(&base) || key.starts_with("jolu-") {
        return Err("Claude Code 当前由 Jolu 接线，没有可导入的用户上游".into());
    }
    let protocol = if base.contains("/api/anthropic") || base.contains("anthropic.com") { "anthropic" } else { "openai" };
    Ok(vec![draft("claude-code", "Claude Code".into(), "claude-code:main".into(), "Claude Code 上游".into(), base, key, protocol)])
}

// ── Codex ──

fn scan_codex() -> Result<Vec<Draft>, String> {
    let dir = home().map(|h| h.join(".codex")).ok_or("无法定位用户目录")?;
    let config = std::fs::read_to_string(dir.join("config.toml")).map_err(|error| format!("Codex config.toml：{error}"))?;
    // 裸解析：找 model_provider 与 model_providers.<id>.base_url（toml_edit 只在需要注释保真处用，这里只读）
    let mut provider_id = String::new();
    for line in config.lines() {
        if let Some(rest) = line.trim().strip_prefix("model_provider") {
            if let Some(value) = rest.trim_start_matches(|c: char| c == ' ' || c == '=').trim().strip_prefix('"') {
                provider_id = value.trim_end_matches('"').to_string();
            }
        }
    }
    let mut base = String::new();
    if !provider_id.is_empty() {
        let needle = format!("[model_providers.{provider_id}]");
        if let Some(pos) = config.find(&needle) {
            for line in config[pos..].lines().skip(1) {
                if line.starts_with('[') { break; }
                if let Some(value) = line.trim().strip_prefix("base_url") {
                    base = value.trim_start_matches(|c: char| c == ' ' || c == '=').trim().trim_matches('"').to_string();
                    break;
                }
            }
        }
    }
    if base.is_empty() || points_at_local_gateway(&base) {
        return Err("Codex 当前没有可导入的用户上游（由 Jolu 接线或未配置）".into());
    }
    let key = std::fs::read_to_string(dir.join("auth.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("OPENAI_API_KEY").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    if key.is_empty() {
        return Err("Codex auth.json 没有 OPENAI_API_KEY（可能是 ChatGPT 登录，不可导出）".into());
    }
    Ok(vec![draft("codex", "Codex".into(), "codex:main".into(), "Codex 上游".into(), base, key, "openai")])
}

// ── CC Switch ──

fn cc_switch_path() -> Option<PathBuf> {
    let dir = home()?.join(".cc-switch");
    let db = dir.join("cc-switch.db");
    if db.exists() { return Some(db); }
    let js = dir.join("config.json");
    if js.exists() { return Some(js); }
    None
}

fn scan_cc_switch() -> Result<Vec<Draft>, String> {
    let Some(path) = cc_switch_path() else { return Err("本机没有 CC Switch（~/.cc-switch）".into()) };
    let mut out = Vec::new();
    if path.extension().is_some_and(|e| e == "db") {
        out = scan_cc_switch_db(&path)?;
    } else {
        let text = std::fs::read_to_string(&path).map_err(|error| format!("CC Switch config.json：{error}"))?;
        let value: Value = serde_json::from_str(&text).map_err(|error| format!("CC Switch config.json 解析失败：{error}"))?;
        // 旧版结构：{ "<app>": { "<name>": { settings_config: { env: {...} } } } }，逐层找 env
        collect_cc_switch_json(&value, &mut out);
    }
    if out.is_empty() {
        return Err("CC Switch 里没有可导入的条目".into());
    }
    Ok(out)
}

fn scan_cc_switch_db(path: &PathBuf) -> Result<Vec<Draft>, String> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("CC Switch 数据库只读打开失败：{error}"))?;
    let mut stmt = conn
        .prepare("SELECT id, app_type, name, settings_config FROM providers ORDER BY app_type, name")
        .map_err(|error| format!("CC Switch 数据库查询失败：{error}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, app_type, name, settings_config) = row.map_err(|error| error.to_string())?;
        let Ok(value) = serde_json::from_str::<Value>(&settings_config) else { continue };
        let base = value["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap_or("").to_string();
        let key = value["env"]["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap_or("").to_string();
        if base.is_empty() || key.is_empty() || points_at_local_gateway(&base) {
            continue;
        }
        let protocol = if base.contains("anthropic") { "anthropic" } else { "openai" };
        out.push(draft("cc-switch", format!("CC Switch · {app_type}"), format!("cc-switch:{id}"), name, base, key, protocol));
    }
    Ok(out)
}

/// 旧版 config.json：递归找带 ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN 的 env 节点。
fn collect_cc_switch_json(value: &Value, out: &mut Vec<Draft>) {
    if let Some(object) = value.as_object() {
        let base = object.get("ANTHROPIC_BASE_URL").and_then(Value::as_str).unwrap_or("");
        let key = object.get("ANTHROPIC_AUTH_TOKEN").and_then(Value::as_str).unwrap_or("");
        if !base.is_empty() && !key.is_empty() && !points_at_local_gateway(base) {
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("CC Switch 条目")
                .to_string();
            let protocol = if base.contains("anthropic") { "anthropic" } else { "openai" };
            out.push(draft("cc-switch", "CC Switch".into(), format!("cc-switch:json:{}", out.len()), name, base.to_string(), key.to_string(), protocol));
            return;
        }
        for child in object.values() {
            collect_cc_switch_json(child, out);
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            collect_cc_switch_json(child, out);
        }
    }
}

fn read_cc_switch_one(entry: &str) -> Option<Draft> {
    scan_cc_switch().ok()?.into_iter().find(|d| d.reference == format!("cc-switch:{entry}"))
}


