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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            webhook: WebhookConfig::default(),
            worker: WorkerConfig::default(),
            retention: RetentionConfig::default(),
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
