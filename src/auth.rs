use std::env;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use anyhow::anyhow;
use rand::rngs::OsRng;
use tracing::warn;
use uuid::Uuid;

pub const DASHBOARD_SESSION_COOKIE: &str = "dashboard_session";

#[derive(Debug, Clone)]
pub struct AuthSettings {
    pub supervisor_username: String,
    pub supervisor_password: String,
    pub session_ttl_minutes: u64,
    pub cookie_secure: bool,
}

impl AuthSettings {
    pub fn from_env() -> Self {
        let supervisor_username =
            env_string("DASHBOARD_SUPERVISOR_USERNAME").unwrap_or_else(|| "supervisor".to_string());
        let supervisor_password =
            env_string("DASHBOARD_SUPERVISOR_PASSWORD").unwrap_or_else(|| "ChangeMeNow123!".to_string());
        let session_ttl_minutes = env_u64("DASHBOARD_SESSION_TTL_MINUTES").unwrap_or(480);
        let cookie_secure = env_bool("DASHBOARD_COOKIE_SECURE").unwrap_or(false);

        if supervisor_password == "ChangeMeNow123!" {
            warn!(
                "using default dashboard supervisor password; set DASHBOARD_SUPERVISOR_PASSWORD in production"
            );
        }

        Self {
            supervisor_username,
            supervisor_password,
            session_ttl_minutes,
            cookie_secure,
        }
    }
}

pub fn hash_password(raw_password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(raw_password.as_bytes(), &salt)
        .map_err(|err| anyhow!("failed to hash password: {err}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(raw_password: &str, password_hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(password_hash) {
        Ok(value) => value,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(raw_password.as_bytes(), &parsed_hash)
        .is_ok()
}

pub fn new_session_token() -> String {
    Uuid::new_v4().to_string()
}

fn env_string(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_u64(key: &str) -> Option<u64> {
    env_string(key)?.parse::<u64>().ok()
}

fn env_bool(key: &str) -> Option<bool> {
    let value = env_string(key)?;
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}
