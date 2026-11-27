use serde::{Deserialize, Serialize};

use crate::session_threads::Message;
use super::baseline::NativeBaseline;
use super::digest::{needs_rollup, range_hash, relevant_digests, reusable, DigestRow, LEVEL_ROLLUP, LEVEL_SEGMENT};
use super::executor::{ExecOutput, Executor, StopReason};
use super::facts::{inline_code, redact, render_facts, summary_line, Facts};
use super::input::{
    condense, display_no, display_range, estimate_tokens, msg_ref, plan_segments, range_ref,
    range_tokens, recent_start, serialize, CondensedMsg,
};
use super::prompts::{digest_label, digest_prompt, fork_prompt, refine_prompt, rollup_prompt, synth_prompt, target_text, DIGEST_PROMPT, ROLLUP_PROMPT, SYNTH_PROMPT};
use super::{
    DIGEST_OUTPUT_ESTIMATE, DIGEST_ROLLUP_TOKENS, DIGEST_SEGMENT_TOKENS, INPUT_BUDGET,
    MAX_REFINE_ROUNDS, NATIVE_NOTE, PROMPT_VERSION, RECENT_RAW_TOKENS, SECTIONS,
};

/* ─────────────────────────── 质量检查 ─────────────────────────── */

pub(crate) const TOOL_MARKERS: [&str; 6] = ["<tool_use", "<function_calls", "<invoke ", "<invoke>", "\"type\":\"tool_use\"", "<tool_call"];

pub(crate) fn has_tool_call(raw: &str) -> bool {
    TOOL_MARKERS.iter().any(|m| raw.contains(m))
}

/// 删掉 `<analysis>…</analysis>`，取 `<summary>` 里的内容
pub(crate) fn summary_body(raw: &str) -> Result<String, String> {
    let mut text = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("<analysis>") {
        text.push_str(&rest[..start]);
        match rest[start..].find("</analysis>") {
            Some(end) => rest = &rest[start + end + "</analysis>".len()..],
            None => return Err("输出被截断：<analysis> 没有结束".into()),
        }
    }
    text.push_str(rest);
    let Some(start) = text.find("<summary>") else { return Err("输出格式不符：缺少 <summary>".into()) };
    let body = &text[start + "<summary>".len()..];
    let Some(end) = body.find("</summary>") else { return Err("输出被截断：<summary> 没有结束".into()) };
    let body = body[..end].trim();
    if body.is_empty() {
        return Err("输出格式不符：<summary> 为空".into());
    }
    Ok(body.to_string())
}

pub(crate) fn common_checks(raw: &str, stop: StopReason) -> Result<String, String> {
    match stop {
        StopReason::MaxTokens => return Err("输出被截断：达到输出上限".into()),
        StopReason::Refusal => return Err("模型拒绝了这次请求".into()),
        _ => {}
    }
    // 只查 `<summary>` 正文：`<analysis>` 里讨论工具不算；真实的工具调用以执行器的结构化 `used_tools` 为准
    let body = summary_body(raw)?;
    if has_tool_call(&body) {
        return Err("输出里有工具调用".into());
    }
    Ok(body)
}

/// 纪要的检查：没有被截断、没有工具调用、有 `<summary>`
pub(crate) fn check_digest(raw: &str, stop: StopReason) -> Result<String, String> {
    common_checks(raw, stop)
}

/// `## 标题` 切分；`###` 不算
pub(crate) fn split_sections(md: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in md.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            out.push((title.trim().to_string(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

/// 是否引用了消息编号 `#n`：对外编号从 1 起，`#0` 不算有效引用
pub(crate) fn has_ref(text: &str) -> bool {
    text.split('#').skip(1).any(|rest| {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<usize>().is_ok_and(|n| n >= display_no(0))
    })
}

/// 合成与修改的检查（不通过的不进入草稿）：输出被截断、有工具调用、缺少章节、「下一步」没有引用 `#n`
pub fn check_output(raw: &str, stop: StopReason) -> Result<String, String> {
    let body = common_checks(raw, stop)?;
    let sections = split_sections(&body);
    let missing: Vec<&str> = SECTIONS.iter().copied().filter(|t| !sections.iter().any(|(h, _)| h == t)).collect();
    if !missing.is_empty() {
        return Err(format!("缺少章节：{}", missing.join("、")));
    }
    let next = sections.iter().find(|(h, _)| h == "下一步").map(|(_, b)| b.as_str()).unwrap_or("");
    if !has_ref(next) {
        return Err("「下一步」没有引用 #n".into());
    }
    Ok(body)
}

pub(crate) const PATH_EXTS: [&str; 48] = [
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "jsonl", "md", "toml", "yaml", "yml", "py", "go", "java", "kt", "css", "scss", "html", "vue", "svelte", "sql", "sh",
    "ps1", "bat", "cmd", "txt", "lock", "pen", "png", "svg", "jpg", "cs", "cpp", "c", "h", "hpp", "rb", "php", "swift", "xml", "ini", "cfg", "env", "log", "csv", "dart",
];

/// 叙述里像文件路径的片段：最后一段带常见扩展名，或带 `/` 且以 `/` 结尾的目录；去掉行号后缀，跳过 URL
pub(crate) fn path_candidates(text: &str) -> Vec<String> {
    let delimiters = |c: char| c.is_whitespace() || "`\"'()（）「」『』《》，。；：、,;<>[]{}|*“”‘’".contains(c);
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(delimiters).filter(|s| !s.is_empty()) {
        if raw.contains("://") || raw.starts_with('#') {
            continue;
        }
        let mut t = raw.trim_end_matches(['.', ',', '!', '?', ':']);
        // `src/a.ts:12`、`src/a.ts:12:3`
        for _ in 0..2 {
            if let Some((head, tail)) = t.rsplit_once(':') {
                if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
                    t = head;
                }
            }
        }
        let t = t.strip_prefix("./").unwrap_or(t);
        if t.len() < 3 || !t.chars().all(|c| c.is_ascii_alphanumeric() || "_-./\\:~@+".contains(c)) {
            continue;
        }
        let last = t.rsplit(['/', '\\']).next().unwrap_or(t);
        let ext_ok = last.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && PATH_EXTS.contains(&ext.to_ascii_lowercase().as_str()));
        let dir_ok = t.ends_with('/') && t.trim_end_matches('/').contains('/');
        if (ext_ok || dir_ok) && !out.iter().any(|p| p == t) {
            out.push(t.to_string());
        }
    }
    out
}

pub(crate) fn normalize_path(p: &str) -> String {
    p.replace('\\', "/").to_lowercase()
}

pub(crate) fn unknown_in(narrative: &str, haystack: &str) -> Vec<String> {
    path_candidates(narrative).into_iter().filter(|p| !haystack.contains(&normalize_path(p))).collect()
}

/// 叙述里出现、但会话里从未出现过的文件路径（预览提示「以下路径未在会话中出现：…」，不阻止保存）
pub fn unknown_paths(narrative: &str, messages: &[Message]) -> Vec<String> {
    let mut haystack = String::new();
    for m in messages {
        haystack.push_str(&m.text);
        haystack.push('\n');
        for t in &m.tools {
            haystack.push_str(&t.summary);
            haystack.push('\n');
            haystack.push_str(t.output.as_deref().unwrap_or(""));
            haystack.push('\n');
            for p in &t.paths {
                haystack.push_str(p);
                haystack.push('\n');
            }
        }
    }
    unknown_in(narrative, &normalize_path(&haystack))
}

/* ─────────────────────────── 生成与修改流程 ─────────────────────────── */

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub round: u32,
    pub narrative_md: String,
    pub path_warnings: Vec<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// 推给对话弹窗的消息；序列化为 `{ "type": "step" | "digest_saved" | "draft" | "error", ...字段 }`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// Jolu 的步骤消息；`done / total` 是本轮已完成 / 计划的模型调用数，没有进度含义时都为 0
    Step { text: String, done: usize, total: usize },
    /// 新生成的纪要（调用方落库后转发，`id` 为库里的 id）
    DigestSaved(DigestRow),
    Draft(Draft),
    Error { text: String, retryable: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviousHandoff {
    /// 旧版本仍可查看，但不能跳过新提示词的来源边界检查。
    pub prompt_version: String,
    pub covered_to: usize,
    pub narrative_md: String,
    pub focus: String,
    /// 上一版快照的校验值；旧记录没有时只比较条数
    pub snapshot_hash: Option<String>,
}

impl PreviousHandoff {
    /// 可以直接复用上一版叙述：条数、交接主题、快照内容都一致
    pub fn matches(&self, total: usize, focus: &str, snapshot: &str) -> bool {
        self.covered_to == total
            && self.prompt_version == PROMPT_VERSION
            && total > 0
            && self.focus.trim() == focus.trim()
            && !self.narrative_md.trim().is_empty()
            && self.snapshot_hash.as_deref().is_none_or(|h| h == snapshot)
    }
}

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub platform: String,
    pub session_id: String,
    /// 已按 `snapshot_to` 截断的消息
    pub messages: Vec<Message>,
    pub facts: Facts,
    /// 库里已有的纪要（含已归并的）
    pub stored: Vec<DigestRow>,
    /// 只有压缩前原文读不到时才由调用方传入
    pub native: Option<NativeBaseline>,
    pub previous: Option<PreviousHandoff>,
    pub focus: String,
    pub target: String,
    pub now_ms: i64,
}

#[derive(Debug, Default)]
pub(crate) struct Usage {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) input_unknown: bool,
    pub(crate) output_unknown: bool,
}

impl Usage {
    pub(crate) fn add(&mut self, o: &ExecOutput) {
        match o.input_tokens {
            Some(n) => self.input += n,
            None => self.input_unknown = true,
        }
        match o.output_tokens {
            Some(n) => self.output += n,
            None => self.output_unknown = true,
        }
    }
    pub(crate) fn input(&self) -> Option<u64> {
        (!self.input_unknown).then_some(self.input)
    }
    pub(crate) fn output(&self) -> Option<u64> {
        (!self.output_unknown).then_some(self.output)
    }
}

pub(crate) enum Attempt {
    Ok(String),
    Exceeded,
    Rejected(String),
}

/// 调用一次，检查不通过时自动重试 1 次
pub(crate) fn attempt(exec: &dyn Executor, prompt: &str, check: &dyn Fn(&str, StopReason) -> Result<String, String>, usage: &mut Usage) -> Result<Attempt, String> {
    let mut reason = String::new();
    for _ in 0..2 {
        let out = exec.run(prompt)?;
        usage.add(&out);
        if out.context_exceeded {
            return Ok(Attempt::Exceeded);
        }
        let checked = if out.used_tools { Err("输出里有工具调用".to_string()) } else { check(&out.text, out.stop) };
        match checked {
            Ok(body) => return Ok(Attempt::Ok(body)),
            Err(r) => reason = r,
        }
    }
    Ok(Attempt::Rejected(reason))
}

pub(crate) fn step(text: impl Into<String>, done: usize, total: usize) -> ChatEvent {
    ChatEvent::Step { text: text.into(), done, total }
}

pub(crate) fn native_row(n: &NativeBaseline, req: &GenerateRequest) -> DigestRow {
    DigestRow {
        id: 0,
        platform: req.platform.clone(),
        session_id: req.session_id.clone(),
        level: LEVEL_SEGMENT,
        from_idx: 0,
        to_idx: 0,
        range_hash: String::new(),
        tokens: estimate_tokens(&n.summary),
        // 原工具摘要也是会话来源的文字：进入合成请求前先遮蔽，并转义冒充的消息标记
        digest_md: crate::handoff_facts::escape_role_markers(&redact(&n.summary)),
        source: n.source.to_string(),
        rolled_into: None,
        prompt_version: PROMPT_VERSION.into(),
        created_ms: req.now_ms,
    }
}

pub(crate) fn ranges_text(ranges: &[(usize, usize)]) -> String {
    ranges.iter().map(|&(a, b)| range_ref(a, b)).collect::<Vec<_>>().join("、")
}

/// D-34：在原工具里分叉生成。原工具带着完整原会话（不落盘的分叉），只发一次提示词，不写段落纪要；
/// 草稿的检查与普通压缩相同（八节齐全、没被截断、下一步引用用户原话的 #n）。
/// 失败时先发出 `ChatEvent::Error` 再返回 Err
pub fn generate_forked(exec: &dyn Executor, req: GenerateRequest, emit: &dyn Fn(ChatEvent)) -> Result<Draft, String> {
    let fail = |text: String, retryable: bool| -> Result<Draft, String> {
        emit(ChatEvent::Error { text: text.clone(), retryable });
        Err(text)
    };
    let total = req.messages.len();
    if total == 0 {
        return fail("会话没有可读的消息，无法压缩".into(), false);
    }
    emit(step(format!("已抽取事实：{}", summary_line(&req.facts)), 0, 0));
    emit(step("在原工具里分叉原会话并写交接草稿（不改原会话、不留新会话）…", 0, 1));
    let facts_md = render_facts(&req.facts);
    let user_nos = user_numbers(&req.facts);
    let check = |raw: &str, stop: StopReason| check_output(raw, stop).and_then(|body| check_refs(&body, total, &user_nos).map(|_| body));
    let mut usage = Usage::default();
    let prompt = fork_prompt(&req.target, &req.focus, &facts_md);
    match attempt(exec, &prompt, &check, &mut usage) {
        Err(e) => fail(e, true),
        Ok(Attempt::Exceeded) => fail("原会话已超出模型上下文，无法在原工具里分叉压缩；请改用 Jolu 整理".into(), false),
        Ok(Attempt::Rejected(r)) => fail(format!("草稿未通过检查：{r}"), true),
        Ok(Attempt::Ok(narrative)) => {
            emit(step("检查通过：章节齐全、没有被截断、下一步引用了 #n", 1, 1));
            let draft = Draft {
                round: 1,
                path_warnings: unknown_paths(&narrative, &req.messages),
                narrative_md: narrative,
                input_tokens: usage.input(),
                output_tokens: usage.output(),
            };
            emit(ChatEvent::Draft(draft.clone()));
            Ok(draft)
        }
    }
}

/// 纯流程：复用或补齐纪要（每生成一份就通过 `DigestSaved` 交给调用方落库）、按需归并、合成、检查、降级。
/// 失败时先发出 `ChatEvent::Error` 再返回 Err
pub fn generate(exec: &dyn Executor, req: GenerateRequest, emit: &dyn Fn(ChatEvent)) -> Result<Draft, String> {
    let fail = |text: String, retryable: bool| -> Result<Draft, String> {
        emit(ChatEvent::Error { text: text.clone(), retryable });
        Err(text)
    };
    let total = req.messages.len();
    emit(step(format!("已抽取事实：{}", summary_line(&req.facts)), 0, 0));
    let msgs = condense(&req.messages);
    let snapshot = snapshot_hash(&msgs);
    let reuse = req.previous.as_ref().filter(|p| p.matches(total, &req.focus, &snapshot));
    if let Some(prev) = reuse {
        emit(step("上次之后没有新消息，直接复用上一版叙述，不调用模型", 0, 0));
        let draft = Draft { round: 1, path_warnings: unknown_paths(&prev.narrative_md, &req.messages), narrative_md: prev.narrative_md.clone(), input_tokens: Some(0), output_tokens: Some(0) };
        emit(ChatEvent::Draft(draft.clone()));
        return Ok(draft);
    }
    if total == 0 {
        return fail("会话没有可读的消息，无法压缩".into(), false);
    }

    let facts_md = render_facts(&req.facts);
    let user_nos = user_numbers(&req.facts);
    let synth_check = |raw: &str, stop: StopReason| check_output(raw, stop).and_then(|body| check_refs(&body, total, &user_nos).map(|_| body));
    let mut stored = req.stored.clone();
    let mut temp_id = -1i64;
    let mut usage = Usage::default();
    let mut recent_tokens = RECENT_RAW_TOKENS;
    let mut rollup_limit = DIGEST_ROLLUP_TOKENS;
    let mut make_row = |level: i64, from: usize, to: usize, md: String| -> DigestRow {
        let row = DigestRow {
            id: temp_id,
            platform: req.platform.clone(),
            session_id: req.session_id.clone(),
            level,
            from_idx: from,
            to_idx: to,
            range_hash: range_hash(&msgs, from, to),
            tokens: estimate_tokens(&md),
            digest_md: md,
            source: "raw".into(),
            rolled_into: None,
            prompt_version: PROMPT_VERSION.into(),
            created_ms: req.now_ms,
        };
        temp_id -= 1;
        row
    };

    // 降级次数：纪要段长减半、最近窗口减半、归并纪要各算一次；第 3 次仍超限报「会话过长，无法压缩」
    let mut degrades = 0usize;
    let mut segment_max = DIGEST_SEGMENT_TOKENS;
    let mut recent_halved = false;
    loop {
        let upto = recent_start(&msgs, recent_tokens);
        let (mut active, need) = reusable(&stored, &msgs, upto);
        let mut queue: std::collections::VecDeque<(usize, usize)> = need.iter().flat_map(|&(a, b)| plan_segments(&msgs, a, b, segment_max)).collect();
        let reused_text = match (active.first(), active.last()) {
            (Some(a), Some(b)) => format!("复用 {} 份纪要（{}）", active.len(), range_ref(a.from_idx, b.to_idx)),
            _ => "没有可复用的纪要".into(),
        };
        let new_text = if queue.is_empty() { "新增消息都在最近原文里".to_string() } else { format!("整理新增 {}", ranges_text(&need)) };
        emit(step(format!("{reused_text}，{new_text} …"), 0, queue.len() + 1));

        let mut done = 0;
        while let Some((a, b)) = queue.pop_front() {
            let planned = done + queue.len() + 2;
            emit(step(format!("整理纪要 第 {}/{} 段（{}）…", done + 1, done + queue.len() + 1, range_ref(a, b)), done, planned));
            match attempt(exec, &digest_prompt(&msgs, a, b), &check_digest, &mut usage) {
                Err(e) => return fail(e, true),
                Ok(Attempt::Ok(md)) => {
                    let row = make_row(LEVEL_SEGMENT, a, b, md);
                    emit(ChatEvent::DigestSaved(row.clone()));
                    stored.push(row.clone());
                    active.push(row);
                    done += 1;
                }
                Ok(Attempt::Exceeded) => {
                    degrades += 1;
                    if degrades >= 3 || a == b {
                        return fail("会话过长，无法压缩".into(), false);
                    }
                    segment_max /= 2;
                    emit(step(format!("整理 {} 的纪要时超出模型上下文，段长减半后重新切段…", range_ref(a, b)), done, planned));
                    let mut parts = plan_segments(&msgs, a, b, segment_max);
                    if parts.len() < 2 {
                        let mid = a + (b - a) / 2;
                        parts = vec![(a, mid), (mid + 1, b)];
                    }
                    for part in parts.into_iter().rev() {
                        queue.push_front(part);
                    }
                }
                Ok(Attempt::Rejected(r)) => return fail(format!("{} 的纪要未通过检查：{r}", range_ref(a, b)), true),
            }
        }
        let planned = done + 1;
        active.sort_by_key(|r| r.from_idx);

        let mut rollups = 0;
        while let Some(ids) = needs_rollup(&active, rollup_limit) {
            rollups += 1;
            if rollups > 8 {
                break;
            }
            let group: Vec<DigestRow> = active.iter().filter(|r| ids.contains(&r.id)).cloned().collect();
            let (a, b) = (group[0].from_idx, group[group.len() - 1].to_idx);
            emit(step(format!("纪要较多，归并较早的 {} 份（{}）…", group.len(), range_ref(a, b)), planned - 1, planned));
            match attempt(exec, &rollup_prompt(&group), &check_digest, &mut usage) {
                Err(e) => return fail(e, true),
                Ok(Attempt::Ok(md)) => {
                    let row = make_row(LEVEL_ROLLUP, a, b, md);
                    emit(ChatEvent::DigestSaved(row.clone()));
                    for r in stored.iter_mut().filter(|r| ids.contains(&r.id)) {
                        r.rolled_into = Some(row.id);
                    }
                    stored.push(row.clone());
                    active.retain(|r| !ids.contains(&r.id));
                    active.push(row);
                    active.sort_by_key(|r| r.from_idx);
                }
                Ok(Attempt::Exceeded) => return fail("归并纪要时超出模型上下文".into(), true),
                Ok(Attempt::Rejected(r)) => return fail(format!("早期纪要未通过检查：{r}"), true),
            }
        }

        let recent_from = upto.max(active.last().map(|r| r.to_idx + 1).unwrap_or(0));
        let mut digests: Vec<DigestRow> = req.native.iter().map(|n| native_row(n, &req)).collect();
        digests.extend(active.iter().cloned());
        let prompt = synth_prompt(&req.target, &req.focus, &facts_md, &digests, &serialize(&msgs, recent_from, total - 1));
        let result = if estimate_tokens(&prompt) > INPUT_BUDGET {
            Attempt::Exceeded
        } else {
            emit(step("合成交接草稿…", planned - 1, planned));
            match attempt(exec, &prompt, &synth_check, &mut usage) {
                Ok(a) => a,
                Err(e) => return fail(e, true),
            }
        };
        match result {
            Attempt::Ok(narrative) => {
                emit(step("检查通过：章节齐全、没有被截断、下一步引用了 #n", planned, planned));
                let draft = Draft {
                    round: 1,
                    path_warnings: unknown_paths(&narrative, &req.messages),
                    narrative_md: narrative,
                    input_tokens: usage.input(),
                    output_tokens: usage.output(),
                };
                emit(ChatEvent::Draft(draft.clone()));
                return Ok(draft);
            }
            Attempt::Rejected(r) => return fail(format!("草稿未通过检查：{r}"), true),
            Attempt::Exceeded => {
                degrades += 1;
                if degrades >= 3 {
                    return fail("会话过长，无法压缩".into(), false);
                }
                if !recent_halved {
                    recent_tokens /= 2;
                    recent_halved = true;
                    emit(step("输入超出模型上下文，最近原文窗口减半后重试…", 0, 0));
                } else {
                    rollup_limit = DIGEST_ROLLUP_TOKENS / 4;
                    emit(step("仍然超出模型上下文，归并纪要后重试…", 0, 0));
                }
            }
        }
    }
}

/// 整个快照的校验值：判断「上次之后没有新消息」时连同内容一起比较（消息被改写时条数不变）
pub(crate) fn snapshot_hash(msgs: &[CondensedMsg]) -> String {
    if msgs.is_empty() { String::new() } else { range_hash(msgs, 0, msgs.len() - 1) }
}

/// 用户消息的对外编号（「下一步」至少要引用其中一条）：用全部用户消息；旧记录没有时退回最近 30 条
pub(crate) fn user_numbers(facts: &Facts) -> Vec<usize> {
    if facts.user_indices.is_empty() {
        facts.user_timeline.iter().map(|(i, _)| display_no(*i)).collect()
    } else {
        facts.user_indices.iter().map(|i| display_no(*i)).collect()
    }
}

/// 「下一步」里引用的 `#n`
pub(crate) fn next_step_refs(body: &str) -> Vec<usize> {
    let sections = split_sections(body);
    let next = sections.iter().find(|(h, _)| h == "下一步").map(|(_, b)| b.as_str()).unwrap_or("");
    next.split('#').skip(1).filter_map(|rest| rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).collect()
}

/// 「下一步」引用的编号不超过总条数，且至少一处指向用户消息（材料里的注入指令不能冒充用户要求）。
/// `total` 为 0 或不知道用户消息时跳过对应检查
pub(crate) fn check_refs(body: &str, total: usize, user_nos: &[usize]) -> Result<(), String> {
    let refs = next_step_refs(body);
    if let Some(n) = refs.iter().find(|&&n| total > 0 && n > total) {
        return Err(format!("「下一步」引用了不存在的消息 #{n}（共 {total} 条）"));
    }
    if !user_nos.is_empty() && !refs.iter().any(|n| user_nos.contains(n)) {
        return Err("「下一步」没有引用用户消息的 #n".into());
    }
    Ok(())
}

/// 修改草稿：输入只有当前草稿、事实区、意见和与意见相关的纪要（最多 3 份），不带会话原文；
/// 第 11 次修改报「已达到修改上限」
pub fn refine(exec: &dyn Executor, draft: &Draft, facts: &Facts, instruction: &str, digests: &[DigestRow], emit: &dyn Fn(ChatEvent)) -> Result<Draft, String> {
    let fail = |text: String, retryable: bool| -> Result<Draft, String> {
        emit(ChatEvent::Error { text: text.clone(), retryable });
        Err(text)
    };
    if draft.round > MAX_REFINE_ROUNDS {
        return fail("已达到修改上限".into(), false);
    }
    if instruction.trim().is_empty() {
        return fail("请先填写修改意见".into(), false);
    }
    let current_digests: Vec<DigestRow> = digests.iter().filter(|d| d.prompt_version == PROMPT_VERSION).cloned().collect();
    let ids = relevant_digests(instruction, &current_digests);
    let mut chosen: Vec<DigestRow> = current_digests.iter().filter(|d| ids.contains(&d.id)).cloned().collect();
    chosen.sort_by_key(|d| d.from_idx);
    let facts_md = render_facts(facts);
    emit(step(
        if chosen.is_empty() {
            "按意见修改草稿…".to_string()
        } else {
            format!("按意见修改草稿，附带 {} 份相关纪要（{}）…", chosen.len(), chosen.iter().map(digest_label).collect::<Vec<_>>().join("、"))
        },
        0,
        1,
    ));
    let mut usage = Usage::default();
    let user_nos = user_numbers(facts);
    let check = |raw: &str, stop: StopReason| check_output(raw, stop).and_then(|body| check_refs(&body, facts.total, &user_nos).map(|_| body));
    match attempt(exec, &refine_prompt(&draft.narrative_md, &facts_md, &chosen, instruction), &check, &mut usage) {
        Err(e) => fail(e, true),
        Ok(Attempt::Exceeded) => fail("修改请求超出模型上下文".into(), true),
        Ok(Attempt::Rejected(r)) => fail(format!("修改后的草稿未通过检查：{r}"), true),
        Ok(Attempt::Ok(narrative)) => {
            // 修改请求不带会话原文：以事实区、纪要和上一版草稿为准；上一版已提示过的路径仍在就继续提示
            // 当前工作区的 Git 补充不是原会话来源，不能据此豁免新增路径提醒。
            let mut conversation_facts = facts.clone();
            conversation_facts.git = None;
            conversation_facts.transcript_path = None;
            let mut known = format!("{}\n{}\n", render_facts(&conversation_facts), current_digests.iter().map(|d| d.digest_md.as_str()).collect::<Vec<_>>().join("\n"));
            known.push_str(&draft.narrative_md);
            let haystack = normalize_path(&known);
            let mut warnings: Vec<String> = draft.path_warnings.iter().filter(|p| path_candidates(&narrative).contains(p)).cloned().collect();
            for p in unknown_in(&narrative, &haystack) {
                if !warnings.contains(&p) {
                    warnings.push(p);
                }
            }
            emit(step("检查通过", 1, 1));
            let next = Draft { round: draft.round + 1, narrative_md: narrative, path_warnings: warnings, input_tokens: usage.input(), output_tokens: usage.output() };
            emit(ChatEvent::Draft(next.clone()));
            Ok(next)
        }
    }
}

/// 「预计发送约 N token」：按与 `generate` 相同的规则估算本次要发送的输入
pub(crate) fn estimate_generate(msgs: &[CondensedMsg], stored: &[DigestRow], facts_md: &str, native: Option<&NativeBaseline>) -> usize {
    if msgs.is_empty() {
        return 0;
    }
    let upto = recent_start(msgs, RECENT_RAW_TOKENS);
    let (active, need) = reusable(stored, msgs, upto);
    let segments: Vec<(usize, usize)> = need.iter().flat_map(|&(a, b)| plan_segments(msgs, a, b, DIGEST_SEGMENT_TOKENS)).collect();
    let digest_overhead = estimate_tokens(DIGEST_PROMPT);
    let digest_inputs: usize = segments.iter().map(|&(a, b)| range_tokens(msgs, a, b) + digest_overhead).sum();
    let mut digest_tokens: usize = active.iter().map(|r| r.tokens).sum::<usize>() + segments.len() * DIGEST_OUTPUT_ESTIMATE;
    let rollup = if digest_tokens > DIGEST_ROLLUP_TOKENS { digest_tokens + estimate_tokens(ROLLUP_PROMPT) } else { 0 };
    digest_tokens = digest_tokens.min(DIGEST_ROLLUP_TOKENS);
    digest_tokens += native.map(|n| estimate_tokens(&n.summary)).unwrap_or(0);
    let covered = active.last().map(|r| r.to_idx + 1).unwrap_or(0).max(segments.last().map(|s| s.1 + 1).unwrap_or(0));
    let recent_from = upto.max(covered);
    let synth = estimate_tokens(SYNTH_PROMPT) + estimate_tokens(facts_md) + digest_tokens + range_tokens(msgs, recent_from, msgs.len() - 1);
    digest_inputs + rollup + synth
}

/* ─────────────────────────── 排版 ─────────────────────────── */

#[derive(Debug, Clone, Default)]
pub struct RenderMeta {
    /// 项目名（文件名与标题用）
    pub project: String,
    /// 交接主题；为空时标题用项目名
    pub topic: String,
    pub target: String,
    pub platform: String,
    pub session_id: String,
    pub total: usize,
    pub covered_to: usize,
    pub created_ms: i64,
    pub version: Option<i64>,
    /// 早期部分用了原工具的压缩摘要
    pub native_note: bool,
}

pub(crate) fn platform_name(platform: &str) -> &str {
    match platform {
        "claude" => "Claude Code",
        "codex" => "Codex",
        other => other,
    }
}

pub(crate) fn format_ms(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "未知".into())
}

/// 「下一步」的第一条（去掉编号）
pub(crate) fn first_next_step(sections: &[(String, String)]) -> Option<String> {
    let body = sections.iter().find(|(h, _)| h == "下一步").map(|(_, b)| b.as_str())?;
    body.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| {
        let l = l.trim_start_matches(['-', '*', ' ']);
        let digits = l.chars().take_while(|c| c.is_ascii_digit()).count();
        let rest = &l[digits..];
        if digits > 0 && (rest.starts_with('.') || rest.starts_with('、') || rest.starts_with(')')) {
            rest.trim_start_matches(['.', '、', ')']).trim().to_string()
        } else {
            l.to_string()
        }
    })
}

/// 覆盖条数（不含的结束下标）→「第 a–b 条」显示范围（1 起，闭区间）；一条都没有时为 None
pub fn covered_range(covered_to: usize) -> Option<[usize; 2]> {
    (covered_to > 0).then(|| display_range(0, covered_to - 1))
}

/// 覆盖范围文字：「覆盖第 1–24 条」
pub(crate) fn covered_text(covered_to: usize) -> String {
    match covered_range(covered_to) {
        Some([a, b]) if a == b => format!("覆盖第 {a} 条"),
        Some([a, b]) => format!("覆盖第 {a}–{b} 条"),
        None => "尚未覆盖任何消息".into(),
    }
}

/// 排版（计划 3.3 第 5 条）：交接目标、当前状态与卡点、下一步放在最前，其余叙述章节，然后是事实区；
/// 文档末尾再重复一次「下一步」的第一条，并给出接手第一步建议。
///
/// 遮蔽只作用于模型叙述与用户填写的文字（标题、主题），在遮蔽之后再拼接事实区：事实区里会话来源的字段
/// （用户原话、命令、验证输出、报错）在抽取时已经遮蔽，Git 快照、文件清单、回查路径来自 git 与文件系统，不再整体遮蔽，
/// 所以合成请求里的事实区与最终文档一致
pub fn render_markdown(narrative: &str, facts: &Facts, meta: &RenderMeta) -> String {
    let sections = split_sections(narrative);
    let title = if meta.topic.trim().is_empty() { meta.project.trim() } else { meta.topic.trim() };
    let mut out = format!("# 会话交接 · {}\n\n", if title.is_empty() { "未命名项目" } else { title });
    out.push_str(&format!(
        "> 项目：{} · 来源：{} 会话 {} · 共 {} 条消息，{}\n",
        if meta.project.trim().is_empty() { "未识别项目" } else { meta.project.trim() },
        platform_name(&meta.platform),
        inline_code(&meta.session_id),
        meta.total,
        covered_text(meta.covered_to)
    ));
    out.push_str(&format!(
        "> 交接给：{} · 生成于 {} · Jolu {PROMPT_VERSION}{}\n",
        target_text(&meta.target),
        format_ms(meta.created_ms),
        meta.version.map(|v| format!(" · 第 {v} 版")).unwrap_or_default()
    ));
    if meta.native_note {
        out.push_str(&format!("> 早期部分{NATIVE_NOTE}。\n"));
    }
    out.push('\n');
    for title in SECTIONS {
        let body = sections.iter().find(|(h, _)| h == title).map(|(_, b)| b.trim()).filter(|b| !b.is_empty()).unwrap_or("（无）");
        out.push_str(&format!("## {title}\n\n{body}\n\n"));
    }
    let mut tail = String::from("\n## 接手第一步\n\n");
    if let Some(first) = first_next_step(&sections) {
        tail.push_str(&format!("- 下一步的第一条：{first}\n"));
    }
    tail.push_str("- 先核对 Git 快照，运行「最后一次验证」里的命令确认状态。\n");
    format!("{}{}{}", redact(&out), render_facts(facts), redact(&tail))
}

/// 本地交接导出里「最近的对话」取几条、每条最多多少字。
pub(crate) const LOCAL_RECENT: usize = 6;
pub(crate) const LOCAL_CLIP: usize = 1200;

/// 本地交接导出（4.7）：没有可用模型时，不调用任何模型，直接把会话整理成可交给另一个 AI 的文档——
/// 最初的需求、最近几条对话的原文摘录、以及抽取出的结构化事实（文件、命令、验证、Git 快照）。
/// 文档开头写明这不是模型压缩；所有正文都过同一套脱敏。
pub fn render_local_export(messages: &[crate::session_threads::Message], facts: &Facts, meta: &RenderMeta) -> String {
    let title = if meta.topic.trim().is_empty() { meta.project.trim() } else { meta.topic.trim() };
    let mut out = format!("# 本地交接导出 · {}\n\n", if title.is_empty() { "未命名项目" } else { title });
    out.push_str("> **这不是模型压缩。** Jolu 没有调用任何模型，下面是从会话记录里直接摘出的原文与结构化事实，未经归纳；\n");
    out.push_str("> 接手的 AI 请先通读「最初的需求」与「最近的对话」，再按事实核对当前状态。\n");
    out.push_str(&format!(
        "> 项目：{} · 来源：{} 会话 {} · 共 {} 条消息 · 交接给：{} · 导出于 {}\n\n",
        if meta.project.trim().is_empty() { "未识别项目" } else { meta.project.trim() },
        platform_name(&meta.platform),
        inline_code(&meta.session_id),
        meta.total,
        target_text(&meta.target),
        format_ms(meta.created_ms)
    ));
    let speaker = |role: &str| if role == "user" { "用户" } else { "AI" };
    let excerpt = |text: &str| {
        let text = text.trim();
        let clipped: String = text.chars().take(LOCAL_CLIP).collect();
        if clipped.chars().count() < text.chars().count() { format!("{clipped}…（已截断）") } else { clipped }
    };
    out.push_str("## 最初的需求\n\n");
    match messages.iter().find(|m| m.role == "user" && !m.text.trim().is_empty()) {
        Some(first) => out.push_str(&format!("{}\n\n", quote_block(&excerpt(&first.text)))),
        None => out.push_str("（会话里没有可读的用户消息）\n\n"),
    }
    out.push_str("## 最近的对话\n\n");
    let recent: Vec<_> = messages.iter().filter(|m| !m.text.trim().is_empty()).collect();
    let start = recent.len().saturating_sub(LOCAL_RECENT);
    if recent.is_empty() {
        out.push_str("（无）\n\n");
    }
    for message in &recent[start..] {
        out.push_str(&format!("**{}**\n\n{}\n\n", speaker(&message.role), quote_block(&excerpt(&message.text))));
    }
    let tail = "\n## 接手第一步\n\n- 先核对 Git 快照，运行「最后一次验证」里的命令确认状态。\n- 原文摘录可能已过时，以仓库当前内容为准。\n";
    format!("{}{}{}", redact(&out), render_facts(facts), redact(tail))
}

/// 每行加 `> `，让摘录和导出说明分得开，也避免原文里的 `#` 标题打乱文档结构。
pub(crate) fn quote_block(text: &str) -> String {
    text.lines().map(|line| if line.trim().is_empty() { ">".to_string() } else { format!("> {line}") }).collect::<Vec<_>>().join("\n")
}

/* ─────────────────────────── 文件名 ─────────────────────────── */

/// 文件名规则（画板 54-09，待确认：本线无法用 Pencil 读取画板，按 Windows 文件名限制实现）：
/// 不能为空、不超过 120 个字符、不含 `\ / : * ? " < > |` 与控制字符、不以点或空格结尾、不用系统保留名；没有 `.md` 时补上
pub fn check_file_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("请输入文件名".into());
    }
    if name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)) {
        return Err("文件名不能包含 \\ / : * ? \" < > | 等字符".into());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("文件名不能以点或空格结尾".into());
    }
    let with_ext = if name.to_lowercase().ends_with(".md") { name.to_string() } else { format!("{name}.md") };
    if with_ext.chars().count() > 120 {
        return Err("文件名不能超过 120 个字符".into());
    }
    let stem = with_ext[..with_ext.len() - 3].split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem[3..].chars().all(|c| c.is_ascii_digit() && c != '0');
    if stem.is_empty() || RESERVED.contains(&stem.as_str()) || numbered {
        return Err("文件名不能使用系统保留名称".into());
    }
    Ok(with_ext)
}

/// 建议的文件名：`<项目>-交接给<对象>-<日期时间>.md`（D-07：交接对象只影响文案和文件名；格式待确认）
pub fn suggested_file_name(project: &str, target: &str, created_ms: i64) -> String {
    use chrono::TimeZone;
    let clean = |s: &str| -> String {
        let s: String = s.trim().chars().map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { '-' } else { c }).collect();
        s.trim_matches(['.', ' ', '-']).chars().take(40).collect()
    };
    let project = clean(project);
    let stamp = chrono::Local.timestamp_millis_opt(created_ms).single().map(|d| d.format("%Y%m%d-%H%M").to_string()).unwrap_or_default();
    let name = format!("{}-交接给{}-{stamp}.md", if project.is_empty() { "session".into() } else { project }, clean(target_text(target)));
    check_file_name(&name).unwrap_or_else(|_| format!("handoff-{stamp}.md"))
}
