//! Library 5.1 第二批：共享指令与子代理（参照 magpie `internal/library/instructions.go`、
//! `subagents.go`、`targets.go`）。无画板，界面与范围标「待确认」。
//!
//! - **共享指令**：一段文本，写进各 agent 的用户级指令文件（CLAUDE.md / AGENTS.md / GEMINI.md /
//!   QWEN.md）里由标记行包住的 Jolu 区块；区块外是用户自己的内容，永不改动。
//! - **子代理**：每个一份定义（名称、描述、提示词、可选模型），按各 agent 自己的格式写成文件
//!   （Claude Code / Gemini / Qwen / Droid 是带 frontmatter 的 md，OpenCode 加 `mode: subagent`，
//!   Codex 是 `agents/<名>.toml` 的 role 文件，只有它认 model）。
//!
//! 归属纪律（magpie 同款）：Jolu 写下的每一处都记哈希（`library_files`）。现在的内容还等于
//! 我们写的才能重写或删除；用户改过就算他的，跳过并说明；同名文件不是 Jolu 写的，永不覆盖。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const BLOCK_BEGIN: &str = "<!-- jolu:begin · 由 Jolu Library 写入，区块外的内容不会被改动 -->";
const BLOCK_END: &str = "<!-- jolu:end -->";

/// 共享指令能写的 agent（用户级指令文件的出处见 magpie targets.go）
pub const INSTRUCTION_AGENTS: &[&str] = &["claude-code", "codex", "gemini", "opencode", "qwen", "droid"];
/// 子代理能写的 agent（magpie subagents.go `SubagentAgents` 与 Jolu 能探测的交集）
pub const SUBAGENT_AGENTS: &[&str] = &["claude-code", "codex", "gemini", "opencode", "qwen", "droid"];

const RESERVED_NAMES: &[&str] = &["self", "system", "user", "model", "tool", "config", "default", "main"];

pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS library_instructions (
            id          TEXT PRIMARY KEY,
            text        TEXT NOT NULL,
            agents_json TEXT NOT NULL,
            updated_ms  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS library_subagents (
            name        TEXT PRIMARY KEY,
            description TEXT NOT NULL,
            prompt      TEXT NOT NULL,
            model       TEXT,
            agents_json TEXT NOT NULL,
            updated_ms  INTEGER NOT NULL
        );
        -- Jolu 写进 agent 目录的每一处：kind = instructions / subagent，owner = 'default' 或子代理名
        CREATE TABLE IF NOT EXISTS library_files (
            kind       TEXT NOT NULL,
            owner      TEXT NOT NULL,
            agent      TEXT NOT NULL,
            path       TEXT NOT NULL,
            hash       TEXT NOT NULL,
            written_ms INTEGER NOT NULL,
            PRIMARY KEY (kind, owner, agent)
        );",
    )
}

fn hash(text: &str) -> String {
    let digest = Sha256::digest(text.replace("\r\n", "\n").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// 各 agent 的用户级配置目录（指令文件与子代理目录都在这下面）
fn agent_dir(agent: &str) -> Option<PathBuf> {
    let home = crate::creds::home()?;
    Some(match agent {
        "claude-code" => crate::creds::claude_dir()?,
        "codex" => crate::creds::codex_dir()?,
        "gemini" => home.join(".gemini"),
        "opencode" => crate::gateway::wiring::config_path_for("opencode")?.parent()?.to_path_buf(),
        "qwen" => env_dir("QWEN_HOME").unwrap_or_else(|| home.join(".qwen")),
        "droid" => home.join(".factory"),
        _ => return None,
    })
}

pub fn instruction_path(agent: &str) -> Option<PathBuf> {
    let file = match agent {
        "claude-code" => "CLAUDE.md",
        "codex" | "opencode" | "droid" => "AGENTS.md",
        "gemini" => "GEMINI.md",
        "qwen" => "QWEN.md",
        _ => return None,
    };
    Some(agent_dir(agent)?.join(file))
}

pub fn subagent_path(agent: &str, name: &str) -> Option<PathBuf> {
    let (dir, ext) = match agent {
        "claude-code" | "gemini" | "opencode" | "qwen" => ("agents", ".md"),
        "codex" => ("agents", ".toml"),
        "droid" => ("droids", ".md"),
        _ => return None,
    };
    Some(agent_dir(agent)?.join(dir).join(format!("{name}{ext}")))
}

/* ─────────────────────────── 指令区块（纯函数） ─────────────────────────── */

/// 拆出 Jolu 区块：（区块前、区块内文本、区块后）；没有区块返回 None
fn split_block(file: &str) -> Option<(&str, &str, &str)> {
    let start = file.find(BLOCK_BEGIN)?;
    let rest = &file[start + BLOCK_BEGIN.len()..];
    let end = rest.find(BLOCK_END)?;
    Some((&file[..start], rest[..end].trim(), &rest[end + BLOCK_END.len()..]))
}

/// 文件里 Jolu 区块的文本（用来比对哈希）
pub fn block_text(file: &str) -> Option<String> {
    split_block(&file.replace("\r\n", "\n")).map(|(_, block, _)| block.to_string())
}

/// 把 Jolu 区块设为 `text`（空串 = 移除）；新区块放在用户内容之后。保持文件原有的换行风格。
pub fn with_block(file: &str, text: &str) -> String {
    let crlf = file.contains("\r\n");
    let file = file.replace("\r\n", "\n");
    let (before, after) = match split_block(&file) {
        Some((before, _, after)) => (before.trim_end_matches('\n').to_string(), after.trim_start_matches('\n').trim_end_matches('\n').to_string()),
        None => (file.trim_end_matches('\n').to_string(), String::new()),
    };
    let block = if text.trim().is_empty() { String::new() } else { format!("{BLOCK_BEGIN}\n{}\n{BLOCK_END}", text.trim()) };
    let parts: Vec<&str> = [before.as_str(), block.as_str(), after.as_str()].into_iter().filter(|p| !p.trim().is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    let out = parts.join("\n\n") + "\n";
    if crlf { out.replace('\n', "\r\n") } else { out }
}

/* ─────────────────────────── 子代理渲染（纯函数） ─────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentDef {
    pub name: String,
    pub description: String,
    pub prompt: String,
    /// 只有 Codex 认（它的 role 文件是一层配置，model 会压过主代理选的）；其余工具忽略
    pub model: Option<String>,
    pub agents: Vec<String>,
}

/// YAML 标量：JSON 的带引号字符串就是合法 YAML（magpie yamlStr）
fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

pub fn render_subagent(agent: &str, def: &SubagentDef) -> Option<String> {
    let prompt = def.prompt.trim_end_matches('\n');
    let md = |keys: Vec<(&str, String)>| {
        let mut out = String::from("---\n");
        for (key, value) in keys {
            out.push_str(&format!("{key}: {value}\n"));
        }
        out.push_str("---\n\n");
        out.push_str(prompt);
        out.push('\n');
        out
    };
    Some(match agent {
        "claude-code" | "gemini" | "qwen" | "droid" => md(vec![("name", yaml_str(&def.name)), ("description", yaml_str(&def.description))]),
        // OpenCode 按文件名认子代理；mode: subagent 让主代理可以调用它
        "opencode" => md(vec![("description", yaml_str(&def.description)), ("mode", "subagent".into())]),
        "codex" => {
            let mut doc = toml_edit::DocumentMut::new();
            doc["name"] = toml_edit::value(def.name.as_str());
            doc["description"] = toml_edit::value(def.description.as_str());
            if let Some(model) = def.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                doc["model"] = toml_edit::value(model);
            }
            doc["developer_instructions"] = toml_edit::value(format!("{prompt}\n"));
            doc.to_string()
        }
        _ => return None,
    })
}

pub fn validate_subagent(def: &SubagentDef) -> Result<(), String> {
    let name = def.name.as_str();
    let ok_chars = name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !(2..=50).contains(&name.len()) || !ok_chars || name.starts_with('-') || name.ends_with('-') {
        return Err("名称用 2–50 个小写字母、数字或 -，不能以 - 开头或结尾".into());
    }
    if RESERVED_NAMES.contains(&name) {
        return Err(format!("「{name}」是工具自己保留的名字"));
    }
    if def.description.trim().is_empty() {
        return Err("要写描述：主代理靠它判断什么时候调用这个子代理".into());
    }
    if def.description.chars().count() > 1000 {
        return Err("描述最多 1000 字".into());
    }
    if def.prompt.trim().is_empty() {
        return Err("要写提示词".into());
    }
    if def.prompt.len() > 100_000 {
        return Err("提示词太长（最多 100 KB）".into());
    }
    if def.model.as_deref().is_some_and(|m| m.len() > 200 || m.contains(char::is_whitespace)) {
        return Err("模型写模型 id，不要有空格".into());
    }
    if let Some(agent) = def.agents.iter().find(|a| !SUBAGENT_AGENTS.contains(&a.as_str())) {
        return Err(format!("{agent} 不支持写子代理"));
    }
    Ok(())
}

/* ─────────────────────────── 落盘与记账 ─────────────────────────── */

/// 一个 agent 一处写入的结果
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileOutcome {
    pub agent: String,
    pub path: Option<String>,
    /// written / removed / unchanged / skipped
    pub action: &'static str,
    /// skipped 时的原因
    pub reason: Option<String>,
}

fn outcome(agent: &str, path: Option<&Path>, action: &'static str, reason: Option<String>) -> FileOutcome {
    FileOutcome { agent: agent.into(), path: path.map(|p| p.display().to_string()), action, reason }
}

fn recorded(conn: &Connection, kind: &str, owner: &str, agent: &str) -> Option<(String, String)> {
    conn.query_row(
        "SELECT path, hash FROM library_files WHERE kind = ?1 AND owner = ?2 AND agent = ?3",
        params![kind, owner, agent],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

fn record(conn: &Connection, kind: &str, owner: &str, agent: &str, path: &Path, hash: &str, now: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO library_files(kind, owner, agent, path, hash, written_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, owner, agent) DO UPDATE SET path = excluded.path, hash = excluded.hash, written_ms = excluded.written_ms",
        params![kind, owner, agent, path.display().to_string(), hash, now],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn forget(conn: &Connection, kind: &str, owner: &str, agent: &str) {
    let _ = conn.execute("DELETE FROM library_files WHERE kind = ?1 AND owner = ?2 AND agent = ?3", params![kind, owner, agent]);
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    crate::steady::read_optional(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))
}

/// 一处写入目标的现状（界面用）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TargetStatus {
    pub agent: String,
    pub path: Option<String>,
    /// Jolu 写过（有记账）
    pub written: bool,
    /// 写过、但现在的内容和我们写的不一样（用户改过）或已被删掉
    pub changed: bool,
}

pub fn instruction_status(conn: &Connection, agent: &str, path: Option<&Path>) -> TargetStatus {
    let rec = recorded(conn, "instructions", "default", agent);
    let changed = match (rec.as_ref(), path) {
        (Some((_, h)), Some(path)) => read_optional(path).ok().flatten().and_then(|text| block_text(&text)).map(|block| hash(&block) != *h).unwrap_or(true),
        _ => false,
    };
    TargetStatus { agent: agent.into(), path: path.map(|p| p.display().to_string()), written: rec.is_some(), changed }
}

pub fn subagent_status(conn: &Connection, name: &str, agent: &str) -> TargetStatus {
    let rec = recorded(conn, "subagent", name, agent);
    let changed = rec.as_ref().is_some_and(|(path, h)| read_optional(Path::new(path)).ok().flatten().map(|text| hash(&text) != *h).unwrap_or(true));
    let path = rec.as_ref().map(|(p, _)| p.clone()).or_else(|| subagent_path(agent, name).map(|p| p.display().to_string()));
    TargetStatus { agent: agent.into(), path, written: rec.is_some(), changed }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    crate::gateway::wiring::write_json_atomic_public(path, text.as_bytes().to_vec())
}

/// 把共享指令写到一个 agent（`text` 为空 = 移除区块）。`path` 由调用方给，便于测试。
pub fn apply_instructions_at(conn: &Connection, agent: &str, path: &Path, text: &str, now: i64) -> FileOutcome {
    let current = match read_optional(path) {
        Ok(current) => current.unwrap_or_default(),
        Err(reason) => return outcome(agent, Some(path), "skipped", Some(reason)),
    };
    let existing = block_text(&current);
    if let Some(existing) = existing.as_deref() {
        // 区块在，但跟我们上次写的不一样 = 用户在区块里改过：不覆盖、不删除
        let ours = recorded(conn, "instructions", "default", agent).is_some_and(|(_, h)| h == hash(existing));
        if !ours {
            return outcome(agent, Some(path), "skipped", Some("Jolu 区块里的内容被改过，未覆盖；要重新写入请先手动删掉这个区块".into()));
        }
    }
    let text = text.trim();
    if existing.as_deref() == Some(text) || (existing.is_none() && text.is_empty()) {
        if text.is_empty() {
            forget(conn, "instructions", "default", agent);
        }
        return outcome(agent, Some(path), "unchanged", None);
    }
    let next = with_block(&current, text);
    if let Err(reason) = write(path, &next) {
        return outcome(agent, Some(path), "skipped", Some(reason));
    }
    if text.is_empty() {
        forget(conn, "instructions", "default", agent);
        outcome(agent, Some(path), "removed", None)
    } else {
        if let Err(reason) = record(conn, "instructions", "default", agent, path, &hash(text), now) {
            return outcome(agent, Some(path), "skipped", Some(reason));
        }
        outcome(agent, Some(path), "written", None)
    }
}

/// 写一个子代理文件（`def = None` = 删除）。
pub fn apply_subagent_at(conn: &Connection, agent: &str, name: &str, path: &Path, def: Option<&SubagentDef>, now: i64) -> FileOutcome {
    let current = match read_optional(path) {
        Ok(current) => current,
        Err(reason) => return outcome(agent, Some(path), "skipped", Some(reason)),
    };
    let ours = recorded(conn, "subagent", name, agent);
    if let Some(current) = current.as_deref() {
        match ours.as_ref() {
            None => return outcome(agent, Some(path), "skipped", Some("同名文件不是 Jolu 写的，不覆盖".into())),
            Some((_, h)) if *h != hash(current) => {
                return outcome(agent, Some(path), "skipped", Some("文件写入后被改过，算你自己的，不再改动".into()));
            }
            _ => {}
        }
    }
    match def {
        None => {
            if current.is_none() {
                forget(conn, "subagent", name, agent);
                return outcome(agent, Some(path), "unchanged", None);
            }
            if let Err(error) = std::fs::remove_file(path) {
                return outcome(agent, Some(path), "skipped", Some(format!("删除失败：{error}")));
            }
            forget(conn, "subagent", name, agent);
            outcome(agent, Some(path), "removed", None)
        }
        Some(def) => {
            let Some(text) = render_subagent(agent, def) else {
                return outcome(agent, Some(path), "skipped", Some("这个工具不支持子代理".into()));
            };
            if current.as_deref().map(|c| c.replace("\r\n", "\n")) == Some(text.clone()) {
                return outcome(agent, Some(path), "unchanged", None);
            }
            if let Err(reason) = write(path, &text) {
                return outcome(agent, Some(path), "skipped", Some(reason));
            }
            if let Err(reason) = record(conn, "subagent", name, agent, path, &hash(&text), now) {
                return outcome(agent, Some(path), "skipped", Some(reason));
            }
            outcome(agent, Some(path), "written", None)
        }
    }
}

/* ─────────────────────────── 定义的读写 ─────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct InstructionsDef {
    pub text: String,
    pub agents: Vec<String>,
}

pub fn load_instructions(conn: &Connection) -> InstructionsDef {
    conn.query_row("SELECT text, agents_json FROM library_instructions WHERE id = 'default'", [], |r| {
        Ok(InstructionsDef { text: r.get(0)?, agents: serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or_default() })
    })
    .optional()
    .ok()
    .flatten()
    .unwrap_or_default()
}

pub fn list_subagents(conn: &Connection) -> Vec<SubagentDef> {
    let Ok(mut stmt) = conn.prepare("SELECT name, description, prompt, model, agents_json FROM library_subagents ORDER BY name") else { return Vec::new() };
    stmt.query_map([], |r| {
        Ok(SubagentDef {
            name: r.get(0)?,
            description: r.get(1)?,
            prompt: r.get(2)?,
            model: r.get(3)?,
            agents: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
        })
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// 保存共享指令并同步到各 agent：勾选的写入，没勾选但写过的移除。
pub fn save_instructions(conn: &Connection, def: &InstructionsDef, now: i64, path_of: &dyn Fn(&str) -> Option<PathBuf>) -> Result<Vec<FileOutcome>, String> {
    if let Some(agent) = def.agents.iter().find(|a| !INSTRUCTION_AGENTS.contains(&a.as_str())) {
        return Err(format!("{agent} 不支持共享指令"));
    }
    if def.text.len() > 200_000 {
        return Err("指令太长（最多 200 KB）".into());
    }
    conn.execute(
        "INSERT INTO library_instructions(id, text, agents_json, updated_ms) VALUES ('default', ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET text = excluded.text, agents_json = excluded.agents_json, updated_ms = excluded.updated_ms",
        params![def.text, serde_json::to_string(&def.agents).unwrap_or_else(|_| "[]".into()), now],
    )
    .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for agent in INSTRUCTION_AGENTS {
        let wanted = def.agents.iter().any(|a| a == agent);
        let had = recorded(conn, "instructions", "default", agent).is_some();
        if !wanted && !had {
            continue;
        }
        let Some(path) = path_of(agent) else {
            out.push(outcome(agent, None, "skipped", Some("找不到这个工具的配置目录".into())));
            continue;
        };
        out.push(apply_instructions_at(conn, agent, &path, if wanted { &def.text } else { "" }, now));
    }
    Ok(out)
}

/// 保存一个子代理并同步：勾选的写入，取消勾选的删除（只删 Jolu 写的、没被改过的）。
pub fn save_subagent(conn: &Connection, def: &SubagentDef, now: i64, path_of: &dyn Fn(&str, &str) -> Option<PathBuf>) -> Result<Vec<FileOutcome>, String> {
    validate_subagent(def)?;
    conn.execute(
        "INSERT INTO library_subagents(name, description, prompt, model, agents_json, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(name) DO UPDATE SET description = excluded.description, prompt = excluded.prompt, model = excluded.model,
             agents_json = excluded.agents_json, updated_ms = excluded.updated_ms",
        params![def.name, def.description.trim(), def.prompt, def.model.as_deref().map(str::trim).filter(|m| !m.is_empty()), serde_json::to_string(&def.agents).unwrap_or_else(|_| "[]".into()), now],
    )
    .map_err(|e| e.to_string())?;
    Ok(sync_subagent(conn, &def.name, Some(def), now, path_of))
}

/// 删除子代理定义：先删掉各 agent 里 Jolu 写的文件；改过的文件留着并在结果里说明。
pub fn remove_subagent(conn: &Connection, name: &str, now: i64, path_of: &dyn Fn(&str, &str) -> Option<PathBuf>) -> Vec<FileOutcome> {
    let out = sync_subagent(conn, name, None, now, path_of);
    let _ = conn.execute("DELETE FROM library_subagents WHERE name = ?1", [name]);
    // 留下的（被改过的）文件不再归 Jolu
    let _ = conn.execute("DELETE FROM library_files WHERE kind = 'subagent' AND owner = ?1", [name]);
    out
}

fn sync_subagent(conn: &Connection, name: &str, def: Option<&SubagentDef>, now: i64, path_of: &dyn Fn(&str, &str) -> Option<PathBuf>) -> Vec<FileOutcome> {
    let mut out = Vec::new();
    for agent in SUBAGENT_AGENTS {
        let wanted = def.is_some_and(|d| d.agents.iter().any(|a| a == agent));
        let had = recorded(conn, "subagent", name, agent);
        if !wanted && had.is_none() {
            continue;
        }
        // 写过的按记下的路径删（目录可能已换），要写的按现在的路径
        let path = if wanted { path_of(agent, name) } else { had.map(|(p, _)| PathBuf::from(p)) };
        let Some(path) = path else {
            out.push(outcome(agent, None, "skipped", Some("找不到这个工具的配置目录".into())));
            continue;
        };
        out.push(apply_subagent_at(conn, agent, name, &path, if wanted { def } else { None }, now));
    }
    out
}

/// 卸载钩子：移除全部 Jolu 写的指令区块与子代理文件（被改过的留着），返回日志行。
pub fn restore_all(conn: &Connection) -> Vec<String> {
    let rows: Vec<(String, String, String, String)> = conn
        .prepare("SELECT kind, owner, agent, path FROM library_files")
        .and_then(|mut stmt| stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map(|rows| rows.flatten().collect()))
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp_millis();
    rows.into_iter()
        .map(|(kind, owner, agent, path)| {
            let path = PathBuf::from(path);
            let result = if kind == "instructions" { apply_instructions_at(conn, &agent, &path, "", now) } else { apply_subagent_at(conn, &agent, &owner, &path, None, now) };
            format!("library {kind} {owner} @ {agent}: {}{}", result.action, result.reason.map(|r| format!("（{r}）")).unwrap_or_default())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-library-shared-{}-{name}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn block_round_trip_keeps_user_text_and_line_endings() {
        let user = "# 我的规则\r\n\r\n- 用中文\r\n";
        let with = with_block(user, "共享内容");
        assert!(with.contains("\r\n"), "保持 CRLF");
        assert!(with.starts_with("# 我的规则"));
        assert_eq!(block_text(&with).as_deref(), Some("共享内容"));
        let replaced = with_block(&with, "新内容");
        assert_eq!(block_text(&replaced).as_deref(), Some("新内容"));
        assert_eq!(replaced.matches(BLOCK_BEGIN).count(), 1);
        let removed = with_block(&replaced, "");
        assert_eq!(removed, "# 我的规则\r\n\r\n- 用中文\r\n");
        assert_eq!(with_block("", ""), "");
        assert_eq!(block_text(&with_block("", "x")).as_deref(), Some("x"));
    }

    #[test]
    fn block_in_the_middle_keeps_text_after_it() {
        let file = format!("前\n\n{BLOCK_BEGIN}\n旧\n{BLOCK_END}\n\n后\n");
        assert_eq!(with_block(&file, ""), "前\n\n后\n");
        assert_eq!(with_block(&file, "新"), format!("前\n\n{BLOCK_BEGIN}\n新\n{BLOCK_END}\n\n后\n"));
    }

    #[test]
    fn instructions_write_update_remove_and_respect_user_edits() {
        let conn = conn();
        let dir = temp("instr");
        let path = dir.join("CLAUDE.md");
        std::fs::write(&path, "我的内容\n").unwrap();
        assert_eq!(apply_instructions_at(&conn, "claude-code", &path, "规则 A", 1).action, "written");
        assert_eq!(apply_instructions_at(&conn, "claude-code", &path, "规则 A", 2).action, "unchanged");
        assert_eq!(apply_instructions_at(&conn, "claude-code", &path, "规则 B", 3).action, "written");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("我的内容\n\n") && text.contains("规则 B"));

        // 用户改了区块里的内容：不覆盖、不删除
        std::fs::write(&path, text.replace("规则 B", "我改的")).unwrap();
        let skipped = apply_instructions_at(&conn, "claude-code", &path, "规则 C", 4);
        assert_eq!(skipped.action, "skipped");
        assert_eq!(apply_instructions_at(&conn, "claude-code", &path, "", 5).action, "skipped");
        assert!(std::fs::read_to_string(&path).unwrap().contains("我改的"));

        // 改回来后能移除，只剩用户的内容
        std::fs::write(&path, text).unwrap();
        assert_eq!(apply_instructions_at(&conn, "claude-code", &path, "", 6).action, "removed");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "我的内容\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn instructions_save_syncs_ticked_and_unticked_agents() {
        let conn = conn();
        let dir = temp("save");
        let path_of = |agent: &str| Some(dir.join(format!("{agent}.md")));
        let def = InstructionsDef { text: "共享".into(), agents: vec!["claude-code".into(), "codex".into()] };
        let out = save_instructions(&conn, &def, 1, &path_of).unwrap();
        assert_eq!(out.iter().map(|o| (o.agent.as_str(), o.action)).collect::<Vec<_>>(), vec![("claude-code", "written"), ("codex", "written")]);
        let out = save_instructions(&conn, &InstructionsDef { text: "共享".into(), agents: vec!["codex".into()] }, 2, &path_of).unwrap();
        assert_eq!(out.iter().map(|o| (o.agent.as_str(), o.action)).collect::<Vec<_>>(), vec![("claude-code", "removed"), ("codex", "unchanged")]);
        assert_eq!(load_instructions(&conn).agents, vec!["codex".to_string()]);
        assert!(save_instructions(&conn, &InstructionsDef { text: "x".into(), agents: vec!["trae".into()] }, 3, &path_of).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn def(agents: &[&str]) -> SubagentDef {
        SubagentDef {
            name: "code-reviewer".into(),
            description: "审查改动里的正确性问题".into(),
            prompt: "你是代码审查员。\n只报告确定的问题。".into(),
            model: Some("kimi/k2".into()),
            agents: agents.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn subagent_formats_per_agent() {
        let d = def(&[]);
        let claude = render_subagent("claude-code", &d).unwrap();
        assert!(claude.starts_with("---\nname: \"code-reviewer\"\ndescription: \"审查改动里的正确性问题\"\n---\n\n你是代码审查员。"));
        assert!(!claude.contains("kimi/k2"), "只有 Codex 写模型");
        let opencode = render_subagent("opencode", &d).unwrap();
        assert!(opencode.contains("mode: subagent") && !opencode.contains("name:"));
        let codex: toml_edit::DocumentMut = render_subagent("codex", &d).unwrap().parse().unwrap();
        assert_eq!(codex["model"].as_str(), Some("kimi/k2"));
        assert_eq!(codex["developer_instructions"].as_str(), Some("你是代码审查员。\n只报告确定的问题。\n"));
        assert!(render_subagent("trae", &d).is_none());
        assert_eq!(subagent_path("droid", "x").map(|p| p.parent().unwrap().file_name().unwrap().to_owned()), Some("droids".into()));
    }

    #[test]
    fn subagent_names_are_checked() {
        for bad in ["A", "-ab", "ab-", "a b", "main", "x"] {
            let mut d = def(&["claude-code"]);
            d.name = bad.into();
            assert!(validate_subagent(&d).is_err(), "{bad}");
        }
        let mut d = def(&["trae"]);
        assert!(validate_subagent(&d).is_err());
        d.agents = vec!["claude-code".into()];
        d.description = " ".into();
        assert!(validate_subagent(&d).is_err());
    }

    #[test]
    fn subagent_save_untick_and_remove_respect_ownership() {
        let conn = conn();
        let dir = temp("sub");
        let path_of = |agent: &str, name: &str| Some(dir.join(agent).join(format!("{name}.md")));
        // claude-code 已有同名文件（用户的）
        std::fs::create_dir_all(dir.join("claude-code")).unwrap();
        std::fs::write(dir.join("claude-code").join("code-reviewer.md"), "我的").unwrap();
        let out = save_subagent(&conn, &def(&["claude-code", "gemini", "qwen"]), 1, &path_of).unwrap();
        assert_eq!(out.iter().map(|o| (o.agent.as_str(), o.action)).collect::<Vec<_>>(), vec![("claude-code", "skipped"), ("gemini", "written"), ("qwen", "written")]);
        assert_eq!(std::fs::read_to_string(dir.join("claude-code").join("code-reviewer.md")).unwrap(), "我的");

        // 取消 qwen：删掉 Jolu 写的文件
        let out = save_subagent(&conn, &def(&["gemini"]), 2, &path_of).unwrap();
        assert!(out.iter().any(|o| o.agent == "qwen" && o.action == "removed"));
        assert!(!dir.join("qwen").join("code-reviewer.md").exists());

        // gemini 的被用户改过：删除定义时留着
        std::fs::write(dir.join("gemini").join("code-reviewer.md"), "改过").unwrap();
        let out = remove_subagent(&conn, "code-reviewer", 3, &path_of);
        assert_eq!(out.iter().map(|o| (o.agent.as_str(), o.action)).collect::<Vec<_>>(), vec![("gemini", "skipped")]);
        assert!(dir.join("gemini").join("code-reviewer.md").exists());
        assert!(list_subagents(&conn).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restore_all_removes_everything_we_still_own() {
        let conn = conn();
        let dir = temp("restore");
        let instr = dir.join("AGENTS.md");
        std::fs::write(&instr, "用户\n").unwrap();
        apply_instructions_at(&conn, "codex", &instr, "共享", 1);
        let sub = dir.join("r.toml");
        apply_subagent_at(&conn, "codex", "code-reviewer", &sub, Some(&def(&["codex"])), 1);
        assert!(sub.exists());
        let lines = restore_all(&conn);
        assert_eq!(lines.len(), 2);
        assert_eq!(std::fs::read_to_string(&instr).unwrap(), "用户\n");
        assert!(!sub.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
