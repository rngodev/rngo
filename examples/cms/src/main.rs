mod error;
mod routes;

use sqlx::PgPool;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Logs to stdout for interactive use and to `logs/app.log` as JSON lines, so the latter can be
/// tailed by external tooling (e.g. an rngo channel with no effects writing to it, used as a
/// signal source). The returned guard must stay alive for the process lifetime, or the
/// non-blocking file writer stops flushing.
fn init_tracing() -> tracing_appender::non_blocking::WorkerGuard {
    std::fs::create_dir_all("logs").expect("failed to create logs directory");
    let (file_writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never("logs", "app.log"));

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(file_writer),
        )
        .init();

    guard
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let _tracing_guard = init_tracing();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPool::connect(&database_url).await?;
    sqlx::migrate!().run(&pool).await?;

    let app = routes::router(pool).layer(
        TraceLayer::new_for_http()
            .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
            .on_response(DefaultOnResponse::new().level(Level::INFO)),
    );
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}
