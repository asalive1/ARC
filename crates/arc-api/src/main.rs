use std::{env, error::Error, net::SocketAddr, sync::Arc};

use arc_api::{
    router_with_database, router_with_database_and_worker_dispatcher, WorkerCommandDispatcher,
};
use arc_application::{AssignmentGenerationStore, WorkerAuthenticator};
use arc_infrastructure::{
    postgres::{connect_postgres, migrate},
    worker_auth::{
        LocalWorkerSecretProvider, SecretProviderWorkerAuthenticator, LOCAL_WORKER_CREDENTIALS_ENV,
    },
    worker_fencing::PgAssignmentGenerationStore,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt().json().init();

    let database_url = env::var("DATABASE_URL")?;
    let database_pool = connect_postgres(&database_url).await?;
    migrate(&database_pool).await?;

    let credentials_json = match env::var(LOCAL_WORKER_CREDENTIALS_ENV) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        Ok(_) | Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let api_router = if let Some(credentials_json) = credentials_json {
        let secret_provider =
            LocalWorkerSecretProvider::from_json(&credentials_json).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {}", error.code, error.message),
                )
            })?;
        let authenticator: Arc<dyn WorkerAuthenticator> =
            Arc::new(SecretProviderWorkerAuthenticator::new(secret_provider));
        let generation_store: Arc<dyn AssignmentGenerationStore> =
            Arc::new(PgAssignmentGenerationStore::new(database_pool.clone()));
        let dispatcher = Arc::new(WorkerCommandDispatcher::new(generation_store));
        tracing::info!("worker control endpoint enabled with local credentials");
        router_with_database_and_worker_dispatcher(database_pool.clone(), authenticator, dispatcher)
    } else {
        tracing::info!("worker control endpoint disabled; no credentials configured");
        router_with_database(database_pool.clone())
    };

    let bind_address = env::var("ARC_API_BIND_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse::<SocketAddr>()?;
    let listener = TcpListener::bind(bind_address).await?;

    tracing::info!(%bind_address, "ARC API listening");
    axum::serve(listener, api_router)
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
