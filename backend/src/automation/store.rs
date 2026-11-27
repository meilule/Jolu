use std::collections::HashSet;
use std::path::Path;

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::notify::TaskNotice;
use super::{
    clip, display_name, next_fire,
    runner::{add, Outcome, Progress, MAX_SKILLS, RATE_LIMIT_RETRY_MS, UNTRUSTED_DIR_REASON},
    schedule::{apply_check, backoff, QuotaVerdict},
    tool_name, ACTIVE, MISSED_GRACE_MS, MISSING_SNAPSHOT,
};

/* ─────────────────────────── 结构 ─────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    /// once / daily / weekly
    pub freq: String,
    /// HH:MM，按本机时区
    pub time: String,
    /// 1 = 周一 … 7 = 周日，仅 weekly
    pub weekday: Option<u32>,
    /// YYYY-MM-DD，仅 once
    pub date: Option<String>,
    pub timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskInput {
    pub name: String,
    pub platform: String,
    pub connection_id: String,
    pub model: Option<String>,
    pub cwd: String,
    /// 选择已有会话时在原会话追加；为空表示新建会话
    pub session_id: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub skills: Vec<String>,
    /// now / schedule / quota
    pub launch: String,
    pub schedule: Option<Schedule>,
    pub auto_resume: bool,
    pub wait_deadline: Option<i64>,
    /// 同账号排队（D-09），缺省为开启
    #[serde(default = "yes")]
    pub serial_queue: bool,
}

pub(crate) fn yes() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub connection_id: String,
    pub model: Option<String>,
    pub cwd: String,
    pub session_id: Option<String>,
    pub prompt: String,
    pub skills: Vec<String>,
    pub launch: String,
    pub schedule: Option<Schedule>,
    pub auto_resume: bool,
    pub wait_deadline: Option<i64>,
    /// active / paused / finished / cancelled
    pub state: String,
    pub next_run_at: Option<i64>,
    pub missed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 同账号排队（D-09）：关闭后可与同账号任务并行
    pub serial_queue: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: i64,
    pub task_id: i64,
    pub task_name: String,
    pub platform: String,
    pub connection_id: String,
    /// now / schedule / quota / manual
    pub source: String,
    /// queued / waiting / running / attention / completed / failed / cancelled
    pub status: String,
    pub block_kind: Option<String>,
    pub reason: Option<String>,
    pub resets_at: Option<i64>,
    pub last_check_at: Option<i64>,
    pub next_check_at: Option<i64>,
    pub check_failures: i64,
    pub resume_pending: bool,
    pub session_id: Option<String>,
    pub account: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exec_ms: i64,
    pub wait_ms: i64,
    pub exec_started_at: Option<i64>,
    pub wait_started_at: Option<i64>,
    pub waits: i64,
    /// 含缓存的全部输入；执行器没有返回时为 None，不以 0 代替
    pub input_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub result: Option<String>,
    pub files: Vec<String>,
    pub error: Option<String>,
    /// 运行创建时的执行参数；旧版本未保存的历史明确为未知。
    #[serde(skip)]
    pub(crate) config: Option<Task>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub ts: i64,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
}

pub(crate) const TASK_COLS: &str = "id, name, platform, connection_id, model, cwd, session_id, prompt, skills, launch, schedule, auto_resume, wait_deadline, state, next_run_at, missed_at, created_at, updated_at, serial_queue";

pub(crate) fn task_from(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    let skills: String = r.get(8)?;
    let schedule: Option<String> = r.get(10)?;
    Ok(Task {
        id: r.get(0)?,
        name: r.get(1)?,
        platform: r.get(2)?,
        connection_id: r.get(3)?,
        model: r.get(4)?,
        cwd: r.get(5)?,
        session_id: r.get(6)?,
        prompt: r.get(7)?,
        skills: serde_json::from_str(&skills).unwrap_or_default(),
        launch: r.get(9)?,
        schedule: schedule.and_then(|s| serde_json::from_str(&s).ok()),
        auto_resume: r.get::<_, i64>(11)? != 0,
        wait_deadline: r.get(12)?,
        state: r.get(13)?,
        next_run_at: r.get(14)?,
        missed_at: r.get(15)?,
        created_at: r.get(16)?,
        updated_at: r.get(17)?,
        serial_queue: r.get::<_, i64>(18)? != 0,
    })
}

pub(crate) const RUN_COLS: &str = "r.id, r.task_id, COALESCE(json_extract(r.task_snapshot, '$.name'), '未知任务'), COALESCE(json_extract(r.task_snapshot, '$.platform'), 'unknown'), COALESCE(json_extract(r.task_snapshot, '$.connection_id'), ''), r.source, r.status, r.block_kind, r.reason, r.resets_at, r.last_check_at, r.next_check_at, r.check_failures, r.resume_pending, r.session_id, r.account, r.model, r.created_at, r.started_at, r.ended_at, r.exec_ms, r.wait_ms, r.exec_started_at, r.wait_started_at, r.waits, r.input_tokens, r.cache_read_tokens, r.output_tokens, r.result, r.files, r.error, r.task_snapshot";

pub(crate) fn run_from(r: &rusqlite::Row) -> rusqlite::Result<Run> {
    let files: String = r.get(29)?;
    Ok(Run {
        id: r.get(0)?,
        task_id: r.get(1)?,
        task_name: r.get(2)?,
        platform: r.get(3)?,
        connection_id: r.get(4)?,
        source: r.get(5)?,
        status: r.get(6)?,
        block_kind: r.get(7)?,
        reason: r.get(8)?,
        resets_at: r.get(9)?,
        last_check_at: r.get(10)?,
        next_check_at: r.get(11)?,
        check_failures: r.get(12)?,
        resume_pending: r.get::<_, i64>(13)? != 0,
        session_id: r.get(14)?,
        account: r.get(15)?,
        model: r.get(16)?,
        created_at: r.get(17)?,
        started_at: r.get(18)?,
        ended_at: r.get(19)?,
        exec_ms: r.get(20)?,
        wait_ms: r.get(21)?,
        exec_started_at: r.get(22)?,
        wait_started_at: r.get(23)?,
        waits: r.get(24)?,
        input_tokens: r.get(25)?,
        cache_read_tokens: r.get(26)?,
        output_tokens: r.get(27)?,
        result: r.get(28)?,
        files: serde_json::from_str(&files).unwrap_or_default(),
        error: r.get(30)?,
        config: r.get::<_, Option<String>>(31)?.map(|text| serde_json::from_str(&text).map_err(|error| rusqlite::Error::FromSqlConversionFailure(31, rusqlite::types::Type::Text, Box::new(error)))).transpose()?,
    })
}

pub(crate) fn load_task(conn: &Connection, id: i64) -> rusqlite::Result<Option<Task>> {
    conn.query_row(&format!("SELECT {TASK_COLS} FROM automation_tasks WHERE id = ?1"), [id], task_from).optional()
}

pub(crate) fn load_run(conn: &Connection, id: i64) -> rusqlite::Result<Option<Run>> {
    conn.query_row(
        &format!("SELECT {RUN_COLS} FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.id = ?1"),
        [id],
        run_from,
    )
    .optional()
}

/// 任务当前进行中的运行（排队 / 等待 / 执行中 / 需要处理）
pub(crate) fn active_run(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<Run>> {
    conn.query_row(
        &format!("SELECT {RUN_COLS} FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.task_id = ?1 AND r.status IN {ACTIVE} ORDER BY r.created_at DESC, r.id DESC LIMIT 1"),
        [task_id],
        run_from,
    )
    .optional()
}

pub(crate) fn last_run(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<Run>> {
    conn.query_row(
        &format!("SELECT {RUN_COLS} FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.task_id = ?1 ORDER BY r.created_at DESC, r.id DESC LIMIT 1"),
        [task_id],
        run_from,
    )
    .optional()
}

pub(crate) fn add_event(conn: &Connection, run_id: i64, now: i64, kind: &str, title: &str, detail: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO automation_events (run_id, ts, kind, title, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id, now, kind, title, detail],
    )?;
    Ok(())
}

pub(crate) fn events_of(conn: &Connection, run_id: i64, logs: bool) -> rusqlite::Result<Vec<Event>> {
    let sql = if logs {
        "SELECT ts, kind, title, detail FROM (SELECT * FROM automation_events WHERE run_id = ?1 AND kind = 'log' ORDER BY id DESC LIMIT 60) ORDER BY id"
    } else {
        "SELECT ts, kind, title, detail FROM automation_events WHERE run_id = ?1 AND kind != 'log' ORDER BY id"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([run_id], |r| Ok(Event { ts: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, detail: r.get(3)? }))?;
    rows.collect()
}

pub(crate) fn connection_name(conn: &Connection, id: &str) -> Option<String> {
    conn.query_row("SELECT name FROM connections WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional().ok().flatten()
}


/* ─────────────────────────── 校验与新建 ─────────────────────────── */

pub(crate) fn safe_model(model: &str) -> bool {
    !model.is_empty() && model.len() <= 120 && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '[' | ']'))
}

pub fn validate(input: &TaskInput, now: i64) -> Result<(), String> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 { return Err("任务名称应为 1 到 80 个字符".into()); }
    if tool_name(&input.platform).is_none() { return Err("该工具尚未接入执行器".into()); }
    if input.connection_id.trim().is_empty() { return Err("请选择账号后再创建任务。".into()); }
    if let Some(model) = input.model.as_deref().filter(|m| !m.is_empty()) {
        if !safe_model(model) { return Err("模型名称包含不支持的字符".into()); }
    }
    if !Path::new(input.cwd.trim()).is_dir() { return Err("项目目录不存在或不可访问".into()); }
    if let Some(id) = input.session_id.as_deref() {
        if !crate::session_threads::safe_id(id) { return Err("会话 ID 无效".into()); }
    }
    if input.prompt.trim().is_empty() { return Err("请填写执行内容".into()); }
    if input.prompt.chars().count() > 20_000 { return Err("执行内容不能超过 20000 个字符".into()); }
    if input.skills.len() > MAX_SKILLS { return Err(format!("最多选择 {MAX_SKILLS} 个 Skills")); }
    if let Some(bad) = input.skills.iter().find(|s| !crate::skills::valid_name(s)) { return Err(format!("Skill 名称无效：{bad}")); }
    if input.skills.iter().collect::<HashSet<_>>().len() != input.skills.len() { return Err("Skills 不能重复选择".into()); }
    match input.launch.as_str() {
        "now" | "quota" => {}
        "schedule" => {
            let s = input.schedule.as_ref().ok_or("请设置执行时间")?;
            NaiveTime::parse_from_str(&s.time, "%H:%M").map_err(|_| "执行时间格式应为 HH:MM")?;
            match s.freq.as_str() {
                "once" => {
                    if next_fire(&Local, s, now).is_none() {
                        // 区分「已经过去」与「夏令时跳过、本地根本没有这个时刻」，后者提示换时间而不是说过期
                        let skipped = NaiveDate::parse_from_str(s.date.as_deref().unwrap_or_default(), "%Y-%m-%d").ok()
                            .zip(NaiveTime::parse_from_str(&s.time, "%H:%M").ok())
                            .is_some_and(|(date, time)| Local.from_local_datetime(&date.and_time(time)).earliest().is_none());
                        return Err(if skipped { "这个时刻因夏令时调整在本地不存在，请换一个时间".into() } else { "单次执行时间已经过去".into() });
                    }
                }
                "daily" => {}
                "weekly" => {
                    if !s.weekday.is_some_and(|d| (1..=7).contains(&d)) { return Err("请选择每周执行的星期".into()); }
                }
                _ => return Err("不支持的执行频率".into()),
            }
        }
        _ => return Err("不支持的启动方式".into()),
    }
    if input.wait_deadline.is_some_and(|d| d <= now) { return Err("等待截止时间应晚于现在".into()); }
    Ok(())
}

pub(crate) fn insert_run(conn: &Connection, task: &Task, source: &str, status: &str, now: i64) -> rusqlite::Result<i64> {
    let account = connection_name(conn, &task.connection_id);
    let waiting = status == "waiting";
    let snapshot = serde_json::to_string(task).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    conn.execute(
        "INSERT INTO automation_runs (task_id, source, status, block_kind, reason, next_check_at, wait_started_at, session_id, account, model, created_at, task_snapshot)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            task.id, source, status,
            waiting.then_some("quota"),
            waiting.then_some("等待检查额度"),
            waiting.then_some(now),
            waiting.then_some(now),
            task.session_id, account, task.model, now, snapshot
        ],
    )?;
    let id = conn.last_insert_rowid();
    let (kind, title) = match status {
        "waiting" => ("wait", "创建任务，等待额度"),
        _ => ("queue", "进入执行队列"),
    };
    add_event(conn, id, now, kind, title, None)?;
    Ok(id)
}

pub fn create_task(conn: &Connection, input: &TaskInput, now: i64) -> Result<i64, String> {
    validate(input, now)?;
    let next = match (input.launch.as_str(), input.schedule.as_ref()) {
        ("schedule", Some(s)) => next_fire(&Local, s, now),
        _ => None,
    };
    let e = |e: rusqlite::Error| e.to_string();
    conn.execute(
        "INSERT INTO automation_tasks (name, platform, connection_id, model, cwd, session_id, prompt, skills, launch, schedule, auto_resume, wait_deadline, state, next_run_at, created_at, updated_at, serial_queue)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14, ?14, ?15)",
        params![
            input.name.trim(), input.platform, input.connection_id,
            input.model.as_deref().filter(|m| !m.is_empty()),
            input.cwd.trim(), input.session_id,
            input.prompt.trim(),
            serde_json::to_string(&input.skills).unwrap_or_else(|_| "[]".into()),
            input.launch,
            input.schedule.as_ref().and_then(|s| serde_json::to_string(s).ok()),
            input.auto_resume as i64, input.wait_deadline, next, now, input.serial_queue as i64
        ],
    ).map_err(e)?;
    let id = conn.last_insert_rowid();
    let task = load_task(conn, id).map_err(e)?.ok_or("任务保存失败")?;
    match input.launch.as_str() {
        "now" => { insert_run(conn, &task, "now", "queued", now).map_err(e)?; }
        "quota" => { insert_run(conn, &task, "quota", "waiting", now).map_err(e)?; }
        _ => {}
    }
    Ok(id)
}

/// 编辑只影响后续执行：不改动正在进行的运行
pub fn update_task(conn: &Connection, id: i64, input: &TaskInput, now: i64) -> Result<(), String> {
    validate(input, now)?;
    let task = load_task(conn, id).map_err(|e| e.to_string())?.ok_or("任务不存在")?;
    if task.state == "cancelled" { return Err("任务已取消，不能再编辑".into()); }
    let next = match (input.launch.as_str(), input.schedule.as_ref()) {
        ("schedule", Some(s)) if task.state == "active" => next_fire(&Local, s, now),
        _ => None,
    };
    conn.execute(
        "UPDATE automation_tasks SET name=?2, platform=?3, connection_id=?4, model=?5, cwd=?6, session_id=?7, prompt=?8, skills=?9, launch=?10, schedule=?11, auto_resume=?12, wait_deadline=?13, next_run_at=?14, updated_at=?15, serial_queue=?16 WHERE id=?1",
        params![
            id, input.name.trim(), input.platform, input.connection_id,
            input.model.as_deref().filter(|m| !m.is_empty()),
            input.cwd.trim(), input.session_id, input.prompt.trim(),
            serde_json::to_string(&input.skills).unwrap_or_else(|_| "[]".into()),
            input.launch,
            input.schedule.as_ref().and_then(|s| serde_json::to_string(s).ok()),
            input.auto_resume as i64, input.wait_deadline, next, now, input.serial_queue as i64
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/* ─────────────────────────── 列表 ─────────────────────────── */

/* ─────────────────────────── 运行生命周期 ─────────────────────────── */

pub fn finish(conn: &Connection, run_id: i64, outcome: &Outcome, p: &Progress, now: i64) -> rusqlite::Result<()> {
    let Some(run) = load_run(conn, run_id)? else { return Ok(()) };
    let task = load_task(conn, run.task_id)?;
    let exec_add = now - run.exec_started_at.unwrap_or(now);
    let mut input = run.input_tokens;
    let mut cache = run.cache_read_tokens;
    let mut output = run.output_tokens;
    add(&mut input, p.input);
    add(&mut cache, p.cache_read);
    add(&mut output, p.output);
    let mut files = run.files.clone();
    for f in &p.files {
        if !files.contains(f) { files.push(f.clone()); }
    }
    let session = p.session_id.clone().or(run.session_id.clone());
    conn.execute(
        "UPDATE automation_runs SET exec_ms=exec_ms+?2, exec_started_at=NULL, input_tokens=?3, cache_read_tokens=?4, output_tokens=?5, files=?6, session_id=?7, result=COALESCE(?8, result) WHERE id=?1",
        params![run_id, exec_add, input, cache, output, serde_json::to_string(&files).unwrap_or_else(|_| "[]".into()), session, p.result.as_deref().map(|r| clip(r, 4000))],
    )?;
    let auto_resume = run.config.as_ref().is_some_and(|t| t.auto_resume);
    // 任务已不是启用状态（执行中被取消）：会让运行继续挂着的结果（等待额度、需要处理）一律按已取消收尾，
    // 不留下等待 / 排队的孤儿运行；完成与失败是真实结果，照常记录
    let inactive = task.as_ref().is_none_or(|t| t.state != "active");
    let lingering = match outcome {
        Outcome::Limit { .. } | Outcome::RateLimited { .. } => auto_resume,
        Outcome::Balance(_) => true,
        Outcome::Reauth(_) | Outcome::UntrustedDir(_) => true,
        _ => false,
    };
    if inactive && lingering {
        conn.execute("UPDATE automation_runs SET status='cancelled', ended_at=?2, resume_pending=0, block_kind=NULL, reason=NULL WHERE id=?1", params![run_id, now])?;
        add_event(conn, run_id, now, "stop", "任务已取消，运行结束", Some("任务不再启用，不进入等待或需要处理"))?;
        return Ok(());
    }
    match outcome {
        Outcome::Completed => {
            conn.execute("UPDATE automation_runs SET status='completed', ended_at=?2, resume_pending=0, block_kind=NULL, reason=NULL WHERE id=?1", params![run_id, now])?;
            add_event(conn, run_id, now, "done", "完成任务", Some(if session.is_some() { "结果已关联到原会话" } else { "执行器已返回结果" }))?;
            if let Some(t) = &task {
                let one_shot = t.launch != "schedule" || t.schedule.as_ref().is_some_and(|s| s.freq == "once");
                if one_shot && t.state == "active" {
                    conn.execute("UPDATE automation_tasks SET state='finished', updated_at=?2 WHERE id=?1", params![t.id, now])?;
                }
            }
        }
        Outcome::Cancelled => {
            conn.execute("UPDATE automation_runs SET status='cancelled', ended_at=?2 WHERE id=?1", params![run_id, now])?;
            add_event(conn, run_id, now, "stop", "已停止运行", Some("保留运行记录和已经完成的文件修改"))?;
        }
        Outcome::Limit { resets_at, message } if auto_resume && session.is_some() => {
            let next = resets_at.map(|r| r.max(now) + 30_000).unwrap_or(now + backoff(1));
            conn.execute(
                "UPDATE automation_runs SET status='waiting', block_kind='quota', reason='额度耗尽，保留进度', resets_at=?2, next_check_at=?3, check_failures=0, wait_started_at=?4, waits=waits+1, resume_pending=1 WHERE id=?1",
                params![run_id, resets_at, next, now],
            )?;
            add_event(conn, run_id, now, "pause", "额度耗尽，保留进度", Some(&clip(message, 200)))?;
        }
        Outcome::Limit { message, .. } if auto_resume => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='resume_unsupported', reason='无法自动续跑 · 需要手动继续：执行器没有返回可续跑的会话' WHERE id=?1",
                params![run_id],
            )?;
            add_event(conn, run_id, now, "attention", "额度耗尽，无法自动续跑", Some(&clip(message, 200)))?;
        }
        Outcome::Limit { message, .. } => {
            conn.execute("UPDATE automation_runs SET status='failed', ended_at=?2, error=?3 WHERE id=?1", params![run_id, now, format!("额度耗尽：{}", clip(message, 1000))])?;
            add_event(conn, run_id, now, "fail", "额度耗尽，运行结束", Some("未开启「等待恢复后自动继续」"))?;
        }
        Outcome::RateLimited { retry_at, message } if auto_resume && session.is_some() => {
            // 不是额度用完：到点后照常检查一次额度，可用就续跑（apply_check 的 available 分支）
            let at = retry_at.filter(|at| *at > now).unwrap_or(now + RATE_LIMIT_RETRY_MS);
            conn.execute(
                "UPDATE automation_runs SET status='waiting', block_kind='rate_limit', reason='短时限流，稍后重试', resets_at=?2, next_check_at=?3, check_failures=0, wait_started_at=?4, waits=waits+1, resume_pending=1 WHERE id=?1",
                params![run_id, at, at + 5_000, now],
            )?;
            add_event(conn, run_id, now, "pause", "短时限流，稍后重试", Some(&clip(message, 200)))?;
        }
        Outcome::RateLimited { message, .. } if auto_resume => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='resume_unsupported', reason='无法自动续跑 · 需要手动继续：执行器没有返回可续跑的会话' WHERE id=?1",
                params![run_id],
            )?;
            add_event(conn, run_id, now, "attention", "短时限流，无法自动续跑", Some(&clip(message, 200)))?;
        }
        Outcome::RateLimited { message, .. } => {
            conn.execute("UPDATE automation_runs SET status='failed', ended_at=?2, error=?3 WHERE id=?1", params![run_id, now, format!("短时限流：{}", clip(message, 1000))])?;
            add_event(conn, run_id, now, "fail", "短时限流，运行结束", Some("未开启「等待恢复后自动继续」"))?;
        }
        Outcome::Balance(message) => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='balance', reason='余额不足：充值后手动继续；等待额度重置不会恢复', error=?2 WHERE id=?1",
                params![run_id, clip(message, 1000)],
            )?;
            add_event(conn, run_id, now, "attention", "余额不足", Some(&clip(message, 200)))?;
        }
        Outcome::Reauth(message) => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='reauth', reason='需要重新登录：停止额度检查，保留任务；重新认证并检查账号后才能继续。', error=?2 WHERE id=?1",
                params![run_id, clip(message, 1000)],
            )?;
            add_event(conn, run_id, now, "attention", "需要重新登录", Some(&clip(message, 200)))?;
        }
        Outcome::UntrustedDir(message) => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='cwd_missing', reason=?2, error=?3 WHERE id=?1",
                params![run_id, UNTRUSTED_DIR_REASON, clip(message, 1000)],
            )?;
            add_event(conn, run_id, now, "attention", "需要处理", Some(UNTRUSTED_DIR_REASON))?;
        }
        Outcome::Failed(message) => {
            conn.execute("UPDATE automation_runs SET status='failed', ended_at=?2, error=?3 WHERE id=?1", params![run_id, now, clip(message, 2000)])?;
            add_event(conn, run_id, now, "fail", "运行失败", Some(&clip(message, 200)))?;
        }
    }
    Ok(())
}

// 任务 2.5 开始
/// 运行当前状态对应的通知（D-10）：完成；失败、需要处理（含等待超时）归入失败；已取消、排队、等待、执行中不通知
pub fn notice_for(run: &Run) -> Option<TaskNotice> {
    let failed = |text: Option<&String>, fallback: &str| TaskNotice::Failed {
        reason: text.map(|t| t.trim()).filter(|t| !t.is_empty()).unwrap_or(fallback).to_string(),
    };
    match run.status.as_str() {
        "completed" => Some(TaskNotice::Completed),
        "failed" => Some(failed(run.error.as_ref(), "运行失败")),
        "attention" => Some(failed(run.reason.as_ref(), "需要处理")),
        _ => None,
    }
}

/// 一条待发的任务通知：任务 ID、通知种类、工具名、任务名
pub struct PendingNotice {
    pub(crate) task_id: i64,
    pub(crate) kind: TaskNotice,
    pub(crate) platform: &'static str,
    pub(crate) task_name: String,
}

/// 在刚改完状态的同一把锁里读出这些运行的当前状态，算出该发的通知，锁外再发；
/// 避免锁外重读时运行已被调度线程推进到下一个状态而重复提醒。调用方只传刚经历状态变化的运行
pub fn notices_of(conn: &Connection, ids: &[i64]) -> rusqlite::Result<Vec<PendingNotice>> {
    let mut out = Vec::new();
    for id in ids {
        let Some(run) = load_run(conn, *id)? else { continue };
        if let Some(kind) = notice_for(&run) {
            out.push(PendingNotice { task_id: run.task_id, kind, platform: display_name(&run.platform), task_name: run.task_name });
        }
    }
    Ok(out)
}

pub(crate) fn send_notices(app: &AppHandle, notices: Vec<PendingNotice>) {
    for n in notices {
        crate::notify::notify_task(app, n.task_id, n.kind, n.platform, &n.task_name);
    }
}

/// 续跑前取走「额度已确认」标记（只用一次）。
/// 只有 `apply_check` 确认额度可用（或不支持读额度、到达预计恢复时间）转回排队时才置位；
/// 转为需要处理、手动重新排队都会清掉，所以手动重新排队后的续跑不算「额度已恢复」。
/// 用运行表列而不是最近一条事件判断：事件只是展示用的日志，以后新增事件不会悄悄改变通知行为
pub fn take_resume_confirmed(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("UPDATE automation_runs SET resume_confirmed=0 WHERE id=?1 AND resume_confirmed=1", [run_id])?;
    Ok(n == 1)
}

/// 有任务在等额度恢复的账号（4.2：灵动岛「等额度」表情的数据源）。
/// 只算未暂停任务的等待运行——暂停的任务不会被额度恢复唤醒，不该让岛一直显示在等。
pub fn waiting_connections(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT json_extract(r.task_snapshot, '$.connection_id') FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND t.state != 'paused' AND json_extract(r.task_snapshot, '$.connection_id') IS NOT NULL",
    )?;
    let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

/// 该账号下等待额度的运行：额度检查前记下，检查后转为「需要处理」的发失败通知
pub(crate) fn waiting_runs(conn: &Connection, connection_id: &str) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT r.id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.status = 'waiting' AND json_extract(r.task_snapshot, '$.connection_id') = ?1",
    )?;
    let ids = stmt.query_map([connection_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}
// 任务 2.5 结束

/// 只改仍在启动流程里的运行（排队中，或刚认领为执行中）；启动途中被删除或取消的运行不再写入
pub(crate) fn set_attention(conn: &Connection, run_id: i64, kind: &str, reason: &str, now: i64) -> rusqlite::Result<()> {
    let n = conn.execute(
        "UPDATE automation_runs SET status='attention', block_kind=?2, reason=?3, resume_confirmed=0 WHERE id=?1 AND status IN ('queued','running')",
        params![run_id, kind, reason],
    )?;
    if n == 0 { return Ok(()); }
    add_event(conn, run_id, now, "attention", "需要处理", Some(reason))
}

/// D-09：每个账号同时运行的任务上限（含排队任务与并行任务）
pub(crate) const MAX_PARALLEL_PER_ACCOUNT: usize = 3;

/// 工作目录比较键：与会话模块一致，小写、`\` 换成 `/`、去掉末尾 `/`
pub(crate) fn cwd_key(path: &str) -> String {
    path.trim().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// 按账号决定本轮可以启动的运行（D-09）。排队与等待的运行只看启用中的任务（与 [`claim_run`] 一致，暂停、取消、
/// 完成的任务不占队列，也不会被反复放行又认领失败）；执行中的运行无论任务状态都计入工作目录与并行数：
/// ③ 账号下任一运行在等待额度时，整个账号不再启动新任务（正在运行的跑完当前这一轮）；
/// 排队任务（serial）一次一个，先放行排队队首，再按顺序放行关闭排队的任务；
/// ① 与任何账号正在运行（或本轮已放行）的任务工作目录相同则不放行；② 同账号运行数不超过上限
pub fn startable(conn: &Connection) -> rusqlite::Result<Vec<i64>> {
    struct Row { id: i64, status: String, cwd: String, serial: bool }
    let mut stmt = conn.prepare(
        "SELECT r.id, r.status, json_extract(r.task_snapshot, '$.connection_id'), json_extract(r.task_snapshot, '$.cwd'), json_extract(r.task_snapshot, '$.serial_queue') FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.task_snapshot IS NOT NULL AND r.status IN ('queued','waiting','running') AND (t.state = 'active' OR r.status = 'running')
         ORDER BY r.created_at, r.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(2)?, Row { id: r.get(0)?, status: r.get(1)?, cwd: cwd_key(&r.get::<_, String>(3)?), serial: r.get::<_, i64>(4)? != 0 }))
    })?;
    let mut accounts: Vec<(String, Vec<Row>)> = Vec::new();
    for row in rows {
        let (connection, row) = row?;
        match accounts.iter_mut().find(|(c, _)| *c == connection) {
            Some((_, list)) => list.push(row),
            None => accounts.push((connection, vec![row])),
        }
    }
    let mut out = Vec::new();
    // ① 同一工作目录跨账号也串行：先放入所有正在运行的目录，本轮放行的也加入
    let mut busy: HashSet<&str> = accounts.iter().flat_map(|(_, rows)| rows.iter()).filter(|r| r.status == "running").map(|r| r.cwd.as_str()).collect();
    for (_, rows) in &accounts {
        if rows.iter().any(|r| r.status == "waiting") { continue; }
        let running: Vec<&Row> = rows.iter().filter(|r| r.status == "running").collect();
        let mut count = running.len();
        let serial_busy = running.iter().any(|r| r.serial);
        let head = rows.iter().find(|r| r.status == "queued" && r.serial).filter(|_| !serial_busy);
        let parallel = rows.iter().filter(|r| r.status == "queued" && !r.serial);
        for r in head.into_iter().chain(parallel) {
            if count >= MAX_PARALLEL_PER_ACCOUNT { break; }
            if !busy.insert(r.cwd.as_str()) { continue; }
            out.push(r.id);
            count += 1;
        }
    }
    Ok(out)
}

/// 到点的定时任务入队；仍在执行时跳过本次触发；发现太晚的触发标「已错过」
pub fn fire_schedules<Tz: TimeZone>(conn: &Connection, tz: &Tz, now: i64) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLS} FROM automation_tasks WHERE state = 'active' AND launch = 'schedule' AND next_run_at IS NOT NULL AND next_run_at <= ?1"))?;
    let due: Vec<Task> = stmt.query_map([now], task_from)?.collect::<Result<_, _>>()?;
    let mut changed = false;
    for task in due {
        let Some(at) = task.next_run_at else { continue };
        let next = task.schedule.as_ref().filter(|s| s.freq != "once").and_then(|s| next_fire(tz, s, now));
        if now - at > MISSED_GRACE_MS {
            conn.execute("UPDATE automation_tasks SET missed_at=?2, next_run_at=?3, updated_at=?4 WHERE id=?1", params![task.id, at, next, now])?;
        } else {
            conn.execute("UPDATE automation_tasks SET next_run_at=?2, updated_at=?3 WHERE id=?1", params![task.id, next, now])?;
            if active_run(conn, task.id)?.is_none() {
                insert_run(conn, &task, "schedule", "queued", now)?;
            }
        }
        changed = true;
    }
    Ok(changed)
}

/// 等待截止已过的运行转为「需要处理」，返回有运行到期需要检查额度的账号（D-09：同账号合并成一次检查）
/// 与这次超时的运行（任务 2.5：调度线程据此发失败通知）
pub fn due_checks(conn: &Connection, now: i64) -> rusqlite::Result<(Vec<String>, Vec<i64>)> {
    let mut stmt = conn.prepare(
        "SELECT r.id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND json_extract(r.task_snapshot, '$.wait_deadline') IS NOT NULL AND json_extract(r.task_snapshot, '$.wait_deadline') < ?1",
    )?;
    let expired: Vec<i64> = stmt.query_map([now], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in &expired {
        conn.execute(
            "UPDATE automation_runs SET status='attention', block_kind='timeout', reason='等待超时：到达截止时间仍不可用，已保留记录，可重新排队', wait_ms=wait_ms+(?2-COALESCE(wait_started_at, ?2)), wait_started_at=NULL WHERE id=?1",
            params![id, now],
        )?;
        add_event(conn, *id, now, "attention", "等待超时", None)?;
    }
    let mut stmt = conn.prepare(
        "SELECT json_extract(r.task_snapshot, '$.connection_id') FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.task_snapshot IS NOT NULL AND r.status = 'waiting' AND t.state = 'active' AND (r.next_check_at IS NULL OR r.next_check_at <= ?1)
         GROUP BY json_extract(r.task_snapshot, '$.connection_id') ORDER BY MIN(r.created_at), MIN(r.id)",
    )?;
    let due = stmt.query_map([now], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok((due, expired))
}

/// D-09 ③：额度检查按账号合并，一次检查的结果写入该账号全部等待中的运行（暂停的任务除外）；
/// `extra` 是手动检查的那条运行，即使所属任务已暂停也照常写入。
/// `checked_from` 是这次额度查询开始的时间：查询期间才进入等待的运行不使用这次（可能已过时的）结果。
/// 返回是否有运行转入队列
pub fn apply_account_check(conn: &Connection, connection_id: &str, extra: Option<i64>, checked_from: i64, v: &QuotaVerdict, now: i64) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT r.id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND json_extract(r.task_snapshot, '$.connection_id') = ?1 AND (t.state != 'paused' OR r.id = ?2)
           AND COALESCE(r.wait_started_at, 0) <= ?3
         ORDER BY r.created_at, r.id",
    )?;
    let ids: Vec<i64> = stmt.query_map(params![connection_id, extra.unwrap_or(-1), checked_from], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut queued = false;
    for id in ids { queued |= apply_check(conn, id, v, now)?; }
    Ok(queued)
}

/// 启动时：上次退出时还在执行的运行标为需要处理（不自动重跑）；等待中的立即重新检查额度
pub fn recover(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    let mut missing = conn.prepare("SELECT id, status, block_kind, reason FROM automation_runs WHERE task_snapshot IS NULL AND status IN ('queued','waiting','running','attention') AND COALESCE(block_kind, '') != 'snapshot_missing'")?;
    let unknown: Vec<(i64, String, Option<String>, Option<String>)> = missing.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    for (id, status, kind, reason) in unknown {
        conn.execute("UPDATE automation_runs SET status='attention', block_kind='snapshot_missing', reason=?2, next_check_at=NULL, exec_started_at=NULL WHERE id=?1", params![id, MISSING_SNAPSHOT])?;
        add_event(conn, id, now, "attention", "原始执行配置未知", Some(&format!("迁移前状态：{status}；原因类型：{}；说明：{}。{MISSING_SNAPSHOT}", kind.as_deref().unwrap_or("无"), reason.as_deref().unwrap_or("无"))))?;
    }
    let mut stmt = conn.prepare("SELECT id FROM automation_runs WHERE status = 'running'")?;
    let ids: Vec<i64> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in ids {
        conn.execute(
            "UPDATE automation_runs SET status='attention', block_kind='interrupted', reason='Jolu 退出时运行被中断；已保留进度，不会自动从头重跑，可查看关联会话后手动继续', exec_started_at=NULL WHERE id=?1",
            [id],
        )?;
        add_event(conn, id, now, "attention", "运行被中断", Some("Jolu 退出或重启"))?;
    }
    conn.execute("UPDATE automation_runs SET next_check_at=?1 WHERE status='waiting'", [now])?;
    Ok(())
}

/// 任务操作；返回需要结束的进程所属运行
pub fn act(conn: &Connection, id: i64, action: &str, now: i64) -> Result<Option<i64>, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let task = load_task(conn, id).map_err(e)?.ok_or("任务不存在")?;
    let active = active_run(conn, id).map_err(e)?;
    let running = active.as_ref().filter(|r| r.status == "running").map(|r| r.id);
    match action {
        "pause" => {
            if running.is_some() { return Err("运行中的任务请先停止运行".into()); }
            if task.state != "active" { return Err("只有启用中的任务可以暂停".into()); }
            conn.execute("UPDATE automation_tasks SET state='paused', updated_at=?2 WHERE id=?1", params![id, now]).map_err(e)?;
            if let Some(r) = &active { add_event(conn, r.id, now, "pause", "暂停等待", Some("暂停期间不自动运行")).map_err(e)?; }
            Ok(None)
        }
        "resume" => {
            if task.state != "paused" { return Err("任务没有暂停".into()); }
            let next = match (task.launch.as_str(), task.schedule.as_ref()) {
                ("schedule", Some(s)) => next_fire(&Local, s, now),
                _ => None,
            };
            conn.execute("UPDATE automation_tasks SET state='active', next_run_at=?2, updated_at=?3 WHERE id=?1", params![id, next, now]).map_err(e)?;
            if let Some(r) = &active {
                conn.execute("UPDATE automation_runs SET next_check_at=?2 WHERE id=?1 AND status='waiting'", params![r.id, now]).map_err(e)?;
                add_event(conn, r.id, now, "resume", "恢复任务", None).map_err(e)?;
            }
            Ok(None)
        }
        "cancel" => {
            if task.state == "cancelled" { return Ok(None); }
            conn.execute("UPDATE automation_tasks SET state='cancelled', next_run_at=NULL, updated_at=?2 WHERE id=?1", params![id, now]).map_err(e)?;
            if let Some(r) = &active {
                if r.status != "running" {
                    conn.execute(
                        "UPDATE automation_runs SET status='cancelled', ended_at=?2, wait_ms=wait_ms+(?2-COALESCE(wait_started_at, ?2)), wait_started_at=NULL WHERE id=?1",
                        params![r.id, now],
                    ).map_err(e)?;
                    add_event(conn, r.id, now, "stop", "取消任务", Some("停止后续等待与调度")).map_err(e)?;
                }
            }
            Ok(running)
        }
        "stop" => running.map(Some).ok_or_else(|| "没有执行中的运行".to_string()),
        "run" | "retry" => {
            if task.state == "cancelled" { return Err("任务已取消".into()); }
            if task.state == "paused" { return Err("请先恢复任务".into()); }
            match &active {
                Some(r) if r.status == "attention" => {
                    if r.config.is_none() { return Err(MISSING_SNAPSHOT.into()); }
                    // 手动重新排队没有确认过额度：续跑时不发「任务已继续」（任务 2.5）
                    conn.execute("UPDATE automation_runs SET status='queued', block_kind=NULL, reason=NULL, resume_confirmed=0 WHERE id=?1", [r.id]).map_err(e)?;
                    add_event(conn, r.id, now, "queue", "重新排队", None).map_err(e)?;
                }
                Some(_) => return Err("任务已有进行中的运行".into()),
                None => {
                    insert_run(conn, &task, "manual", "queued", now).map_err(e)?;
                }
            }
            conn.execute(
                "UPDATE automation_tasks SET missed_at=NULL, state=CASE state WHEN 'finished' THEN 'active' ELSE state END, updated_at=?2 WHERE id=?1",
                params![id, now],
            ).map_err(e)?;
            Ok(None)
        }
        _ => Err("不支持的操作".into()),
    }
}

/// 删除任务（任务 2.9）：在一个事务里删除任务、它的全部运行记录和事件；用量统计（`requests` 等表）不动。
/// 有执行中的运行时拒绝；排队或等待额度的运行先转为已取消再删除。调度线程只认 `status='queued'` 的运行
/// （见 [`claim_run`]），`startable`、`due_checks` 与合并额度检查都按库里现存的运行查询，删除后自然不再出现。
pub fn delete_task(conn: &mut Connection, task_id: i64) -> Result<(), String> {
    let e = |e: rusqlite::Error| e.to_string();
    let tx = conn.transaction().map_err(e)?;
    if load_task(&tx, task_id).map_err(e)?.is_none() { return Err("任务不存在".into()); }
    let running: bool = tx
        .query_row("SELECT EXISTS(SELECT 1 FROM automation_runs WHERE task_id = ?1 AND status = 'running')", [task_id], |r| r.get(0))
        .map_err(e)?;
    if running { return Err("任务正在运行，请先停止".into()); }
    tx.execute("UPDATE automation_runs SET status='cancelled', next_check_at=NULL WHERE task_id = ?1 AND status IN ('queued','waiting')", [task_id]).map_err(e)?;
    tx.execute("DELETE FROM automation_events WHERE run_id IN (SELECT id FROM automation_runs WHERE task_id = ?1)", [task_id]).map_err(e)?;
    tx.execute("DELETE FROM automation_runs WHERE task_id = ?1", [task_id]).map_err(e)?;
    tx.execute("DELETE FROM automation_tasks WHERE id = ?1", [task_id]).map_err(e)?;
    tx.commit().map_err(e)
}

/// 启动前认领运行：只有仍在排队、且所属任务处于启用状态（未暂停、未取消）的运行才转为执行中。
/// 认领发生在拉起进程之前，运行在这之间被删除、取消或任务被暂停时认领失败，不会启动执行器
pub fn claim_run(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE automation_runs SET status='running', block_kind=NULL, reason=NULL
         WHERE id=?1 AND status='queued' AND task_snapshot IS NOT NULL
           AND EXISTS(SELECT 1 FROM automation_tasks t WHERE t.id = automation_runs.task_id AND t.state = 'active')",
        [run_id],
    )?;
    Ok(n == 1)
}
