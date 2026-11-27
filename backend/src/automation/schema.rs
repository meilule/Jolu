use rusqlite::Connection;

/* ─────────────────────────── 数据表 ─────────────────────────── */

pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS automation_tasks (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            name          TEXT    NOT NULL,
            platform      TEXT    NOT NULL,
            connection_id TEXT    NOT NULL,
            model         TEXT,
            cwd           TEXT    NOT NULL,
            session_id    TEXT,
            prompt        TEXT    NOT NULL,
            skills        TEXT    NOT NULL DEFAULT '[]',
            launch        TEXT    NOT NULL,
            schedule      TEXT,
            auto_resume   INTEGER NOT NULL DEFAULT 1,
            wait_deadline INTEGER,
            state         TEXT    NOT NULL DEFAULT 'active',
            next_run_at   INTEGER,
            missed_at     INTEGER,
            created_at    INTEGER NOT NULL,
            updated_at    INTEGER NOT NULL,
            serial_queue  INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS automation_runs (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id         INTEGER NOT NULL,
            source          TEXT    NOT NULL,
            status          TEXT    NOT NULL,
            block_kind      TEXT,
            reason          TEXT,
            resets_at       INTEGER,
            last_check_at   INTEGER,
            next_check_at   INTEGER,
            check_failures  INTEGER NOT NULL DEFAULT 0,
            resume_pending  INTEGER NOT NULL DEFAULT 0,
            resume_confirmed INTEGER NOT NULL DEFAULT 0,
            session_id      TEXT,
            account         TEXT,
            model           TEXT,
            created_at      INTEGER NOT NULL,
            started_at      INTEGER,
            ended_at        INTEGER,
            exec_ms         INTEGER NOT NULL DEFAULT 0,
            wait_ms         INTEGER NOT NULL DEFAULT 0,
            exec_started_at INTEGER,
            wait_started_at INTEGER,
            waits           INTEGER NOT NULL DEFAULT 0,
            input_tokens    INTEGER,
            cache_read_tokens INTEGER,
            output_tokens   INTEGER,
            result          TEXT,
            files           TEXT    NOT NULL DEFAULT '[]',
            error           TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_automation_runs_task ON automation_runs(task_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_automation_runs_created ON automation_runs(created_at DESC);
        CREATE TABLE IF NOT EXISTS automation_events (
            id     INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id INTEGER NOT NULL,
            ts     INTEGER NOT NULL,
            kind   TEXT    NOT NULL,
            title  TEXT    NOT NULL,
            detail TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_automation_events_run ON automation_events(run_id, id);
        "#,
    )?;
    // 任务 2.6：旧库补「同账号排队」列，默认开启
    let has_serial = conn.prepare("SELECT 1 FROM pragma_table_info('automation_tasks') WHERE name = 'serial_queue'")?.exists([])?;
    if !has_serial {
        conn.execute("ALTER TABLE automation_tasks ADD COLUMN serial_queue INTEGER NOT NULL DEFAULT 1", [])?;
    }
    // 任务 2.5：旧库补「额度已确认」标记列（见 [`take_resume_confirmed`]），默认未确认
    let has_confirmed = conn.prepare("SELECT 1 FROM pragma_table_info('automation_runs') WHERE name = 'resume_confirmed'")?.exists([])?;
    if !has_confirmed {
        conn.execute("ALTER TABLE automation_runs ADD COLUMN resume_confirmed INTEGER NOT NULL DEFAULT 0", [])?;
    }
    // 只加可空列，不用当前任务猜填旧运行的历史参数。
    let has_snapshot = conn.prepare("SELECT 1 FROM pragma_table_info('automation_runs') WHERE name = 'task_snapshot'")?.exists([])?;
    if !has_snapshot {
        conn.execute("ALTER TABLE automation_runs ADD COLUMN task_snapshot TEXT", [])?;
    }
    Ok(())
}
