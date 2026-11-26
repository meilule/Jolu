//! DeepSeek Harness（dsh）的本地会话读取（只读）。
//!
//! 第三个走「本地会话文件」这条路的工具（前两个是 [`crate::kimi_sessions`] 与
//! [`crate::pi_sessions`]）。起因相同：dsh 的会话不在 `requests` 表里，
//! `session_threads::list` 的主查询看不到它。
//!
//! ## 目录结构与实测格式（2026-11-22 本机核实）
//!
//! ```text
//! ~/.dsh/sessions/<项目路径转义>/<sessionId>/session.v4.jsonl.zstd
//! ```
//!
//! 与 Kimi / Pi 最大的不同：**它是 zstd 压缩的**。所以要用 `ruzstd` 先解压再按行解析。
//! 选纯 Rust 的 `ruzstd` 而不是绑 C 库的 `zstd`：这里只解压、不压缩，没必要引入 native 依赖。
//!
//! 解压后是逐行 JSON 事件流（225 行 / 792KB 的实测样本），用得上的三类：
//!
//! ```json
//! {"type":"session","version":4,"id":"0135f942-…","createdAt":1793859630498,"cwd":"D:\\develop\\Jolu",
//!  "origin":"subagent","parentSession":"session-…"}
//! {"type":"user/message","time":…,"data":{…}}
//! {"type":"assistant/message","time":…,"data":{"message":{…},
//!   "usage":{"inputTokens":7666,"outputTokens":196,"totalTokens":24246,"cacheReadTokens":16384}}}
//! ```
//!
//! ## 两个必须处理的细节
//!
//! 1. **子代理会话要过滤**：`session` 事件里 `origin: "subagent"` 或 `parentSession` 非空
//!    的是子代理会话（本机 63 个会话里大量是这种）。它们不该出现在用户的会话列表里，
//!    否则一个任务会冒出十几条「会话」。判据用 `origin == "subagent"`，与 ZCode 的
//!    `parent_id` 过滤是同一类规则。
//! 2. **`usage` 里的输入要扣掉 `cacheReadTokens`**：实测样本
//!    `inputTokens=7666, cacheReadTokens=16384, totalTokens=24246`，说明 `inputTokens`
//!    不含缓存读取（7666 + 196 + 16384 = 24246）。这里仍按「扣掉」处理并钳到 0：
//!    万一同版本改成包含，也不会重复计数。
//!
//! ## 不做什么
//!
//! - 不解析对话正文（`user/message` / `assistant/message` 里都有），`has_source` 给 false
//! - 解压设上限：解压后的体积可能远大于压缩包，按 [`MAX_DECODED`] 截断而不是把内存吃满

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::db::{RequestRecord, Store};
use crate::session_threads::SessionItem;
use crate::source_store::{self, Record};

/// 单个会话解压后的读取上限。
///
/// 实测样本压缩 233KB → 解压 792KB（约 3.4 倍）。正常会话在这个量级，但损坏或恶意的
/// 文件可以解压出任意大小；`ruzstd` 文档也明确提醒「frames can be REALLY big」。
/// 16MB 足够覆盖真实会话（约 20 倍余量），超出就停止读取并保留已读部分。
const MAX_DECODED: usize = 16 * 1024 * 1024;

/// `session` 事件（只需几个字段）。
#[derive(Debug, Default, Deserialize)]
struct SessionEvent {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "createdAt")]
    created_at: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    /// `"subagent"` 表示这是子代理自己开的会话，不是用户的
    #[serde(default)]
    origin: Option<String>,
    /// 非空表示有父会话（子代理 / fork）
    #[serde(default, rename = "parentSession")]
    parent_session: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default, rename = "inputTokens")]
    input: Option<i64>,
    #[serde(default, rename = "outputTokens")]
    output: Option<i64>,
    #[serde(default, rename = "cacheReadTokens")]
    cache_read: Option<i64>,
    #[serde(default, rename = "cacheWriteTokens")]
    cache_write: Option<i64>,
    #[serde(default, rename = "totalTokens")]
    total_tokens: Option<i64>,
}

/// 判定 dsh 的 `inputTokens` 是否包含 `cacheReadTokens`。
/// 实测 dsh v4 样本：`inputTokens=7666, outputTokens=196, cacheReadTokens=16384, totalTokens=24246`
/// （7666 + 196 + 16384 = 24246），明确为 `excludes_cache`；仅当 `totalTokens` 明确等于
/// `input + output`（或无 `totalTokens` 且 `input >= cacheRead` 的旧测试夹具）时按 `includes_cache`。
fn dsh_input_includes_cache(usage: &Usage) -> bool {
    match (usage.input, usage.output, usage.cache_read, usage.total_tokens) {
        (Some(i), Some(o), Some(r), Some(t)) if r > 0 => {
            let excl = i + o + r + usage.cache_write.unwrap_or(0);
            let incl = i + o + usage.cache_write.unwrap_or(0);
            if excl == t {
                false
            } else {
                incl == t
            }
        }
        (Some(i), _, Some(r), None) if r > 0 => i >= r,
        _ => false,
    }
}

/// dsh 配置根目录：`DSH_HOME` 覆盖，否则 `~/.dsh`。与 `wiring_dsh::dsh_dir` 同一套判定。
pub(crate) fn dsh_home(home: &Path) -> PathBuf {
    std::env::var_os("DSH_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".dsh"))
}

/// 解压一个 `.zstd` 文件到内存，最多 [`MAX_DECODED`] 字节。
///
/// ## 为什么要逐帧循环
///
/// dsh 是**每次追加写一个独立的 zstd frame** 的：本机一个 233KB 的会话文件里有
/// **122 个 frame 起始标记**。`StreamingDecoder` 只解第一帧就 `Ok(0)` 结束，
/// 直接 `read_to_end` 只能拿到 270 字节（实测踩到，整个会话看起来是空的）。
/// 所以要反复构造解码器、跳过已消费的字节，直到文件末尾。
///
/// `StreamingDecoder` 内部用 `BufReader` 包了源，读完后无法精确知道消费了多少字节，
/// 所以这里自己在字节层面切帧：按 zstd magic（`28 B5 2F FD`）找下一帧起点。
/// 帧内出现同样四字节的概率极低，且即便误切也只是那一帧解不出来（`Err` 分支跳过），
/// 不会把整次读取搞坏。
fn inflate(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < data.len() && out.len() < MAX_DECODED {
        let mut decoder = match ruzstd::decoding::StreamingDecoder::new(&data[start..]) {
            Ok(d) => d,
            Err(_) => break,
        };
        let before = out.len();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match decoder.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    out.extend_from_slice(&chunk[..n]);
                    if out.len() >= MAX_DECODED {
                        break;
                    }
                }
                // 单帧读到一半出错：保留已解出的部分，换下一帧继续
                Err(_) => break,
            }
        }
        // 这一帧一个字节都没产出：可能是坏数据或已到末尾，别无限循环
        if out.len() == before {
            start += 4;
        } else {
            start = next_frame(&data, start + 4).unwrap_or(data.len());
        }
    }
    String::from_utf8(out).ok()
}

/// 从 `from` 开始找下一个 zstd frame 的起始标记（`28 B5 2F FD`）。
fn next_frame(data: &[u8], from: usize) -> Option<usize> {
    const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    if from + 4 > data.len() {
        return None;
    }
    (from..=data.len() - 4).find(|&i| data[i..i + 4] == MAGIC)
}

#[derive(Debug, Default)]
struct Totals {
    requests: i64,
    fresh_input: Option<i64>,
    cache_read: Option<i64>,
    output: Option<i64>,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
}

/// 扫一个会话目录，返回 Some((sessionId, cwd, totals))；子代理会话返回 None。
fn scan(dir: &Path) -> Option<(String, Option<String>, Totals)> {
    let text = inflate(&dir.join("session.v4.jsonl.zstd"))?;
    let mut meta: Option<SessionEvent> = None;
    let mut totals = Totals::default();
    let mut sum = (0i64, 0i64, 0i64);
    let mut known = (false, false, false);
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match value.get("type").and_then(|t| t.as_str()) {
            Some("session") => {
                if let Ok(event) = serde_json::from_value::<SessionEvent>(value) {
                    // 子代理会话不进列表：一个任务可能开出十几条，用户看到的只该是发起它的那个
                    let is_subagent = event.origin.as_deref() == Some("subagent")
                        || event.parent_session.as_deref().is_some_and(|p| !p.trim().is_empty());
                    if is_subagent {
                        return None;
                    }
                    totals.first_ts = event.created_at;
                    totals.last_ts = event.created_at;
                    meta = Some(event);
                }
            }
            Some("assistant/message") => {
                let Some(time) = value.get("time").and_then(|t| t.as_i64()) else { continue };
                totals.first_ts = Some(totals.first_ts.map_or(time, |v| v.min(time)));
                totals.last_ts = Some(totals.last_ts.map_or(time, |v| v.max(time)));
                totals.requests += 1;
                let Some(usage) = value.pointer("/data/usage") else { continue };
                let Ok(usage) = serde_json::from_value::<Usage>(usage.clone()) else { continue };
                if let Some(input) = usage.input {
                    let fresh = if dsh_input_includes_cache(&usage) {
                        (input - usage.cache_read.unwrap_or(0)).max(0)
                    } else {
                        input
                    };
                    sum.0 += fresh;
                    known.0 = true;
                }
                if let Some(v) = usage.output {
                    sum.1 += v;
                    known.1 = true;
                }
                if let Some(v) = usage.cache_read {
                    sum.2 += v;
                    known.2 = true;
                }
            }
            _ => {}
        }
    }
    let meta = meta?;
    if meta.id.trim().is_empty() {
        return None;
    }
    totals.fresh_input = known.0.then_some(sum.0);
    totals.output = known.1.then_some(sum.1);
    totals.cache_read = known.2.then_some(sum.2);
    Some((meta.id, meta.cwd.filter(|c| !c.trim().is_empty()), totals))
}

/// 列出 dsh 的会话（只读）。`since` 用来裁掉太久没动的。
pub(crate) fn sessions(home: &Path, since: Option<i64>) -> Vec<SessionItem> {
    if crate::validation::isolated() {
        return Vec::new();
    }
    let root = dsh_home(home).join("sessions");
    let Ok(projects) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut out = Vec::new();
    for project in projects.flatten() {
        let project_dir = project.path();
        if !project_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&project_dir) else { continue };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            // **先看修改时间再解压**：dsh 的会话是 zstd 压缩的，解压一个文件要十几毫秒，
            // 本机 61 个会话全解约 1 秒。而 `since` 只是想要最近活跃的——会话文件最后
            // 修改时间就是它最后一次写入的时刻，用它先筛一遍能省掉绝大部分解压。
            //
            // 留 1 小时余量：文件系统的 mtime 精度、以及「会话在 since 之前创建但在
            // 窗口内仍在追加」这两种情况都需要放宽，宁可多解几个也不要漏掉活跃会话。
            if let Some(since) = since {
                let file = dir.join("session.v4.jsonl.zstd");
                let fresh = std::fs::metadata(&file)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .is_some_and(|mtime| mtime >= since - 60 * 60 * 1000);
                if !fresh {
                    continue;
                }
            }
            let Some((id, cwd, totals)) = scan(&dir) else { continue };
            let last_ts = totals.last_ts.unwrap_or(0);
            if let Some(since) = since {
                if last_ts < since {
                    continue;
                }
            }
            out.push(SessionItem {
                platform: "dsh".into(),
                // 带平台前缀，与 `kimi:` / `pi:` / `opencode:` 同一套约定
                session_id: format!("dsh:{id}"),
                // dsh 的会话文件里没有标题字段；不解析正文所以拿不到首条用户消息。
                // 给 None，界面按既有规则显示「会话 <id 前 8 位>」，不编一个假标题。
                title: None,
                cwd,
                first_ts: totals.first_ts.unwrap_or(last_ts),
                last_ts,
                requests: totals.requests,
                fresh_input: totals.fresh_input,
                cache_read: totals.cache_read,
                // dsh 的 usage 里没有 cacheWrite
                cache_write: None,
                output: totals.output,
                connection_id: None,
                status: String::new(),
                has_source: false,
            });
        }
    }
    out
}

fn file_stamp(path: &Path) -> (i64, i64) {
    let Ok(meta) = std::fs::metadata(path) else { return (0, 0) };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    (meta.len() as i64, mtime)
}

fn save_stamp(conn: &rusqlite::Connection, key: &str, size: i64, time: i64) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO scan_state(path,offset,mtime,parser_initialized) VALUES(?1,?2,?3,1)
         ON CONFLICT(path) DO UPDATE SET offset=?2,mtime=?3,parser_initialized=1",
    )?
    .execute(rusqlite::params![key, size, time])?;
    Ok(())
}

struct ParsedDshFile {
    session_id: String,
    cwd: Option<String>,
    is_subagent: bool,
    records: Vec<Record>,
}

fn extract_dsh_model(value: &serde_json::Value) -> Option<String> {
    for ptr in [
        "/data/model",
        "/data/message/model",
        "/data/message/source/model",
        "/data/step/model",
        "/data/turn/model",
    ] {
        if let Some(m) = value
            .pointer(ptr)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(m.to_string());
        }
    }
    None
}

/// 解析 dsh 单个会话文件用于用量入库：**保留主会话与子代理的全部消耗**，并标出 `is_subagent`
/// 供会话线程页隐藏子代理会话（用量完整、会话列表不刷屏）。
fn parse_usage_file(path: &Path) -> Option<ParsedDshFile> {
    let text = inflate(path)?;
    let mut meta: Option<SessionEvent> = None;
    let mut is_subagent = false;
    let mut prev_user_ts: Option<i64> = None;
    let mut records = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match value.get("type").and_then(|t| t.as_str()) {
            Some("session") => {
                if let Ok(event) = serde_json::from_value::<SessionEvent>(value) {
                    is_subagent = event.origin.as_deref() == Some("subagent")
                        || event.parent_session.as_deref().is_some_and(|p| !p.trim().is_empty());
                    meta = Some(event);
                }
            }
            Some("user/message") => {
                if let Some(t) = value.get("time").and_then(|v| v.as_i64()).filter(|t| *t > 0) {
                    prev_user_ts = Some(t);
                }
            }
            Some("assistant/message") => {
                let Some(ts) = value.get("time").and_then(|t| t.as_i64()).filter(|t| *t > 0) else {
                    continue;
                };
                let Some(sid) = meta.as_ref().map(|m| m.id.as_str()).filter(|s| !s.trim().is_empty()) else {
                    continue;
                };
                let Some(usage_val) = value.pointer("/data/usage") else { continue };
                let Ok(usage) = serde_json::from_value::<Usage>(usage_val.clone()) else { continue };
                let input = usage.input.filter(|v| *v >= 0);
                let output = usage.output.filter(|v| *v >= 0);
                let cache_read = usage.cache_read.filter(|v| *v >= 0);
                let cache_write = usage.cache_write.filter(|v| *v >= 0);
                if [input, output, cache_read, cache_write, usage.total_tokens]
                    .iter()
                    .all(Option::is_none)
                {
                    continue;
                }
                let includes_cache = dsh_input_includes_cache(&usage);
                let semantics = if includes_cache { "includes_cache" } else { "excludes_cache" };
                let total = usage.total_tokens.filter(|v| *v >= 0).or_else(|| {
                    input.zip(output).map(|(i, o)| {
                        if includes_cache {
                            i + o + cache_write.unwrap_or(0)
                        } else {
                            i + o + cache_read.unwrap_or(0) + cache_write.unwrap_or(0)
                        }
                    })
                });
                let duration_ms = prev_user_ts
                    .take()
                    .and_then(|start| ts.checked_sub(start))
                    .filter(|d| (100..=3_600_000).contains(d));
                let msg_id = value
                    .pointer("/data/message/id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("msg-{line_idx}-{ts}"));
                records.push(Record {
                    request: RequestRecord {
                        platform: "dsh".into(),
                        source: "native:dsh:main".into(),
                        dedup_key: format!("{sid}:{msg_id}"),
                        session_id: Some(format!("dsh:{sid}")),
                        ts,
                        model: extract_dsh_model(&value),
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: cache_read,
                        cache_write_tokens: cache_write,
                        total_tokens: total,
                        effort: None,
                    },
                    input_semantics: semantics,
                    credits: None,
                    original_credits: None,
                    billable: None,
                    context_ratio: None,
                    reasoning: None,
                    tool: None,
                    duration_ms,
                    first_token_ms: None,
                });
            }
            _ => {}
        }
    }
    let meta = meta?;
    if meta.id.trim().is_empty() {
        return None;
    }
    Some(ParsedDshFile {
        session_id: meta.id,
        cwd: meta.cwd.filter(|c| !c.trim().is_empty()),
        is_subagent,
        records,
    })
}

/// 增量将 DeepSeek Harness 用量（含子代理）与会话元数据写入 `requests` / `sessions`（P2）。
pub(crate) fn scan_usage(db: &mut impl Store, home: &Path) -> Result<(usize, Vec<String>), String> {
    let root = dsh_home(home).join("sessions");
    let Ok(projects) = std::fs::read_dir(&root) else {
        return Ok((0, Vec::new()));
    };
    let mut changed = 0usize;
    let errors = Vec::new();
    for project in projects.flatten() {
        let project_dir = project.path();
        if !project_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&project_dir) else { continue };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let file = dir.join("session.v4.jsonl.zstd");
            if !file.is_file() {
                continue;
            }
            let stamp = file_stamp(&file);
            let key = format!("dsh-usage:{}", file.to_string_lossy());
            if db.write(|conn| crate::db::get_scan_state(conn, &key) == Some(stamp)) {
                continue;
            }
            let Some(parsed) = parse_usage_file(&file) else {
                let _ = db.write(|conn| save_stamp(conn, &key, stamp.0, stamp.1));
                continue;
            };
            let prefixed_session = format!("dsh:{}", parsed.session_id);
            let n = db
                .write(|conn| -> Result<usize, String> {
                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    let written = source_store::write(&tx, &parsed.records).map_err(|e| e.to_string())?;
                    if let Some(cwd) = parsed.cwd.as_deref() {
                        crate::db::set_session_cwd(&tx, "dsh", &prefixed_session, cwd)
                            .map_err(|e| e.to_string())?;
                    }
                    crate::db::set_session_subagent(&tx, "dsh", &prefixed_session, parsed.is_subagent)
                        .map_err(|e| e.to_string())?;
                    if let Some(last_ts) = parsed.records.iter().map(|r| r.request.ts).max() {
                        crate::db::set_session_activity(&tx, "dsh", &prefixed_session, "done", last_ts)
                            .map_err(|e| e.to_string())?;
                    }
                    save_stamp(&tx, &key, stamp.0, stamp.1).map_err(|e| e.to_string())?;
                    tx.commit().map_err(|e| e.to_string())?;
                    Ok(written)
                })
                .map_err(|e| e.to_string())?;
            changed += n;
        }
    }
    Ok((changed, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的真实 zstd 压缩数据（由 Python `zstandard` 生成，内容见 `ROOT_PLAIN`）。
    ///
    /// 内嵌而不调外部 `zstd` 命令：测试环境的 PATH 未必有它，靠外部命令会让这条测试
    /// 在 CI 上默默跳过，等于没测。字节串是固定的，解压逻辑能真跑。
    const ROOT_ZSTD: &[u8] = &[40, 181, 47, 253, 96, 108, 0, 149, 5, 0, 82, 137, 30, 29, 112, 197, 109, 141, 142, 198, 138, 29, 163, 213, 191, 88, 202, 117, 47, 227, 250, 11, 36, 93, 252, 54, 112, 186, 226, 255, 220, 128, 3, 239, 61, 85, 36, 81, 8, 239, 169, 38, 106, 25, 193, 138, 180, 73, 121, 199, 124, 49, 131, 37, 222, 57, 171, 93, 62, 136, 114, 127, 85, 210, 244, 51, 179, 238, 220, 12, 126, 237, 123, 187, 111, 224, 101, 99, 119, 171, 153, 110, 108, 193, 154, 179, 44, 232, 0, 144, 18, 15, 85, 69, 16, 229, 27, 20, 255, 114, 75, 136, 224, 25, 107, 170, 19, 143, 2, 162, 198, 141, 196, 35, 23, 107, 119, 246, 68, 148, 236, 4, 124, 62, 1, 21, 0, 160, 8, 193, 52, 40, 213, 30, 40, 217, 130, 41, 15, 42, 149, 95, 89, 34, 135, 15, 30, 175, 183, 88, 24, 218, 144, 10, 188, 10, 99, 227, 84, 187, 218, 176, 166, 43, 43, 73, 25, 127, 84, 179, 160, 182, 8, 181, 56, 22, 202, 24];
    const SUB_ZSTD: &[u8] = &[40, 181, 47, 253, 32, 119, 229, 2, 0, 210, 68, 17, 24, 128, 53, 110, 193, 113, 17, 98, 75, 108, 33, 138, 253, 218, 193, 148, 83, 22, 125, 30, 100, 142, 196, 1, 1, 129, 35, 102, 232, 77, 86, 174, 55, 153, 90, 101, 202, 237, 88, 206, 57, 8, 211, 91, 88, 217, 42, 185, 64, 134, 48, 150, 30, 75, 128, 137, 243, 56, 150, 111, 211, 247, 242, 29, 211, 211, 37, 125, 64, 7, 0, 101, 108, 160, 133, 52, 76, 89, 93, 82, 57, 128, 178, 44, 84, 227, 88, 40, 99];

    /// 写一个 .zstd 文件
    fn write(dir: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("session.v4.jsonl.zstd"), bytes).unwrap();
    }

    #[test]
    fn reads_a_real_session_and_skips_subagents() {
        let root = std::env::temp_dir().join(format!("jolu-dsh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions_root = root.join(".dsh").join("sessions").join("--proj--");
        write(&sessions_root.join("root-1"), ROOT_ZSTD);
        write(&sessions_root.join("sub-1"), SUB_ZSTD);

        let list = sessions(&root, None);
        assert_eq!(list.len(), 1, "子代理会话不进列表");
        let item = &list[0];
        assert_eq!(item.platform, "dsh");
        assert_eq!(item.session_id, "dsh:root-1");
        assert_eq!(item.cwd.as_deref(), Some(r"D:\proj"));
        assert_eq!(item.requests, 2, "按 assistant/message 计调用");
        // (1000-200) + (500-100)
        assert_eq!(item.fresh_input, Some(1200), "输入要扣掉缓存读取");
        assert_eq!(item.output, Some(70));
        assert_eq!(item.cache_read, Some(300));
        assert!(item.title.is_none(), "没有标题字段就不编");
        assert!(!item.has_source, "正文不解析就不能说能打开");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// mtime 预筛只是省解压，**不能替代内容时间判断**。
    ///
    /// 优化前每次 overview 都解压全部会话（本机 61 个约 1 秒，模型路由页每 5 秒轮询一次），
    /// 所以加了「先看文件修改时间、太旧就不解压」。这条锁住它的语义边界：
    /// 文件刚写过（mtime 新）但内容里的时间在窗口外的会话，会被 prefilter 放进来、
    /// 再被 `scan` 里的内容时间筛掉——两者是「先粗筛再精筛」，不是替换关系。
    ///
    /// 反过来若只信 mtime，一个被外部工具 touch 过的旧会话会永久留在活跃列表里。
    #[test]
    fn mtime_prefilter_only_gates_decompression_not_activity() {
        let root = std::env::temp_dir().join(format!("jolu-dsh-mtime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions_root = root.join(".dsh").join("sessions").join("--proj--");
        write(&sessions_root.join("root-1"), ROOT_ZSTD);

        let now = chrono::Local::now().timestamp_millis();
        let cutoff = now - 24 * 60 * 60 * 1000;

        // fixture 的内容时间是 1970（1000/3000ms），刚写过的文件也进不了 24 小时窗口：
        // prefilter 放行了解压，但内容时间说了不算
        assert!(
            sessions(&root, Some(cutoff)).is_empty(),
            "内容时间在窗口外的会话不该因为文件被 touch 过就算活跃"
        );

        // 不设 since 时照常返回：证明上面是「被筛选」不是「读不出来」
        assert_eq!(sessions(&root, None).len(), 1, "不筛时间时应能正常读出");

        // 文件 mtime 在窗口外时根本不解压（这条测的是性能路径，结果等价）
        let file = sessions_root.join("root-1").join("session.v4.jsonl.zstd");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 24 * 3600);
        let f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        f.set_modified(old).unwrap();
        drop(f);
        assert!(sessions(&root, Some(cutoff)).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn since_filters_and_non_zstd_files_are_skipped() {
        let root = std::env::temp_dir().join(format!("jolu-dsh-since-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions_root = root.join(".dsh").join("sessions").join("--proj--");
        write(&sessions_root.join("root-1"), ROOT_ZSTD);
        // 会话时间是 1000/3000（1970 年）；要 2000 年之后的就是空的
        let cutoff = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z").unwrap().timestamp_millis();
        assert!(sessions(&root, Some(cutoff)).is_empty(), "太久没动的不进活跃列表");
        // 不是 zstd 的文件：解压失败就跳过，不产生空会话
        let bad = sessions_root.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("session.v4.jsonl.zstd"), b"not zstd at all").unwrap();
        let list = sessions(&root, None);
        assert_eq!(list.len(), 1, "坏文件被跳过，只剩那条正常的");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_usage_includes_subagent_tokens_while_hiding_subagent_from_session_threads() {
        let root = std::env::temp_dir().join(format!("jolu-dsh-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sessions_root = root.join(".dsh").join("sessions").join("--proj--");
        write(&sessions_root.join("root-1"), ROOT_ZSTD);
        write(&sessions_root.join("sub-1"), SUB_ZSTD);

        let mut db = crate::db::open(Path::new(":memory:")).unwrap();
        let (n, errs) = scan_usage(&mut db, &root).unwrap();
        assert!(errs.is_empty());
        assert!(n >= 2, "主会话与子代理用量都应写入 requests");

        // 核对 sessions.is_subagent 标记
        let sub_flag: i64 = db
            .query_row("SELECT is_subagent FROM sessions WHERE platform='dsh' AND session_id='dsh:sub-1'", [], |r| r.get(0))
            .unwrap();
        let root_flag: i64 = db
            .query_row("SELECT is_subagent FROM sessions WHERE platform='dsh' AND session_id='dsh:root-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!((root_flag, sub_flag), (0, 1));

        // 会话线程列表只展示主会话（子代理被 is_subagent = 0 隐藏）
        let listed = crate::session_threads::list(
            &db,
            &crate::session_threads::SessionFilter { platform: Some("dsh".into()), skip_external: true, ..Default::default() },
        )
        .unwrap();
        assert_eq!(listed.sessions.len(), 1);
        assert_eq!(listed.sessions[0].session_id, "dsh:root-1");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
