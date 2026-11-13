//! 接线改动预览（P2-3 / F6，画板 V5-1「写入的改动 · 查看差异」、V5-2「写入预览」）。
//!
//! 只做展示用的逐行对比：接入前给「将要写成什么」，已接入时给「Jolu 改了什么」
//! （断开会还原成的内容 → 现在的内容）。用户配置里可能有他自己的 API Key，
//! 所以每一行都先过一遍密钥遮挡，原值不离开后端。

use std::path::Path;

/// 一行对比。`op`：`+` 新增、`-` 删除、` ` 未改动的上下文、`…` 折叠掉的未改动段。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DiffLine {
    pub op: &'static str,
    pub text: String,
}

/// 一个文件的对比结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FileDiff {
    /// 展示路径：用户目录换成 `~`，分隔符统一为 `/`
    pub path: String,
    /// 对比前文件是否存在（接入前预览里「会新建这个文件」）
    pub existed: bool,
    /// 改动的键数（JSON 按叶子路径计；其余格式按改动行数计）
    pub keys: usize,
    pub lines: Vec<DiffLine>,
}

/// 未改动段保留的上下文行数。
const CONTEXT: usize = 2;
/// 逐行 LCS 的规模上限：超过就退化为「整段删除 + 整段新增」，不让超大文件卡住界面。
const MAX_CELLS: usize = 4_000_000;

/// 展示路径：家目录前缀换成 `~`。
pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    let home = crate::creds::home().map(|home| home.display().to_string());
    let text = match home {
        Some(home) if !home.is_empty() && text.starts_with(&home) => format!("~{}", &text[home.len()..]),
        _ => text,
    };
    text.replace('\\', "/")
}

/// 对比一个文件。`before` 为 None = 文件原本不存在。
pub fn diff_file(path: &Path, before: Option<&str>, after: &str) -> FileDiff {
    let old = before.unwrap_or("");
    FileDiff {
        path: display_path(path),
        existed: before.is_some(),
        keys: changed_keys(old, after),
        lines: line_diff(old, after),
    }
}

/// 改动的键数：两边都是 JSON 时按叶子路径比；否则按改动行数（增删配对取大者）。
pub fn changed_keys(before: &str, after: &str) -> usize {
    let parse = |text: &str| -> Option<serde_json::Value> {
        if text.trim().is_empty() {
            Some(serde_json::json!({}))
        } else {
            serde_json::from_str(text).ok()
        }
    };
    if let (Some(a), Some(b)) = (parse(before), parse(after)) {
        let (mut left, mut right) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
        flatten("", &a, &mut left);
        flatten("", &b, &mut right);
        let keys: std::collections::BTreeSet<&String> = left.keys().chain(right.keys()).collect();
        return keys.into_iter().filter(|key| left.get(*key) != right.get(*key)).count();
    }
    let lines = line_diff(before, after);
    let adds = lines.iter().filter(|line| line.op == "+").count();
    let removes = lines.iter().filter(|line| line.op == "-").count();
    adds.max(removes)
}

fn flatten(prefix: &str, value: &serde_json::Value, out: &mut std::collections::BTreeMap<String, String>) {
    match value {
        serde_json::Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                flatten(&path, child, out);
            }
        }
        // 根上的空对象（文件原本不存在 / 是 `{}`）不算一个键
        _ if prefix.is_empty() => {}
        other => {
            out.insert(prefix.to_string(), other.to_string());
        }
    }
}

/// 逐行对比（LCS），未改动段只留前后各 [`CONTEXT`] 行，中间折成一行 `…`。
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let raw = if a.len().saturating_mul(b.len()) > MAX_CELLS { naive(&a, &b) } else { lcs(&a, &b) };
    collapse(raw)
}

fn naive<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(&'static str, &'a str)> {
    a.iter().map(|line| ("-", *line)).chain(b.iter().map(|line| ("+", *line))).collect()
}

fn lcs<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(&'static str, &'a str)> {
    let (n, m) = (a.len(), b.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if a[i] == b[j] { table[at(i + 1, j + 1)] + 1 } else { table[at(i + 1, j)].max(table[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(n + m);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((" ", a[i]));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            out.push(("-", a[i]));
            i += 1;
        } else {
            out.push(("+", b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|line| ("-", *line)));
    out.extend(b[j..].iter().map(|line| ("+", *line)));
    out
}

fn collapse(raw: Vec<(&'static str, &str)>) -> Vec<DiffLine> {
    let changed: Vec<usize> = raw.iter().enumerate().filter(|(_, (op, _))| *op != " ").map(|(i, _)| i).collect();
    let near = |i: usize| changed.iter().any(|c| c.abs_diff(i) <= CONTEXT);
    let mut out = Vec::new();
    let mut folded = 0usize;
    for (i, (op, text)) in raw.iter().enumerate() {
        if *op == " " && !near(i) {
            folded += 1;
            continue;
        }
        if folded > 0 {
            out.push(DiffLine { op: "…", text: format!("{folded} 行未改动") });
            folded = 0;
        }
        out.push(DiffLine { op, text: redact_line(text) });
    }
    if folded > 0 {
        out.push(DiffLine { op: "…", text: format!("{folded} 行未改动") });
    }
    out
}

/// 遮住一行里的密钥：名字像密钥的键（key / token / secret / password / auth / credential）
/// 的值，以及脱敏规则认得的 API Key、私钥、令牌。Jolu 自己的接线令牌（`jolu-` 开头）不是秘密，保留。
pub fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let named = ["key", "token", "secret", "password", "auth", "credential"];
    let mut text = line.to_string();
    if let Some(at) = value_start(line) {
        let name = &lower[..at];
        let value = line[at..].trim();
        let bare = value.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c.is_whitespace());
        let is_secret_name = named.iter().any(|word| name.contains(word));
        if is_secret_name && !bare.is_empty() && !bare.starts_with("jolu-") && !crate::gateway::wiring::is_gateway_value(bare) && bare != "{" && bare != "[" {
            text = format!("{}{}", &line[..at], mask_value(&line[at..], bare));
        }
    }
    let spans = crate::gateway::mask::sensitive_spans(&text, &["APIKEY", "PRIVATEKEY", "TOKEN"]);
    if spans.is_empty() {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in spans {
        // 已经遮过的值（含 •）与 Jolu 自己的令牌不再处理
        if start < cursor || text[start..end].starts_with("jolu-") || text[start..end].contains('•') {
            continue;
        }
        out.push_str(&text[cursor..start]);
        out.push_str(&masked(&text[start..end]));
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// `"name": value` / `name = value` / `NAME=value` 里值开始的位置（分隔符之后）。
fn value_start(line: &str) -> Option<usize> {
    let colon = line.find("\":").map(|i| i + 2);
    let equals = line.find('=').map(|i| i + 1);
    match (colon, equals) {
        (Some(c), Some(e)) => Some(c.min(e)),
        (c, e) => c.or(e),
    }
}

fn mask_value(segment: &str, bare: &str) -> String {
    segment.replacen(bare, &masked(bare), 1)
}

fn masked(value: &str) -> String {
    let tail: String = value.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    if value.chars().count() <= 8 { "••••".to_string() } else { format!("••••{tail}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_marks_added_and_removed_lines_and_folds_context() {
        let before = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3,\n  \"d\": 4,\n  \"e\": 5,\n  \"f\": 6\n}";
        let after = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3,\n  \"d\": 4,\n  \"e\": 5,\n  \"f\": 7\n}";
        let lines = line_diff(before, after);
        assert_eq!(lines.first().map(|l| l.op), Some("…"), "前面的未改动段折叠：{lines:?}");
        assert!(lines.iter().any(|l| l.op == "-" && l.text.contains("\"f\": 6")));
        assert!(lines.iter().any(|l| l.op == "+" && l.text.contains("\"f\": 7")));
        assert_eq!(changed_keys(before, after), 1);
        assert_eq!(changed_keys("", "{\"env\":{\"A\":\"1\",\"B\":\"2\"}}"), 2, "新建文件按键数计");
        assert_eq!(changed_keys("x = 1\ny = 2", "x = 1\ny = 3\nz = 4"), 2, "非 JSON 按行计");
    }

    #[test]
    fn secrets_never_leave_in_diff_lines() {
        let line = r#"    "apiKey": "sk-abcdefghijklmnopqrstuvwxyz123456","#;
        let out = redact_line(line);
        assert!(!out.contains("abcdefghijklmnop"), "{out}");
        assert!(out.contains("••••3456"), "{out}");
        assert_eq!(redact_line(r#"  "ANTHROPIC_AUTH_TOKEN": "jolu-claude-code.abcd1234","#), r#"  "ANTHROPIC_AUTH_TOKEN": "jolu-claude-code.abcd1234","#, "Jolu 自己的令牌照常显示");
        assert_eq!(redact_line(r#"  "model": "zcode/glm-5.3","#), r#"  "model": "zcode/glm-5.3","#);
        assert!(!redact_line("GEMINI_API_KEY=AIzaSyA1234567890abcdefghij").contains("1234567890abc"));
        assert_eq!(redact_line(r#"    "options": {"#), r#"    "options": {"#);
    }
}
