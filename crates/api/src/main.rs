//! The API process. `DEMO_MODE=1 cargo run -p microinsurance-api` (from the repository root)
//! serves the synthetic book offline with saved AI replies and the built apps at `/app`,
//! `/embed`, `/partner` and `/ops`; otherwise it needs `API_TOKEN_SECRET`, `AUDIT_SEED` and
//! `RATING_BASIS`, and keeps its records in PostgreSQL with `DATABASE_URL` (see
//! `Settings::from_env`).
use microinsurance_api::{Settings, app, build};

#[tokio::main]
async fn main() -> Result<(), String> {
    let settings = Settings::from_env()?;
    let root = settings.root.clone();
    let demo = settings.demo;
    let state = build(settings).await?;
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "api     listening on http://localhost:{port}{}",
        if demo {
            " (demo mode: offline, synthetic book; apps at /app /embed /partner /ops)"
        } else {
            ""
        }
    );
    axum::serve(listener, app(state, &root))
        .await
        .map_err(|e| e.to_string())
}
