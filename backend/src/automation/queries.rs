use chrono::{Local, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{
    clip, display_name,
    scheduler::{current_connection_ids, with_conn},
    store::{active_run, events_of, last_run, load_run, load_task, run_from, safe_model, task_from, Event, Run, Task, RUN_COLS, TASK_COLS},
    tool_name, Shared, PAGE,
};


#[derive(Debug, Serialize)]
pub struct TaskItem {
    #[serde(flatten)]
    pub task: Task,
    pub active: Option<Run>,
    pub last: Option<Run>,
    /// 排队中时，同账号前方的任务数
    pub queue_ahead: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub tasks: Vec<TaskItem>,
    pub enabled: i64,
    pub running: i64,
    /// 第一个执行中的运行：工具 · 账号
    pub running_note: Option<String>,
    pub waiting: i64,
    /// 等待中最早的预计恢复时间
    pub next_reset: Option<i64>,
    pub done_today: i64,
    pub failed_today: i64,
}

pub(crate) fn day_start(now: i64) -> i64 {
    Local.timestamp_millis_opt(now).single()
        .and_then(|d| Local.from_local_datetime(&d.date_naive().and_hms_opt(0, 0, 0)?).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now)
}

/// 同账号排在前面的排队任务数；关闭排队的任务不挡排队任务，不计入（D-09）。口径同 [`startable`]：排队与等待只看启用中的任务
pub(crate) fn queue_ahead(conn: &Connection, run: &Run) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE json_extract(r.task_snapshot, '$.connection_id') = ?1 AND (t.state = 'active' OR r.status = 'running') AND json_extract(r.task_snapshot, '$.serial_queue') = 1 AND r.id != ?2
           AND r.status IN ('queued','waiting','running')
           AND (r.status = 'running' OR r.created_at < ?3 OR (r.created_at = ?3 AND r.id < ?2))",
        params![run.connection_id, run.id, run.created_at],
        |r| r.get(0),
    )
}

pub(crate) fn task_item(conn: &Connection, task: Task) -> rusqlite::Result<TaskItem> {
    let active = active_run(conn, task.id)?;
    let last = last_run(conn, task.id)?;
    // 关闭排队的任务没有队列位置（只受工作目录、并行上限与额度等待限制），不给出前方任务数；显示方式待确认
    let queue_ahead = match &active {
        Some(run) if run.status == "queued" && run.config.as_ref().is_some_and(|c| c.serial_queue) => Some(queue_ahead(conn, run)?),
        _ => None,
    };
    Ok(TaskItem { task, active, last, queue_ahead })
}

pub fn overview(conn: &Connection, now: i64) -> rusqlite::Result<Overview> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLS} FROM automation_tasks ORDER BY updated_at DESC, id DESC"))?;
    let tasks: Vec<Task> = stmt.query_map([], task_from)?.collect::<Result<_, _>>()?;
    let tasks: Vec<TaskItem> = tasks.into_iter().map(|t| task_item(conn, t)).collect::<Result<_, _>>()?;
    let status_of = |item: &TaskItem| item.active.as_ref().map(|r| r.status.clone());
    let enabled = tasks.iter().filter(|i| i.task.state == "active" && (i.task.launch == "schedule" || i.active.is_some())).count() as i64;
    let running: Vec<&TaskItem> = tasks.iter().filter(|i| status_of(i).as_deref() == Some("running")).collect();
    let waiting: Vec<&TaskItem> = tasks.iter().filter(|i| status_of(i).as_deref() == Some("waiting")).collect();
    let running_note = running.first().and_then(|i| i.active.as_ref()).map(|run| {
        match &run.account {
            Some(a) => format!("{} · {a}", display_name(&run.platform)),
            None => display_name(&run.platform).to_string(),
        }
    });
    let next_reset = waiting.iter().filter_map(|i| i.active.as_ref().and_then(|r| r.resets_at)).filter(|t| *t > now).min();
    let start = day_start(now);
    let count = |status: &str| -> rusqlite::Result<i64> {
        conn.query_row("SELECT COUNT(*) FROM automation_runs WHERE status = ?1 AND ended_at >= ?2", params![status, start], |r| r.get(0))
    };
    Ok(Overview {
        enabled,
        running: running.len() as i64,
        running_note,
        waiting: waiting.len() as i64,
        next_reset,
        done_today: count("completed")?,
        failed_today: count("failed")?,
        tasks,
    })
}

#[derive(Debug, Serialize)]
pub struct QueueItem {
    pub task_id: i64,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct TaskDetail {
    pub item: TaskItem,
    /// 当前活动运行（或最近运行）的原始配置；还没有运行时为任务配置，旧运行缺失快照时为 None。
    pub execution: Option<Task>,
    /// 同账号的队列（执行中 / 等待额度 / 排队中），按先后顺序；只列开启排队的任务（D-09）
    pub queue: Vec<QueueItem>,
    /// 当前（或最近一次）运行的时间线
    pub events: Vec<Event>,
}

pub fn task_detail(conn: &Connection, id: i64) -> Result<TaskDetail, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let task = load_task(conn, id).map_err(e)?.ok_or("任务不存在")?;
    let item = task_item(conn, task).map_err(e)?;
    let mut stmt = conn.prepare(
        "SELECT t.id, COALESCE(json_extract(r.task_snapshot, '$.name'), '未知任务'), r.status FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE json_extract(r.task_snapshot, '$.connection_id') = ?1 AND (t.state = 'active' OR r.status = 'running') AND json_extract(r.task_snapshot, '$.serial_queue') = 1 AND r.status IN ('queued','waiting','running')
         ORDER BY CASE r.status WHEN 'running' THEN 0 ELSE 1 END, r.created_at, r.id",
    ).map_err(e)?;
    let queue = stmt.query_map([item.active.as_ref().map(|r| r.connection_id.as_str()).unwrap_or(&item.task.connection_id)], |r| Ok(QueueItem { task_id: r.get(0)?, name: r.get(1)?, status: r.get(2)? }))
        .map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
    let run_id = item.active.as_ref().or(item.last.as_ref()).map(|r| r.id);
    let events = match run_id { Some(id) => events_of(conn, id, false).map_err(e)?, None => vec![] };
    let execution = match item.active.as_ref().or(item.last.as_ref()) {
        Some(run) => run.config.clone(),
        None => Some(item.task.clone()),
    };
    Ok(TaskDetail { item, execution, queue, events })
}

#[derive(Debug, Deserialize)]
pub struct RunFilter {
    /// today / 7d / 30d / all
    pub range: String,
    pub status: Option<String>,
    pub connection: Option<String>,
    pub task: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunPage {
    pub rows: Vec<Run>,
    pub total: i64,
    pub page: u32,
    pub pages: u32,
    pub completed: i64,
    /// 完成的运行里经历过额度等待的次数
    pub completed_waited: i64,
    pub running: i64,
    pub failed: i64,
    pub failed_note: Option<String>,
    /// 范围内已知 Token 合计；全部未知时为 None
    pub tokens: Option<i64>,
    pub tasks: Vec<QueueItem>,
}

pub(crate) fn range_start(range: &str, now: i64) -> Option<i64> {
    let start = day_start(now);
    match range {
        "today" => Some(start),
        "7d" => Some(start - 6 * 86_400_000),
        "30d" => Some(start - 29 * 86_400_000),
        _ => None,
    }
}

pub fn runs(conn: &Connection, f: &RunFilter, page: u32, now: i64) -> rusqlite::Result<RunPage> {
    let since = range_start(&f.range, now).unwrap_or(i64::MIN);
    let base = "FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.created_at >= ?1 AND (?2 IS NULL OR json_extract(r.task_snapshot, '$.connection_id') = ?2) AND (?3 IS NULL OR r.task_id = ?3)";
    let status_clause = "AND (?4 IS NULL OR r.status = ?4)";
    let p = params![since, f.connection, f.task, f.status];
    let total: i64 = conn.query_row(&format!("SELECT COUNT(*) {base} {status_clause}"), p, |r| r.get(0))?;
    let pages = ((total + PAGE - 1) / PAGE).max(1) as u32;
    let page = page.clamp(1, pages);
    let mut stmt = conn.prepare(&format!("SELECT {RUN_COLS} {base} {status_clause} ORDER BY r.created_at DESC, r.id DESC LIMIT {PAGE} OFFSET ?5"))?;
    let rows = stmt.query_map(params![since, f.connection, f.task, f.status, (page as i64 - 1) * PAGE], run_from)?.collect::<Result<_, _>>()?;
    let q = params![since, f.connection, f.task];
    let (completed, completed_waited, running, failed, known, tokens): (i64, i64, i64, i64, i64, i64) = conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(r.status = 'completed'), 0), COALESCE(SUM(r.status = 'completed' AND r.waits > 0), 0),
                    COALESCE(SUM(r.status = 'running'), 0), COALESCE(SUM(r.status = 'failed'), 0),
                    COALESCE(SUM(r.input_tokens IS NOT NULL OR r.output_tokens IS NOT NULL), 0),
                    COALESCE(SUM(COALESCE(r.input_tokens, 0) + COALESCE(r.output_tokens, 0)), 0) {base}"
        ),
        q,
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    )?;
    let failed_note = conn.query_row(
        &format!("SELECT COALESCE(r.error, r.reason) {base} AND r.status = 'failed' ORDER BY r.created_at DESC LIMIT 1"),
        q,
        |r| r.get::<_, Option<String>>(0),
    ).optional()?.flatten().map(|s| clip(&s, 40));
    let mut stmt = conn.prepare("SELECT id, name, state FROM automation_tasks ORDER BY name")?;
    let tasks = stmt.query_map([], |r| Ok(QueueItem { task_id: r.get(0)?, name: r.get(1)?, status: r.get(2)? }))?.collect::<Result<_, _>>()?;
    Ok(RunPage { rows, total, page, pages, completed, completed_waited, running, failed, failed_note, tokens: (known > 0).then_some(tokens), tasks })
}

#[derive(Debug, Serialize)]
pub struct RunDetail {
    pub run: Run,
    pub task: Option<Task>,
    pub events: Vec<Event>,
    pub logs: Vec<Event>,
}

pub fn run_detail(conn: &Connection, id: i64) -> Result<RunDetail, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let run = load_run(conn, id).map_err(e)?.ok_or("运行记录不存在")?;
    let task = run.config.clone();
    Ok(RunDetail { events: events_of(conn, id, false).map_err(e)?, logs: events_of(conn, id, true).map_err(e)?, run, task })
}

#[derive(Debug, Serialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub current: bool,
}

pub(crate) fn accounts_for(db: &Shared, platform: &str) -> Result<Vec<Account>, String> {
    let current = current_connection_ids(db);
    with_conn(db, |conn| {
        let list = crate::connections::list(conn).map_err(|e| e.to_string())?;
        Ok(list
            .into_iter()
            .filter(|c| c.platform == platform)
            .map(|c| Account {
                current: current.contains(&c.id),
                id: c.id,
                name: c.name,
                kind: c.kind,
                status: c.status,
            })
            .collect())
    })
}

pub(crate) fn models_for(
    conn: &Connection,
    platform: &str,
    connection_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(id) = connection_id {
        let model: Option<String> = conn
            .query_row("SELECT model FROM connections WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        if let Some(m) = model.filter(|m| safe_model(m)) {
            out.push(m);
        }
    }
    let mut stmt = conn
        .prepare("SELECT model FROM requests WHERE platform = ?1 AND model IS NOT NULL AND model != '' GROUP BY model ORDER BY MAX(ts) DESC LIMIT 12")
        .map_err(|e| e.to_string())?;
    let seen: Vec<String> = stmt
        .query_map([platform], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for m in seen {
        if safe_model(&m) && !out.contains(&m) {
            out.push(m);
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub platform: String,
    pub found: bool,
}

pub(crate) fn tools_list() -> Vec<ToolInfo> {
    ["claude", "codex"]
        .iter()
        .map(|p| ToolInfo {
            platform: p.to_string(),
            found: tool_name(p)
                .and_then(crate::session_threads::find_executable)
                .is_some(),
        })
        .collect()
}
