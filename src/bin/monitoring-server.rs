use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use rustdesk_monitoring_mvp::config::{ServerConfig, WebhookMethod};
use rustdesk_monitoring_mvp::server;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum WebhookMethodArg {
    Post,
    Put,
}

impl From<WebhookMethodArg> for WebhookMethod {
    fn from(value: WebhookMethodArg) -> Self {
        match value {
            WebhookMethodArg::Post => WebhookMethod::Post,
            WebhookMethodArg::Put => WebhookMethod::Put,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "monitoring-server")]
#[command(about = "Server de monitoreo y auditoria para eventos de sesion")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: String,
    #[arg(long, default_value = "./data/outbox.db")]
    database_path: PathBuf,
    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long)]
    webhook_enabled: Option<bool>,
    #[arg(long)]
    webhook_url: Option<String>,
    #[arg(long)]
    webhook_method: Option<WebhookMethodArg>,
    #[arg(long)]
    webhook_timeout_ms: Option<u64>,
    #[arg(long)]
    webhook_max_attempts: Option<u32>,
    #[arg(long)]
    webhook_backoff_ms: Option<u64>,
    #[arg(long)]
    webhook_hmac_enabled: Option<bool>,
    #[arg(long)]
    webhook_hmac_secret: Option<String>,
    #[arg(long)]
    worker_concurrency: Option<usize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .json()
        .init();

    let args = Args::parse();

    let mut config = if let Some(path) = args.config.as_ref() {
        ServerConfig::load(path)?
    } else {
        ServerConfig::default()
    };

    if let Some(enabled) = args.webhook_enabled {
        config.webhook.enabled = enabled;
    }
    if let Some(url) = args.webhook_url {
        config.webhook.url = Some(url);
        if args.webhook_enabled.is_none() {
            config.webhook.enabled = true;
        }
    }
    if let Some(method) = args.webhook_method {
        config.webhook.method = method.into();
    }
    if let Some(timeout_ms) = args.webhook_timeout_ms {
        config.webhook.timeout_ms = timeout_ms;
    }
    if let Some(max_attempts) = args.webhook_max_attempts {
        config.webhook.retry.max_attempts = max_attempts;
    }
    if let Some(backoff_ms) = args.webhook_backoff_ms {
        config.webhook.retry.backoff_ms = backoff_ms;
    }
    if let Some(hmac_enabled) = args.webhook_hmac_enabled {
        config.webhook.hmac.enabled = hmac_enabled;
    }
    if let Some(secret) = args.webhook_hmac_secret {
        config.webhook.hmac.secret = Some(secret);
    }
    if let Some(concurrency) = args.worker_concurrency {
        config.worker.concurrency = concurrency;
    }

    server::run(&args.bind, &args.database_path, config).await
}
