use lfsx_server::config::Config;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env();
    tokio::fs::create_dir_all(&config.storage_root).await?;

    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(bind = %config.bind, root = ?config.storage_root, "lfsx listening");

    axum::serve(listener, lfsx_server::app(config))
        .with_graceful_shutdown(shutdown())
        .await?;

    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
