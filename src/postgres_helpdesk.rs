use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::model::{
    HelpdeskAgentStatus, HelpdeskAgentV1, HelpdeskAuditEventV1,
    HelpdeskAuthorizedAgentUpsertRequestV1, HelpdeskAuthorizedAgentV1,
    HelpdeskOperationalSummaryV1, HelpdeskTicketCreateRequestV1, HelpdeskTicketStatus,
    HelpdeskTicketV1,
};
pub async fn upsert_helpdesk_authorized_agent_pg(
    pool: &PgPool,
    payload: &HelpdeskAuthorizedAgentUpsertRequestV1,
) -> Result<HelpdeskAuthorizedAgentV1> {
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    let display_name = normalize_optional_text(payload.display_name.as_deref());
    ensure_helpdesk_agent_display_name_available_pg(pool, &agent_id, display_name.as_deref()).await?;
    let now_ms = unix_millis_now();

    let row = sqlx::query(
        r#"
        INSERT INTO helpdesk_authorized_agents (agent_id, display_name, created_at, updated_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (agent_id) DO UPDATE SET
            display_name = COALESCE($2, helpdesk_authorized_agents.display_name),
            updated_at = $4
        RETURNING agent_id, display_name, created_at, updated_at
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to upsert Postgres authorized agent '{}'", agent_id))?;

    row_to_helpdesk_authorized_agent_pg(row)
}

pub async fn list_helpdesk_authorized_agents_pg(pool: &PgPool) -> Result<Vec<HelpdeskAuthorizedAgentV1>> {
    let rows = sqlx::query(
        r#"
        SELECT agent_id, display_name, created_at, updated_at
        FROM helpdesk_authorized_agents
        ORDER BY created_at ASC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list Postgres authorized agents")?;

    rows.into_iter().map(row_to_helpdesk_authorized_agent_pg).collect()
}

pub async fn delete_helpdesk_authorized_agent_pg(pool: &PgPool, agent_id: &str) -> Result<bool> {
    let result = sqlx::query(
        r#"
        DELETE FROM helpdesk_authorized_agents
        WHERE agent_id = $1
        "#,
    )
    .bind(normalize_helpdesk_agent_id(agent_id))
    .execute(pool)
    .await
    .with_context(|| format!("failed to delete Postgres authorized agent '{}'", agent_id))?;

    Ok(result.rows_affected() > 0)
}

pub async fn list_helpdesk_agents_pg(pool: &PgPool) -> Result<Vec<HelpdeskAgentV1>> {
    let rows = sqlx::query(
        r#"
        SELECT agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, updated_at
        FROM helpdesk_agents
        ORDER BY created_at ASC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list Postgres helpdesk agents")?;

    rows.into_iter().map(row_to_helpdesk_agent_pg).collect()
}

pub async fn get_helpdesk_operational_summary_pg(pool: &PgPool) -> Result<HelpdeskOperationalSummaryV1> {
    let ticket_rows = sqlx::query(
        r#"
        SELECT status, COUNT(*) AS count
        FROM helpdesk_tickets
        GROUP BY status
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query Postgres helpdesk ticket summary")?;

    let agent_rows = sqlx::query(
        r#"
        SELECT status, COUNT(*) AS count
        FROM helpdesk_agents
        GROUP BY status
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to query Postgres helpdesk agent summary")?;

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
        let count = i64_to_u64(row.get("count"));
        match helpdesk_ticket_status_from_db(&status) {
            HelpdeskTicketStatus::New => summary.tickets_new = count,
            HelpdeskTicketStatus::Queued | HelpdeskTicketStatus::Assigned => summary.tickets_queued = count,
            HelpdeskTicketStatus::Opening => summary.tickets_opening = count,
            HelpdeskTicketStatus::InProgress => summary.tickets_in_progress = count,
            HelpdeskTicketStatus::Resolved => summary.tickets_resolved = count,
            HelpdeskTicketStatus::Cancelled => summary.tickets_cancelled = count,
            HelpdeskTicketStatus::Failed => summary.tickets_failed = count,
        }
    }

    for row in agent_rows {
        let status: String = row.get("status");
        let count = i64_to_u64(row.get("count"));
        match helpdesk_agent_status_from_db(&status) {
            HelpdeskAgentStatus::Offline => summary.agents_offline = count,
            HelpdeskAgentStatus::Available => summary.agents_available = count,
            HelpdeskAgentStatus::Opening => summary.agents_opening = count,
            HelpdeskAgentStatus::Busy => summary.agents_busy = count,
            HelpdeskAgentStatus::Away => summary.agents_away = count,
        }
    }

    Ok(summary)
}

pub async fn create_helpdesk_ticket_pg(
    pool: &PgPool,
    payload: &HelpdeskTicketCreateRequestV1,
) -> Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now();
    let ticket_id = Uuid::new_v4().to_string();
    let normalized_title = normalize_optional_text(payload.title.as_deref());
    let normalized_summary =
        normalize_optional_text(payload.summary.as_deref()).or_else(|| normalized_title.clone());

    let row = sqlx::query(
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
            latest_agent_report,
            latest_agent_report_by,
            latest_agent_report_at,
            opening_deadline_at,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, NULL, $8, 'queued', NULL, NULL, NULL, NULL, NULL, $9, $10)
        RETURNING ticket_id, client_id, client_display_name, device_id, requested_by, title,
                  description, difficulty, estimated_minutes, summary, status, assigned_agent_id,
                  latest_agent_report, latest_agent_report_by, latest_agent_report_at,
                  opening_deadline_at, created_at, updated_at
        "#,
    )
    .bind(&ticket_id)
    .bind(payload.client_id.trim())
    .bind(normalize_optional_text(payload.client_display_name.as_deref()))
    .bind(normalize_optional_text(payload.device_id.as_deref()))
    .bind(normalize_optional_text(payload.requested_by.as_deref()))
    .bind(normalized_title)
    .bind(normalize_optional_text(payload.description.as_deref()))
    .bind(normalized_summary)
    .bind(now_ms)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to create Postgres helpdesk ticket '{}'", ticket_id))?;

    append_helpdesk_audit_event_pg(
        pool,
        "ticket",
        &ticket_id,
        "help_request_created",
        Some(serde_json::json!({
            "client_id": payload.client_id.trim(),
        })),
    )
    .await?;

    row_to_helpdesk_ticket_pg(row)
}

pub async fn list_helpdesk_tickets_pg(pool: &PgPool) -> Result<Vec<HelpdeskTicketV1>> {
    let rows = sqlx::query(
        r#"
        SELECT ticket_id, client_id, client_display_name, device_id, requested_by, title,
               description, difficulty, estimated_minutes, summary, status, assigned_agent_id,
               latest_agent_report, latest_agent_report_by, latest_agent_report_at,
               opening_deadline_at, created_at, updated_at
        FROM helpdesk_tickets
        ORDER BY created_at DESC, ticket_id DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list Postgres helpdesk tickets")?;

    rows.into_iter().map(row_to_helpdesk_ticket_pg).collect()
}

pub async fn get_helpdesk_ticket_pg(pool: &PgPool, ticket_id: &str) -> Result<Option<HelpdeskTicketV1>> {
    let row = sqlx::query(
        r#"
        SELECT ticket_id, client_id, client_display_name, device_id, requested_by, title,
               description, difficulty, estimated_minutes, summary, status, assigned_agent_id,
               latest_agent_report, latest_agent_report_by, latest_agent_report_at,
               opening_deadline_at, created_at, updated_at
        FROM helpdesk_tickets
        WHERE ticket_id = $1
        "#,
    )
    .bind(ticket_id.trim())
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to get Postgres helpdesk ticket '{}'", ticket_id))?;

    row.map(row_to_helpdesk_ticket_pg).transpose()
}

pub async fn append_helpdesk_audit_event_pg(
    pool: &PgPool,
    entity_type: &str,
    entity_id: &str,
    event_type: &str,
    payload: Option<Value>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO helpdesk_audit_events (entity_type, entity_id, event_type, payload, created_at)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(entity_type.trim())
    .bind(entity_id.trim())
    .bind(event_type.trim())
    .bind(payload.map(|value| value.to_string()))
    .bind(unix_millis_now())
    .execute(pool)
    .await
    .with_context(|| format!("failed to append Postgres audit event '{}' for '{}'", event_type, entity_id))?;
    Ok(())
}

pub async fn list_helpdesk_ticket_audit_pg(pool: &PgPool, ticket_id: &str) -> Result<Vec<HelpdeskAuditEventV1>> {
    let rows = sqlx::query(
        r#"
        SELECT entity_type, entity_id, event_type, payload, created_at
        FROM helpdesk_audit_events
        WHERE entity_type = 'ticket' AND entity_id = $1
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(ticket_id.trim())
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list Postgres helpdesk audit for '{}'", ticket_id))?;

    rows.into_iter().map(row_to_helpdesk_audit_event_pg).collect()
}

fn row_to_helpdesk_authorized_agent_pg(row: PgRow) -> Result<HelpdeskAuthorizedAgentV1> {
    Ok(HelpdeskAuthorizedAgentV1 {
        agent_id: normalize_helpdesk_agent_id(&row.get::<String, _>("agent_id")),
        display_name: row.get("display_name"),
        created_at: millis_to_utc(row.get("created_at")),
        updated_at: millis_to_utc(row.get("updated_at")),
    })
}

fn row_to_helpdesk_ticket_pg(row: PgRow) -> Result<HelpdeskTicketV1> {
    let status: String = row.get("status");
    let latest_agent_report_at: Option<i64> = row.get("latest_agent_report_at");
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
        latest_agent_report_at: latest_agent_report_at.map(millis_to_utc),
        opening_deadline_at: opening_deadline_at.map(millis_to_utc),
        created_at: millis_to_utc(row.get("created_at")),
        updated_at: millis_to_utc(row.get("updated_at")),
    })
}

fn row_to_helpdesk_agent_pg(row: PgRow) -> Result<HelpdeskAgentV1> {
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

fn row_to_helpdesk_audit_event_pg(row: PgRow) -> Result<HelpdeskAuditEventV1> {
    let payload: Option<String> = row.get("payload");
    Ok(HelpdeskAuditEventV1 {
        entity_type: row.get("entity_type"),
        entity_id: row.get("entity_id"),
        event_type: row.get("event_type"),
        payload: payload
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("failed to deserialize Postgres helpdesk audit payload")?,
        created_at: millis_to_utc(row.get("created_at")),
    })
}

async fn ensure_helpdesk_agent_display_name_available_pg(
    pool: &PgPool,
    agent_id: &str,
    display_name: Option<&str>,
) -> Result<()> {
    let Some(display_name) = display_name.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };

    let conflicting_authorized_agent = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT agent_id
        FROM helpdesk_authorized_agents
        WHERE agent_id != $1
          AND lower(trim(display_name)) = lower(trim($2))
        LIMIT 1
        "#,
    )
    .bind(agent_id)
    .bind(display_name)
    .fetch_optional(pool)
    .await
    .context("failed to validate Postgres authorized agent display name uniqueness")?
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
        WHERE agent_id != $1
          AND lower(trim(display_name)) = lower(trim($2))
        LIMIT 1
        "#,
    )
    .bind(agent_id)
    .bind(display_name)
    .fetch_optional(pool)
    .await
    .context("failed to validate Postgres live agent display name uniqueness")?
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

fn normalize_helpdesk_agent_id(raw: &str) -> String {
    raw.trim().chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn unix_millis_now() -> i64 {
    Utc::now().timestamp_millis()
}

fn millis_to_utc(value: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("unix epoch should exist"))
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

fn helpdesk_agent_status_from_db(raw: &str) -> HelpdeskAgentStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "available" => HelpdeskAgentStatus::Available,
        "opening" => HelpdeskAgentStatus::Opening,
        "busy" => HelpdeskAgentStatus::Busy,
        "away" => HelpdeskAgentStatus::Away,
        _ => HelpdeskAgentStatus::Offline,
    }
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}
