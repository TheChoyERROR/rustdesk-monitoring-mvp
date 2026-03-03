use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::config::ServerConfig;
use crate::metrics::Metrics;
use crate::model::SessionEventV1;
use crate::storage::{
    claim_due_events, cleanup_failed_older_than, connect_sqlite, insert_event, mark_delivered,
    get_session_presence, list_active_session_presence, mark_failed, reset_stuck_processing,
    schedule_retry, unix_millis_now, InsertOutcome, OutboxRecord,
};
use crate::webhook::WebhookDispatcher;

#[derive(Clone)]
struct AppState {
    pool: sqlx::SqlitePool,
    metrics: Arc<Metrics>,
    dispatcher: WebhookDispatcher,
    config: Arc<ServerConfig>,
    circuit_breaker: Arc<CircuitBreaker>,
    presence_updates: broadcast::Sender<String>,
}

pub async fn run(bind_addr: &str, database_path: &Path, config: ServerConfig) -> anyhow::Result<()> {
    validate_server_config(&config)?;

    let pool = connect_sqlite(database_path).await?;
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
        circuit_breaker,
        presence_updates,
    };

    let reset_count = reset_stuck_processing(&state.pool, 60_000, unix_millis_now()).await?;
    if reset_count > 0 {
        warn!(reset_count, "reset stale processing rows on startup");
    }

    let router = Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
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
        .with_state(state.clone());

    let mut background_jobs: Vec<JoinHandle<()>> = Vec::new();

    if state.dispatcher.enabled() {
        background_jobs.push(tokio::spawn(webhook_worker(state.clone())));
    } else {
        warn!("webhook is disabled; events will remain queued in outbox");
    }

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

    Ok(())
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
