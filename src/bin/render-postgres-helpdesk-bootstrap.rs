use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use rustdesk_monitoring_mvp::postgres::bootstrap_helpdesk_from_sqlite;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "render-postgres-helpdesk-bootstrap")]
#[command(about = "Bootstrap de helpdesk desde SQLite local hacia Render Postgres")]
struct Args {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[arg(long, default_value = "./data/outbox.db")]
    sqlite_path: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .json()
        .init();

    let args = Args::parse();
    let summary = bootstrap_helpdesk_from_sqlite(&args.database_url, &args.sqlite_path)
        .await
        .with_context(|| {
            format!(
                "failed to bootstrap Postgres helpdesk from '{}'",
                args.sqlite_path.display()
            )
        })?;

    println!(
        "Render Postgres helpdesk bootstrap OK: authorized_agents={}, agents={}, tickets={}, assignments={}, heartbeats={}, audit_events={}",
        summary.authorized_agents,
        summary.agents,
        summary.tickets,
        summary.assignments,
        summary.heartbeats,
        summary.audit_events
    );

    Ok(())
}
