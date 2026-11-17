//! 工具结果的摘要与脱敏（执行文档 §5）：只留命令或路径、行数、退出码，不存输出正文。
//!
//! 脱敏复用 `handoff_facts::redact`：凭证键名后的值、前缀 Key、JWT、URL 密码、长串；它内部还复用网关隐私脱敏
//! （`gateway::mask`）的卡号与身份证规则。在此之上再把本机用户目录换成 `~`、用户名换成 `<user>`。

/// 本机用户目录与用户名（只算一次）。
fn local_identity() -> &'static (Vec<String>, Option<String>) {
    static IDENTITY: std::sync::OnceLock<(Vec<String>, Option<String>)> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(|| {
        let mut homes = Vec::new();
        for var in ["USERPROFILE", "HOME"] {
            if let Ok(home) = std::env::var(var) {
                let home = home.trim_end_matches(['\\', '/']).to_string();
                if home.len() >= 3 {
                    homes.push(home);
                }
            }
        }
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .ok()
            .filter(|u| u.chars().count() >= 3);
        (homes, user)
    })
}

/// 把 `text` 里的 `needle` 全部换成 `with`，ASCII 部分不区分大小写（Windows 盘符与目录名大小写不定）。
fn replace_ignore_case(text: &str, needle: &str, with: &str) -> (String, bool) {
    if needle.is_empty() {
        return (text.to_string(), false);
    }
    let lower_text = text.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut hit = false;
    while let Some(i) = lower_text[from..].find(&lower_needle) {
        let at = from + i;
        out.push_str(&text[from..at]);
        out.push_str(with);
        from = at + needle.len();
        hit = true;
    }
    out.push_str(&text[from..]);
    (out, hit)
}

/// 用给定的目录与用户名脱敏（测试直接调这个）。返回脱敏后的文字与是否改过。
pub fn mask_with(text: &str, homes: &[String], user: Option<&str>) -> (String, bool) {
    let mut out = crate::handoff_facts::redact(text);
    let mut changed = out != text;
    for home in homes {
        for variant in [home.clone(), home.replace('\\', "/"), home.replace('/', "\\"), home.replace('\\', "\\\\")] {
            let (next, hit) = replace_ignore_case(&out, &variant, "~");
            out = next;
            changed |= hit;
        }
    }
    if let Some(user) = user {
        let (next, hit) = replace_ignore_case(&out, user, "<user>");
        out = next;
        changed |= hit;
    }
    (out, changed)
}

/// 命令或路径的脱敏（本机目录、用户名、凭证），过长的截到 300 个字符。
pub fn mask(text: &str) -> (String, bool) {
    let (homes, user) = local_identity();
    let (masked, changed) = mask_with(text, homes, user.as_deref());
    if masked.chars().count() > 300 {
        (masked.chars().take(300).collect::<String>() + "…", changed)
    } else {
        (masked, changed)
    }
}

/// 输出里写明的退出码：`exit code 1`、`Exit Code: 0`、`exited with code 2`、`exit_code=0`。
pub fn exit_code_of(output: &str) -> Option<i64> {
    let lower = output.to_ascii_lowercase();
    for marker in ["exited with code", "exit code", "exit_code"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(marker) {
            let after = &output[from + i + marker.len()..];
            from += i + marker.len();
            let rest = after.trim_start_matches([' ', ':', '=', '\t']);
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            if let Ok(code) = digits.parse::<i64>() {
                return Some(code);
            }
        }
    }
    None
}

/// 输出的行数与一句话摘要。
pub fn summarize(output: &str) -> (i64, Option<i64>, String) {
    let trimmed = output.trim_end();
    let lines = if trimmed.is_empty() { 0 } else { trimmed.lines().count() as i64 };
    let exit = exit_code_of(output);
    let summary = match (exit, lines) {
        (Some(code), 0) => format!("退出码 {code}，无输出"),
        (Some(code), n) => format!("退出码 {code}，输出 {n} 行"),
        (None, 0) => "无输出".to_string(),
        (None, n) => format!("输出 {n} 行"),
    };
    (lines, exit, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_home_user_and_credentials() {
        let homes = vec![r"C:\Users\alice".to_string()];
        let (out, changed) = mask_with(r"cat C:\Users\alice\proj\a.ts && cat c:/users/alice/.env", &homes, Some("alice"));
        assert!(changed);
        assert_eq!(out, r"cat ~\proj\a.ts && cat ~/.env");
        let (out, changed) = mask_with("curl -H 'Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz123456' x", &[], None);
        assert!(changed && !out.contains("sk-abcdefghijklmnopqrstuvwxyz123456"), "{out}");
        let (out, changed) = mask_with("pnpm --dir frontend typecheck", &homes, Some("alice"));
        assert_eq!((out.as_str(), changed), ("pnpm --dir frontend typecheck", false));
    }

    #[test]
    fn exit_codes_and_summaries() {
        assert_eq!(exit_code_of("Process exited with code 0\nOutput:"), Some(0));
        assert_eq!(exit_code_of("Exit Code: 2"), Some(2));
        assert_eq!(exit_code_of("exit_code=127"), Some(127));
        assert_eq!(exit_code_of("no code here"), None);
        assert_eq!(summarize("a\nb\n\n"), (2, None, "输出 2 行".to_string()));
        assert_eq!(summarize(""), (0, None, "无输出".to_string()));
        assert_eq!(summarize("Exit code 1\nboom").2, "退出码 1，输出 2 行");
    }
}
