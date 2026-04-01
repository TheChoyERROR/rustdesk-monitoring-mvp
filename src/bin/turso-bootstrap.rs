use clap::Parser;
use rustdesk_monitoring_mvp::turso::bootstrap_turso_remote;

#[derive(Debug, Parser)]
#[command(name = "turso-bootstrap")]
#[command(about = "Conecta a Turso y aplica el schema base del proyecto")]
struct Args {
    #[arg(long, env = "TURSO_DATABASE_URL")]
    url: String,
    #[arg(long, env = "TURSO_AUTH_TOKEN")]
    auth_token: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    eprintln!("turso-bootstrap: starting remote bootstrap");
    let summary = bootstrap_turso_remote(&args.url, &args.auth_token).await?;

    println!("Connected to Turso: {}", summary.url);
    println!("Schema applied successfully.");
    println!("Tables detected ({}):", summary.tables.len());
    for table in summary.tables {
        println!("- {}", table);
    }

    Ok(())
}
