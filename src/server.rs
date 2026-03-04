use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::response::sse::{Event, KeepAlive, Sse};
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
use tracing::{error, info, warn};

use crate::auth::{self, AuthSettings, DASHBOARD_SESSION_COOKIE};
use crate::config::ServerConfig;
use crate::metrics::Metrics;
use crate::model::{
    AuthLoginRequestV1, AuthLoginResponseV1, AuthRoleV1, AuthUserV1, PaginatedResponseV1,
    SessionEventType, SessionEventV1,
};
use crate::storage::{
    claim_due_events, cleanup_expired_dashboard_sessions, cleanup_failed_older_than, connect_sqlite,
    create_dashboard_session, delete_dashboard_session, get_dashboard_session_by_token, get_dashboard_summary,
    get_dashboard_user_by_username, get_session_presence, insert_event, list_active_session_presence,
    mark_delivered, mark_failed, query_session_report_rows, query_session_timeline, query_timeline_events,
    reset_stuck_processing, schedule_retry, unix_millis_now, upsert_dashboard_user, EventQueryFilter,
    InsertOutcome, OutboxRecord, expire_stale_presence,
};
use crate::webhook::WebhookDispatcher;

#[derive(Clone)]
struct AppState {
    pool: sqlx::SqlitePool,
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

#[derive(Debug, Clone, Deserialize)]
struct SummaryQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct EventListQuery {
    session_id: Option<String>,
    user_id: Option<String>,
    event_type: Option<String>,
    from: Option<String>,
    to: Option<String>,
    page: Option<u64>,
    page_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TimelineQuery {
    page: Option<u64>,
    page_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct CsvReportQuery {
    from: Option<String>,
    to: Option<String>,
    user_id: Option<String>,
}

pub async fn run(bind_addr: &str, database_path: &Path, config: ServerConfig) -> anyhow::Result<()> {
    validate_server_config(&config)?;
    info!(
        stale_after_seconds = config.presence.stale_after_seconds,
        cleanup_interval_seconds = config.presence.cleanup_interval_seconds,
        "presence cleanup configuration"
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

    let circuit_breaker = Arc::new(CircuitBreaker::new(
        config.worker.circuit_breaker_threshold,
        config.worker.circuit_breaker_cooldown_ms,
    ));
    let (presence_updates, _) = broadcast::channel(1024);

    let state = AppState {
        pool,
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
        .route(
            "/api/v1/sessions/:session_id/timeline",
            get(session_timeline_handler),
        )
        .route("/api/v1/reports/sessions.csv", get(sessions_report_csv_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_dashboard_auth,
        ));

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/api/v1/auth/login", post(auth_login_handler))
        .route("/api/v1/auth/logout", post(auth_logout_handler))
        .route("/api/v1/sessions/presence", get(list_presence_sessions_handler))
        .route(
            "/api/v1/sessions/:session_id/presence",
            get(get_session_presence_handler),
        )
        .route(
            "/api/v1/sessions/:session_id/presence/stream",
            get(stream_session_presence_handler),
        )
        .route("/api/v1/session-events", post(ingest_session_event))
        .merge(protected_routes)
        .with_state(state.clone());

    if let Some(dist_dir) = resolve_dashboard_dist_dir() {
        let index_file = dist_dir.join("index.html");
        if index_file.is_file() {
            info!(path = %dist_dir.display(), "dashboard static files enabled");
            let static_service = get_service(
                ServeDir::new(dist_dir).fallback(ServeFile::new(index_file)),
            );
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
    background_jobs.push(tokio::spawn(cleanup_worker(state.clone())));

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
        if config.webhook.hmac.enabled && config.webhook.hmac.secret.as_deref().unwrap_or("").is_empty() {
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

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.metrics.render_prometheus();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4"))],
        body,
    )
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

    if !user_record.is_active || !auth::verify_password(&payload.password, &user_record.password_hash) {
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
    let ttl_minutes = i64::try_from(state.auth.session_ttl_minutes).unwrap_or(480);
    let expires_at = now + ChronoDuration::minutes(ttl_minutes.max(1));
    let session_token = auth::new_session_token();

    if let Err(err) = create_dashboard_session(&state.pool, &session_token, user_record.id, expires_at).await {
        error!(error = %err, username, "failed to create dashboard session");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            jar,
            Json(json!({ "error": "internal_error" })),
        )
            .into_response();
    }

    let cookie = Cookie::build((DASHBOARD_SESSION_COOKIE, session_token.clone()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.auth.cookie_secure)
        .path("/")
        .build();
    let jar = jar.add(cookie);

    let response = AuthLoginResponseV1 {
        user: AuthUserV1 {
            id: user_record.id,
            username: user_record.username,
            role: user_record.role,
        },
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
    let from = match parse_optional_datetime(query.from.as_deref(), now - ChronoDuration::hours(24)) {
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

    match get_dashboard_summary(&state.pool, from, to).await {
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
        session_id: query.session_id.map(|value| value.trim().to_string()).filter(|v| !v.is_empty()),
        user_id: query.user_id.map(|value| value.trim().to_string()).filter(|v| !v.is_empty()),
        event_type,
        from,
        to,
    };

    match query_timeline_events(&state.pool, &filter, page, page_size).await {
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

    match query_session_timeline(&state.pool, &session_id, page, page_size).await {
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
    let from = match parse_optional_datetime(query.from.as_deref(), now - ChronoDuration::hours(24)) {
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

    let rows = match query_session_report_rows(&state.pool, from, to, user_id).await {
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
            (header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"sessions-report.csv\"")),
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
    let session_record = match get_dashboard_session_by_token(&state.pool, &session_token, now).await {
        Ok(Some(record)) => record,
        Ok(None) => return unauthorized(),
        Err(err) => {
            error!(error = %err, "failed to validate dashboard session");
            return internal_error();
        }
    };

    request.extensions_mut().insert(AuthContext {
        user: session_record.user,
        expires_at: session_record.expires_at,
    });

    next.run(request).await
}

async fn list_presence_sessions_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_active_session_presence(&state.pool).await {
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
    match get_session_presence(&state.pool, &session_id).await {
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
                    return Some((Ok::<Event, Infallible>(event), (receiver, state, session_id, emit_initial)));
                }

                match receiver.recv().await {
                    Ok(changed_session_id) => {
                        if changed_session_id != session_id {
                            continue;
                        }
                        let event = presence_snapshot_sse_event(&state, &session_id).await;
                        return Some((Ok::<Event, Infallible>(event), (receiver, state, session_id, emit_initial)));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        let event = Event::default()
                            .event("presence_lagged")
                            .data(json!({ "session_id": session_id, "skipped_updates": skipped }).to_string());
                        return Some((Ok::<Event, Infallible>(event), (receiver, state, session_id, emit_initial)));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)).text("keepalive"))
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

    match insert_event(&state.pool, &event).await {
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
    match get_session_presence(&state.pool, session_id).await {
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

fn parse_optional_datetime(raw: Option<&str>, default: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
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

        match reset_stuck_processing(&state.pool, stuck_processing_threshold_ms, now_ms).await {
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

        let due_events = match claim_due_events(&state.pool, batch_limit, now_ms).await {
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

async fn process_outbox_record(state: AppState, record: OutboxRecord) -> anyhow::Result<()> {
    let event: SessionEventV1 = match serde_json::from_str(&record.payload) {
        Ok(payload) => payload,
        Err(err) => {
            let attempts = record.attempts.saturating_add(1);
            let now_ms = unix_millis_now();
            mark_failed(
                &state.pool,
                &record.event_id,
                attempts,
                &format!("invalid JSON payload in outbox: {err}"),
                now_ms,
            )
            .await?;
            state.metrics.inc_webhook_failed();
            return Ok(());
        }
    };

    let current_attempt = record.attempts.saturating_add(1);
    let now_ms = unix_millis_now();

    match state.dispatcher.send_event(&event).await {
        Ok(elapsed) => {
            state.circuit_breaker.on_success();
            mark_delivered(&state.pool, &record.event_id, current_attempt, now_ms).await?;
            state
                .metrics
                .inc_webhook_sent(elapsed.as_millis().min(u128::from(u64::MAX)) as u64);
        }
        Err(err) => {
            let error_message = truncate_error(&err.to_string());
            state.circuit_breaker.on_failure(now_ms);
            state.metrics.inc_webhook_failed();

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
                    "webhook delivery permanently failed"
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

async fn cleanup_worker(state: AppState) {
    let interval = Duration::from_secs(state.config.retention.cleanup_interval_minutes.saturating_mul(60));

    loop {
        tokio::time::sleep(interval).await;

        let now_ms = unix_millis_now();
        let retention_ms = state
            .config
            .retention
            .failed_retention_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1_000);
        let cutoff_ms = now_ms.saturating_sub(retention_ms);

        match cleanup_failed_older_than(&state.pool, cutoff_ms).await {
            Ok(deleted) if deleted > 0 => info!(deleted, "cleaned old failed outbox events"),
            Ok(_) => {}
            Err(err) => error!(error = %err, "failed to cleanup old failed outbox events"),
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
    let stale_after_ms = state.config.presence.stale_after_seconds.saturating_mul(1_000);

    loop {
        tokio::time::sleep(interval).await;

        let now_ms = unix_millis_now();
        let stale_before_ms = now_ms.saturating_sub(stale_after_ms) as i64;

        match expire_stale_presence(&state.pool, stale_before_ms, now_ms as i64).await {
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
            warn!(open_until, "circuit breaker opened after repeated webhook failures");
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
