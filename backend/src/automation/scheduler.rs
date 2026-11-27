use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use chrono::Local;
use rusqlite::{params, Connection};
use tauri::{AppHandle, Emitter, Manager};

use crate::notify::TaskNotice;
use super::{
    display_name, now_ms,
    queries::{overview, run_detail, runs, task_detail, Overview, RunDetail, RunFilter, RunPage, TaskDetail},
    runner::{build_args, classify, kill_tree, run_prompt, skills_ready, Procs, Progress},
    schedule::{verdict, QuotaVerdict},
    schema::ensure_schema,
    store::{
        act, active_run, add_event, apply_account_check, claim_run, create_task, delete_task,
        due_checks, finish, fire_schedules, load_run, notices_of, recover, send_notices,
        set_attention, startable, take_resume_confirmed, update_task, waiting_connections,
        waiting_runs, TaskInput,
    },
    tool_name, Shared, CHANGED, LOG_LIMIT, MISSING_SNAPSHOT, TICK,
};

static TICK_HOOK: OnceLock<fn(&AppHandle)> = OnceLock::new();

pub(crate) fn register_tick_hook(hook: fn(&AppHandle)) {
    let _ = TICK_HOOK.set(hook);
}

pub(crate) type QuotaQueryFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<crate::quota::QuotaState, String>> + Send>>;
pub(crate) type QuotaQuerier = fn(AppHandle, String, bool) -> QuotaQueryFuture;

static QUOTA_QUERIER: OnceLock<QuotaQuerier> = OnceLock::new();

pub(crate) fn register_quota_querier(querier: QuotaQuerier) {
    let _ = QUOTA_QUERIER.set(querier);
}

async fn query_quota(
    app: AppHandle,
    connection_id: String,
    force: bool,
) -> Result<crate::quota::QuotaState, String> {
    let Some(querier) = QUOTA_QUERIER.get() else {
        return Err("额度查询器尚未初始化".into());
    };
    querier(app, connection_id, force).await
}

/* ─────────────────────────── 调度器 ─────────────────────────── */

pub(crate) struct Engine {
    pub(crate) wake: Mutex<bool>,
    pub(crate) cv: Condvar,
    /// 执行中的进程与停止请求，同一把锁，避免「先停止、后登记」时漏掉停止请求
    pub(crate) procs: Mutex<Procs>,
}

pub(crate) fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine { wake: Mutex::new(false), cv: Condvar::new(), procs: Mutex::new(Procs::default()) })
}

pub(crate) static SCHEMA_READY: AtomicBool = AtomicBool::new(false);

pub(crate) fn shared(app: &AppHandle) -> Shared {
    app.state::<crate::Db>().0.clone()
}

pub(crate) fn kick() {
    let e = engine();
    *e.wake.lock().unwrap_or_else(|p| p.into_inner()) = true;
    e.cv.notify_all();
}

pub(crate) fn emit(app: &AppHandle) {
    let _ = app.emit(CHANGED, ());
}

pub(crate) fn with_conn<T>(db: &Shared, work: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    if !SCHEMA_READY.load(Ordering::Acquire) {
        ensure_schema(&conn).map_err(|e| e.to_string())?;
        SCHEMA_READY.store(true, Ordering::Release);
    }
    work(&conn)
}

pub(crate) async fn blocking<T: Send + 'static>(app: &AppHandle, work: impl FnOnce(&Connection) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let db = shared(app);
    tauri::async_runtime::spawn_blocking(move || with_conn(&db, work)).await.map_err(|e| e.to_string())?
}

/// 本机 CLI 当前登录的连接；判断逻辑与 `current_connections` 一致，只比对、不返回凭证
pub(crate) fn current_connection_ids(db: &Shared) -> Vec<String> {
    if crate::validation::isolated() { return vec![]; }
    let local = crate::current_connection::read_local();
    let Ok(conn) = db.lock() else { return vec![] };
    local.iter().filter_map(|local| crate::connections::find_current_local(&conn, local).ok().flatten()).collect()
}

/// 应用初始化后启动：建表、恢复中断的运行，然后每 30 秒（或有操作时）检查一次
pub fn start(app: AppHandle) {
    let db = shared(&app);
    let interrupted = with_conn(&db, |conn| {
        // 任务 2.5：上次退出时还在执行的运行会转为「需要处理」，恢复后发失败通知
        let mut stmt = conn.prepare("SELECT id FROM automation_runs WHERE status = 'running'").map_err(|e| e.to_string())?;
        let ids: Vec<i64> = stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        drop(stmt);
        recover(conn, now_ms()).map_err(|e| e.to_string())?;
        notices_of(conn, &ids).map_err(|e| e.to_string())
    });
    match interrupted {
        Ok(notices) => send_notices(&app, notices),
        Err(error) => {
            eprintln!("[自动化] 初始化失败：{error}");
            return;
        }
    }
    let spawned = std::thread::Builder::new().name("jolu-automation".into()).spawn(move || loop {
        tick(&app);
        let e = engine();
        let guard = e.wake.lock().unwrap_or_else(|p| p.into_inner());
        let (mut guard, _) = e.cv.wait_timeout_while(guard, TICK, |woken| !*woken).unwrap_or_else(|p| p.into_inner());
        *guard = false;
    });
    if let Err(error) = spawned { eprintln!("[自动化] 调度线程启动失败：{error}"); }
}

pub(crate) fn tick(app: &AppHandle) {
    if let Some(hook) = TICK_HOOK.get() {
        hook(app);
    }
    let db = shared(app);
    let now = now_ms();
    let due = with_conn(&db, |conn| {
        let fired = fire_schedules(conn, &Local, now).map_err(|e| e.to_string())?;
        let (due, expired) = due_checks(conn, now).map_err(|e| e.to_string())?;
        let notices = notices_of(conn, &expired).map_err(|e| e.to_string())?; // 任务 2.5：等待超时归入失败通知
        Ok((due, fired || !expired.is_empty(), notices))
    });
    let due = match due {
        Ok((due, changed, notices)) => {
            if changed { emit(app); }
            send_notices(app, notices);
            due
        }
        Err(error) => {
            eprintln!("[自动化] 调度检查失败：{error}");
            return;
        }
    };
    for connection_id in due {
        check_account_blocking(app, &connection_id);
    }
    match with_conn(&db, |conn| startable(conn).map_err(|e| e.to_string())) {
        Ok(runs) => {
            for run_id in runs { start_run(app, run_id); }
        }
        Err(error) => eprintln!("[自动化] 读取队列失败：{error}"),
    }
}

pub(crate) fn check_account_blocking(app: &AppHandle, connection_id: &str) {
    let db = shared(app);
    let checked_from = now_ms();
    let state = tauri::async_runtime::block_on(query_quota(app.clone(), connection_id.to_string(), true));
    let v = match state {
        Ok(state) => verdict(&state),
        Err(error) => QuotaVerdict::simple("unknown", error),
    };
    let checked = with_conn(&db, |conn| {
        let waiting = waiting_runs(conn, connection_id).map_err(|e| e.to_string())?;
        apply_account_check(conn, connection_id, None, checked_from, &v, now_ms()).map_err(|e| e.to_string())?;
        notices_of(conn, &waiting).map_err(|e| e.to_string()) // 任务 2.5：需要重新登录、无法读取额度归入失败通知
    });
    if let Ok(notices) = checked {
        emit(app);
        send_notices(app, notices);
    }
}

pub(crate) fn start_run(app: &AppHandle, run_id: i64) {
    let db = shared(app);
    let now = now_ms();
    let loaded = with_conn(&db, |conn| {
        let run = load_run(conn, run_id).map_err(|e| e.to_string())?.ok_or("运行记录不存在")?;
        Ok(run)
    });
    let Ok(run) = loaded else { return };
    if run.status != "queued" { return; }
    let attention = |kind: &str, reason: String| {
        let notices = with_conn(&db, |conn| {
            set_attention(conn, run_id, kind, &reason, now).map_err(|e| e.to_string())?;
            notices_of(conn, &[run_id]).map_err(|e| e.to_string()) // 任务 2.5：需要处理归入失败通知
        });
        emit(app);
        send_notices(app, notices.unwrap_or_default());
    };
    let Some(task) = run.config.as_ref() else { return attention("snapshot_missing", MISSING_SNAPSHOT.into()) };
    let Some(tool) = tool_name(&task.platform) else { return attention("tool_missing", "该工具尚未接入执行器".into()) };
    let Some(exe) = crate::session_threads::find_executable(tool) else {
        return attention("tool_missing", format!("未检测到 {}，请安装或配置后重试；已保留进度。", display_name(&task.platform)));
    };
    if !Path::new(&task.cwd).is_dir() {
        return attention("cwd_missing", "项目目录不可访问，无法在原目录执行。".into());
    }
    if !current_connection_ids(&db).contains(&task.connection_id) {
        let name = run.account.clone().unwrap_or_else(|| "所选账号".into());
        return attention(
            "account_mismatch",
            format!("{name} 不是 {} 当前登录的账号；Jolu 不会自动切换账号，请在供应商页启用该账号后重试。", display_name(&task.platform)),
        );
    }
    // D-08：首次运行与续跑前都检查选中的 Skills 仍然启用，不在了就转为需要处理
    if !task.skills.is_empty() {
        let available = match crate::skills::enabled_skills(&task.platform, Some(Path::new(&task.cwd))) {
            Ok(list) => list,
            Err(error) => return attention("skill_missing", format!("无法读取 Skills：{error}")),
        };
        let checked = with_conn(&db, |conn| {
            if skills_ready(conn, run_id, &task.skills, &available, now).map_err(|e| e.to_string())? { return Ok(None); }
            notices_of(conn, &[run_id]).map(Some).map_err(|e| e.to_string()) // 任务 2.5
        });
        match checked {
            Ok(None) => {}
            Ok(Some(notices)) => {
                emit(app);
                send_notices(app, notices);
                return;
            }
            Err(_) => {
                emit(app);
                return;
            }
        }
    }
    // 先认领再拉起进程：认领之后删除会被拒绝（运行中），认领之前被删除或取消则不启动
    engine().procs.lock().unwrap_or_else(|p| p.into_inner()).clear_stop(run_id);
    // 任务 2.5：认领的同时取走「额度已确认」标记，只有额度检查确认可用后的续跑才发「任务已继续」
    let confirmed = match with_conn(&db, |conn| {
        if !claim_run(conn, run_id).map_err(|e| e.to_string())? { return Ok(None); }
        take_resume_confirmed(conn, run_id).map(Some).map_err(|e| e.to_string())
    }) {
        Ok(Some(confirmed)) => confirmed,
        _ => return,
    };
    let session = run.session_id.clone();
    let prompt = run_prompt(&task.platform, &task.skills, &task.prompt, run.resume_pending);
    let args = build_args(&task.platform, task.model.as_deref(), session.as_deref());
    let mut command = Command::new(&exe);
    command.args(&args).current_dir(&task.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return attention("tool_missing", format!("启动 {} 失败：{error}", display_name(&task.platform))),
    };
    let resumed = run.resume_pending;
    let _ = with_conn(&db, |conn| {
        conn.execute(
            "UPDATE automation_runs SET started_at=COALESCE(started_at, ?2), exec_started_at=?2 WHERE id=?1",
            params![run_id, now],
        ).map_err(|e| e.to_string())?;
        let (title, detail) = if resumed { ("恢复原会话", "继续未完成的任务") } else if session.is_some() { ("开始执行", "在所选会话中追加任务指令") } else { ("开始执行", "新建会话") };
        add_event(conn, run_id, now, "start", title, Some(detail)).map_err(|e| e.to_string())
    });
    // 认领后、登记前收到的停止请求：登记时立即结束进程树，不写入提示词；进程退出后按「已取消」收尾
    let stop_now = engine().procs.lock().unwrap_or_else(|p| p.into_inner()).register(run_id, child.id());
    if stop_now { kill_tree(child.id()); }
    emit(app);
    // 任务 2.5：额度恢复后在原会话中继续（等待中的运行经 apply_check 的 available 分支转回排队，到这里真正续跑）
    if resumed && confirmed && !stop_now {
        crate::notify::notify_task(app, task.id, TaskNotice::Resumed, display_name(&task.platform), &task.name);
    }

    let app = app.clone();
    let platform = task.platform.clone();
    std::thread::spawn(move || {
        if let Some(mut stdin) = child.stdin.take().filter(|_| !stop_now) {
            let _ = stdin.write_all(prompt.as_bytes());
        }
        let stderr = child.stderr.take().map(|mut err| std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            lines[lines.len().saturating_sub(8)..].join("\n")
        }));
        let mut progress = Progress::default();
        let db = shared(&app);
        if let Some(out) = child.stdout.take() {
            let mut logged = 0i64;
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let before = progress.session_id.clone();
                let logs = progress.feed(&platform, &line);
                let session_changed = progress.session_id != before;
                if logs.is_empty() && !session_changed { continue; }
                let now = now_ms();
                let _ = with_conn(&db, |conn| {
                    if session_changed {
                        conn.execute("UPDATE automation_runs SET session_id=?2 WHERE id=?1", params![run_id, progress.session_id]).map_err(|e| e.to_string())?;
                    }
                    for log in &logs {
                        if logged >= LOG_LIMIT { break; }
                        add_event(conn, run_id, now, "log", log, None).map_err(|e| e.to_string())?;
                        logged += 1;
                    }
                    Ok(())
                });
            }
        }
        let exit_ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let stderr_tail = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
        let stopped = engine().procs.lock().unwrap_or_else(|p| p.into_inner()).exited(run_id);
        let outcome = classify(&progress, exit_ok, &stderr_tail, stopped);
        let finished = with_conn(&db, |conn| {
            finish(conn, run_id, &outcome, &progress, now_ms()).map_err(|e| e.to_string())?;
            notices_of(conn, &[run_id]).map_err(|e| e.to_string()) // 任务 2.5：完成 / 失败 / 需要处理；已取消与进入等待不通知
        });
        let notices = finished.unwrap_or_else(|error| {
            eprintln!("[自动化] 记录运行结果失败：{error}");
            Vec::new()
        });
        emit(&app);
        send_notices(&app, notices);
        kick();
    });
}

/// 停止运行：进程已登记时结束整个进程树；还没登记（刚认领、正在拉起）时留下停止标记，登记时再结束
pub(crate) fn kill_run(run_id: i64) {
    let pid = engine().procs.lock().unwrap_or_else(|p| p.into_inner()).request_stop(run_id);
    if let Some(pid) = pid { kill_tree(pid); }
}

pub(crate) async fn cmd_overview(app: AppHandle) -> Result<Overview, String> {
    blocking(&app, |conn| overview(conn, now_ms()).map_err(|e| e.to_string())).await
}

pub(crate) async fn cmd_waiting_connections(app: AppHandle) -> Result<Vec<String>, String> {
    blocking(&app, |conn| waiting_connections(conn).map_err(|e| e.to_string())).await
}

pub(crate) async fn cmd_task(app: AppHandle, id: i64) -> Result<TaskDetail, String> {
    blocking(&app, move |conn| task_detail(conn, id)).await
}

pub(crate) async fn cmd_create(app: AppHandle, input: TaskInput) -> Result<i64, String> {
    let id = blocking(&app, move |conn| create_task(conn, &input, now_ms())).await?;
    emit(&app);
    kick();
    Ok(id)
}

pub(crate) async fn cmd_update(app: AppHandle, id: i64, input: TaskInput) -> Result<(), String> {
    blocking(&app, move |conn| update_task(conn, id, &input, now_ms())).await?;
    emit(&app);
    kick();
    Ok(())
}

pub(crate) async fn cmd_delete(app: AppHandle, task_id: i64) -> Result<(), String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|e| e.to_string())?;
        if !SCHEMA_READY.load(Ordering::Acquire) {
            ensure_schema(&conn).map_err(|e| e.to_string())?;
            SCHEMA_READY.store(true, Ordering::Release);
        }
        delete_task(&mut conn, task_id)
    })
    .await
    .map_err(|e| e.to_string())??;
    emit(&app);
    kick();
    Ok(())
}

pub(crate) async fn cmd_action(app: AppHandle, id: i64, action: String) -> Result<(), String> {
    if action == "check" {
        let run = blocking(&app, move |conn| active_run(conn, id).map_err(|e| e.to_string()))
            .await?
            .filter(|r| r.status == "waiting")
            .ok_or("当前没有等待额度的运行")?;
        let checked_from = now_ms();
        let v = match query_quota(
            app.clone(),
            run.connection_id.clone(),
            true,
        )
        .await
        {
            Ok(state) => verdict(&state),
            Err(error) => QuotaVerdict::simple("unknown", error),
        };
        let notices = blocking(&app, move |conn| {
            let waiting = waiting_runs(conn, &run.connection_id).map_err(|e| e.to_string())?;
            apply_account_check(conn, &run.connection_id, Some(run.id), checked_from, &v, now_ms())
                .map_err(|e| e.to_string())?;
            notices_of(conn, &waiting).map_err(|e| e.to_string())
        })
        .await?;
        send_notices(&app, notices);
    } else {
        let kill = blocking(&app, move |conn| act(conn, id, &action, now_ms())).await?;
        if let Some(run_id) = kill {
            kill_run(run_id);
        }
    }
    emit(&app);
    kick();
    Ok(())
}

pub(crate) async fn cmd_runs(app: AppHandle, filter: RunFilter, page: u32) -> Result<RunPage, String> {
    blocking(&app, move |conn| runs(conn, &filter, page, now_ms()).map_err(|e| e.to_string())).await
}

pub(crate) async fn cmd_run(app: AppHandle, id: i64) -> Result<RunDetail, String> {
    blocking(&app, move |conn| run_detail(conn, id)).await
}

pub(crate) async fn cmd_quota(
    app: AppHandle,
    connection_id: String,
    force: bool,
) -> Result<QuotaVerdict, String> {
    Ok(
        match query_quota(app, connection_id, force).await {
            Ok(state) => verdict(&state),
            Err(error) => QuotaVerdict::simple("unknown", error),
        },
    )
}

pub(crate) async fn cmd_skills(
    platform: String,
    cwd: Option<String>,
) -> Result<Vec<crate::skills::SkillOption>, String> {
    if tool_name(&platform).is_none() {
        return Err("该工具尚未接入执行器".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let cwd = cwd
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && Path::new(c).is_dir());
        crate::skills::enabled_skills(&platform, cwd.as_deref().map(Path::new))
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) async fn cmd_pick_dir(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file().set_title("选择项目目录");
    if let Some(window) = app.get_webview_window(crate::MAIN) {
        dialog = dialog.set_parent(&window);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    picked
        .map(|p| p.into_path().map(|p| p.display().to_string()).map_err(|e| e.to_string()))
        .transpose()
}
