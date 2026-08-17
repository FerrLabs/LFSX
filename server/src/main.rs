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

    // Once at boot, since a crash mid-transfer is exactly what leaves these
    // behind, then hourly so a long-lived process reclaims them too.
    lfsx_server::reclaim(&config).await;
    tokio::spawn(reclaim_periodically(config.clone()));

    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(bind = %config.bind, root = ?config.storage_root, "lfsx listening");

    axum::serve(listener, lfsx_server::app(config))
        .with_graceful_shutdown(shutdown())
        .await?;

    Ok(())
}

async fn reclaim_periodically(config: Config) {
    let mut hourly = tokio::time::interval(std::time::Duration::from_secs(3600));
    hourly.tick().await;

    loop {
        hourly.tick().await;
        lfsx_server::reclaim(&config).await;
    }
}

async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }

    tracing::info!("shutting down");
}
