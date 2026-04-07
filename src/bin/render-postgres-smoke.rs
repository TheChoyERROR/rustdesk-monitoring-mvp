use anyhow::Context;
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "render-postgres-smoke")]
#[command(about = "Smoke test minimo para Render Postgres")]
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
    let database_url = normalize_database_url(&args.database_url);

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .context("failed to connect to Render Postgres")?;

    let row = sqlx::query("SELECT 1 AS ok")
        .fetch_one(&pool)
        .await
        .context("failed to run SELECT 1 on Render Postgres")?;
    let ok: i32 = row.get("ok");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS postgres_smoke_runs (
            id BIGSERIAL PRIMARY KEY,
            note TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(&pool)
    .await
    .context("failed to create postgres_smoke_runs table")?;

    sqlx::query(
        r#"
        INSERT INTO postgres_smoke_runs (note)
        VALUES ($1)
        "#,
    )
    .bind("render postgres smoke ok")
    .execute(&pool)
    .await
    .context("failed to insert smoke row into Render Postgres")?;

    let count_row = sqlx::query("SELECT COUNT(*) AS total FROM postgres_smoke_runs")
        .fetch_one(&pool)
        .await
        .context("failed to count smoke rows")?;
    let total: i64 = count_row.get("total");

    println!("Render Postgres OK: select_one={ok}, smoke_rows={total}");
    Ok(())
}

fn normalize_database_url(url: &str) -> String {
    if url.contains("sslmode=") {
        url.to_string()
    } else {
        format!("{url}?sslmode=require")
    }
}
