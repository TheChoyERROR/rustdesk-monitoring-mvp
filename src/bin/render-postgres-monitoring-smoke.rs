use anyhow::Context;
use clap::Parser;
use rustdesk_monitoring_mvp::model::{SessionDirection, SessionEventType, SessionEventV1};
use rustdesk_monitoring_mvp::postgres::connect_postgres;
use rustdesk_monitoring_mvp::postgres_monitoring::{
    get_postgres_monitoring_counts, init_postgres_monitoring_schema, insert_event_pg,
};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "render-postgres-monitoring-smoke")]
#[command(about = "Smoke minimo de session_events/outbox/session_presence sobre Render Postgres")]
struct Args {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .json()
        .init();

    let args = Args::parse();
    let pool = connect_postgres(&args.database_url).await?;
    init_postgres_monitoring_schema(&pool).await?;

    let session_id = format!(
        "pg-monitoring-{}",
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let participant_id = format!("pg-agent-{}", &Uuid::new_v4().simple().to_string()[..8]);

    let joined = SessionEventV1 {
        event_id: Uuid::new_v4(),
        event_type: SessionEventType::ParticipantJoined,
        session_id: session_id.clone(),
        user_id: participant_id.clone(),
        direction: SessionDirection::Outgoing,
        timestamp: chrono::Utc::now(),
        host_info: None,
        meta: Some(serde_json::json!({
            "participant_id": participant_id,
            "display_name": "Postgres Monitoring Smoke",
        })),
    };

    let started = SessionEventV1 {
        event_id: Uuid::new_v4(),
        event_type: SessionEventType::SessionStarted,
        session_id: session_id.clone(),
        user_id: "pg-agent-runtime".to_string(),
        direction: SessionDirection::Outgoing,
        timestamp: chrono::Utc::now(),
        host_info: None,
        meta: None,
    };

    insert_event_pg(&pool, &joined)
        .await
        .context("failed to insert participant_joined smoke event into Postgres monitoring")?;
    insert_event_pg(&pool, &started)
        .await
        .context("failed to insert session_started smoke event into Postgres monitoring")?;

    let counts = get_postgres_monitoring_counts(&pool).await?;

    anyhow::ensure!(
        counts.outbox_events >= 2,
        "expected smoke outbox rows in Postgres"
    );
    anyhow::ensure!(
        counts.session_events >= 2,
        "expected smoke session rows in Postgres"
    );
    anyhow::ensure!(
        counts.session_presence >= 1,
        "expected smoke session_presence rows in Postgres"
    );

    println!(
        "Render Postgres monitoring OK: outbox_events={}, session_events={}, session_presence={}",
        counts.outbox_events, counts.session_events, counts.session_presence
    );

    Ok(())
}
