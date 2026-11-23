use sqlx::{migrate::Migrator, postgres::PgPoolOptions};

pub use sqlx::PgPool;

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

#[cfg(test)]
mod tests {
    use std::env;

    use sqlx::PgPool;

    use super::{connect_postgres, migrate};

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn sqlx_migrator_creates_foundational_schema() {
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is set for this test");
        let pool: PgPool = connect_postgres(&database_url)
            .await
            .expect("PostgreSQL connection succeeds");
        migrate(&pool)
            .await
            .expect("SQLx applies the initial migration");

        let table_exists: bool =
            sqlx::query_scalar("SELECT to_regclass('public.sources') IS NOT NULL")
                .fetch_one(&pool)
                .await
                .expect("schema query succeeds");
        assert!(table_exists, "initial migration creates the sources table");
    }
}
