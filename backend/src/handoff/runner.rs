use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::session_threads::Message;
use super::baseline::{claude_baseline, codex_baseline, NativeBaseline};
use super::digest::{relevant_digests, reusable, DigestRow};
use super::executor::{
    codex_config_text, codex_mcp_audit, executor_args, fork_args, kill_tree, CliExecutor, Totals,
};
use super::facts::{extract_facts, render_facts, Facts};
use super::input::{condense, display_range, estimate_tokens, recent_start};
use super::pipeline::{
    check_file_name, covered_range, estimate_generate, generate, generate_forked, platform_name,
    refine, render_local_export, render_markdown, snapshot_hash, step, suggested_file_name,
    ChatEvent, Draft, GenerateRequest, RenderMeta,
};
use super::prompts::refine_prompt;
use super::store::{
    ensure_schema, insert_handoff, latest_handoff, load_digests, save_digest, HandoffMeta,
    HandoffRecord, NewHandoff,
};
use super::{CHAT_EVENT, MAX_REFINE_ROUNDS, PROMPT_VERSION, RECENT_RAW_TOKENS};

/* ─────────────────────────── 对话任务 ─────────────────────────── */

pub(crate) struct Job {
    pub(crate) id: String,
    pub(crate) platform: String,
    pub(crate) session_id: String,
    pub(crate) snapshot_to: usize,
    /// 快照内容的校验值，写入交接版本，用于判断之后有没有新消息或改写
    pub(crate) snapshot_hash: String,
    pub(crate) facts: Facts,
    pub(crate) focus: String,
    pub(crate) target: String,
    pub(crate) executor_platform: String,
    pub(crate) connection_id: String,
    pub(crate) model: Option<String>,
    pub(crate) exe: PathBuf,
    pub(crate) native_used: bool,
    pub(crate) drafts: Vec<Draft>,
    /// 生成后从库里读出的有效纪要，供修改时按意见挑选
    pub(crate) digests: Vec<DigestRow>,
    pub(crate) busy: bool,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) pid: Arc<Mutex<Option<u32>>>,
    pub(crate) totals: Arc<Totals>,
}

impl Job {
    pub(crate) fn executor(&self) -> Result<CliExecutor, String> {
        Ok(CliExecutor {
            platform: self.executor_platform.clone(),
            exe: self.exe.clone(),
            args: executor_args(&self.executor_platform, self.model.as_deref())?,
            cwd: None,
            cancel: self.cancel.clone(),
            pid: self.pid.clone(),
            totals: self.totals.clone(),
        })
    }

    /// D-34：分叉续接原会话的执行器（只用于首次生成；修改草稿仍走隔离执行器，不需要原会话）
    pub(crate) fn fork_executor(&self, cwd: Option<PathBuf>) -> Result<CliExecutor, String> {
        Ok(CliExecutor {
            platform: self.executor_platform.clone(),
            exe: self.exe.clone(),
            args: fork_args(&self.executor_platform, &self.session_id, self.model.as_deref())?,
            cwd,
            cancel: self.cancel.clone(),
            pid: self.pid.clone(),
            totals: self.totals.clone(),
        })
    }

    pub(crate) fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(pid) = *self.pid.lock().unwrap_or_else(|p| p.into_inner()) {
            kill_tree(pid);
        }
    }
}

/// 每次只有一个活动任务
pub(crate) static JOB: LazyLock<Mutex<Option<Job>>> = LazyLock::new(|| Mutex::new(None));

/// 应用退出（`RunEvent::Exit`）时结束正在运行的压缩进程，避免留下孤儿 CLI 进程
pub fn shutdown() {
    let job = JOB.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(job) = job {
        job.stop();
    }
}
pub(crate) static JOB_SEQ: AtomicU64 = AtomicU64::new(1);
pub(crate) static SCHEMA_READY: AtomicBool = AtomicBool::new(false);

pub(crate) fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

pub(crate) type Shared = Arc<Mutex<Connection>>;

pub(crate) fn shared(app: &AppHandle) -> Shared {
    app.state::<crate::Db>().0.clone()
}

pub(crate) fn with_conn<T>(db: &Shared, work: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    if !SCHEMA_READY.load(Ordering::Acquire) {
        ensure_schema(&conn).map_err(|e| e.to_string())?;
        SCHEMA_READY.store(true, Ordering::Release);
    }
    work(&conn)
}

pub(crate) fn with_job<T>(job_id: &str, work: impl FnOnce(&mut Job) -> Result<T, String>) -> Result<T, String> {
    let mut guard = JOB.lock().unwrap_or_else(|p| p.into_inner());
    match guard.as_mut().filter(|j| j.id == job_id) {
        Some(job) => work(job),
        None => Err("压缩对话已结束，请重新开始".into()),
    }
}

pub(crate) fn job_active(job_id: &str) -> bool {
    JOB.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|j| j.id == job_id && !j.cancel.load(Ordering::Acquire))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatPayload {
    pub(crate) job_id: String,
    pub(crate) event: ChatEvent,
}

/// 读出会话：源文件、详细解析的消息、工作目录；源文件不存在时为 None
pub(crate) struct Loaded {
    pub(crate) path: PathBuf,
    pub(crate) lines: Vec<String>,
    pub(crate) messages: Vec<Message>,
    pub(crate) cwd: Option<String>,
}

pub(crate) fn load_session(db: &Shared, platform: &str, session_id: &str) -> Result<Option<Loaded>, String> {
    let path = with_conn(db, |c| crate::session_threads::source_path(c, platform, session_id).map_err(|e| e.to_string()))?;
    let Some(path) = path else { return Ok(None) };
    let lines = crate::session_threads::read_lines(&path)?;
    let messages = crate::session_threads::parse_detail(platform, &lines);
    let cwd = crate::session_threads::read_cwd(&path);
    Ok(Some(Loaded { path, lines, messages, cwd }))
}

/// 原工具摘要只在压缩前的原文读不到时使用
pub(crate) fn native_if_missing(platform: &str, lines: &[String]) -> Option<NativeBaseline> {
    let text = lines.join("\n");
    let b = match platform {
        "claude" => claude_baseline(&text),
        "codex" => codex_baseline(&text),
        _ => None,
    }?;
    b.raw_missing().then_some(b)
}

#[derive(Debug, Clone, Serialize)]
pub struct HandoffState {
    /// 有可读的源文件且解析出消息
    pub readable: bool,
    pub total: usize,
    pub latest: Option<HandoffMeta>,
    /// 已有纪要连续覆盖的条数（= 覆盖到的最后一条的对外编号）
    pub digest_covered_to: usize,
    /// 给界面显示的「复用纪要 第 a–b 条」（1 起，闭区间）；没有可复用纪要时为 null
    pub digest_range: Option<[usize; 2]>,
    /// 上一版交接之后新增的「第 a–b 条」（没有交接版本时为全部消息）；没有新消息时为 null
    pub new_range: Option<[usize; 2]>,
    /// 上次之后没有新消息，可以不调用模型直接复用
    pub reusable: bool,
    /// 预计发送约 N token；可以直接复用时为 0
    pub estimate_tokens: usize,
}

/// `covered` 条之后到 `total` 条为止的新增范围（1 起，闭区间）
pub(crate) fn new_range(covered: usize, total: usize) -> Option<[usize; 2]> {
    (total > covered).then(|| display_range(covered, total - 1))
}

/// `focus` 为本次要用的交接主题；不传时按上一版的主题判断能否复用
pub(crate) async fn cmd_state(app: AppHandle, platform: String, session_id: String, focus: Option<String>) -> Result<HandoffState, String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let latest = with_conn(&db, |c| latest_handoff(c, &platform, &session_id).map_err(|e| e.to_string()))?;
        let stored = with_conn(&db, |c| load_digests(c, &platform, &session_id).map_err(|e| e.to_string()))?;
        let Some(loaded) = load_session(&db, &platform, &session_id)? else {
            return Ok(HandoffState { readable: false, total: 0, latest: latest.map(|l| l.meta), digest_covered_to: 0, digest_range: None, new_range: None, reusable: false, estimate_tokens: 0 });
        };
        let total = loaded.messages.len();
        let msgs = condense(&loaded.messages);
        let (reused, _) = reusable(&stored, &msgs, recent_start(&msgs, RECENT_RAW_TOKENS));
        // 与 generate 相同的判断：条数、交接主题、快照内容都一致才复用
        let snapshot = snapshot_hash(&msgs);
        let can_reuse = latest.clone().is_some_and(|l| {
            let focus = focus.clone().unwrap_or_else(|| l.meta.focus.clone());
            l.previous().matches(total, &focus, &snapshot)
        });
        let estimate = if can_reuse {
            0
        } else {
            let facts = extract_facts(&loaded.messages, loaded.cwd.as_deref().map(Path::new), Some(&loaded.path));
            estimate_generate(&msgs, &stored, &render_facts(&facts), native_if_missing(&platform, &loaded.lines).as_ref())
        };
        let digest_covered_to = reused.last().map(|r| r.to_idx + 1).unwrap_or(0);
        Ok(HandoffState {
            readable: total > 0,
            total,
            new_range: new_range(latest.as_ref().map(|l| l.meta.covered_to).unwrap_or(0), total),
            latest: latest.map(|l| l.meta),
            digest_covered_to,
            digest_range: covered_range(digest_covered_to),
            reusable: can_reuse,
            estimate_tokens: estimate,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn connection_info(db: &Shared, connection_id: &str) -> Result<(String, String), String> {
    with_conn(db, |c| {
        c.query_row("SELECT platform, name FROM connections WHERE id=?1", [connection_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "所选连接不存在".to_string())
    })
}

pub(crate) async fn cmd_start(app: AppHandle, platform: String, session_id: String, connection_id: String, model: Option<String>, focus: String, target: String, channel: Option<String>) -> Result<String, String> {
    let db = shared(&app);
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let (exec_platform, name) = connection_info(&db, &connection_id)?;
        let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        executor_args(&exec_platform, model.as_deref())?;
        if !crate::automation::current_connection_ids(&db).contains(&connection_id) {
            return Err(format!("{name} 不是 {} 当前登录的账号；压缩只使用命令行当前登录的账号，请先在供应商页启用该账号", platform_name(&exec_platform)));
        }
        let tool = crate::automation::tool_name(&exec_platform).ok_or("该连接不支持压缩")?;
        // D-34：在原工具里分叉。只能用会话所属工具的账号（续接要读它自己的会话记录）
        let forked = channel.as_deref() == Some("fork");
        if forked {
            if exec_platform != platform {
                return Err(format!("在原工具里分叉只能用 {} 的账号；请选 {} 当前登录的账号，或改用 Jolu 整理", platform_name(&platform), platform_name(&platform)));
            }
            fork_args(&exec_platform, &session_id, model.as_deref())?;
        }
        let exe = crate::session_threads::find_executable(tool).ok_or_else(|| format!("未检测到 {} 命令，请安装或配置后重试", platform_name(&exec_platform)))?;
        // 一开始就固定快照：之后新写入的消息不进入本次压缩
        let loaded = load_session(&db, &platform, &session_id)?.ok_or("会话源文件不可读，无法压缩")?;
        let snapshot_to = loaded.messages.len();
        if snapshot_to == 0 {
            return Err("会话没有可读的消息，无法压缩".into());
        }
        let facts = extract_facts(&loaded.messages, loaded.cwd.as_deref().map(Path::new), Some(&loaded.path));
        let stored = with_conn(&db, |c| load_digests(c, &platform, &session_id).map_err(|e| e.to_string()))?;
        let previous = with_conn(&db, |c| latest_handoff(c, &platform, &session_id).map_err(|e| e.to_string()))?.map(HandoffRecord::previous);
        let native = native_if_missing(&platform, &loaded.lines);
        let snapshot = snapshot_hash(&condense(&loaded.messages));

        let job_id = format!("handoff-{}-{}", now_ms(), JOB_SEQ.fetch_add(1, Ordering::AcqRel));
        let job = Job {
            id: job_id.clone(),
            platform: platform.clone(),
            session_id: session_id.clone(),
            snapshot_to,
            snapshot_hash: snapshot,
            facts: facts.clone(),
            focus: focus.clone(),
            target: target.clone(),
            executor_platform: exec_platform,
            connection_id,
            model,
            exe,
            native_used: native.is_some(),
            drafts: Vec::new(),
            digests: Vec::new(),
            busy: true,
            cancel: Arc::new(AtomicBool::new(false)),
            pid: Arc::new(Mutex::new(None)),
            totals: Arc::new(Totals::default()),
        };
        // Claude 按项目目录找会话：分叉时在原会话的工作目录里运行（工具全部禁用，不会改动项目文件）
        let exec = if forked { job.fork_executor(loaded.cwd.as_deref().map(PathBuf::from).filter(|dir| dir.is_dir()))? } else { job.executor()? };
        if let Some(old) = JOB.lock().unwrap_or_else(|p| p.into_inner()).replace(job) {
            old.stop();
        }
        let req = GenerateRequest { platform, session_id, messages: loaded.messages, facts, stored, native, previous, focus, target, now_ms: now_ms() };
        let id = job_id.clone();
        std::thread::Builder::new()
            .name("jolu-handoff".into())
            .spawn(move || run_generate(app2, db, id, exec, req, forked))
            .map_err(|e| format!("启动压缩失败：{e}"))?;
        Ok(job_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 纪要落库后再转发；任务已取消时照常落库（请求已经产生用量），只是不再推给弹窗
pub(crate) fn forward(app: &AppHandle, db: &Shared, job_id: &str, event: ChatEvent) {
    let event = match event {
        ChatEvent::DigestSaved(mut row) => {
            match with_conn(db, |c| save_digest(c, &row).map_err(|e| e.to_string())) {
                Ok(id) => row.id = id,
                Err(e) => eprintln!("[交接] 保存纪要失败：{e}"),
            }
            ChatEvent::DigestSaved(row)
        }
        other => other,
    };
    if job_active(job_id) {
        let _ = app.emit(CHAT_EVENT, ChatPayload { job_id: job_id.to_string(), event });
    }
}

pub(crate) fn run_generate(app: AppHandle, db: Shared, job_id: String, exec: CliExecutor, req: GenerateRequest, forked: bool) {
    let (platform, session_id) = (req.platform.clone(), req.session_id.clone());
    let messages = req.messages.clone();
    let emit = |event: ChatEvent| forward(&app, &db, &job_id, event);
    if exec.platform == "codex" {
        if let Some(note) = codex_mcp_audit(codex_config_text().as_deref()) {
            emit(step(note, 0, 0));
        }
    }
    let result = if forked { generate_forked(&exec, req, &emit) } else { generate(&exec, req, &emit) };
    let digests = with_conn(&db, |c| load_digests(c, &platform, &session_id).map_err(|e| e.to_string())).unwrap_or_default();
    let msgs = condense(&messages);
    let active = reusable(&digests, &msgs, msgs.len()).0;
    let _ = with_job(&job_id, |job| {
        job.busy = false;
        job.digests = active;
        if let Ok(draft) = result {
            job.drafts.push(draft);
        }
        Ok(())
    });
}

pub(crate) async fn cmd_refine(app: AppHandle, job_id: String, instruction: String) -> Result<(), String> {
    let db = shared(&app);
    let (exec, draft, facts, digests) = with_job(&job_id, |job| {
        if job.busy {
            return Err("请等待当前步骤完成".into());
        }
        let draft = job.drafts.last().cloned().ok_or("还没有草稿，请先完成压缩")?;
        if draft.round > MAX_REFINE_ROUNDS {
            return Err("已达到修改上限".into());
        }
        if instruction.trim().is_empty() {
            return Err("请先填写修改意见".into());
        }
        job.busy = true;
        Ok((job.executor()?, draft, job.facts.clone(), job.digests.clone()))
    })?;
    std::thread::Builder::new()
        .name("jolu-handoff-refine".into())
        .spawn(move || {
            let emit = |event: ChatEvent| forward(&app, &db, &job_id, event);
            // 修改草稿同样会调用 CLI，隔离不完全时也要提示
            if exec.platform == "codex" {
                if let Some(note) = codex_mcp_audit(codex_config_text().as_deref()) {
                    emit(step(note, 0, 0));
                }
            }
            let result = refine(&exec, &draft, &facts, &instruction, &digests, &emit);
            let _ = with_job(&job_id, |job| {
                job.busy = false;
                if let Ok(next) = result {
                    job.drafts.push(next);
                }
                Ok(())
            });
        })
        .map_err(|e| format!("启动修改失败：{e}"))?;
    Ok(())
}

pub(crate) async fn cmd_estimate_refine(job_id: String, instruction: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // 锁内只取数据，渲染与估算在锁外做
        let (draft, facts, digests) = with_job(&job_id, |job| Ok((job.drafts.last().cloned().ok_or("还没有草稿")?, job.facts.clone(), job.digests.clone())))?;
        let digests: Vec<DigestRow> = digests.into_iter().filter(|d| d.prompt_version == PROMPT_VERSION).collect();
        let ids = relevant_digests(&instruction, &digests);
        let chosen: Vec<DigestRow> = digests.into_iter().filter(|d| ids.contains(&d.id)).collect();
        Ok(estimate_tokens(&refine_prompt(&draft.narrative_md, &render_facts(&facts), &chosen, &instruction)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Serialize)]
pub struct HandoffResult {
    pub version: i64,
    pub markdown: String,
    /// 建议的文件名（格式待确认）
    pub file_name: String,
    pub path_warnings: Vec<String>,
    /// 给界面显示的「覆盖第 a–b 条」（1 起，闭区间）
    pub covered_range: Option<[usize; 2]>,
}

/// 写入交接版本所需的数据（从任务里复制出来，写库时不持有任务锁）
pub(crate) struct FinishData {
    pub(crate) platform: String,
    pub(crate) session_id: String,
    pub(crate) snapshot_to: usize,
    pub(crate) snapshot_hash: String,
    pub(crate) draft: Draft,
    pub(crate) facts: Facts,
    pub(crate) focus: String,
    pub(crate) target: String,
    pub(crate) draft_rounds: usize,
    pub(crate) executor_platform: String,
    pub(crate) connection_id: String,
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
    pub(crate) calls: u64,
    pub(crate) native_used: bool,
}

/// 用选定的版本写入新的交接版本：写库成功后才移除任务；写库失败时草稿保留、可以再试
pub(crate) fn finish_job(job_id: &str, round: u32, project: &str, now: i64, insert: impl FnOnce(&NewHandoff) -> Result<i64, String>) -> Result<HandoffResult, String> {
    let data = with_job(job_id, |job| {
        if job.busy {
            return Err("请等待当前步骤完成".to_string());
        }
        let draft = job.drafts.iter().find(|d| d.round == round).cloned().ok_or_else(|| format!("没有 v{round} 草稿"))?;
        job.busy = true;
        let usage = job.totals.usage.lock().unwrap_or_else(|p| p.into_inner());
        Ok(FinishData {
            platform: job.platform.clone(),
            session_id: job.session_id.clone(),
            snapshot_to: job.snapshot_to,
            snapshot_hash: job.snapshot_hash.clone(),
            draft,
            facts: job.facts.clone(),
            focus: job.focus.clone(),
            target: job.target.clone(),
            draft_rounds: job.drafts.len(),
            executor_platform: job.executor_platform.clone(),
            connection_id: job.connection_id.clone(),
            model: job.model.clone(),
            input_tokens: usage.input(),
            output_tokens: usage.output(),
            calls: job.totals.calls.load(Ordering::Acquire),
            native_used: job.native_used,
        })
    })?;
    let written = insert(&NewHandoff {
        platform: &data.platform,
        session_id: &data.session_id,
        covered_to: data.snapshot_to,
        draft: &data.draft,
        facts: &data.facts,
        focus: &data.focus,
        target: &data.target,
        draft_rounds: data.draft_rounds,
        executor_platform: &data.executor_platform,
        connection_id: &data.connection_id,
        model: data.model.as_deref(),
        input_tokens: data.input_tokens,
        output_tokens: data.output_tokens,
        calls: data.calls,
        now_ms: now,
        snapshot_hash: &data.snapshot_hash,
    });
    let version = match written {
        Ok(v) => v,
        Err(e) => {
            let _ = with_job(job_id, |job| {
                job.busy = false;
                Ok(())
            });
            return Err(format!("保存交接版本失败，草稿已保留：{e}"));
        }
    };
    {
        let mut guard = JOB.lock().unwrap_or_else(|p| p.into_inner());
        if guard.as_ref().is_some_and(|j| j.id == job_id) {
            guard.take();
        }
    }
    let meta = RenderMeta {
        project: project.to_string(),
        topic: data.focus.clone(),
        target: data.target.clone(),
        platform: data.platform.clone(),
        session_id: data.session_id.clone(),
        total: data.snapshot_to,
        covered_to: data.snapshot_to,
        created_ms: now,
        version: Some(version),
        native_note: data.native_used,
    };
    Ok(HandoffResult {
        version,
        markdown: render_markdown(&data.draft.narrative_md, &data.facts, &meta),
        file_name: suggested_file_name(project, &data.target, now),
        path_warnings: data.draft.path_warnings.clone(),
        covered_range: covered_range(data.snapshot_to),
    })
}

pub(crate) async fn cmd_finish(app: AppHandle, job_id: String, round: u32, project: String) -> Result<HandoffResult, String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || finish_job(&job_id, round, &project, now_ms(), |h| with_conn(&db, |c| insert_handoff(c, h).map_err(|e| e.to_string()))))
        .await
        .map_err(|e| e.to_string())?
}

/// 结束进程树，丢弃草稿；已保存的纪要保留
pub(crate) async fn cmd_cancel(job_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let job = {
            let mut guard = JOB.lock().unwrap_or_else(|p| p.into_inner());
            if guard.as_ref().is_some_and(|j| j.id == job_id) { guard.take() } else { None }
        };
        // taskkill 在锁外执行
        if let Some(job) = job {
            job.stop();
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 本地交接导出（4.7）：没有可用模型时不调用模型，直接从会话记录生成交接文档。
/// 不写交接版本、不推进检查点——它不是压缩。
pub(crate) async fn cmd_local_export(app: AppHandle, platform: String, session_id: String, project: String, topic: String, target: String) -> Result<String, String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let loaded = load_session(&db, &platform, &session_id)?.ok_or("会话源文件不可读，无法导出")?;
        if loaded.messages.is_empty() {
            return Err("会话没有可读的消息，无法导出".into());
        }
        let facts = extract_facts(&loaded.messages, loaded.cwd.as_deref().map(Path::new), Some(&loaded.path));
        let meta = RenderMeta {
            project,
            topic,
            target,
            platform: platform.clone(),
            session_id,
            total: loaded.messages.len(),
            covered_to: loaded.messages.len(),
            created_ms: now_ms(),
            version: None,
            native_note: false,
        };
        Ok(render_local_export(&loaded.messages, &facts, &meta))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 不调用模型（画板 52）：用最新的交接版本重新排版
pub(crate) async fn cmd_render(app: AppHandle, platform: String, session_id: String, project: String, topic: String, target: String) -> Result<String, String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let latest = with_conn(&db, |c| latest_handoff(c, &platform, &session_id).map_err(|e| e.to_string()))?.ok_or("还没有交接版本，请先压缩会话")?;
        let lines = with_conn(&db, |c| crate::session_threads::source_path(c, &platform, &session_id).map_err(|e| e.to_string()))?
            .map(|p| crate::session_threads::read_lines(&p))
            .transpose()?
            .unwrap_or_default();
        let meta = RenderMeta {
            project,
            topic,
            target,
            platform: platform.clone(),
            session_id,
            total: latest.meta.covered_to,
            covered_to: latest.meta.covered_to,
            created_ms: latest.meta.created_ms,
            version: Some(latest.meta.version),
            native_note: native_if_missing(&platform, &lines).is_some(),
        };
        Ok(render_markdown(&latest.narrative_md, &latest.facts, &meta))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileNameCheck {
    /// 规范后的文件名（补 `.md`）
    pub name: String,
    /// 目标目录里已有同名文件
    pub exists: bool,
    /// 同名时建议的另存名（`…-2.md`），覆盖前由界面明确确认（画板 54-09）
    pub suggestion: Option<String>,
}

/// 目录里第一个不重名的文件名：原名可用就用原名，否则依次尝试 `-2`、`-3`……
pub fn available_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    (2..10_000).map(|n| format!("{stem}-{n}{ext}")).find(|candidate| !dir.join(candidate).exists()).unwrap_or_else(|| name.to_string())
}

pub(crate) fn file_name_check(file_name: &str, dir: Option<&Path>) -> Result<FileNameCheck, String> {
    let name = check_file_name(file_name)?;
    let exists = dir.is_some_and(|d| d.join(&name).exists());
    let suggestion = dir.filter(|_| exists).map(|d| available_name(d, &name));
    Ok(FileNameCheck { name, exists, suggestion })
}

/// 文件名规则（画板 54-09）；给出 `dir` 时同时检查同名文件并给出 `-2` 之类的建议名

/// 系统保存框：默认目录是应用数据目录下的 `handoffs/`；`prefer_project_dir` 给出可访问的项目目录时从那里开始（D-02）。
/// 取消保存时返回 None
pub(crate) async fn cmd_save(app: AppHandle, markdown: String, file_name: String, prefer_project_dir: Option<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let name = check_file_name(&file_name)?;
    let (data_dir, _) = crate::app::state::data_paths(&app)?;
    let default_dir = data_dir.join("handoffs");
    std::fs::create_dir_all(&default_dir).map_err(|e| format!("创建交接目录失败：{e}"))?;
    let dir = prefer_project_dir.map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or(default_dir);
    // 默认另存为首个可用名称；用户主动改回已有文件时由系统保存框确认覆盖。
    let name = available_name(&dir, &name);
    let mut dialog = app.dialog().file().set_title("保存交接文档").add_filter("Markdown", &["md"]).set_directory(&dir).set_file_name(&name);
    if let Some(window) = app.get_webview_window(crate::MAIN) {
        dialog = dialog.set_parent(&window);
    }
    let selected = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file()).await.map_err(|e| e.to_string())?;
    let Some(file) = selected else { return Ok(None) };
    let path = file.into_path().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, markdown.as_bytes()).map_err(|e| format!("保存失败：{e}"))?;
        Ok(Some(path.display().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}
