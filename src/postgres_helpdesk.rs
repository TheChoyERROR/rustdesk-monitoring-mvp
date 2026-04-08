use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::MonitoringConfig;
use crate::helpdesk_agent_auth::{
    generate_helpdesk_agent_token, hash_helpdesk_agent_token, helpdesk_agent_token_hint,
};
use crate::model::{
    HelpdeskAgentAuthorizationStatusV1, HelpdeskAgentPresenceUpdateV1, HelpdeskAgentStatus,
    HelpdeskAgentV1, HelpdeskAssignmentV1, HelpdeskAuditEventV1,
    HelpdeskAuthorizedAgentProvisioningV1, HelpdeskAuthorizedAgentUpsertRequestV1,
    HelpdeskAuthorizedAgentV1, HelpdeskOperationalSummaryV1, HelpdeskTicketCreateRequestV1,
    HelpdeskTicketStatus, HelpdeskTicketV1, SessionEventType, SessionEventV1,
};
use crate::storage::HelpdeskRuntimeReconcileResult;

const HELPDESK_OPENING_WINDOW_MS: i64 = 30_000;

pub async fn ensure_postgres_helpdesk_agent_auth_schema(pool: &PgPool) -> Result<()> {
    for statement in [
        "ALTER TABLE helpdesk_authorized_agents ADD COLUMN IF NOT EXISTS agent_token_hash TEXT",
        "ALTER TABLE helpdesk_authorized_agents ADD COLUMN IF NOT EXISTS agent_token_hint TEXT",
        "ALTER TABLE helpdesk_authorized_agents ADD COLUMN IF NOT EXISTS agent_token_rotated_at BIGINT",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .context("failed to apply Postgres helpdesk agent auth schema change")?;
    }
    Ok(())
}

pub async fn upsert_helpdesk_authorized_agent_pg(
    pool: &PgPool,
    payload: &HelpdeskAuthorizedAgentUpsertRequestV1,
) -> Result<HelpdeskAuthorizedAgentV1> {
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    let display_name = normalize_optional_text(payload.display_name.as_deref());
    ensure_helpdesk_agent_display_name_available_pg(pool, &agent_id, display_name.as_deref())
        .await?;
    let now_ms = unix_millis_now();

    let row = sqlx::query(
        r#"
        INSERT INTO helpdesk_authorized_agents (agent_id, display_name, created_at, updated_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (agent_id) DO UPDATE SET
            display_name = COALESCE($2, helpdesk_authorized_agents.display_name),
            updated_at = $4
        RETURNING
            agent_id,
            display_name,
            agent_token_hash,
            agent_token_hint,
            agent_token_rotated_at,
            created_at,
            updated_at
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to upsert Postgres authorized agent '{}'", agent_id))?;

    sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET display_name = $2,
            updated_at = $3
        WHERE agent_id = $1
          AND $2 IS NOT NULL
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to align Postgres live helpdesk agent display name for '{}'",
            agent_id
        )
    })?;

    row_to_helpdesk_authorized_agent_pg(row)
}

pub async fn provision_helpdesk_authorized_agent_pg(
    pool: &PgPool,
    payload: &HelpdeskAuthorizedAgentUpsertRequestV1,
) -> Result<HelpdeskAuthorizedAgentProvisioningV1> {
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    let existing_agent = get_helpdesk_authorized_agent_pg(pool, &agent_id).await?;
    let display_name = normalize_optional_text(payload.display_name.as_deref()).or_else(|| {
        existing_agent
            .as_ref()
            .and_then(|agent| agent.display_name.clone())
    });
    ensure_helpdesk_agent_display_name_available_pg(pool, &agent_id, display_name.as_deref())
        .await?;

    let now_ms = unix_millis_now();
    let issued_token = generate_helpdesk_agent_token();
    let token_hash = hash_helpdesk_agent_token(&issued_token);
    let token_hint = helpdesk_agent_token_hint(&issued_token);

    let row = sqlx::query(
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
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (agent_id) DO UPDATE SET
            display_name = COALESCE($2, helpdesk_authorized_agents.display_name),
            agent_token_hash = $3,
            agent_token_hint = $4,
            agent_token_rotated_at = $5,
            updated_at = $7
        RETURNING
            agent_id,
            display_name,
            agent_token_hash,
            agent_token_hint,
            agent_token_rotated_at,
            created_at,
            updated_at
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(&token_hash)
    .bind(token_hint.as_deref())
    .bind(now_ms)
    .bind(now_ms)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .with_context(|| {
        format!(
            "failed to provision Postgres authorized agent '{}'",
            agent_id
        )
    })?;

    sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET display_name = $2,
            updated_at = $3
        WHERE agent_id = $1
          AND $2 IS NOT NULL
        "#,
    )
    .bind(&agent_id)
    .bind(display_name.as_deref())
    .bind(now_ms)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to align Postgres live helpdesk agent display name for '{}'",
            agent_id
        )
    })?;

    Ok(HelpdeskAuthorizedAgentProvisioningV1 {
        agent: row_to_helpdesk_authorized_agent_pg(row)?,
        agent_token: issued_token,
    })
}

pub async fn list_helpdesk_authorized_agents_pg(
    pool: &PgPool,
) -> Result<Vec<HelpdeskAuthorizedAgentV1>> {
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
        ORDER BY created_at ASC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list Postgres authorized agents")?;

    rows.into_iter()
        .map(row_to_helpdesk_authorized_agent_pg)
        .collect()
}

pub async fn delete_helpdesk_authorized_agent_pg(pool: &PgPool, agent_id: &str) -> Result<bool> {
    let now_ms = unix_millis_now();
    let agent_id = normalize_helpdesk_agent_id(agent_id);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres authorized helpdesk agent delete transaction")?;

    let current_ticket_id = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT current_ticket_id
        FROM helpdesk_agents
        WHERE agent_id = $1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to query current ticket before deleting Postgres authorized helpdesk agent '{}'",
            agent_id
        )
    })?
    .flatten();

    let result = sqlx::query(
        r#"
        DELETE FROM helpdesk_authorized_agents
        WHERE agent_id = $1
        "#,
    )
    .bind(&agent_id)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to delete Postgres authorized agent '{}'", agent_id))?;

    sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = 'offline',
            current_ticket_id = NULL,
            updated_at = $2
        WHERE agent_id = $1
        "#,
    )
    .bind(&agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to offboard Postgres live helpdesk agent '{}'",
            agent_id
        )
    })?;

    if let Some(ticket_id) = current_ticket_id {
        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET status = 'queued',
                assigned_agent_id = NULL,
                opening_deadline_at = NULL,
                updated_at = $2
            WHERE ticket_id = $1
              AND assigned_agent_id = $3
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
                "failed to requeue ticket '{}' while deleting Postgres authorized helpdesk agent '{}'",
                ticket_id, agent_id
            )
        })?;
    }

    tx.commit()
        .await
        .context("failed to commit Postgres authorized helpdesk agent delete transaction")?;

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

pub async fn get_helpdesk_operational_summary_pg(
    pool: &PgPool,
) -> Result<HelpdeskOperationalSummaryV1> {
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
            HelpdeskTicketStatus::Queued | HelpdeskTicketStatus::Assigned => {
                summary.tickets_queued = count
            }
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

pub async fn get_helpdesk_agent_authorization_status_pg(
    pool: &PgPool,
    agent_id: &str,
) -> Result<HelpdeskAgentAuthorizationStatusV1> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let authorized_agent = get_helpdesk_authorized_agent_pg(pool, &agent_id).await?;

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

pub async fn verify_helpdesk_agent_token_pg(
    pool: &PgPool,
    agent_id: &str,
    raw_token: &str,
) -> Result<bool> {
    let normalized_agent_id = normalize_helpdesk_agent_id(agent_id);
    let token_hash = hash_helpdesk_agent_token(raw_token);
    let stored_hash = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT agent_token_hash
        FROM helpdesk_authorized_agents
        WHERE agent_id = $1
        LIMIT 1
        "#,
    )
    .bind(&normalized_agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query Postgres helpdesk agent token hash for '{}'",
            normalized_agent_id
        )
    })?
    .flatten();

    Ok(matches!(stored_hash.as_deref(), Some(value) if value == token_hash))
}

pub async fn upsert_helpdesk_agent_presence_pg(
    pool: &PgPool,
    payload: &HelpdeskAgentPresenceUpdateV1,
) -> Result<HelpdeskAgentV1> {
    let now_ms = unix_millis_now();
    let agent_id = normalize_helpdesk_agent_id(&payload.agent_id);
    let authorized_agent = get_helpdesk_authorized_agent_pg(pool, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "agent '{}' is not authorized for helpdesk operator mode",
                agent_id
            )
        })?;

    let fallback_display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&agent_id)
        .to_string();

    let display_name = authorized_agent
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or(fallback_display_name);

    ensure_helpdesk_agent_display_name_available_pg(pool, &agent_id, Some(&display_name)).await?;

    let avatar_url = payload
        .avatar_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk agent transaction")?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_agents (
            agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, NULL, $5, $6, $7)
        ON CONFLICT (agent_id) DO UPDATE SET
            display_name = CASE
                WHEN trim($2) = '' THEN helpdesk_agents.display_name
                ELSE $2
            END,
            avatar_url = COALESCE($3, helpdesk_agents.avatar_url),
            status = CASE
                WHEN helpdesk_agents.status IN ('opening', 'busy')
                     AND helpdesk_agents.current_ticket_id IS NOT NULL
                     AND $4 IN ('available', 'away', 'offline')
                THEN helpdesk_agents.status
                ELSE $4
            END,
            current_ticket_id = CASE
                WHEN helpdesk_agents.status IN ('opening', 'busy')
                     AND helpdesk_agents.current_ticket_id IS NOT NULL
                THEN helpdesk_agents.current_ticket_id
                WHEN $4 IN ('available', 'away', 'offline') THEN NULL
                ELSE helpdesk_agents.current_ticket_id
            END,
            last_heartbeat_at = $5,
            updated_at = $7
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
    .with_context(|| format!("failed to upsert Postgres helpdesk agent '{}'", agent_id))?;

    sqlx::query(
        r#"
        INSERT INTO helpdesk_agent_heartbeats (agent_id, status, created_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(&agent_id)
    .bind(payload.status.as_str())
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to insert Postgres agent heartbeat '{}'", agent_id))?;

    append_helpdesk_audit_event_pg_tx(
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

    let agent = get_helpdesk_agent_pg_tx(&mut tx, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk agent '{}' not found after upsert",
                agent_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk agent transaction")?;

    Ok(agent)
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

pub async fn assign_helpdesk_ticket_pg(
    pool: &PgPool,
    ticket_id: &str,
    requested_agent_id: Option<&str>,
    reason: Option<&str>,
) -> Result<(HelpdeskTicketV1, HelpdeskAgentV1)> {
    let now_ms = unix_millis_now();
    let ticket_id = ticket_id.trim();
    let requested_agent_id = normalize_optional_text(requested_agent_id);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk assign transaction")?;

    let current_ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found before assign",
                ticket_id
            )
        })?;

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

    let assigned = assign_helpdesk_ticket_to_agent_pg_tx(
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

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after assign",
                ticket_id
            )
        })?;
    let agent = get_helpdesk_agent_pg_tx(&mut tx, selected_agent_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk agent '{}' not found after assign",
                selected_agent_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk assign transaction")?;

    Ok((ticket, agent))
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

pub async fn get_helpdesk_ticket_pg(
    pool: &PgPool,
    ticket_id: &str,
) -> Result<Option<HelpdeskTicketV1>> {
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

pub async fn get_helpdesk_assignment_for_agent_pg(
    pool: &PgPool,
    agent_id: &str,
) -> Result<Option<HelpdeskAssignmentV1>> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);

    let row = sqlx::query(
        r#"
        SELECT current_ticket_id
        FROM helpdesk_agents
        WHERE agent_id = $1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query current Postgres assignment for agent '{}'",
            agent_id
        )
    })?;

    let Some(row) = row else {
        return Ok(None);
    };

    let Some(ticket_id): Option<String> = row.get("current_ticket_id") else {
        return Ok(None);
    };

    let agent = get_helpdesk_agent_pg(pool, &agent_id)
        .await?
        .with_context(|| format!("missing Postgres helpdesk agent '{}'", agent_id))?;
    let ticket = get_helpdesk_ticket_pg(pool, &ticket_id)
        .await?
        .with_context(|| format!("missing Postgres helpdesk ticket '{}'", ticket_id))?;

    Ok(Some(HelpdeskAssignmentV1 { ticket, agent }))
}

pub async fn start_helpdesk_ticket_pg(
    pool: &PgPool,
    agent_id: &str,
    ticket_id: &str,
) -> Result<(HelpdeskTicketV1, HelpdeskAgentV1)> {
    let now_ms = unix_millis_now();
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk start transaction")?;

    let ticket_update = sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'in_progress',
            opening_deadline_at = NULL,
            updated_at = $3
        WHERE ticket_id = $1
          AND assigned_agent_id = $2
          AND status = 'opening'
        "#,
    )
    .bind(ticket_id)
    .bind(&agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to start Postgres helpdesk ticket '{}'", ticket_id))?;

    if ticket_update.rows_affected() == 0 {
        anyhow::bail!("ticket is not in opening state for this agent");
    }

    let agent_update = sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = 'busy',
            current_ticket_id = $2,
            updated_at = $3
        WHERE agent_id = $1
          AND status = 'opening'
          AND current_ticket_id = $2
        "#,
    )
    .bind(&agent_id)
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to move Postgres helpdesk agent '{}' to busy",
            agent_id
        )
    })?;

    if agent_update.rows_affected() == 0 {
        anyhow::bail!("agent is not in opening state for this ticket");
    }

    sqlx::query(
        r#"
        UPDATE helpdesk_ticket_assignments
        SET status = 'in_progress', updated_at = $3
        WHERE ticket_id = $1
          AND agent_id = $2
          AND status = 'opening'
        "#,
    )
    .bind(ticket_id)
    .bind(&agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to update Postgres assignment state for ticket '{}'",
            ticket_id
        )
    })?;

    append_helpdesk_audit_event_pg_tx(
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

    append_helpdesk_audit_event_pg_tx(
        &mut tx,
        "agent",
        &agent_id,
        "agent_became_busy",
        Some(serde_json::json!({
            "ticket_id": ticket_id,
        })),
        now_ms,
    )
    .await?;

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after start",
                ticket_id
            )
        })?;
    let agent = get_helpdesk_agent_pg_tx(&mut tx, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk agent '{}' not found after start",
                agent_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk start transaction")?;

    Ok((ticket, agent))
}

pub async fn update_helpdesk_ticket_operational_fields_pg(
    pool: &PgPool,
    ticket_id: &str,
    difficulty: Option<&str>,
    estimated_minutes: Option<u32>,
) -> Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now();
    let ticket_id = ticket_id.trim();
    let normalized_difficulty = normalize_optional_text(difficulty);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk ticket operational update transaction")?;

    let current_ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found before operational update",
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
        SET difficulty = COALESCE($2, difficulty),
            estimated_minutes = COALESCE($3, estimated_minutes),
            updated_at = $4
        WHERE ticket_id = $1
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
            "failed to update operational fields for Postgres helpdesk ticket '{}'",
            ticket_id
        )
    })?;

    append_helpdesk_audit_event_pg_tx(
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

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after operational update",
                ticket_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk ticket operational update transaction")?;

    Ok(ticket)
}

pub async fn add_helpdesk_ticket_agent_report_pg(
    pool: &PgPool,
    ticket_id: &str,
    agent_id: &str,
    note: &str,
) -> Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now();
    let ticket_id = ticket_id.trim();
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let normalized_note =
        normalize_optional_text(Some(note)).context("agent report note cannot be empty")?;

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk agent report transaction")?;

    let current_ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found before agent report",
                ticket_id
            )
        })?;

    if current_ticket.assigned_agent_id.as_deref() != Some(agent_id.as_str()) {
        anyhow::bail!("ticket is not currently assigned to this agent");
    }

    if !matches!(
        current_ticket.status,
        HelpdeskTicketStatus::Opening | HelpdeskTicketStatus::InProgress
    ) {
        anyhow::bail!("ticket is not active for agent reporting");
    }

    let agent = get_helpdesk_agent_pg_tx(&mut tx, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk agent '{}' not found before agent report",
                agent_id
            )
        })?;

    sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET latest_agent_report = $2,
            latest_agent_report_by = $3,
            latest_agent_report_at = $4,
            updated_at = $4
        WHERE ticket_id = $1
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
            "failed to store latest Postgres agent report for ticket '{}'",
            ticket_id
        )
    })?;

    append_helpdesk_audit_event_pg_tx(
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

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after agent report",
                ticket_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk agent report transaction")?;

    Ok(ticket)
}

pub async fn resolve_helpdesk_ticket_pg(
    pool: &PgPool,
    ticket_id: &str,
    agent_id: &str,
    next_agent_status: HelpdeskAgentStatus,
) -> Result<(HelpdeskTicketV1, HelpdeskAgentV1)> {
    let now_ms = unix_millis_now();
    let ticket_id = ticket_id.trim();
    let agent_id = normalize_helpdesk_agent_id(agent_id);

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk resolve transaction")?;

    let ticket_update = sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'resolved',
            opening_deadline_at = NULL,
            updated_at = $3
        WHERE ticket_id = $1
          AND assigned_agent_id = $2
          AND status = 'in_progress'
        "#,
    )
    .bind(ticket_id)
    .bind(&agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to resolve Postgres helpdesk ticket '{}'", ticket_id))?;

    if ticket_update.rows_affected() == 0 {
        anyhow::bail!("ticket is not in progress for this agent");
    }

    let agent_status = next_agent_status.as_str();
    let agent_update = sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = $3,
            current_ticket_id = NULL,
            updated_at = $4
        WHERE agent_id = $1
          AND current_ticket_id = $2
          AND status = 'busy'
        "#,
    )
    .bind(&agent_id)
    .bind(ticket_id)
    .bind(agent_status)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to release Postgres helpdesk agent '{}'", agent_id))?;

    if agent_update.rows_affected() == 0 {
        anyhow::bail!("agent is not busy with this ticket");
    }

    sqlx::query(
        r#"
        UPDATE helpdesk_ticket_assignments
        SET status = 'resolved', updated_at = $3
        WHERE ticket_id = $1
          AND agent_id = $2
          AND status IN ('opening', 'in_progress')
        "#,
    )
    .bind(ticket_id)
    .bind(&agent_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to resolve Postgres assignment for ticket '{}'",
            ticket_id
        )
    })?;

    append_helpdesk_audit_event_pg_tx(
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

    append_helpdesk_audit_event_pg_tx(
        &mut tx,
        "agent",
        &agent_id,
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

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after resolve",
                ticket_id
            )
        })?;
    let agent = get_helpdesk_agent_pg_tx(&mut tx, &agent_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk agent '{}' not found after resolve",
                agent_id
            )
        })?;

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk resolve transaction")?;

    Ok((ticket, agent))
}

pub async fn requeue_helpdesk_ticket_pg(
    pool: &PgPool,
    ticket_id: &str,
    next_agent_status: HelpdeskAgentStatus,
    reason: Option<&str>,
) -> Result<(HelpdeskTicketV1, Option<HelpdeskAgentV1>)> {
    let now_ms = unix_millis_now();
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk requeue transaction")?;

    let current_ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found before requeue",
                ticket_id
            )
        })?;

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
            updated_at = $2
        WHERE ticket_id = $1
        "#,
    )
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to requeue Postgres helpdesk ticket '{}'", ticket_id))?;

    if let Some(agent_id) = assigned_agent_id.as_deref() {
        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET status = $3,
                current_ticket_id = NULL,
                updated_at = $4
            WHERE agent_id = $1
              AND current_ticket_id = $2
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
                "failed to release Postgres helpdesk agent '{}' during requeue",
                agent_id
            )
        })?;

        sqlx::query(
            r#"
            UPDATE helpdesk_ticket_assignments
            SET status = 'requeued', updated_at = $3
            WHERE ticket_id = $1
              AND agent_id = $2
              AND status IN ('opening', 'in_progress')
            "#,
        )
        .bind(ticket_id)
        .bind(agent_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to requeue Postgres assignment for ticket '{}'",
                ticket_id
            )
        })?;

        append_helpdesk_audit_event_pg_tx(
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

    append_helpdesk_audit_event_pg_tx(
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

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after requeue",
                ticket_id
            )
        })?;
    let agent = if let Some(agent_id) = current_ticket.assigned_agent_id.as_deref() {
        get_helpdesk_agent_pg_tx(&mut tx, agent_id).await?
    } else {
        None
    };

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk requeue transaction")?;

    Ok((ticket, agent))
}

pub async fn cancel_helpdesk_ticket_pg(
    pool: &PgPool,
    ticket_id: &str,
    next_agent_status: HelpdeskAgentStatus,
    reason: Option<&str>,
) -> Result<(HelpdeskTicketV1, Option<HelpdeskAgentV1>)> {
    let now_ms = unix_millis_now();
    let ticket_id = ticket_id.trim();

    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk cancel transaction")?;

    let current_ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found before cancel",
                ticket_id
            )
        })?;

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
            updated_at = $2
        WHERE ticket_id = $1
        "#,
    )
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to cancel Postgres helpdesk ticket '{}'", ticket_id))?;

    if let Some(agent_id) = assigned_agent_id.as_deref() {
        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET status = $3,
                current_ticket_id = NULL,
                updated_at = $4
            WHERE agent_id = $1
              AND current_ticket_id = $2
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
                "failed to release Postgres helpdesk agent '{}' during cancel",
                agent_id
            )
        })?;

        sqlx::query(
            r#"
            UPDATE helpdesk_ticket_assignments
            SET status = 'cancelled', updated_at = $3
            WHERE ticket_id = $1
              AND agent_id = $2
              AND status IN ('opening', 'in_progress')
            "#,
        )
        .bind(ticket_id)
        .bind(agent_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to cancel Postgres assignment for ticket '{}'",
                ticket_id
            )
        })?;

        append_helpdesk_audit_event_pg_tx(
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

    append_helpdesk_audit_event_pg_tx(
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

    let ticket = get_helpdesk_ticket_pg_tx(&mut tx, ticket_id)
        .await?
        .with_context(|| {
            format!(
                "Postgres helpdesk ticket '{}' not found after cancel",
                ticket_id
            )
        })?;
    let agent = if let Some(agent_id) = current_ticket.assigned_agent_id.as_deref() {
        get_helpdesk_agent_pg_tx(&mut tx, agent_id).await?
    } else {
        None
    };

    tx.commit()
        .await
        .context("failed to commit Postgres helpdesk cancel transaction")?;

    Ok((ticket, agent))
}

pub async fn reconcile_helpdesk_runtime_pg(
    pool: &PgPool,
    agent_stale_after_ms: i64,
    now_ms: i64,
) -> Result<HelpdeskRuntimeReconcileResult> {
    let stale_before_ms = now_ms.saturating_sub(agent_stale_after_ms);
    let mut tx = pool
        .begin()
        .await
        .context("failed to open Postgres helpdesk runtime reconcile transaction")?;
    let mut stats = HelpdeskRuntimeReconcileResult::default();

    let expired_openings = sqlx::query(
        r#"
        SELECT ticket_id, assigned_agent_id
        FROM helpdesk_tickets
        WHERE status = 'opening'
          AND opening_deadline_at IS NOT NULL
          AND opening_deadline_at <= $1
        ORDER BY opening_deadline_at ASC, ticket_id ASC
        "#,
    )
    .bind(now_ms)
    .fetch_all(&mut *tx)
    .await
    .context("failed to query expired Postgres helpdesk openings")?;

    for row in expired_openings {
        let ticket_id: String = row.get("ticket_id");
        let agent_id: Option<String> = row.get("assigned_agent_id");

        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET status = 'queued',
                assigned_agent_id = NULL,
                opening_deadline_at = NULL,
                updated_at = $2
            WHERE ticket_id = $1
              AND status = 'opening'
            "#,
        )
        .bind(&ticket_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to requeue expired Postgres opening ticket '{}'",
                ticket_id
            )
        })?;

        if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                UPDATE helpdesk_agents
                SET status = 'available',
                    current_ticket_id = NULL,
                    updated_at = $2
                WHERE agent_id = $1
                  AND status = 'opening'
                  AND current_ticket_id = $3
                "#,
            )
            .bind(&agent_id)
            .bind(now_ms)
            .bind(&ticket_id)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "failed to release expired Postgres opening agent '{}'",
                    agent_id
                )
            })?;

            sqlx::query(
                r#"
                UPDATE helpdesk_ticket_assignments
                SET status = 'expired', updated_at = $3
                WHERE ticket_id = $1
                  AND agent_id = $2
                  AND status = 'opening'
                "#,
            )
            .bind(&ticket_id)
            .bind(&agent_id)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "failed to expire Postgres assignment for ticket '{}'",
                    ticket_id
                )
            })?;

            append_helpdesk_audit_event_pg_tx(
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

        append_helpdesk_audit_event_pg_tx(
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
          AND last_heartbeat_at < $1
        ORDER BY last_heartbeat_at ASC, agent_id ASC
        "#,
    )
    .bind(stale_before_ms)
    .fetch_all(&mut *tx)
    .await
    .context("failed to query stale Postgres helpdesk agents")?;

    for row in stale_agents {
        let agent_id: String = row.get("agent_id");
        let status: String = row.get("status");
        let current_ticket_id: Option<String> = row.get("current_ticket_id");

        sqlx::query(
            r#"
            UPDATE helpdesk_agents
            SET status = 'offline',
                current_ticket_id = NULL,
                updated_at = $2
            WHERE agent_id = $1
            "#,
        )
        .bind(&agent_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to mark stale Postgres agent '{}' offline", agent_id))?;

        append_helpdesk_audit_event_pg_tx(
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
                        updated_at = $2
                    WHERE ticket_id = $1
                      AND status = 'opening'
                    "#,
                )
                .bind(&ticket_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to requeue Postgres ticket '{}' for stale opening agent",
                        ticket_id
                    )
                })?;

                sqlx::query(
                    r#"
                    UPDATE helpdesk_ticket_assignments
                    SET status = 'expired', updated_at = $3
                    WHERE ticket_id = $1
                      AND agent_id = $2
                      AND status = 'opening'
                    "#,
                )
                .bind(&ticket_id)
                .bind(&agent_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to expire stale Postgres opening assignment '{}'",
                        ticket_id
                    )
                })?;

                append_helpdesk_audit_event_pg_tx(
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
                        updated_at = $2
                    WHERE ticket_id = $1
                      AND status = 'in_progress'
                    "#,
                )
                .bind(&ticket_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to fail in-progress Postgres ticket '{}' for stale agent",
                        ticket_id
                    )
                })?;

                sqlx::query(
                    r#"
                    UPDATE helpdesk_ticket_assignments
                    SET status = 'failed', updated_at = $3
                    WHERE ticket_id = $1
                      AND agent_id = $2
                      AND status IN ('opening', 'in_progress')
                    "#,
                )
                .bind(&ticket_id)
                .bind(&agent_id)
                .bind(now_ms)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to fail stale busy Postgres assignment '{}'",
                        ticket_id
                    )
                })?;

                append_helpdesk_audit_event_pg_tx(
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
        .context("failed to commit Postgres helpdesk runtime reconcile transaction")?;

    Ok(stats)
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
    .with_context(|| {
        format!(
            "failed to append Postgres audit event '{}' for '{}'",
            event_type, entity_id
        )
    })?;
    Ok(())
}

pub async fn list_helpdesk_ticket_audit_pg(
    pool: &PgPool,
    ticket_id: &str,
) -> Result<Vec<HelpdeskAuditEventV1>> {
    let rows = sqlx::query(
        r#"
        SELECT entity_type, entity_id, event_type, payload, created_at
        FROM helpdesk_audit_events
        WHERE (entity_type = 'ticket' AND entity_id = $1)
           OR (
                entity_type = 'agent'
                AND payload IS NOT NULL
                AND payload::jsonb ->> 'ticket_id' = $1
           )
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(ticket_id.trim())
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to list Postgres helpdesk audit for '{}'", ticket_id))?;

    rows.into_iter()
        .map(row_to_helpdesk_audit_event_pg)
        .collect()
}

pub async fn get_helpdesk_agent_pg(
    pool: &PgPool,
    agent_id: &str,
) -> Result<Option<HelpdeskAgentV1>> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let row = sqlx::query(
        r#"
        SELECT agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, updated_at
        FROM helpdesk_agents
        WHERE agent_id = $1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to query Postgres helpdesk agent '{}'", agent_id))?;

    row.map(row_to_helpdesk_agent_pg).transpose()
}

pub async fn is_known_helpdesk_agent_id_pg(pool: &PgPool, user_id: &str) -> Result<bool> {
    let normalized_user_id = normalize_helpdesk_agent_id(user_id);
    if normalized_user_id.is_empty() {
        return Ok(false);
    }

    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM helpdesk_authorized_agents haa
            WHERE regexp_replace(trim(haa.agent_id), '\s+', '', 'g') = $1
            UNION
            SELECT 1
            FROM helpdesk_agents ha
            WHERE regexp_replace(trim(ha.agent_id), '\s+', '', 'g') = $1
        )
        "#,
    )
    .bind(&normalized_user_id)
    .fetch_one(pool)
    .await
    .context("failed to classify Postgres session actor against helpdesk agents")?;

    Ok(exists)
}

pub async fn helpdesk_agent_has_active_ticket_pg(pool: &PgPool, user_id: &str) -> Result<bool> {
    let normalized_user_id = normalize_helpdesk_agent_id(user_id);
    if normalized_user_id.is_empty() {
        return Ok(false);
    }

    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM helpdesk_tickets ht
            WHERE regexp_replace(trim(COALESCE(ht.assigned_agent_id, '')), '\s+', '', 'g') = $1
              AND ht.status IN ('opening', 'in_progress')
            UNION
            SELECT 1
            FROM helpdesk_agents ha
            WHERE regexp_replace(trim(ha.agent_id), '\s+', '', 'g') = $1
              AND ha.current_ticket_id IS NOT NULL
              AND ha.status IN ('opening', 'busy')
        )
        "#,
    )
    .bind(&normalized_user_id)
    .fetch_one(pool)
    .await
    .context("failed to determine whether Postgres helpdesk agent has an active ticket")?;

    Ok(exists)
}

pub async fn should_store_session_event_pg(
    sqlite_pool: &sqlx::SqlitePool,
    helpdesk_pool: &PgPool,
    event: &SessionEventV1,
    monitoring: &MonitoringConfig,
) -> Result<bool> {
    if monitoring.capture_non_agent_events {
        return crate::storage::should_store_participant_activity_for_monitoring(
            sqlite_pool,
            event,
            monitoring,
        )
        .await;
    }

    if !is_known_helpdesk_agent_id_pg(helpdesk_pool, &event.user_id).await? {
        return Ok(false);
    }

    if !helpdesk_agent_has_active_ticket_pg(helpdesk_pool, &event.user_id).await? {
        return Ok(false);
    }

    if event.event_type == SessionEventType::ParticipantActivity {
        return crate::storage::should_store_participant_activity_for_monitoring(
            sqlite_pool,
            event,
            monitoring,
        )
        .await;
    }

    Ok(true)
}

async fn get_helpdesk_authorized_agent_pg(
    pool: &PgPool,
    agent_id: &str,
) -> Result<Option<HelpdeskAuthorizedAgentV1>> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let row = sqlx::query(
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
        WHERE agent_id = $1
        LIMIT 1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query Postgres authorized helpdesk agent '{}'",
            agent_id
        )
    })?;

    row.map(row_to_helpdesk_authorized_agent_pg).transpose()
}

async fn get_helpdesk_ticket_pg_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ticket_id: &str,
) -> Result<Option<HelpdeskTicketV1>> {
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
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| {
        format!(
            "failed to query Postgres helpdesk ticket '{}' in transaction",
            ticket_id
        )
    })?;

    row.map(row_to_helpdesk_ticket_pg).transpose()
}

async fn get_helpdesk_agent_pg_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent_id: &str,
) -> Result<Option<HelpdeskAgentV1>> {
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let row = sqlx::query(
        r#"
        SELECT agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, updated_at
        FROM helpdesk_agents
        WHERE agent_id = $1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("failed to query Postgres helpdesk agent '{}' in transaction", agent_id))?;

    row.map(row_to_helpdesk_agent_pg).transpose()
}

async fn append_helpdesk_audit_event_pg_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entity_type: &str,
    entity_id: &str,
    event_type: &str,
    payload: Option<Value>,
    now_ms: i64,
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
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| {
        format!(
            "failed to append Postgres audit event '{}:{}' in transaction",
            entity_type, entity_id
        )
    })?;

    Ok(())
}

async fn assign_helpdesk_ticket_to_agent_pg_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ticket_id: &str,
    agent_id: &str,
    now_ms: i64,
    dispatch_source: &str,
    reason: Option<&str>,
) -> Result<bool> {
    let deadline_ms = now_ms + HELPDESK_OPENING_WINDOW_MS;
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());

    let ticket_update = sqlx::query(
        r#"
        UPDATE helpdesk_tickets
        SET status = 'opening',
            assigned_agent_id = $2,
            opening_deadline_at = $3,
            updated_at = $4
        WHERE ticket_id = $1
          AND status = 'queued'
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(deadline_ms)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to move Postgres ticket '{}' to opening", ticket_id))?;

    if ticket_update.rows_affected() == 0 {
        return Ok(false);
    }

    let agent_update = sqlx::query(
        r#"
        UPDATE helpdesk_agents
        SET status = 'opening',
            current_ticket_id = $2,
            updated_at = $3
        WHERE agent_id = $1
          AND status = 'available'
        "#,
    )
    .bind(agent_id)
    .bind(ticket_id)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to move Postgres agent '{}' to opening", agent_id))?;

    if agent_update.rows_affected() == 0 {
        sqlx::query(
            r#"
            UPDATE helpdesk_tickets
            SET status = 'queued',
                assigned_agent_id = NULL,
                opening_deadline_at = NULL,
                updated_at = $2
            WHERE ticket_id = $1
            "#,
        )
        .bind(ticket_id)
        .bind(now_ms)
        .execute(&mut **tx)
        .await
        .with_context(|| {
            format!(
                "failed to rollback Postgres ticket '{}' opening state",
                ticket_id
            )
        })?;
        return Ok(false);
    }

    sqlx::query(
        r#"
        INSERT INTO helpdesk_ticket_assignments (ticket_id, agent_id, status, created_at, updated_at)
        VALUES ($1, $2, 'opening', $3, $4)
        "#,
    )
    .bind(ticket_id)
    .bind(agent_id)
    .bind(now_ms)
    .bind(now_ms)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to insert Postgres assignment for ticket '{}'", ticket_id))?;

    append_helpdesk_audit_event_pg_tx(
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

    append_helpdesk_audit_event_pg_tx(
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

fn row_to_helpdesk_authorized_agent_pg(row: PgRow) -> Result<HelpdeskAuthorizedAgentV1> {
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
    raw.trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
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
    Utc.timestamp_millis_opt(value).single().unwrap_or_else(|| {
        Utc.timestamp_opt(0, 0)
            .single()
            .expect("unix epoch should exist")
    })
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
