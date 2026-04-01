use anyhow::Context;
use libsql::Connection as LibsqlConnection;
use sqlx::{Row, SqlitePool};

const SCHEMA_STATEMENTS: &[(&str, &str)] = &[
    (
        "failed to create outbox_events table",
        r#"
        CREATE TABLE IF NOT EXISTS outbox_events (
            event_id TEXT PRIMARY KEY,
            payload TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_attempt_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_error TEXT
        )
        "#,
    ),
    (
        "failed to create outbox_events index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_outbox_status_next_attempt
        ON outbox_events(status, next_attempt_at)
        "#,
    ),
    (
        "failed to create session_events table",
        r#"
        CREATE TABLE IF NOT EXISTS session_events (
            event_id TEXT PRIMARY KEY,
            event_type TEXT NOT NULL,
            session_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            direction TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create session_events timestamp index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_timestamp
        ON session_events(timestamp DESC)
        "#,
    ),
    (
        "failed to create session_events session index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_session_timestamp
        ON session_events(session_id, timestamp DESC)
        "#,
    ),
    (
        "failed to create session_events user index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_user_timestamp
        ON session_events(user_id, timestamp DESC)
        "#,
    ),
    (
        "failed to create session_events type index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_type_timestamp
        ON session_events(event_type, timestamp DESC)
        "#,
    ),
    (
        "failed to create session_presence table",
        r#"
        CREATE TABLE IF NOT EXISTS session_presence (
            session_id TEXT NOT NULL,
            participant_id TEXT NOT NULL,
            display_name TEXT NOT NULL,
            avatar_url TEXT,
            is_active INTEGER NOT NULL DEFAULT 1,
            is_control_active INTEGER NOT NULL DEFAULT 0,
            last_activity_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(session_id, participant_id)
        )
        "#,
    ),
    (
        "failed to create session_presence active index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_presence_active
        ON session_presence(session_id, is_active)
        "#,
    ),
    (
        "failed to create session_presence updated index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_presence_updated
        ON session_presence(updated_at)
        "#,
    ),
    (
        "failed to create dashboard_users table",
        r#"
        CREATE TABLE IF NOT EXISTS dashboard_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL,
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create dashboard_sessions table",
        r#"
        CREATE TABLE IF NOT EXISTS dashboard_sessions (
            session_token TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            FOREIGN KEY(user_id) REFERENCES dashboard_users(id)
        )
        "#,
    ),
    (
        "failed to create dashboard_sessions expiry index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_dashboard_sessions_expiry
        ON dashboard_sessions(expires_at)
        "#,
    ),
    (
        "failed to create helpdesk_agents table",
        r#"
        CREATE TABLE IF NOT EXISTS helpdesk_agents (
            agent_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            avatar_url TEXT,
            status TEXT NOT NULL,
            current_ticket_id TEXT,
            last_heartbeat_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create helpdesk_agents status index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_helpdesk_agents_status_updated
        ON helpdesk_agents(status, updated_at)
        "#,
    ),
    (
        "failed to create helpdesk_authorized_agents table",
        r#"
        CREATE TABLE IF NOT EXISTS helpdesk_authorized_agents (
            agent_id TEXT PRIMARY KEY,
            display_name TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create helpdesk_tickets table",
        r#"
        CREATE TABLE IF NOT EXISTS helpdesk_tickets (
            ticket_id TEXT PRIMARY KEY,
            client_id TEXT NOT NULL,
            client_display_name TEXT,
            device_id TEXT,
            requested_by TEXT,
            title TEXT,
            description TEXT,
            difficulty TEXT,
            estimated_minutes INTEGER,
            summary TEXT,
            status TEXT NOT NULL,
            assigned_agent_id TEXT,
            opening_deadline_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create helpdesk_tickets status index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_helpdesk_tickets_status_created
        ON helpdesk_tickets(status, created_at)
        "#,
    ),
    (
        "failed to create helpdesk_tickets agent index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_helpdesk_tickets_agent
        ON helpdesk_tickets(assigned_agent_id, updated_at)
        "#,
    ),
    (
        "failed to create helpdesk_ticket_assignments table",
        r#"
        CREATE TABLE IF NOT EXISTS helpdesk_ticket_assignments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ticket_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create helpdesk_ticket_assignments ticket index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_helpdesk_ticket_assignments_ticket
        ON helpdesk_ticket_assignments(ticket_id, updated_at DESC)
        "#,
    ),
    (
        "failed to create helpdesk_agent_heartbeats table",
        r#"
        CREATE TABLE IF NOT EXISTS helpdesk_agent_heartbeats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create helpdesk_agent_heartbeats index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_helpdesk_agent_heartbeats_agent_created
        ON helpdesk_agent_heartbeats(agent_id, created_at DESC)
        "#,
    ),
    (
        "failed to create helpdesk_audit_events table",
        r#"
        CREATE TABLE IF NOT EXISTS helpdesk_audit_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            payload TEXT,
            created_at INTEGER NOT NULL
        )
        "#,
    ),
    (
        "failed to create helpdesk_audit_events index",
        r#"
        CREATE INDEX IF NOT EXISTS idx_helpdesk_audit_entity
        ON helpdesk_audit_events(entity_type, entity_id, created_at DESC)
        "#,
    ),
];

pub async fn init_sqlite_schema(pool: &SqlitePool) -> anyhow::Result<()> {
    for (error_context, statement) in SCHEMA_STATEMENTS {
        sqlx::query(statement)
            .execute(pool)
            .await
            .context(*error_context)?;
    }

    ensure_sqlite_text_column(pool, "helpdesk_agents", "avatar_url").await?;
    ensure_sqlite_text_column(pool, "helpdesk_tickets", "title").await?;
    ensure_sqlite_text_column(pool, "helpdesk_tickets", "description").await?;
    ensure_sqlite_text_column(pool, "helpdesk_tickets", "difficulty").await?;
    ensure_sqlite_integer_column(pool, "helpdesk_tickets", "estimated_minutes").await?;

    Ok(())
}

pub async fn init_libsql_schema(conn: &LibsqlConnection) -> anyhow::Result<()> {
    for (error_context, statement) in SCHEMA_STATEMENTS {
        conn.execute(statement, ()).await.context(*error_context)?;
    }

    ensure_libsql_text_column(conn, "helpdesk_agents", "avatar_url").await?;
    ensure_libsql_text_column(conn, "helpdesk_tickets", "title").await?;
    ensure_libsql_text_column(conn, "helpdesk_tickets", "description").await?;
    ensure_libsql_text_column(conn, "helpdesk_tickets", "difficulty").await?;
    ensure_libsql_integer_column(conn, "helpdesk_tickets", "estimated_minutes").await?;

    Ok(())
}

async fn ensure_sqlite_text_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> anyhow::Result<()> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to inspect SQLite schema for table '{table}'"))?;

    let exists = rows
        .iter()
        .any(|row| row.get::<String, _>("name").eq_ignore_ascii_case(column));
    if exists {
        return Ok(());
    }

    sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"))
        .execute(pool)
        .await
        .with_context(|| format!("failed to add column '{column}' to table '{table}'"))?;
    Ok(())
}

async fn ensure_sqlite_integer_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> anyhow::Result<()> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to inspect SQLite schema for table '{table}'"))?;

    let exists = rows
        .iter()
        .any(|row| row.get::<String, _>("name").eq_ignore_ascii_case(column));
    if exists {
        return Ok(());
    }

    sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {column} INTEGER"))
        .execute(pool)
        .await
        .with_context(|| format!("failed to add column '{column}' to table '{table}'"))?;
    Ok(())
}

async fn ensure_libsql_text_column(
    conn: &LibsqlConnection,
    table: &str,
    column: &str,
) -> anyhow::Result<()> {
    let mut rows = conn
        .query(&format!("PRAGMA table_info({table})"), ())
        .await
        .with_context(|| format!("failed to inspect libSQL schema for table '{table}'"))?;

    while let Some(row) = rows.next().await? {
        let name: String = row.get(1)?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(());
        }
    }

    conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"), ())
        .await
        .with_context(|| format!("failed to add column '{column}' to table '{table}'"))?;
    Ok(())
}

async fn ensure_libsql_integer_column(
    conn: &LibsqlConnection,
    table: &str,
    column: &str,
) -> anyhow::Result<()> {
    let mut rows = conn
        .query(&format!("PRAGMA table_info({table})"), ())
        .await
        .with_context(|| format!("failed to inspect libSQL schema for table '{table}'"))?;

    while let Some(row) = rows.next().await? {
        let name: String = row.get(1)?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(());
        }
    }

    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} INTEGER"),
        (),
    )
    .await
    .with_context(|| format!("failed to add column '{column}' to table '{table}'"))?;
    Ok(())
}
