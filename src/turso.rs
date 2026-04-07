use anyhow::Context;
use chrono::TimeZone;
use libsql::{Builder, Connection, Database, Rows, Value};
use sqlx::{Row, SqlitePool};

use crate::auth;
use crate::model::{
    AuthRoleV1, HelpdeskOperationalSummaryV1, HelpdeskTicketCreateRequestV1,
    HelpdeskTicketStatus, HelpdeskTicketV1,
};

use crate::schema::{init_libsql_schema, init_sqlite_schema};

#[derive(Debug, Clone)]
pub struct TursoBootstrapSummary {
    pub url: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TursoHelpdeskSmokeSummary {
    pub url: String,
    pub authorized_agent_id: String,
    pub created_ticket: HelpdeskTicketV1,
    pub tickets_total: usize,
    pub operational_summary: HelpdeskOperationalSummaryV1,
}

#[derive(Debug, Clone)]
pub struct TursoSyncConfig {
    pub url: String,
    pub auth_token: String,
    pub interval_ms: u64,
    pub monitoring_outbox_retention_ms: i64,
    pub monitoring_session_event_retention_ms: i64,
    pub monitoring_presence_retention_ms: i64,
    pub helpdesk_heartbeat_retention_ms: i64,
    pub helpdesk_audit_retention_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct HelpdeskSnapshotCounts {
    pub authorized_agents: usize,
    pub agents: usize,
    pub tickets: usize,
    pub assignments: usize,
    pub heartbeats: usize,
    pub audit_events: usize,
}

impl HelpdeskSnapshotCounts {
    pub fn total_rows(&self) -> usize {
        self.authorized_agents
            + self.agents
            + self.tickets
            + self.assignments
            + self.heartbeats
            + self.audit_events
    }
}

#[derive(Debug, Clone)]
pub struct HelpdeskTursoBridgeSummary {
    pub mode: &'static str,
    pub local_counts: HelpdeskSnapshotCounts,
    pub remote_counts: HelpdeskSnapshotCounts,
}

#[derive(Debug, Clone, Default)]
pub struct MonitoringSnapshotCounts {
    pub outbox_events: usize,
    pub session_events: usize,
    pub session_presence: usize,
}

impl MonitoringSnapshotCounts {
    pub fn total_rows(&self) -> usize {
        self.outbox_events + self.session_events + self.session_presence
    }
}

#[derive(Debug, Clone)]
pub struct MonitoringTursoBridgeSummary {
    pub mode: &'static str,
    pub local_counts: MonitoringSnapshotCounts,
    pub remote_counts: MonitoringSnapshotCounts,
}

pub async fn connect_turso_remote(url: &str, auth_token: &str) -> anyhow::Result<Database> {
    Builder::new_remote(url.to_string(), auth_token.to_string())
        .build()
        .await
        .with_context(|| format!("failed to connect to Turso database at {url}"))
}

impl TursoSyncConfig {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("TURSO_DATABASE_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let auth_token = std::env::var("TURSO_AUTH_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        match (url, auth_token) {
            (Some(url), Some(auth_token)) => Some(Self {
                url,
                auth_token,
                interval_ms: env_u64("TURSO_SYNC_INTERVAL_MS").unwrap_or(60_000),
                monitoring_outbox_retention_ms: env_i64("TURSO_OUTBOX_RETENTION_MS")
                    .unwrap_or(3 * 24 * 60 * 60 * 1000),
                monitoring_session_event_retention_ms: env_i64(
                    "TURSO_SESSION_EVENT_RETENTION_MS",
                )
                .unwrap_or(14 * 24 * 60 * 60 * 1000),
                monitoring_presence_retention_ms: env_i64("TURSO_SESSION_PRESENCE_RETENTION_MS")
                    .unwrap_or(24 * 60 * 60 * 1000),
                helpdesk_heartbeat_retention_ms: env_i64("TURSO_HEARTBEAT_RETENTION_MS")
                    .unwrap_or(7 * 24 * 60 * 60 * 1000),
                helpdesk_audit_retention_ms: env_i64("TURSO_AUDIT_RETENTION_MS")
                    .unwrap_or(90 * 24 * 60 * 60 * 1000),
            }),
            _ => None,
        }
    }
}

pub async fn initialize_helpdesk_turso_bridge(
    pool: &SqlitePool,
    sync_cfg: &TursoSyncConfig,
) -> anyhow::Result<HelpdeskTursoBridgeSummary> {
    init_sqlite_schema(pool).await?;

    let db = connect_turso_remote(&sync_cfg.url, &sync_cfg.auth_token).await?;
    let conn = db
        .connect()
        .context("failed to open Turso connection for helpdesk bridge bootstrap")?;
    init_libsql_schema(&conn).await?;

    let local_counts = count_helpdesk_rows_sqlite(pool).await?;
    let remote_snapshot = fetch_helpdesk_snapshot_from_turso(&conn).await?;
    let remote_counts = remote_snapshot.counts();

    if remote_counts.total_rows() > 0 {
        apply_helpdesk_snapshot_to_sqlite(pool, &remote_snapshot).await?;
        let local_counts = count_helpdesk_rows_sqlite(pool).await?;
        return Ok(HelpdeskTursoBridgeSummary {
            mode: "restored_from_turso",
            local_counts,
            remote_counts,
        });
    }

    if local_counts.total_rows() > 0 {
        let local_snapshot = fetch_helpdesk_snapshot_from_sqlite(pool, Some(sync_cfg)).await?;
        apply_helpdesk_snapshot_to_turso(&conn, &local_snapshot).await?;
        let remote_counts = count_helpdesk_rows_turso(&conn).await?;
        return Ok(HelpdeskTursoBridgeSummary {
            mode: "seeded_turso_from_sqlite",
            local_counts,
            remote_counts,
        });
    }

    Ok(HelpdeskTursoBridgeSummary {
        mode: "empty",
        local_counts,
        remote_counts,
    })
}

pub async fn sync_helpdesk_snapshot_to_turso(
    pool: &SqlitePool,
    sync_cfg: &TursoSyncConfig,
) -> anyhow::Result<HelpdeskSnapshotCounts> {
    init_sqlite_schema(pool).await?;

    let snapshot = fetch_helpdesk_snapshot_from_sqlite(pool, Some(sync_cfg)).await?;
    let db = connect_turso_remote(&sync_cfg.url, &sync_cfg.auth_token).await?;
    let conn = db
        .connect()
        .context("failed to open Turso connection for helpdesk snapshot sync")?;
    init_libsql_schema(&conn).await?;
    apply_helpdesk_snapshot_to_turso(&conn, &snapshot).await?;
    Ok(snapshot.counts())
}

pub async fn initialize_monitoring_turso_bridge(
    pool: &SqlitePool,
    sync_cfg: &TursoSyncConfig,
) -> anyhow::Result<MonitoringTursoBridgeSummary> {
    init_sqlite_schema(pool).await?;

    let db = connect_turso_remote(&sync_cfg.url, &sync_cfg.auth_token).await?;
    let conn = db
        .connect()
        .context("failed to open Turso connection for monitoring bridge bootstrap")?;
    init_libsql_schema(&conn).await?;

    let local_counts = count_monitoring_rows_sqlite(pool).await?;
    let remote_snapshot = fetch_monitoring_snapshot_from_turso(&conn).await?;
    let remote_counts = remote_snapshot.counts();

    if remote_counts.total_rows() > 0 {
        apply_monitoring_snapshot_to_sqlite(pool, &remote_snapshot).await?;
        let local_counts = count_monitoring_rows_sqlite(pool).await?;
        return Ok(MonitoringTursoBridgeSummary {
            mode: "restored_from_turso",
            local_counts,
            remote_counts,
        });
    }

    if local_counts.total_rows() > 0 {
        let local_snapshot = fetch_monitoring_snapshot_from_sqlite(pool, Some(sync_cfg)).await?;
        apply_monitoring_snapshot_to_turso(&conn, &local_snapshot).await?;
        let remote_counts = count_monitoring_rows_turso(&conn).await?;
        return Ok(MonitoringTursoBridgeSummary {
            mode: "seeded_turso_from_sqlite",
            local_counts,
            remote_counts,
        });
    }

    Ok(MonitoringTursoBridgeSummary {
        mode: "empty",
        local_counts,
        remote_counts,
    })
}

pub async fn sync_monitoring_snapshot_to_turso(
    pool: &SqlitePool,
    sync_cfg: &TursoSyncConfig,
) -> anyhow::Result<MonitoringSnapshotCounts> {
    init_sqlite_schema(pool).await?;

    let snapshot = fetch_monitoring_snapshot_from_sqlite(pool, Some(sync_cfg)).await?;
    let db = connect_turso_remote(&sync_cfg.url, &sync_cfg.auth_token).await?;
    let conn = db
        .connect()
        .context("failed to open Turso connection for monitoring snapshot sync")?;
    init_libsql_schema(&conn).await?;
    apply_monitoring_snapshot_to_turso(&conn, &snapshot).await?;
    Ok(snapshot.counts())
}

pub async fn bootstrap_turso_remote(
    url: &str,
    auth_token: &str,
) -> anyhow::Result<TursoBootstrapSummary> {
    eprintln!("turso-bootstrap: connecting to remote database");
    let db = connect_turso_remote(url, auth_token).await?;
    eprintln!("turso-bootstrap: opening connection");
    let conn = db
        .connect()
        .context("failed to open Turso connection after build")?;

    eprintln!("turso-bootstrap: applying schema");
    init_libsql_schema(&conn).await?;
    eprintln!("turso-bootstrap: listing tables");

    let tables = list_tables(&conn).await?;
    Ok(TursoBootstrapSummary {
        url: url.to_string(),
        tables,
    })
}

async fn list_tables(conn: &libsql::Connection) -> anyhow::Result<Vec<String>> {
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
            (),
        )
        .await
        .context("failed to query tables from Turso")?;

    let mut tables = Vec::new();
    while let Some(row) = rows.next().await? {
        let table_name: String = row.get(0)?;
        tables.push(table_name);
    }

    Ok(tables)
}

pub async fn run_helpdesk_smoke(
    url: &str,
    auth_token: &str,
    supervisor_username: &str,
    supervisor_password: &str,
    authorized_agent_id: &str,
    authorized_agent_name: Option<&str>,
    ticket: &HelpdeskTicketCreateRequestV1,
) -> anyhow::Result<TursoHelpdeskSmokeSummary> {
    let db = connect_turso_remote(url, auth_token).await?;
    let conn = db
        .connect()
        .context("failed to open Turso connection for helpdesk smoke")?;

    init_libsql_schema(&conn).await?;
    seed_supervisor_user(&conn, supervisor_username, supervisor_password).await?;
    upsert_authorized_agent(&conn, authorized_agent_id, authorized_agent_name).await?;
    let created_ticket = create_helpdesk_ticket_remote(&conn, ticket).await?;
    let tickets = list_helpdesk_tickets_remote(&conn).await?;
    let operational_summary = get_helpdesk_operational_summary_remote(&conn).await?;

    Ok(TursoHelpdeskSmokeSummary {
        url: url.to_string(),
        authorized_agent_id: normalize_helpdesk_agent_id(authorized_agent_id),
        created_ticket,
        tickets_total: tickets.len(),
        operational_summary,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HelpdeskSyncSignature {
    pub authorized_agents: usize,
    pub agents: usize,
    pub tickets: usize,
    pub assignments: usize,
    pub heartbeats: usize,
    pub audit_events: usize,
    pub max_authorized_updated_at: i64,
    pub max_agent_updated_at: i64,
    pub max_ticket_updated_at: i64,
    pub max_assignment_updated_at: i64,
    pub max_heartbeat_created_at: i64,
    pub max_audit_created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MonitoringSyncSignature {
    pub outbox_events: usize,
    pub session_events: usize,
    pub session_presence: usize,
    pub max_outbox_updated_at: i64,
    pub max_session_event_created_at: i64,
    pub max_session_presence_updated_at: i64,
}

pub async fn compute_helpdesk_sync_signature(
    pool: &SqlitePool,
    sync_cfg: &TursoSyncConfig,
) -> anyhow::Result<HelpdeskSyncSignature> {
    let snapshot = fetch_helpdesk_snapshot_from_sqlite(pool, Some(sync_cfg)).await?;
    Ok(HelpdeskSyncSignature {
        authorized_agents: snapshot.authorized_agents.len(),
        agents: snapshot.agents.len(),
        tickets: snapshot.tickets.len(),
        assignments: snapshot.assignments.len(),
        heartbeats: snapshot.heartbeats.len(),
        audit_events: snapshot.audit_events.len(),
        max_authorized_updated_at: snapshot
            .authorized_agents
            .iter()
            .map(|row| row.updated_at)
            .max()
            .unwrap_or(0),
        max_agent_updated_at: snapshot.agents.iter().map(|row| row.updated_at).max().unwrap_or(0),
        max_ticket_updated_at: snapshot.tickets.iter().map(|row| row.updated_at).max().unwrap_or(0),
        max_assignment_updated_at: snapshot
            .assignments
            .iter()
            .map(|row| row.updated_at)
            .max()
            .unwrap_or(0),
        max_heartbeat_created_at: snapshot.heartbeats.iter().map(|row| row.created_at).max().unwrap_or(0),
        max_audit_created_at: snapshot.audit_events.iter().map(|row| row.created_at).max().unwrap_or(0),
    })
}

pub async fn compute_monitoring_sync_signature(
    pool: &SqlitePool,
    sync_cfg: &TursoSyncConfig,
) -> anyhow::Result<MonitoringSyncSignature> {
    let snapshot = fetch_monitoring_snapshot_from_sqlite(pool, Some(sync_cfg)).await?;
    Ok(MonitoringSyncSignature {
        outbox_events: snapshot.outbox_events.len(),
        session_events: snapshot.session_events.len(),
        session_presence: snapshot.session_presence.len(),
        max_outbox_updated_at: snapshot.outbox_events.iter().map(|row| row.updated_at).max().unwrap_or(0),
        max_session_event_created_at: snapshot
            .session_events
            .iter()
            .map(|row| row.created_at)
            .max()
            .unwrap_or(0),
        max_session_presence_updated_at: snapshot
            .session_presence
            .iter()
            .map(|row| row.updated_at)
            .max()
            .unwrap_or(0),
    })
}

async fn seed_supervisor_user(
    conn: &Connection,
    supervisor_username: &str,
    supervisor_password: &str,
) -> anyhow::Result<()> {
    let now_ms = unix_millis_now();
    let password_hash = auth::hash_password(supervisor_password)
        .context("failed to hash supervisor password for Turso smoke")?;

    conn.execute(
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
        (
            supervisor_username.trim(),
            password_hash,
            AuthRoleV1::Supervisor.as_str(),
            now_ms,
            now_ms,
        ),
    )
    .await
    .context("failed to seed supervisor user in Turso")?;

    Ok(())
}

async fn upsert_authorized_agent(
    conn: &Connection,
    agent_id: &str,
    display_name: Option<&str>,
) -> anyhow::Result<()> {
    let now_ms = unix_millis_now();
    let agent_id = normalize_helpdesk_agent_id(agent_id);
    let display_name = normalize_optional_text(display_name);

    conn.execute(
        r#"
        INSERT INTO helpdesk_authorized_agents (agent_id, display_name, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(agent_id) DO UPDATE SET
            display_name = COALESCE(?2, helpdesk_authorized_agents.display_name),
            updated_at = ?4
        "#,
        (agent_id, display_name, now_ms, now_ms),
    )
    .await
    .context("failed to upsert authorized helpdesk agent in Turso")?;

    Ok(())
}

async fn create_helpdesk_ticket_remote(
    conn: &Connection,
    payload: &HelpdeskTicketCreateRequestV1,
) -> anyhow::Result<HelpdeskTicketV1> {
    let now_ms = unix_millis_now();
    let ticket_id = uuid::Uuid::new_v4().to_string();
    let client_id = payload.client_id.trim().to_string();
    let normalized_title = normalize_optional_text(payload.title.as_deref());
    let normalized_summary =
        normalize_optional_text(payload.summary.as_deref()).or_else(|| normalized_title.clone());
    let normalized_description = normalize_optional_text(payload.description.as_deref());
    let normalized_difficulty = normalize_optional_text(payload.difficulty.as_deref());
    let insert_params = vec![
        Value::from(ticket_id.clone()),
        Value::from(client_id.clone()),
        optional_text_value(payload.client_display_name.as_deref()),
        optional_text_value(payload.device_id.as_deref()),
        optional_text_value(payload.requested_by.as_deref()),
        optional_string_value(normalized_title.clone()),
        optional_string_value(normalized_description.clone()),
        optional_string_value(normalized_difficulty.clone()),
        optional_i64_value(payload.estimated_minutes.map(i64::from)),
        optional_string_value(normalized_summary.clone()),
        Value::from(now_ms),
        Value::from(now_ms),
    ];

    let tx = conn
        .transaction()
        .await
        .context("failed to open Turso helpdesk ticket transaction")?;

    tx.execute(
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
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued', NULL, NULL, NULL, NULL, NULL, ?, ?)
        "#,
        insert_params,
    )
    .await
    .context("failed to insert helpdesk ticket in Turso")?;

    tx.execute(
        r#"
        INSERT INTO helpdesk_audit_events (entity_type, entity_id, event_type, payload, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        (
            "ticket",
            ticket_id.clone(),
            "help_request_created",
            serde_json::json!({
                "client_id": payload.client_id.trim(),
                "device_id": payload.device_id,
                "title": normalized_title,
                "description": normalized_description,
                "difficulty": normalized_difficulty,
                "estimated_minutes": payload.estimated_minutes,
                "summary": normalized_summary,
            })
            .to_string(),
            now_ms,
        ),
    )
    .await
    .context("failed to insert helpdesk audit event in Turso")?;

    tx.commit()
        .await
        .context("failed to commit Turso helpdesk ticket transaction")?;

    get_helpdesk_ticket_remote(conn, &ticket_id)
        .await?
        .with_context(|| format!("ticket '{ticket_id}' was not found after Turso insert"))
}

pub async fn list_helpdesk_tickets_remote(
    conn: &Connection,
) -> anyhow::Result<Vec<HelpdeskTicketV1>> {
    let mut rows = conn
        .query(
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
            (),
        )
        .await
        .context("failed to list helpdesk tickets from Turso")?;

    collect_helpdesk_tickets(&mut rows).await
}

pub async fn get_helpdesk_ticket_remote(
    conn: &Connection,
    ticket_id: &str,
) -> anyhow::Result<Option<HelpdeskTicketV1>> {
    let mut rows = conn
        .query(
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
            [ticket_id],
        )
        .await
        .with_context(|| format!("failed to query helpdesk ticket '{ticket_id}' from Turso"))?;

    match rows.next().await? {
        Some(row) => row_to_helpdesk_ticket_libsql(&row).map(Some),
        None => Ok(None),
    }
}

pub async fn get_helpdesk_operational_summary_remote(
    conn: &Connection,
) -> anyhow::Result<HelpdeskOperationalSummaryV1> {
    let mut ticket_rows = conn
        .query(
            r#"
            SELECT status, COUNT(*) AS total
            FROM helpdesk_tickets
            GROUP BY status
            "#,
            (),
        )
        .await
        .context("failed to aggregate helpdesk ticket summary from Turso")?;

    let mut agent_rows = conn
        .query(
            r#"
            SELECT status, COUNT(*) AS total
            FROM helpdesk_agents
            GROUP BY status
            "#,
            (),
        )
        .await
        .context("failed to aggregate helpdesk agent summary from Turso")?;

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

    while let Some(row) = ticket_rows.next().await? {
        let status: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        let total = i64_to_u64(total);
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

    while let Some(row) = agent_rows.next().await? {
        let status: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        let total = i64_to_u64(total);
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

async fn collect_helpdesk_tickets(rows: &mut Rows) -> anyhow::Result<Vec<HelpdeskTicketV1>> {
    let mut tickets = Vec::new();
    while let Some(row) = rows.next().await? {
        tickets.push(row_to_helpdesk_ticket_libsql(&row)?);
    }
    Ok(tickets)
}

fn row_to_helpdesk_ticket_libsql(row: &libsql::Row) -> anyhow::Result<HelpdeskTicketV1> {
    let status: String = row.get(10)?;
    let latest_agent_report_at: Option<i64> = row.get(14)?;
    let opening_deadline_at: Option<i64> = row.get(15)?;

    Ok(HelpdeskTicketV1 {
        ticket_id: row.get(0)?,
        client_id: row.get(1)?,
        client_display_name: row.get(2)?,
        device_id: row.get(3)?,
        requested_by: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        difficulty: row.get(7)?,
        estimated_minutes: row
            .get::<Option<i64>>(8)?
            .and_then(|value| u32::try_from(value).ok()),
        summary: row.get(9)?,
        status: helpdesk_ticket_status_from_db(&status),
        assigned_agent_id: row.get(11)?,
        latest_agent_report: row.get(12)?,
        latest_agent_report_by: row.get(13)?,
        latest_agent_report_at: latest_agent_report_at.map(millis_to_utc),
        opening_deadline_at: opening_deadline_at.map(millis_to_utc),
        created_at: millis_to_utc(row.get(16)?),
        updated_at: millis_to_utc(row.get(17)?),
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
    chrono::Utc::now().timestamp_millis()
}

fn millis_to_utc(value: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc
        .timestamp_millis_opt(value)
        .single()
        .unwrap_or_else(|| {
            chrono::Utc
                .timestamp_opt(0, 0)
                .single()
                .expect("unix epoch should exist")
        })
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}

fn optional_text_value(value: Option<&str>) -> Value {
    match normalize_optional_text(value) {
        Some(value) => Value::from(value),
        None => Value::Null,
    }
}

fn optional_string_value(value: Option<String>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::Null,
    }
}

fn optional_i64_value(value: Option<i64>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::Null,
    }
}

#[derive(Debug, Clone, Default)]
struct HelpdeskSnapshot {
    authorized_agents: Vec<AuthorizedAgentRecord>,
    agents: Vec<AgentRecord>,
    tickets: Vec<TicketRecord>,
    assignments: Vec<AssignmentRecord>,
    heartbeats: Vec<HeartbeatRecord>,
    audit_events: Vec<AuditEventRecord>,
}

impl HelpdeskSnapshot {
    fn counts(&self) -> HelpdeskSnapshotCounts {
        HelpdeskSnapshotCounts {
            authorized_agents: self.authorized_agents.len(),
            agents: self.agents.len(),
            tickets: self.tickets.len(),
            assignments: self.assignments.len(),
            heartbeats: self.heartbeats.len(),
            audit_events: self.audit_events.len(),
        }
    }
}

#[derive(Debug, Clone)]
struct AuthorizedAgentRecord {
    agent_id: String,
    display_name: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct AgentRecord {
    agent_id: String,
    display_name: String,
    avatar_url: Option<String>,
    status: String,
    current_ticket_id: Option<String>,
    last_heartbeat_at: i64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct TicketRecord {
    ticket_id: String,
    client_id: String,
    client_display_name: Option<String>,
    device_id: Option<String>,
    requested_by: Option<String>,
    title: Option<String>,
    description: Option<String>,
    difficulty: Option<String>,
    estimated_minutes: Option<i64>,
    summary: Option<String>,
    status: String,
    assigned_agent_id: Option<String>,
    latest_agent_report: Option<String>,
    latest_agent_report_by: Option<String>,
    latest_agent_report_at: Option<i64>,
    opening_deadline_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct AssignmentRecord {
    id: i64,
    ticket_id: String,
    agent_id: String,
    status: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct HeartbeatRecord {
    id: i64,
    agent_id: String,
    status: String,
    created_at: i64,
}

#[derive(Debug, Clone)]
struct AuditEventRecord {
    id: i64,
    entity_type: String,
    entity_id: String,
    event_type: String,
    payload: Option<String>,
    created_at: i64,
}

async fn count_helpdesk_rows_sqlite(pool: &SqlitePool) -> anyhow::Result<HelpdeskSnapshotCounts> {
    Ok(HelpdeskSnapshotCounts {
        authorized_agents: count_sqlite_table(pool, "helpdesk_authorized_agents").await?,
        agents: count_sqlite_table(pool, "helpdesk_agents").await?,
        tickets: count_sqlite_table(pool, "helpdesk_tickets").await?,
        assignments: count_sqlite_table(pool, "helpdesk_ticket_assignments").await?,
        heartbeats: count_sqlite_table(pool, "helpdesk_agent_heartbeats").await?,
        audit_events: count_sqlite_table(pool, "helpdesk_audit_events").await?,
    })
}

async fn count_helpdesk_rows_turso(conn: &Connection) -> anyhow::Result<HelpdeskSnapshotCounts> {
    Ok(HelpdeskSnapshotCounts {
        authorized_agents: count_turso_table(conn, "helpdesk_authorized_agents").await?,
        agents: count_turso_table(conn, "helpdesk_agents").await?,
        tickets: count_turso_table(conn, "helpdesk_tickets").await?,
        assignments: count_turso_table(conn, "helpdesk_ticket_assignments").await?,
        heartbeats: count_turso_table(conn, "helpdesk_agent_heartbeats").await?,
        audit_events: count_turso_table(conn, "helpdesk_audit_events").await?,
    })
}

async fn count_sqlite_table(pool: &SqlitePool, table: &str) -> anyhow::Result<usize> {
    let count = sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to count SQLite table '{table}'"))?;
    Ok(usize::try_from(count.max(0)).unwrap_or(usize::MAX))
}

async fn count_turso_table(conn: &Connection, table: &str) -> anyhow::Result<usize> {
    let mut rows = conn
        .query(&format!("SELECT COUNT(*) FROM {table}"), ())
        .await
        .with_context(|| format!("failed to count Turso table '{table}'"))?;
    match rows.next().await? {
        Some(row) => {
            let count: i64 = row.get(0)?;
            Ok(usize::try_from(count.max(0)).unwrap_or(usize::MAX))
        }
        None => Ok(0),
    }
}

async fn fetch_helpdesk_snapshot_from_sqlite(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<HelpdeskSnapshot> {
    Ok(HelpdeskSnapshot {
        authorized_agents: fetch_sqlite_authorized_agents(pool).await?,
        agents: fetch_sqlite_agents(pool).await?,
        tickets: fetch_sqlite_tickets(pool).await?,
        assignments: fetch_sqlite_assignments(pool).await?,
        heartbeats: fetch_sqlite_heartbeats(pool, sync_cfg).await?,
        audit_events: fetch_sqlite_audit_events(pool, sync_cfg).await?,
    })
}

async fn fetch_helpdesk_snapshot_from_turso(conn: &Connection) -> anyhow::Result<HelpdeskSnapshot> {
    Ok(HelpdeskSnapshot {
        authorized_agents: fetch_turso_authorized_agents(conn).await?,
        agents: fetch_turso_agents(conn).await?,
        tickets: fetch_turso_tickets(conn).await?,
        assignments: fetch_turso_assignments(conn).await?,
        heartbeats: fetch_turso_heartbeats(conn).await?,
        audit_events: fetch_turso_audit_events(conn).await?,
    })
}

async fn apply_helpdesk_snapshot_to_sqlite(
    pool: &SqlitePool,
    snapshot: &HelpdeskSnapshot,
) -> anyhow::Result<()> {
    init_sqlite_schema(pool).await?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to open SQLite transaction for Turso restore")?;

    for table in [
        "helpdesk_ticket_assignments",
        "helpdesk_agent_heartbeats",
        "helpdesk_audit_events",
        "helpdesk_tickets",
        "helpdesk_agents",
        "helpdesk_authorized_agents",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to clear SQLite table '{table}' during Turso restore"))?;
    }

    for row in &snapshot.authorized_agents {
        sqlx::query(
            r#"
            INSERT INTO helpdesk_authorized_agents (agent_id, display_name, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind(&row.agent_id)
        .bind(&row.display_name)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore authorized agent '{}' into SQLite", row.agent_id))?;
    }

    for row in &snapshot.agents {
        sqlx::query(
            r#"
            INSERT INTO helpdesk_agents (
                agent_id, display_name, avatar_url, status, current_ticket_id,
                last_heartbeat_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(&row.agent_id)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(&row.status)
        .bind(&row.current_ticket_id)
        .bind(row.last_heartbeat_at)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore helpdesk agent '{}' into SQLite", row.agent_id))?;
    }

    for row in &snapshot.tickets {
        sqlx::query(
            r#"
            INSERT INTO helpdesk_tickets (
                ticket_id, client_id, client_display_name, device_id, requested_by,
                title, description, difficulty, estimated_minutes, summary, status,
                assigned_agent_id, latest_agent_report, latest_agent_report_by,
                latest_agent_report_at, opening_deadline_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
        )
        .bind(&row.ticket_id)
        .bind(&row.client_id)
        .bind(&row.client_display_name)
        .bind(&row.device_id)
        .bind(&row.requested_by)
        .bind(&row.title)
        .bind(&row.description)
        .bind(&row.difficulty)
        .bind(row.estimated_minutes)
        .bind(&row.summary)
        .bind(&row.status)
        .bind(&row.assigned_agent_id)
        .bind(&row.latest_agent_report)
        .bind(&row.latest_agent_report_by)
        .bind(row.latest_agent_report_at)
        .bind(row.opening_deadline_at)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore helpdesk ticket '{}' into SQLite", row.ticket_id))?;
    }

    for row in &snapshot.assignments {
        sqlx::query(
            r#"
            INSERT INTO helpdesk_ticket_assignments (id, ticket_id, agent_id, status, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(row.id)
        .bind(&row.ticket_id)
        .bind(&row.agent_id)
        .bind(&row.status)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore assignment '{}' into SQLite", row.id))?;
    }

    for row in &snapshot.heartbeats {
        sqlx::query(
            r#"
            INSERT INTO helpdesk_agent_heartbeats (id, agent_id, status, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind(row.id)
        .bind(&row.agent_id)
        .bind(&row.status)
        .bind(row.created_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore heartbeat '{}' into SQLite", row.id))?;
    }

    for row in &snapshot.audit_events {
        sqlx::query(
            r#"
            INSERT INTO helpdesk_audit_events (id, entity_type, entity_id, event_type, payload, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(row.id)
        .bind(&row.entity_type)
        .bind(&row.entity_id)
        .bind(&row.event_type)
        .bind(&row.payload)
        .bind(row.created_at)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore audit event '{}' into SQLite", row.id))?;
    }

    tx.commit()
        .await
        .context("failed to commit SQLite Turso restore transaction")?;

    Ok(())
}

async fn apply_helpdesk_snapshot_to_turso(
    conn: &Connection,
    snapshot: &HelpdeskSnapshot,
) -> anyhow::Result<()> {
    init_libsql_schema(conn).await?;
    let tx = conn
        .transaction()
        .await
        .context("failed to open Turso transaction for helpdesk sync")?;

    for table in [
        "helpdesk_ticket_assignments",
        "helpdesk_agent_heartbeats",
        "helpdesk_audit_events",
        "helpdesk_tickets",
        "helpdesk_agents",
        "helpdesk_authorized_agents",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), ())
            .await
            .with_context(|| format!("failed to clear Turso table '{table}' during helpdesk sync"))?;
    }

    for row in &snapshot.authorized_agents {
        tx.execute(
            r#"
            INSERT INTO helpdesk_authorized_agents (agent_id, display_name, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            (
                row.agent_id.as_str(),
                row.display_name.as_deref(),
                row.created_at,
                row.updated_at,
            ),
        )
        .await
        .with_context(|| format!("failed to sync authorized agent '{}' to Turso", row.agent_id))?;
    }

    for row in &snapshot.agents {
        tx.execute(
            r#"
            INSERT INTO helpdesk_agents (
                agent_id, display_name, avatar_url, status, current_ticket_id,
                last_heartbeat_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            (
                row.agent_id.as_str(),
                row.display_name.as_str(),
                row.avatar_url.as_deref(),
                row.status.as_str(),
                row.current_ticket_id.as_deref(),
                row.last_heartbeat_at,
                row.created_at,
                row.updated_at,
            ),
        )
        .await
        .with_context(|| format!("failed to sync helpdesk agent '{}' to Turso", row.agent_id))?;
    }

    for row in &snapshot.tickets {
        let ticket_params = vec![
            Value::from(row.ticket_id.clone()),
            Value::from(row.client_id.clone()),
            optional_string_value(row.client_display_name.clone()),
            optional_string_value(row.device_id.clone()),
            optional_string_value(row.requested_by.clone()),
            optional_string_value(row.title.clone()),
            optional_string_value(row.description.clone()),
            optional_string_value(row.difficulty.clone()),
            optional_i64_value(row.estimated_minutes),
            optional_string_value(row.summary.clone()),
            Value::from(row.status.clone()),
            optional_string_value(row.assigned_agent_id.clone()),
            optional_string_value(row.latest_agent_report.clone()),
            optional_string_value(row.latest_agent_report_by.clone()),
            optional_i64_value(row.latest_agent_report_at),
            optional_i64_value(row.opening_deadline_at),
            Value::from(row.created_at),
            Value::from(row.updated_at),
        ];
        tx.execute(
            r#"
            INSERT INTO helpdesk_tickets (
                ticket_id, client_id, client_display_name, device_id, requested_by,
                title, description, difficulty, estimated_minutes, summary, status,
                assigned_agent_id, latest_agent_report, latest_agent_report_by,
                latest_agent_report_at, opening_deadline_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
            ticket_params,
        )
        .await
        .with_context(|| format!("failed to sync helpdesk ticket '{}' to Turso", row.ticket_id))?;
    }

    for row in &snapshot.assignments {
        tx.execute(
            r#"
            INSERT INTO helpdesk_ticket_assignments (id, ticket_id, agent_id, status, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            (
                row.id,
                row.ticket_id.as_str(),
                row.agent_id.as_str(),
                row.status.as_str(),
                row.created_at,
                row.updated_at,
            ),
        )
        .await
        .with_context(|| format!("failed to sync assignment '{}' to Turso", row.id))?;
    }

    for row in &snapshot.heartbeats {
        tx.execute(
            r#"
            INSERT INTO helpdesk_agent_heartbeats (id, agent_id, status, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            (
                row.id,
                row.agent_id.as_str(),
                row.status.as_str(),
                row.created_at,
            ),
        )
        .await
        .with_context(|| format!("failed to sync heartbeat '{}' to Turso", row.id))?;
    }

    for row in &snapshot.audit_events {
        tx.execute(
            r#"
            INSERT INTO helpdesk_audit_events (id, entity_type, entity_id, event_type, payload, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            (
                row.id,
                row.entity_type.as_str(),
                row.entity_id.as_str(),
                row.event_type.as_str(),
                row.payload.as_deref(),
                row.created_at,
            ),
        )
        .await
        .with_context(|| format!("failed to sync audit event '{}' to Turso", row.id))?;
    }

    tx.commit()
        .await
        .context("failed to commit Turso helpdesk sync transaction")?;

    Ok(())
}

async fn fetch_sqlite_authorized_agents(pool: &SqlitePool) -> anyhow::Result<Vec<AuthorizedAgentRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT agent_id, display_name, created_at, updated_at
        FROM helpdesk_authorized_agents
        ORDER BY created_at ASC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to read SQLite authorized helpdesk agents")?;

    Ok(rows
        .into_iter()
        .map(|row| AuthorizedAgentRecord {
            agent_id: row.get("agent_id"),
            display_name: row.get("display_name"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

async fn fetch_sqlite_agents(pool: &SqlitePool) -> anyhow::Result<Vec<AgentRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, created_at, updated_at
        FROM helpdesk_agents
        ORDER BY created_at ASC, agent_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to read SQLite helpdesk agents")?;

    Ok(rows
        .into_iter()
        .map(|row| AgentRecord {
            agent_id: row.get("agent_id"),
            display_name: row.get("display_name"),
            avatar_url: row.get("avatar_url"),
            status: row.get("status"),
            current_ticket_id: row.get("current_ticket_id"),
            last_heartbeat_at: row.get("last_heartbeat_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

async fn fetch_sqlite_tickets(pool: &SqlitePool) -> anyhow::Result<Vec<TicketRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT ticket_id, client_id, client_display_name, device_id, requested_by, title,
               description, difficulty, estimated_minutes, summary, status, assigned_agent_id,
               latest_agent_report, latest_agent_report_by, latest_agent_report_at,
               opening_deadline_at, created_at, updated_at
        FROM helpdesk_tickets
        ORDER BY created_at ASC, ticket_id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to read SQLite helpdesk tickets")?;

    Ok(rows
        .into_iter()
        .map(|row| TicketRecord {
            ticket_id: row.get("ticket_id"),
            client_id: row.get("client_id"),
            client_display_name: row.get("client_display_name"),
            device_id: row.get("device_id"),
            requested_by: row.get("requested_by"),
            title: row.get("title"),
            description: row.get("description"),
            difficulty: row.get("difficulty"),
            estimated_minutes: row.get("estimated_minutes"),
            summary: row.get("summary"),
            status: row.get("status"),
            assigned_agent_id: row.get("assigned_agent_id"),
            latest_agent_report: row.get("latest_agent_report"),
            latest_agent_report_by: row.get("latest_agent_report_by"),
            latest_agent_report_at: row.get("latest_agent_report_at"),
            opening_deadline_at: row.get("opening_deadline_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

async fn fetch_sqlite_assignments(pool: &SqlitePool) -> anyhow::Result<Vec<AssignmentRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, ticket_id, agent_id, status, created_at, updated_at
        FROM helpdesk_ticket_assignments
        ORDER BY id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to read SQLite helpdesk assignments")?;

    Ok(rows
        .into_iter()
        .map(|row| AssignmentRecord {
            id: row.get("id"),
            ticket_id: row.get("ticket_id"),
            agent_id: row.get("agent_id"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

async fn fetch_sqlite_heartbeats(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<Vec<HeartbeatRecord>> {
    let rows = if let Some(sync_cfg) = sync_cfg {
        let cutoff = unix_millis_now() - sync_cfg.helpdesk_heartbeat_retention_ms;
        sqlx::query(
            r#"
            SELECT id, agent_id, status, created_at
            FROM helpdesk_agent_heartbeats
            WHERE created_at >= ?1
            ORDER BY id ASC
            "#,
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await
        .context("failed to read retained SQLite helpdesk heartbeats")?
    } else {
        sqlx::query(
            r#"
            SELECT id, agent_id, status, created_at
            FROM helpdesk_agent_heartbeats
            ORDER BY id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to read SQLite helpdesk heartbeats")?
    };

    Ok(rows
        .into_iter()
        .map(|row| HeartbeatRecord {
            id: row.get("id"),
            agent_id: row.get("agent_id"),
            status: row.get("status"),
            created_at: row.get("created_at"),
        })
        .collect())
}

async fn fetch_sqlite_audit_events(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<Vec<AuditEventRecord>> {
    let rows = if let Some(sync_cfg) = sync_cfg {
        let cutoff = unix_millis_now() - sync_cfg.helpdesk_audit_retention_ms;
        sqlx::query(
            r#"
            SELECT id, entity_type, entity_id, event_type, payload, created_at
            FROM helpdesk_audit_events
            WHERE created_at >= ?1
            ORDER BY id ASC
            "#,
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await
        .context("failed to read retained SQLite helpdesk audit events")?
    } else {
        sqlx::query(
            r#"
            SELECT id, entity_type, entity_id, event_type, payload, created_at
            FROM helpdesk_audit_events
            ORDER BY id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to read SQLite helpdesk audit events")?
    };

    Ok(rows
        .into_iter()
        .map(|row| AuditEventRecord {
            id: row.get("id"),
            entity_type: row.get("entity_type"),
            entity_id: row.get("entity_id"),
            event_type: row.get("event_type"),
            payload: row.get("payload"),
            created_at: row.get("created_at"),
        })
        .collect())
}

async fn fetch_turso_authorized_agents(conn: &Connection) -> anyhow::Result<Vec<AuthorizedAgentRecord>> {
    let mut rows = conn
        .query(
            r#"
            SELECT agent_id, display_name, created_at, updated_at
            FROM helpdesk_authorized_agents
            ORDER BY created_at ASC, agent_id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso authorized helpdesk agents")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(AuthorizedAgentRecord {
            agent_id: row.get(0)?,
            display_name: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_agents(conn: &Connection) -> anyhow::Result<Vec<AgentRecord>> {
    let mut rows = conn
        .query(
            r#"
            SELECT agent_id, display_name, avatar_url, status, current_ticket_id, last_heartbeat_at, created_at, updated_at
            FROM helpdesk_agents
            ORDER BY created_at ASC, agent_id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso helpdesk agents")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(AgentRecord {
            agent_id: row.get(0)?,
            display_name: row.get(1)?,
            avatar_url: row.get(2)?,
            status: row.get(3)?,
            current_ticket_id: row.get(4)?,
            last_heartbeat_at: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_tickets(conn: &Connection) -> anyhow::Result<Vec<TicketRecord>> {
    let mut rows = conn
        .query(
            r#"
            SELECT ticket_id, client_id, client_display_name, device_id, requested_by, title,
                   description, difficulty, estimated_minutes, summary, status, assigned_agent_id,
                   latest_agent_report, latest_agent_report_by, latest_agent_report_at,
                   opening_deadline_at, created_at, updated_at
            FROM helpdesk_tickets
            ORDER BY created_at ASC, ticket_id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso helpdesk tickets")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(TicketRecord {
            ticket_id: row.get(0)?,
            client_id: row.get(1)?,
            client_display_name: row.get(2)?,
            device_id: row.get(3)?,
            requested_by: row.get(4)?,
            title: row.get(5)?,
            description: row.get(6)?,
            difficulty: row.get(7)?,
            estimated_minutes: row.get(8)?,
            summary: row.get(9)?,
            status: row.get(10)?,
            assigned_agent_id: row.get(11)?,
            latest_agent_report: row.get(12)?,
            latest_agent_report_by: row.get(13)?,
            latest_agent_report_at: row.get(14)?,
            opening_deadline_at: row.get(15)?,
            created_at: row.get(16)?,
            updated_at: row.get(17)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_assignments(conn: &Connection) -> anyhow::Result<Vec<AssignmentRecord>> {
    let mut rows = conn
        .query(
            r#"
            SELECT id, ticket_id, agent_id, status, created_at, updated_at
            FROM helpdesk_ticket_assignments
            ORDER BY id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso helpdesk assignments")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(AssignmentRecord {
            id: row.get(0)?,
            ticket_id: row.get(1)?,
            agent_id: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_heartbeats(conn: &Connection) -> anyhow::Result<Vec<HeartbeatRecord>> {
    let mut rows = conn
        .query(
            r#"
            SELECT id, agent_id, status, created_at
            FROM helpdesk_agent_heartbeats
            ORDER BY id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso helpdesk heartbeats")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(HeartbeatRecord {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            status: row.get(2)?,
            created_at: row.get(3)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_audit_events(conn: &Connection) -> anyhow::Result<Vec<AuditEventRecord>> {
    let mut rows = conn
        .query(
            r#"
            SELECT id, entity_type, entity_id, event_type, payload, created_at
            FROM helpdesk_audit_events
            ORDER BY id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso helpdesk audit events")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(AuditEventRecord {
            id: row.get(0)?,
            entity_type: row.get(1)?,
            entity_id: row.get(2)?,
            event_type: row.get(3)?,
            payload: row.get(4)?,
            created_at: row.get(5)?,
        });
    }
    Ok(values)
}

#[derive(Debug, Clone, Default)]
struct MonitoringSnapshot {
    outbox_events: Vec<OutboxEventRow>,
    session_events: Vec<SessionEventRow>,
    session_presence: Vec<SessionPresenceRow>,
}

impl MonitoringSnapshot {
    fn counts(&self) -> MonitoringSnapshotCounts {
        MonitoringSnapshotCounts {
            outbox_events: self.outbox_events.len(),
            session_events: self.session_events.len(),
            session_presence: self.session_presence.len(),
        }
    }
}

#[derive(Debug, Clone)]
struct OutboxEventRow {
    event_id: String,
    payload: String,
    status: String,
    attempts: i64,
    next_attempt_at: i64,
    created_at: i64,
    updated_at: i64,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionEventRow {
    event_id: String,
    event_type: String,
    session_id: String,
    user_id: String,
    direction: String,
    timestamp: String,
    payload: String,
    created_at: i64,
}

#[derive(Debug, Clone)]
struct SessionPresenceRow {
    session_id: String,
    participant_id: String,
    display_name: String,
    avatar_url: Option<String>,
    is_active: i64,
    is_control_active: i64,
    last_activity_at: i64,
    updated_at: i64,
}

async fn count_monitoring_rows_sqlite(
    pool: &SqlitePool,
) -> anyhow::Result<MonitoringSnapshotCounts> {
    Ok(MonitoringSnapshotCounts {
        outbox_events: count_sqlite_table(pool, "outbox_events").await?,
        session_events: count_sqlite_table(pool, "session_events").await?,
        session_presence: count_sqlite_table(pool, "session_presence").await?,
    })
}

async fn count_monitoring_rows_turso(
    conn: &Connection,
) -> anyhow::Result<MonitoringSnapshotCounts> {
    Ok(MonitoringSnapshotCounts {
        outbox_events: count_turso_table(conn, "outbox_events").await?,
        session_events: count_turso_table(conn, "session_events").await?,
        session_presence: count_turso_table(conn, "session_presence").await?,
    })
}

async fn fetch_monitoring_snapshot_from_sqlite(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<MonitoringSnapshot> {
    Ok(MonitoringSnapshot {
        outbox_events: fetch_sqlite_outbox_events(pool, sync_cfg).await?,
        session_events: fetch_sqlite_session_events(pool, sync_cfg).await?,
        session_presence: fetch_sqlite_session_presence(pool, sync_cfg).await?,
    })
}

async fn fetch_monitoring_snapshot_from_turso(
    conn: &Connection,
) -> anyhow::Result<MonitoringSnapshot> {
    Ok(MonitoringSnapshot {
        outbox_events: fetch_turso_outbox_events(conn).await?,
        session_events: fetch_turso_session_events(conn).await?,
        session_presence: fetch_turso_session_presence(conn).await?,
    })
}

async fn apply_monitoring_snapshot_to_sqlite(
    pool: &SqlitePool,
    snapshot: &MonitoringSnapshot,
) -> anyhow::Result<()> {
    init_sqlite_schema(pool).await?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to open SQLite transaction for monitoring Turso restore")?;

    for table in ["session_presence", "session_events", "outbox_events"] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("failed to clear SQLite table '{table}' during monitoring restore")
            })?;
    }

    for row in &snapshot.outbox_events {
        sqlx::query(
            r#"
            INSERT INTO outbox_events (
                event_id, payload, status, attempts, next_attempt_at, created_at, updated_at, last_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(&row.event_id)
        .bind(&row.payload)
        .bind(&row.status)
        .bind(row.attempts)
        .bind(row.next_attempt_at)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(&row.last_error)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore outbox event '{}' into SQLite", row.event_id))?;
    }

    for row in &snapshot.session_events {
        sqlx::query(
            r#"
            INSERT INTO session_events (
                event_id, event_type, session_id, user_id, direction, timestamp, payload, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(&row.event_id)
        .bind(&row.event_type)
        .bind(&row.session_id)
        .bind(&row.user_id)
        .bind(&row.direction)
        .bind(&row.timestamp)
        .bind(&row.payload)
        .bind(row.created_at)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to restore session event '{}' into SQLite",
                row.event_id
            )
        })?;
    }

    for row in &snapshot.session_presence {
        sqlx::query(
            r#"
            INSERT INTO session_presence (
                session_id, participant_id, display_name, avatar_url, is_active,
                is_control_active, last_activity_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(&row.session_id)
        .bind(&row.participant_id)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(row.is_active)
        .bind(row.is_control_active)
        .bind(row.last_activity_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "failed to restore session presence '{}:{}' into SQLite",
                row.session_id, row.participant_id
            )
        })?;
    }

    tx.commit()
        .await
        .context("failed to commit SQLite monitoring Turso restore transaction")?;
    Ok(())
}

async fn apply_monitoring_snapshot_to_turso(
    conn: &Connection,
    snapshot: &MonitoringSnapshot,
) -> anyhow::Result<()> {
    init_libsql_schema(conn).await?;
    let tx = conn
        .transaction()
        .await
        .context("failed to open Turso transaction for monitoring sync")?;

    for table in ["session_presence", "session_events", "outbox_events"] {
        tx.execute(&format!("DELETE FROM {table}"), ())
            .await
            .with_context(|| {
                format!("failed to clear Turso table '{table}' during monitoring sync")
            })?;
    }

    for row in &snapshot.outbox_events {
        tx.execute(
            r#"
            INSERT INTO outbox_events (
                event_id, payload, status, attempts, next_attempt_at, created_at, updated_at, last_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            (
                row.event_id.as_str(),
                row.payload.as_str(),
                row.status.as_str(),
                row.attempts,
                row.next_attempt_at,
                row.created_at,
                row.updated_at,
                row.last_error.as_deref(),
            ),
        )
        .await
        .with_context(|| format!("failed to sync outbox event '{}' to Turso", row.event_id))?;
    }

    for row in &snapshot.session_events {
        tx.execute(
            r#"
            INSERT INTO session_events (
                event_id, event_type, session_id, user_id, direction, timestamp, payload, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            (
                row.event_id.as_str(),
                row.event_type.as_str(),
                row.session_id.as_str(),
                row.user_id.as_str(),
                row.direction.as_str(),
                row.timestamp.as_str(),
                row.payload.as_str(),
                row.created_at,
            ),
        )
        .await
        .with_context(|| format!("failed to sync session event '{}' to Turso", row.event_id))?;
    }

    for row in &snapshot.session_presence {
        tx.execute(
            r#"
            INSERT INTO session_presence (
                session_id, participant_id, display_name, avatar_url, is_active,
                is_control_active, last_activity_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            (
                row.session_id.as_str(),
                row.participant_id.as_str(),
                row.display_name.as_str(),
                row.avatar_url.as_deref(),
                row.is_active,
                row.is_control_active,
                row.last_activity_at,
                row.updated_at,
            ),
        )
        .await
        .with_context(|| {
            format!(
                "failed to sync session presence '{}:{}' to Turso",
                row.session_id, row.participant_id
            )
        })?;
    }

    tx.commit()
        .await
        .context("failed to commit Turso monitoring sync transaction")?;
    Ok(())
}

async fn fetch_sqlite_outbox_events(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<Vec<OutboxEventRow>> {
    let rows = if let Some(sync_cfg) = sync_cfg {
        let cutoff = unix_millis_now() - sync_cfg.monitoring_outbox_retention_ms;
        sqlx::query(
            r#"
            SELECT event_id, payload, status, attempts, next_attempt_at, created_at, updated_at, last_error
            FROM outbox_events
            WHERE updated_at >= ?1
            ORDER BY created_at ASC, event_id ASC
            "#,
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await
        .context("failed to read retained SQLite outbox events")?
    } else {
        sqlx::query(
            r#"
            SELECT event_id, payload, status, attempts, next_attempt_at, created_at, updated_at, last_error
            FROM outbox_events
            ORDER BY created_at ASC, event_id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to read SQLite outbox events")?
    };

    Ok(rows
        .into_iter()
        .map(|row| OutboxEventRow {
            event_id: row.get("event_id"),
            payload: row.get("payload"),
            status: row.get("status"),
            attempts: row.get("attempts"),
            next_attempt_at: row.get("next_attempt_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            last_error: row.get("last_error"),
        })
        .collect())
}

async fn fetch_sqlite_session_events(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<Vec<SessionEventRow>> {
    let rows = if let Some(sync_cfg) = sync_cfg {
        let cutoff = unix_millis_now() - sync_cfg.monitoring_session_event_retention_ms;
        sqlx::query(
            r#"
            SELECT event_id, event_type, session_id, user_id, direction, timestamp, payload, created_at
            FROM session_events
            WHERE created_at >= ?1
            ORDER BY created_at ASC, event_id ASC
            "#,
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await
        .context("failed to read retained SQLite session events")?
    } else {
        sqlx::query(
            r#"
            SELECT event_id, event_type, session_id, user_id, direction, timestamp, payload, created_at
            FROM session_events
            ORDER BY created_at ASC, event_id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to read SQLite session events")?
    };

    Ok(rows
        .into_iter()
        .map(|row| SessionEventRow {
            event_id: row.get("event_id"),
            event_type: row.get("event_type"),
            session_id: row.get("session_id"),
            user_id: row.get("user_id"),
            direction: row.get("direction"),
            timestamp: row.get("timestamp"),
            payload: row.get("payload"),
            created_at: row.get("created_at"),
        })
        .collect())
}

async fn fetch_sqlite_session_presence(
    pool: &SqlitePool,
    sync_cfg: Option<&TursoSyncConfig>,
) -> anyhow::Result<Vec<SessionPresenceRow>> {
    let rows = if let Some(sync_cfg) = sync_cfg {
        let cutoff = unix_millis_now() - sync_cfg.monitoring_presence_retention_ms;
        sqlx::query(
            r#"
            SELECT session_id, participant_id, display_name, avatar_url, is_active,
                   is_control_active, last_activity_at, updated_at
            FROM session_presence
            WHERE is_active = 1 OR updated_at >= ?1
            ORDER BY session_id ASC, participant_id ASC
            "#,
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await
        .context("failed to read retained SQLite session presence")?
    } else {
        sqlx::query(
            r#"
            SELECT session_id, participant_id, display_name, avatar_url, is_active,
                   is_control_active, last_activity_at, updated_at
            FROM session_presence
            ORDER BY session_id ASC, participant_id ASC
            "#,
        )
        .fetch_all(pool)
        .await
        .context("failed to read SQLite session presence")?
    };

    Ok(rows
        .into_iter()
        .map(|row| SessionPresenceRow {
            session_id: row.get("session_id"),
            participant_id: row.get("participant_id"),
            display_name: row.get("display_name"),
            avatar_url: row.get("avatar_url"),
            is_active: row.get("is_active"),
            is_control_active: row.get("is_control_active"),
            last_activity_at: row.get("last_activity_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

async fn fetch_turso_outbox_events(conn: &Connection) -> anyhow::Result<Vec<OutboxEventRow>> {
    let mut rows = conn
        .query(
            r#"
            SELECT event_id, payload, status, attempts, next_attempt_at, created_at, updated_at, last_error
            FROM outbox_events
            ORDER BY created_at ASC, event_id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso outbox events")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(OutboxEventRow {
            event_id: row.get(0)?,
            payload: row.get(1)?,
            status: row.get(2)?,
            attempts: row.get(3)?,
            next_attempt_at: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            last_error: row.get(7)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_session_events(conn: &Connection) -> anyhow::Result<Vec<SessionEventRow>> {
    let mut rows = conn
        .query(
            r#"
            SELECT event_id, event_type, session_id, user_id, direction, timestamp, payload, created_at
            FROM session_events
            ORDER BY created_at ASC, event_id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso session events")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(SessionEventRow {
            event_id: row.get(0)?,
            event_type: row.get(1)?,
            session_id: row.get(2)?,
            user_id: row.get(3)?,
            direction: row.get(4)?,
            timestamp: row.get(5)?,
            payload: row.get(6)?,
            created_at: row.get(7)?,
        });
    }
    Ok(values)
}

async fn fetch_turso_session_presence(conn: &Connection) -> anyhow::Result<Vec<SessionPresenceRow>> {
    let mut rows = conn
        .query(
            r#"
            SELECT session_id, participant_id, display_name, avatar_url, is_active,
                   is_control_active, last_activity_at, updated_at
            FROM session_presence
            ORDER BY session_id ASC, participant_id ASC
            "#,
            (),
        )
        .await
        .context("failed to read Turso session presence")?;

    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(SessionPresenceRow {
            session_id: row.get(0)?,
            participant_id: row.get(1)?,
            display_name: row.get(2)?,
            avatar_url: row.get(3)?,
            is_active: row.get(4)?,
            is_control_active: row.get(5)?,
            last_activity_at: row.get(6)?,
            updated_at: row.get(7)?,
        });
    }
    Ok(values)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
}

fn env_i64(key: &str) -> Option<i64> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
}
