use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub webhook: WebhookConfig,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub presence: PresenceConfig,
    #[serde(default)]
    pub monitoring: MonitoringConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            webhook: WebhookConfig::default(),
            worker: WorkerConfig::default(),
            retention: RetentionConfig::default(),
            presence: PresenceConfig::default(),
            monitoring: MonitoringConfig::default(),
        }
    }
}

impl ServerConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw)
            .with_context(|| format!("invalid TOML in {}", path.display()))?;
        Ok(cfg)
    }

    pub fn apply_env_overrides(&mut self) {
        if let Some(value) = env_bool("MONITORING_CAPTURE_NON_AGENT_EVENTS") {
            self.monitoring.capture_non_agent_events = value;
        }
        if let Some(value) = env_u64("MONITORING_PARTICIPANT_ACTIVITY_MIN_INTERVAL_SECONDS") {
            self.monitoring.participant_activity_min_interval_seconds = value;
        }
        if let Some(value) = env_u64("MONITORING_LOCAL_DELIVERED_OUTBOX_RETENTION_DAYS") {
            self.monitoring.local_delivered_outbox_retention_days = value;
        }
        if let Some(value) = env_u64("MONITORING_LOCAL_SESSION_EVENT_RETENTION_DAYS") {
            self.monitoring.local_session_event_retention_days = value;
        }
        if let Some(value) = env_u64("MONITORING_LOCAL_SESSION_PRESENCE_RETENTION_HOURS") {
            self.monitoring.local_session_presence_retention_hours = value;
        }
        if let Some(value) = env_u64("MONITORING_LOCAL_AGENT_HEARTBEAT_RETENTION_DAYS") {
            self.monitoring.local_agent_heartbeat_retention_days = value;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub enabled: bool,
    pub url: Option<String>,
    #[serde(default)]
    pub method: WebhookMethod,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub retry: RetryConfig,
    #[serde(default)]
    pub hmac: HmacConfig,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: None,
            method: WebhookMethod::Post,
            timeout_ms: default_timeout_ms(),
            retry: RetryConfig::default(),
            hmac: HmacConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum WebhookMethod {
    Post,
    Put,
}

impl Default for WebhookMethod {
    fn default() -> Self {
        Self::Post
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_backoff_ms")]
    pub backoff_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            backoff_ms: default_backoff_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HmacConfig {
    pub enabled: bool,
    pub secret: Option<String>,
}

impl Default for HmacConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            secret: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    #[serde(default = "default_worker_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_worker_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,
    #[serde(default = "default_circuit_breaker_cooldown_ms")]
    pub circuit_breaker_cooldown_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: default_worker_concurrency(),
            poll_interval_ms: default_worker_poll_interval_ms(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            circuit_breaker_cooldown_ms: default_circuit_breaker_cooldown_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    #[serde(default = "default_failed_retention_days")]
    pub failed_retention_days: u64,
    #[serde(default = "default_cleanup_interval_minutes")]
    pub cleanup_interval_minutes: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            failed_retention_days: default_failed_retention_days(),
            cleanup_interval_minutes: default_cleanup_interval_minutes(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceConfig {
    #[serde(default = "default_presence_stale_after_seconds")]
    pub stale_after_seconds: u64,
    #[serde(default = "default_presence_cleanup_interval_seconds")]
    pub cleanup_interval_seconds: u64,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            stale_after_seconds: default_presence_stale_after_seconds(),
            cleanup_interval_seconds: default_presence_cleanup_interval_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    #[serde(default = "default_capture_non_agent_events")]
    pub capture_non_agent_events: bool,
    #[serde(default = "default_participant_activity_min_interval_seconds")]
    pub participant_activity_min_interval_seconds: u64,
    #[serde(default = "default_local_delivered_outbox_retention_days")]
    pub local_delivered_outbox_retention_days: u64,
    #[serde(default = "default_local_session_event_retention_days")]
    pub local_session_event_retention_days: u64,
    #[serde(default = "default_local_session_presence_retention_hours")]
    pub local_session_presence_retention_hours: u64,
    #[serde(default = "default_local_agent_heartbeat_retention_days")]
    pub local_agent_heartbeat_retention_days: u64,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            capture_non_agent_events: default_capture_non_agent_events(),
            participant_activity_min_interval_seconds:
                default_participant_activity_min_interval_seconds(),
            local_delivered_outbox_retention_days:
                default_local_delivered_outbox_retention_days(),
            local_session_event_retention_days: default_local_session_event_retention_days(),
            local_session_presence_retention_hours:
                default_local_session_presence_retention_hours(),
            local_agent_heartbeat_retention_days:
                default_local_agent_heartbeat_retention_days(),
        }
    }
}

const fn default_timeout_ms() -> u64 {
    3_000
}

const fn default_max_attempts() -> u32 {
    5
}

const fn default_backoff_ms() -> u64 {
    500
}

const fn default_worker_concurrency() -> usize {
    4
}

const fn default_worker_poll_interval_ms() -> u64 {
    500
}

const fn default_circuit_breaker_threshold() -> u32 {
    5
}

const fn default_circuit_breaker_cooldown_ms() -> u64 {
    10_000
}

const fn default_failed_retention_days() -> u64 {
    7
}

const fn default_cleanup_interval_minutes() -> u64 {
    60
}

const fn default_presence_stale_after_seconds() -> u64 {
    120
}

const fn default_presence_cleanup_interval_seconds() -> u64 {
    30
}

const fn default_capture_non_agent_events() -> bool {
    false
}

const fn default_participant_activity_min_interval_seconds() -> u64 {
    60
}

const fn default_local_delivered_outbox_retention_days() -> u64 {
    1
}

const fn default_local_session_event_retention_days() -> u64 {
    7
}

const fn default_local_session_presence_retention_hours() -> u64 {
    24
}

const fn default_local_agent_heartbeat_retention_days() -> u64 {
    7
}

fn env_bool(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok()?;
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}
