use std::{env, error::Error, net::SocketAddr};

use arc_api::router;
use arc_infrastructure::postgres::{connect_postgres, migrate};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt().json().init();

    let database_url = env::var("DATABASE_URL")?;
    let database_pool = connect_postgres(&database_url).await?;
    migrate(&database_pool).await?;

    let bind_address = env::var("ARC_API_BIND_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse::<SocketAddr>()?;
    let listener = TcpListener::bind(bind_address).await?;

    tracing::info!(%bind_address, "ARC API listening");
    axum::serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    drop(database_pool);
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
        return;
    }

    tracing::info!("shutdown signal received");
}
