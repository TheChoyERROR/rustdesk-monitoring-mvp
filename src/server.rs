use std::convert::Infallible;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, get_service, post};
use axum::{Extension, Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{debug, error, info, warn};

use crate::auth::{self, AuthSettings, DASHBOARD_SESSION_COOKIE};
use crate::config::ServerConfig;
use crate::helpdesk_agent_auth::HELPDESK_AGENT_TOKEN_HEADER;
use crate::metrics::Metrics;
use crate::model::{
    AuthLoginRequestV1, AuthLoginResponseV1, AuthRoleV1, AuthUserV1, HelpdeskAgentPresenceUpdateV1,
    HelpdeskAgentStatus, HelpdeskAssignmentStartRequestV1, HelpdeskAuthorizedAgentProvisioningV1,
    HelpdeskAuthorizedAgentUpsertRequestV1, HelpdeskTicketAgentReportCreateRequestV1,
    HelpdeskTicketAssignRequestV1, HelpdeskTicketCreateRequestV1,
    HelpdeskTicketOperationalUpdateRequestV1, HelpdeskTicketResolveRequestV1,
    HelpdeskTicketSupervisorActionRequestV1, PaginatedResponseV1, SessionActorTypeV1,
    SessionEventType, SessionEventV1,
};
use crate::postgres::{connect_postgres, init_postgres_helpdesk_schema};
use crate::postgres_helpdesk::{
    add_helpdesk_ticket_agent_report_pg, assign_helpdesk_ticket_pg, cancel_helpdesk_ticket_pg,
    create_helpdesk_ticket_pg, delete_helpdesk_authorized_agent_pg,
    ensure_postgres_helpdesk_agent_auth_schema, get_helpdesk_agent_authorization_status_pg,
    get_helpdesk_assignment_for_agent_pg, get_helpdesk_operational_summary_pg,
    get_helpdesk_ticket_pg, list_helpdesk_agents_pg, list_helpdesk_authorized_agents_pg,
    list_helpdesk_ticket_audit_pg, list_helpdesk_tickets_pg,
    provision_helpdesk_authorized_agent_pg, reconcile_helpdesk_runtime_pg,
    requeue_helpdesk_ticket_pg, resolve_helpdesk_ticket_pg, should_store_session_event_pg,
    start_helpdesk_ticket_pg, update_helpdesk_ticket_operational_fields_pg,
    upsert_helpdesk_agent_presence_pg, verify_helpdesk_agent_token_pg,
};
use crate::postgres_monitoring::{
    claim_due_events_pg, cleanup_delivered_older_than_pg, cleanup_failed_older_than_pg,
    cleanup_inactive_session_presence_older_than_pg, cleanup_session_events_older_than_pg,
    expire_stale_presence_pg, get_dashboard_summary_pg, get_session_presence_pg,
    init_postgres_monitoring_schema, insert_event_pg, list_active_session_presence_pg,
    mark_delivered_pg, mark_failed_pg, query_session_report_rows_pg, query_timeline_events_pg,
    reset_stuck_processing_pg, schedule_retry_pg,
};
use crate::storage::{
    add_helpdesk_ticket_agent_report, assign_helpdesk_ticket, cancel_helpdesk_ticket,
    claim_due_events, cleanup_delivered_older_than, cleanup_expired_dashboard_sessions,
    cleanup_failed_older_than, cleanup_helpdesk_agent_heartbeats_older_than,
    cleanup_inactive_session_presence_older_than, cleanup_session_events_older_than,
    connect_sqlite, create_helpdesk_ticket, delete_dashboard_session,
    delete_helpdesk_authorized_agent, delete_outbox_event, expire_stale_presence,
    get_dashboard_session_by_token, get_dashboard_summary, get_dashboard_user_by_username,
    get_helpdesk_agent_authorization_status, get_helpdesk_assignment_for_agent,
    get_helpdesk_operational_summary, get_helpdesk_ticket, get_session_presence, insert_event,
    list_active_session_presence, list_helpdesk_agents, list_helpdesk_authorized_agents,
    list_helpdesk_ticket_audit_events, list_helpdesk_tickets, mark_delivered, mark_failed,
    provision_helpdesk_authorized_agent, query_session_report_rows, query_timeline_events,
    reconcile_helpdesk_runtime, requeue_helpdesk_ticket, reset_stuck_processing,
    resolve_helpdesk_ticket, schedule_retry, should_store_session_event, start_helpdesk_ticket,
    unix_millis_now, update_helpdesk_ticket_operational_fields, upsert_dashboard_user,
    upsert_helpdesk_agent_presence, verify_helpdesk_agent_token, EventQueryFilter, InsertOutcome,
    OutboxRecord,
};
use crate::turso::{
    compute_helpdesk_sync_signature, compute_monitoring_sync_signature,
    initialize_helpdesk_turso_bridge, initialize_monitoring_turso_bridge,
    sync_helpdesk_snapshot_to_turso, sync_monitoring_snapshot_to_turso, TursoSyncConfig,
};
use crate::webhook::WebhookDispatcher;

const MAX_SESSION_EVENT_BODY_BYTES: usize = 4 * 1024 * 1024;
const HELPDESK_RECONCILE_INTERVAL_MS: u64 = 5_000;
const HELPDESK_AGENT_STALE_AFTER_MS: i64 = 30_000;

#[derive(Clone)]
struct AppState {
    pool: sqlx::SqlitePool,
    helpdesk_postgres: Option<sqlx::PgPool>,
    helpdesk_backend: HelpdeskBackend,
    monitoring_backend: MonitoringBackend,
    sqlite_fallback_enabled: bool,
    helpdesk_turso: Option<TursoSyncConfig>,
    metrics: Arc<Metrics>,
    dispatcher: WebhookDispatcher,
    config: Arc<ServerConfig>,
    auth: Arc<AuthSettings>,
    circuit_breaker: Arc<CircuitBreaker>,
    presence_updates: broadcast::Sender<String>,
}

#[derive(Debug, Clone)]
struct AuthContext {
    user: AuthUserV1,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpdeskBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitoringBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Deserialize)]
struct SummaryQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct EventListQuery {
    session_id: Option<String>,
    user_id: Option<String>,
    actor_type: Option<String>,
    event_type: Option<String>,
    from: Option<String>,
    to: Option<String>,
    page: Option<u64>,
    page_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TimelineQuery {
    actor_type: Option<String>,
    page: Option<u64>,
    page_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct CsvReportQuery {
    from: Option<String>,
    to: Option<String>,
    user_id: Option<String>,
    actor_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct HelpdeskAuditQuery {
    limit: Option<u64>,
}

pub async fn run(
    bind_addr: &str,
    database_path: &Path,
    config: ServerConfig,
) -> anyhow::Result<()> {
    validate_server_config(&config)?;
    info!(
        stale_after_seconds = config.presence.stale_after_seconds,
        cleanup_interval_seconds = config.presence.cleanup_interval_seconds,
        "presence cleanup configuration"
    );
    info!(
        capture_non_agent_events = config.monitoring.capture_non_agent_events,
        participant_activity_min_interval_seconds =
            config.monitoring.participant_activity_min_interval_seconds,
        local_delivered_outbox_retention_days =
            config.monitoring.local_delivered_outbox_retention_days,
        local_session_event_retention_days = config.monitoring.local_session_event_retention_days,
        local_session_presence_retention_hours =
            config.monitoring.local_session_presence_retention_hours,
        local_agent_heartbeat_retention_days =
            config.monitoring.local_agent_heartbeat_retention_days,
        "monitoring ingest and retention configuration"
    );

    let pool = connect_sqlite(database_path).await?;
    let auth = Arc::new(AuthSettings::from_env());
    let supervisor_password_hash = auth::hash_password(&auth.supervisor_password)
        .context("failed to hash supervisor dashboard password")?;
    upsert_dashboard_user(
        &pool,
        &auth.supervisor_username,
        &supervisor_password_hash,
        AuthRoleV1::Supervisor,
    )
    .await
    .context("failed to seed dashboard supervisor user")?;

    let metrics = Arc::new(Metrics::default());
    let dispatcher = WebhookDispatcher::new(config.webhook.clone())?;
    let helpdesk_postgres = match env::var("HELPDESK_POSTGRES_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("DATABASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
        }) {
        Some(database_url) => match connect_postgres(&database_url).await {
            Ok(pool) => match init_postgres_helpdesk_schema(&pool).await {
                Ok(()) => match ensure_postgres_helpdesk_agent_auth_schema(&pool).await {
                    Ok(()) => match init_postgres_monitoring_schema(&pool).await {
                        Ok(()) => {
                            info!("Postgres helpdesk and monitoring pools initialized");
                            Some(pool)
                        }
                        Err(err) => {
                            error!(
                                error = %err,
                                "failed to initialize Postgres monitoring schema; continuing without Postgres-backed monitoring runtime"
                            );
                            None
                        }
                    },
                    Err(err) => {
                        error!(
                            error = %err,
                            "failed to initialize Postgres helpdesk agent auth schema; continuing without Postgres-backed runtime"
                        );
                        None
                    }
                },
                Err(err) => {
                    error!(
                        error = %err,
                        "failed to initialize Postgres helpdesk schema; continuing without Postgres-backed helpdesk runtime"
                    );
                    None
                }
            },
            Err(err) => {
                error!(
                    error = %err,
                    "failed to initialize Postgres runtime pool; continuing without Postgres-backed runtime"
                );
                None
            }
        },
        None => {
            info!("Postgres runtime disabled; HELPDESK_POSTGRES_DATABASE_URL/DATABASE_URL not set");
            None
        }
    };

    let monitoring_backend = match env::var("PRIMARY_DB_BACKEND")
        .unwrap_or_else(|_| "sqlite".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "postgres" if helpdesk_postgres.is_some() => {
            info!("monitoring runtime configured to use Postgres as primary backend");
            MonitoringBackend::Postgres
        }
        "postgres" => {
            warn!(
                "PRIMARY_DB_BACKEND=postgres was requested, but Postgres is unavailable; falling back to SQLite"
            );
            MonitoringBackend::Sqlite
        }
        "sqlite" => MonitoringBackend::Sqlite,
        other => {
            warn!(
                value = other,
                "unknown PRIMARY_DB_BACKEND value; falling back to SQLite"
            );
            MonitoringBackend::Sqlite
        }
    };

    let sqlite_fallback_enabled = env::var("SQLITE_FALLBACK_ENABLED")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true);

    let helpdesk_backend = match env::var("HELPDESK_BACKEND")
        .unwrap_or_else(|_| "sqlite".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "postgres" if helpdesk_postgres.is_some() => {
            info!("helpdesk backend runtime configured to use Postgres");
            HelpdeskBackend::Postgres
        }
        "postgres" => {
            warn!(
                "HELPDESK_BACKEND=postgres was requested, but Postgres is unavailable; falling back to SQLite"
            );
            HelpdeskBackend::Sqlite
        }
        "sqlite" => HelpdeskBackend::Sqlite,
        other => {
            warn!(
                value = other,
                "unknown HELPDESK_BACKEND value; falling back to SQLite"
            );
            HelpdeskBackend::Sqlite
        }
    };

    let mut helpdesk_turso = TursoSyncConfig::from_env();
    if helpdesk_backend == HelpdeskBackend::Postgres
        || monitoring_backend == MonitoringBackend::Postgres
    {
        if helpdesk_turso.is_some() {
            info!("Turso bridges disabled because Postgres is the active runtime backend");
        }
        helpdesk_turso = None;
    } else if let Some(sync_cfg) = helpdesk_turso.clone() {
        match initialize_helpdesk_turso_bridge(&pool, &sync_cfg).await {
            Ok(summary) => {
                info!(
                    mode = summary.mode,
                    local_rows = summary.local_counts.total_rows(),
                    remote_rows = summary.remote_counts.total_rows(),
                    local_tickets = summary.local_counts.tickets,
                    remote_tickets = summary.remote_counts.tickets,
                    "helpdesk Turso bridge initialized"
                );

                match initialize_monitoring_turso_bridge(&pool, &sync_cfg).await {
                    Ok(monitoring_summary) => {
                        info!(
                            mode = monitoring_summary.mode,
                            local_rows = monitoring_summary.local_counts.total_rows(),
                            remote_rows = monitoring_summary.remote_counts.total_rows(),
                            local_session_events = monitoring_summary.local_counts.session_events,
                            remote_session_events = monitoring_summary.remote_counts.session_events,
                            local_presence_rows = monitoring_summary.local_counts.session_presence,
                            remote_presence_rows =
                                monitoring_summary.remote_counts.session_presence,
                            local_outbox_rows = monitoring_summary.local_counts.outbox_events,
                            remote_outbox_rows = monitoring_summary.remote_counts.outbox_events,
                            "monitoring Turso bridge initialized"
                        );
                    }
                    Err(err) => {
                        error!(
                            error = %err,
                            "failed to initialize Turso monitoring bridge; continuing with local SQLite only"
                        );
                        helpdesk_turso = None;
                    }
                }
            }
            Err(err) => {
                error!(
                    error = %err,
                    "failed to initialize Turso helpdesk bridge; continuing with local SQLite only"
                );
                helpdesk_turso = None;
            }
        }
    } else {
        info!("Turso bridges disabled; TURSO_DATABASE_URL/TURSO_AUTH_TOKEN not set");
    }

    let circuit_breaker = Arc::new(CircuitBreaker::new(
        config.worker.circuit_breaker_threshold,
        config.worker.circuit_breaker_cooldown_ms,
    ));
    let (presence_updates, _) = broadcast::channel(1024);

    let state = AppState {
        pool,
        helpdesk_postgres,
        helpdesk_backend,
        monitoring_backend,
        sqlite_fallback_enabled,
        helpdesk_turso,
        metrics,
        dispatcher,
        config: Arc::new(config),
        auth,
        circuit_breaker,
        presence_updates,
    };

    let reset_count = reset_stuck_processing(&state.pool, 60_000, unix_millis_now()).await?;
    if reset_count > 0 {
        warn!(reset_count, "reset stale processing rows on startup");
    }

    let protected_routes = Router::new()
        .route("/api/v1/auth/me", get(auth_me_handler))
        .route("/api/v1/dashboard/summary", get(dashboard_summary_handler))
        .route("/api/v1/events", get(events_list_handler))
        .route("/api/v1/helpdesk/agents", get(list_helpdesk_agents_handler))
        .route(
            "/api/v1/helpdesk/summary",
            get(get_helpdesk_summary_handler),
        )
        .route(
            "/api/v1/helpdesk/agent-authorizations",
            get(list_helpdesk_authorized_agents_handler)
                .post(upsert_helpdesk_authorized_agent_handler),
        )
        .route(
            "/api/v1/helpdesk/agent-authorizations/:agent_id",
            axum::routing::delete(delete_helpdesk_authorized_agent_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets",
            get(list_helpdesk_tickets_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id",
            get(get_helpdesk_ticket_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/assign",
            post(assign_helpdesk_ticket_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/audit",
            get(list_helpdesk_ticket_audit_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/requeue",
            post(requeue_helpdesk_ticket_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/cancel",
            post(cancel_helpdesk_ticket_handler),
        )
        .route(
            "/api/v1/sessions/presence",
            get(list_presence_sessions_handler),
        )
        .route(
            "/api/v1/sessions/:session_id/presence",
            get(get_session_presence_handler),
        )
        .route(
            "/api/v1/sessions/:session_id/presence/stream",
            get(stream_session_presence_handler),
        )
        .route(
            "/api/v1/sessions/:session_id/timeline",
            get(session_timeline_handler),
        )
        .route(
            "/api/v1/reports/sessions.csv",
            get(sessions_report_csv_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_dashboard_auth,
        ));

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/api/v1/auth/login", post(auth_login_handler))
        .route("/api/v1/auth/logout", post(auth_logout_handler))
        .route(
            "/api/v1/helpdesk/agents/presence",
            post(upsert_helpdesk_agent_presence_handler),
        )
        .route(
            "/api/v1/helpdesk/agents/:agent_id/authorization",
            get(get_helpdesk_agent_authorization_handler),
        )
        .route(
            "/api/v1/helpdesk/agents/:agent_id/assignment",
            get(get_helpdesk_assignment_for_agent_handler),
        )
        .route(
            "/api/v1/helpdesk/agents/:agent_id/assignment/start",
            post(start_helpdesk_assignment_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets",
            post(create_helpdesk_ticket_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/operational",
            post(update_helpdesk_ticket_operational_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/report",
            post(create_helpdesk_ticket_agent_report_handler),
        )
        .route(
            "/api/v1/helpdesk/tickets/:ticket_id/resolve",
            post(resolve_helpdesk_ticket_handler),
        )
        .route(
            "/api/v1/session-events",
            post(ingest_session_event).layer(DefaultBodyLimit::max(MAX_SESSION_EVENT_BODY_BYTES)),
        )
        .merge(protected_routes)
        .with_state(state.clone());

    if let Some(dist_dir) = resolve_dashboard_dist_dir() {
        let index_file = dist_dir.join("index.html");
        if index_file.is_file() {
            info!(path = %dist_dir.display(), "dashboard static files enabled");
            let static_service =
                get_service(ServeDir::new(dist_dir).fallback(ServeFile::new(index_file)));
            router = router.fallback_service(static_service);
        } else {
            warn!(
                path = %index_file.display(),
                "dashboard dist path found but index.html is missing; static UI disabled"
            );
        }
    } else {
        info!("dashboard static files not found; API-only mode");
    }

    let mut background_jobs: Vec<JoinHandle<()>> = Vec::new();

    if state.dispatcher.enabled() {
        background_jobs.push(tokio::spawn(webhook_worker(state.clone())));
    } else {
        warn!("webhook is disabled; events will remain queued in outbox");
    }

    background_jobs.push(tokio::spawn(presence_cleanup_worker(state.clone())));
    background_jobs.push(tokio::spawn(helpdesk_reconcile_worker(state.clone())));
    if state.helpdesk_turso.is_some() {
        background_jobs.push(tokio::spawn(turso_sync_worker(state.clone())));
    }
    background_jobs.push(tokio::spawn(cleanup_worker(state.clone())));
    background_jobs.push(tokio::spawn(sqlite_fallback_replay_worker(state.clone())));

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind server on {bind_addr}"))?;

    info!(bind_addr, "monitoring server listening");

    axum::serve(listener, router.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("axum server terminated with error")?;

    for job in background_jobs {
        job.abort();
    }

    Ok(())
}

fn validate_server_config(config: &ServerConfig) -> anyhow::Result<()> {
    if config.webhook.enabled {
        if config.webhook.url.is_none() {
            anyhow::bail!("webhook.enabled=true requires webhook.url");
        }
        if config.webhook.hmac.enabled
            && config
                .webhook
                .hmac
                .secret
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            anyhow::bail!("webhook.hmac.enabled=true requires webhook.hmac.secret");
        }
    }

    if config.worker.concurrency == 0 {
        anyhow::bail!("worker.concurrency must be greater than 0");
    }

    if config.presence.stale_after_seconds == 0 {
        anyhow::bail!("presence.stale_after_seconds must be greater than 0");
    }

    if config.presence.cleanup_interval_seconds == 0 {
        anyhow::bail!("presence.cleanup_interval_seconds must be greater than 0");
    }

    Ok(())
}

fn resolve_dashboard_dist_dir() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("DASHBOARD_DIST_DIR") {
        let candidate = PathBuf::from(raw.trim());
        if candidate.is_dir() {
            return Some(candidate);
        }
        warn!(
            path = %candidate.display(),
            "DASHBOARD_DIST_DIR is set but directory does not exist"
        );
    }

    let candidates = [
        PathBuf::from("web-dashboard/dist"),
        PathBuf::from("./web-dashboard/dist"),
        PathBuf::from("../web-dashboard/dist"),
    ];

    candidates.into_iter().find(|path| path.is_dir())
}

fn active_helpdesk_postgres_pool(state: &AppState) -> Option<&sqlx::PgPool> {
    if state.helpdesk_backend == HelpdeskBackend::Postgres {
        state.helpdesk_postgres.as_ref()
    } else {
        None
    }
}

fn active_monitoring_postgres_pool(state: &AppState) -> Option<&sqlx::PgPool> {
    if state.monitoring_backend == MonitoringBackend::Postgres {
        state.helpdesk_postgres.as_ref()
    } else {
        None
    }
}

async fn should_store_session_event_for_active_helpdesk_backend(
    state: &AppState,
    event: &SessionEventV1,
) -> anyhow::Result<bool> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        return should_store_session_event_pg(&state.pool, pool, event, &state.config.monitoring)
            .await;
    }

    should_store_session_event(&state.pool, event, &state.config.monitoring).await
}

async fn insert_session_event_for_active_monitoring_backend(
    state: &AppState,
    event: &SessionEventV1,
) -> anyhow::Result<InsertOutcome> {
    if let Some(pool) = active_monitoring_postgres_pool(state) {
        match insert_event_pg(pool, event).await {
            Ok(outcome) => return Ok(outcome),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    event_id = %event.event_id,
                    "failed to persist session event in Postgres; falling back to SQLite buffer"
                );
            }
            Err(err) => return Err(err),
        }
    }

    insert_event(&state.pool, event).await
}

async fn query_dashboard_summary_for_active_monitoring_backend(
    state: &AppState,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<crate::model::DashboardSummaryV1> {
    if let Some(pool) = active_monitoring_postgres_pool(state) {
        match get_dashboard_summary_pg(pool, from, to).await {
            Ok(summary) => return Ok(summary),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to query dashboard summary from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    get_dashboard_summary(&state.pool, from, to).await
}

async fn query_timeline_events_for_active_monitoring_backend(
    state: &AppState,
    filter: &EventQueryFilter,
    page: u64,
    page_size: u64,
) -> anyhow::Result<(Vec<crate::model::SessionTimelineItemV1>, u64)> {
    if let Some(pool) = active_monitoring_postgres_pool(state) {
        match query_timeline_events_pg(pool, filter, page, page_size).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to query timeline events from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    query_timeline_events(&state.pool, filter, page, page_size).await
}

async fn query_session_report_rows_for_active_monitoring_backend(
    state: &AppState,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    user_id: Option<&str>,
    actor_type: Option<SessionActorTypeV1>,
) -> anyhow::Result<Vec<crate::model::SessionReportRowV1>> {
    if let Some(pool) = active_monitoring_postgres_pool(state) {
        match query_session_report_rows_pg(pool, from, to, user_id, actor_type).await {
            Ok(rows) => return Ok(rows),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to query session report rows from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    query_session_report_rows(&state.pool, from, to, user_id, actor_type).await
}

async fn list_active_session_presence_for_active_monitoring_backend(
    state: &AppState,
) -> anyhow::Result<Vec<crate::model::PresenceSessionSummaryV1>> {
    if let Some(pool) = active_monitoring_postgres_pool(state) {
        match list_active_session_presence_pg(pool).await {
            Ok(rows) => return Ok(rows),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to list active session presence from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    list_active_session_presence(&state.pool).await
}

async fn get_session_presence_for_active_monitoring_backend(
    state: &AppState,
    session_id: &str,
) -> anyhow::Result<Option<crate::model::SessionPresenceV1>> {
    if let Some(pool) = active_monitoring_postgres_pool(state) {
        match get_session_presence_pg(pool, session_id).await {
            Ok(snapshot) => return Ok(snapshot),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    session_id,
                    "failed to query session presence from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    get_session_presence(&state.pool, session_id).await
}

async fn list_helpdesk_agents_for_active_backend(
    state: &AppState,
) -> anyhow::Result<Vec<crate::model::HelpdeskAgentV1>> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match list_helpdesk_agents_pg(pool).await {
            Ok(agents) => return Ok(agents),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to list helpdesk agents from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    list_helpdesk_agents(&state.pool).await
}

async fn list_helpdesk_authorized_agents_for_active_backend(
    state: &AppState,
) -> anyhow::Result<Vec<crate::model::HelpdeskAuthorizedAgentV1>> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match list_helpdesk_authorized_agents_pg(pool).await {
            Ok(agents) => return Ok(agents),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to list authorized helpdesk agents from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    list_helpdesk_authorized_agents(&state.pool).await
}

async fn provision_helpdesk_authorized_agent_for_active_backend(
    state: &AppState,
    payload: &HelpdeskAuthorizedAgentUpsertRequestV1,
) -> anyhow::Result<HelpdeskAuthorizedAgentProvisioningV1> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match provision_helpdesk_authorized_agent_pg(pool, payload).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id = payload.agent_id,
                    "failed to provision authorized helpdesk agent in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    provision_helpdesk_authorized_agent(&state.pool, payload).await
}

async fn delete_helpdesk_authorized_agent_for_active_backend(
    state: &AppState,
    agent_id: &str,
) -> anyhow::Result<bool> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match delete_helpdesk_authorized_agent_pg(pool, agent_id).await {
            Ok(deleted) => return Ok(deleted),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id,
                    "failed to delete authorized helpdesk agent from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    delete_helpdesk_authorized_agent(&state.pool, agent_id).await
}

async fn get_helpdesk_agent_authorization_for_active_backend(
    state: &AppState,
    agent_id: &str,
) -> anyhow::Result<crate::model::HelpdeskAgentAuthorizationStatusV1> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match get_helpdesk_agent_authorization_status_pg(pool, agent_id).await {
            Ok(status) => return Ok(status),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id,
                    "failed to query helpdesk agent authorization from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    get_helpdesk_agent_authorization_status(&state.pool, agent_id).await
}

async fn verify_helpdesk_agent_token_for_active_backend(
    state: &AppState,
    agent_id: &str,
    raw_token: &str,
) -> anyhow::Result<bool> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match verify_helpdesk_agent_token_pg(pool, agent_id, raw_token).await {
            Ok(valid) => return Ok(valid),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id,
                    "failed to verify helpdesk agent token in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    verify_helpdesk_agent_token(&state.pool, agent_id, raw_token).await
}

async fn get_helpdesk_summary_for_active_backend(
    state: &AppState,
) -> anyhow::Result<crate::model::HelpdeskOperationalSummaryV1> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match get_helpdesk_operational_summary_pg(pool).await {
            Ok(summary) => return Ok(summary),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to get helpdesk operational summary from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    get_helpdesk_operational_summary(&state.pool).await
}

async fn upsert_helpdesk_agent_presence_for_active_backend(
    state: &AppState,
    payload: &HelpdeskAgentPresenceUpdateV1,
) -> anyhow::Result<crate::model::HelpdeskAgentV1> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match upsert_helpdesk_agent_presence_pg(pool, payload).await {
            Ok(agent) => return Ok(agent),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id = payload.agent_id,
                    "failed to upsert helpdesk agent presence in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    upsert_helpdesk_agent_presence(&state.pool, payload).await
}

async fn get_helpdesk_assignment_for_agent_for_active_backend(
    state: &AppState,
    agent_id: &str,
) -> anyhow::Result<Option<crate::model::HelpdeskAssignmentV1>> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match get_helpdesk_assignment_for_agent_pg(pool, agent_id).await {
            Ok(assignment) => return Ok(assignment),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id,
                    "failed to get helpdesk assignment from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    get_helpdesk_assignment_for_agent(&state.pool, agent_id).await
}

async fn start_helpdesk_assignment_for_active_backend(
    state: &AppState,
    agent_id: &str,
    ticket_id: &str,
) -> anyhow::Result<(
    crate::model::HelpdeskTicketV1,
    crate::model::HelpdeskAgentV1,
)> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match start_helpdesk_ticket_pg(pool, agent_id, ticket_id).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    agent_id,
                    ticket_id,
                    "failed to start helpdesk assignment in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    start_helpdesk_ticket(&state.pool, agent_id, ticket_id).await
}

async fn list_helpdesk_tickets_for_active_backend(
    state: &AppState,
) -> anyhow::Result<Vec<crate::model::HelpdeskTicketV1>> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match list_helpdesk_tickets_pg(pool).await {
            Ok(tickets) => return Ok(tickets),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to list helpdesk tickets from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    list_helpdesk_tickets(&state.pool).await
}

async fn get_helpdesk_ticket_for_active_backend(
    state: &AppState,
    ticket_id: &str,
) -> anyhow::Result<Option<crate::model::HelpdeskTicketV1>> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match get_helpdesk_ticket_pg(pool, ticket_id).await {
            Ok(ticket) => return Ok(ticket),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    "failed to get helpdesk ticket from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    get_helpdesk_ticket(&state.pool, ticket_id).await
}

async fn list_helpdesk_ticket_audit_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    limit: u64,
) -> anyhow::Result<Vec<crate::model::HelpdeskAuditEventV1>> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match list_helpdesk_ticket_audit_pg(pool, ticket_id).await {
            Ok(events) => return Ok(events.into_iter().take(limit as usize).collect()),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    "failed to list helpdesk ticket audit from Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    list_helpdesk_ticket_audit_events(&state.pool, ticket_id, limit).await
}

async fn create_helpdesk_ticket_for_active_backend(
    state: &AppState,
    payload: &HelpdeskTicketCreateRequestV1,
) -> anyhow::Result<crate::model::HelpdeskTicketV1> {
    let mut sanitized_payload = payload.clone();
    sanitized_payload.difficulty = None;
    sanitized_payload.estimated_minutes = None;

    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match create_helpdesk_ticket_pg(pool, &sanitized_payload).await {
            Ok(ticket) => return Ok(ticket),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    client_id = payload.client_id,
                    "failed to create helpdesk ticket in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    create_helpdesk_ticket(&state.pool, &sanitized_payload).await
}

async fn assign_helpdesk_ticket_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    agent_id: Option<&str>,
    reason: Option<&str>,
) -> anyhow::Result<(
    crate::model::HelpdeskTicketV1,
    crate::model::HelpdeskAgentV1,
)> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match assign_helpdesk_ticket_pg(pool, ticket_id, agent_id, reason).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    agent_id,
                    "failed to assign helpdesk ticket in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    assign_helpdesk_ticket(&state.pool, ticket_id, agent_id, reason).await
}

async fn update_helpdesk_ticket_operational_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    difficulty: Option<&str>,
    estimated_minutes: Option<u32>,
) -> anyhow::Result<crate::model::HelpdeskTicketV1> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match update_helpdesk_ticket_operational_fields_pg(
            pool,
            ticket_id,
            difficulty,
            estimated_minutes,
        )
        .await
        {
            Ok(ticket) => return Ok(ticket),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    "failed to update helpdesk operational fields in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    update_helpdesk_ticket_operational_fields(&state.pool, ticket_id, difficulty, estimated_minutes)
        .await
}

async fn add_helpdesk_ticket_agent_report_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    agent_id: &str,
    note: &str,
) -> anyhow::Result<crate::model::HelpdeskTicketV1> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match add_helpdesk_ticket_agent_report_pg(pool, ticket_id, agent_id, note).await {
            Ok(ticket) => return Ok(ticket),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    agent_id,
                    "failed to add helpdesk agent report in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    add_helpdesk_ticket_agent_report(&state.pool, ticket_id, agent_id, note).await
}

async fn resolve_helpdesk_ticket_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    agent_id: &str,
    next_agent_status: HelpdeskAgentStatus,
) -> anyhow::Result<(
    crate::model::HelpdeskTicketV1,
    crate::model::HelpdeskAgentV1,
)> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match resolve_helpdesk_ticket_pg(pool, ticket_id, agent_id, next_agent_status).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    agent_id,
                    "failed to resolve helpdesk ticket in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    resolve_helpdesk_ticket(&state.pool, ticket_id, agent_id, next_agent_status).await
}

async fn requeue_helpdesk_ticket_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    next_agent_status: HelpdeskAgentStatus,
    reason: Option<&str>,
) -> anyhow::Result<(
    crate::model::HelpdeskTicketV1,
    Option<crate::model::HelpdeskAgentV1>,
)> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match requeue_helpdesk_ticket_pg(pool, ticket_id, next_agent_status, reason).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    "failed to requeue helpdesk ticket in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    requeue_helpdesk_ticket(&state.pool, ticket_id, next_agent_status, reason).await
}

async fn cancel_helpdesk_ticket_for_active_backend(
    state: &AppState,
    ticket_id: &str,
    next_agent_status: HelpdeskAgentStatus,
    reason: Option<&str>,
) -> anyhow::Result<(
    crate::model::HelpdeskTicketV1,
    Option<crate::model::HelpdeskAgentV1>,
)> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match cancel_helpdesk_ticket_pg(pool, ticket_id, next_agent_status, reason).await {
            Ok(result) => return Ok(result),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    ticket_id,
                    "failed to cancel helpdesk ticket in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    cancel_helpdesk_ticket(&state.pool, ticket_id, next_agent_status, reason).await
}

async fn reconcile_helpdesk_runtime_for_active_backend(
    state: &AppState,
    stale_after_ms: i64,
    now_ms: i64,
) -> anyhow::Result<crate::storage::HelpdeskRuntimeReconcileResult> {
    if let Some(pool) = active_helpdesk_postgres_pool(state) {
        match reconcile_helpdesk_runtime_pg(pool, stale_after_ms, now_ms).await {
            Ok(stats) => return Ok(stats),
            Err(err) if state.sqlite_fallback_enabled => {
                warn!(
                    error = %err,
                    "failed to reconcile helpdesk runtime in Postgres; falling back to SQLite"
                );
            }
            Err(err) => return Err(err),
        }
    }

    reconcile_helpdesk_runtime(&state.pool, stale_after_ms, now_ms).await
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.metrics.render_prometheus();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4"),
        )],
        body,
    )
}

async fn list_helpdesk_agents_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_helpdesk_agents_for_active_backend(&state).await {
        Ok(agents) => (StatusCode::OK, Json(json!({ "agents": agents }))).into_response(),
        Err(err) => {
            error!(error = %err, "failed to list helpdesk agents");
            internal_error()
        }
    }
}

async fn list_helpdesk_authorized_agents_handler(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match list_helpdesk_authorized_agents_for_active_backend(&state).await {
        Ok(agents) => (StatusCode::OK, Json(json!({ "agents": agents }))).into_response(),
        Err(err) => {
            error!(error = %err, "failed to list authorized helpdesk agents");
            internal_error()
        }
    }
}

async fn upsert_helpdesk_authorized_agent_handler(
    State(state): State<AppState>,
    Json(payload): Json<HelpdeskAuthorizedAgentUpsertRequestV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    match provision_helpdesk_authorized_agent_for_active_backend(&state, &payload).await {
        Ok(provisioning) => (
            StatusCode::OK,
            Json(json!({
                "agent": provisioning.agent,
                "agent_token": provisioning.agent_token,
            })),
        )
            .into_response(),
        Err(err) => {
            if err.to_string().contains("display name '") {
                return bad_request(err.to_string());
            }
            error!(
                error = %err,
                agent_id = payload.agent_id,
                "failed to upsert authorized helpdesk agent"
            );
            internal_error()
        }
    }
}

async fn delete_helpdesk_authorized_agent_handler(
    State(state): State<AppState>,
    AxumPath(agent_id): AxumPath<String>,
) -> impl IntoResponse {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return bad_request("agent_id cannot be empty");
    }

    match delete_helpdesk_authorized_agent_for_active_backend(&state, &agent_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "not_found",
                "message": "Authorized agent was not found",
            })),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, agent_id, "failed to delete authorized helpdesk agent");
            internal_error()
        }
    }
}

async fn get_helpdesk_agent_authorization_handler(
    State(state): State<AppState>,
    AxumPath(agent_id): AxumPath<String>,
) -> impl IntoResponse {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return bad_request("agent_id cannot be empty");
    }

    match get_helpdesk_agent_authorization_for_active_backend(&state, &agent_id).await {
        Ok(status) => (StatusCode::OK, Json(json!({ "authorization": status }))).into_response(),
        Err(err) => {
            error!(error = %err, agent_id, "failed to query helpdesk agent authorization");
            internal_error()
        }
    }
}

async fn get_helpdesk_summary_handler(State(state): State<AppState>) -> impl IntoResponse {
    match get_helpdesk_summary_for_active_backend(&state).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(err) => {
            error!(error = %err, "failed to get helpdesk operational summary");
            internal_error()
        }
    }
}

async fn upsert_helpdesk_agent_presence_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<HelpdeskAgentPresenceUpdateV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    if let Err(response) = require_helpdesk_agent_auth(&state, &headers, &payload.agent_id).await {
        return response;
    }

    match upsert_helpdesk_agent_presence_for_active_backend(&state, &payload).await {
        Ok(agent) => (StatusCode::OK, Json(json!({ "agent": agent }))).into_response(),
        Err(err) => {
            if err
                .to_string()
                .contains("is not authorized for helpdesk operator mode")
            {
                return forbidden(err.to_string());
            }
            if err.to_string().contains("display name '") {
                return bad_request(err.to_string());
            }
            error!(error = %err, agent_id = payload.agent_id, "failed to upsert helpdesk agent presence");
            internal_error()
        }
    }
}

async fn get_helpdesk_assignment_for_agent_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
) -> impl IntoResponse {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return bad_request("agent_id cannot be empty");
    }

    if let Err(response) = require_helpdesk_agent_auth(&state, &headers, &agent_id).await {
        return response;
    }

    match get_helpdesk_assignment_for_agent_for_active_backend(&state, &agent_id).await {
        Ok(Some(assignment)) => {
            (StatusCode::OK, Json(json!({ "assignment": assignment }))).into_response()
        }
        Ok(None) => (StatusCode::OK, Json(json!({ "assignment": null }))).into_response(),
        Err(err) => {
            error!(error = %err, agent_id, "failed to get helpdesk assignment for agent");
            internal_error()
        }
    }
}

async fn start_helpdesk_assignment_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    Json(payload): Json<HelpdeskAssignmentStartRequestV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    if let Err(response) = require_helpdesk_agent_auth(&state, &headers, &agent_id).await {
        return response;
    }

    match start_helpdesk_assignment_for_active_backend(&state, &agent_id, &payload.ticket_id).await
    {
        Ok((ticket, agent)) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
                "agent": agent,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(
                error = %err,
                agent_id,
                ticket_id = payload.ticket_id,
                "failed to start helpdesk assignment"
            );
            bad_request(err.to_string())
        }
    }
}

async fn list_helpdesk_tickets_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_helpdesk_tickets_for_active_backend(&state).await {
        Ok(tickets) => (StatusCode::OK, Json(json!({ "tickets": tickets }))).into_response(),
        Err(err) => {
            error!(error = %err, "failed to list helpdesk tickets");
            internal_error()
        }
    }
}

async fn get_helpdesk_ticket_handler(
    State(state): State<AppState>,
    AxumPath(ticket_id): AxumPath<String>,
) -> impl IntoResponse {
    let ticket_id = ticket_id.trim().to_string();
    if ticket_id.is_empty() {
        return bad_request("ticket_id cannot be empty");
    }

    match get_helpdesk_ticket_for_active_backend(&state, &ticket_id).await {
        Ok(Some(ticket)) => (StatusCode::OK, Json(json!({ "ticket": ticket }))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "ticket_not_found",
                "ticket_id": ticket_id,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, ticket_id, "failed to get helpdesk ticket");
            internal_error()
        }
    }
}

async fn list_helpdesk_ticket_audit_handler(
    State(state): State<AppState>,
    AxumPath(ticket_id): AxumPath<String>,
    Query(query): Query<HelpdeskAuditQuery>,
) -> impl IntoResponse {
    let ticket_id = ticket_id.trim().to_string();
    if ticket_id.is_empty() {
        return bad_request("ticket_id cannot be empty");
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    match list_helpdesk_ticket_audit_for_active_backend(&state, &ticket_id, limit).await {
        Ok(events) => (
            StatusCode::OK,
            Json(json!({
                "events": events,
                "ticket_id": ticket_id,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, ticket_id, "failed to list helpdesk ticket audit");
            internal_error()
        }
    }
}

async fn create_helpdesk_ticket_handler(
    State(state): State<AppState>,
    Json(payload): Json<HelpdeskTicketCreateRequestV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    match create_helpdesk_ticket_for_active_backend(&state, &payload).await {
        Ok(ticket) => (StatusCode::CREATED, Json(json!({ "ticket": ticket }))).into_response(),
        Err(err) => {
            error!(error = %err, client_id = payload.client_id, "failed to create helpdesk ticket");
            internal_error()
        }
    }
}

async fn assign_helpdesk_ticket_handler(
    State(state): State<AppState>,
    AxumPath(ticket_id): AxumPath<String>,
    Json(payload): Json<HelpdeskTicketAssignRequestV1>,
) -> impl IntoResponse {
    let ticket_id = ticket_id.trim().to_string();
    if ticket_id.is_empty() {
        return bad_request("ticket_id cannot be empty");
    }

    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    match assign_helpdesk_ticket_for_active_backend(
        &state,
        &ticket_id,
        payload.agent_id.as_deref(),
        payload.reason.as_deref(),
    )
    .await
    {
        Ok((ticket, agent)) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
                "agent": agent,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(
                error = %err,
                ticket_id,
                agent_id = payload.agent_id,
                "failed to assign helpdesk ticket"
            );
            bad_request(err.to_string())
        }
    }
}

async fn update_helpdesk_ticket_operational_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(ticket_id): AxumPath<String>,
    Json(payload): Json<HelpdeskTicketOperationalUpdateRequestV1>,
) -> impl IntoResponse {
    let ticket_id = ticket_id.trim().to_string();
    if ticket_id.is_empty() {
        return bad_request("ticket_id cannot be empty");
    }

    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    match authenticate_dashboard_from_headers(&state, &headers).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            let Some(agent_id) = payload
                .agent_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return unauthorized_message(
                    "agent_id is required when updating ticket operational fields from the agent app",
                );
            };

            if let Err(response) = require_helpdesk_agent_auth(&state, &headers, agent_id).await {
                return response;
            }

            if let Err(err) =
                ensure_helpdesk_agent_controls_ticket(&state, &ticket_id, agent_id).await
            {
                return forbidden(err.to_string());
            }
        }
        Err(err) => {
            error!(error = %err, "failed to validate dashboard session for operational update");
            return internal_error();
        }
    }

    match update_helpdesk_ticket_operational_for_active_backend(
        &state,
        &ticket_id,
        payload.difficulty.as_deref(),
        payload.estimated_minutes,
    )
    .await
    {
        Ok(ticket) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
            })),
        )
            .into_response(),
        Err(err) => {
            if err
                .to_string()
                .contains("can no longer be updated operationally")
            {
                return bad_request(err.to_string());
            }
            error!(
                error = %err,
                ticket_id,
                "failed to update helpdesk ticket operational fields"
            );
            internal_error()
        }
    }
}

async fn create_helpdesk_ticket_agent_report_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(ticket_id): AxumPath<String>,
    Json(payload): Json<HelpdeskTicketAgentReportCreateRequestV1>,
) -> impl IntoResponse {
    let ticket_id = ticket_id.trim().to_string();
    if ticket_id.is_empty() {
        return bad_request("ticket_id cannot be empty");
    }

    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    if let Err(response) = require_helpdesk_agent_auth(&state, &headers, &payload.agent_id).await {
        return response;
    }

    match add_helpdesk_ticket_agent_report_for_active_backend(
        &state,
        &ticket_id,
        &payload.agent_id,
        &payload.note,
    )
    .await
    {
        Ok(ticket) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(
                error = %err,
                ticket_id,
                agent_id = payload.agent_id,
                "failed to create helpdesk agent report"
            );
            bad_request(err.to_string())
        }
    }
}

async fn resolve_helpdesk_ticket_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(ticket_id): AxumPath<String>,
    Json(payload): Json<HelpdeskTicketResolveRequestV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    if let Err(response) = require_helpdesk_agent_auth(&state, &headers, &payload.agent_id).await {
        return response;
    }

    let next_agent_status = payload
        .next_agent_status
        .unwrap_or(HelpdeskAgentStatus::Available);

    match resolve_helpdesk_ticket_for_active_backend(
        &state,
        &ticket_id,
        &payload.agent_id,
        next_agent_status,
    )
    .await
    {
        Ok((ticket, agent)) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
                "agent": agent,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(
                error = %err,
                ticket_id,
                agent_id = payload.agent_id,
                "failed to resolve helpdesk ticket"
            );
            bad_request(err.to_string())
        }
    }
}

async fn requeue_helpdesk_ticket_handler(
    State(state): State<AppState>,
    AxumPath(ticket_id): AxumPath<String>,
    Json(payload): Json<HelpdeskTicketSupervisorActionRequestV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    let next_agent_status = payload
        .next_agent_status
        .unwrap_or(HelpdeskAgentStatus::Available);

    match requeue_helpdesk_ticket_for_active_backend(
        &state,
        &ticket_id,
        next_agent_status,
        payload.reason.as_deref(),
    )
    .await
    {
        Ok((ticket, agent)) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
                "agent": agent,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, ticket_id, "failed to requeue helpdesk ticket");
            bad_request(err.to_string())
        }
    }
}

async fn cancel_helpdesk_ticket_handler(
    State(state): State<AppState>,
    AxumPath(ticket_id): AxumPath<String>,
    Json(payload): Json<HelpdeskTicketSupervisorActionRequestV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = payload.validate() {
        return bad_request(validation_error.to_string());
    }

    let next_agent_status = payload
        .next_agent_status
        .unwrap_or(HelpdeskAgentStatus::Available);

    match cancel_helpdesk_ticket_for_active_backend(
        &state,
        &ticket_id,
        next_agent_status,
        payload.reason.as_deref(),
    )
    .await
    {
        Ok((ticket, agent)) => (
            StatusCode::OK,
            Json(json!({
                "ticket": ticket,
                "agent": agent,
            })),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, ticket_id, "failed to cancel helpdesk ticket");
            bad_request(err.to_string())
        }
    }
}

async fn auth_login_handler(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(payload): Json<AuthLoginRequestV1>,
) -> Response {
    let username = payload.username.trim();
    if username.is_empty() || payload.password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            jar,
            Json(json!({
                "error": "invalid_credentials"
            })),
        )
            .into_response();
    }

    let user_record = match get_dashboard_user_by_username(&state.pool, username).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                jar,
                Json(json!({
                    "error": "invalid_credentials"
                })),
            )
                .into_response();
        }
        Err(err) => {
            error!(error = %err, username, "failed to query dashboard user");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                jar,
                Json(json!({ "error": "internal_error" })),
            )
                .into_response();
        }
    };

    if !user_record.is_active
        || !auth::verify_password(&payload.password, &user_record.password_hash)
    {
        return (
            StatusCode::UNAUTHORIZED,
            jar,
            Json(json!({
                "error": "invalid_credentials"
            })),
        )
            .into_response();
    }

    let now = Utc::now();
    let response_user = AuthUserV1 {
        id: user_record.id,
        username: user_record.username,
        role: user_record.role,
    };
    let expires_at = now + ChronoDuration::minutes(session_ttl_minutes(&state.auth));
    let session_token =
        match auth::issue_dashboard_session_token(&state.auth, &response_user, expires_at) {
            Ok(token) => token,
            Err(err) => {
                error!(error = %err, username, "failed to issue dashboard session token");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    jar,
                    Json(json!({ "error": "internal_error" })),
                )
                    .into_response();
            }
        };

    let cookie = build_dashboard_session_cookie(&state.auth, session_token, expires_at);
    let jar = jar.add(cookie);

    let response = AuthLoginResponseV1 {
        user: response_user,
        expires_at,
    };

    (StatusCode::OK, jar, Json(response)).into_response()
}

async fn auth_logout_handler(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(existing) = jar.get(DASHBOARD_SESSION_COOKIE) {
        if let Err(err) = delete_dashboard_session(&state.pool, existing.value()).await {
            error!(error = %err, "failed to delete dashboard session on logout");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                jar,
                Json(json!({ "error": "internal_error" })),
            )
                .into_response();
        }
    }

    let removal = Cookie::build((DASHBOARD_SESSION_COOKIE, ""))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.auth.cookie_secure)
        .path("/")
        .build();
    let jar = jar.remove(removal);

    (
        StatusCode::OK,
        jar,
        Json(json!({
            "status": "ok"
        })),
    )
        .into_response()
}

async fn auth_me_handler(Extension(auth): Extension<AuthContext>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(AuthLoginResponseV1 {
            user: auth.user,
            expires_at: auth.expires_at,
        }),
    )
}

async fn dashboard_summary_handler(
    State(state): State<AppState>,
    Query(query): Query<SummaryQuery>,
) -> impl IntoResponse {
    let now = Utc::now();
    let from = match parse_optional_datetime(query.from.as_deref(), now - ChronoDuration::hours(24))
    {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };
    let to = match parse_optional_datetime(query.to.as_deref(), now) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };

    if from > to {
        return bad_request("from must be less than or equal to to");
    }

    match query_dashboard_summary_for_active_monitoring_backend(&state, from, to).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(err) => {
            error!(error = %err, "failed to query dashboard summary");
            internal_error()
        }
    }
}

async fn events_list_handler(
    State(state): State<AppState>,
    Query(query): Query<EventListQuery>,
) -> impl IntoResponse {
    let actor_type = match parse_optional_actor_type(query.actor_type.as_deref()) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };
    let event_type = match parse_optional_event_type(query.event_type.as_deref()) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };
    let from = match parse_optional_datetime_option(query.from.as_deref()) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };
    let to = match parse_optional_datetime_option(query.to.as_deref()) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };

    if let (Some(from), Some(to)) = (from, to) {
        if from > to {
            return bad_request("from must be less than or equal to to");
        }
    }

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);

    let filter = EventQueryFilter {
        session_id: query
            .session_id
            .map(|value| value.trim().to_string())
            .filter(|v| !v.is_empty()),
        user_id: query
            .user_id
            .map(|value| value.trim().to_string())
            .filter(|v| !v.is_empty()),
        actor_type,
        event_type,
        from,
        to,
    };

    match query_timeline_events_for_active_monitoring_backend(&state, &filter, page, page_size)
        .await
    {
        Ok((items, total)) => (
            StatusCode::OK,
            Json(PaginatedResponseV1 {
                items,
                page,
                page_size,
                total,
            }),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, "failed to list timeline events");
            internal_error()
        }
    }
}

async fn session_timeline_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<TimelineQuery>,
) -> impl IntoResponse {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return bad_request("session_id cannot be empty");
    }

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    let actor_type = match parse_optional_actor_type(query.actor_type.as_deref()) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };

    let filter = EventQueryFilter {
        session_id: Some(session_id.clone()),
        actor_type,
        ..Default::default()
    };

    match query_timeline_events_for_active_monitoring_backend(&state, &filter, page, page_size)
        .await
    {
        Ok((items, total)) => (
            StatusCode::OK,
            Json(PaginatedResponseV1 {
                items,
                page,
                page_size,
                total,
            }),
        )
            .into_response(),
        Err(err) => {
            error!(error = %err, session_id, "failed to query session timeline");
            internal_error()
        }
    }
}

async fn sessions_report_csv_handler(
    State(state): State<AppState>,
    Query(query): Query<CsvReportQuery>,
) -> impl IntoResponse {
    let now = Utc::now();
    let from = match parse_optional_datetime(query.from.as_deref(), now - ChronoDuration::hours(24))
    {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };
    let to = match parse_optional_datetime(query.to.as_deref(), now) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };
    if from > to {
        return bad_request("from must be less than or equal to to");
    }

    let user_id = query
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let actor_type = match parse_optional_actor_type(query.actor_type.as_deref()) {
        Ok(value) => value,
        Err(err) => return bad_request(err),
    };

    let rows = match query_session_report_rows_for_active_monitoring_backend(
        &state, from, to, user_id, actor_type,
    )
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            error!(error = %err, "failed to query session report rows");
            return internal_error();
        }
    };

    let mut csv = String::from("session_id,started_at,last_event_at,events_total,users\n");
    for row in rows {
        csv.push_str(&csv_escape(&row.session_id));
        csv.push(',');
        csv.push_str(&row.started_at.to_rfc3339());
        csv.push(',');
        csv.push_str(&row.last_event_at.to_rfc3339());
        csv.push(',');
        csv.push_str(&row.events_total.to_string());
        csv.push(',');
        csv.push_str(&csv_escape(&row.users.join("|")));
        csv.push('\n');
    }

    let filename = format!(
        "sessions-report-{}-{}.csv",
        from.format("%Y%m%d%H%M%S"),
        to.format("%Y%m%d%H%M%S")
    );

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                    .unwrap_or_else(|_| {
                        HeaderValue::from_static("attachment; filename=\"sessions-report.csv\"")
                    }),
            ),
        ],
        csv,
    )
        .into_response()
}

async fn require_dashboard_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(cookie_header) = request.headers().get(header::COOKIE) else {
        return unauthorized();
    };
    let Ok(cookie_header) = cookie_header.to_str() else {
        return unauthorized();
    };
    let Some(session_token) = find_cookie_value(cookie_header, DASHBOARD_SESSION_COOKIE) else {
        return unauthorized();
    };

    let now = Utc::now();
    let authenticated_user = if let Some(signed_session) =
        auth::verify_dashboard_session_token(&state.auth, &session_token, now)
    {
        signed_session.user
    } else {
        match get_dashboard_session_by_token(&state.pool, &session_token, now).await {
            Ok(Some(record)) => record.user,
            Ok(None) => return unauthorized(),
            Err(err) => {
                error!(error = %err, "failed to validate dashboard session");
                return internal_error();
            }
        }
    };

    let refreshed_expires_at = now + ChronoDuration::minutes(session_ttl_minutes(&state.auth));
    let refreshed_token = match auth::issue_dashboard_session_token(
        &state.auth,
        &authenticated_user,
        refreshed_expires_at,
    ) {
        Ok(token) => token,
        Err(err) => {
            error!(error = %err, "failed to refresh dashboard session token");
            return internal_error();
        }
    };

    request.extensions_mut().insert(AuthContext {
        user: authenticated_user,
        expires_at: refreshed_expires_at,
    });

    let response = next.run(request).await;
    let jar = CookieJar::new().add(build_dashboard_session_cookie(
        &state.auth,
        refreshed_token,
        refreshed_expires_at,
    ));
    (jar, response).into_response()
}

async fn authenticate_dashboard_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> anyhow::Result<Option<AuthUserV1>> {
    let Some(cookie_header) = headers.get(header::COOKIE) else {
        return Ok(None);
    };
    let Ok(cookie_header) = cookie_header.to_str() else {
        return Ok(None);
    };
    let Some(session_token) = find_cookie_value(cookie_header, DASHBOARD_SESSION_COOKIE) else {
        return Ok(None);
    };

    let now = Utc::now();
    if let Some(signed_session) =
        auth::verify_dashboard_session_token(&state.auth, &session_token, now)
    {
        return Ok(Some(signed_session.user));
    }

    match get_dashboard_session_by_token(&state.pool, &session_token, now).await {
        Ok(Some(record)) => Ok(Some(record.user)),
        Ok(None) => Ok(None),
        Err(err) => Err(err).context("failed to validate dashboard session from request headers"),
    }
}

fn extract_helpdesk_agent_token(headers: &HeaderMap) -> Option<String> {
    if let Some(raw_value) = headers
        .get(HELPDESK_AGENT_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(raw_value.to_string());
    }

    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

async fn require_helpdesk_agent_auth(
    state: &AppState,
    headers: &HeaderMap,
    agent_id: &str,
) -> Result<(), Response> {
    let Some(raw_token) = extract_helpdesk_agent_token(headers) else {
        return Err(unauthorized_message(
            "helpdesk agent token is required for this action",
        ));
    };

    match verify_helpdesk_agent_token_for_active_backend(state, agent_id, &raw_token).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(forbidden(
            "invalid helpdesk agent token for this RustDesk ID",
        )),
        Err(err) => {
            error!(
                error = %err,
                agent_id,
                "failed to verify helpdesk agent token"
            );
            Err(internal_error())
        }
    }
}

async fn ensure_helpdesk_agent_controls_ticket(
    state: &AppState,
    ticket_id: &str,
    agent_id: &str,
) -> anyhow::Result<()> {
    let ticket = get_helpdesk_ticket_for_active_backend(state, ticket_id)
        .await?
        .with_context(|| format!("ticket '{}' was not found", ticket_id))?;

    if ticket.assigned_agent_id.as_deref() != Some(agent_id) {
        anyhow::bail!("ticket is not currently assigned to this agent");
    }

    if !matches!(
        ticket.status,
        crate::model::HelpdeskTicketStatus::Opening
            | crate::model::HelpdeskTicketStatus::InProgress
    ) {
        anyhow::bail!("ticket is not active for this agent");
    }

    Ok(())
}

async fn list_presence_sessions_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_active_session_presence_for_active_monitoring_backend(&state).await {
        Ok(sessions) => (
            StatusCode::OK,
            Json(json!({
                "sessions": sessions,
            })),
        ),
        Err(err) => {
            error!(error = %err, "failed to list active presence sessions");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error" })),
            )
        }
    }
}

async fn get_session_presence_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> impl IntoResponse {
    match get_session_presence_for_active_monitoring_backend(&state, &session_id).await {
        Ok(Some(snapshot)) => (
            StatusCode::OK,
            Json(json!({
                "presence": snapshot
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "presence_not_found",
                "session_id": session_id,
            })),
        ),
        Err(err) => {
            error!(error = %err, session_id, "failed to query session presence");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "internal_error"
                })),
            )
        }
    }
}

async fn stream_session_presence_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> impl IntoResponse {
    let receiver = state.presence_updates.subscribe();

    let stream = futures::stream::unfold(
        (receiver, state, session_id, true),
        |(mut receiver, state, session_id, mut emit_initial)| async move {
            loop {
                if emit_initial {
                    emit_initial = false;
                    let event = presence_snapshot_sse_event(&state, &session_id).await;
                    return Some((
                        Ok::<Event, Infallible>(event),
                        (receiver, state, session_id, emit_initial),
                    ));
                }

                match receiver.recv().await {
                    Ok(changed_session_id) => {
                        if changed_session_id != session_id {
                            continue;
                        }
                        let event = presence_snapshot_sse_event(&state, &session_id).await;
                        return Some((
                            Ok::<Event, Infallible>(event),
                            (receiver, state, session_id, emit_initial),
                        ));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        let event = Event::default().event("presence_lagged").data(
                            json!({ "session_id": session_id, "skipped_updates": skipped })
                                .to_string(),
                        );
                        return Some((
                            Ok::<Event, Infallible>(event),
                            (receiver, state, session_id, emit_initial),
                        ));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keepalive"),
    )
}

async fn ingest_session_event(
    State(state): State<AppState>,
    Json(event): Json<SessionEventV1>,
) -> impl IntoResponse {
    if let Err(validation_error) = event.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid_event",
                "message": validation_error.to_string()
            })),
        );
    }

    match should_store_session_event_for_active_helpdesk_backend(&state, &event).await {
        Ok(false) => {
            return (
                StatusCode::ACCEPTED,
                Json(json!({
                    "status": "ignored",
                    "event_id": event.event_id,
                    "reason": "event_filtered_by_monitoring_policy",
                })),
            );
        }
        Ok(true) => {}
        Err(err) => {
            error!(error = %err, "failed to apply monitoring ingest policy");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "internal_error"
                })),
            );
        }
    }

    match insert_session_event_for_active_monitoring_backend(&state, &event).await {
        Ok(InsertOutcome::Inserted) => {
            state.metrics.inc_events_received();
            if event.event_type.affects_presence() {
                let _ = state.presence_updates.send(event.session_id.clone());
            }
            (
                StatusCode::ACCEPTED,
                Json(json!({
                    "status": "accepted",
                    "event_id": event.event_id,
                })),
            )
        }
        Ok(InsertOutcome::Duplicate) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_event_id",
                "event_id": event.event_id,
            })),
        ),
        Err(err) => {
            error!(error = %err, "failed to ingest event");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "internal_error"
                })),
            )
        }
    }
}

async fn presence_snapshot_sse_event(state: &AppState, session_id: &str) -> Event {
    match get_session_presence_for_active_monitoring_backend(state, session_id).await {
        Ok(Some(snapshot)) => Event::default().event("presence_snapshot").data(
            json!({
                "session_id": session_id,
                "presence": snapshot,
            })
            .to_string(),
        ),
        Ok(None) => Event::default().event("presence_missing").data(
            json!({
                "session_id": session_id,
                "presence": null,
            })
            .to_string(),
        ),
        Err(err) => {
            error!(error = %err, session_id, "failed to build presence snapshot for stream");
            Event::default().event("presence_error").data(
                json!({
                    "session_id": session_id,
                    "error": "internal_error",
                })
                .to_string(),
            )
        }
    }
}

fn find_cookie_value(cookie_header: &str, cookie_name: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let mut pair = part.trim().splitn(2, '=');
        let name = pair.next()?.trim();
        let value = pair.next().unwrap_or("").trim();
        if name == cookie_name && !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn session_ttl_minutes(auth: &AuthSettings) -> i64 {
    i64::try_from(auth.session_ttl_minutes)
        .unwrap_or(480)
        .max(1)
}

fn build_dashboard_session_cookie(
    auth: &AuthSettings,
    session_token: String,
    _expires_at: DateTime<Utc>,
) -> Cookie<'static> {
    Cookie::build((DASHBOARD_SESSION_COOKIE, session_token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(auth.cookie_secure)
        .path("/")
        .build()
}

fn parse_optional_datetime(
    raw: Option<&str>,
    default: DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    match raw {
        Some(value) if !value.trim().is_empty() => parse_datetime(value),
        _ => Ok(default),
    }
}

fn parse_optional_datetime_option(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    match raw {
        Some(value) if !value.trim().is_empty() => parse_datetime(value).map(Some),
        _ => Ok(None),
    }
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| format!("invalid RFC3339 datetime: {value}"))
}

fn parse_optional_event_type(raw: Option<&str>) -> Result<Option<SessionEventType>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let parsed = match raw {
        "session_started" => SessionEventType::SessionStarted,
        "session_ended" => SessionEventType::SessionEnded,
        "recording_started" => SessionEventType::RecordingStarted,
        "recording_stopped" => SessionEventType::RecordingStopped,
        "participant_joined" => SessionEventType::ParticipantJoined,
        "participant_left" => SessionEventType::ParticipantLeft,
        "control_changed" => SessionEventType::ControlChanged,
        "participant_activity" => SessionEventType::ParticipantActivity,
        _ => {
            return Err(format!(
                "invalid event_type '{raw}'. allowed: session_started, session_ended, recording_started, recording_stopped, participant_joined, participant_left, control_changed, participant_activity"
            ))
        }
    };
    Ok(Some(parsed))
}

fn parse_optional_actor_type(raw: Option<&str>) -> Result<Option<SessionActorTypeV1>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let parsed = match raw {
        "agent" => SessionActorTypeV1::Agent,
        "client" => SessionActorTypeV1::Client,
        "unknown" => SessionActorTypeV1::Unknown,
        _ => {
            return Err(format!(
                "invalid actor_type '{raw}'. allowed: agent, client, unknown"
            ))
        }
    };
    Ok(Some(parsed))
}

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": "bad_request",
            "message": message.into(),
        })),
    )
        .into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "unauthorized"
        })),
    )
        .into_response()
}

fn unauthorized_message(message: impl Into<String>) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "unauthorized",
            "message": message.into(),
        })),
    )
        .into_response()
}

fn forbidden(message: impl Into<String>) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "forbidden",
            "message": message.into(),
        })),
    )
        .into_response()
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "internal_error"
        })),
    )
        .into_response()
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

async fn webhook_worker(state: AppState) {
    let poll_interval = Duration::from_millis(state.config.worker.poll_interval_ms);
    let stuck_processing_threshold_ms = state
        .config
        .webhook
        .timeout_ms
        .saturating_mul(4)
        .max(30_000);

    loop {
        let now_ms = unix_millis_now();

        let reset_result = if let Some(pool) = active_monitoring_postgres_pool(&state) {
            reset_stuck_processing_pg(pool, stuck_processing_threshold_ms, now_ms).await
        } else {
            reset_stuck_processing(&state.pool, stuck_processing_threshold_ms, now_ms).await
        };

        match reset_result {
            Ok(reset_count) if reset_count > 0 => {
                warn!(reset_count, "reset stale processing rows");
            }
            Ok(_) => {}
            Err(err) => {
                error!(error = %err, "failed to reset stale processing rows");
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        }

        if state.circuit_breaker.is_open(now_ms) {
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        let batch_limit = state.config.worker.concurrency.saturating_mul(2);

        let due_events_result = if let Some(pool) = active_monitoring_postgres_pool(&state) {
            claim_due_events_pg(pool, batch_limit, now_ms).await
        } else {
            claim_due_events(&state.pool, batch_limit, now_ms).await
        };

        let due_events = match due_events_result {
            Ok(rows) => rows,
            Err(err) => {
                error!(error = %err, "failed to claim due events");
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        if due_events.is_empty() {
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        futures::stream::iter(due_events)
            .for_each_concurrent(state.config.worker.concurrency, |record| {
                let worker_state = state.clone();
                async move {
                    if let Err(err) = process_outbox_record(worker_state, record).await {
                        error!(error = %err, "error while processing outbox record");
                    }
                }
            })
            .await;
    }
}

async fn sqlite_fallback_replay_worker(state: AppState) {
    if active_monitoring_postgres_pool(&state).is_none() || !state.sqlite_fallback_enabled {
        return;
    }

    let poll_interval = Duration::from_millis(state.config.worker.poll_interval_ms);
    let stuck_processing_threshold_ms = state
        .config
        .webhook
        .timeout_ms
        .saturating_mul(4)
        .max(30_000);

    info!("SQLite fallback replay worker enabled for Postgres monitoring backend");

    loop {
        let now_ms = unix_millis_now();

        match reset_stuck_processing(&state.pool, stuck_processing_threshold_ms, now_ms).await {
            Ok(reset_count) if reset_count > 0 => {
                warn!(reset_count, "reset stale SQLite fallback processing rows");
            }
            Ok(_) => {}
            Err(err) => {
                error!(
                    error = %err,
                    "failed to reset stale SQLite fallback processing rows"
                );
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        }

        let batch_limit = state.config.worker.concurrency.saturating_mul(2);
        let due_events = match claim_due_events(&state.pool, batch_limit, now_ms).await {
            Ok(rows) => rows,
            Err(err) => {
                error!(error = %err, "failed to claim SQLite fallback events");
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        if due_events.is_empty() {
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        futures::stream::iter(due_events)
            .for_each_concurrent(state.config.worker.concurrency, |record| {
                let worker_state = state.clone();
                async move {
                    let event_id = record.event_id.clone();
                    if let Err(err) = process_sqlite_fallback_record(worker_state, record).await {
                        error!(
                            error = %err,
                            event_id = %event_id,
                            "error while replaying SQLite fallback event into Postgres"
                        );
                    }
                }
            })
            .await;
    }
}

async fn process_outbox_record(state: AppState, record: OutboxRecord) -> anyhow::Result<()> {
    let event: SessionEventV1 = match serde_json::from_str(&record.payload) {
        Ok(payload) => payload,
        Err(err) => {
            let attempts = record.attempts.saturating_add(1);
            let now_ms = unix_millis_now();
            if let Some(pool) = active_monitoring_postgres_pool(&state) {
                mark_failed_pg(
                    pool,
                    &record.event_id,
                    attempts,
                    &format!("invalid JSON payload in outbox: {err}"),
                    now_ms,
                )
                .await?;
            } else {
                mark_failed(
                    &state.pool,
                    &record.event_id,
                    attempts,
                    &format!("invalid JSON payload in outbox: {err}"),
                    now_ms,
                )
                .await?;
            }
            state.metrics.inc_webhook_failed();
            return Ok(());
        }
    };

    let current_attempt = record.attempts.saturating_add(1);
    let now_ms = unix_millis_now();

    match state.dispatcher.send_event(&event).await {
        Ok(elapsed) => {
            state.circuit_breaker.on_success();
            if let Some(pool) = active_monitoring_postgres_pool(&state) {
                mark_delivered_pg(pool, &record.event_id, current_attempt, now_ms).await?;
            } else {
                mark_delivered(&state.pool, &record.event_id, current_attempt, now_ms).await?;
            }
            state
                .metrics
                .inc_webhook_sent(elapsed.as_millis().min(u128::from(u64::MAX)) as u64);
        }
        Err(err) => {
            let error_message = truncate_error(&err.to_string());
            state.circuit_breaker.on_failure(now_ms);
            state.metrics.inc_webhook_failed();

            if current_attempt >= state.config.webhook.retry.max_attempts {
                if let Some(pool) = active_monitoring_postgres_pool(&state) {
                    mark_failed_pg(
                        pool,
                        &record.event_id,
                        current_attempt,
                        &error_message,
                        now_ms,
                    )
                    .await?;
                } else {
                    mark_failed(
                        &state.pool,
                        &record.event_id,
                        current_attempt,
                        &error_message,
                        now_ms,
                    )
                    .await?;
                }
                warn!(
                    event_id = %record.event_id,
                    attempts = current_attempt,
                    error = %error_message,
                    "webhook delivery permanently failed"
                );
            } else {
                let base = state.config.webhook.retry.backoff_ms;
                let exponent = current_attempt.saturating_sub(1).min(16);
                let backoff_ms = base.saturating_mul(1u64 << exponent);
                let next_attempt_at = now_ms.saturating_add(backoff_ms);

                if let Some(pool) = active_monitoring_postgres_pool(&state) {
                    schedule_retry_pg(
                        pool,
                        &record.event_id,
                        current_attempt,
                        next_attempt_at,
                        &error_message,
                        now_ms,
                    )
                    .await?;
                } else {
                    schedule_retry(
                        &state.pool,
                        &record.event_id,
                        current_attempt,
                        next_attempt_at,
                        &error_message,
                        now_ms,
                    )
                    .await?;
                }
                state.metrics.inc_webhook_retry();
                warn!(
                    event_id = %record.event_id,
                    attempts = current_attempt,
                    next_attempt_at,
                    error = %error_message,
                    "scheduled webhook retry"
                );
            }
        }
    }

    Ok(())
}

async fn process_sqlite_fallback_record(
    state: AppState,
    record: OutboxRecord,
) -> anyhow::Result<()> {
    let event: SessionEventV1 = match serde_json::from_str(&record.payload) {
        Ok(payload) => payload,
        Err(err) => {
            let attempts = record.attempts.saturating_add(1);
            let now_ms = unix_millis_now();
            mark_failed(
                &state.pool,
                &record.event_id,
                attempts,
                &format!("invalid JSON payload in SQLite fallback outbox: {err}"),
                now_ms,
            )
            .await?;
            return Ok(());
        }
    };

    let Some(pool) = active_monitoring_postgres_pool(&state) else {
        let now_ms = unix_millis_now();
        let current_attempt = record.attempts.saturating_add(1);
        let next_attempt_at =
            now_ms.saturating_add(state.config.webhook.retry.backoff_ms.max(1_000));
        schedule_retry(
            &state.pool,
            &record.event_id,
            current_attempt,
            next_attempt_at,
            "Postgres monitoring backend unavailable during SQLite fallback replay",
            now_ms,
        )
        .await?;
        return Ok(());
    };

    let current_attempt = record.attempts.saturating_add(1);
    let now_ms = unix_millis_now();

    match insert_event_pg(pool, &event).await {
        Ok(InsertOutcome::Inserted) | Ok(InsertOutcome::Duplicate) => {
            delete_outbox_event(&state.pool, &record.event_id).await?;
            if event.event_type.affects_presence() {
                let _ = state.presence_updates.send(event.session_id.clone());
            }
            debug!(
                event_id = %record.event_id,
                "replayed SQLite fallback event into Postgres"
            );
        }
        Err(err) => {
            let error_message = truncate_error(&err.to_string());
            if current_attempt >= state.config.webhook.retry.max_attempts {
                mark_failed(
                    &state.pool,
                    &record.event_id,
                    current_attempt,
                    &error_message,
                    now_ms,
                )
                .await?;
                warn!(
                    event_id = %record.event_id,
                    attempts = current_attempt,
                    error = %error_message,
                    "SQLite fallback replay permanently failed"
                );
            } else {
                let base = state.config.webhook.retry.backoff_ms;
                let exponent = current_attempt.saturating_sub(1).min(16);
                let backoff_ms = base.saturating_mul(1u64 << exponent);
                let next_attempt_at = now_ms.saturating_add(backoff_ms);
                schedule_retry(
                    &state.pool,
                    &record.event_id,
                    current_attempt,
                    next_attempt_at,
                    &error_message,
                    now_ms,
                )
                .await?;
                warn!(
                    event_id = %record.event_id,
                    attempts = current_attempt,
                    next_attempt_at,
                    error = %error_message,
                    "scheduled SQLite fallback replay retry"
                );
            }
        }
    }

    Ok(())
}

async fn cleanup_worker(state: AppState) {
    let interval = Duration::from_secs(
        state
            .config
            .retention
            .cleanup_interval_minutes
            .saturating_mul(60),
    );

    loop {
        tokio::time::sleep(interval).await;

        let now_ms = unix_millis_now();
        let cleanup_sqlite_fallback =
            active_monitoring_postgres_pool(&state).is_some() && state.sqlite_fallback_enabled;
        let failed_retention_ms = state
            .config
            .retention
            .failed_retention_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000);
        let failed_cutoff_ms = now_ms.saturating_sub(failed_retention_ms);

        let cleanup_failed_result = if let Some(pool) = active_monitoring_postgres_pool(&state) {
            cleanup_failed_older_than_pg(pool, failed_cutoff_ms).await
        } else {
            cleanup_failed_older_than(&state.pool, failed_cutoff_ms).await
        };

        match cleanup_failed_result {
            Ok(deleted) if deleted > 0 => info!(deleted, "cleaned old failed outbox events"),
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup old failed outbox events"),
        }

        if cleanup_sqlite_fallback {
            match cleanup_failed_older_than(&state.pool, failed_cutoff_ms).await {
                Ok(deleted) if deleted > 0 => {
                    info!(deleted, "cleaned old failed SQLite fallback outbox events")
                }
                Ok(_) => {}
                Err(err) => error!(
                    error = %err,
                    "failed to cleanup old failed SQLite fallback outbox events"
                ),
            }
        }

        let delivered_retention_ms = state
            .config
            .monitoring
            .local_delivered_outbox_retention_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000);
        let delivered_cutoff_ms = now_ms.saturating_sub(delivered_retention_ms);

        let cleanup_delivered_result = if let Some(pool) = active_monitoring_postgres_pool(&state) {
            cleanup_delivered_older_than_pg(pool, delivered_cutoff_ms).await
        } else {
            cleanup_delivered_older_than(&state.pool, delivered_cutoff_ms).await
        };

        match cleanup_delivered_result {
            Ok(deleted) if deleted > 0 => {
                info!(deleted, "cleaned old delivered outbox events")
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup old delivered outbox events"),
        }

        if cleanup_sqlite_fallback {
            match cleanup_delivered_older_than(&state.pool, delivered_cutoff_ms).await {
                Ok(deleted) if deleted > 0 => {
                    info!(
                        deleted,
                        "cleaned old delivered SQLite fallback outbox events"
                    )
                }
                Ok(_) => {}
                Err(err) => error!(
                    error = %err,
                    "failed to cleanup old delivered SQLite fallback outbox events"
                ),
            }
        }

        let session_event_retention_ms = state
            .config
            .monitoring
            .local_session_event_retention_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000);
        let session_event_cutoff_ms = now_ms.saturating_sub(session_event_retention_ms);

        let cleanup_session_events_result =
            if let Some(pool) = active_monitoring_postgres_pool(&state) {
                cleanup_session_events_older_than_pg(pool, session_event_cutoff_ms).await
            } else {
                cleanup_session_events_older_than(&state.pool, session_event_cutoff_ms).await
            };

        match cleanup_session_events_result {
            Ok(deleted) if deleted > 0 => info!(deleted, "cleaned old session events"),
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup old session events"),
        }

        if cleanup_sqlite_fallback {
            match cleanup_session_events_older_than(&state.pool, session_event_cutoff_ms).await {
                Ok(deleted) if deleted > 0 => {
                    info!(deleted, "cleaned old SQLite fallback session events")
                }
                Ok(_) => {}
                Err(err) => error!(
                    error = %err,
                    "failed to cleanup old SQLite fallback session events"
                ),
            }
        }

        let session_presence_retention_ms = state
            .config
            .monitoring
            .local_session_presence_retention_hours
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000);
        let session_presence_cutoff_ms = now_ms.saturating_sub(session_presence_retention_ms);

        let cleanup_presence_result = if let Some(pool) = active_monitoring_postgres_pool(&state) {
            cleanup_inactive_session_presence_older_than_pg(pool, session_presence_cutoff_ms).await
        } else {
            cleanup_inactive_session_presence_older_than(&state.pool, session_presence_cutoff_ms)
                .await
        };

        match cleanup_presence_result {
            Ok(deleted) if deleted > 0 => {
                info!(deleted, "cleaned stale inactive session presence rows")
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup session presence rows"),
        }

        if cleanup_sqlite_fallback {
            match cleanup_inactive_session_presence_older_than(
                &state.pool,
                session_presence_cutoff_ms,
            )
            .await
            {
                Ok(deleted) if deleted > 0 => {
                    info!(
                        deleted,
                        "cleaned stale inactive SQLite fallback session presence rows"
                    )
                }
                Ok(_) => {}
                Err(err) => error!(
                    error = %err,
                    "failed to cleanup SQLite fallback session presence rows"
                ),
            }
        }

        let heartbeat_retention_ms = state
            .config
            .monitoring
            .local_agent_heartbeat_retention_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000);
        let heartbeat_cutoff_ms = now_ms.saturating_sub(heartbeat_retention_ms);

        match cleanup_helpdesk_agent_heartbeats_older_than(&state.pool, heartbeat_cutoff_ms).await {
            Ok(deleted) if deleted > 0 => {
                info!(deleted, "cleaned stale helpdesk agent heartbeats")
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup helpdesk agent heartbeats"),
        }

        match cleanup_expired_dashboard_sessions(&state.pool, Utc::now()).await {
            Ok(deleted) if deleted > 0 => info!(deleted, "cleaned expired dashboard sessions"),
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup expired dashboard sessions"),
        }
    }
}

async fn presence_cleanup_worker(state: AppState) {
    let interval = Duration::from_secs(state.config.presence.cleanup_interval_seconds);
    let stale_after_ms = state
        .config
        .presence
        .stale_after_seconds
        .saturating_mul(1_000);

    loop {
        tokio::time::sleep(interval).await;

        let now_ms = unix_millis_now();
        let stale_before_ms = now_ms.saturating_sub(stale_after_ms) as i64;
        let cleanup_sqlite_fallback =
            active_monitoring_postgres_pool(&state).is_some() && state.sqlite_fallback_enabled;

        let expire_result = if let Some(pool) = active_monitoring_postgres_pool(&state) {
            expire_stale_presence_pg(pool, stale_before_ms, now_ms as i64).await
        } else {
            expire_stale_presence(&state.pool, stale_before_ms, now_ms as i64).await
        };

        match expire_result {
            Ok((expired_rows, touched_sessions)) if expired_rows > 0 => {
                info!(
                    expired_rows,
                    sessions = touched_sessions.len(),
                    stale_after_seconds = state.config.presence.stale_after_seconds,
                    "expired stale session presence rows"
                );

                for session_id in touched_sessions {
                    let _ = state.presence_updates.send(session_id);
                }
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup stale session presence rows"),
        }

        if cleanup_sqlite_fallback {
            match expire_stale_presence(&state.pool, stale_before_ms, now_ms as i64).await {
                Ok((expired_rows, touched_sessions)) if expired_rows > 0 => {
                    info!(
                        expired_rows,
                        sessions = touched_sessions.len(),
                        stale_after_seconds = state.config.presence.stale_after_seconds,
                        "expired stale SQLite fallback session presence rows"
                    );

                    for session_id in touched_sessions {
                        let _ = state.presence_updates.send(session_id);
                    }
                }
                Ok(_) => {}
                Err(err) => error!(
                    error = %err,
                    "failed to cleanup stale SQLite fallback session presence rows"
                ),
            }
        }
    }
}

async fn helpdesk_reconcile_worker(state: AppState) {
    let interval = Duration::from_millis(HELPDESK_RECONCILE_INTERVAL_MS);

    loop {
        tokio::time::sleep(interval).await;

        let reconcile_result = reconcile_helpdesk_runtime_for_active_backend(
            &state,
            HELPDESK_AGENT_STALE_AFTER_MS,
            unix_millis_now() as i64,
        )
        .await;

        match reconcile_result {
            Ok(stats)
                if stats.opening_timeouts > 0
                    || stats.agents_marked_offline > 0
                    || stats.tickets_requeued > 0
                    || stats.tickets_failed > 0 =>
            {
                info!(
                    opening_timeouts = stats.opening_timeouts,
                    agents_marked_offline = stats.agents_marked_offline,
                    tickets_requeued = stats.tickets_requeued,
                    tickets_failed = stats.tickets_failed,
                    "helpdesk runtime reconciliation applied"
                );
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to reconcile helpdesk runtime"),
        }
    }
}

async fn turso_sync_worker(state: AppState) {
    let Some(sync_cfg) = state.helpdesk_turso.clone() else {
        return;
    };

    let interval = Duration::from_millis(sync_cfg.interval_ms);
    let mut last_helpdesk_signature = compute_helpdesk_sync_signature(&state.pool, &sync_cfg)
        .await
        .ok();
    let mut last_monitoring_signature = compute_monitoring_sync_signature(&state.pool, &sync_cfg)
        .await
        .ok();

    loop {
        tokio::time::sleep(interval).await;

        match compute_helpdesk_sync_signature(&state.pool, &sync_cfg).await {
            Ok(signature) if last_helpdesk_signature.as_ref() != Some(&signature) => {
                match sync_helpdesk_snapshot_to_turso(&state.pool, &sync_cfg).await {
                    Ok(counts) => {
                        debug!(
                            rows = counts.total_rows(),
                            tickets = counts.tickets,
                            agents = counts.agents,
                            authorized_agents = counts.authorized_agents,
                            "helpdesk snapshot synced to Turso"
                        );
                        last_helpdesk_signature = Some(signature);
                    }
                    Err(err) => error!(error = %err, "failed to sync helpdesk snapshot to Turso"),
                }
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to compute helpdesk Turso sync signature"),
        }

        if active_monitoring_postgres_pool(&state).is_some() {
            continue;
        }

        match compute_monitoring_sync_signature(&state.pool, &sync_cfg).await {
            Ok(signature) if last_monitoring_signature.as_ref() != Some(&signature) => {
                match sync_monitoring_snapshot_to_turso(&state.pool, &sync_cfg).await {
                    Ok(counts) => {
                        debug!(
                            rows = counts.total_rows(),
                            session_events = counts.session_events,
                            session_presence = counts.session_presence,
                            outbox_events = counts.outbox_events,
                            "monitoring snapshot synced to Turso"
                        );
                        last_monitoring_signature = Some(signature);
                    }
                    Err(err) => {
                        error!(error = %err, "failed to sync monitoring snapshot to Turso")
                    }
                }
            }
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to compute monitoring Turso sync signature"),
        }
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to register shutdown signal");
    }
}

fn truncate_error(error_message: &str) -> String {
    const MAX_LEN: usize = 500;
    if error_message.len() <= MAX_LEN {
        error_message.to_string()
    } else {
        format!("{}...", &error_message[..MAX_LEN])
    }
}

#[derive(Debug)]
struct CircuitBreaker {
    threshold: u32,
    cooldown_ms: u64,
    state: Mutex<CircuitBreakerState>,
}

#[derive(Debug, Default)]
struct CircuitBreakerState {
    consecutive_failures: u32,
    open_until_ms: Option<u64>,
}

impl CircuitBreaker {
    fn new(threshold: u32, cooldown_ms: u64) -> Self {
        Self {
            threshold,
            cooldown_ms,
            state: Mutex::new(CircuitBreakerState::default()),
        }
    }

    fn is_open(&self, now_ms: u64) -> bool {
        let mut guard = self.state.lock().expect("circuit breaker poisoned");
        if let Some(open_until_ms) = guard.open_until_ms {
            if now_ms < open_until_ms {
                return true;
            }
            guard.open_until_ms = None;
            guard.consecutive_failures = 0;
        }
        false
    }

    fn on_success(&self) {
        let mut guard = self.state.lock().expect("circuit breaker poisoned");
        guard.consecutive_failures = 0;
        guard.open_until_ms = None;
    }

    fn on_failure(&self, now_ms: u64) {
        let mut guard = self.state.lock().expect("circuit breaker poisoned");
        guard.consecutive_failures = guard.consecutive_failures.saturating_add(1);

        if guard.consecutive_failures >= self.threshold {
            let open_until = now_ms.saturating_add(self.cooldown_ms);
            guard.open_until_ms = Some(open_until);
            guard.consecutive_failures = 0;
            warn!(
                open_until,
                "circuit breaker opened after repeated webhook failures"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CircuitBreaker;

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let breaker = CircuitBreaker::new(2, 1_000);
        assert!(!breaker.is_open(0));
        breaker.on_failure(100);
        assert!(!breaker.is_open(100));
        breaker.on_failure(200);
        assert!(breaker.is_open(300));
    }
}
