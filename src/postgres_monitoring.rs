use anyhow::Context;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::model::SessionEventType;
use crate::model::SessionEventV1;
use crate::storage::{unix_millis_now, InsertOutcome};

const POSTGRES_MONITORING_SCHEMA: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS outbox_events (
        event_id TEXT PRIMARY KEY,
        payload TEXT NOT NULL,
        status TEXT NOT NULL,
        attempts BIGINT NOT NULL DEFAULT 0,
        next_attempt_at BIGINT NOT NULL,
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        last_error TEXT
    )
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_outbox_status_next_attempt
    ON outbox_events(status, next_attempt_at)
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS session_events (
        event_id TEXT PRIMARY KEY,
        event_type TEXT NOT NULL,
        session_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        timestamp TEXT NOT NULL,
        payload TEXT NOT NULL,
        created_at BIGINT NOT NULL
    )
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_session_events_timestamp
    ON session_events(timestamp DESC)
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_session_events_session_timestamp
    ON session_events(session_id, timestamp DESC)
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_session_events_user_timestamp
    ON session_events(user_id, timestamp DESC)
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_session_events_type_timestamp
    ON session_events(event_type, timestamp DESC)
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS session_presence (
        session_id TEXT NOT NULL,
        participant_id TEXT NOT NULL,
        display_name TEXT NOT NULL,
        avatar_url TEXT,
        is_active BOOLEAN NOT NULL DEFAULT TRUE,
        is_control_active BOOLEAN NOT NULL DEFAULT FALSE,
        last_activity_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL,
        PRIMARY KEY(session_id, participant_id)
    )
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_session_presence_active
    ON session_presence(session_id, is_active)
    "#,
    r#"
    CREATE INDEX IF NOT EXISTS idx_session_presence_updated
    ON session_presence(updated_at)
    "#,
];

#[derive(Debug, Clone, Default)]
pub struct PostgresMonitoringCounts {
    pub outbox_events: u64,
    pub session_events: u64,
    pub session_presence: u64,
}

#[derive(Debug, Clone)]
struct PresenceActor {
    participant_id: String,
    display_name_override: Option<String>,
    avatar_url: Option<String>,
}

pub async fn init_postgres_monitoring_schema(pool: &PgPool) -> anyhow::Result<()> {
    for statement in POSTGRES_MONITORING_SCHEMA {
        sqlx::query(statement)
            .execute(pool)
            .await
            .context("failed to apply Postgres monitoring schema statement")?;
    }
    Ok(())
}

pub async fn insert_event_pg(
    pool: &PgPool,
    event: &SessionEventV1,
) -> anyhow::Result<InsertOutcome> {
    let payload = serde_json::to_string(event).context("failed to serialize event payload")?;
    let now_ms = unix_millis_now() as i64;

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres monitoring ingest transaction")?;

    let outbox_insert = sqlx::query(
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
        )
        VALUES ($1, $2, 'pending', 0, $3, $4, $5, NULL)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event.event_id.to_string())
    .bind(payload)
    .bind(now_ms)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .context("failed to insert event into Postgres outbox")?;

    if outbox_insert.rows_affected() == 0 {
        tx.rollback()
            .await
            .context("failed to rollback duplicate Postgres monitoring ingest transaction")?;
        return Ok(InsertOutcome::Duplicate);
    }

    let event_payload =
        serde_json::to_string(event).context("failed to serialize Postgres session event")?;
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
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
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
    .context("failed to insert Postgres session event")?;

    apply_presence_event_pg_tx(&mut tx, event, now_ms).await?;

    tx.commit()
        .await
        .context("failed to commit Postgres monitoring ingest transaction")?;

    Ok(InsertOutcome::Inserted)
}

pub async fn get_postgres_monitoring_counts(
    pool: &PgPool,
) -> anyhow::Result<PostgresMonitoringCounts> {
    Ok(PostgresMonitoringCounts {
        outbox_events: count_pg_table(pool, "outbox_events").await?,
        session_events: count_pg_table(pool, "session_events").await?,
        session_presence: count_pg_table(pool, "session_presence").await?,
    })
}

async fn count_pg_table(pool: &PgPool, table_name: &str) -> anyhow::Result<u64> {
    let row = sqlx::query(&format!("SELECT COUNT(*) AS total FROM {table_name}"))
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to count Postgres table '{table_name}'"))?;
    Ok(i64_to_u64(row.get("total")))
}

async fn apply_presence_event_pg_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &SessionEventV1,
    now_ms: i64,
) -> anyhow::Result<()> {
    let event_ms = event.timestamp.timestamp_millis().max(0);

    match event.event_type {
        SessionEventType::ParticipantJoined => {
            let actor = extract_presence_actor(event);
            upsert_participant_presence_pg(
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
            upsert_participant_presence_pg(
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
            let is_control_active =
                meta_bool(event.meta.as_ref(), "is_control_active").unwrap_or(true);

            sqlx::query(
                r#"
                UPDATE session_presence
                SET is_control_active = FALSE,
                    updated_at = $2
                WHERE session_id = $1
                  AND is_control_active = TRUE
                "#,
            )
            .bind(&event.session_id)
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .with_context(|| {
                format!(
                    "failed to clear previous Postgres control owner for session {}",
                    event.session_id
                )
            })?;

            upsert_participant_presence_pg(
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
            upsert_participant_presence_pg(
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
                SET is_active = FALSE,
                    is_control_active = FALSE,
                    updated_at = $2
                WHERE session_id = $1
                "#,
            )
            .bind(&event.session_id)
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .with_context(|| {
                format!(
                    "failed to close Postgres presence for session {}",
                    event.session_id
                )
            })?;
        }
        SessionEventType::SessionStarted
        | SessionEventType::RecordingStarted
        | SessionEventType::RecordingStopped => {}
    }

    Ok(())
}

async fn upsert_participant_presence_pg(
    tx: &mut Transaction<'_, Postgres>,
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
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (session_id, participant_id) DO UPDATE SET
            display_name = CASE
                WHEN $9 IS NULL OR trim($9) = '' THEN session_presence.display_name
                ELSE $9
            END,
            avatar_url = COALESCE($10, session_presence.avatar_url),
            is_active = COALESCE($11, session_presence.is_active),
            is_control_active = COALESCE($12, session_presence.is_control_active),
            last_activity_at = GREATEST(session_presence.last_activity_at, $13),
            updated_at = $14
        "#,
    )
    .bind(session_id)
    .bind(&actor.participant_id)
    .bind(display_name_insert)
    .bind(actor.avatar_url.as_deref())
    .bind(is_active.unwrap_or(true))
    .bind(is_control_active.unwrap_or(false))
    .bind(last_activity_ms)
    .bind(updated_ms)
    .bind(actor.display_name_override.as_deref())
    .bind(actor.avatar_url.as_deref())
    .bind(is_active)
    .bind(is_control_active)
    .bind(last_activity_ms)
    .bind(updated_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| {
        format!(
            "failed to upsert Postgres presence for session {} participant {}",
            session_id, actor.participant_id
        )
    })?;

    Ok(())
}

fn extract_presence_actor(event: &SessionEventV1) -> PresenceActor {
    let participant_id =
        meta_string(event.meta.as_ref(), "participant_id").unwrap_or_else(|| event.user_id.clone());
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

fn i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}
