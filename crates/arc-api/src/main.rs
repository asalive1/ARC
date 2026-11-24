use std::{
    env,
    error::Error,
    io::{self, Read},
    net::SocketAddr,
    sync::Arc,
};

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

    let credentials_json = worker_credentials_json()?;
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
        let dispatcher = Arc::new(WorkerCommandDispatcher::new_with_lease_pool(
            generation_store,
            database_pool.clone(),
        ));
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

fn worker_credentials_json() -> Result<Option<String>, Box<dyn Error>> {
    if let Ok(path) = env::var("ARC_WORKER_CREDENTIALS_FILE") {
        if path.trim().is_empty() {
            return Err(invalid_config(
                "ARC_WORKER_CREDENTIALS_FILE",
                "must not be empty",
            ).into());
        }
        let mut file = std::fs::File::open(&path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        if contents.trim().is_empty() {
            return Err(invalid_config(
                "ARC_WORKER_CREDENTIALS_FILE",
                "secret file is empty",
            )
            .into());
        }
        return Ok(Some(contents));
    }

    match env::var(LOCAL_WORKER_CREDENTIALS_ENV) {
        Ok(value) if !value.trim().is_empty() => Ok(Some(value)),
        Ok(_) | Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn invalid_config(name: &str, detail: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{name}: {detail}"),
    )
}
