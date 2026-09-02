use lfsx_server::config::Config;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = lfsx_server::telemetry::init();

    let mut config = Config::from_env();
    tokio::fs::create_dir_all(&config.storage_root).await?;

    // Before anything is served, because it decides whether a client is ever
    // handed a write URL, and a store that will not prove it checks what comes
    // back through one cannot be given that job.
    lfsx_server::verify_presign(&mut config).await;

    // And whether it can arbitrate between two clients reaching for the same
    // lock, which is a different capability and a different failure: one lets a
    // client write bytes nobody checked, the other hands the same lock to two
    // people.
    lfsx_server::verify_locking(&mut config).await;
    let config = config;

    // Once at boot, since a crash mid-transfer is exactly what leaves these
    // behind, then hourly so a long-lived process reclaims them too.
    lfsx_server::reclaim(&config).await;
    tokio::spawn(reclaim_periodically(config.clone()));

    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %config.bind,
        root = ?config.storage_root,
        "lfsx listening"
    );

    let served = axum::serve(listener, lfsx_server::app(config))
        .with_graceful_shutdown(shutdown())
        .await;

    if let Some(provider) = telemetry
        && let Err(error) = provider.shutdown()
    {
        tracing::warn!(%error, "the last batch of spans may not have been exported");
    }

    served?;

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
