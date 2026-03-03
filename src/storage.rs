use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::model::{
    PresenceParticipantV1, PresenceSessionSummaryV1, SessionEventType, SessionEventV1,
    SessionPresenceV1,
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
                Some(true),
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

fn i64_to_bool(value: i64) -> bool {
    value != 0
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
        connect_sqlite, get_session_presence, insert_event, list_active_session_presence, InsertOutcome,
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
}
