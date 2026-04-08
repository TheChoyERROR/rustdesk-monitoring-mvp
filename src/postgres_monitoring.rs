use anyhow::Context;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder, Row, Transaction};

use crate::model::{
    DashboardSummaryV1, PresenceParticipantV1, PresenceSessionSummaryV1, SessionActorTypeV1,
    SessionEventType, SessionEventV1, SessionPresenceV1, SessionReportRowV1, SessionTimelineItemV1,
};
use crate::storage::{unix_millis_now, EventQueryFilter, InsertOutcome, OutboxRecord};

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

#[derive(Debug, Clone, Default)]
struct SessionActorReferenceIndex {
    agent_ids: std::collections::HashSet<String>,
    client_ids: std::collections::HashSet<String>,
}

impl SessionActorReferenceIndex {
    fn classify(&self, user_id: &str) -> SessionActorTypeV1 {
        let normalized_user_id = normalize_helpdesk_identity_id(user_id);
        if normalized_user_id.is_empty() {
            return SessionActorTypeV1::Unknown;
        }
        if self.agent_ids.contains(&normalized_user_id) {
            return SessionActorTypeV1::Agent;
        }
        if self.client_ids.contains(&normalized_user_id) {
            return SessionActorTypeV1::Client;
        }
        SessionActorTypeV1::Unknown
    }
}

pub async fn claim_due_events_pg(
    pool: &PgPool,
    limit: usize,
    now_ms: u64,
) -> anyhow::Result<Vec<OutboxRecord>> {
    let now_ms = now_ms as i64;
    let rows = sqlx::query(
        r#"
        WITH due AS (
            SELECT event_id
            FROM outbox_events
            WHERE status = 'pending'
              AND next_attempt_at <= $1
            ORDER BY created_at ASC
            LIMIT $2
            FOR UPDATE SKIP LOCKED
        )
        UPDATE outbox_events AS oe
        SET status = 'processing',
            updated_at = $1
        FROM due
        WHERE oe.event_id = due.event_id
        RETURNING oe.event_id, oe.payload, oe.attempts
        "#,
    )
    .bind(now_ms)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("failed to claim due Postgres outbox events")?;

    Ok(rows
        .into_iter()
        .map(|row| OutboxRecord {
            event_id: row.get("event_id"),
            payload: row.get("payload"),
            attempts: u32::try_from(row.get::<i64, _>("attempts")).unwrap_or(u32::MAX),
        })
        .collect())
}

pub async fn mark_delivered_pg(
    pool: &PgPool,
    event_id: &str,
    attempts: u32,
    now_ms: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET status = 'delivered',
            attempts = $1,
            updated_at = $2,
            last_error = NULL
        WHERE event_id = $3
        "#,
    )
    .bind(attempts as i64)
    .bind(now_ms as i64)
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark Postgres outbox event '{event_id}' delivered"))?;
    Ok(())
}

pub async fn schedule_retry_pg(
    pool: &PgPool,
    event_id: &str,
    attempts: u32,
    next_attempt_at_ms: u64,
    error_message: &str,
    now_ms: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET status = 'pending',
            attempts = $1,
            next_attempt_at = $2,
            updated_at = $3,
            last_error = $4
        WHERE event_id = $5
        "#,
    )
    .bind(attempts as i64)
    .bind(next_attempt_at_ms as i64)
    .bind(now_ms as i64)
    .bind(error_message)
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to schedule Postgres retry for event '{event_id}'"))?;
    Ok(())
}

pub async fn mark_failed_pg(
    pool: &PgPool,
    event_id: &str,
    attempts: u32,
    error_message: &str,
    now_ms: u64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET status = 'failed',
            attempts = $1,
            updated_at = $2,
            next_attempt_at = $3,
            last_error = $4
        WHERE event_id = $5
        "#,
    )
    .bind(attempts as i64)
    .bind(now_ms as i64)
    .bind(now_ms as i64)
    .bind(error_message)
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to mark Postgres outbox event '{event_id}' failed"))?;
    Ok(())
}

pub async fn reset_stuck_processing_pg(
    pool: &PgPool,
    older_than_ms: u64,
    now_ms: u64,
) -> anyhow::Result<u64> {
    let threshold = now_ms.saturating_sub(older_than_ms) as i64;
    let result = sqlx::query(
        r#"
        UPDATE outbox_events
        SET status = 'pending',
            updated_at = $1
        WHERE status = 'processing'
          AND updated_at <= $2
        "#,
    )
    .bind(now_ms as i64)
    .bind(threshold)
    .execute(pool)
    .await
    .context("failed to reset stuck Postgres processing rows")?;
    Ok(result.rows_affected())
}

pub async fn cleanup_failed_older_than_pg(pool: &PgPool, cutoff_ms: u64) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM outbox_events
        WHERE status = 'failed'
          AND updated_at < $1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to clean old failed Postgres outbox events")?;
    Ok(result.rows_affected())
}

pub async fn cleanup_delivered_older_than_pg(pool: &PgPool, cutoff_ms: u64) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM outbox_events
        WHERE status = 'delivered'
          AND updated_at < $1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to clean old delivered Postgres outbox events")?;
    Ok(result.rows_affected())
}

pub async fn cleanup_session_events_older_than_pg(
    pool: &PgPool,
    cutoff_ms: u64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM session_events
        WHERE created_at < $1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to clean old Postgres session events")?;
    Ok(result.rows_affected())
}

pub async fn cleanup_inactive_session_presence_older_than_pg(
    pool: &PgPool,
    cutoff_ms: u64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM session_presence
        WHERE updated_at < $1
          AND is_active = FALSE
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to clean stale inactive Postgres session presence rows")?;
    Ok(result.rows_affected())
}

pub async fn expire_stale_presence_pg(
    pool: &PgPool,
    stale_before_ms: i64,
    now_ms: i64,
) -> anyhow::Result<(u64, Vec<String>)> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT session_id
        FROM session_presence
        WHERE is_active = TRUE
          AND last_activity_at < $1
        "#,
    )
    .bind(stale_before_ms)
    .fetch_all(pool)
    .await
    .context("failed to query stale Postgres presence sessions")?;

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
        SET is_active = FALSE,
            is_control_active = FALSE,
            updated_at = $2
        WHERE is_active = TRUE
          AND last_activity_at < $1
        "#,
    )
    .bind(stale_before_ms)
    .bind(now_ms)
    .execute(pool)
    .await
    .context("failed to expire stale Postgres session presence rows")?;

    Ok((result.rows_affected(), touched_sessions))
}

pub async fn get_session_presence_pg(
    pool: &PgPool,
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
        WHERE session_id = $1
        ORDER BY is_active DESC, last_activity_at DESC, participant_id ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to query Postgres presence for session {session_id}"))?;

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
        let is_active: bool = row.get("is_active");
        let is_control_active: bool = row.get("is_control_active");
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

pub async fn list_active_session_presence_pg(
    pool: &PgPool,
) -> anyhow::Result<Vec<PresenceSessionSummaryV1>> {
    let rows = sqlx::query(
        r#"
        SELECT
            session_id,
            COUNT(*) AS active_participants,
            MAX(updated_at) AS updated_at
        FROM session_presence
        WHERE is_active = TRUE
        GROUP BY session_id
        ORDER BY updated_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query active Postgres session presence")?;

    Ok(rows
        .into_iter()
        .map(|row| PresenceSessionSummaryV1 {
            session_id: row.get("session_id"),
            active_participants: i64_to_u64(row.get("active_participants")),
            updated_at: millis_to_utc(row.get("updated_at")),
        })
        .collect())
}

pub async fn get_dashboard_summary_pg(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<DashboardSummaryV1> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    let from_ms = from.timestamp_millis();
    let to_ms = to.timestamp_millis();

    let row = sqlx::query(
        r#"
        SELECT
            COUNT(*) AS events_total,
            SUM(CASE WHEN event_type = 'session_started' THEN 1 ELSE 0 END) AS sessions_started,
            SUM(CASE WHEN event_type = 'session_ended' THEN 1 ELSE 0 END) AS sessions_ended
        FROM session_events
        WHERE timestamp >= $1
          AND timestamp <= $2
        "#,
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_one(pool)
    .await
    .context("failed to aggregate Postgres dashboard summary events")?;

    let active_sessions_row = sqlx::query(
        r#"
        SELECT COUNT(DISTINCT session_id) AS active_sessions
        FROM session_presence
        WHERE is_active = TRUE
        "#,
    )
    .fetch_one(pool)
    .await
    .context("failed to count active Postgres sessions")?;

    let outbox_rows = sqlx::query(
        r#"
        SELECT status, COUNT(*) AS total
        FROM outbox_events
        WHERE created_at >= $1
          AND created_at <= $2
        GROUP BY status
        "#,
    )
    .bind(from_ms)
    .bind(to_ms)
    .fetch_all(pool)
    .await
    .context("failed to aggregate Postgres outbox status counts")?;

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

pub async fn query_timeline_events_pg(
    pool: &PgPool,
    filter: &EventQueryFilter,
    page: u64,
    page_size: u64,
) -> anyhow::Result<(Vec<SessionTimelineItemV1>, u64)> {
    let page_size = page_size.clamp(1, 500);
    let offset = page.saturating_sub(1).saturating_mul(page_size);

    let mut count_qb =
        QueryBuilder::<Postgres>::new("SELECT COUNT(*) AS total FROM session_events WHERE 1=1");
    apply_event_filters_pg(&mut count_qb, filter);
    let total_raw: i64 = count_qb
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .context("failed to count filtered Postgres timeline events")?;
    let total = i64_to_u64(total_raw);

    let mut data_qb = QueryBuilder::<Postgres>::new("SELECT payload FROM session_events WHERE 1=1");
    apply_event_filters_pg(&mut data_qb, filter);
    data_qb.push(" ORDER BY timestamp DESC LIMIT ");
    data_qb.push_bind(page_size as i64);
    data_qb.push(" OFFSET ");
    data_qb.push_bind(offset as i64);

    let rows = data_qb
        .build()
        .fetch_all(pool)
        .await
        .context("failed to query filtered Postgres timeline events")?;

    if rows.is_empty() {
        return Ok((Vec::new(), total));
    }

    let actor_reference_index = load_session_actor_reference_index_pg(pool).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let payload: String = row.get("payload");
        let event: SessionEventV1 = serde_json::from_str(&payload)
            .context("failed to deserialize event payload from Postgres session_events")?;
        let actor_type = actor_reference_index.classify(&event.user_id);
        items.push(event_to_timeline_item(event, actor_type));
    }

    Ok((items, total))
}

pub async fn query_session_report_rows_pg(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    user_id: Option<&str>,
    actor_type: Option<SessionActorTypeV1>,
) -> anyhow::Result<Vec<SessionReportRowV1>> {
    let mut qb = QueryBuilder::<Postgres>::new(
        r#"
        SELECT
            session_id,
            MIN(timestamp) AS started_at,
            MAX(timestamp) AS last_event_at,
            COUNT(*) AS events_total,
            STRING_AGG(DISTINCT user_id, ',') AS users
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

    if let Some(actor_type) = actor_type {
        qb.push(" AND ");
        push_session_actor_type_match_clause_pg(&mut qb, actor_type, "session_events.user_id");
    }

    qb.push(" GROUP BY session_id ORDER BY started_at DESC");

    let rows = qb
        .build()
        .fetch_all(pool)
        .await
        .context("failed to query Postgres session report rows")?;

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

async fn load_session_actor_reference_index_pg(
    pool: &PgPool,
) -> anyhow::Result<SessionActorReferenceIndex> {
    let mut actor_reference_index = SessionActorReferenceIndex::default();

    let agent_ids = sqlx::query_scalar::<_, String>(
        r#"
        SELECT agent_id FROM helpdesk_authorized_agents
        UNION
        SELECT agent_id FROM helpdesk_agents
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query Postgres helpdesk agent ids for session actor classification")?;

    for agent_id in agent_ids {
        let normalized = normalize_helpdesk_identity_id(&agent_id);
        if !normalized.is_empty() {
            actor_reference_index.agent_ids.insert(normalized);
        }
    }

    let client_ids = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT client_id
        FROM helpdesk_tickets
        WHERE client_id IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query Postgres helpdesk client ids for session actor classification")?;

    for client_id in client_ids {
        let normalized = normalize_helpdesk_identity_id(&client_id);
        if !normalized.is_empty() {
            actor_reference_index.client_ids.insert(normalized);
        }
    }

    Ok(actor_reference_index)
}

fn apply_event_filters_pg(qb: &mut QueryBuilder<'_, Postgres>, filter: &EventQueryFilter) {
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
    if let Some(actor_type) = filter.actor_type {
        qb.push(" AND ");
        push_session_actor_type_match_clause_pg(qb, actor_type, "session_events.user_id");
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

fn push_session_actor_type_match_clause_pg(
    qb: &mut QueryBuilder<'_, Postgres>,
    actor_type: SessionActorTypeV1,
    user_id_expr: &str,
) {
    match actor_type {
        SessionActorTypeV1::Agent => {
            push_helpdesk_agent_match_clause_pg(qb, user_id_expr);
        }
        SessionActorTypeV1::Client => {
            push_helpdesk_client_match_clause_pg(qb, user_id_expr);
        }
        SessionActorTypeV1::Unknown => {
            qb.push("NOT (");
            push_helpdesk_agent_match_clause_pg(qb, user_id_expr);
            qb.push(") AND NOT (");
            push_helpdesk_client_match_clause_pg(qb, user_id_expr);
            qb.push(")");
        }
    }
}

fn push_helpdesk_agent_match_clause_pg(qb: &mut QueryBuilder<'_, Postgres>, user_id_expr: &str) {
    let normalized_user_expr = format!("regexp_replace(trim({user_id_expr}), '\\s+', '', 'g')");
    qb.push(format!(
        "(EXISTS (SELECT 1 FROM helpdesk_authorized_agents haa WHERE regexp_replace(trim(haa.agent_id), '\\s+', '', 'g') = {normalized_user_expr}) \
OR EXISTS (SELECT 1 FROM helpdesk_agents ha WHERE regexp_replace(trim(ha.agent_id), '\\s+', '', 'g') = {normalized_user_expr}))"
    ));
}

fn push_helpdesk_client_match_clause_pg(qb: &mut QueryBuilder<'_, Postgres>, user_id_expr: &str) {
    let normalized_user_expr = format!("regexp_replace(trim({user_id_expr}), '\\s+', '', 'g')");
    qb.push(format!(
        "EXISTS (SELECT 1 FROM helpdesk_tickets ht WHERE regexp_replace(trim(ht.client_id), '\\s+', '', 'g') = {normalized_user_expr})"
    ));
}

fn normalize_helpdesk_identity_id(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

fn event_to_timeline_item(
    event: SessionEventV1,
    actor_type: SessionActorTypeV1,
) -> SessionTimelineItemV1 {
    SessionTimelineItemV1 {
        event_id: event.event_id,
        event_type: event.event_type,
        session_id: event.session_id,
        user_id: event.user_id,
        actor_type,
        direction: event.direction,
        timestamp: event.timestamp,
        host_info: event.host_info,
        meta: event.meta,
    }
}

fn parse_rfc3339_to_utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .unwrap_or_else(|_| {
            Utc.timestamp_millis_opt(0)
                .single()
                .unwrap_or_else(Utc::now)
        })
}

fn millis_to_utc(value: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or_else(Utc::now)
}
