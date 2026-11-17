//! 工具定义、工具调用读了哪些文件、工具结果归哪一类（移植 magpie `tool` / `callOf` / `filesOfCommand` / `result`）。

use serde_json::{Map, Value};

use super::parts::{Builder, Kind, ToolCall, ToolResult};

/// 读文件的工具。
const READ_TOOLS: &[&str] = &["Read", "read", "read_file", "ReadFile", "view", "View", "NotebookRead", "readFile", "open_file", "cat"];
/// 读文件工具的路径参数（Gemini CLI 的 read_file 用 `file_path`，旧版用 `absolute_path`）。
const PATH_KEYS: &[&str] =
    &["file_path", "path", "filePath", "absolute_path", "target_file", "notebook_path", "file", "filename", "uri"];
/// 加载 Skill 的工具与它们放 Skill 名的参数（Claude Code `Skill` 的 `skill` / 旧版 `command`，Gemini CLI `activate_skill` 的 `name`）。
const SKILL_TOOLS: &[&str] = &["Skill", "skill", "activate_skill", "use_skill", "load_skill"];
const SKILL_KEYS: &[&str] = &["skill", "name", "command", "skill_name"];

/// 只读文件的命令（`sed` 另带 `-n` 才算）。
const READ_CMDS: &[&str] = &["cat", "head", "tail", "nl", "bat", "less", "more"];

pub fn is_read_tool(name: &str) -> bool {
    READ_TOOLS.contains(&name)
}

/// 一条 shell 命令读了哪些文件：只在它只是读文件时（cat、head、tail、nl、bat、sed -n……）才算。
pub fn files_of_command(cmd: &str) -> Vec<String> {
    let mut cmd = cmd.trim().to_string();
    // bash -lc "…"，Codex 的 shell 工具这样发
    for pre in ["bash -lc ", "bash -c ", "sh -c ", "zsh -lc "] {
        if let Some(rest) = cmd.strip_prefix(pre) {
            cmd = rest.trim().trim_matches(['"', '\'']).to_string();
        }
    }
    let special = |s: &str| s.contains(['|', ';', '&', '>', '<', '`', '$']);
    if special(&cmd) {
        // 管道或多条命令都不算单纯读文件；但「cd 目录 && cat x」算
        match cmd.find("&&") {
            Some(i) if cmd.starts_with("cd ") && !special(&cmd[i + 2..]) => cmd = cmd[i + 2..].trim().to_string(),
            _ => return vec![],
        }
    }
    let f: Vec<&str> = cmd.split_whitespace().collect();
    if f.len() < 2 {
        return vec![];
    }
    let sed = f[0] == "sed" && f.contains(&"-n");
    if !READ_CMDS.contains(&f[0]) && !sed {
        return vec![];
    }
    let mut files = vec![];
    let mut i = 1;
    while i < f.len() {
        let a = f[i].trim_matches(['"', '\'']);
        i += 1;
        if a.starts_with('-') {
            if (a == "-n" || a == "-c") && !sed && i < f.len() {
                i += 1; // head -n 40
            }
            continue;
        }
        // sed -n 1,80p
        if sed && a.ends_with('p') && a.contains(|c: char| c.is_ascii_digit() || c == ',') && !a.contains('/') && !a.contains('.') {
            continue;
        }
        if a.parse::<i64>().is_ok() {
            continue;
        }
        files.push(a.to_string());
    }
    files
}

/// 路径是某个 Skill 的 `SKILL.md` 时返回 Skill 名（它所在目录的名字）。
pub fn skill_of_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [.., dir, file] if file.eq_ignore_ascii_case("SKILL.md") => Some(dir.to_string()),
        _ => None,
    }
}

/// 前一个字符不是单词字符（`\b`）。
fn word_start(s: &str, i: usize) -> bool {
    i == 0 || !s[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Codex 的 exec 工具收 JavaScript：`text(await tools.exec_command({cmd:"cat x"}))` → (工具名, 命令)。
fn js_call(s: &str) -> (Option<String>, Option<String>) {
    let mut tool = None;
    let mut from = 0;
    while let Some(j) = s[from..].find("tools.") {
        let at = from + j;
        from = at + 6;
        if !word_start(s, at) {
            continue;
        }
        let name: String = s[from..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !name.is_empty() && s[from + name.len()..].trim_start().starts_with('(') {
            tool = Some(name);
            break;
        }
    }
    let mut cmd = None;
    let mut from = 0;
    while let Some(j) = s[from..].find("cmd") {
        let at = from + j;
        from = at + 3;
        if !word_start(s, at) {
            continue;
        }
        let Some(rest) = s[from..].trim_start().strip_prefix(':') else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('"') else { continue };
        let mut end = None;
        let mut escaped = false;
        for (k, c) in rest.char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => {
                    end = Some(k);
                    break;
                }
                _ => {}
            }
        }
        if let Some(k) = end {
            cmd = serde_json::from_str::<String>(&format!("\"{}\"", &rest[..k])).ok();
            break;
        }
    }
    (tool, cmd)
}

/// 从工具名与参数看出这次调用读了哪些文件。`args` 是参数对象；参数不是 JSON 对象时给 `text`（Codex exec 的 JavaScript）。
pub fn call_of(name: &str, args: Option<&Map<String, Value>>, text: Option<&str>) -> ToolCall {
    let mut c = ToolCall { name: name.to_string(), ..Default::default() };
    let str_of = |k: &str| args.and_then(|a| a.get(k)).and_then(Value::as_str).unwrap_or("");
    if SKILL_TOOLS.contains(&name) {
        if let Some(skill) = SKILL_KEYS.iter().map(|k| str_of(k).trim()).find(|s| !s.is_empty()) {
            c.skill = Some(skill.trim_start_matches('/').to_string());
            c.target = c.skill.clone();
            return c;
        }
    }
    if is_read_tool(name) {
        if let Some(path) = PATH_KEYS.iter().map(|k| str_of(k)).find(|s| !s.is_empty()) {
            c.files = vec![path.to_string()];
            c.target = Some(path.to_string());
            return c;
        }
    }
    let mut cmd = str_of("command").to_string();
    if cmd.is_empty() {
        cmd = str_of("cmd").to_string();
    }
    if cmd.is_empty() {
        if let Some(argv) = args.and_then(|a| a.get("command")).and_then(Value::as_array) {
            if argv.iter().all(Value::is_string) {
                cmd = argv.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
            }
        }
    }
    if cmd.is_empty() && args.is_none() {
        if let Some(t) = text {
            let (tool, js_cmd) = js_call(t);
            if let Some(tool) = tool {
                c.name = tool;
            }
            cmd = js_cmd.unwrap_or_default();
        }
    }
    if !cmd.is_empty() {
        c.files = files_of_command(&cmd);
        c.target = Some(cmd);
    } else if let Some(path) = PATH_KEYS.iter().map(|k| str_of(k)).find(|s| !s.is_empty()) {
        // 不是读文件工具但带路径参数（写文件、列目录……）：路径就是它操作的对象
        c.target = Some(path.to_string());
    }
    c
}

/// 参数是 JSON 文本时（OpenAI 的 `arguments`、Responses 的 `input`）：能解成对象就按对象读，否则按 JavaScript 读。
pub fn call_of_text(name: &str, text: &str) -> ToolCall {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => call_of(name, Some(&m), None),
        Ok(Value::Null) | Err(_) => call_of(name, None, Some(text)),
        Ok(_) => call_of(name, None, None),
    }
}

impl Builder {
    /// 一个工具定义：同一 MCP 服务器（`mcp__<服务器>__<工具>`）的工具记成一项。
    pub fn tool(&mut self, name: &str, tokens: i64) {
        if let Some(rest) = name.strip_prefix("mcp__") {
            let server = match rest.find("__") {
                Some(i) if i > 0 => &rest[..i],
                _ => rest,
            };
            self.add(Kind::Tools, server, "mcp", tokens);
            return;
        }
        self.add(Kind::Tools, name, "", tokens);
    }

    /// 一个工具结果：调用读了文件的记到 files（多个文件平分），否则按工具名记到 results。
    /// 另记一条明细（`ToolResult`）：工具名、脱敏后的命令或路径、行数、退出码，不存输出正文。
    pub fn result(&mut self, call_id: &str, tokens: i64, output: &str) {
        let call = self.calls.get(call_id).cloned().unwrap_or_else(|| ToolCall { name: "tool".into(), ..Default::default() });
        let (lines, exit_code, summary) = super::summary::summarize(output);
        let (target, masked) = match call.target.as_deref() {
            Some(t) => {
                let (m, changed) = super::summary::mask(t);
                (Some(m), changed)
            }
            None => (None, false),
        };
        // Skill 的加载证据（V10-11）：调用了 Skill 工具，或读的文件就是某个 Skill 的 SKILL.md。
        // 这部分归入 Skills 一次，不再算进工具结果或文件
        let loaded = call.skill.clone().or_else(|| match call.files.as_slice() {
            [only] => skill_of_path(only),
            _ => None,
        });
        let kind = if loaded.is_some() { Kind::Skills } else if call.files.is_empty() { Kind::Results } else { Kind::Files };
        if tokens > 0 {
            self.results.push(ToolResult { tool: call.name.clone(), target, kind, tokens, lines, exit_code, masked, summary, captured: false });
        }
        if let Some(skill) = loaded {
            self.add(Kind::Skills, &skill, "loaded", tokens);
            return;
        }
        if call.files.is_empty() {
            self.add(Kind::Results, &call.name, "result", tokens);
            return;
        }
        let count = call.files.len() as i64;
        let each = tokens / count;
        let tag = if is_read_tool(&call.name) { "read" } else { "shell" };
        for (i, f) in call.files.iter().enumerate() {
            let n = if i == 0 { each + tokens - each * count } else { each };
            self.add(Kind::Files, f, tag, n);
        }
    }
}
