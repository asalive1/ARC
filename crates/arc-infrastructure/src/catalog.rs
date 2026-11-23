use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub id: Uuid,
    pub friendly_name: String,
    pub source_type: String,
    pub desired_state: String,
    pub current_revision: i64,
    pub revision_id: Uuid,
    pub lifecycle: String,
    pub configuration: Value,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationRecord {
    pub id: Uuid,
    pub friendly_name: String,
    pub destination_type: String,
    pub desired_state: String,
    pub current_revision: i64,
    pub revision_id: Uuid,
    pub lifecycle: String,
    pub configuration: Value,
}

pub async fn create_source(
    pool: &PgPool,
    friendly_name: &str,
    source_type: &str,
    configuration: Value,
) -> Result<SourceRecord, sqlx::Error> {
    let id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let mut transaction = pool.begin().await?;

    sqlx::query(
        "INSERT INTO sources (id, friendly_name, source_type, desired_state, current_revision) \
         VALUES ($1, $2, $3, 'disabled', 1)",
    )
    .bind(id)
    .bind(friendly_name)
    .bind(source_type)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO source_revisions (id, source_id, revision, lifecycle, configuration) \
         VALUES ($1, $2, 1, 'draft', $3)",
    )
    .bind(revision_id)
    .bind(id)
    .bind(&configuration)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(SourceRecord {
        id,
        friendly_name: friendly_name.to_owned(),
        source_type: source_type.to_owned(),
        desired_state: "disabled".to_owned(),
        current_revision: 1,
        revision_id,
        lifecycle: "draft".to_owned(),
        configuration,
    })
}

pub async fn list_sources(pool: &PgPool) -> Result<Vec<SourceRecord>, sqlx::Error> {
    sqlx::query_as(
        "SELECT s.id, s.friendly_name, s.source_type, s.desired_state, \
         s.current_revision, r.id AS revision_id, r.lifecycle, r.configuration \
         FROM sources s \
         JOIN source_revisions r \
           ON r.source_id = s.id AND r.revision = s.current_revision \
         ORDER BY s.created_at, s.id",
    )
    .fetch_all(pool)
    .await
}

pub async fn create_destination(
    pool: &PgPool,
    friendly_name: &str,
    destination_type: &str,
    configuration: Value,
) -> Result<DestinationRecord, sqlx::Error> {
    let id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let mut transaction = pool.begin().await?;

    sqlx::query(
        "INSERT INTO destinations \
         (id, friendly_name, destination_type, desired_state, current_revision) \
         VALUES ($1, $2, $3, 'disabled', 1)",
    )
    .bind(id)
    .bind(friendly_name)
    .bind(destination_type)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO destination_revisions \
         (id, destination_id, revision, lifecycle, configuration) \
         VALUES ($1, $2, 1, 'draft', $3)",
    )
    .bind(revision_id)
    .bind(id)
    .bind(&configuration)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(DestinationRecord {
        id,
        friendly_name: friendly_name.to_owned(),
        destination_type: destination_type.to_owned(),
        desired_state: "disabled".to_owned(),
        current_revision: 1,
        revision_id,
        lifecycle: "draft".to_owned(),
        configuration,
    })
}

pub async fn list_destinations(pool: &PgPool) -> Result<Vec<DestinationRecord>, sqlx::Error> {
    sqlx::query_as(
        "SELECT d.id, d.friendly_name, d.destination_type, d.desired_state, \
         d.current_revision, r.id AS revision_id, r.lifecycle, r.configuration \
         FROM destinations d \
         JOIN destination_revisions r \
           ON r.destination_id = d.id AND r.revision = d.current_revision \
         ORDER BY d.created_at, d.id",
    )
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use std::env;

    use serde_json::json;

    use crate::postgres::{connect_postgres, migrate};

    use super::*;

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn catalog_creates_and_lists_source_and_destination_drafts() {
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is set for this test");
        let pool = connect_postgres(&database_url)
            .await
            .expect("PostgreSQL connection succeeds");
        migrate(&pool)
            .await
            .expect("SQLx applies the initial migration");

        let suffix = Uuid::new_v4();
        let source_name = format!("catalog-source-{suffix}");
        let source_configuration = json!({
            "kind": "rtp_mp2t",
            "bindAddress": "0.0.0.0",
            "port": 5000
        });
        let source = create_source(
            &pool,
            &source_name,
            "rtp_mp2t",
            source_configuration.clone(),
        )
        .await
        .expect("source and initial revision are created");

        assert_eq!(source.friendly_name, source_name);
        assert_eq!(source.desired_state, "disabled");
        assert_eq!(source.current_revision, 1);
        assert_eq!(source.lifecycle, "draft");
        assert_eq!(source.configuration, source_configuration);
        assert!(list_sources(&pool)
            .await
            .expect("sources can be listed")
            .iter()
            .any(|listed| listed.id == source.id && listed.revision_id == source.revision_id));

        let destination_name = format!("catalog-destination-{suffix}");
        let destination_configuration = json!({
            "kind": "srt_mpeg_ts",
            "host": "127.0.0.1",
            "port": 6000,
            "mode": "caller"
        });
        let destination = create_destination(
            &pool,
            &destination_name,
            "srt_mpeg_ts",
            destination_configuration.clone(),
        )
        .await
        .expect("destination and initial revision are created");

        assert_eq!(destination.friendly_name, destination_name);
        assert_eq!(destination.desired_state, "disabled");
        assert_eq!(destination.current_revision, 1);
        assert_eq!(destination.lifecycle, "draft");
        assert_eq!(destination.configuration, destination_configuration);
        assert!(list_destinations(&pool)
            .await
            .expect("destinations can be listed")
            .iter()
            .any(|listed| listed.id == destination.id
                && listed.revision_id == destination.revision_id));
    }
}
