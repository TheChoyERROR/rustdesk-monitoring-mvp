use clap::Parser;
use rustdesk_monitoring_mvp::model::HelpdeskTicketCreateRequestV1;
use rustdesk_monitoring_mvp::turso::run_helpdesk_smoke;

#[derive(Debug, Parser)]
#[command(name = "turso-helpdesk-smoke")]
#[command(about = "Valida operaciones reales de helpdesk sobre Turso")]
struct Args {
    #[arg(long, env = "TURSO_DATABASE_URL")]
    url: String,
    #[arg(long, env = "TURSO_AUTH_TOKEN")]
    auth_token: String,
    #[arg(
        long,
        env = "DASHBOARD_SUPERVISOR_USERNAME",
        default_value = "supervisor"
    )]
    supervisor_username: String,
    #[arg(
        long,
        env = "DASHBOARD_SUPERVISOR_PASSWORD",
        default_value = "ChangeMeNow123!"
    )]
    supervisor_password: String,
    #[arg(long)]
    agent_id: String,
    #[arg(long)]
    agent_name: Option<String>,
    #[arg(long)]
    client_id: String,
    #[arg(long)]
    client_name: Option<String>,
    #[arg(long)]
    title: String,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    difficulty: Option<String>,
    #[arg(long)]
    estimated_minutes: Option<u32>,
    #[arg(long)]
    summary: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let ticket = HelpdeskTicketCreateRequestV1 {
        client_id: args.client_id,
        client_display_name: args.client_name,
        device_id: None,
        requested_by: Some("turso-helpdesk-smoke".to_string()),
        title: Some(args.title),
        description: args.description,
        difficulty: args.difficulty,
        estimated_minutes: args.estimated_minutes,
        summary: args.summary,
        preferred_agent_id: None,
    };

    ticket.validate()?;

    let summary = run_helpdesk_smoke(
        &args.url,
        &args.auth_token,
        &args.supervisor_username,
        &args.supervisor_password,
        &args.agent_id,
        args.agent_name.as_deref(),
        &ticket,
    )
    .await?;

    println!("Connected to Turso: {}", summary.url);
    println!("Authorized agent: {}", summary.authorized_agent_id);
    println!("Created ticket: {}", summary.created_ticket.ticket_id);
    println!(
        "Created ticket status: {}",
        summary.created_ticket.status.as_str()
    );
    println!("Tickets total: {}", summary.tickets_total);
    println!(
        "Operational summary: queued={}, opening={}, in_progress={}, resolved={}",
        summary.operational_summary.tickets_queued,
        summary.operational_summary.tickets_opening,
        summary.operational_summary.tickets_in_progress,
        summary.operational_summary.tickets_resolved
    );

    Ok(())
}
