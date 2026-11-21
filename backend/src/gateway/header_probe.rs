//! 请求头诊断（只读，默认关闭）。
//!
//! 目的：确认某个工具发请求时**到底带不带会话标识**，好决定 `wire::session_identity`
//! 能不能认出它的会话。没有这份事实就只能猜头名，猜错的表现是「永远采不到」或
//! 「采到一串垃圾 ID」——两种都不报错，最难查。
//!
//! ## 怎么开
//!
//! 设环境变量 `JOLU_HEADER_PROBE=1` 再启动桌面端。日志落到数据目录的
//! `header-probe.ndjson`（一行一个请求，JSON）。
//!
//! ## 记什么（以及为什么不记更多）
//!
//! - **头的名字**：全部记。会话标识往往就是个自定义头名。
//! - **`user-agent` 的值**：记。它不含机密，且是「这个请求来自哪个工具」的主要依据。
//! - **其它头的值**：**不记**。`authorization` / `x-api-key` / cookie 里就是密钥；
//!   按 AGENTS.md「认证信息不得写入日志」，这里一律只留名字。
//! - **请求体的键路径**：只记**结构和长度**，不记内容（用户提示词属于隐私）。
//!   键路径够判断「会话 id 是不是在 body 里」。
//!
//! 也就是说：这份日志可以用来回答「有没有会话标识、叫什么」，但**复原不出任何密钥或对话内容**。

use std::io::Write;
use std::path::PathBuf;

use axum::http::HeaderMap;

/// 诊断是否开启。每次调用都读环境变量：桌面端一次启动内不会变，开销可忽略，
/// 但不做缓存也就不必在测试里处理跨用例的状态。
pub(crate) fn enabled() -> bool {
    std::env::var_os("JOLU_HEADER_PROBE").is_some_and(|value| !value.is_empty() && value != "0")
}

/// 日志文件：数据目录下的 `header-probe.ndjson`。取不到数据目录时返回 None（静默跳过）。
fn log_path() -> Option<PathBuf> {
    let dir = crate::validation::data_dir().or_else(|| {
        // 非隔离模式下没有 JOLU_TEST_DIR，用 Tauri 的应用数据目录不可行（这里没有 AppHandle），
        // 退到用户主目录下的固定位置：与 `~/.jolu` 同级，用户找得到。
        crate::creds::home().map(|home| home.join(".jolu").join("data"))
    })?;
    Some(dir.join("header-probe.ndjson"))
}

/// 记一条请求头样本。失败时静默——诊断绝不能影响请求本身。
///
/// `agent` 是 `agent_from_headers` 已经认出来的工具（可能为 None），
/// `session` 是 `session_identity` 的结果（None 就是「这家的会话认不出来」）。
pub(crate) fn record(agent: Option<&str>, headers: &HeaderMap, body: &[u8], session: Option<(&str, &str)>) {
    if !enabled() {
        return;
    }
    let Some(path) = log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 头的名字：排序后拼起来，便于肉眼比对两次请求的差异
    let mut names: Vec<&str> = headers.keys().map(|name| name.as_str()).collect();
    names.sort_unstable();

    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().chars().take(200).collect::<String>());

    let line = serde_json::json!({
        "ts": chrono::Local::now().to_rfc3339(),
        "agent": agent,
        // 认出来的会话（platform, session_id）；None = 这家没会话标识，正是我们要查的
        "session": session.map(|(platform, id)| serde_json::json!({ "platform": platform, "id": id })),
        "user_agent": user_agent,
        "header_names": names,
        // 请求体的键路径（只到第二层，不带值）：够判断会话 id 是否在 body 里
        "body_keys": body_key_paths(body),
        "body_len": body.len(),
    });

    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

/// 请求体的键路径（顶层 + 第二层），不带任何值。
///
/// 只到第二层：会话标识要么在顶层（少见），要么在 `metadata.user_id` 这种一层嵌套里；
/// 再往下的 `messages[].content[]` 全是用户内容，不该进日志。
fn body_key_paths(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Some(object) = value.as_object() {
        for (key, child) in object {
            paths.push(key.clone());
            if let Some(nested) = child.as_object() {
                for nested_key in nested.keys() {
                    paths.push(format!("{key}.{nested_key}"));
                }
            }
        }
    }
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_key_paths_are_structure_only() {
        // br"" 只接受 ASCII，所以用 \u 转义写中文
        let body = br#"{"model":"m","metadata":{"user_id":"{\"session_id\":\"ses-1\"}"},"messages":[{"role":"user","content":"\u79d8\u5bc6\u5185\u5bb9"}]}"#;
        let paths = body_key_paths(body);
        assert!(paths.contains(&"model".to_string()));
        assert!(paths.contains(&"metadata.user_id".to_string()), "会话可能藏在 metadata 里：{paths:?}");
        assert!(paths.contains(&"messages".to_string()));
        // 不记值：路径里不该出现任何来自 body 的具体内容（含中文的转义序列）
        let joined = paths.join(",");
        assert!(!joined.contains("\\u79d8"), "键路径不能带值：{joined}");
        assert!(!joined.contains("ses-1"), "键路径不能带值：{joined}");
        // 不展开第三层：messages[].content 是用户内容
        assert!(!joined.contains("content"), "不该展开到第三层：{joined}");
    }

    #[test]
    fn body_key_paths_survives_non_json() {
        assert!(body_key_paths(b"not json").is_empty());
        assert!(body_key_paths(b"").is_empty());
        assert!(body_key_paths(b"[1,2,3]").is_empty(), "顶层是数组时没有键路径");
    }

    #[test]
    fn probe_is_off_by_default() {
        // 测试进程里没人设过这个变量（CI 也不该设）
        assert!(!enabled(), "诊断默认必须关闭");
    }

    /// 端到端：开启后真的写出一行 NDJSON，且**不含任何头的值**（密钥不能进日志）。
    ///
    /// 用子进程 + 临时 `JOLU_TEST_DIR`（`validation::data_dir` 只认带
    /// `.acceptance-profile` 标记的绝对路径），不碰真实数据目录。
    #[test]
    fn probe_writes_header_names_but_never_values() {
        if std::env::var_os("JOLU_PROBE_TEST_CHILD").is_some() {
            // 子进程：造一个带密钥的请求，跑一次 record，退出码即断言结果
            let mut headers = HeaderMap::new();
            headers.insert("authorization", "Bearer jolu-zcode.secret-token-value".parse().unwrap());
            headers.insert("x-api-key", "sk-super-secret".parse().unwrap());
            headers.insert("x-session-id", "sess_abc123".parse().unwrap());
            headers.insert("user-agent", "ZCode/1.2.3".parse().unwrap());
            record(None, &headers, br#"{"model":"glm"}"#, None);

            let root = std::path::PathBuf::from(std::env::var("JOLU_TEST_DIR").unwrap());
            let text = std::fs::read_to_string(root.join("header-probe.ndjson")).unwrap();
            // 头名要在
            assert!(text.contains("x-session-id"), "头名必须记下来：{text}");
            assert!(text.contains("ZCode/1.2.3"), "UA 要记（识别工具的依据）");
            // 值一律不许出现
            for secret in ["secret-token-value", "sk-super-secret", "sess_abc123"] {
                assert!(!text.contains(secret), "头的值不能进日志，但出现了 {secret}：{text}");
            }
            assert_eq!(text.lines().count(), 1, "一次 record 只写一行");
            return;
        }
        let root = std::env::temp_dir().join(format!("jolu-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".acceptance-profile"), "test").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "gateway::header_probe::tests::probe_writes_header_names_but_never_values", "--nocapture"])
            .env("JOLU_PROBE_TEST_CHILD", "1")
            .env("JOLU_TEST_DIR", &root)
            .env("JOLU_HEADER_PROBE", "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&root);
        assert!(output.status.success(), "子进程失败：\n{stderr}");
    }
}
