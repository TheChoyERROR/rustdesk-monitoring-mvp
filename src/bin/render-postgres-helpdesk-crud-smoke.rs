use anyhow::Context;
use clap::Parser;
use rustdesk_monitoring_mvp::model::{HelpdeskAuthorizedAgentUpsertRequestV1, HelpdeskTicketCreateRequestV1};
use rustdesk_monitoring_mvp::postgres::{connect_postgres, init_postgres_helpdesk_schema};
use rustdesk_monitoring_mvp::postgres_helpdesk::{
    append_helpdesk_audit_event_pg, create_helpdesk_ticket_pg, get_helpdesk_ticket_pg,
    list_helpdesk_authorized_agents_pg, list_helpdesk_ticket_audit_pg, list_helpdesk_tickets_pg,
    upsert_helpdesk_authorized_agent_pg,
};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "render-postgres-helpdesk-crud-smoke")]
#[command(about = "Smoke CRUD de helpdesk sobre Render Postgres")]
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
    init_postgres_helpdesk_schema(&pool).await?;

    let smoke_suffix = Uuid::new_v4().simple().to_string();
    let agent_id = format!("pgsmoke{}", &smoke_suffix[..8]);
    let display_name = format!("PG Smoke {}", &smoke_suffix[..8]);
    let client_id = format!("client-{}", &smoke_suffix[..8]);

    let agent = upsert_helpdesk_authorized_agent_pg(
        &pool,
        &HelpdeskAuthorizedAgentUpsertRequestV1 {
            agent_id: agent_id.clone(),
            display_name: Some(display_name.clone()),
        },
    )
    .await?;

    let authorized_agents = list_helpdesk_authorized_agents_pg(&pool).await?;
    let ticket = create_helpdesk_ticket_pg(
        &pool,
        &HelpdeskTicketCreateRequestV1 {
            client_id: client_id.clone(),
            client_display_name: Some("Cliente Smoke".to_string()),
            device_id: Some("device-smoke".to_string()),
            requested_by: Some("Render Postgres Smoke".to_string()),
            title: Some("Smoke ticket".to_string()),
            description: Some("Validacion CRUD helpdesk sobre Postgres".to_string()),
            difficulty: None,
            estimated_minutes: None,
            summary: Some("Smoke summary".to_string()),
            preferred_agent_id: None,
        },
    )
    .await?;

    append_helpdesk_audit_event_pg(
        &pool,
        "ticket",
        &ticket.ticket_id,
        "postgres_smoke_checked",
        Some(serde_json::json!({
            "agent_id": agent.agent_id,
            "display_name": agent.display_name,
        })),
    )
    .await?;

    let fetched_ticket = get_helpdesk_ticket_pg(&pool, &ticket.ticket_id)
        .await?
        .context("expected Postgres smoke ticket to exist")?;
    let tickets = list_helpdesk_tickets_pg(&pool).await?;
    let audit = list_helpdesk_ticket_audit_pg(&pool, &ticket.ticket_id).await?;

    println!(
        "Render Postgres helpdesk CRUD OK: authorized_agents={}, tickets={}, fetched_ticket={}, audit_events={}",
        authorized_agents.len(),
        tickets.len(),
        fetched_ticket.ticket_id,
        audit.len()
    );

    Ok(())
}
