//! 通用接入（P1-1，交接清单 §二）：Jolu 没有专门适配的工具，由用户手填
//! 配置文件路径、格式（JSON / TOML / env）与三个键名，Jolu 按这份说明接入模型路由。
//!
//! 纪律（AGENTS.md / 交接清单 §四）：
//! - 接入前把三个键的原值存进 `custom_agents.original_json`，断开时逐键还原；
//! - 只还原 Jolu 自己写的值：断开时某个键已被用户改成别的值，就不动它；
//! - 写前备份到应用数据目录 `gateway-backups/`；令牌是 Jolu 自己生成的接线令牌，不是用户的 Key。
//!
//! 键名写法：JSON / TOML 用点分隔的路径（如 `env.ANTHROPIC_BASE_URL`、`provider.jolu.options.baseURL`），
//! 键名本身不能含点；env 文件直接写变量名（如 `OPENAI_BASE_URL`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

/// 接线记录（`agent_wiring`）里的 agent id 前缀：网关严格鉴权按它认令牌
pub const RECORD_PREFIX: &str = "custom-agent:";

pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_agents (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            slug          TEXT NOT NULL UNIQUE,
            config_path   TEXT NOT NULL,
            format        TEXT NOT NULL,
            base_url_key  TEXT NOT NULL,
            auth_key      TEXT NOT NULL,
            model_key     TEXT,
            base_suffix   TEXT NOT NULL DEFAULT '',
            original_json TEXT,
            applied_json  TEXT,
            wired_ms      INTEGER,
            created_ms    INTEGER NOT NULL
        );",
    )
}

/// 用户填的接入说明
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CustomAgentInput {
    pub name: String,
    pub config_path: String,
    pub format: String,
    pub base_url_key: String,
    pub auth_key: String,
    #[serde(default)]
    pub model_key: Option<String>,
    /// 地址后缀，如 `/v1`（OpenAI 兼容的工具通常要）；空 = 只写 `http://127.0.0.1:端口`
    #[serde(default)]
    pub base_suffix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CustomAgent {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub config_path: String,
    pub format: String,
    pub base_url_key: String,
    pub auth_key: String,
    pub model_key: Option<String>,
    pub base_suffix: String,
    pub original: Option<BTreeMap<String, Option<String>>>,
    pub applied: Option<BTreeMap<String, String>>,
    pub wired_ms: Option<i64>,
}

/// 给界面的视图（不含原值快照：里面可能有用户自己的 Key）
#[derive(Debug, Clone, serde::Serialize)]
pub struct CustomAgentView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub config_path: String,
    pub format: String,
    pub base_url_key: String,
    pub auth_key: String,
    pub model_key: Option<String>,
    pub base_suffix: String,
    /// 配置文件或它所在的目录存在
    pub installed: bool,
    /// 文件里的地址仍是 Jolu 写的
    pub wired: bool,
    /// 接入过但地址已被改掉：changed；配置文件没了：removed
    pub drift: Option<String>,
    /// 接入时写下的模型
    pub model: Option<String>,
    /// 接入时间（毫秒）；没接入为 None
    pub wired_ms: Option<i64>,
}

pub const FORMATS: [&str; 3] = ["json", "toml", "env"];

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "tool".into() } else { slug.chars().take(32).collect() }
}

/// `~` 开头的路径按用户目录展开
pub fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match (path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")), crate::creds::home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if path == "~" => crate::creds::home().unwrap_or_else(|| PathBuf::from(path)),
        _ => PathBuf::from(path),
    }
}

pub fn validate(input: &CustomAgentInput) -> Result<(), String> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("名称不能为空，最多 40 个字".into());
    }
    if input.config_path.trim().is_empty() {
        return Err("请填写配置文件路径".into());
    }
    let expanded = expand(&input.config_path);
    if !expanded.is_absolute() {
        return Err("配置文件路径要写绝对路径，或以 ~ 开头".into());
    }
    if !FORMATS.contains(&input.format.as_str()) {
        return Err("格式只能是 JSON、TOML 或 env".into());
    }
    for (label, key) in [("地址键", Some(&input.base_url_key)), ("令牌键", Some(&input.auth_key)), ("模型键", input.model_key.as_ref())] {
        let Some(key) = key.map(|k| k.trim()) else { continue };
        if key.is_empty() {
            if label == "模型键" {
                continue;
            }
            return Err(format!("请填写{label}"));
        }
        if key.split('.').any(|part| part.trim().is_empty()) || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("{label}「{key}」写法不对：用点分隔路径，不要有空格"));
        }
        if input.format == "env" && key.contains('.') {
            return Err(format!("env 文件的{label}直接写变量名，不要带点"));
        }
    }
    if let Some(suffix) = input.base_suffix.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !suffix.starts_with('/') || suffix.contains(char::is_whitespace) {
            return Err("地址后缀以 / 开头，如 /v1".into());
        }
    }
    Ok(())
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomAgent> {
    let original: Option<String> = row.get(9)?;
    let applied: Option<String> = row.get(10)?;
    Ok(CustomAgent {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        config_path: row.get(3)?,
        format: row.get(4)?,
        base_url_key: row.get(5)?,
        auth_key: row.get(6)?,
        model_key: row.get(7)?,
        base_suffix: row.get(8)?,
        original: original.and_then(|text| serde_json::from_str(&text).ok()),
        applied: applied.and_then(|text| serde_json::from_str(&text).ok()),
        wired_ms: row.get(11)?,
    })
}

const COLUMNS: &str = "id, name, slug, config_path, format, base_url_key, auth_key, model_key, base_suffix, original_json, applied_json, wired_ms";

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<CustomAgent>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM custom_agents ORDER BY created_ms ASC"))?;
    let rows = stmt.query_map([], read_row)?;
    rows.collect()
}

pub fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<CustomAgent>> {
    conn.query_row(&format!("SELECT {COLUMNS} FROM custom_agents WHERE id = ?1"), [id], read_row).optional()
}

pub fn add(conn: &Connection, input: &CustomAgentInput, now_ms: i64) -> Result<CustomAgent, String> {
    validate(input)?;
    let base = slugify(&input.name);
    let mut slug = base.clone();
    let mut n = 2;
    while conn
        .query_row("SELECT 1 FROM custom_agents WHERE slug = ?1", [&slug], |_| Ok(()))
        .optional()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        slug = format!("{base}-{n}");
        n += 1;
    }
    let id = format!("{RECORD_PREFIX}{slug}");
    let model_key = input.model_key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    let suffix = input.base_suffix.as_deref().map(str::trim).unwrap_or("");
    conn.execute(
        "INSERT INTO custom_agents(id, name, slug, config_path, format, base_url_key, auth_key, model_key, base_suffix, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![id, input.name.trim(), slug, input.config_path.trim(), input.format, input.base_url_key.trim(), input.auth_key.trim(), model_key, suffix, now_ms],
    )
    .map_err(|e| e.to_string())?;
    load(conn, &id).map_err(|e| e.to_string())?.ok_or_else(|| "保存后读不到这条记录".into())
}

pub fn remove(conn: &Connection, id: &str) -> Result<(), String> {
    let agent = load(conn, id).map_err(|e| e.to_string())?.ok_or("没有这个自定义工具")?;
    if agent.applied.is_some() {
        return Err("请先断开并还原，再删除".into());
    }
    conn.execute("DELETE FROM custom_agents WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
    Ok(())
}

/* ─────────────────────────── 三种格式的键读写（纯函数） ─────────────────────────── */

pub fn get_key(format: &str, text: &str, key: &str) -> Result<Option<String>, String> {
    match format {
        "json" => {
            if text.trim().is_empty() {
                return Ok(None);
            }
            let root: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("不是有效的 JSON：{e}"))?;
            let mut node = &root;
            for part in key.split('.') {
                match node.get(part) {
                    Some(next) => node = next,
                    None => return Ok(None),
                }
            }
            Ok(Some(match node {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            }))
        }
        "toml" => {
            let doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("不是有效的 TOML：{e}"))?;
            let mut item = doc.as_item();
            for part in key.split('.') {
                match item.get(part) {
                    Some(next) => item = next,
                    None => return Ok(None),
                }
            }
            Ok(item.as_value().map(|value| value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string().trim().to_string())))
        }
        _ => Ok(env_lines(text).into_iter().find(|(name, _)| name == key).map(|(_, value)| value)),
    }
}

/// `value = None` 表示删除这个键
pub fn set_key(format: &str, text: &str, key: &str, value: Option<&str>) -> Result<String, String> {
    let parts: Vec<&str> = key.split('.').collect();
    match format {
        "json" => {
            let mut root: serde_json::Value = if text.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(text).map_err(|e| format!("不是有效的 JSON：{e}"))? };
            let (last, parents) = parts.split_last().ok_or("键名为空")?;
            let mut node = &mut root;
            for part in parents {
                if value.is_none() && node.get(*part).is_none() {
                    return Ok(text.to_string());
                }
                let object = node.as_object_mut().ok_or_else(|| format!("{key} 的上一级不是对象"))?;
                node = object.entry(part.to_string()).or_insert_with(|| serde_json::json!({}));
            }
            let object = node.as_object_mut().ok_or_else(|| format!("{key} 的上一级不是对象"))?;
            match value {
                Some(v) => {
                    object.insert(last.to_string(), serde_json::Value::String(v.into()));
                }
                None => {
                    object.remove(*last);
                }
            }
            serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
        }
        "toml" => {
            let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("不是有效的 TOML：{e}"))?;
            let (last, parents) = parts.split_last().ok_or("键名为空")?;
            let mut table = doc.as_table_mut() as &mut dyn toml_edit::TableLike;
            for part in parents {
                if table.get(part).is_none() {
                    if value.is_none() {
                        return Ok(text.to_string());
                    }
                    table.insert(part, toml_edit::Item::Table(toml_edit::Table::new()));
                }
                table = table
                    .get_mut(part)
                    .and_then(|item| item.as_table_like_mut())
                    .ok_or_else(|| format!("{key} 的上一级不是表"))?;
            }
            match value {
                Some(v) => {
                    table.insert(last, toml_edit::value(v));
                }
                None => {
                    table.remove(last);
                }
            }
            Ok(doc.to_string())
        }
        _ => {
            let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
            let index = lines.iter().position(|line| env_name(line).as_deref() == Some(key));
            let rendered = value.map(|v| if v.contains(char::is_whitespace) || v.contains('#') { format!("{key}=\"{v}\"") } else { format!("{key}={v}") });
            match (index, rendered) {
                (Some(i), Some(line)) => lines[i] = line,
                (Some(i), None) => {
                    lines.remove(i);
                }
                (None, Some(line)) => lines.push(line),
                (None, None) => {}
            }
            let mut out = lines.join(nl);
            if !out.is_empty() {
                out.push_str(nl);
            }
            Ok(out)
        }
    }
}

fn env_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (name, _) = line.split_once('=')?;
    Some(name.trim().to_string())
}

fn env_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let name = env_name(line)?;
            let value = line.split_once('=')?.1.trim();
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))).unwrap_or(value);
            Some((name, value.to_string()))
        })
        .collect()
}

/* ─────────────────────────── 接入 / 断开（计划与写盘分开） ─────────────────────────── */

pub fn token(slug: &str, instance_short: &str) -> String {
    format!("jolu-custom-{slug}.{instance_short}")
}

/// 这次要写的值：键 → 值
pub fn plan_values(agent: &CustomAgent, port: u16, instance_short: &str, model: Option<&str>) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    values.insert(agent.base_url_key.clone(), format!("http://127.0.0.1:{port}{}", agent.base_suffix));
    values.insert(agent.auth_key.clone(), token(&agent.slug, instance_short));
    if let (Some(key), Some(model)) = (agent.model_key.as_deref(), model.map(str::trim).filter(|m| !m.is_empty())) {
        values.insert(key.to_string(), model.to_string());
    }
    values
}

/// 接入：返回（新文本、接入前三个键的原值）。已接入时沿用旧快照，不把自己写的值当原值。
pub fn plan_wire(agent: &CustomAgent, text: &str, values: &BTreeMap<String, String>) -> Result<(String, BTreeMap<String, Option<String>>), String> {
    let mut original = BTreeMap::new();
    let mut next = text.to_string();
    for (key, value) in values {
        let before = get_key(&agent.format, text, key)?;
        let ours = agent.applied.as_ref().and_then(|applied| applied.get(key)).is_some_and(|written| Some(written) == before.as_ref());
        let prior = match agent.original.as_ref().and_then(|saved| saved.get(key)) {
            Some(saved) if ours || agent.applied.is_some() => saved.clone(),
            _ => before,
        };
        original.insert(key.clone(), prior);
        next = set_key(&agent.format, &next, key, Some(value))?;
    }
    // 上次写过、这次不写的键（例如这次没选模型）：还原成原值
    if let (Some(applied), Some(saved)) = (agent.applied.as_ref(), agent.original.as_ref()) {
        for (key, written) in applied {
            if values.contains_key(key) {
                continue;
            }
            if get_key(&agent.format, &next, key)?.as_ref() == Some(written) {
                next = set_key(&agent.format, &next, key, saved.get(key).cloned().flatten().as_deref())?;
            }
        }
    }
    Ok((next, original))
}

/// 断开：只还原仍等于 Jolu 写下的值的键。
pub fn plan_unwire(agent: &CustomAgent, text: &str) -> Result<String, String> {
    let Some(applied) = agent.applied.as_ref() else { return Ok(text.to_string()) };
    let mut next = text.to_string();
    for (key, written) in applied {
        if get_key(&agent.format, &next, key)?.as_ref() != Some(written) {
            continue;
        }
        let prior = agent.original.as_ref().and_then(|saved| saved.get(key)).cloned().flatten();
        next = set_key(&agent.format, &next, key, prior.as_deref())?;
    }
    Ok(next)
}

pub fn view(agent: &CustomAgent) -> CustomAgentView {
    let path = expand(&agent.config_path);
    let exists = crate::steady::exists(&path);
    let installed = exists || path.parent().is_some_and(|dir| dir.is_dir());
    let text = if exists { crate::steady::read_to_string(&path).unwrap_or_default() } else { String::new() };
    let written_base = agent.applied.as_ref().and_then(|applied| applied.get(&agent.base_url_key));
    let current_base = get_key(&agent.format, &text, &agent.base_url_key).ok().flatten();
    let wired = written_base.is_some() && written_base == current_base.as_ref();
    let drift = match (agent.applied.is_some(), wired, exists) {
        (true, false, false) => Some("removed".into()),
        (true, false, true) => Some("changed".into()),
        _ => None,
    };
    CustomAgentView {
        id: agent.id.clone(),
        name: agent.name.clone(),
        slug: agent.slug.clone(),
        config_path: crate::gateway::wiring_diff::display_path(&path),
        format: agent.format.clone(),
        base_url_key: agent.base_url_key.clone(),
        auth_key: agent.auth_key.clone(),
        model_key: agent.model_key.clone(),
        base_suffix: agent.base_suffix.clone(),
        installed,
        wired,
        drift,
        model: agent.model_key.as_ref().and_then(|key| agent.applied.as_ref()?.get(key).cloned()),
        wired_ms: agent.wired_ms,
    }
}

fn backup(path: &Path, backup_dir: &Path) -> Result<(), String> {
    if !crate::steady::exists(path) {
        return Ok(());
    }
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("创建备份目录失败：{e}"))?;
    let name = format!("{}-{}.bak", path.file_name().and_then(|n| n.to_str()).unwrap_or("config"), chrono::Local::now().format("%Y%m%d%H%M%S"));
    std::fs::copy(path, backup_dir.join(name)).map_err(|e| format!("备份 {} 失败：{e}", path.display()))?;
    Ok(())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let tmp = path.with_extension("jolu-tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("写入失败：{e}")
    })
}

/// 接入一个自定义工具：备份 → 写文件 → 记快照与写下的值 → 登记接线令牌（局域网严格鉴权用）。
pub fn wire(conn: &Connection, id: &str, port: u16, instance_id: &str, model: Option<&str>, backup_dir: &Path) -> Result<CustomAgentView, String> {
    let agent = load(conn, id).map_err(|e| e.to_string())?.ok_or("没有这个自定义工具")?;
    let instance_short: String = instance_id.chars().take(8).collect();
    let path = expand(&agent.config_path);
    let text = crate::steady::read_optional(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?.unwrap_or_default();
    let values = plan_values(&agent, port, &instance_short, model);
    let (next, original) = plan_wire(&agent, &text, &values)?;
    backup(&path, backup_dir)?;
    write(&path, &next)?;
    let now = chrono::Local::now().timestamp_millis();
    conn.execute(
        "UPDATE custom_agents SET original_json = ?2, applied_json = ?3, wired_ms = ?4 WHERE id = ?1",
        params![id, serde_json::to_string(&original).map_err(|e| e.to_string())?, serde_json::to_string(&values).map_err(|e| e.to_string())?, now],
    )
    .map_err(|e| e.to_string())?;
    let applied = crate::gateway::wiring_store::Applied {
        base: values.get(&agent.base_url_key).cloned().unwrap_or_default(),
        token: values.get(&agent.auth_key).cloned(),
        ..Default::default()
    };
    crate::gateway::wiring_store::mark_pending(conn, id, instance_id, "relay", None, &applied, applied.token.as_deref()).map_err(|e| e.to_string())?;
    crate::gateway::wiring_store::mark_wired(conn, id, now).map_err(|e| e.to_string())?;
    let agent = load(conn, id).map_err(|e| e.to_string())?.ok_or("没有这个自定义工具")?;
    Ok(view(&agent))
}

/// 断开并还原。返回 true = 改过文件。
pub fn unwire(conn: &Connection, id: &str, backup_dir: &Path) -> Result<bool, String> {
    let agent = load(conn, id).map_err(|e| e.to_string())?.ok_or("没有这个自定义工具")?;
    let path = expand(&agent.config_path);
    let mut changed = false;
    if let Some(text) = crate::steady::read_optional(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))? {
        let next = plan_unwire(&agent, &text)?;
        if next != text {
            backup(&path, backup_dir)?;
            write(&path, &next)?;
            changed = true;
        }
    }
    conn.execute("UPDATE custom_agents SET original_json = NULL, applied_json = NULL, wired_ms = NULL WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
    let _ = crate::gateway::wiring_store::mark_idle(conn, id);
    Ok(changed)
}

/// 改动预览：已接入 = Jolu 改了什么（还原后 → 现在）；未接入 = 将要写成什么。
pub fn preview(conn: &Connection, id: &str, port: u16, instance_id: &str, model: Option<&str>) -> Result<crate::gateway::escape::WiringPreview, String> {
    let agent = load(conn, id).map_err(|e| e.to_string())?.ok_or("没有这个自定义工具")?;
    let path = expand(&agent.config_path);
    let text = crate::steady::read_optional(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let current = text.clone().unwrap_or_default();
    if view(&agent).wired && model.is_none() {
        let restored = plan_unwire(&agent, &current)?;
        return Ok(crate::gateway::escape::WiringPreview { mode: "wired", files: vec![crate::gateway::wiring_diff::diff_file(&path, Some(&restored), &current)] });
    }
    let instance_short: String = instance_id.chars().take(8).collect();
    let (next, _) = plan_wire(&agent, &current, &plan_values(&agent, port, &instance_short, model))?;
    Ok(crate::gateway::escape::WiringPreview { mode: "wire", files: vec![crate::gateway::wiring_diff::diff_file(&path, text.as_deref(), &next)] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(format: &str, base: &str, auth: &str, model: Option<&str>) -> CustomAgent {
        CustomAgent {
            id: "custom-agent:crush".into(), name: "Crush".into(), slug: "crush".into(), config_path: "~/x".into(), format: format.into(),
            base_url_key: base.into(), auth_key: auth.into(), model_key: model.map(str::to_string), base_suffix: "/v1".into(),
            original: None, applied: None, wired_ms: None,
        }
    }

    fn round_trip(format: &str, original: &str, base: &str, auth: &str, model: Option<&str>) -> (String, String) {
        let mut a = agent(format, base, auth, model);
        let values = plan_values(&a, 15721, "abcd1234", Some("group/fast"));
        let (wired, saved) = plan_wire(&a, original, &values).unwrap();
        assert_eq!(get_key(format, &wired, base).unwrap().as_deref(), Some("http://127.0.0.1:15721/v1"));
        assert_eq!(get_key(format, &wired, auth).unwrap().as_deref(), Some("jolu-custom-crush.abcd1234"));
        if let Some(key) = model {
            assert_eq!(get_key(format, &wired, key).unwrap().as_deref(), Some("group/fast"));
        }
        a.original = Some(saved);
        a.applied = Some(values);
        let restored = plan_unwire(&a, &wired).unwrap();
        (wired, restored)
    }

    #[test]
    fn json_wire_and_unwire_restore_only_our_keys() {
        let original = r#"{"providers":{"openai":{"base_url":"https://api.openai.com/v1","api_key":"sk-user"}},"other":1}"#;
        let (_, restored) = round_trip("json", original, "providers.openai.base_url", "providers.openai.api_key", Some("model"));
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["providers"]["openai"]["base_url"], "https://api.openai.com/v1");
        assert_eq!(value["providers"]["openai"]["api_key"], "sk-user", "用户自己的 Key 原样回来");
        assert!(value.get("model").is_none(), "原本没有的键还原后删掉");
        assert_eq!(value["other"], 1);
    }

    #[test]
    fn toml_and_env_round_trip() {
        let toml = "model = \"gpt-4o\"\n\n[provider]\nname = \"x\"\n";
        let (wired, restored) = round_trip("toml", toml, "provider.base_url", "provider.api_key", Some("model"));
        assert!(wired.contains("[provider]"));
        assert_eq!(get_key("toml", &restored, "model").unwrap().as_deref(), Some("gpt-4o"));
        assert_eq!(get_key("toml", &restored, "provider.base_url").unwrap(), None);
        assert_eq!(get_key("toml", &restored, "provider.name").unwrap().as_deref(), Some("x"));

        let env = "# 注释\nOPENAI_API_KEY=sk-user\nOTHER=1\n";
        let (wired, restored) = round_trip("env", env, "OPENAI_BASE_URL", "OPENAI_API_KEY", None);
        assert!(wired.contains("OPENAI_BASE_URL=http://127.0.0.1:15721/v1"));
        assert_eq!(restored, env, "env 文件原样还原");
    }

    #[test]
    fn unwire_leaves_values_the_user_changed_after_wiring() {
        let mut a = agent("env", "BASE", "KEY", None);
        let values = plan_values(&a, 15721, "abcd1234", None);
        let (wired, saved) = plan_wire(&a, "BASE=https://old\n", &values).unwrap();
        a.original = Some(saved);
        a.applied = Some(values);
        let edited = set_key("env", &wired, "BASE", Some("https://user-new")).unwrap();
        let restored = plan_unwire(&a, &edited).unwrap();
        assert_eq!(get_key("env", &restored, "BASE").unwrap().as_deref(), Some("https://user-new"), "用户接入后自己改的不还原");
        assert_eq!(get_key("env", &restored, "KEY").unwrap(), None, "我们写的令牌删掉");
    }

    #[test]
    fn rewire_keeps_the_first_snapshot() {
        let mut a = agent("env", "BASE", "KEY", None);
        let values = plan_values(&a, 15721, "abcd1234", None);
        let (wired, saved) = plan_wire(&a, "BASE=https://old\n", &values).unwrap();
        a.original = Some(saved);
        a.applied = Some(values.clone());
        let (_, again) = plan_wire(&a, &wired, &values).unwrap();
        assert_eq!(again.get("BASE").cloned().flatten().as_deref(), Some("https://old"), "重新接入不把自己写的值当原值");
    }

    #[test]
    fn validation_and_storage() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let input = CustomAgentInput { name: "Crush CLI".into(), config_path: "~/.config/crush/crush.json".into(), format: "json".into(), base_url_key: "providers.jolu.base_url".into(), auth_key: "providers.jolu.api_key".into(), model_key: Some("".into()), base_suffix: Some("/v1".into()) };
        let first = add(&conn, &input, 1).unwrap();
        assert_eq!(first.id, "custom-agent:crush-cli");
        assert_eq!(first.model_key, None, "空的模型键不存");
        assert_eq!(add(&conn, &input, 2).unwrap().slug, "crush-cli-2", "重名自动加序号");
        assert!(validate(&CustomAgentInput { format: "yaml".into(), ..input.clone() }).is_err());
        assert!(validate(&CustomAgentInput { config_path: "relative/path.json".into(), ..input.clone() }).is_err());
        assert!(validate(&CustomAgentInput { format: "env".into(), base_url_key: "A.B".into(), ..input.clone() }).is_err());
        assert!(validate(&CustomAgentInput { base_suffix: Some("v1".into()), ..input.clone() }).is_err());
        conn.execute("UPDATE custom_agents SET applied_json='{}' WHERE id=?1", [&first.id]).unwrap();
        assert!(remove(&conn, &first.id).is_err(), "接入中不能删");
    }
}
