//! 幂等建表与列迁移（零业务模块扇出）。

use rusqlite::Connection;

pub(crate) fn ensure_core_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS requests (
            id                 INTEGER PRIMARY KEY,
            platform           TEXT    NOT NULL,
            source             TEXT    NOT NULL,
            dedup_key          TEXT    NOT NULL,
            session_id         TEXT,
            ts                 INTEGER NOT NULL,
            model              TEXT,
            input_tokens       INTEGER NOT NULL DEFAULT 0,
            output_tokens      INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens       INTEGER NOT NULL DEFAULT 0,
            input_known        INTEGER NOT NULL DEFAULT 1,
            output_known       INTEGER NOT NULL DEFAULT 1,
            cache_read_known   INTEGER NOT NULL DEFAULT 1,
            cache_write_known  INTEGER NOT NULL DEFAULT 1,
            total_known        INTEGER NOT NULL DEFAULT 1,
            effort             TEXT,
            duration_ms        INTEGER,
            first_token_ms     INTEGER,
            status_code        INTEGER,
            UNIQUE(source, dedup_key)
        );
        CREATE INDEX IF NOT EXISTS idx_requests_ts ON requests(ts DESC);
        CREATE INDEX IF NOT EXISTS idx_requests_platform_ts ON requests(platform, ts DESC);
        CREATE INDEX IF NOT EXISTS idx_requests_session ON requests(platform, session_id, ts);

        CREATE TABLE IF NOT EXISTS usage_events (
            id          INTEGER PRIMARY KEY,
            request_id  INTEGER NOT NULL,
            platform    TEXT    NOT NULL,
            session_id  TEXT,
            ts          INTEGER NOT NULL,
            delta_total INTEGER NOT NULL,
            FOREIGN KEY(request_id) REFERENCES requests(id)
        );
        CREATE INDEX IF NOT EXISTS idx_usage_events_platform_id ON usage_events(platform, id);
        CREATE INDEX IF NOT EXISTS idx_usage_events_session_ts ON usage_events(platform, session_id, ts);
        CREATE INDEX IF NOT EXISTS idx_usage_events_request ON usage_events(request_id);

        CREATE TABLE IF NOT EXISTS sessions (
            platform   TEXT NOT NULL,
            session_id TEXT NOT NULL,
            title      TEXT,
            is_running INTEGER NOT NULL DEFAULT 0,
            activity_state TEXT NOT NULL DEFAULT 'done',
            activity_updated_ms INTEGER,
            PRIMARY KEY (platform, session_id)
        );

        CREATE TABLE IF NOT EXISTS connections (
            id           TEXT PRIMARY KEY,
            platform     TEXT    NOT NULL,
            kind         TEXT    NOT NULL CHECK (kind IN ('auth','api')),
            name         TEXT    NOT NULL,
            label        TEXT    NOT NULL DEFAULT '',
            secret       TEXT,
            masked       TEXT    NOT NULL DEFAULT '',
            base_url     TEXT,
            status       TEXT    NOT NULL DEFAULT 'connected',
            last_sync_ms INTEGER,
            created_ms   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS scan_state (
            path   TEXT PRIMARY KEY,
            offset INTEGER NOT NULL,
            mtime  INTEGER NOT NULL,
            parser_model TEXT,
            parser_session_id TEXT,
            parser_activity_state TEXT,
            parser_updated_at_ms INTEGER,
            parser_initialized INTEGER NOT NULL DEFAULT 0,
            parser_effort TEXT
        );
        "#,
    )
}

pub(crate) fn ensure_gateway_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS slug_history (
            slug          TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            account_fp    TEXT,
            retired_ms    INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS agent_wiring (
            agent_id      TEXT PRIMARY KEY REFERENCES agents(id),
            instance_id   TEXT NOT NULL,
            owner         TEXT NOT NULL DEFAULT 'none',
            state         TEXT NOT NULL DEFAULT 'idle',
            original_json TEXT,
            applied_json  TEXT,
            token         TEXT UNIQUE,
            wired_ms      INTEGER
        );
        CREATE TABLE IF NOT EXISTS source_groups (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort       INTEGER NOT NULL DEFAULT 0,
            created_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS source_group_members (
            connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
            group_id      TEXT NOT NULL REFERENCES source_groups(id) ON DELETE CASCADE,
            sort          INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS agent_groups (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort       INTEGER NOT NULL DEFAULT 0,
            created_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS gateway_middleware (
            name         TEXT PRIMARY KEY,
            kind         TEXT NOT NULL,
            config_json  TEXT NOT NULL,
            enabled      INTEGER NOT NULL DEFAULT 1,
            sort         INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS routing_groups (
            id           TEXT PRIMARY KEY,
            slug         TEXT UNIQUE NOT NULL,
            name         TEXT NOT NULL,
            strategy     TEXT NOT NULL DEFAULT 'order',
            members_json TEXT NOT NULL DEFAULT '[]',
            created_ms   INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agent_group_members (
            agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
            group_id TEXT NOT NULL REFERENCES agent_groups(id) ON DELETE CASCADE,
            sort     INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS migration_log (
            id     INTEGER PRIMARY KEY,
            at_ms  INTEGER NOT NULL,
            kind   TEXT NOT NULL,
            detail TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS gateway_route_log (
            id             INTEGER PRIMARY KEY,
            ts             INTEGER NOT NULL,
            agent          TEXT,
            requested_model TEXT NOT NULL,
            source         TEXT,
            upstream_model TEXT,
            status_code    INTEGER,
            result         TEXT NOT NULL,
            duration_ms    INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_gateway_route_log_ts ON gateway_route_log(ts DESC);
        CREATE TABLE IF NOT EXISTS gateway_request_sequence (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            value INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO gateway_request_sequence(singleton, value) VALUES(1, 0);
        CREATE TABLE IF NOT EXISTS gateway_context_snapshot (
            id               INTEGER PRIMARY KEY,
            session_platform TEXT    NOT NULL,
            session_id       TEXT    NOT NULL,
            request_id       INTEGER,
            connection_id    TEXT,
            ts               INTEGER NOT NULL,
            state            TEXT    NOT NULL,
            protocol         TEXT    NOT NULL,
            input_tokens     INTEGER,
            output_tokens    INTEGER,
            cache_read       INTEGER,
            window_tokens    INTEGER,
            estimated        INTEGER,
            counted          INTEGER NOT NULL DEFAULT 0,
            parts_json       TEXT,
            calls_json       TEXT,
            error            TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_ctx_snap_session ON gateway_context_snapshot(session_platform, session_id, ts DESC);
        CREATE INDEX IF NOT EXISTS idx_ctx_snap_request ON gateway_context_snapshot(request_id);
        CREATE TABLE IF NOT EXISTS gateway_client_keys (
            name             TEXT PRIMARY KEY,
            protected        TEXT NOT NULL,
            tokens_per_day   INTEGER,
            requests_per_day INTEGER,
            created_ms       INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS library_applied (
            agent        TEXT NOT NULL,
            name         TEXT NOT NULL,
            kind         TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            installed_ms INTEGER NOT NULL,
            PRIMARY KEY (agent, name)
        );
        "#,
    )
}

pub(crate) fn ensure_source_store_schema(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN native_outcome TEXT", []);
    if conn
        .execute(
            "ALTER TABLE requests ADD COLUMN input_semantics TEXT NOT NULL DEFAULT 'unknown'",
            [],
        )
        .is_ok()
    {
        conn.execute(
            "UPDATE requests SET input_semantics=CASE platform WHEN 'claude' THEN 'excludes_cache' WHEN 'codex' THEN 'includes_cache' ELSE 'unknown' END",
            [],
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_metrics(
            request_id INTEGER PRIMARY KEY REFERENCES requests(id),
            credits REAL, original_credits REAL, billable INTEGER, context_ratio REAL, reasoning INTEGER
        );
        CREATE TABLE IF NOT EXISTS native_context(platform TEXT NOT NULL,source TEXT NOT NULL,session_id TEXT NOT NULL,
          model TEXT,ratio REAL NOT NULL,ts INTEGER NOT NULL,PRIMARY KEY(source,session_id));
        CREATE TABLE IF NOT EXISTS collection_policy(singleton INTEGER PRIMARY KEY CHECK(singleton=1), cutoff_ms INTEGER NOT NULL);
        INSERT OR IGNORE INTO collection_policy VALUES(1,0);
        CREATE TABLE IF NOT EXISTS event_sequence(singleton INTEGER PRIMARY KEY CHECK(singleton=1), value INTEGER NOT NULL);
        INSERT OR IGNORE INTO event_sequence SELECT 1,COALESCE(MAX(id),0) FROM usage_events;
        CREATE TRIGGER IF NOT EXISTS usage_event_sequence_insert AFTER INSERT ON usage_events
        BEGIN
          UPDATE usage_events SET id=max(NEW.id,(SELECT value+1 FROM event_sequence WHERE singleton=1)) WHERE id=NEW.id;
          UPDATE event_sequence SET value=(SELECT MAX(id) FROM usage_events) WHERE singleton=1;
        END;
        CREATE TRIGGER IF NOT EXISTS usage_event_sequence_delete AFTER DELETE ON usage_events
        BEGIN UPDATE event_sequence SET value=value+1 WHERE singleton=1; END;
        CREATE TRIGGER IF NOT EXISTS legacy_input_semantics AFTER INSERT ON requests
        WHEN NEW.input_semantics='unknown' AND NEW.platform IN ('claude','codex')
        BEGIN UPDATE requests SET input_semantics=CASE NEW.platform WHEN 'claude' THEN 'excludes_cache' ELSE 'includes_cache' END WHERE id=NEW.id; END;
        ",
    )?;
    let _ = conn.execute("ALTER TABLE request_metrics ADD COLUMN tool INTEGER", []);
    Ok(())
}

pub(crate) fn ensure_auth_snapshot_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_snapshots (
            connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
            protected BLOB NOT NULL, updated_ms INTEGER NOT NULL);
         CREATE TRIGGER IF NOT EXISTS delete_auth_snapshot AFTER DELETE ON connections
            BEGIN DELETE FROM auth_snapshots WHERE connection_id=OLD.id; END;",
    )
}

pub(crate) fn ensure_custom_agents_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_agents (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            slug          TEXT NOT NULL UNIQUE,
            config_path   TEXT NOT NULL,
            format        TEXT NOT NULL,
            base_url_key  TEXT NOT NULL,
            auth_key      TEXT NOT NULL,
            model_key     TEXT,
            base_suffix   TEXT NOT NULL DEFAULT '',
            original_json TEXT,
            applied_json  TEXT,
            wired_ms      INTEGER,
            created_ms    INTEGER NOT NULL
        );",
    )
}

pub(crate) fn ensure_wsl_wiring_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wsl_wiring (
            distro      TEXT NOT NULL,
            agent       TEXT NOT NULL,
            path        TEXT NOT NULL,
            base        TEXT NOT NULL,
            client_key  TEXT,
            snapshot    TEXT,
            wired_ms    INTEGER NOT NULL,
            PRIMARY KEY (distro, agent)
        );",
    )
}

pub(crate) fn ensure_library_shared_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS library_instructions (
            id          TEXT PRIMARY KEY,
            text        TEXT NOT NULL,
            agents_json TEXT NOT NULL,
            updated_ms  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS library_subagents (
            name        TEXT PRIMARY KEY,
            description TEXT NOT NULL,
            prompt      TEXT NOT NULL,
            model       TEXT,
            agents_json TEXT NOT NULL,
            updated_ms  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS library_files (
            kind       TEXT NOT NULL,
            owner      TEXT NOT NULL,
            agent      TEXT NOT NULL,
            path       TEXT NOT NULL,
            hash       TEXT NOT NULL,
            written_ms INTEGER NOT NULL,
            PRIMARY KEY (kind, owner, agent)
        );",
    )
}

pub(crate) fn ensure_daily_checkin_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS daily_checkin_records (
            id INTEGER PRIMARY KEY, connection_id TEXT NOT NULL, name TEXT NOT NULL,
            platform TEXT NOT NULL, edition TEXT NOT NULL, account_key TEXT NOT NULL,
            outcome TEXT NOT NULL, created_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_daily_checkin_records_time ON daily_checkin_records(created_ms DESC, id DESC);
        CREATE INDEX IF NOT EXISTS idx_daily_checkin_records_account_time ON daily_checkin_records(account_key,created_ms);",
    )
}

pub(crate) fn ensure_pricing_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_overrides (
            scope TEXT NOT NULL,
            model_id TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            input REAL NOT NULL DEFAULT 0,
            output REAL NOT NULL DEFAULT 0,
            cache_read REAL NOT NULL DEFAULT 0,
            cache_write REAL,
            cache_write_1h REAL,
            long_ctx_threshold INTEGER,
            long_ctx_input REAL,
            long_ctx_output REAL,
            long_ctx_cache_read REAL,
            long_ctx_cache_write REAL,
            disabled INTEGER NOT NULL DEFAULT 0,
            updated_ms INTEGER NOT NULL,
            PRIMARY KEY (scope, model_id)
        );
        CREATE TABLE IF NOT EXISTS pricing_aliases (
            scope TEXT NOT NULL,
            alias_model TEXT NOT NULL,
            target_model TEXT NOT NULL,
            updated_ms INTEGER NOT NULL,
            PRIMARY KEY (scope, alias_model)
        );
        CREATE TABLE IF NOT EXISTS official_cost_snapshots (
            connection_id TEXT NOT NULL,
            day TEXT NOT NULL,
            cost_usd REAL NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            synced_ms INTEGER NOT NULL,
            PRIMARY KEY (connection_id, day)
        );
        "#,
    )
}

pub(crate) fn run_column_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN input_known INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN output_known INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN cache_read_known INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN cache_write_known INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN total_known INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN effort TEXT", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN duration_ms INTEGER", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN first_token_ms INTEGER", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN status_code INTEGER", []);
    let _ = conn.execute("ALTER TABLE usage_events ADD COLUMN delta_fresh INTEGER", []);

    conn.execute(
        "INSERT INTO usage_events(request_id, platform, session_id, ts, delta_total, delta_fresh)
         SELECT r.id, r.platform, r.session_id, r.ts, r.total_tokens,
                r.input_tokens + r.output_tokens
         FROM requests r
         WHERE r.total_known = 1 AND r.total_tokens > 0
           AND NOT EXISTS (SELECT 1 FROM usage_events e WHERE e.request_id = r.id)",
        [],
    )?;

    let _ = conn.execute("ALTER TABLE connections ADD COLUMN base_url TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN model TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN effort TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN context_1m INTEGER", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN protocol TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN service TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN account_email TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN cwd TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN is_subagent INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN serve_enabled INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN is_running INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN start_channel TEXT", []);
    let _ = conn.execute("ALTER TABLE routing_groups ADD COLUMN intent_json TEXT", []);

    ensure_custom_agents_schema(conn)?;
    ensure_wsl_wiring_schema(conn)?;
    ensure_library_shared_schema(conn)?;

    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN connection_id TEXT", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN session_platform TEXT", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN session_id TEXT", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN client_key TEXT", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN request_id INTEGER", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN request_uid INTEGER", []);
    let _ = conn.execute("ALTER TABLE gateway_context_snapshot ADD COLUMN request_uid INTEGER", []);
    conn.execute("CREATE INDEX IF NOT EXISTS idx_gateway_route_log_uid ON gateway_route_log(request_uid)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_ctx_snap_uid ON gateway_context_snapshot(request_uid)", [])?;
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_gateway_route_log_request ON gateway_route_log(request_id)", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN resolved_from TEXT", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN profile TEXT", []);
    let _ = conn.execute("ALTER TABLE gateway_route_log ADD COLUMN decision TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN activity_state TEXT NOT NULL DEFAULT 'done'", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN activity_updated_ms INTEGER", []);
    let _ = conn.execute(
        "UPDATE sessions SET activity_state = 'running' WHERE is_running = 1 AND activity_state = 'done'",
        [],
    );
    let _ = conn.execute("ALTER TABLE scan_state ADD COLUMN parser_model TEXT", []);
    let _ = conn.execute("ALTER TABLE scan_state ADD COLUMN parser_session_id TEXT", []);
    let _ = conn.execute("ALTER TABLE scan_state ADD COLUMN parser_activity_state TEXT", []);
    let _ = conn.execute("ALTER TABLE scan_state ADD COLUMN parser_updated_at_ms INTEGER", []);
    let _ = conn.execute("ALTER TABLE scan_state ADD COLUMN parser_effort TEXT", []);
    let _ = conn.execute("ALTER TABLE scan_state ADD COLUMN parser_initialized INTEGER NOT NULL DEFAULT 0", []);
    if conn.execute("ALTER TABLE sessions ADD COLUMN turn_started_ms INTEGER", []).is_ok() {
        conn.execute("UPDATE scan_state SET parser_initialized = 0", [])?;
    }
    if conn.execute("ALTER TABLE scan_state ADD COLUMN parser_duration_v1 INTEGER NOT NULL DEFAULT 0", []).is_ok() {
        conn.execute("UPDATE scan_state SET parser_initialized = 0", [])?;
    }
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN connection_id TEXT", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN attribution_source TEXT", []);
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_requests_connection ON requests(connection_id, ts);")?;
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN provider_key TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN slug TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN account_fp TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN models_json TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN catalog_auto TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN exhausted_day TEXT", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN billing_mode TEXT NOT NULL DEFAULT 'auto'", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN price_multiplier REAL NOT NULL DEFAULT 1.0", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN budget_daily_usd REAL", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN budget_monthly_usd REAL", []);
    let _ = conn.execute("ALTER TABLE connections ADD COLUMN budget_action TEXT NOT NULL DEFAULT 'alert'", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN upstream_model TEXT", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN served_model TEXT", []);
    let _ = conn.execute("ALTER TABLE requests ADD COLUMN cache_write_1h_tokens INTEGER NOT NULL DEFAULT 0", []);
    Ok(())
}

/// 纯表结构与列迁移初始化（不含业务层数据回填/身份迁移）。
pub(crate) fn init_tables(conn: &Connection) -> rusqlite::Result<()> {
    ensure_core_tables(conn)?;
    ensure_gateway_tables(conn)?;
    ensure_source_store_schema(conn)?;
    ensure_auth_snapshot_schema(conn)?;
    run_column_migrations(conn)?;
    ensure_pricing_tables(conn)?;
    ensure_daily_checkin_schema(conn)?;
    Ok(())
}
