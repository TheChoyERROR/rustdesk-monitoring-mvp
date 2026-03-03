use clap::Parser;
use rustdesk_monitoring_mvp::client::Cli;
use rustdesk_monitoring_mvp::client::run;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let cli = Cli::parse();
    run(cli).await
}
