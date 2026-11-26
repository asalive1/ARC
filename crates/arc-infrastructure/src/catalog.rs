use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug)]
pub enum DraftValidationError {
    NotFound,
    NotDraft,
    Database(sqlx::Error),
}

#[derive(Debug)]
pub enum ResourceUpdateError {
    NotFound,
    Enabled,
    Database(sqlx::Error),
}

#[derive(Debug)]
pub enum ResourceDeleteError {
    NotFound,
    Enabled,
    StaleRevision,
    InUse,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for ResourceDeleteError {
    fn from(error: sqlx::Error) -> Self {
        if matches!(&error, sqlx::Error::Database(db) if db.is_foreign_key_violation()) {
            Self::InUse
        } else {
            Self::Database(error)
        }
    }
}

pub async fn delete_source(
    pool: &PgPool,
    id: Uuid,
    expected_revision: i64,
) -> Result<(), ResourceDeleteError> {
    delete_unused_resource(pool, id, expected_revision, true).await
}

pub async fn delete_destination(
    pool: &PgPool,
    id: Uuid,
    expected_revision: i64,
) -> Result<(), ResourceDeleteError> {
    delete_unused_resource(pool, id, expected_revision, false).await
}

async fn delete_unused_resource(
    pool: &PgPool,
    id: Uuid,
    expected_revision: i64,
    source: bool,
) -> Result<(), ResourceDeleteError> {
    let (select, delete) = if source {
        (
            "SELECT desired_state, current_revision FROM sources WHERE id = $1 FOR UPDATE",
            "DELETE FROM sources WHERE id = $1",
        )
    } else {
        (
            "SELECT desired_state, current_revision FROM destinations WHERE id = $1 FOR UPDATE",
            "DELETE FROM destinations WHERE id = $1",
        )
    };
    let mut transaction = pool.begin().await?;
    let (state, revision): (String, i64) = sqlx::query_as(select)
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ResourceDeleteError::NotFound)?;
    if revision != expected_revision {
        return Err(ResourceDeleteError::StaleRevision);
    }
    if state != "disabled" {
        return Err(ResourceDeleteError::Enabled);
    }
    sqlx::query(delete)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

impl From<sqlx::Error> for ResourceUpdateError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<sqlx::Error> for DraftValidationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

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

pub async fn update_source(
    pool: &PgPool,
    source_id: Uuid,
    friendly_name: &str,
    source_type: &str,
    configuration: Value,
) -> Result<SourceRecord, ResourceUpdateError> {
    let mut transaction = pool.begin().await?;
    let desired_state = sqlx::query_scalar::<_, String>(
        "SELECT desired_state FROM sources WHERE id = $1 FOR UPDATE",
    )
    .bind(source_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ResourceUpdateError::NotFound)?;

    if desired_state != "disabled" {
        return Err(ResourceUpdateError::Enabled);
    }

    let revision = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(revision), 0) + 1 FROM source_revisions WHERE source_id = $1",
    )
    .bind(source_id)
    .fetch_one(&mut *transaction)
    .await?;
    let revision_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO source_revisions (id, source_id, revision, lifecycle, configuration) \
         VALUES ($1, $2, $3, 'draft', $4)",
    )
    .bind(revision_id)
    .bind(source_id)
    .bind(revision)
    .bind(&configuration)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "UPDATE sources \
         SET friendly_name = $2, source_type = $3, current_revision = $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(source_id)
    .bind(friendly_name)
    .bind(source_type)
    .bind(revision)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(SourceRecord {
        id: source_id,
        friendly_name: friendly_name.to_owned(),
        source_type: source_type.to_owned(),
        desired_state,
        current_revision: revision,
        revision_id,
        lifecycle: "draft".to_owned(),
        configuration,
    })
}

pub async fn validate_source_draft(
    pool: &PgPool,
    source_id: Uuid,
) -> Result<SourceRecord, DraftValidationError> {
    let mut transaction = pool.begin().await?;
    let mut source = sqlx::query_as::<_, SourceRecord>(
        "SELECT s.id, s.friendly_name, s.source_type, s.desired_state, \
         s.current_revision, r.id AS revision_id, r.lifecycle, r.configuration \
         FROM sources s \
         JOIN source_revisions r \
           ON r.source_id = s.id AND r.revision = s.current_revision \
         WHERE s.id = $1 \
         FOR UPDATE OF s, r",
    )
    .bind(source_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(DraftValidationError::NotFound)?;

    if source.lifecycle != "draft" {
        return Err(DraftValidationError::NotDraft);
    }

    sqlx::query(
        "UPDATE source_revisions SET lifecycle = 'validated' \
         WHERE id = $1 AND lifecycle = 'draft'",
    )
    .bind(source.revision_id)
    .execute(&mut *transaction)
    .await?;
    source.lifecycle = "validated".to_owned();
    transaction.commit().await?;

    Ok(source)
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

pub async fn update_destination(
    pool: &PgPool,
    destination_id: Uuid,
    friendly_name: &str,
    destination_type: &str,
    configuration: Value,
) -> Result<DestinationRecord, ResourceUpdateError> {
    let mut transaction = pool.begin().await?;
    let desired_state = sqlx::query_scalar::<_, String>(
        "SELECT desired_state FROM destinations WHERE id = $1 FOR UPDATE",
    )
    .bind(destination_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ResourceUpdateError::NotFound)?;

    if desired_state != "disabled" {
        return Err(ResourceUpdateError::Enabled);
    }

    let revision = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(revision), 0) + 1 \
         FROM destination_revisions WHERE destination_id = $1",
    )
    .bind(destination_id)
    .fetch_one(&mut *transaction)
    .await?;
    let revision_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO destination_revisions (id, destination_id, revision, lifecycle, configuration) \
         VALUES ($1, $2, $3, 'draft', $4)",
    )
    .bind(revision_id)
    .bind(destination_id)
    .bind(revision)
    .bind(&configuration)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "UPDATE destinations \
         SET friendly_name = $2, destination_type = $3, current_revision = $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(destination_id)
    .bind(friendly_name)
    .bind(destination_type)
    .bind(revision)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(DestinationRecord {
        id: destination_id,
        friendly_name: friendly_name.to_owned(),
        destination_type: destination_type.to_owned(),
        desired_state,
        current_revision: revision,
        revision_id,
        lifecycle: "draft".to_owned(),
        configuration,
    })
}

pub async fn validate_destination_draft(
    pool: &PgPool,
    destination_id: Uuid,
) -> Result<DestinationRecord, DraftValidationError> {
    let mut transaction = pool.begin().await?;
    let mut destination = sqlx::query_as::<_, DestinationRecord>(
        "SELECT d.id, d.friendly_name, d.destination_type, d.desired_state, \
         d.current_revision, r.id AS revision_id, r.lifecycle, r.configuration \
         FROM destinations d \
         JOIN destination_revisions r \
           ON r.destination_id = d.id AND r.revision = d.current_revision \
         WHERE d.id = $1 \
         FOR UPDATE OF d, r",
    )
    .bind(destination_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(DraftValidationError::NotFound)?;

    if destination.lifecycle != "draft" {
        return Err(DraftValidationError::NotDraft);
    }

    sqlx::query(
        "UPDATE destination_revisions SET lifecycle = 'validated' \
         WHERE id = $1 AND lifecycle = 'draft'",
    )
    .bind(destination.revision_id)
    .execute(&mut *transaction)
    .await?;
    destination.lifecycle = "validated".to_owned();
    transaction.commit().await?;

    Ok(destination)
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

        assert!(matches!(
            validate_source_draft(&pool, Uuid::new_v4()).await,
            Err(DraftValidationError::NotFound)
        ));
        assert!(matches!(
            validate_destination_draft(&pool, Uuid::new_v4()).await,
            Err(DraftValidationError::NotFound)
        ));
        assert!(matches!(
            update_source(&pool, Uuid::new_v4(), "missing", "rtp_l24", json!({})).await,
            Err(ResourceUpdateError::NotFound)
        ));
        assert!(matches!(
            update_destination(&pool, Uuid::new_v4(), "missing", "rtp_l24", json!({})).await,
            Err(ResourceUpdateError::NotFound)
        ));

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
        let validated_source = validate_source_draft(&pool, source.id)
            .await
            .expect("source draft transitions to validated");
        assert_eq!(validated_source.lifecycle, "validated");
        assert!(list_sources(&pool)
            .await
            .expect("sources can be listed after validation")
            .iter()
            .any(|listed| listed.id == source.id && listed.lifecycle == "validated"));
        assert!(matches!(
            validate_source_draft(&pool, source.id).await,
            Err(DraftValidationError::NotDraft)
        ));
        let updated_source_configuration = json!({
            "kind": "rtp_l24",
            "bindAddress": "0.0.0.0",
            "port": 5002,
            "payloadType": 96
        });
        let updated_source = update_source(
            &pool,
            source.id,
            "Updated RTP source",
            "rtp_l24",
            updated_source_configuration.clone(),
        )
        .await
        .expect("editing a source creates a new revision");
        assert_eq!(updated_source.id, source.id);
        assert_eq!(updated_source.friendly_name, "Updated RTP source");
        assert_eq!(updated_source.desired_state, "disabled");
        assert_eq!(updated_source.current_revision, 2);
        assert_ne!(updated_source.revision_id, source.revision_id);
        assert_eq!(updated_source.lifecycle, "draft");
        assert_eq!(updated_source.configuration, updated_source_configuration);
        let prior_source_lifecycle: String = sqlx::query_scalar(
            "SELECT lifecycle FROM source_revisions WHERE source_id = $1 AND revision = 1",
        )
        .bind(source.id)
        .fetch_one(&pool)
        .await
        .expect("prior source revision remains available");
        assert_eq!(prior_source_lifecycle, "validated");
        sqlx::query("UPDATE sources SET desired_state = 'enabled' WHERE id = $1")
            .bind(source.id)
            .execute(&pool)
            .await
            .expect("source can be enabled for the update guard check");
        assert!(matches!(
            update_source(&pool, source.id, "Enabled source", "rtp_l24", json!({})).await,
            Err(ResourceUpdateError::Enabled)
        ));

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
        let validated_destination = validate_destination_draft(&pool, destination.id)
            .await
            .expect("destination draft transitions to validated");
        assert_eq!(validated_destination.lifecycle, "validated");
        assert!(list_destinations(&pool)
            .await
            .expect("destinations can be listed after validation")
            .iter()
            .any(|listed| listed.id == destination.id && listed.lifecycle == "validated"));
        assert!(matches!(
            validate_destination_draft(&pool, destination.id).await,
            Err(DraftValidationError::NotDraft)
        ));
        let updated_destination_configuration = json!({
            "kind": "rtp_l24",
            "address": "127.0.0.1",
            "port": 6002,
            "payloadType": 97
        });
        let updated_destination = update_destination(
            &pool,
            destination.id,
            "Updated RTP destination",
            "rtp_l24",
            updated_destination_configuration.clone(),
        )
        .await
        .expect("editing a destination creates a new revision");
        assert_eq!(updated_destination.id, destination.id);
        assert_eq!(updated_destination.friendly_name, "Updated RTP destination");
        assert_eq!(updated_destination.desired_state, "disabled");
        assert_eq!(updated_destination.current_revision, 2);
        assert_ne!(updated_destination.revision_id, destination.revision_id);
        assert_eq!(updated_destination.lifecycle, "draft");
        assert_eq!(
            updated_destination.configuration,
            updated_destination_configuration
        );
        let prior_destination_lifecycle: String = sqlx::query_scalar(
            "SELECT lifecycle FROM destination_revisions \
             WHERE destination_id = $1 AND revision = 1",
        )
        .bind(destination.id)
        .fetch_one(&pool)
        .await
        .expect("prior destination revision remains available");
        assert_eq!(prior_destination_lifecycle, "validated");
        sqlx::query("UPDATE destinations SET desired_state = 'enabled' WHERE id = $1")
            .bind(destination.id)
            .execute(&pool)
            .await
            .expect("destination can be enabled for the update guard check");
        assert!(matches!(
            update_destination(
                &pool,
                destination.id,
                "Enabled destination",
                "rtp_l24",
                json!({})
            )
            .await,
            Err(ResourceUpdateError::Enabled)
        ));

        sqlx::query("DELETE FROM sources WHERE id = $1")
            .bind(source.id)
            .execute(&pool)
            .await
            .expect("test source and its revisions are removed");
        sqlx::query("DELETE FROM destinations WHERE id = $1")
            .bind(destination.id)
            .execute(&pool)
            .await
            .expect("test destination and its revisions are removed");
    }
}
