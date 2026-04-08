use anyhow::Context;
use clap::Parser;
use rustdesk_monitoring_mvp::model::{
    HelpdeskAgentPresenceUpdateV1, HelpdeskAgentStatus, HelpdeskAuthorizedAgentUpsertRequestV1,
    HelpdeskTicketCreateRequestV1,
};
use rustdesk_monitoring_mvp::postgres::{connect_postgres, init_postgres_helpdesk_schema};
use rustdesk_monitoring_mvp::postgres_helpdesk::{
    add_helpdesk_ticket_agent_report_pg, assign_helpdesk_ticket_pg, create_helpdesk_ticket_pg,
    get_helpdesk_agent_authorization_status_pg, get_helpdesk_agent_pg,
    get_helpdesk_assignment_for_agent_pg, get_helpdesk_ticket_pg, resolve_helpdesk_ticket_pg,
    start_helpdesk_ticket_pg, upsert_helpdesk_agent_presence_pg,
    upsert_helpdesk_authorized_agent_pg,
};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "render-postgres-helpdesk-agent-runtime-smoke")]
#[command(about = "Smoke end-to-end del runtime de helpdesk sobre Render Postgres")]
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
    let agent_id = format!("pgruntime{}", &smoke_suffix[..8]);
    let agent_name = format!("PG Runtime {}", &smoke_suffix[..8]);
    let client_id = format!("pg-client-{}", &smoke_suffix[..8]);

    upsert_helpdesk_authorized_agent_pg(
        &pool,
        &HelpdeskAuthorizedAgentUpsertRequestV1 {
            agent_id: agent_id.clone(),
            display_name: Some(agent_name.clone()),
        },
    )
    .await?;

    let authorization = get_helpdesk_agent_authorization_status_pg(&pool, &agent_id).await?;
    anyhow::ensure!(
        authorization.authorized,
        "expected authorized agent '{}' to be visible in Postgres runtime",
        agent_id
    );

    let presence_agent = upsert_helpdesk_agent_presence_pg(
        &pool,
        &HelpdeskAgentPresenceUpdateV1 {
            agent_id: agent_id.clone(),
            display_name: Some(agent_name.clone()),
            avatar_url: None,
            status: HelpdeskAgentStatus::Available,
        },
    )
    .await?;

    let ticket = create_helpdesk_ticket_pg(
        &pool,
        &HelpdeskTicketCreateRequestV1 {
            client_id: client_id.clone(),
            client_display_name: Some("Cliente Runtime".to_string()),
            device_id: Some("pg-runtime-device".to_string()),
            requested_by: Some("Render Postgres Runtime Smoke".to_string()),
            title: Some("Runtime smoke ticket".to_string()),
            description: Some("Validacion del runtime del agente sobre Postgres".to_string()),
            difficulty: None,
            estimated_minutes: None,
            summary: Some("Runtime smoke summary".to_string()),
            preferred_agent_id: None,
        },
    )
    .await?;

    let (opening_ticket, opening_agent) = assign_helpdesk_ticket_pg(
        &pool,
        &ticket.ticket_id,
        Some(&agent_id),
        Some("runtime-smoke"),
    )
    .await?;

    let assignment = get_helpdesk_assignment_for_agent_pg(&pool, &agent_id)
        .await?
        .context("expected assignment to be visible for agent after manual dispatch")?;

    let (in_progress_ticket, busy_agent) =
        start_helpdesk_ticket_pg(&pool, &agent_id, &ticket.ticket_id).await?;

    let reported_ticket = add_helpdesk_ticket_agent_report_pg(
        &pool,
        &ticket.ticket_id,
        &agent_id,
        "Runtime smoke report: customer accepted connection.",
    )
    .await?;

    let (resolved_ticket, released_agent) = resolve_helpdesk_ticket_pg(
        &pool,
        &ticket.ticket_id,
        &agent_id,
        HelpdeskAgentStatus::Available,
    )
    .await?;

    let fetched_agent = get_helpdesk_agent_pg(&pool, &agent_id)
        .await?
        .context("expected agent to exist after runtime smoke")?;
    let fetched_ticket = get_helpdesk_ticket_pg(&pool, &ticket.ticket_id)
        .await?
        .context("expected ticket to exist after runtime smoke")?;

    println!(
        "Render Postgres helpdesk runtime OK: authorized={}, opening_status={:?}, assignment_ticket={}, in_progress={:?}, report_present={}, resolved={:?}, released_agent={:?}, fetched_agent={:?}, fetched_ticket={:?}, initial_presence={:?}",
        authorization.authorized,
        opening_ticket.status,
        assignment.ticket.ticket_id,
        in_progress_ticket.status,
        reported_ticket.latest_agent_report.is_some(),
        resolved_ticket.status,
        released_agent.status,
        fetched_agent.status,
        fetched_ticket.status,
        presence_agent.status,
    );

    let _ = opening_agent;
    let _ = busy_agent;

    Ok(())
}
