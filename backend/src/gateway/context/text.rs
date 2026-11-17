//! 文本里的指令文件、技能列表、提醒与 harness 消息（移植 magpie `splitMemory` / `splitSkills` / `systemText` /
//! `userText` / `userWords`；不引入正则，按行与前缀扫描）。
//!
//! Jolu 相对 magpie 的补充（本机 2026-11-17 抓的真实请求为依据）：
//! - Gemini CLI 的 `--- Context from: <路径> ---` … `--- End of Context from: <路径> ---` 是指令文件；
//! - OpenCode / Gemini CLI 的 `<available_skills>…</available_skills>` 是技能列表；
//! - Gemini CLI 以用户身份发的 `<session_context>` 是 harness 消息，不是用户说的话；
//! - Codex 的 `# AGENTS.md instructions for <目录>` 里有 `--- project-doc ---` 时，前半是 `$CODEX_HOME/AGENTS.md`（user），后半才是项目的；
//! - Windows 路径（`\.claude\` 等）同样按用户目录认；描述里带 auto-memory 的先认成 memory；
//! - 技能列表先于指令文件拆出：OpenCode 把技能块紧接在最后一个指令文件后面，先拆指令文件会把技能算进去。

use super::estimate::tokens_of;
use super::parts::{Builder, Kind};

/// Claude Code 列技能的固定开头；列表到下一个空行为止。
pub const SKILLS_LIST: &str = "The following skills are available for use with the Skill tool:";

/// 用户消息里 agent 代为放进去的文字的开头：出现这些开头的文字不算用户说的话（magpie `injected`，加 Gemini CLI 的 `<session_context>`）。
const INJECTED: &[&str] = &[
    "<system-reminder",
    "<environment_context>",
    "<user_instructions>",
    "<turn_aborted>",
    "<user_shell_command>",
    "<subagent_notification>",
    "<in-app-browser-context",
    "<external_codex_apps_open_page>",
    "<session-context",
    "<session_context>",
    "<task-notification>",
    "<subagent-notification>",
    "<plugin_reference>",
    "Available agent types for the Agent tool:",
    "Available agent types and the tools they have access to:",
    "New agent types are now available for the Agent tool:",
    "The TodoWrite tool hasn't been used recently.",
    "The task tools haven't been used recently.",
    "Conversation rewind applied.",
    "Workspace rewind applied.",
    "The following deferred tools are",
    "# MCP Server Instructions",
    "The following MCP servers",
];

/// 以用户身份发、但其实是 harness 上下文的文字：整段算 system，按开头标签命名。
const HARNESS_USER: &[&str] =
    &["<environment_context>", "<user_instructions>", "<turn_aborted>", "<user_shell_command>", "<session_context>"];

const REMINDER_OPEN: &str = "<system-reminder";
const REMINDER_CLOSE: &str = "</system-reminder>";
const PROJECT_DOC: &str = "\n--- project-doc ---\n";
const GEMINI_FROM: &str = "--- Context from: ";
const GEMINI_END: &str = "--- End of Context from: ";

/// 指令文件归谁：先看 agent 怎么描述它，没有描述再看路径。
pub fn memory_tag(path: &str, desc: &str) -> &'static str {
    let d = desc.to_lowercase();
    if d.contains("auto-memory") {
        return "memory";
    }
    if d.contains("user's") || d.contains("global") {
        return "user";
    }
    if d.contains("local") {
        return "local";
    }
    if d.contains("memory") {
        return "memory";
    }
    if !d.is_empty() {
        return "project";
    }
    let p = path.replace('\\', "/");
    if [".claude/", ".codex/", ".config/", ".gemini/"].iter().any(|dir| p.contains(&format!("/{dir}"))) {
        return "user";
    }
    "project"
}

/// 每行的起止（不含换行符）。
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut at = 0;
    text.split('\n').map(move |line| {
        let start = at;
        at += line.len() + 1;
        (start, line)
    })
}

/// `Contents of <路径> (<描述>):` → (路径, 描述)。描述里允许一层括号。
fn contents_of(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("Contents of ")?;
    if rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_end_matches([' ', '\t', '\r']);
    let inner = rest.strip_suffix("):")?;
    // 从后往前找与结尾右括号配对的左括号
    let mut depth = 0;
    let mut open = None;
    for (i, c) in inner.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => {
                open = Some(i);
                break;
            }
            '(' => depth -= 1,
            _ => {}
        }
    }
    let open = open?;
    let path = inner[..open].strip_suffix(' ')?;
    if path.is_empty() || depth != 0 {
        return None;
    }
    Some((path, &inner[open + 1..]))
}

/// `# <x>.md instructions for <目录>\n` 开头（Codex）→ (文件名, 目录, 标题行之后的位置)。
pub fn agents_for(text: &str) -> Option<(&str, &str, usize)> {
    let rest = text.strip_prefix("# ")?;
    let nl = rest.find('\n')?;
    let line = &rest[..nl];
    let (file, dir) = line.split_once(" instructions for ")?;
    if file.is_empty() || file.contains(char::is_whitespace) || !file.ends_with(".md") {
        return None;
    }
    let dir = dir.trim_end_matches([' ', '\t', '\r']);
    if dir.is_empty() || dir.starts_with(char::is_whitespace) {
        return None;
    }
    Some((file, dir, 2 + nl + 1))
}

struct Section<'a> {
    at: usize,
    path: &'a str,
    desc: &'a str,
    gemini: bool,
}

/// 开头的 `<标签>`（允许前导空白）：字母或下划线开头，其后最多 40 个字母数字、下划线、空格或连字符。
fn lead_tag(text: &str) -> Option<&str> {
    let t = text.trim_start();
    let rest = t.strip_prefix('<')?;
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    for (i, c) in chars {
        if c == '>' {
            return (i <= 41).then(|| rest[..i].trim());
        }
        if !(c.is_ascii_alphanumeric() || c == '_' || c == ' ' || c == '-') || i > 41 {
            return None;
        }
    }
    None
}

/// system / developer 文字的名字：开头标签，`# Environment` 开头的叫 environment，否则用给定的名字。
pub fn harness_name<'a>(text: &'a str, fallback: &'a str) -> &'a str {
    if let Some(tag) = lead_tag(text) {
        return tag;
    }
    if text.starts_with("# Environment") {
        return "environment";
    }
    fallback
}

/// 去掉所有 `<system-reminder …>…</system-reminder>`（没有闭合的保留）。
fn strip_reminders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(REMINDER_OPEN) {
        let after = &rest[i + REMINDER_OPEN.len()..];
        let ok_open = after.starts_with('>') || after.starts_with(char::is_whitespace);
        let close = after.find('>').and_then(|gt| after[gt..].find(REMINDER_CLOSE).map(|c| gt + c));
        match (ok_open, close) {
            (true, Some(c)) => {
                out.push_str(&rest[..i]);
                rest = &after[c + REMINDER_CLOSE.len()..];
            }
            _ => {
                out.push_str(&rest[..i + REMINDER_OPEN.len()]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 最后一个行首的 `## My request:` / `## My request for Codex:` 之后的文字（Codex 应用把用户输入放在上下文之后）。
fn after_my_request(text: &str) -> Option<&str> {
    let mut found = None;
    for (at, line) in lines(text) {
        let Some(rest) = line.strip_prefix("## My request") else { continue };
        let rest = rest.strip_prefix(" for Codex").unwrap_or(rest);
        if rest.starts_with(':') {
            found = Some(at + line.len() - rest.len() + 1);
        }
    }
    let at = found?;
    let t = text[at..].trim_start_matches([' ', '\t']);
    Some(t.strip_prefix('\n').unwrap_or(t))
}

/// 这段用户文字是不是用户自己说的话（不是 agent 代放的指令文件、提醒、通知）。
pub fn user_says(text: &str) -> bool {
    let stripped;
    let mut t = text;
    if t.contains(REMINDER_OPEN) {
        stripped = strip_reminders(t);
        t = &stripped;
    }
    let mut t = t.trim();
    if t.contains("## My request") {
        if let Some(rest) = after_my_request(t) {
            t = rest.trim();
        }
    }
    if t.is_empty() || agents_for(t).is_some() {
        return false;
    }
    !INJECTED.iter().any(|lead| t.starts_with(lead))
}

impl Builder {
    /// 把文字里的指令文件拆出来记为 memory，返回剩下的。
    pub fn split_memory(&mut self, text: &str) -> String {
        let mut ss: Vec<Section> = Vec::new();
        for (at, line) in lines(text) {
            if let Some((path, desc)) = contents_of(line) {
                ss.push(Section { at, path, desc, gemini: false });
            } else if let Some(rest) = line.strip_prefix("Instructions from: ") {
                let path = rest.trim_end_matches([' ', '\t', '\r']);
                if !path.is_empty() && !path.starts_with(char::is_whitespace) {
                    ss.push(Section { at, path, desc: "", gemini: false });
                }
            } else if let Some(rest) = line.strip_prefix(GEMINI_FROM) {
                if let Some(path) = rest.trim_end_matches([' ', '\t', '\r']).strip_suffix(" ---") {
                    if !path.is_empty() {
                        ss.push(Section { at, path, desc: "", gemini: true });
                    }
                }
            }
        }
        if ss.is_empty() {
            return text.to_string();
        }
        let mut rest = String::with_capacity(text.len());
        rest.push_str(&text[..ss[0].at]);
        for i in 0..ss.len() {
            let s = &ss[i];
            let end = ss.get(i + 1).map_or(text.len(), |n| n.at);
            let chunk = &text[s.at..end];
            // 提醒的闭合标签及其后的内容不属于这个文件；Gemini CLI 用结束行标出文件的末尾
            let mut file_end = chunk.find(REMINDER_CLOSE).unwrap_or(chunk.len());
            if s.gemini {
                if let Some(j) = chunk.find(GEMINI_END) {
                    file_end = file_end.min(chunk[j..].find('\n').map_or(chunk.len(), |k| j + k + 1));
                }
            }
            rest.push_str(&chunk[file_end..]);
            self.add(Kind::Memory, s.path, memory_tag(s.path, s.desc), tokens_of(&chunk[..file_end]));
        }
        rest
    }

    /// 把技能列表拆出来记为 skills，返回剩下的：Claude Code 的固定句开头的列表（到下一个空行），
    /// 以及 OpenCode / Gemini CLI 的 `<available_skills>` 块。
    pub fn split_skills(&mut self, text: &str) -> String {
        let mut text = text.to_string();
        if let Some(at) = text.find(SKILLS_LIST) {
            let body = &text[at + SKILLS_LIST.len()..];
            let items = body.trim_start_matches('\n');
            let end = match items.find("\n\n") {
                Some(j) => text.len() - (items.len() - j),
                None => text.len(),
            };
            self.add(Kind::Skills, "skills", "skills", tokens_of(&text[at..end]));
            text = format!("{}{}", &text[..at], text[end..].trim_start_matches('\n'));
        }
        if let Some(at) = text.find("<available_skills>") {
            if let Some(close) = text[at..].find("</available_skills>") {
                let end = at + close + "</available_skills>".len();
                self.add(Kind::Skills, "skills", "skills", tokens_of(&text[at..end]));
                text = format!("{}{}", &text[..at], &text[end..]);
            }
        }
        text
    }

    /// agent 自己的提示词：先拆技能与指令文件，剩下的按开头标签命名。
    pub fn system_text(&mut self, text: &str, name: &str) {
        let skills_out = self.split_skills(text);
        let rest = self.split_memory(&skills_out);
        let trimmed = rest.trim();
        if trimmed.starts_with("<skills_instructions>") {
            self.add(Kind::Skills, "skills", "skills", tokens_of(&rest));
            return;
        }
        if trimmed.is_empty() {
            return;
        }
        let name = harness_name(&rest, name).to_string();
        self.add(Kind::System, &name, "", tokens_of(&rest));
    }

    /// 一段用户文字：agent 代放的指令文件、提醒、harness 上下文，或用户说的话。返回是否用户说的话。
    pub fn user_text(&mut self, text: &str) -> bool {
        let t = text.trim();
        if t.starts_with(REMINDER_OPEN) {
            let skills_out = self.split_skills(t);
            let rest = self.split_memory(&skills_out);
            let mut r = rest.trim();
            if let Some(after) = r.strip_prefix(REMINDER_OPEN) {
                r = after.find('>').map_or(after, |gt| &after[gt + 1..]);
            }
            let r = r.trim().strip_suffix(REMINDER_CLOSE).unwrap_or(r.trim()).trim();
            if !r.is_empty() {
                self.add(Kind::Memory, "reminders", "reminder", tokens_of(r));
            }
            return false;
        }
        if let Some((file, dir, body_at)) = agents_for(t) {
            let dir = dir.trim_end_matches(['/', '\\']);
            let sep = if dir.contains('\\') && !dir.contains('/') { '\\' } else { '/' };
            let project = format!("{dir}{sep}{file}");
            match t[body_at..].find(PROJECT_DOC) {
                Some(j) => {
                    let split = body_at + j;
                    self.add(Kind::Memory, &format!("$CODEX_HOME{sep}{file}"), "user", tokens_of(&t[..split]));
                    self.add(Kind::Memory, &project, "project", tokens_of(&t[split..]));
                }
                None => self.add(Kind::Memory, &project, "project", tokens_of(t)),
            }
            return false;
        }
        if HARNESS_USER.iter().any(|lead| t.starts_with(lead)) {
            let skills_out = self.split_skills(t);
            let rest = self.split_memory(&skills_out);
            let rest = rest.trim();
            if !rest.is_empty() {
                let name = harness_name(rest, "context").to_string();
                self.add(Kind::System, &name, "", tokens_of(rest));
            }
            return false;
        }
        self.chat(tokens_of(text));
        true
    }
}
