use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::config::MonitoringConfig;
use crate::helpdesk_agent_auth::{
    generate_helpdesk_agent_token, hash_helpdesk_agent_token, helpdesk_agent_token_hint,
};
use crate::model::{
    AuthRoleV1, AuthUserV1, DashboardSummaryV1, HelpdeskAgentAuthorizationStatusV1,
    HelpdeskAgentPresenceUpdateV1, HelpdeskAgentStatus, HelpdeskAgentV1, HelpdeskAssignmentV1,
    HelpdeskAuditEventV1, HelpdeskAuthorizedAgentProvisioningV1,
    HelpdeskAuthorizedAgentUpsertRequestV1, HelpdeskAuthorizedAgentV1,
    HelpdeskOperationalSummaryV1, HelpdeskTicketCreateRequestV1, HelpdeskTicketStatus,
    HelpdeskTicketV1, PresenceParticipantV1, PresenceSessionSummaryV1, SessionActorTypeV1,
    SessionEventType, SessionEventV1, SessionPresenceV1, SessionReportRowV1, SessionTimelineItemV1,
};
use crate::schema::init_sqlite_schema;

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

#[derive(Debug, Clone, Default)]
struct SessionActorReferenceIndex {
    agent_ids: HashSet<String>,
    client_ids: HashSet<String>,
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
    pub actor_type: Option<SessionActorTypeV1>,
    pub event_type: Option<SessionEventType>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct HelpdeskRuntimeReconcileResult {
    pub opening_timeouts: u64,
    pub agents_marked_offline: u64,
    pub tickets_requeued: u64,
    pub tickets_failed: u64,
}

const HELPDESK_OPENING_WINDOW_MS: i64 = 30_000;

pub async fn connect_sqlite(database_path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = database_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create database directory: {}", parent.display())
            })?;
        }
    }

    let url = format!("sqlite://{}", database_path.display());
    let options = url
        .parse::<SqliteConnectOptions>()
        .with_context(|| {
            format!(
                "invalid SQLite URL generated from path: {}",
                database_path.display()
            )
        })?
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
    init_sqlite_schema(pool).await
}

pub async fn insert_event(
    pool: &SqlitePool,
    event: &SessionEventV1,
) -> anyhow::Result<InsertOutcome> {
    let payload = serde_json::to_string(event).context("failed to serialize event payload")?;
    let now_ms = unix_millis_now() as i64;

    let mut tx = pool
        .begin()
        .await
        .context("failed to open ingest transaction")?;

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
            let event_payload =
                serde_json::to_string(event).context("failed to serialize session event")?;
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
            tx.commit()
                .await
                .context("failed to commit ingest transaction")?;
            Ok(InsertOutcome::Inserted)
        }
        Err(sqlx::Error::Database(db_error)) if db_error.is_unique_violation() => {
            let _ = tx.rollback().await;
            Ok(InsertOutcome::Duplicate)
        }
        Err(err) => Err(err).context("failed to insert event into outbox"),
    }
}

pub async fn should_store_session_event(
    pool: &SqlitePool,
    event: &SessionEventV1,
    monitoring: &MonitoringConfig,
) -> anyhow::Result<bool> {
    if monitoring.capture_non_agent_events {
        return should_store_participant_activity_for_monitoring(pool, event, monitoring).await;
    }

    if !is_known_helpdesk_agent_id(pool, &event.user_id).await? {
        return Ok(false);
    }

    if !helpdesk_agent_has_active_ticket(pool, &event.user_id).await? {
        return Ok(false);
    }

    should_store_participant_activity_for_monitoring(pool, event, monitoring).await
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

pub async fn cleanup_delivered_older_than(
    pool: &SqlitePool,
    cutoff_ms: u64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM outbox_events
        WHERE status = 'delivered' AND updated_at < ?1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to delete old delivered outbox events")?;

    Ok(result.rows_affected())
}

pub async fn delete_outbox_event(pool: &SqlitePool, event_id: &str) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM outbox_events
        WHERE event_id = ?1
        "#,
    )
    .bind(event_id)
    .execute(pool)
    .await
    .with_context(|| format!("failed to delete outbox event {event_id}"))?;

    Ok(result.rows_affected())
}

pub async fn cleanup_session_events_older_than(
    pool: &SqlitePool,
    cutoff_ms: u64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM session_events
        WHERE created_at < ?1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to delete old session events")?;

    Ok(result.rows_affected())
}

pub async fn cleanup_inactive_session_presence_older_than(
    pool: &SqlitePool,
    cutoff_ms: u64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM session_presence
        WHERE updated_at < ?1
          AND is_active = 0
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to delete stale inactive session presence rows")?;

    Ok(result.rows_affected())
}

pub async fn cleanup_helpdesk_agent_heartbeats_older_than(
    pool: &SqlitePool,
    cutoff_ms: u64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM helpdesk_agent_heartbeats
        WHERE created_at < ?1
        "#,
    )
    .bind(cutoff_ms as i64)
    .execute(pool)
    .await
    .context("failed to delete stale helpdesk agent heartbeats")?;

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

pub async fn list_active_session_presence(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<PresenceSessionSummaryV1>> {
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

pub async fn delete_dashboard_session(
    pool: &SqlitePool,
    session_token: &str,
) -> anyhow::Result<()> {
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

pub async fn upsert_helpdesk_agent_presence(
    pool: &SqlitePool,
    payload: &HelpdeskAgentPresenceUpdateV1,
) -> anyhow::Result<HelpdeskAgentV1> {
    let now_ms = unix_millis_now() as i64;
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    if !is_helpdesk_agent_authorized(pool, &agent_id).await? {
        anyhow::bail!(
            "agent '{}' is not authorized for helpdesk operator mode",
            agent_id
        );
    }
    let authorized_agent = get_helpdesk_authorized_agent(pool, &agent_id).await?;
    let fallback_display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&agent_id)
        .to_string();
    let display_name = authorized_agent
        .as_ref()
        .and_then(|agent| agent.display_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or(fallback_display_name);
    ensure_helpdesk_agent_display_name_available(pool, &agent_id, Some(&display_name)).await?;
    let avatar_url = payload
        .avatar_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk agent transaction")?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_agents (
            agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7)
        ON CONFLICT(agent_id) DO UPDATE SET
            display_name = CASE
                WHEN TRIM(?2) = '' THEN helpdesk_agents.display_name
                ELSE ?2
            END,
            avatar_url = COALESCE(?3, helpdesk_agents.avatar_url),
            status = CASE
                WHEN helpdesk_agents.status IN ('opening', 'busy')
                     AND helpdesk_agents.current_ticket_id IS NOT NULL
                     AND ?4 IN ('available', 'away', 'offline')
                THEN helpdesk_agents.status
                ELSE ?4
            END,
            current_ticket_id = CASE
                WHEN helpdesk_agents.status IN ('opening', 'busy')
                     AND helpdesk_agents.current_ticket_id IS NOT NULL
                THEN helpdesk_agents.current_ticket_id
                WHEN ?4 IN ('available', 'away', 'offline') THEN NULL
                ELSE helpdesk_agents.current_ticket_id
            END,
            last_heartbeat_at = ?5,
            updated_at = ?7
        "#,
    )
    .bind(&agent_id)
    .bind(&display_name)
    .bind(avatar_url.as_deref())
    .bind(payload.status.as_str())
    .bind(now_ms)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to upsert helpdesk agent '{}'", agent_id))?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_agent_heartbeats (agent_id, status, created_at)
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(&agent_id)
    .bind(payload.status.as_str())
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to insert agent heartbeat '{}'", agent_id))?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "agent",
        &agent_id,
        "agent_presence_updated",
        Some(serde_json::json!({
            "status": payload.status.as_str(),
            "display_name": display_name,
            "has_avatar": avatar_url.is_some(),
        })),
        now_ms,
    )
    .await?;

    let agent = get_helpdesk_agent_tx(&mut tx, &agent_id)
        .await?
        .with_context(|| format!("helpdesk agent '{}' not found after upsert", agent_id))?;

    tx.commit()
        .await
        .context("failed to commit helpdesk agent transaction")?;
    Ok(agent)
}

pub async fn upsert_helpdesk_authorized_agent(
    pool: &SqlitePool,
    payload: &HelpdeskAuthorizedAgentUpsertRequestV1,
) -> anyhow::Result<HelpdeskAuthorizedAgentV1> {
    let now_ms = unix_millis_now() as i64;
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    let existing_agent = get_helpdesk_authorized_agent(pool, &agent_id).await?;
    let display_name = normalize_optional_text(payload.display_name.as_deref())
        .or_else(|| existing_agent.and_then(|agent| agent.display_name));
    ensure_helpdesk_agent_display_name_available(pool, &agent_id, display_name.as_deref()).await?;

    let mut tx = pool
        .begin()
        .await
        .context("failed to open authorized helpdesk agent transaction")?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_authorized_agents (agent_id, display_name, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(agent_id) DO UPDATE SET
            display_name = COALESCE(?2, helpdesk_authorized_agents.display_name),
            updated_at = ?4
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to upsert authorized helpdesk agent '{}'", agent_id))?;

    sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET display_name = ?2,
            updated_at = ?3
        WHERE agent_id = ?1
          AND ?2 IS NOT NULL
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to align live helpdesk agent display name for '{}'",
            agent_id
        )
    })?;

    tx.commit()
        .await
        .context("failed to commit authorized helpdesk agent transaction")?;

    delete_legacy_helpdesk_authorized_agent_variants(pool, &agent_id).await?;

    get_helpdesk_authorized_agent(pool, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "authorized helpdesk agent '{}' not found after upsert",
                agent_id
            )
        })
}

pub async fn provision_helpdesk_authorized_agent(
    pool: &SqlitePool,
    payload: &HelpdeskAuthorizedAgentUpsertRequestV1,
) -> anyhow::Result<HelpdeskAuthorizedAgentProvisioningV1> {
    let now_ms = unix_millis_now() as i64;
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    let existing_agent = get_helpdesk_authorized_agent(pool, &agent_id).await?;
    let display_name = normalize_optional_text(payload.display_name.as_deref()).or_else(|| {
        existing_agent
            .as_ref()
            .and_then(|agent| agent.display_name.clone())
    });
    ensure_helpdesk_agent_display_name_available(pool, &agent_id, display_name.as_deref()).await?;

    let issued_token = generate_helpdesk_agent_token();
    let token_hash = hash_helpdesk_agent_token(&issued_token);
    let token_hint = helpdesk_agent_token_hint(&issued_token);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open authorized helpdesk agent provisioning transaction")?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_authorized_agents (
            agent_id,
            display_name,
            agent_token_hash,
            agent_token_hint,
            agent_token_rotated_at,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(agent_id) DO UPDATE SET
            display_name = COALESCE(?2, helpdesk_authorized_agents.display_name),
            agent_token_hash = ?3,
            agent_token_hint = ?4,
            agent_token_rotated_at = ?5,
            updated_at = ?7
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(&token_hash)
    .bind(token_hint.as_deref())
    .bind(now_ms)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to provision authorized helpdesk agent '{}'",
            agent_id
        )
    })?;

    sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET display_name = ?2,
            updated_at = ?3
        WHERE agent_id = ?1
          AND ?2 IS NOT NULL
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to align live helpdesk agent display name for '{}'",
            agent_id
        )
    })?;

    tx.commit()
        .await
        .context("failed to commit authorized helpdesk agent provisioning transaction")?;

    delete_legacy_helpdesk_authorized_agent_variants(pool, &agent_id).await?;

    let agent = get_helpdesk_authorized_agent(pool, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "authorized helpdesk agent '{}' not found after provisioning",
                agent_id
            )
        })?;

    Ok(HelpdeskAuthorizedAgentProvisioningV1 {
        agent,
        agent_token: issued_token,
    })
}

pub async fn list_helpdesk_authorized_agents(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<HelpdeskAuthorizedAgentV1>> {
    let rows = sqlx::query(
        r#"
        SELECT
            agent_id,
            display_name,
            agent_token_hash,
            agent_token_hint,
            agent_token_rotated_at,
            created_at,
            updated_at
        FROM helpdesk_authorized_agents
        ORDER BY updated_at DESC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list authorized helpdesk agents")?;

    let mut normalized_ids = HashSet::new();
    let mut agents = Vec::new();
    for row in rows {
        let agent = row_to_helpdesk_authorized_agent(row)?;
        if normalized_ids.insert(agent.agent_id.clone()) {
            agents.push(agent);
        }
    }
    Ok(agents)
}

pub async fn get_helpdesk_authorized_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Option<HelpdeskAuthorizedAgentV1>> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let normalized_sql = normalized_helpdesk_agent_id_sql("agent_id");
    let query = format!(
        r#"
        SELECT
            agent_id,
            display_name,
            agent_token_hash,
            agent_token_hint,
            agent_token_rotated_at,
            created_at,
            updated_at
        FROM helpdesk_authorized_agents
        WHERE {normalized_sql} = ?1
        ORDER BY updated_at DESC, agent_id ASC
        LIMIT 1
        "#
    );
    let row = sqlx::query(&query)
        .bind(&agent_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("failed to query authorized helpdesk agent '{}'", agent_id))?;

    row.map(row_to_helpdesk_authorized_agent).transpose()
}

pub async fn get_helpdesk_agent_authorization_status(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<HelpdeskAgentAuthorizationStatusV1> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let authorized_agent = get_helpdesk_authorized_agent(pool, &agent_id).await?;
    Ok(HelpdeskAgentAuthorizationStatusV1 {
        agent_id,
        authorized: authorized_agent.is_some(),
        display_name: authorized_agent
            .as_ref()
            .and_then(|agent| agent.display_name.clone()),
        token_configured: authorized_agent
            .as_ref()
            .is_some_and(|agent| agent.token_configured),
        token_hint: authorized_agent.and_then(|agent| agent.token_hint),
    })
}

pub async fn verify_helpdesk_agent_token(
    pool: &SqlitePool,
    agent_id: &str,
    raw_token: &str,
) -> anyhow::Result<bool> {
    let normalized_agent_id = normalize_helpdesk_agent_id(agent_id);
    let token_hash = hash_helpdesk_agent_token(raw_token);
    let normalized_sql = normalized_helpdesk_agent_id_sql("agent_id");
    let query = format!(
        r#"
        SELECT agent_token_hash
        FROM helpdesk_authorized_agents
        WHERE {normalized_sql} = ?1
        ORDER BY updated_at DESC, agent_id ASC
        LIMIT 1
        "#
    );

    let stored_hash = sqlx::query_scalar::<_, Option<String>>(&query)
        .bind(&normalized_agent_id)
        .fetch_optional(pool)
        .await
        .with_context(|| {
            format!(
                "failed to query helpdesk agent token hash for '{}'",
                normalized_agent_id
            )
        })?
        .flatten();

    Ok(matches!(stored_hash.as_deref(), Some(value) if value == token_hash))
}

pub async fn delete_helpdesk_authorized_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<bool> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to open delete authorized helpdesk agent transaction")?;
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let now_ms = unix_millis_now() as i64;

    let current_ticket_id = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT current_ticket_id
        FROM helpdesk_agents
        WHERE agent_id = ?1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to query current ticket before deleting authorized helpdesk agent '{}'",
            agent_id
        )
    })?
    .flatten();

    let normalized_sql = normalized_helpdesk_agent_id_sql("agent_id");
    let delete_query = format!(
        r#"
        DELETE FROM helpdesk_authorized_agents
        WHERE {normalized_sql} = ?1
        "#
    );
    let result = sqlx::query(&delete_query)
        .bind(&agent_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to delete authorized helpdesk agent '{}'", agent_id))?;

    sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = 'offline',
            current_ticket_id = NULL,
            updated_at = ?2
        WHERE agent_id = ?1
        "#,
    )
    .bind(&agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to offboard live helpdesk agent '{}'", agent_id))?;

    if let Some(ticket_id) = current_ticket_id {
        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET status = 'queued',
                assigned_agent_id = NULL,
                opening_deadline_at = NULL,
                updated_at = ?2
            WHERE ticket_id = ?1
              AND assigned_agent_id = ?3
              AND status IN ('opening', 'in_progress')
            "#,
        )
        .bind(&ticket_id)
        .bind(now_ms)
        .bind(&agent_id)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to requeue ticket '{}' while deleting authorized helpdesk agent '{}'",
                ticket_id, agent_id
            )
        })?;
    }

    tx.commit()
        .await
        .context("failed to commit authorized helpdesk agent deletion")?;

    Ok(result.rows_affected() > 0)
}

pub async fn is_helpdesk_agent_authorized(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<bool> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let normalized_sql = normalized_helpdesk_agent_id_sql("agent_id");
    let query = format!(
        r#"
        SELECT COUNT(*)
        FROM helpdesk_authorized_agents
        WHERE {normalized_sql} = ?1
        "#
    );
    let row = sqlx::query_scalar::<_, i64>(&query)
        .bind(&agent_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to verify authorized helpdesk agent '{}'", agent_id))?;

    Ok(row > 0)
}

pub async fn create_helpdesk_ticket(
    pool: &SqlitePool,
    payload: &HelpdeskTicketCreateRequestV1,
) -> anyhow::Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now() as i64;
    let ticket_id = Uuid::new_v4().to_string();
    let client_id = payload.client_id.trim().to_string();
    let preferred_agent_id = normalize_optional_text(payload.preferred_agent_id.as_deref());
    let normalized_title = normalize_optional_text(payload.title.as_deref());
    let normalized_summary =
        normalize_optional_text(payload.summary.as_deref()).or_else(|| normalized_title.clone());
    let normalized_description = normalize_optional_text(payload.description.as_deref());
    let normalized_difficulty = normalize_optional_text(payload.difficulty.as_deref());

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk ticket transaction")?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_tickets (
            ticket_id,
            client_id,
            client_display_name,
            device_id,
            requested_by,
            title,
            description,
            difficulty,
            estimated_minutes,
            summary,
            status,
            assigned_agent_id,
            opening_deadline_at,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'queued', NULL, NULL, ?11, ?12)
        "#,
    )
    .bind(&ticket_id)
    .bind(&client_id)
    .bind(normalize_optional_text(
        payload.client_display_name.as_deref(),
    ))
    .bind(normalize_optional_text(payload.device_id.as_deref()))
    .bind(normalize_optional_text(payload.requested_by.as_deref()))
    .bind(normalized_title.clone())
    .bind(normalized_description.clone())
    .bind(normalized_difficulty.clone())
    .bind(payload.estimated_minutes.map(i64::from))
    .bind(normalized_summary.clone())
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to create helpdesk ticket '{}'", ticket_id))?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        &ticket_id,
        "help_request_created",
        Some(serde_json::json!({
            "client_id": client_id,
            "device_id": payload.device_id,
            "title": normalized_title,
            "description": normalized_description,
            "difficulty": normalized_difficulty,
            "estimated_minutes": payload.estimated_minutes,
            "summary": normalized_summary,
        })),
        now_ms,
    )
    .await?;

    if let Some(preferred_agent_id) = preferred_agent_id.as_deref() {
        insert_helpdesk_audit_event_tx(
            &mut tx,
            "ticket",
            &ticket_id,
            "preferred_agent_requested",
            Some(serde_json::json!({
                "preferred_agent_id": preferred_agent_id,
            })),
            now_ms,
        )
        .await?;
    }

    let ticket = get_helpdesk_ticket_tx(&mut tx, &ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found after create", ticket_id))?;

    tx.commit()
        .await
        .context("failed to commit helpdesk ticket transaction")?;
    Ok(ticket)
}

pub async fn update_helpdesk_ticket_operational_fields(
    pool: &SqlitePool,
    ticket_id: &str,
    difficulty: Option<&str>,
    estimated_minutes: Option<u32>,
) -> anyhow::Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now() as i64;
    let ticket_id = ticket_id.trim();
    let normalized_difficulty = normalize_optional_text(difficulty);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk ticket operational update transaction")?;

    let current_ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "helpdesk ticket '{}' not found before operational update",
                ticket_id
            )
        })?;

    if matches!(
        current_ticket.status,
        HelpdeskTicketStatus::Resolved
            | HelpdeskTicketStatus::Cancelled
            | HelpdeskTicketStatus::Failed
    ) {
        anyhow::bail!("ticket can no longer be updated operationally");
    }

    sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET difficulty = COALESCE(?2, difficulty),
            estimated_minutes = COALESCE(?3, estimated_minutes),
            updated_at = ?4
        WHERE ticket_id = ?1
        "#,
    )
    .bind(ticket_id)
    .bind(normalized_difficulty.as_deref())
    .bind(estimated_minutes.map(i64::from))
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to update operational fields for helpdesk ticket '{}'",
            ticket_id
        )
    })?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        ticket_id,
        "operational_fields_updated",
        Some(serde_json::json!({
            "difficulty": normalized_difficulty,
            "estimated_minutes": estimated_minutes,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "helpdesk ticket '{}' not found after operational update",
                ticket_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit helpdesk ticket operational update transaction")?;
    Ok(ticket)
}

pub async fn add_helpdesk_ticket_agent_report(
    pool: &SqlitePool,
    ticket_id: &str,
    agent_id: &str,
    note: &str,
) -> anyhow::Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now() as i64;
    let ticket_id = ticket_id.trim();
    let agent_id = agent_id.trim();
    let normalized_note =
        normalize_optional_text(Some(note)).context("agent report note cannot be empty")?;

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk agent report transaction")?;

    let current_ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "helpdesk ticket '{}' not found before agent report",
                ticket_id
            )
        })?;

    if current_ticket.assigned_agent_id.as_deref() != Some(agent_id) {
        anyhow::bail!("ticket is not currently assigned to this agent");
    }

    if !matches!(
        current_ticket.status,
        HelpdeskTicketStatus::Opening | HelpdeskTicketStatus::InProgress
    ) {
        anyhow::bail!("ticket is not active for agent reporting");
    }

    let agent = get_helpdesk_agent_tx(&mut tx, agent_id)
        .await?
        .with_context(|| {
            format!(
                "helpdesk agent '{}' not found before agent report",
                agent_id
            )
        })?;

    sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET latest_agent_report = ?2,
            latest_agent_report_by = ?3,
            latest_agent_report_at = ?4,
            updated_at = ?4
        WHERE ticket_id = ?1
        "#,
    )
    .bind(ticket_id)
    .bind(&normalized_note)
    .bind(&agent.display_name)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to store latest agent report for ticket '{}'",
            ticket_id
        )
    })?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        ticket_id,
        "agent_report_added",
        Some(serde_json::json!({
            "agent_id": agent_id,
            "agent_display_name": agent.display_name,
            "note": normalized_note,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "helpdesk ticket '{}' not found after agent report",
                ticket_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit helpdesk agent report transaction")?;
    Ok(ticket)
}

pub async fn assign_helpdesk_ticket(
    pool: &SqlitePool,
    ticket_id: &str,
    requested_agent_id: Option<&str>,
    reason: Option<&str>,
) -> anyhow::Result<(HelpdeskTicketV1, HelpdeskAgentV1)> {
    let now_ms = unix_millis_now() as i64;
    let ticket_id = ticket_id.trim();
    let requested_agent_id = normalize_optional_text(requested_agent_id);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk assign transaction")?;

    let current_ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found before assign", ticket_id))?;

    if current_ticket.status != HelpdeskTicketStatus::Queued {
        anyhow::bail!("ticket must be queued before it can be assigned");
    }

    let Some(selected_agent_id) = requested_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        anyhow::bail!("agent_id is required for manual assignment");
    };

    let assigned = assign_helpdesk_ticket_to_agent_tx(
        &mut tx,
        ticket_id,
        selected_agent_id,
        now_ms,
        "supervisor_manual",
        reason,
    )
    .await?;

    if !assigned {
        anyhow::bail!("the selected agent is no longer available for assignment");
    }

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found after assign", ticket_id))?;
    let agent = get_helpdesk_agent_tx(&mut tx, selected_agent_id)
        .await?
        .with_context(|| {
            format!(
                "helpdesk agent '{}' not found after assign",
                selected_agent_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit helpdesk assign transaction")?;
    Ok((ticket, agent))
}

pub async fn list_helpdesk_agents(pool: &SqlitePool) -> anyhow::Result<Vec<HelpdeskAgentV1>> {
    let rows = sqlx::query(
        r#"
        SELECT
            agent_id,
            display_name,
            avatar_url,
            status,
            current_ticket_id,
            last_heartbeat_at,
            updated_at
        FROM helpdesk_agents
        ORDER BY updated_at DESC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list helpdesk agents")?;

    rows.into_iter().map(row_to_helpdesk_agent).collect()
}

pub async fn list_helpdesk_tickets(pool: &SqlitePool) -> anyhow::Result<Vec<HelpdeskTicketV1>> {
    let rows = sqlx::query(
        r#"
        SELECT
            ticket_id,
            client_id,
            client_display_name,
            device_id,
            requested_by,
            title,
            description,
            difficulty,
            estimated_minutes,
            summary,
            status,
            assigned_agent_id,
            latest_agent_report,
            latest_agent_report_by,
            latest_agent_report_at,
            opening_deadline_at,
            created_at,
            updated_at
        FROM helpdesk_tickets
        ORDER BY created_at DESC, ticket_id DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list helpdesk tickets")?;

    rows.into_iter().map(row_to_helpdesk_ticket).collect()
}

pub async fn get_helpdesk_assignment_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Option<HelpdeskAssignmentV1>> {
    let row = sqlx::query(
        r#"
        SELECT current_ticket_id
        FROM helpdesk_agents
        WHERE agent_id = ?1
        "#,
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query current assignment for agent '{}'",
            agent_id
        )
    })?;

    let Some(row) = row else {
        return Ok(None);
    };

    let Some(ticket_id): Option<String> = row.get("current_ticket_id") else {
        return Ok(None);
    };

    let agent = get_helpdesk_agent(pool, agent_id)
        .await?
        .with_context(|| format!("missing helpdesk agent '{}'", agent_id))?;
    let ticket = get_helpdesk_ticket(pool, &ticket_id)
        .await?
        .with_context(|| format!("missing helpdesk ticket '{}'", ticket_id))?;

    Ok(Some(HelpdeskAssignmentV1 { ticket, agent }))
}

pub async fn get_helpdesk_ticket(
    pool: &SqlitePool,
    ticket_id: &str,
) -> anyhow::Result<Option<HelpdeskTicketV1>> {
    let row = sqlx::query(
        r#"
        SELECT
            ticket_id,
            client_id,
            client_display_name,
            device_id,
            requested_by,
            title,
            description,
            difficulty,
            estimated_minutes,
            summary,
            status,
            assigned_agent_id,
            latest_agent_report,
            latest_agent_report_by,
            latest_agent_report_at,
            opening_deadline_at,
            created_at,
            updated_at
        FROM helpdesk_tickets
        WHERE ticket_id = ?1
        "#,
    )
    .bind(ticket_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to query helpdesk ticket '{}'", ticket_id))?;

    row.map(row_to_helpdesk_ticket).transpose()
}

pub async fn get_helpdesk_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<Option<HelpdeskAgentV1>> {
    let row = sqlx::query(
        r#"
        SELECT
            agent_id,
            display_name,
            avatar_url,
            status,
            current_ticket_id,
            last_heartbeat_at,
            updated_at
        FROM helpdesk_agents
        WHERE agent_id = ?1
        "#,
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to query helpdesk agent '{}'", agent_id))?;

    row.map(row_to_helpdesk_agent).transpose()
}

pub async fn list_helpdesk_ticket_audit_events(
    pool: &SqlitePool,
    ticket_id: &str,
    limit: u64,
) -> anyhow::Result<Vec<HelpdeskAuditEventV1>> {
    let limit = limit.clamp(1, 500);
    let rows = sqlx::query(
        r#"
        SELECT entity_type, entity_id, event_type, payload, created_at
        FROM helpdesk_audit_events
        WHERE (entity_type = 'ticket' AND entity_id = ?1)
           OR (
                entity_type = 'agent'
                AND json_extract(payload, '$.ticket_id') = ?1
           )
        ORDER BY created_at DESC, id DESC
        LIMIT ?2
        "#,
    )
    .bind(ticket_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!(
            "failed to list helpdesk audit events for ticket '{}'",
            ticket_id
        )
    })?;

    rows.into_iter().map(row_to_helpdesk_audit_event).collect()
}

pub async fn get_helpdesk_operational_summary(
    pool: &SqlitePool,
) -> anyhow::Result<HelpdeskOperationalSummaryV1> {
    let ticket_rows = sqlx::query(
        r#"
        SELECT status, COUNT(*) AS total
        FROM helpdesk_tickets
        GROUP BY status
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to aggregate helpdesk ticket summary")?;

    let agent_rows = sqlx::query(
        r#"
        SELECT status, COUNT(*) AS total
        FROM helpdesk_agents
        GROUP BY status
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to aggregate helpdesk agent summary")?;

    let mut summary = HelpdeskOperationalSummaryV1 {
        tickets_new: 0,
        tickets_queued: 0,
        tickets_opening: 0,
        tickets_in_progress: 0,
        tickets_resolved: 0,
        tickets_cancelled: 0,
        tickets_failed: 0,
        agents_offline: 0,
        agents_available: 0,
        agents_opening: 0,
        agents_busy: 0,
        agents_away: 0,
    };

    for row in ticket_rows {
        let status: String = row.get("status");
        let total = i64_to_u64(row.get("total"));
        match status.as_str() {
            "new" => summary.tickets_new = total,
            "queued" => summary.tickets_queued = total,
            "opening" => summary.tickets_opening = total,
            "in_progress" => summary.tickets_in_progress = total,
            "resolved" => summary.tickets_resolved = total,
            "cancelled" => summary.tickets_cancelled = total,
            "failed" => summary.tickets_failed = total,
            _ => {}
        }
    }

    for row in agent_rows {
        let status: String = row.get("status");
        let total = i64_to_u64(row.get("total"));
        match status.as_str() {
            "offline" => summary.agents_offline = total,
            "available" => summary.agents_available = total,
            "opening" => summary.agents_opening = total,
            "busy" => summary.agents_busy = total,
            "away" => summary.agents_away = total,
            _ => {}
        }
    }

    Ok(summary)
}

pub async fn start_helpdesk_ticket(
    pool: &SqlitePool,
    agent_id: &str,
    ticket_id: &str,
) -> anyhow::Result<(HelpdeskTicketV1, HelpdeskAgentV1)> {
    let now_ms = unix_millis_now() as i64;
    let agent_id = agent_id.trim();
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk start transaction")?;

    let ticket_update = sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'in_progress',
            opening_deadline_at = NULL,
            updated_at = ?3
        WHERE ticket_id = ?1
          AND assigned_agent_id = ?2
          AND status = 'opening'
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to start helpdesk ticket '{}'", ticket_id))?;

    if ticket_update.rows_affected() == 0 {
        anyhow::bail!("ticket is not in opening state for this agent");
    }

    let agent_update = sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = 'busy',
            current_ticket_id = ?2,
            updated_at = ?3
        WHERE agent_id = ?1
          AND status = 'opening'
          AND current_ticket_id = ?2
        "#,
    )
    .bind(agent_id)
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to move helpdesk agent '{}' to busy", agent_id))?;

    if agent_update.rows_affected() == 0 {
        anyhow::bail!("agent is not in opening state for this ticket");
    }

    sqlx::query(
        r#"
        UPDATE helpdesk_ticket_assignments
        SET status = 'in_progress', updated_at = ?3
        WHERE ticket_id = ?1
          AND agent_id = ?2
          AND status = 'opening'
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to update assignment state for ticket '{}'",
            ticket_id
        )
    })?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        ticket_id,
        "remote_session_started",
        Some(serde_json::json!({
            "agent_id": agent_id,
        })),
        now_ms,
    )
    .await?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "agent",
        agent_id,
        "agent_became_busy",
        Some(serde_json::json!({
            "ticket_id": ticket_id,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found after start", ticket_id))?;
    let agent = get_helpdesk_agent_tx(&mut tx, agent_id)
        .await?
        .with_context(|| format!("helpdesk agent '{}' not found after start", agent_id))?;

    tx.commit()
        .await
        .context("failed to commit helpdesk start transaction")?;
    Ok((ticket, agent))
}

pub async fn resolve_helpdesk_ticket(
    pool: &SqlitePool,
    ticket_id: &str,
    agent_id: &str,
    next_agent_status: HelpdeskAgentStatus,
) -> anyhow::Result<(HelpdeskTicketV1, HelpdeskAgentV1)> {
    let now_ms = unix_millis_now() as i64;
    let agent_id = agent_id.trim();
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk resolve transaction")?;

    let ticket_update = sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'resolved',
            opening_deadline_at = NULL,
            updated_at = ?3
        WHERE ticket_id = ?1
          AND assigned_agent_id = ?2
          AND status = 'in_progress'
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to resolve helpdesk ticket '{}'", ticket_id))?;

    if ticket_update.rows_affected() == 0 {
        anyhow::bail!("ticket is not in progress for this agent");
    }

    let agent_status = next_agent_status.as_str();
    let agent_update = sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = ?3,
            current_ticket_id = NULL,
            updated_at = ?4
        WHERE agent_id = ?1
          AND current_ticket_id = ?2
          AND status = 'busy'
        "#,
    )
    .bind(agent_id)
    .bind(ticket_id)
    .bind(agent_status)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to release helpdesk agent '{}'", agent_id))?;

    if agent_update.rows_affected() == 0 {
        anyhow::bail!("agent is not busy with this ticket");
    }

    sqlx::query(
        r#"
        UPDATE helpdesk_ticket_assignments
        SET status = 'resolved', updated_at = ?3
        WHERE ticket_id = ?1
          AND agent_id = ?2
          AND status IN ('opening', 'in_progress')
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to resolve assignment for ticket '{}'", ticket_id))?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        ticket_id,
        "ticket_resolved",
        Some(serde_json::json!({
            "agent_id": agent_id,
        })),
        now_ms,
    )
    .await?;

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "agent",
        agent_id,
        if next_agent_status == HelpdeskAgentStatus::Away {
            "agent_became_away"
        } else {
            "agent_became_available"
        },
        Some(serde_json::json!({
            "ticket_id": ticket_id,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found after resolve", ticket_id))?;
    let agent = get_helpdesk_agent_tx(&mut tx, agent_id)
        .await?
        .with_context(|| format!("helpdesk agent '{}' not found after resolve", agent_id))?;

    tx.commit()
        .await
        .context("failed to commit helpdesk resolve transaction")?;
    Ok((ticket, agent))
}

pub async fn requeue_helpdesk_ticket(
    pool: &SqlitePool,
    ticket_id: &str,
    next_agent_status: HelpdeskAgentStatus,
    reason: Option<&str>,
) -> anyhow::Result<(HelpdeskTicketV1, Option<HelpdeskAgentV1>)> {
    let now_ms = unix_millis_now() as i64;
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk requeue transaction")?;

    let current_ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found before requeue", ticket_id))?;

    if current_ticket.status == HelpdeskTicketStatus::Resolved {
        anyhow::bail!("resolved ticket cannot be requeued");
    }

    let assigned_agent_id = current_ticket.assigned_agent_id.clone();
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());

    sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'queued',
            assigned_agent_id = NULL,
            opening_deadline_at = NULL,
            updated_at = ?2
        WHERE ticket_id = ?1
        "#,
    )
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to requeue helpdesk ticket '{}'", ticket_id))?;

    if let Some(agent_id) = assigned_agent_id.as_deref() {
        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET status = ?3,
                current_ticket_id = NULL,
                updated_at = ?4
            WHERE agent_id = ?1
              AND current_ticket_id = ?2
            "#,
        )
        .bind(agent_id)
        .bind(ticket_id)
        .bind(next_agent_status.as_str())
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to release helpdesk agent '{}' during requeue",
                agent_id
            )
        })?;

        sqlx::query(
            r#"
            UPDATE helpdesk_ticket_assignments
            SET status = 'requeued', updated_at = ?3
            WHERE ticket_id = ?1
              AND agent_id = ?2
              AND status IN ('opening', 'in_progress')
            "#,
        )
        .bind(ticket_id)
        .bind(agent_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to requeue assignment for ticket '{}'", ticket_id))?;

        insert_helpdesk_audit_event_tx(
            &mut tx,
            "agent",
            agent_id,
            "ticket_requeued_by_supervisor",
            Some(serde_json::json!({
                "ticket_id": ticket_id,
                "next_agent_status": next_agent_status.as_str(),
                "reason": reason,
            })),
            now_ms,
        )
        .await?;
    }

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        ticket_id,
        "ticket_requeued_by_supervisor",
        Some(serde_json::json!({
            "previous_status": current_ticket.status.as_str(),
            "previous_agent_id": assigned_agent_id,
            "next_agent_status": next_agent_status.as_str(),
            "reason": reason,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found after requeue", ticket_id))?;
    let agent = if let Some(agent_id) = current_ticket.assigned_agent_id.as_deref() {
        get_helpdesk_agent_tx(&mut tx, agent_id).await?
    } else {
        None
    };

    tx.commit()
        .await
        .context("failed to commit helpdesk requeue transaction")?;
    Ok((ticket, agent))
}

pub async fn cancel_helpdesk_ticket(
    pool: &SqlitePool,
    ticket_id: &str,
    next_agent_status: HelpdeskAgentStatus,
    reason: Option<&str>,
) -> anyhow::Result<(HelpdeskTicketV1, Option<HelpdeskAgentV1>)> {
    let now_ms = unix_millis_now() as i64;
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk cancel transaction")?;

    let current_ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found before cancel", ticket_id))?;

    if matches!(
        current_ticket.status,
        HelpdeskTicketStatus::Resolved | HelpdeskTicketStatus::Cancelled
    ) {
        anyhow::bail!("ticket is already terminal and cannot be cancelled");
    }

    let assigned_agent_id = current_ticket.assigned_agent_id.clone();
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());

    sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'cancelled',
            opening_deadline_at = NULL,
            updated_at = ?2
        WHERE ticket_id = ?1
        "#,
    )
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to cancel helpdesk ticket '{}'", ticket_id))?;

    if let Some(agent_id) = assigned_agent_id.as_deref() {
        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET status = ?3,
                current_ticket_id = NULL,
                updated_at = ?4
            WHERE agent_id = ?1
              AND current_ticket_id = ?2
            "#,
        )
        .bind(agent_id)
        .bind(ticket_id)
        .bind(next_agent_status.as_str())
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to release helpdesk agent '{}' during cancel",
                agent_id
            )
        })?;

        sqlx::query(
            r#"
            UPDATE helpdesk_ticket_assignments
            SET status = 'cancelled', updated_at = ?3
            WHERE ticket_id = ?1
              AND agent_id = ?2
              AND status IN ('opening', 'in_progress')
            "#,
        )
        .bind(ticket_id)
        .bind(agent_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to cancel assignment for ticket '{}'", ticket_id))?;

        insert_helpdesk_audit_event_tx(
            &mut tx,
            "agent",
            agent_id,
            "ticket_cancelled_by_supervisor",
            Some(serde_json::json!({
                "ticket_id": ticket_id,
                "next_agent_status": next_agent_status.as_str(),
                "reason": reason,
            })),
            now_ms,
        )
        .await?;
    }

    insert_helpdesk_audit_event_tx(
        &mut tx,
        "ticket",
        ticket_id,
        "ticket_cancelled_by_supervisor",
        Some(serde_json::json!({
            "previous_status": current_ticket.status.as_str(),
            "previous_agent_id": assigned_agent_id,
            "next_agent_status": next_agent_status.as_str(),
            "reason": reason,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| format!("helpdesk ticket '{}' not found after cancel", ticket_id))?;
    let agent = if let Some(agent_id) = current_ticket.assigned_agent_id.as_deref() {
        get_helpdesk_agent_tx(&mut tx, agent_id).await?
    } else {
        None
    };

    tx.commit()
        .await
        .context("failed to commit helpdesk cancel transaction")?;
    Ok((ticket, agent))
}

pub async fn reconcile_helpdesk_runtime(
    pool: &SqlitePool,
    agent_stale_after_ms: i64,
    now_ms: i64,
) -> anyhow::Result<HelpdeskRuntimeReconcileResult> {
    let stale_before_ms = now_ms.saturating_sub(agent_stale_after_ms);
    let mut tx = pool
        .begin()
        .await
        .context("failed to open helpdesk runtime reconcile transaction")?;
    let mut stats = HelpdeskRuntimeReconcileResult::default();

    let expired_openings = sqlx::query(
        r#"
        SELECT ticket_id, assigned_agent_id
        FROM helpdesk_tickets
        WHERE status = 'opening'
          AND opening_deadline_at IS NOT NULL
          AND opening_deadline_at <= ?1
        ORDER BY opening_deadline_at ASC, ticket_id ASC
        "#,
    )
    .bind(now_ms)
    .fetch_all(&mut *tx)
    .await
    .context("failed to query expired helpdesk openings")?;

    for row in expired_openings {
        let ticket_id: String = row.get("ticket_id");
        let agent_id: Option<String> = row.get("assigned_agent_id");

        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET status = 'queued',
                assigned_agent_id = NULL,
                opening_deadline_at = NULL,
                updated_at = ?2
            WHERE ticket_id = ?1
              AND status = 'opening'
            "#,
        )
        .bind(&ticket_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to requeue expired opening ticket '{}'", ticket_id))?;

        if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                UPDATE helpdesk_agents
                SET status = 'available',
                    current_ticket_id = NULL,
                    updated_at = ?2
                WHERE agent_id = ?1
                  AND status = 'opening'
                  AND current_ticket_id = ?3
                "#,
            )
            .bind(&agent_id)
            .bind(now_ms)
            .bind(&ticket_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to release expired opening agent '{}'", agent_id))?;

            sqlx::query(
                r#"
                UPDATE helpdesk_ticket_assignments
                SET status = 'expired', updated_at = ?3
                WHERE ticket_id = ?1
                  AND agent_id = ?2
                  AND status = 'opening'
                "#,
            )
            .bind(&ticket_id)
            .bind(&agent_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to expire assignment for ticket '{}'", ticket_id))?;

            insert_helpdesk_audit_event_tx(
                &mut tx,
                "agent",
                &agent_id,
                "opening_countdown_cancelled",
                Some(serde_json::json!({
                    "ticket_id": ticket_id,
                    "reason": "deadline_expired",
                })),
                now_ms,
            )
            .await?;
        }

        insert_helpdesk_audit_event_tx(
            &mut tx,
            "ticket",
            &ticket_id,
            "opening_countdown_cancelled",
            Some(serde_json::json!({
                "reason": "deadline_expired",
            })),
            now_ms,
        )
        .await?;

        stats.opening_timeouts = stats.opening_timeouts.saturating_add(1);
        stats.tickets_requeued = stats.tickets_requeued.saturating_add(1);
    }

    let stale_agents = sqlx::query(
        r#"
        SELECT agent_id, status, current_ticket_id
        FROM helpdesk_agents
        WHERE status != 'offline'
          AND last_heartbeat_at < ?1
        ORDER BY last_heartbeat_at ASC, agent_id ASC
        "#,
    )
    .bind(stale_before_ms)
    .fetch_all(&mut *tx)
    .await
    .context("failed to query stale helpdesk agents")?;

    for row in stale_agents {
        let agent_id: String = row.get("agent_id");
        let status: String = row.get("status");
        let current_ticket_id: Option<String> = row.get("current_ticket_id");

        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET status = 'offline',
                current_ticket_id = NULL,
                updated_at = ?2
            WHERE agent_id = ?1
            "#,
        )
        .bind(&agent_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to mark stale agent '{}' offline", agent_id))?;

        insert_helpdesk_audit_event_tx(
            &mut tx,
            "agent",
            &agent_id,
            "agent_went_offline",
            Some(serde_json::json!({
                "previous_status": status,
            })),
            now_ms,
        )
        .await?;

        stats.agents_marked_offline = stats.agents_marked_offline.saturating_add(1);

        if let Some(ticket_id) = current_ticket_id {
            if status == "opening" {
                sqlx::query(
                    r#"
                    UPDATE helpdesk_tickets
                    SET status = 'queued',
                        assigned_agent_id = NULL,
                        opening_deadline_at = NULL,
                        updated_at = ?2
                    WHERE ticket_id = ?1
                      AND status = 'opening'
                    "#,
                )
                .bind(&ticket_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to requeue ticket '{}' for stale opening agent",
                        ticket_id
                    )
                })?;

                sqlx::query(
                    r#"
                    UPDATE helpdesk_ticket_assignments
                    SET status = 'expired', updated_at = ?3
                    WHERE ticket_id = ?1
                      AND agent_id = ?2
                      AND status = 'opening'
                    "#,
                )
                .bind(&ticket_id)
                .bind(&agent_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!("failed to expire stale opening assignment '{}'", ticket_id)
                })?;

                insert_helpdesk_audit_event_tx(
                    &mut tx,
                    "ticket",
                    &ticket_id,
                    "opening_countdown_cancelled",
                    Some(serde_json::json!({
                        "reason": "agent_heartbeat_expired",
                        "agent_id": agent_id,
                    })),
                    now_ms,
                )
                .await?;

                stats.tickets_requeued = stats.tickets_requeued.saturating_add(1);
            } else if status == "busy" {
                sqlx::query(
                    r#"
                    UPDATE helpdesk_tickets
                    SET status = 'failed',
                        assigned_agent_id = NULL,
                        opening_deadline_at = NULL,
                        updated_at = ?2
                    WHERE ticket_id = ?1
                      AND status = 'in_progress'
                    "#,
                )
                .bind(&ticket_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to fail in-progress ticket '{}' for stale agent",
                        ticket_id
                    )
                })?;

                sqlx::query(
                    r#"
                    UPDATE helpdesk_ticket_assignments
                    SET status = 'failed', updated_at = ?3
                    WHERE ticket_id = ?1
                      AND agent_id = ?2
                      AND status IN ('opening', 'in_progress')
                    "#,
                )
                .bind(&ticket_id)
                .bind(&agent_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("failed to fail stale busy assignment '{}'", ticket_id))?;

                insert_helpdesk_audit_event_tx(
                    &mut tx,
                    "ticket",
                    &ticket_id,
                    "remote_session_failed",
                    Some(serde_json::json!({
                        "reason": "agent_heartbeat_expired",
                        "agent_id": agent_id,
                    })),
                    now_ms,
                )
                .await?;

                stats.tickets_failed = stats.tickets_failed.saturating_add(1);
            }
        }
    }

    tx.commit()
        .await
        .context("failed to commit helpdesk runtime reconcile transaction")?;
    Ok(stats)
}

pub async fn get_dashboard_summary(
    pool: &SqlitePool,
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
        WHERE created_at >= ?1 AND created_at <= ?2
        GROUP BY status
        "#,
    )
    .bind(from_ms)
    .bind(to_ms)
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
    let page_size = page_size.clamp(1, 500);
    let offset = page.saturating_sub(1).saturating_mul(page_size);

    let mut count_qb =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS total FROM session_events WHERE 1=1");
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

    if rows.is_empty() {
        return Ok((Vec::new(), total));
    }

    let actor_reference_index = load_session_actor_reference_index(pool).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let payload: String = row.get("payload");
        let event: SessionEventV1 = serde_json::from_str(&payload)
            .context("failed to deserialize event payload from session_events")?;
        let actor_type = actor_reference_index.classify(&event.user_id);
        items.push(event_to_timeline_item(event, actor_type));
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
    actor_type: Option<SessionActorTypeV1>,
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

    if let Some(actor_type) = actor_type {
        qb.push(" AND ");
        push_session_actor_type_match_clause(&mut qb, actor_type, "session_events.user_id");
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
            let is_control_active =
                meta_bool(event.meta.as_ref(), "is_control_active").unwrap_or(true);
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
            .with_context(|| {
                format!("failed to close presence for session {}", event.session_id)
            })?;
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
    if let Some(actor_type) = filter.actor_type {
        qb.push(" AND ");
        push_session_actor_type_match_clause(qb, actor_type, "session_events.user_id");
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

fn push_session_actor_type_match_clause(
    qb: &mut QueryBuilder<'_, Sqlite>,
    actor_type: SessionActorTypeV1,
    user_id_expr: &str,
) {
    match actor_type {
        SessionActorTypeV1::Agent => {
            push_helpdesk_agent_match_clause(qb, user_id_expr);
        }
        SessionActorTypeV1::Client => {
            push_helpdesk_client_match_clause(qb, user_id_expr);
        }
        SessionActorTypeV1::Unknown => {
            qb.push("NOT (");
            push_helpdesk_agent_match_clause(qb, user_id_expr);
            qb.push(") AND NOT (");
            push_helpdesk_client_match_clause(qb, user_id_expr);
            qb.push(")");
        }
    }
}

fn push_helpdesk_agent_match_clause(qb: &mut QueryBuilder<'_, Sqlite>, user_id_expr: &str) {
    let normalized_user_expr = format!("REPLACE(TRIM({user_id_expr}), ' ', '')");
    qb.push(format!(
        "(EXISTS (SELECT 1 FROM helpdesk_authorized_agents haa WHERE REPLACE(TRIM(haa.agent_id), ' ', '') = {normalized_user_expr}) \
OR EXISTS (SELECT 1 FROM helpdesk_agents ha WHERE REPLACE(TRIM(ha.agent_id), ' ', '') = {normalized_user_expr}))"
    ));
}

fn push_helpdesk_client_match_clause(qb: &mut QueryBuilder<'_, Sqlite>, user_id_expr: &str) {
    let normalized_user_expr = format!("REPLACE(TRIM({user_id_expr}), ' ', '')");
    qb.push(format!(
        "EXISTS (SELECT 1 FROM helpdesk_tickets ht WHERE REPLACE(TRIM(ht.client_id), ' ', '') = {normalized_user_expr})"
    ));
}

pub async fn should_store_participant_activity_for_monitoring(
    pool: &SqlitePool,
    event: &SessionEventV1,
    monitoring: &MonitoringConfig,
) -> anyhow::Result<bool> {
    if event.event_type != SessionEventType::ParticipantActivity {
        return Ok(true);
    }

    let min_interval_ms = monitoring
        .participant_activity_min_interval_seconds
        .saturating_mul(1_000) as i64;
    if min_interval_ms <= 0 {
        return Ok(true);
    }

    let actor = extract_presence_actor(event);
    let normalized_participant_id = normalize_helpdesk_identity_id(&actor.participant_id);
    if normalized_participant_id.is_empty() {
        return Ok(true);
    }

    let last_activity_at = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT last_activity_at
        FROM session_presence
        WHERE session_id = ?1
          AND REPLACE(TRIM(participant_id), ' ', '') = ?2
        ORDER BY last_activity_at DESC
        LIMIT 1
        "#,
    )
    .bind(&event.session_id)
    .bind(normalized_participant_id)
    .fetch_optional(pool)
    .await
    .context("failed to inspect previous participant activity timestamp")?;

    let Some(last_activity_at) = last_activity_at else {
        return Ok(true);
    };

    let event_ms = event.timestamp.timestamp_millis().max(0);
    Ok(event_ms.saturating_sub(last_activity_at) >= min_interval_ms)
}

pub async fn is_known_helpdesk_agent_id(pool: &SqlitePool, user_id: &str) -> anyhow::Result<bool> {
    let normalized_user_id = normalize_helpdesk_identity_id(user_id);
    if normalized_user_id.is_empty() {
        return Ok(false);
    }

    let exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM helpdesk_authorized_agents haa
            WHERE REPLACE(TRIM(haa.agent_id), ' ', '') = ?1
            UNION
            SELECT 1
            FROM helpdesk_agents ha
            WHERE REPLACE(TRIM(ha.agent_id), ' ', '') = ?1
        )
        "#,
    )
    .bind(normalized_user_id)
    .fetch_one(pool)
    .await
    .context("failed to classify session actor against helpdesk agents")?;

    Ok(exists != 0)
}

pub async fn helpdesk_agent_has_active_ticket(
    pool: &SqlitePool,
    user_id: &str,
) -> anyhow::Result<bool> {
    let normalized_user_id = normalize_helpdesk_identity_id(user_id);
    if normalized_user_id.is_empty() {
        return Ok(false);
    }

    let exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM helpdesk_tickets ht
            WHERE REPLACE(TRIM(COALESCE(ht.assigned_agent_id, '')), ' ', '') = ?1
              AND ht.status IN ('opening', 'in_progress')
            UNION
            SELECT 1
            FROM helpdesk_agents ha
            WHERE REPLACE(TRIM(ha.agent_id), ' ', '') = ?1
              AND ha.current_ticket_id IS NOT NULL
              AND ha.status IN ('opening', 'busy')
        )
        "#,
    )
    .bind(normalized_user_id)
    .fetch_one(pool)
    .await
    .context("failed to determine whether helpdesk agent has an active ticket")?;

    Ok(exists != 0)
}

async fn load_session_actor_reference_index(
    pool: &SqlitePool,
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
    .context("failed to query helpdesk agent ids for session actor classification")?;

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
        WHERE TRIM(client_id) != ''
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query helpdesk client ids for session actor classification")?;

    for client_id in client_ids {
        let normalized = normalize_helpdesk_identity_id(&client_id);
        if !normalized.is_empty() {
            actor_reference_index.client_ids.insert(normalized);
        }
    }

    Ok(actor_reference_index)
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

async fn assign_helpdesk_ticket_to_agent_tx(
    tx: &mut Transaction<'_, Sqlite>,
    ticket_id: &str,
    agent_id: &str,
    now_ms: i64,
    dispatch_source: &str,
    reason: Option<&str>,
) -> anyhow::Result<bool> {
    let deadline_ms = now_ms + HELPDESK_OPENING_WINDOW_MS;
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());

    let ticket_update = sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'opening',
            assigned_agent_id = ?2,
            opening_deadline_at = ?3,
            updated_at = ?4
        WHERE ticket_id = ?1 AND status = 'queued'
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(deadline_ms)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to move ticket '{}' to opening", ticket_id))?;

    if ticket_update.rows_affected() == 0 {
        return Ok(false);
    }

    let agent_update = sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = 'opening',
            current_ticket_id = ?2,
            updated_at = ?3
        WHERE agent_id = ?1 AND status = 'available'
        "#,
    )
    .bind(agent_id)
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to move agent '{}' to opening", agent_id))?;

    if agent_update.rows_affected() == 0 {
        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET status = 'queued',
                assigned_agent_id = NULL,
                opening_deadline_at = NULL,
                updated_at = ?2
            WHERE ticket_id = ?1
            "#,
        )
        .bind(ticket_id)
        .bind(now_ms)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("failed to rollback ticket '{}' opening state", ticket_id))?;
        return Ok(false);
    }

    sqlx::query(
        r#"
        INSERT INTO helpdesk_ticket_assignments (
            ticket_id, agent_id, status, created_at, updated_at
        )
        VALUES (?1, ?2, 'opening', ?3, ?4)
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to insert assignment for ticket '{}'", ticket_id))?;

    insert_helpdesk_audit_event_tx(
        tx,
        "ticket",
        ticket_id,
        "ticket_assigned",
        Some(serde_json::json!({
            "agent_id": agent_id,
            "opening_deadline_at": millis_to_utc(deadline_ms).to_rfc3339(),
            "dispatch_source": dispatch_source,
            "reason": reason,
        })),
        now_ms,
    )
    .await?;

    insert_helpdesk_audit_event_tx(
        tx,
        "agent",
        agent_id,
        "opening_countdown_started",
        Some(serde_json::json!({
            "ticket_id": ticket_id,
            "opening_deadline_at": millis_to_utc(deadline_ms).to_rfc3339(),
            "dispatch_source": dispatch_source,
            "reason": reason,
        })),
        now_ms,
    )
    .await?;

    Ok(true)
}

async fn insert_helpdesk_audit_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    entity_type: &str,
    entity_id: &str,
    event_type: &str,
    payload: Option<Value>,
    now_ms: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO helpdesk_audit_events (
            entity_type, entity_id, event_type, payload, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(entity_type)
    .bind(entity_id)
    .bind(event_type)
    .bind(payload.map(|value| value.to_string()))
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| {
        format!(
            "failed to insert helpdesk audit event '{}:{}'",
            entity_type, entity_id
        )
    })?;

    Ok(())
}

async fn get_helpdesk_agent_tx(
    tx: &mut Transaction<'_, Sqlite>,
    agent_id: &str,
) -> anyhow::Result<Option<HelpdeskAgentV1>> {
    let row = sqlx::query(
        r#"
        SELECT
            agent_id,
            display_name,
            avatar_url,
            status,
            current_ticket_id,
            last_heartbeat_at,
            updated_at
        FROM helpdesk_agents
        WHERE agent_id = ?1
        "#,
    )
    .bind(agent_id)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("failed to query helpdesk agent '{}' in tx", agent_id))?;

    row.map(row_to_helpdesk_agent).transpose()
}

async fn get_helpdesk_ticket_tx(
    tx: &mut Transaction<'_, Sqlite>,
    ticket_id: &str,
) -> anyhow::Result<Option<HelpdeskTicketV1>> {
    let row = sqlx::query(
        r#"
        SELECT
            ticket_id,
            client_id,
            client_display_name,
            device_id,
            requested_by,
            title,
            description,
            difficulty,
            estimated_minutes,
            summary,
            status,
            assigned_agent_id,
            latest_agent_report,
            latest_agent_report_by,
            latest_agent_report_at,
            opening_deadline_at,
            created_at,
            updated_at
        FROM helpdesk_tickets
        WHERE ticket_id = ?1
        "#,
    )
    .bind(ticket_id)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("failed to query helpdesk ticket '{}' in tx", ticket_id))?;

    row.map(row_to_helpdesk_ticket).transpose()
}

fn row_to_helpdesk_agent(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<HelpdeskAgentV1> {
    let status: String = row.get("status");
    Ok(HelpdeskAgentV1 {
        agent_id: row.get("agent_id"),
        display_name: row.get("display_name"),
        avatar_url: row.get("avatar_url"),
        status: helpdesk_agent_status_from_db(&status),
        current_ticket_id: row.get("current_ticket_id"),
        last_heartbeat_at: millis_to_utc(row.get("last_heartbeat_at")),
        updated_at: millis_to_utc(row.get("updated_at")),
    })
}

fn row_to_helpdesk_authorized_agent(
    row: sqlx::sqlite::SqliteRow,
) -> anyhow::Result<HelpdeskAuthorizedAgentV1> {
    let rotated_at: Option<i64> = row.get("agent_token_rotated_at");
    let token_hash: Option<String> = row.get("agent_token_hash");
    Ok(HelpdeskAuthorizedAgentV1 {
        agent_id: normalize_helpdesk_agent_id(&row.get::<String, _>("agent_id")),
        display_name: row.get("display_name"),
        token_configured: token_hash
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty()),
        token_hint: row.get("agent_token_hint"),
        agent_token_rotated_at: rotated_at.map(millis_to_utc),
        created_at: millis_to_utc(row.get("created_at")),
        updated_at: millis_to_utc(row.get("updated_at")),
    })
}

fn normalize_helpdesk_identity_id(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

fn normalize_helpdesk_agent_id(raw: &str) -> String {
    normalize_helpdesk_identity_id(raw)
}

async fn ensure_helpdesk_agent_display_name_available(
    pool: &SqlitePool,
    agent_id: &str,
    display_name: Option<&str>,
) -> anyhow::Result<()> {
    let Some(display_name) = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    let conflicting_authorized_agent = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT agent_id
        FROM helpdesk_authorized_agents
        WHERE agent_id != ?1
          AND lower(trim(display_name)) = lower(trim(?2))
        LIMIT 1
        "#,
    )
    .bind(agent_id)
    .bind(display_name)
    .fetch_optional(pool)
    .await
    .context("failed to validate authorized helpdesk agent display name uniqueness")?
    .flatten();

    if let Some(conflicting_agent_id) = conflicting_authorized_agent {
        anyhow::bail!(
            "display name '{}' is already assigned to agent '{}'",
            display_name,
            conflicting_agent_id
        );
    }

    let conflicting_live_agent = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT agent_id
        FROM helpdesk_agents
        WHERE agent_id != ?1
          AND lower(trim(display_name)) = lower(trim(?2))
        LIMIT 1
        "#,
    )
    .bind(agent_id)
    .bind(display_name)
    .fetch_optional(pool)
    .await
    .context("failed to validate live helpdesk agent display name uniqueness")?
    .flatten();

    if let Some(conflicting_agent_id) = conflicting_live_agent {
        anyhow::bail!(
            "display name '{}' is already assigned to agent '{}'",
            display_name,
            conflicting_agent_id
        );
    }

    Ok(())
}

fn normalized_helpdesk_agent_id_sql(column: &str) -> String {
    format!(
        "REPLACE(REPLACE(REPLACE(REPLACE({column}, ' ', ''), char(9), ''), char(10), ''), char(13), '')"
    )
}

async fn delete_legacy_helpdesk_authorized_agent_variants(
    pool: &SqlitePool,
    agent_id: &str,
) -> anyhow::Result<()> {
    let normalized_sql = normalized_helpdesk_agent_id_sql("agent_id");
    let query = format!(
        r#"
        DELETE FROM helpdesk_authorized_agents
        WHERE {normalized_sql} = ?1
          AND agent_id != ?2
        "#
    );
    sqlx::query(&query)
        .bind(agent_id)
        .bind(agent_id)
        .execute(pool)
        .await
        .with_context(|| {
            format!(
                "failed to delete legacy authorized helpdesk agent variants for '{}'",
                agent_id
            )
        })?;
    Ok(())
}

fn row_to_helpdesk_ticket(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<HelpdeskTicketV1> {
    let status: String = row.get("status");
    let opening_deadline_at: Option<i64> = row.get("opening_deadline_at");
    Ok(HelpdeskTicketV1 {
        ticket_id: row.get("ticket_id"),
        client_id: row.get("client_id"),
        client_display_name: row.get("client_display_name"),
        device_id: row.get("device_id"),
        requested_by: row.get("requested_by"),
        title: row.get("title"),
        description: row.get("description"),
        difficulty: row.get("difficulty"),
        estimated_minutes: row
            .get::<Option<i64>, _>("estimated_minutes")
            .and_then(|value| u32::try_from(value).ok()),
        summary: row.get("summary"),
        status: helpdesk_ticket_status_from_db(&status),
        assigned_agent_id: row.get("assigned_agent_id"),
        latest_agent_report: row.get("latest_agent_report"),
        latest_agent_report_by: row.get("latest_agent_report_by"),
        latest_agent_report_at: row
            .get::<Option<i64>, _>("latest_agent_report_at")
            .map(millis_to_utc),
        opening_deadline_at: opening_deadline_at.map(millis_to_utc),
        created_at: millis_to_utc(row.get("created_at")),
        updated_at: millis_to_utc(row.get("updated_at")),
    })
}

fn row_to_helpdesk_audit_event(
    row: sqlx::sqlite::SqliteRow,
) -> anyhow::Result<HelpdeskAuditEventV1> {
    let payload: Option<String> = row.get("payload");
    Ok(HelpdeskAuditEventV1 {
        entity_type: row.get("entity_type"),
        entity_id: row.get("entity_id"),
        event_type: row.get("event_type"),
        payload: payload
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("failed to deserialize helpdesk audit payload")?,
        created_at: millis_to_utc(row.get("created_at")),
    })
}

fn helpdesk_agent_status_from_db(raw: &str) -> HelpdeskAgentStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "available" => HelpdeskAgentStatus::Available,
        "opening" => HelpdeskAgentStatus::Opening,
        "busy" => HelpdeskAgentStatus::Busy,
        "away" => HelpdeskAgentStatus::Away,
        _ => HelpdeskAgentStatus::Offline,
    }
}

fn helpdesk_ticket_status_from_db(raw: &str) -> HelpdeskTicketStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "queued" => HelpdeskTicketStatus::Queued,
        "assigned" => HelpdeskTicketStatus::Assigned,
        "opening" => HelpdeskTicketStatus::Opening,
        "in_progress" => HelpdeskTicketStatus::InProgress,
        "resolved" => HelpdeskTicketStatus::Resolved,
        "cancelled" => HelpdeskTicketStatus::Cancelled,
        "failed" => HelpdeskTicketStatus::Failed,
        _ => HelpdeskTicketStatus::New,
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
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
    Utc.timestamp_millis_opt(value).single().unwrap_or_else(|| {
        Utc.timestamp_opt(0, 0)
            .single()
            .expect("unix epoch should exist")
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use tempfile::tempdir;
    use uuid::Uuid;

    use crate::config::MonitoringConfig;
    use crate::model::{
        HelpdeskAgentPresenceUpdateV1, HelpdeskAgentStatus, HelpdeskAuthorizedAgentUpsertRequestV1,
        HelpdeskTicketCreateRequestV1, HelpdeskTicketStatus, SessionActorTypeV1, SessionDirection,
        SessionEventType, SessionEventV1,
    };

    use super::{
        add_helpdesk_ticket_agent_report, assign_helpdesk_ticket, cancel_helpdesk_ticket,
        cleanup_delivered_older_than, cleanup_helpdesk_agent_heartbeats_older_than,
        cleanup_inactive_session_presence_older_than, cleanup_session_events_older_than,
        connect_sqlite, create_helpdesk_ticket, expire_stale_presence, get_helpdesk_agent,
        get_helpdesk_agent_authorization_status, get_helpdesk_assignment_for_agent,
        get_helpdesk_operational_summary, get_helpdesk_ticket, get_session_presence, insert_event,
        list_active_session_presence, list_helpdesk_ticket_audit_events, list_helpdesk_tickets,
        provision_helpdesk_authorized_agent, query_timeline_events, reconcile_helpdesk_runtime,
        requeue_helpdesk_ticket, resolve_helpdesk_ticket, should_store_session_event,
        start_helpdesk_ticket, unix_millis_now, update_helpdesk_ticket_operational_fields,
        upsert_helpdesk_agent_presence, upsert_helpdesk_authorized_agent,
        verify_helpdesk_agent_token, EventQueryFilter, InsertOutcome,
    };

    async fn authorize_agent(pool: &sqlx::SqlitePool, agent_id: &str, display_name: &str) {
        upsert_helpdesk_authorized_agent(
            pool,
            &HelpdeskAuthorizedAgentUpsertRequestV1 {
                agent_id: agent_id.to_string(),
                display_name: Some(display_name.to_string()),
                rotate_agent_token: None,
            },
        )
        .await
        .expect("authorize agent");
    }

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
    async fn provisioned_agent_token_is_verified_and_status_exposes_hint() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-agent-token.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let provisioning = provision_helpdesk_authorized_agent(
            &pool,
            &HelpdeskAuthorizedAgentUpsertRequestV1 {
                agent_id: "agent-token-1".to_string(),
                display_name: Some("Agent Token".to_string()),
                rotate_agent_token: Some(true),
            },
        )
        .await
        .expect("provision agent");

        assert!(provisioning.agent.token_configured);
        assert!(provisioning.agent.token_hint.is_some());
        assert!(provisioning.agent.agent_token_rotated_at.is_some());
        assert!(
            verify_helpdesk_agent_token(&pool, "agent-token-1", &provisioning.agent_token)
                .await
                .expect("verify issued token")
        );
        assert!(
            !verify_helpdesk_agent_token(&pool, "agent-token-1", "wrong-token")
                .await
                .expect("reject wrong token")
        );

        let status = get_helpdesk_agent_authorization_status(&pool, "agent-token-1")
            .await
            .expect("get authorization status");
        assert!(status.authorized);
        assert!(status.token_configured);
        assert!(status
            .token_hint
            .as_deref()
            .unwrap_or_default()
            .starts_with("..."));
    }

    #[tokio::test]
    async fn monitoring_policy_ignores_non_agent_session_events_by_default() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("monitoring-policy.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let event = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::SessionStarted,
            session_id: "sess-non-agent".to_string(),
            user_id: "client-plain".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: None,
        };

        let should_store = should_store_session_event(&pool, &event, &MonitoringConfig::default())
            .await
            .expect("apply monitoring policy");

        assert!(!should_store);
    }

    #[tokio::test]
    async fn participant_activity_is_sampled_for_agents() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("participant-activity-sampling.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        authorize_agent(&pool, "278084673", "Edward soporte").await;
        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "278084673".to_string(),
                display_name: Some("Edward soporte".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent presence");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-sampled".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Sampling".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("278084673"),
            Some("monitoring sampling test"),
        )
        .await
        .expect("assign ticket");

        let base_ts = Utc::now();
        let first = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantActivity,
            session_id: "sess-sampled".to_string(),
            user_id: "278084673".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: base_ts,
            host_info: None,
            meta: Some(json!({
                "participant_id": "278084673",
            })),
        };

        let second = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantActivity,
            session_id: "sess-sampled".to_string(),
            user_id: "278084673".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: base_ts + chrono::Duration::seconds(30),
            host_info: None,
            meta: Some(json!({
                "participant_id": "278084673",
            })),
        };

        let third = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::ParticipantActivity,
            session_id: "sess-sampled".to_string(),
            user_id: "278084673".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: base_ts + chrono::Duration::seconds(90),
            host_info: None,
            meta: Some(json!({
                "participant_id": "278084673",
            })),
        };

        assert!(
            should_store_session_event(&pool, &first, &MonitoringConfig::default())
                .await
                .expect("first activity should be stored")
        );
        insert_event(&pool, &first)
            .await
            .expect("insert first activity");

        assert!(
            !should_store_session_event(&pool, &second, &MonitoringConfig::default())
                .await
                .expect("second activity should be sampled out")
        );
        assert!(
            should_store_session_event(&pool, &third, &MonitoringConfig::default())
                .await
                .expect("third activity should be stored")
        );
    }

    #[tokio::test]
    async fn monitoring_policy_ignores_agent_events_without_active_ticket() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("monitoring-agent-idle.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        authorize_agent(&pool, "agent-idle", "Agente Idle").await;
        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-idle".to_string(),
                display_name: Some("Agente Idle".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent presence");

        let event = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::SessionStarted,
            session_id: "sess-agent-idle".to_string(),
            user_id: "agent-idle".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: None,
        };

        let should_store = should_store_session_event(&pool, &event, &MonitoringConfig::default())
            .await
            .expect("apply monitoring policy");

        assert!(!should_store);
    }

    #[tokio::test]
    async fn monitoring_policy_accepts_agent_events_with_active_ticket() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("monitoring-agent-active.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        authorize_agent(&pool, "agent-active", "Agente Activo").await;
        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-active".to_string(),
                display_name: Some("Agente Activo".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent presence");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-active".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Active monitoring".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-active"),
            Some("monitoring active test"),
        )
        .await
        .expect("assign ticket");

        let event = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::SessionStarted,
            session_id: "sess-agent-active".to_string(),
            user_id: "agent-active".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: None,
        };

        let should_store = should_store_session_event(&pool, &event, &MonitoringConfig::default())
            .await
            .expect("apply monitoring policy");

        assert!(should_store);
    }

    #[tokio::test]
    async fn monitoring_cleanup_removes_old_local_rows() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("monitoring-cleanup.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        authorize_agent(&pool, "278084673", "Edward soporte").await;
        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "278084673".to_string(),
                display_name: Some("Edward soporte".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent presence");

        let event = SessionEventV1 {
            event_id: Uuid::new_v4(),
            event_type: SessionEventType::SessionStarted,
            session_id: "sess-cleanup".to_string(),
            user_id: "278084673".to_string(),
            direction: SessionDirection::Outgoing,
            timestamp: Utc::now(),
            host_info: None,
            meta: None,
        };
        insert_event(&pool, &event)
            .await
            .expect("insert session event");

        let cutoff_ms = unix_millis_now() + 1_000;

        assert_eq!(
            cleanup_delivered_older_than(&pool, cutoff_ms)
                .await
                .expect("cleanup delivered"),
            0
        );
        assert_eq!(
            cleanup_session_events_older_than(&pool, cutoff_ms)
                .await
                .expect("cleanup session events"),
            1
        );
        assert_eq!(
            cleanup_inactive_session_presence_older_than(&pool, cutoff_ms)
                .await
                .expect("cleanup inactive presence"),
            0
        );
        assert!(
            cleanup_helpdesk_agent_heartbeats_older_than(&pool, cutoff_ms)
                .await
                .expect("cleanup agent heartbeats")
                >= 1
        );
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
        let (expired_rows, touched_sessions) =
            expire_stale_presence(&pool, stale_before_ms, now_ms)
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
    async fn helpdesk_presence_requires_authorized_agent() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-auth.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let payload = HelpdeskAgentPresenceUpdateV1 {
            agent_id: "agent-unauthorized".to_string(),
            display_name: Some("Unauthorized".to_string()),
            avatar_url: None,
            status: HelpdeskAgentStatus::Available,
        };

        let err = upsert_helpdesk_agent_presence(&pool, &payload)
            .await
            .expect_err("presence should fail for unauthorized agent");
        assert!(err
            .to_string()
            .contains("is not authorized for helpdesk operator mode"));

        upsert_helpdesk_authorized_agent(
            &pool,
            &HelpdeskAuthorizedAgentUpsertRequestV1 {
                agent_id: payload.agent_id.clone(),
                display_name: Some("Authorized".to_string()),
                rotate_agent_token: None,
            },
        )
        .await
        .expect("authorize agent");

        let agent = upsert_helpdesk_agent_presence(&pool, &payload)
            .await
            .expect("presence should succeed for authorized agent");
        assert_eq!(agent.agent_id, payload.agent_id);

        let authorization = get_helpdesk_agent_authorization_status(&pool, &payload.agent_id)
            .await
            .expect("authorization status");
        assert!(authorization.authorized);
    }

    #[tokio::test]
    async fn helpdesk_authorization_ignores_agent_id_whitespace() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-auth-whitespace.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        upsert_helpdesk_authorized_agent(
            &pool,
            &HelpdeskAuthorizedAgentUpsertRequestV1 {
                agent_id: "419 797 027".to_string(),
                display_name: Some("Edward soporte".to_string()),
                rotate_agent_token: None,
            },
        )
        .await
        .expect("authorize agent with formatted id");

        let authorization = get_helpdesk_agent_authorization_status(&pool, "419797027")
            .await
            .expect("authorization status without spaces");
        assert!(authorization.authorized);
        assert_eq!(authorization.agent_id, "419797027");

        let agent = upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "419797027".to_string(),
                display_name: Some("Edward Mendoza".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("presence should succeed for normalized id");
        assert_eq!(agent.agent_id, "419797027");
    }

    #[tokio::test]
    async fn session_timeline_events_can_be_filtered_by_actor_type() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("session-actor-filter.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        authorize_agent(&pool, "278084673", "Edward soporte").await;
        create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "1945832881".to_string(),
                client_display_name: Some("Cliente prueba".to_string()),
                device_id: Some("device-actor".to_string()),
                requested_by: Some("cliente".to_string()),
                title: Some("Necesito ayuda".to_string()),
                description: Some("Validar filtro de actores".to_string()),
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Filtro actor".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create client ticket");

        let events = vec![
            SessionEventV1 {
                event_id: Uuid::new_v4(),
                event_type: SessionEventType::SessionStarted,
                session_id: "sess-actor-filter".to_string(),
                user_id: "278084673".to_string(),
                direction: SessionDirection::Outgoing,
                timestamp: Utc::now(),
                host_info: None,
                meta: None,
            },
            SessionEventV1 {
                event_id: Uuid::new_v4(),
                event_type: SessionEventType::ParticipantJoined,
                session_id: "sess-actor-filter".to_string(),
                user_id: "1945832881".to_string(),
                direction: SessionDirection::Incoming,
                timestamp: Utc::now(),
                host_info: None,
                meta: None,
            },
            SessionEventV1 {
                event_id: Uuid::new_v4(),
                event_type: SessionEventType::ParticipantActivity,
                session_id: "sess-actor-filter".to_string(),
                user_id: "guest-unknown".to_string(),
                direction: SessionDirection::Incoming,
                timestamp: Utc::now(),
                host_info: None,
                meta: None,
            },
        ];

        for event in events {
            insert_event(&pool, &event).await.expect("insert event");
        }

        let (agent_items, agent_total) = query_timeline_events(
            &pool,
            &EventQueryFilter {
                actor_type: Some(SessionActorTypeV1::Agent),
                ..Default::default()
            },
            1,
            50,
        )
        .await
        .expect("query agent actor type");
        assert_eq!(agent_total, 1);
        assert_eq!(agent_items.len(), 1);
        assert_eq!(agent_items[0].actor_type, SessionActorTypeV1::Agent);
        assert_eq!(agent_items[0].user_id, "278084673");

        let (client_items, client_total) = query_timeline_events(
            &pool,
            &EventQueryFilter {
                actor_type: Some(SessionActorTypeV1::Client),
                ..Default::default()
            },
            1,
            50,
        )
        .await
        .expect("query client actor type");
        assert_eq!(client_total, 1);
        assert_eq!(client_items.len(), 1);
        assert_eq!(client_items[0].actor_type, SessionActorTypeV1::Client);
        assert_eq!(client_items[0].user_id, "1945832881");

        let (unknown_items, unknown_total) = query_timeline_events(
            &pool,
            &EventQueryFilter {
                actor_type: Some(SessionActorTypeV1::Unknown),
                ..Default::default()
            },
            1,
            50,
        )
        .await
        .expect("query unknown actor type");
        assert_eq!(unknown_total, 1);
        assert_eq!(unknown_items.len(), 1);
        assert_eq!(unknown_items[0].actor_type, SessionActorTypeV1::Unknown);
        assert_eq!(unknown_items[0].user_id, "guest-unknown");
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

    #[tokio::test]
    async fn helpdesk_ticket_stays_queued_even_if_an_agent_is_available() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-assign.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-1", "Agent One").await;

        let agent = upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-1".to_string(),
                display_name: Some("Agent One".to_string()),
                avatar_url: Some("https://example.com/agent-one.png".to_string()),
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        assert_eq!(agent.status, HelpdeskAgentStatus::Available);
        assert_eq!(
            agent.avatar_url.as_deref(),
            Some("https://example.com/agent-one.png")
        );

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-1".to_string(),
                client_display_name: Some("Cliente 1".to_string()),
                device_id: Some("device-1".to_string()),
                requested_by: Some("user@example.com".to_string()),
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Necesito ayuda".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assert_eq!(ticket.status, HelpdeskTicketStatus::Queued);
        assert!(ticket.assigned_agent_id.is_none());
        assert!(ticket.opening_deadline_at.is_none());

        let assignment = get_helpdesk_assignment_for_agent(&pool, "agent-1")
            .await
            .expect("get assignment");
        assert!(assignment.is_none());
    }

    #[tokio::test]
    async fn helpdesk_ticket_does_not_auto_assign_even_with_preferred_agent() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-preferred-agent.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-1", "Agent One").await;
        authorize_agent(&pool, "agent-2", "Agent Two").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-1".to_string(),
                display_name: Some("Agent One".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent 1");

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-2".to_string(),
                display_name: Some("Agent Two".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent 2");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-preferred".to_string(),
                client_display_name: Some("Cliente Preferred".to_string()),
                device_id: None,
                requested_by: Some("Supervisor".to_string()),
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Dispatch to agent 2".to_string()),
                preferred_agent_id: Some("agent-2".to_string()),
            },
        )
        .await
        .expect("create ticket");

        assert_eq!(ticket.status, HelpdeskTicketStatus::Queued);
        assert!(ticket.assigned_agent_id.is_none());

        let agent_one = get_helpdesk_agent(&pool, "agent-1")
            .await
            .expect("get agent 1")
            .expect("agent 1 exists");
        assert_eq!(agent_one.status, HelpdeskAgentStatus::Available);
        assert!(agent_one.current_ticket_id.is_none());

        let assignment = get_helpdesk_assignment_for_agent(&pool, "agent-2")
            .await
            .expect("get assignment");
        assert!(assignment.is_none());
    }

    #[tokio::test]
    async fn queued_helpdesk_ticket_is_not_picked_when_agent_becomes_available() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-queue.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-queued", "Agent Queue").await;

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-queued".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("En cola".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create queued ticket");

        assert_eq!(ticket.status, HelpdeskTicketStatus::Queued);
        assert!(ticket.assigned_agent_id.is_none());

        let agent = upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-queued".to_string(),
                display_name: Some("Agent Queue".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert available agent");

        assert_eq!(agent.status, HelpdeskAgentStatus::Available);

        let tickets = list_helpdesk_tickets(&pool)
            .await
            .expect("list helpdesk tickets");
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].status, HelpdeskTicketStatus::Queued);
        assert!(tickets[0].assigned_agent_id.is_none());
    }

    #[tokio::test]
    async fn helpdesk_ticket_can_move_from_opening_to_resolved() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-lifecycle.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-life", "Agent Life").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-life".to_string(),
                display_name: Some("Agent Life".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-life".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Lifecycle".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-life"),
            Some("manual test"),
        )
        .await
        .expect("assign ticket");

        let (started_ticket, started_agent) =
            start_helpdesk_ticket(&pool, "agent-life", &ticket.ticket_id)
                .await
                .expect("start ticket");
        assert_eq!(started_ticket.status, HelpdeskTicketStatus::InProgress);
        assert_eq!(started_agent.status, HelpdeskAgentStatus::Busy);

        let (resolved_ticket, resolved_agent) = resolve_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            "agent-life",
            HelpdeskAgentStatus::Available,
        )
        .await
        .expect("resolve ticket");
        assert_eq!(resolved_ticket.status, HelpdeskTicketStatus::Resolved);
        assert_eq!(resolved_agent.status, HelpdeskAgentStatus::Available);
        assert_eq!(resolved_agent.current_ticket_id, None);
    }

    #[tokio::test]
    async fn opening_assignment_survives_available_presence_heartbeat() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-opening-heartbeat.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-heartbeat", "Agent Heartbeat").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-heartbeat".to_string(),
                display_name: Some("Agent Heartbeat".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-heartbeat".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Opening heartbeat".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-heartbeat"),
            Some("manual dispatch"),
        )
        .await
        .expect("assign ticket");

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-heartbeat".to_string(),
                display_name: Some("Agent Heartbeat".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("heartbeat while opening");

        let agent_during_opening = get_helpdesk_agent(&pool, "agent-heartbeat")
            .await
            .expect("get agent")
            .expect("agent exists");
        assert_eq!(agent_during_opening.status, HelpdeskAgentStatus::Opening);
        assert_eq!(
            agent_during_opening.current_ticket_id.as_deref(),
            Some(ticket.ticket_id.as_str())
        );

        let (started_ticket, started_agent) =
            start_helpdesk_ticket(&pool, "agent-heartbeat", &ticket.ticket_id)
                .await
                .expect("start ticket after heartbeat");
        assert_eq!(started_ticket.status, HelpdeskTicketStatus::InProgress);
        assert_eq!(started_agent.status, HelpdeskAgentStatus::Busy);
    }

    #[tokio::test]
    async fn expired_opening_ticket_is_requeued() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-expired-opening.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-expire", "Agent Expire").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-expire".to_string(),
                display_name: Some("Agent Expire".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-expire".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Opening timeout".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-expire"),
            Some("manual test"),
        )
        .await
        .expect("assign ticket");

        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET opening_deadline_at = ?2
            WHERE ticket_id = ?1
            "#,
        )
        .bind(&ticket.ticket_id)
        .bind(0_i64)
        .execute(&pool)
        .await
        .expect("force expired deadline");

        let stats = reconcile_helpdesk_runtime(&pool, 60_000, unix_millis_now() as i64)
            .await
            .expect("reconcile runtime");
        assert_eq!(stats.opening_timeouts, 1);

        let ticket_after = get_helpdesk_ticket(&pool, &ticket.ticket_id)
            .await
            .expect("get ticket")
            .expect("ticket exists");
        let agent_after = get_helpdesk_agent(&pool, "agent-expire")
            .await
            .expect("get agent")
            .expect("agent exists");

        assert_eq!(ticket_after.status, HelpdeskTicketStatus::Queued);
        assert!(ticket_after.assigned_agent_id.is_none());
        assert_eq!(agent_after.status, HelpdeskAgentStatus::Available);
    }

    #[tokio::test]
    async fn stale_busy_agent_marks_ticket_failed_and_agent_offline() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-stale-busy.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-stale", "Agent Stale").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-stale".to_string(),
                display_name: Some("Agent Stale".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-stale".to_string(),
                client_display_name: None,
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Busy stale".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-stale"),
            Some("manual test"),
        )
        .await
        .expect("assign ticket");

        start_helpdesk_ticket(&pool, "agent-stale", &ticket.ticket_id)
            .await
            .expect("start ticket");

        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET last_heartbeat_at = 0
            WHERE agent_id = ?1
            "#,
        )
        .bind("agent-stale")
        .execute(&pool)
        .await
        .expect("expire heartbeat");

        let stats = reconcile_helpdesk_runtime(&pool, 1_000, unix_millis_now() as i64)
            .await
            .expect("reconcile runtime");
        assert_eq!(stats.agents_marked_offline, 1);
        assert_eq!(stats.tickets_failed, 1);

        let ticket_after = get_helpdesk_ticket(&pool, &ticket.ticket_id)
            .await
            .expect("get ticket")
            .expect("ticket exists");
        let agent_after = get_helpdesk_agent(&pool, "agent-stale")
            .await
            .expect("get agent")
            .expect("agent exists");

        assert_eq!(ticket_after.status, HelpdeskTicketStatus::Failed);
        assert_eq!(agent_after.status, HelpdeskAgentStatus::Offline);
        assert_eq!(agent_after.current_ticket_id, None);
    }

    #[tokio::test]
    async fn helpdesk_audit_and_summary_are_queryable() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-audit-summary.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-audit", "Agent Audit").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-audit".to_string(),
                display_name: Some("Agent Audit".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-audit".to_string(),
                client_display_name: Some("Cliente Audit".to_string()),
                device_id: Some("device-audit".to_string()),
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Resumen".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-audit"),
            Some("manual test"),
        )
        .await
        .expect("assign ticket");

        start_helpdesk_ticket(&pool, "agent-audit", &ticket.ticket_id)
            .await
            .expect("start ticket");

        let audit = list_helpdesk_ticket_audit_events(&pool, &ticket.ticket_id, 50)
            .await
            .expect("list audit");
        assert!(!audit.is_empty());
        assert!(audit
            .iter()
            .any(|event| event.event_type == "help_request_created"));
        assert!(audit
            .iter()
            .any(|event| event.event_type == "remote_session_started"));

        let summary = get_helpdesk_operational_summary(&pool)
            .await
            .expect("get summary");
        assert_eq!(summary.tickets_in_progress, 1);
        assert_eq!(summary.agents_busy, 1);
    }

    #[tokio::test]
    async fn supervisor_can_assign_queued_ticket_to_specific_agent() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-manual-assign.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-queue-owner", "Agent Queue Owner").await;
        authorize_agent(&pool, "agent-2", "Agent Two").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-queue-owner".to_string(),
                display_name: Some("Agent Queue Owner".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert queue owner");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-manual".to_string(),
                client_display_name: Some("Cliente Manual".to_string()),
                device_id: None,
                requested_by: Some("Supervisor".to_string()),
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Manual dispatch".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-2".to_string(),
                display_name: Some("Agent Two".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent 2");

        let (queued_ticket, released_agent) = requeue_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            HelpdeskAgentStatus::Away,
            Some("dispatch to another agent"),
        )
        .await
        .expect("requeue ticket");

        assert_eq!(queued_ticket.status, HelpdeskTicketStatus::Queued);
        let released_agent = released_agent.expect("released agent exists");
        assert_eq!(released_agent.status, HelpdeskAgentStatus::Away);

        let (assigned_ticket, assigned_agent) = assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-2"),
            Some("manual dispatch"),
        )
        .await
        .expect("assign queued ticket");

        assert_eq!(assigned_ticket.status, HelpdeskTicketStatus::Opening);
        assert_eq!(
            assigned_ticket.assigned_agent_id.as_deref(),
            Some("agent-2")
        );
        assert_eq!(assigned_agent.status, HelpdeskAgentStatus::Opening);
        assert_eq!(
            assigned_agent.current_ticket_id.as_deref(),
            Some(ticket.ticket_id.as_str())
        );
    }

    #[tokio::test]
    async fn supervisor_cannot_assign_without_explicit_agent_selection() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-manual-agent-required.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-required", "Agent Required").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-required".to_string(),
                display_name: Some("Agent Required".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-required".to_string(),
                client_display_name: Some("Cliente Required".to_string()),
                device_id: None,
                requested_by: Some("Supervisor".to_string()),
                title: Some("Manual only".to_string()),
                description: Some("Agent selection required".to_string()),
                difficulty: None,
                estimated_minutes: None,
                summary: Some("manual assign".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        let error =
            assign_helpdesk_ticket(&pool, &ticket.ticket_id, None, Some("no agent selected"))
                .await
                .expect_err("assignment without explicit agent must fail");
        assert!(error.to_string().contains("agent_id is required"));
    }

    #[tokio::test]
    async fn duplicate_authorized_agent_display_name_is_rejected() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-duplicate-authorized-name.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-one", "Edward soporte").await;

        let error = upsert_helpdesk_authorized_agent(
            &pool,
            &HelpdeskAuthorizedAgentUpsertRequestV1 {
                agent_id: "agent-two".to_string(),
                display_name: Some("Edward soporte".to_string()),
                rotate_agent_token: None,
            },
        )
        .await
        .expect_err("duplicate display name must fail");

        assert!(error.to_string().contains("display name 'Edward soporte'"));
    }

    #[tokio::test]
    async fn operational_fields_can_be_updated_after_client_ticket_creation() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-operational-fields.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-operational".to_string(),
                client_display_name: Some("Cliente Operativo".to_string()),
                device_id: None,
                requested_by: Some("Cliente".to_string()),
                title: Some("Issue".to_string()),
                description: Some("Needs triage".to_string()),
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Issue".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        let updated = update_helpdesk_ticket_operational_fields(
            &pool,
            &ticket.ticket_id,
            Some("high"),
            Some(45),
        )
        .await
        .expect("update operational fields");

        assert_eq!(updated.difficulty.as_deref(), Some("high"));
        assert_eq!(updated.estimated_minutes, Some(45));
    }

    #[tokio::test]
    async fn agent_assignment_reflects_operational_fields_updated_by_supervisor() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp
            .path()
            .join("helpdesk-assignment-operational-fields.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-operational", "Agent Operational").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-operational".to_string(),
                display_name: Some("Agent Operational".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-operational-assignment".to_string(),
                client_display_name: Some("Cliente Assignment".to_string()),
                device_id: None,
                requested_by: Some("Cliente".to_string()),
                title: Some("Need triage".to_string()),
                description: Some("Operational fields should reach the agent".to_string()),
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Need triage".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        let updated = update_helpdesk_ticket_operational_fields(
            &pool,
            &ticket.ticket_id,
            Some("medium"),
            Some(30),
        )
        .await
        .expect("update operational fields");

        assert_eq!(updated.difficulty.as_deref(), Some("medium"));
        assert_eq!(updated.estimated_minutes, Some(30));

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-operational"),
            Some("manual dispatch"),
        )
        .await
        .expect("assign ticket");

        let assignment = get_helpdesk_assignment_for_agent(&pool, "agent-operational")
            .await
            .expect("get assignment")
            .expect("assignment should exist");

        assert_eq!(assignment.ticket.difficulty.as_deref(), Some("medium"));
        assert_eq!(assignment.ticket.estimated_minutes, Some(30));
    }

    #[tokio::test]
    async fn agent_report_is_saved_on_ticket_and_visible_in_audit() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-agent-report.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-report", "Agent Report").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-report".to_string(),
                display_name: Some("Agent Report".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-report".to_string(),
                client_display_name: Some("Cliente Reporte".to_string()),
                device_id: None,
                requested_by: Some("Cliente".to_string()),
                title: Some("Need support".to_string()),
                description: Some("First attempt".to_string()),
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Need support".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        assign_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            Some("agent-report"),
            Some("manual dispatch"),
        )
        .await
        .expect("assign ticket");

        start_helpdesk_ticket(&pool, "agent-report", &ticket.ticket_id)
            .await
            .expect("start assignment");

        let updated = add_helpdesk_ticket_agent_report(
            &pool,
            &ticket.ticket_id,
            "agent-report",
            "VPN still failing after credential reset. Next agent should verify gateway policy and test with the backup profile.",
        )
        .await
        .expect("add agent report");

        assert!(updated
            .latest_agent_report
            .as_deref()
            .unwrap_or_default()
            .contains("backup profile"));
        assert_eq!(
            updated.latest_agent_report_by.as_deref(),
            Some("Agent Report")
        );
        assert!(updated.latest_agent_report_at.is_some());

        let audit = list_helpdesk_ticket_audit_events(&pool, &ticket.ticket_id, 50)
            .await
            .expect("list audit");
        assert!(audit
            .iter()
            .any(|event| event.event_type == "agent_report_added"));

        let requeued = requeue_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            HelpdeskAgentStatus::Available,
            Some("handoff to another agent"),
        )
        .await
        .expect("requeue ticket")
        .0;
        assert!(requeued
            .latest_agent_report
            .as_deref()
            .unwrap_or_default()
            .contains("gateway policy"));
    }

    #[tokio::test]
    async fn supervisor_can_requeue_opening_ticket() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-requeue.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-requeue", "Agent Requeue").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-requeue".to_string(),
                display_name: Some("Agent Requeue".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-requeue".to_string(),
                client_display_name: Some("Cliente Requeue".to_string()),
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Supervisor requeue".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        let (ticket_after, agent_after) = requeue_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            HelpdeskAgentStatus::Available,
            Some("retry"),
        )
        .await
        .expect("requeue ticket");

        assert_eq!(ticket_after.status, HelpdeskTicketStatus::Queued);
        assert!(ticket_after.assigned_agent_id.is_none());
        let agent_after = agent_after.expect("agent after");
        assert_eq!(agent_after.status, HelpdeskAgentStatus::Available);
        assert!(agent_after.current_ticket_id.is_none());
    }

    #[tokio::test]
    async fn supervisor_can_cancel_in_progress_ticket() {
        let temp = tempdir().expect("create temp dir");
        let db_path = temp.path().join("helpdesk-cancel.db");
        let pool = connect_sqlite(&db_path).await.expect("connect sqlite");
        authorize_agent(&pool, "agent-cancel", "Agent Cancel").await;

        upsert_helpdesk_agent_presence(
            &pool,
            &HelpdeskAgentPresenceUpdateV1 {
                agent_id: "agent-cancel".to_string(),
                display_name: Some("Agent Cancel".to_string()),
                avatar_url: None,
                status: HelpdeskAgentStatus::Available,
            },
        )
        .await
        .expect("upsert agent");

        let ticket = create_helpdesk_ticket(
            &pool,
            &HelpdeskTicketCreateRequestV1 {
                client_id: "client-cancel".to_string(),
                client_display_name: Some("Cliente Cancel".to_string()),
                device_id: None,
                requested_by: None,
                title: None,
                description: None,
                difficulty: None,
                estimated_minutes: None,
                summary: Some("Supervisor cancel".to_string()),
                preferred_agent_id: None,
            },
        )
        .await
        .expect("create ticket");

        start_helpdesk_ticket(&pool, "agent-cancel", &ticket.ticket_id)
            .await
            .expect("start ticket");

        let (ticket_after, agent_after) = cancel_helpdesk_ticket(
            &pool,
            &ticket.ticket_id,
            HelpdeskAgentStatus::Away,
            Some("operator unavailable"),
        )
        .await
        .expect("cancel ticket");

        assert_eq!(ticket_after.status, HelpdeskTicketStatus::Cancelled);
        let agent_after = agent_after.expect("agent after");
        assert_eq!(agent_after.status, HelpdeskAgentStatus::Away);
        assert!(agent_after.current_ticket_id.is_none());
    }
}
