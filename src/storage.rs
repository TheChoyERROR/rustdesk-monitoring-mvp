use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use crate::model::{
    AuthRoleV1, AuthUserV1, DashboardSummaryV1, PresenceParticipantV1, PresenceSessionSummaryV1,
    SessionEventType, SessionEventV1, SessionPresenceV1, SessionReportRowV1, SessionTimelineItemV1,
};

#[derive(Debug, Clone)]
pub struct OutboxRecord {
    pub event_id: String,
    pub payload: String,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone)]
struct PresenceActor {
    participant_id: String,
    display_name_override: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DashboardUserRecord {
    pub id: i64,
    pub username: String,
    pub role: AuthRoleV1,
    pub password_hash: String,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct DashboardSessionRecord {
    pub session_token: String,
    pub user: AuthUserV1,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct EventQueryFilter {
    pub session_id: Option<String>,
    pub user_id: Option<String>,
    pub event_type: Option<SessionEventType>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

pub async fn connect_sqlite(database_path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = database_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create database directory: {}", parent.display()))?;
        }
    }

    let url = format!("sqlite://{}", database_path.display());
    let options = url
        .parse::<SqliteConnectOptions>()
        .with_context(|| format!("invalid SQLite URL generated from path: {}", database_path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await
        .context("failed to open SQLite database")?;

    init_schema(&pool).await?;
    Ok(pool)
}

pub async fn init_schema(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
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
    )
    .execute(pool)
    .await
    .context("failed to create outbox_events table")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_outbox_status_next_attempt
        ON outbox_events(status, next_attempt_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create outbox_events index")?;

    sqlx::query(
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
    )
    .execute(pool)
    .await
    .context("failed to create session_events table")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_timestamp
        ON session_events(timestamp DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create session_events timestamp index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_session_timestamp
        ON session_events(session_id, timestamp DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create session_events session index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_user_timestamp
        ON session_events(user_id, timestamp DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create session_events user index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_events_type_timestamp
        ON session_events(event_type, timestamp DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create session_events type index")?;

    sqlx::query(
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
    )
    .execute(pool)
    .await
    .context("failed to create session_presence table")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_presence_active
        ON session_presence(session_id, is_active)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create session_presence active index")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_presence_updated
        ON session_presence(updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create session_presence updated index")?;

    sqlx::query(
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
    )
    .execute(pool)
    .await
    .context("failed to create dashboard_users table")?;

    sqlx::query(
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
    )
    .execute(pool)
    .await
    .context("failed to create dashboard_sessions table")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_dashboard_sessions_expiry
        ON dashboard_sessions(expires_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to create dashboard_sessions expiry index")?;

    Ok(())
}

pub async fn insert_event(pool: &SqlitePool, event: &SessionEventV1) -> anyhow::Result<InsertOutcome> {
    let payload = serde_json::to_string(event).context("failed to serialize event payload")?;
    let now_ms = unix_millis_now() as i64;

    let mut tx = pool.begin().await.context("failed to open ingest transaction")?;

    let insert_result = sqlx::query(
        r#"
        INSERT INTO outbox_events (
            event_id,
            payload,
            status,
            attempts,
            next_attempt_at,
            created_at,
            updated_at,
            last_error
        ) VALUES (?1, ?2, 'pending', 0, ?3, ?4, ?5, NULL)
        "#,
    )
    .bind(event.event_id.to_string())
    .bind(payload)
    .bind(now_ms)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await;

    match insert_result {
        Ok(_) => {
            let event_payload = serde_json::to_string(event).context("failed to serialize session event")?;
            sqlx::query(
                r#"
                INSERT INTO session_events (
                    event_id,
                    event_type,
                    session_id,
                    user_id,
                    direction,
                    timestamp,
                    payload,
                    created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
            )
            .bind(event.event_id.to_string())
            .bind(event.event_type.as_str())
            .bind(&event.session_id)
            .bind(&event.user_id)
            .bind(match event.direction {
                crate::model::SessionDirection::Incoming => "incoming",
                crate::model::SessionDirection::Outgoing => "outgoing",
            })
            .bind(event.timestamp.to_rfc3339())
            .bind(event_payload)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .context("failed to insert session event")?;

            apply_presence_event_tx(&mut tx, event, now_ms).await?;
            tx.commit().await.context("failed to commit ingest transaction")?;
            Ok(InsertOutcome::Inserted)
        }
        Err(sqlx::Error::Database(db_error)) if db_error.is_unique_violation() => {
            let _ = tx.rollback().await;
            Ok(InsertOutcome::Duplicate)
        }
        Err(err) => Err(err).context("failed to insert event into outbox"),
    }
}

pub async fn claim_due_events(
    pool: &SqlitePool,
    limit: usize,
    now_ms: u64,
) -> anyhow::Result<Vec<OutboxRecord>> {
    let now_ms = now_ms as i64;
    let rows = sqlx::query(
        r#"
        SELECT event_id, payload, attempts
        FROM outbox_events
        WHERE status = 'pending' AND next_attempt_at <= ?1
        ORDER BY created_at ASC
        LIMIT ?2
        "#,
    )
    .bind(now_ms)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("failed to query due events")?;

    let mut claimed = Vec::with_capacity(rows.len());

    for row in rows {
        let event_id: String = row.get("event_id");
        let payload: String = row.get("payload");
        let attempts_i64: i64 = row.get("attempts");
        let attempts = u32::try_from(attempts_i64).unwrap_or(u32::MAX);

        let claim_result = sqlx::query(
            r#"
            UPDATE outbox_events
            SET status = 'processing', updated_at = ?1
            WHERE event_id = ?2 AND status = 'pending'
            "#,
        )
        .bind(now_ms)
        .bind(&event_id)
        .execute(pool)
        .await
        .with_context(|| format!("failed to claim event {event_id}"))?;

        if claim_result.rows_affected() == 1 {
            claimed.push(OutboxRecord {
                event_id,
                payload,
                attempts,
            });
        }
    }

    Ok(claimed)
}

pub async fn mark_delivered(
    pool: &SqlitePool,
    event_id: &str,
    attempts: u32,
    now_ms: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET status = 'delivered', attempts = ?1, updated_at = ?2, last_error = NULL
        WHERE event_id = ?3
        "#,
    )
    .bind(attempts as i64)
    .bind(now_ms as i64)
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark delivered for event {event_id}"))?;
    Ok(())
}

pub async fn schedule_retry(
    pool: &SqlitePool,
    event_id: &str,
    attempts: u32,
    next_attempt_at_ms: u64,
    error_message: &str,
    now_ms: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET
            status = 'pending',
            attempts = ?1,
            next_attempt_at = ?2,
            updated_at = ?3,
            last_error = ?4
        WHERE event_id = ?5
        "#,
    )
    .bind(attempts as i64)
    .bind(next_attempt_at_ms as i64)
    .bind(now_ms as i64)
    .bind(error_message)
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to schedule retry for event {event_id}"))?;

    Ok(())
}

pub async fn mark_failed(
    pool: &SqlitePool,
    event_id: &str,
    attempts: u32,
    error_message: &str,
    now_ms: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET
            status = 'failed',
            attempts = ?1,
            updated_at = ?2,
            next_attempt_at = ?3,
            last_error = ?4
        WHERE event_id = ?5
        "#,
    )
    .bind(attempts as i64)
    .bind(now_ms as i64)
    .bind(now_ms as i64)
    .bind(error_message)
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark failed for event {event_id}"))?;
    Ok(())
}

pub async fn reset_stuck_processing(
    pool: &SqlitePool,
    older_than_ms: u64,
    now_ms: u64,
) -> anyhow::Result<u64> {
    let threshold = now_ms.saturating_sub(older_than_ms) as i64;
    let result = sqlx::query(
        r#"
        UPDATE outbox_events
        SET status = 'pending', updated_at = ?1
        WHERE status = 'processing' AND updated_at <= ?2
        "#,
    )
    .bind(now_ms as i64)
    .bind(threshold)
    .execute(pool)
    .await
    .context("failed to reset stuck processing rows")?;

    Ok(result.rows_affected())
}

pub async fn cleanup_failed_older_than(pool: &SqlitePool, cutoff_ms: u64) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM outbox_events
        WHERE status = 'failed' AND updated_at < ?1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to clean old failed events")?;

    Ok(result.rows_affected())
}

pub async fn expire_stale_presence(
    pool: &SqlitePool,
    stale_before_ms: i64,
    now_ms: i64,
) -> anyhow::Result<(u64, Vec<String>)> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT session_id
        FROM session_presence
        WHERE is_active = 1 AND last_activity_at < ?1
        "#,
    )
    .bind(stale_before_ms)
    .fetch_all(pool)
    .await
    .context("failed to query stale presence sessions")?;

    if rows.is_empty() {
        return Ok((0, Vec::new()));
    }

    let touched_sessions = rows
        .into_iter()
        .map(|row| row.get::<String, _>("session_id"))
        .collect::<Vec<_>>();

    let result = sqlx::query(
        r#"
        UPDATE session_presence
        SET is_active = 0, is_control_active = 0, updated_at = ?2
        WHERE is_active = 1 AND last_activity_at < ?1
        "#,
    )
    .bind(stale_before_ms)
    .bind(now_ms)
    .execute(pool)
    .await
    .context("failed to expire stale session presence rows")?;

    Ok((result.rows_affected(), touched_sessions))
}

pub async fn get_session_presence(
    pool: &SqlitePool,
    session_id: &str,
) -> anyhow::Result<Option<SessionPresenceV1>> {
    let rows = sqlx::query(
        r#"
        SELECT
            participant_id,
            display_name,
            avatar_url,
            is_active,
            is_control_active,
            last_activity_at,
            updated_at
        FROM session_presence
        WHERE session_id = ?1
        ORDER BY is_active DESC, last_activity_at DESC, participant_id ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to query presence for session {session_id}"))?;

    if rows.is_empty() {
        return Ok(None);
    }

    let mut participants = Vec::with_capacity(rows.len());
    let mut control_participant_id = None;
    let mut newest_update_ms = 0_i64;

    for row in rows {
        let participant_id: String = row.get("participant_id");
        let display_name: String = row.get("display_name");
        let avatar_url: Option<String> = row.get("avatar_url");
        let is_active = i64_to_bool(row.get("is_active"));
        let is_control_active = i64_to_bool(row.get("is_control_active"));
        let last_activity_at: i64 = row.get("last_activity_at");
        let updated_at: i64 = row.get("updated_at");

        if is_control_active && is_active {
            control_participant_id = Some(participant_id.clone());
        }
        if updated_at > newest_update_ms {
            newest_update_ms = updated_at;
        }

        participants.push(PresenceParticipantV1 {
            participant_id,
            display_name,
            avatar_url,
            is_active,
            is_control_active,
            last_activity_at: millis_to_utc(last_activity_at),
        });
    }

    Ok(Some(SessionPresenceV1 {
        session_id: session_id.to_string(),
        control_participant_id,
        participants,
        updated_at: millis_to_utc(newest_update_ms),
    }))
}

pub async fn list_active_session_presence(pool: &SqlitePool) -> anyhow::Result<Vec<PresenceSessionSummaryV1>> {
    let rows = sqlx::query(
        r#"
        SELECT
            session_id,
            COUNT(*) AS active_participants,
            MAX(updated_at) AS updated_at
        FROM session_presence
        WHERE is_active = 1
        GROUP BY session_id
        ORDER BY updated_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query active session presence")?;

    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        let session_id: String = row.get("session_id");
        let active_participants_raw: i64 = row.get("active_participants");
        let updated_at: i64 = row.get("updated_at");

        summaries.push(PresenceSessionSummaryV1 {
            session_id,
            active_participants: u64::try_from(active_participants_raw).unwrap_or(0),
            updated_at: millis_to_utc(updated_at),
        });
    }

    Ok(summaries)
}

pub async fn upsert_dashboard_user(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    role: AuthRoleV1,
) -> anyhow::Result<()> {
    let now_ms = unix_millis_now() as i64;
    sqlx::query(
        r#"
        INSERT INTO dashboard_users (
            username, password_hash, role, is_active, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, 1, ?4, ?5)
        ON CONFLICT(username) DO UPDATE SET
            password_hash = excluded.password_hash,
            role = excluded.role,
            is_active = 1,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(role.as_str())
    .bind(now_ms)
    .bind(now_ms)
    .execute(pool)
    .await
    .with_context(|| format!("failed to upsert dashboard user '{username}'"))?;
    Ok(())
}

pub async fn get_dashboard_user_by_username(
    pool: &SqlitePool,
    username: &str,
) -> anyhow::Result<Option<DashboardUserRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, username, role, password_hash, is_active
        FROM dashboard_users
        WHERE username = ?1
        "#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to query dashboard user '{username}'"))?;

    let Some(row) = row else {
        return Ok(None);
    };

    let role_str: String = row.get("role");
    Ok(Some(DashboardUserRecord {
        id: row.get("id"),
        username: row.get("username"),
        role: role_from_db(&role_str),
        password_hash: row.get("password_hash"),
        is_active: i64_to_bool(row.get("is_active")),
    }))
}

pub async fn create_dashboard_session(
    pool: &SqlitePool,
    session_token: &str,
    user_id: i64,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let now_ms = unix_millis_now() as i64;
    sqlx::query(
        r#"
        INSERT INTO dashboard_sessions (
            session_token, user_id, expires_at, created_at, last_seen_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(session_token)
    .bind(user_id)
    .bind(expires_at.timestamp_millis())
    .bind(now_ms)
    .bind(now_ms)
    .execute(pool)
    .await
    .with_context(|| format!("failed to create dashboard session for user_id={user_id}"))?;
    Ok(())
}

pub async fn get_dashboard_session_by_token(
    pool: &SqlitePool,
    session_token: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<DashboardSessionRecord>> {
    let now_ms = now.timestamp_millis();
    let row = sqlx::query(
        r#"
        SELECT
            s.session_token,
            s.expires_at,
            u.id AS user_id,
            u.username,
            u.role,
            u.is_active
        FROM dashboard_sessions s
        INNER JOIN dashboard_users u ON u.id = s.user_id
        WHERE s.session_token = ?1 AND s.expires_at > ?2
        "#,
    )
    .bind(session_token)
    .bind(now_ms)
    .fetch_optional(pool)
    .await
    .context("failed to query dashboard session")?;

    let Some(row) = row else {
        return Ok(None);
    };

    if !i64_to_bool(row.get("is_active")) {
        return Ok(None);
    }

    let user_id: i64 = row.get("user_id");
    let username: String = row.get("username");
    let role_str: String = row.get("role");
    let expires_at_ms: i64 = row.get("expires_at");

    sqlx::query(
        r#"
        UPDATE dashboard_sessions
        SET last_seen_at = ?2
        WHERE session_token = ?1
        "#,
    )
    .bind(session_token)
    .bind(unix_millis_now() as i64)
    .execute(pool)
    .await
    .context("failed to touch dashboard session")?;

    Ok(Some(DashboardSessionRecord {
        session_token: row.get("session_token"),
        user: AuthUserV1 {
            id: user_id,
            username,
            role: role_from_db(&role_str),
        },
        expires_at: millis_to_utc(expires_at_ms),
    }))
}

pub async fn delete_dashboard_session(pool: &SqlitePool, session_token: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        DELETE FROM dashboard_sessions
        WHERE session_token = ?1
        "#,
    )
    .bind(session_token)
    .execute(pool)
    .await
    .context("failed to delete dashboard session")?;
    Ok(())
}

pub async fn cleanup_expired_dashboard_sessions(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM dashboard_sessions
        WHERE expires_at <= ?1
        "#,
    )
    .bind(now.timestamp_millis())
    .execute(pool)
    .await
    .context("failed to cleanup expired dashboard sessions")?;
    Ok(result.rows_affected())
}

pub async fn get_dashboard_summary(
    pool: &SqlitePool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<DashboardSummaryV1> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();

    let row = sqlx::query(
        r#"
        SELECT
            COUNT(*) AS events_total,
            SUM(CASE WHEN event_type = 'session_started' THEN 1 ELSE 0 END) AS sessions_started,
            SUM(CASE WHEN event_type = 'session_ended' THEN 1 ELSE 0 END) AS sessions_ended
        FROM session_events
        WHERE timestamp >= ?1 AND timestamp <= ?2
        "#,
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_one(pool)
    .await
    .context("failed to aggregate dashboard summary events")?;

    let active_sessions_row = sqlx::query(
        r#"
        SELECT COUNT(DISTINCT session_id) AS active_sessions
        FROM session_presence
        WHERE is_active = 1
        "#,
    )
    .fetch_one(pool)
    .await
    .context("failed to count active sessions")?;

    let outbox_rows = sqlx::query(
        r#"
        SELECT status, COUNT(*) AS total
        FROM outbox_events
        GROUP BY status
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to aggregate outbox status counts")?;

    let mut webhook_pending = 0_u64;
    let mut webhook_failed = 0_u64;
    let mut webhook_delivered = 0_u64;

    for row in outbox_rows {
        let status: String = row.get("status");
        let total = i64_to_u64(row.get("total"));
        match status.as_str() {
            "pending" | "processing" => webhook_pending = webhook_pending.saturating_add(total),
            "failed" => webhook_failed = total,
            "delivered" => webhook_delivered = total,
            _ => {}
        }
    }

    Ok(DashboardSummaryV1 {
        from,
        to,
        events_total: i64_to_u64(row.get("events_total")),
        sessions_started: i64_to_u64(row.get("sessions_started")),
        sessions_ended: i64_to_u64(row.get("sessions_ended")),
        active_sessions: i64_to_u64(active_sessions_row.get("active_sessions")),
        webhook_pending,
        webhook_failed,
        webhook_delivered,
    })
}

pub async fn query_timeline_events(
    pool: &SqlitePool,
    filter: &EventQueryFilter,
    page: u64,
    page_size: u64,
) -> anyhow::Result<(Vec<SessionTimelineItemV1>, u64)> {
    let page_size = page_size.clamp(1, 200);
    let offset = page.saturating_sub(1).saturating_mul(page_size);

    let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS total FROM session_events WHERE 1=1");
    apply_event_filters(&mut count_qb, filter);
    let total_raw: i64 = count_qb
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .context("failed to count filtered timeline events")?;
    let total = i64_to_u64(total_raw);

    let mut data_qb = QueryBuilder::<Sqlite>::new("SELECT payload FROM session_events WHERE 1=1");
    apply_event_filters(&mut data_qb, filter);
    data_qb.push(" ORDER BY timestamp DESC LIMIT ");
    data_qb.push_bind(page_size as i64);
    data_qb.push(" OFFSET ");
    data_qb.push_bind(offset as i64);

    let rows = data_qb
        .build()
        .fetch_all(pool)
        .await
        .context("failed to query filtered timeline events")?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let payload: String = row.get("payload");
        let event: SessionEventV1 = serde_json::from_str(&payload)
            .context("failed to deserialize event payload from session_events")?;
        items.push(event_to_timeline_item(event));
    }

    Ok((items, total))
}

pub async fn query_session_timeline(
    pool: &SqlitePool,
    session_id: &str,
    page: u64,
    page_size: u64,
) -> anyhow::Result<(Vec<SessionTimelineItemV1>, u64)> {
    let filter = EventQueryFilter {
        session_id: Some(session_id.to_string()),
        ..Default::default()
    };
    query_timeline_events(pool, &filter, page, page_size).await
}

pub async fn query_session_report_rows(
    pool: &SqlitePool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    user_id: Option<&str>,
) -> anyhow::Result<Vec<SessionReportRowV1>> {
    let mut qb = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            session_id,
            MIN(timestamp) AS started_at,
            MAX(timestamp) AS last_event_at,
            COUNT(*) AS events_total,
            GROUP_CONCAT(DISTINCT user_id) AS users
        FROM session_events
        WHERE timestamp >= "#,
    );
    qb.push_bind(from.to_rfc3339());
    qb.push(" AND timestamp <= ");
    qb.push_bind(to.to_rfc3339());

    if let Some(user_id) = user_id {
        qb.push(" AND user_id = ");
        qb.push_bind(user_id.trim().to_string());
    }

    qb.push(" GROUP BY session_id ORDER BY started_at DESC");

    let rows = qb
        .build()
        .fetch_all(pool)
        .await
        .context("failed to query session report rows")?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let users_raw: Option<String> = row.get("users");
        let mut users = users_raw
            .unwrap_or_default()
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        users.sort();
        users.dedup();

        let started_at_str: String = row.get("started_at");
        let last_event_at_str: String = row.get("last_event_at");

        out.push(SessionReportRowV1 {
            session_id: row.get("session_id"),
            started_at: parse_rfc3339_to_utc(&started_at_str),
            last_event_at: parse_rfc3339_to_utc(&last_event_at_str),
            events_total: i64_to_u64(row.get("events_total")),
            users,
        });
    }

    Ok(out)
}

pub fn unix_millis_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn apply_presence_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &SessionEventV1,
    now_ms: i64,
) -> anyhow::Result<()> {
    let event_ms = event.timestamp.timestamp_millis().max(0);

    match event.event_type {
        SessionEventType::ParticipantJoined => {
            let actor = extract_presence_actor(event);
            upsert_participant_presence(
                tx,
                &event.session_id,
                &actor,
                Some(true),
                None,
                event_ms,
                now_ms,
            )
            .await?;
        }
        SessionEventType::ParticipantLeft => {
            let actor = extract_presence_actor(event);
            upsert_participant_presence(
                tx,
                &event.session_id,
                &actor,
                Some(false),
                Some(false),
                event_ms,
                now_ms,
            )
            .await?;
        }
        SessionEventType::ControlChanged => {
            let actor = extract_presence_actor(event);
            let is_control_active = meta_bool(event.meta.as_ref(), "is_control_active").unwrap_or(true);
            sqlx::query(
                r#"
                UPDATE session_presence
                SET is_control_active = 0, updated_at = ?2
                WHERE session_id = ?1 AND is_control_active = 1
                "#,
            )
            .bind(&event.session_id)
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .with_context(|| {
                format!(
                    "failed to clear previous control owner for session {}",
                    event.session_id
                )
            })?;

            upsert_participant_presence(
                tx,
                &event.session_id,
                &actor,
                Some(true),
                Some(is_control_active),
                event_ms,
                now_ms,
            )
            .await?;
        }
        SessionEventType::ParticipantActivity => {
            let actor = extract_presence_actor(event);
            upsert_participant_presence(
                tx,
                &event.session_id,
                &actor,
                Some(true),
                None,
                event_ms,
                now_ms,
            )
            .await?;
        }
        SessionEventType::SessionEnded => {
            sqlx::query(
                r#"
                UPDATE session_presence
                SET is_active = 0, is_control_active = 0, updated_at = ?2
                WHERE session_id = ?1
                "#,
            )
            .bind(&event.session_id)
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("failed to close presence for session {}", event.session_id))?;
        }
        SessionEventType::SessionStarted
        | SessionEventType::RecordingStarted
        | SessionEventType::RecordingStopped => {}
    }

    Ok(())
}

async fn upsert_participant_presence(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    actor: &PresenceActor,
    is_active: Option<bool>,
    is_control_active: Option<bool>,
    last_activity_ms: i64,
    updated_ms: i64,
) -> anyhow::Result<()> {
    let display_name_insert = actor
        .display_name_override
        .as_deref()
        .unwrap_or(&actor.participant_id);

    sqlx::query(
        r#"
        INSERT INTO session_presence (
            session_id,
            participant_id,
            display_name,
            avatar_url,
            is_active,
            is_control_active,
            last_activity_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(session_id, participant_id) DO UPDATE SET
            display_name = CASE
                WHEN ?9 IS NULL OR TRIM(?9) = '' THEN session_presence.display_name
                ELSE ?9
            END,
            avatar_url = COALESCE(?10, session_presence.avatar_url),
            is_active = COALESCE(?11, session_presence.is_active),
            is_control_active = COALESCE(?12, session_presence.is_control_active),
            last_activity_at = MAX(session_presence.last_activity_at, ?13),
            updated_at = ?14
        "#,
    )
    .bind(session_id)
    .bind(&actor.participant_id)
    .bind(display_name_insert)
    .bind(actor.avatar_url.as_deref())
    .bind(is_active.unwrap_or(true) as i64)
    .bind(is_control_active.unwrap_or(false) as i64)
    .bind(last_activity_ms)
    .bind(updated_ms)
    .bind(actor.display_name_override.as_deref())
    .bind(actor.avatar_url.as_deref())
    .bind(is_active.map(|flag| flag as i64))
    .bind(is_control_active.map(|flag| flag as i64))
    .bind(last_activity_ms)
    .bind(updated_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| {
        format!(
            "failed to upsert presence for session {} participant {}",
            session_id, actor.participant_id
        )
    })?;

    Ok(())
}

fn extract_presence_actor(event: &SessionEventV1) -> PresenceActor {
    let participant_id = meta_string(event.meta.as_ref(), "participant_id").unwrap_or_else(|| event.user_id.clone());
    let display_name_override = meta_string(event.meta.as_ref(), "display_name");
    let avatar_url = meta_string(event.meta.as_ref(), "avatar_url")
        .or_else(|| meta_string(event.meta.as_ref(), "avatar"));

    PresenceActor {
        participant_id,
        display_name_override,
        avatar_url,
    }
}

fn meta_string(meta: Option<&Value>, key: &str) -> Option<String> {
    let raw = meta?.get(key)?.as_str()?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

fn meta_bool(meta: Option<&Value>, key: &str) -> Option<bool> {
    meta?.get(key)?.as_bool()
}

fn role_from_db(raw: &str) -> AuthRoleV1 {
    match raw.trim().to_ascii_lowercase().as_str() {
        "supervisor" => AuthRoleV1::Supervisor,
        _ => AuthRoleV1::Supervisor,
    }
}

fn apply_event_filters(qb: &mut QueryBuilder<'_, Sqlite>, filter: &EventQueryFilter) {
    if let Some(session_id) = filter.session_id.as_ref() {
        let session_id = session_id.trim();
        if !session_id.is_empty() {
            qb.push(" AND session_id = ");
            qb.push_bind(session_id.to_string());
        }
    }
    if let Some(user_id) = filter.user_id.as_ref() {
        let user_id = user_id.trim();
        if !user_id.is_empty() {
            qb.push(" AND user_id = ");
            qb.push_bind(user_id.to_string());
        }
    }
    if let Some(event_type) = filter.event_type {
        qb.push(" AND event_type = ");
        qb.push_bind(event_type.as_str().to_string());
    }
    if let Some(from) = filter.from.as_ref() {
        qb.push(" AND timestamp >= ");
        qb.push_bind(from.to_rfc3339());
    }
    if let Some(to) = filter.to.as_ref() {
        qb.push(" AND timestamp <= ");
        qb.push_bind(to.to_rfc3339());
    }
}

fn event_to_timeline_item(event: SessionEventV1) -> SessionTimelineItemV1 {
    SessionTimelineItemV1 {
        event_id: event.event_id,
        event_type: event.event_type,
        session_id: event.session_id,
        user_id: event.user_id,
        direction: event.direction,
        timestamp: event.timestamp,
        host_info: event.host_info,
        meta: event.meta,
    }
}

fn parse_rfc3339_to_utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| millis_to_utc(0))
}

fn i64_to_bool(value: i64) -> bool {
    value != 0
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}

fn millis_to_utc(value: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch should exist"))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use tempfile::tempdir;
    use uuid::Uuid;

    use crate::model::{SessionDirection, SessionEventType, SessionEventV1};

    use super::{
        connect_sqlite, expire_stale_presence, get_session_presence, insert_event, list_active_session_presence,
        unix_millis_now, InsertOutcome,
    };

    #[tokio::test]
    async fn duplicate_event_id_is_rejected() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("outbox.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let event = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::SessionStarted,
            session_id: "sess-001".to_string(),
            user_id: "alice".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: None,
        };

        let first = insert_event(&pool, &event).await.expect("first insert");
        let second = insert_event(&pool, &event).await.expect("second insert");

        assert_eq!(first, InsertOutcome::Inserted);
        assert_eq!(second, InsertOutcome::Duplicate);
    }

    #[tokio::test]
    async fn presence_lifecycle_is_updated_from_events() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("presence.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let session_id = "sess-presence-1";

        let joined = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantJoined,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({
                "participant_id": "alice",
                "display_name": "Alice",
                "avatar_url": "https://cdn.example/alice.png"
            })),
        };

        let control = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ControlChanged,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({ "participant_id": "alice" })),
        };

        let left = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantLeft,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({ "participant_id": "alice" })),
        };

        insert_event(&pool, &joined).await.expect("insert joined");
        insert_event(&pool, &control).await.expect("insert control");

        let snapshot = get_session_presence(&pool, session_id)
            .await
            .expect("read presence")
            .expect("presence exists");

        assert_eq!(snapshot.control_participant_id.as_deref(), Some("alice"));
        assert_eq!(snapshot.participants.len(), 1);
        assert!(snapshot.participants[0].is_active);
        assert!(snapshot.participants[0].is_control_active);

        insert_event(&pool, &left).await.expect("insert left");

        let after_leave = get_session_presence(&pool, session_id)
            .await
            .expect("read after leave")
            .expect("presence still stored");
        assert_eq!(after_leave.control_participant_id, None);
        assert!(!after_leave.participants[0].is_active);
        assert!(!after_leave.participants[0].is_control_active);

        let active_sessions = list_active_session_presence(&pool)
            .await
            .expect("active summary");
        assert!(active_sessions.is_empty());
    }

    #[tokio::test]
    async fn stale_presence_is_expired() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("presence-expiry.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let session_id = "sess-presence-expiry";
        let joined = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantJoined,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({
                "participant_id": "ghost-user",
                "display_name": "Ghost User"
            })),
        };

        insert_event(&pool, &joined).await.expect("insert joined");

        let before = list_active_session_presence(&pool)
            .await
            .expect("active sessions before cleanup");
        assert_eq!(before.len(), 1);

        let stale_before_ms = i64::MAX;
        let now_ms = unix_millis_now() as i64;
        let (expired_rows, touched_sessions) = expire_stale_presence(&pool, stale_before_ms, now_ms)
            .await
            .expect("expire stale presence");

        assert_eq!(expired_rows, 1);
        assert_eq!(touched_sessions, vec![session_id.to_string()]);

        let after = get_session_presence(&pool, session_id)
            .await
            .expect("read presence after expiry")
            .expect("presence row still exists");
        assert_eq!(after.control_participant_id, None);
        assert_eq!(after.participants.len(), 1);
        assert!(!after.participants[0].is_active);
        assert!(!after.participants[0].is_control_active);
    }

    #[tokio::test]
    async fn control_changed_false_clears_active_controller() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("presence-control.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let session_id = "sess-presence-control";
        let joined = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantJoined,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({
                "participant_id": "alice",
                "display_name": "Alice"
            })),
        };

        let control_on = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ControlChanged,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({
                "participant_id": "alice",
                "is_control_active": true
            })),
        };

        let control_off = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ControlChanged,
            session_id: session_id.to_string(),
            user_id: "operator".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: Some(json!({
                "participant_id": "alice",
                "is_control_active": false
            })),
        };

        insert_event(&pool, &joined).await.expect("insert joined");
        insert_event(&pool, &control_on)
            .await
            .expect("insert control on");
        insert_event(&pool, &control_off)
            .await
            .expect("insert control off");

        let snapshot = get_session_presence(&pool, session_id)
            .await
            .expect("read presence")
            .expect("presence exists");

        assert_eq!(snapshot.control_participant_id, None);
        assert_eq!(snapshot.participants.len(), 1);
        assert!(snapshot.participants[0].is_active);
        assert!(!snapshot.participants[0].is_control_active);
    }
}
