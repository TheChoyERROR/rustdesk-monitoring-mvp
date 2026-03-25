use std::env;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tracing::warn;

use crate::model::AuthUserV1;

pub const DASHBOARD_SESSION_COOKIE: &str = "dashboard_session";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct AuthSettings {
    pub supervisor_username: String,
    pub supervisor_password: String,
    pub session_ttl_minutes: u64,
    pub cookie_secure: bool,
    pub session_secret: String,
}

#[derive(Debug, Clone)]
pub struct DashboardSessionToken {
    pub user: AuthUserV1,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DashboardSessionClaims {
    user: AuthUserV1,
    expires_at_ms: i64,
    issued_at_ms: i64,
}

impl AuthSettings {
    pub fn from_env() -> Self {
        let supervisor_username =
            env_string("DASHBOARD_SUPERVISOR_USERNAME").unwrap_or_else(|| "supervisor".to_string());
        let supervisor_password =
            env_string("DASHBOARD_SUPERVISOR_PASSWORD").unwrap_or_else(|| "ChangeMeNow123!".to_string());
        let session_ttl_minutes = env_u64("DASHBOARD_SESSION_TTL_MINUTES").unwrap_or(480);
        let cookie_secure = env_bool("DASHBOARD_COOKIE_SECURE").unwrap_or(false);
        let session_secret = env_string("DASHBOARD_SESSION_SECRET")
            .unwrap_or_else(|| format!("{supervisor_username}:{supervisor_password}"));

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
            session_secret,
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

pub fn issue_dashboard_session_token(
    settings: &AuthSettings,
    user: &AuthUserV1,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<String> {
    let claims = DashboardSessionClaims {
        user: user.clone(),
        expires_at_ms: expires_at.timestamp_millis(),
        issued_at_ms: Utc::now().timestamp_millis(),
    };
    let payload = serde_json::to_vec(&claims).map_err(|err| anyhow!("failed to serialize dashboard session: {err}"))?;
    let mut mac = HmacSha256::new_from_slice(settings.session_secret.as_bytes())
        .map_err(|err| anyhow!("failed to initialize dashboard session signer: {err}"))?;
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();

    Ok(format!("{}.{}", hex::encode(payload), hex::encode(signature)))
}

pub fn verify_dashboard_session_token(
    settings: &AuthSettings,
    token: &str,
    now: DateTime<Utc>,
) -> Option<DashboardSessionToken> {
    let (payload_hex, signature_hex) = token.split_once('.')?;
    let payload = hex::decode(payload_hex).ok()?;
    let signature = hex::decode(signature_hex).ok()?;
    let mut mac = HmacSha256::new_from_slice(settings.session_secret.as_bytes()).ok()?;
    mac.update(&payload);
    mac.verify_slice(&signature).ok()?;

    let claims: DashboardSessionClaims = serde_json::from_slice(&payload).ok()?;
    let expires_at = DateTime::<Utc>::from_timestamp_millis(claims.expires_at_ms)?;
    if expires_at <= now {
        return None;
    }

    Some(DashboardSessionToken {
        user: claims.user,
        expires_at,
    })
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

#[cfg(test)]
mod tests {
    use super::{issue_dashboard_session_token, verify_dashboard_session_token, AuthSettings};
    use crate::model::{AuthRoleV1, AuthUserV1};
    use chrono::{Duration, Utc};

    fn settings() -> AuthSettings {
        AuthSettings {
            supervisor_username: "supervisor".to_string(),
            supervisor_password: "secret".to_string(),
            session_ttl_minutes: 480,
            cookie_secure: true,
            session_secret: "session-secret".to_string(),
        }
    }

    fn user() -> AuthUserV1 {
        AuthUserV1 {
            id: 7,
            username: "supervisor".to_string(),
            role: AuthRoleV1::Supervisor,
        }
    }

    #[test]
    fn signed_dashboard_session_round_trip() {
        let now = Utc::now();
        let expires_at = now + Duration::hours(4);
        let token = issue_dashboard_session_token(&settings(), &user(), expires_at).expect("issue token");
        let session = verify_dashboard_session_token(&settings(), &token, now).expect("verify token");

        assert_eq!(session.user.id, 7);
        assert_eq!(session.user.username, "supervisor");
        assert_eq!(session.expires_at.timestamp_millis(), expires_at.timestamp_millis());
    }

    #[test]
    fn signed_dashboard_session_rejects_tampering() {
        let now = Utc::now();
        let expires_at = now + Duration::hours(1);
        let token = issue_dashboard_session_token(&settings(), &user(), expires_at).expect("issue token");
        let tampered = format!("{token}00");

        assert!(verify_dashboard_session_token(&settings(), &tampered, now).is_none());
    }

    #[test]
    fn signed_dashboard_session_rejects_expired_tokens() {
        let now = Utc::now();
        let expires_at = now - Duration::minutes(1);
        let token = issue_dashboard_session_token(&settings(), &user(), expires_at).expect("issue token");

        assert!(verify_dashboard_session_token(&settings(), &token, now).is_none());
    }
}
