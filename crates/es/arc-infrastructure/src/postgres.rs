use sqlx::{migrate::Migrator, postgres::PgPoolOptions, PgPool};

pub static MIGRATOR: Migrator = sqlx::migrate!("../../database/migrations");

pub async fn connect_postgres(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}
