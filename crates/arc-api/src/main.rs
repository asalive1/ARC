use std::{
    env,
    error::Error,
    io,
    net::SocketAddr,
    sync::Arc,
};

use arc_api::{
    router_with_database_and_admin_auth,
    router_with_database_worker_dispatcher_and_admin_auth, AdminAuth, WorkerCommandDispatcher,
};
use arc_application::{AssignmentGenerationStore, WorkerAuthenticator};
use arc_infrastructure::{
    postgres::{connect_postgres, migrate},
    worker_auth::{
        FileWorkerSecretProvider, LocalWorkerSecretProvider,
        SecretProviderWorkerAuthenticator, LOCAL_WORKER_CREDENTIALS_ENV,
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
    let admin_auth = AdminAuth::from_env(database_pool.clone()).await?;

    let api_router = if let Some(authenticator) = worker_authenticator()? {
        let generation_store: Arc<dyn AssignmentGenerationStore> =
            Arc::new(PgAssignmentGenerationStore::new(database_pool.clone()));
        let dispatcher = Arc::new(WorkerCommandDispatcher::new_with_lease_pool(
            generation_store,
            database_pool.clone(),
        ));
        tracing::info!("worker control endpoint enabled with local credentials");
        router_with_database_worker_dispatcher_and_admin_auth(
            database_pool.clone(),
            authenticator,
            dispatcher,
            admin_auth,
        )
    } else {
        tracing::info!("worker control endpoint disabled; no credentials configured");
        router_with_database_and_admin_auth(database_pool.clone(), admin_auth)
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

fn worker_authenticator() -> Result<Option<Arc<dyn WorkerAuthenticator>>, Box<dyn Error>> {
    if let Ok(path) = env::var("ARC_WORKER_CREDENTIALS_FILE") {
        if path.trim().is_empty() {
            return Err(invalid_config(
                "ARC_WORKER_CREDENTIALS_FILE",
                "must not be empty",
            ).into());
        }
        let secret_provider = FileWorkerSecretProvider::from_file(path).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {}", error.code, error.message),
            )
        })?;
        return Ok(Some(Arc::new(SecretProviderWorkerAuthenticator::new(
            secret_provider,
        ))));
    }

    match env::var(LOCAL_WORKER_CREDENTIALS_ENV) {
        Ok(value) if !value.trim().is_empty() => {
            let secret_provider = LocalWorkerSecretProvider::from_json(&value).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {}", error.code, error.message),
                )
            })?;
            Ok(Some(Arc::new(SecretProviderWorkerAuthenticator::new(
                secret_provider,
            ))))
        }
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
