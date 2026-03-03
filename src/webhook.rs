use std::time::{Duration, Instant};

use anyhow::Context;
use hmac::{Hmac, Mac};
use reqwest::Method;
use sha2::Sha256;

use crate::config::{WebhookConfig, WebhookMethod};
use crate::model::SessionEventV1;
use crate::storage::unix_millis_now;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct WebhookDispatcher {
    client: reqwest::Client,
    cfg: WebhookConfig,
}

impl WebhookDispatcher {
    pub fn new(cfg: WebhookConfig) -> anyhow::Result<Self> {
        let timeout = Duration::from_millis(cfg.timeout_ms);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build webhook HTTP client")?;

        Ok(Self { client, cfg })
    }

    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    pub async fn send_event(&self, event: &SessionEventV1) -> anyhow::Result<Duration> {
        let webhook_url = self
            .cfg
            .url
            .as_deref()
            .context("webhook url is missing")?;

        let method = match self.cfg.method {
            WebhookMethod::Post => Method::POST,
            WebhookMethod::Put => Method::PUT,
        };

        let payload = serde_json::to_string(event).context("failed to serialize webhook payload")?;
        let timestamp = unix_millis_now().to_string();

        let mut request = self
            .client
            .request(method, webhook_url)
            .header("content-type", "application/json")
            .header("x-event-id", event.event_id.to_string())
            .header("x-event-type", event.event_type.as_str())
            .header("x-timestamp", &timestamp)
            .body(payload.clone());

        if self.cfg.hmac.enabled {
            let secret = self
                .cfg
                .hmac
                .secret
                .as_deref()
                .context("webhook HMAC enabled but secret is missing")?;
            let signature = build_hmac_signature(secret, &timestamp, &payload)?;
            request = request
                .header("x-signature", format!("sha256={signature}"))
                .header("x-signature-version", "v1");
        }

        let started = Instant::now();
        let response = request
            .send()
            .await
            .context("failed to send webhook request")?;
        let elapsed = started.elapsed();

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("webhook returned HTTP {} with body: {}", status, body);
        }

        Ok(elapsed)
    }
}

pub fn build_hmac_signature(secret: &str, timestamp: &str, payload: &str) -> anyhow::Result<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .context("invalid HMAC secret bytes for SHA-256")?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(payload.as_bytes());

    let signature = mac.finalize().into_bytes();
    Ok(hex::encode(signature))
}

#[cfg(test)]
mod tests {
    use super::build_hmac_signature;

    #[test]
    fn signature_is_deterministic() {
        let secret = "top-secret";
        let timestamp = "1700000000000";
        let payload = r#"{"hello":"world"}"#;

        let a = build_hmac_signature(secret, timestamp, payload).expect("signature A");
        let b = build_hmac_signature(secret, timestamp, payload).expect("signature B");
        assert_eq!(a, b);
    }
}
