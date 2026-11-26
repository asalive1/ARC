use arc_domain::{
    estimate_pipeline_requirements, DestinationMediaProfile, MediaProfileError, SourceMediaProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

const ROUTE_GLOBAL_REVISION_LOCK_ID: i64 = 0x415243;

#[derive(Clone, Debug, Deserialize, FromRow, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteRecord {
    pub destination_id: Uuid,
    pub source_id: Uuid,
    pub desired_state: String,
    pub observed_state: String,
    pub observed_source_id: Option<Uuid>,
    pub revision: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesiredRouteChange {
    pub source_id: Uuid,
    pub destination_id: Uuid,
    pub expected_route_revision: i64,
    pub confirm_replace: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBatchOperation {
    pub operation_id: Uuid,
    pub state: String,
    pub routes: Vec<RouteRecord>,
    #[serde(default)]
    pub results: Vec<RouteBatchItemResult>,
}

#[derive(Clone, Debug, Deserialize, FromRow, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBatchItemResult {
    pub source_id: Uuid,
    pub destination_id: Uuid,
    pub pipeline_id: Uuid,
    pub state: String,
    pub assignment_id: Option<Uuid>,
    pub assignment_generation: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, FromRow)]
pub struct RouteBatchWorkItem {
    pub operation_id: Uuid,
    pub source_id: Uuid,
    pub destination_id: Uuid,
    pub source_revision: i64,
    pub destination_revision: i64,
    pub route_revision: i64,
    pub replacement_assignment_id: Option<Uuid>,
    pub pipeline_id: Uuid,
    pub planning_key: Uuid,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub attempts: i32,
    pub lease_token: Uuid,
}

pub enum RouteBatchItemCompletion<'a> {
    Succeeded {
        assignment_id: Uuid,
        generation: i64,
    },
    Failed {
        assignment: Option<(Uuid, i64)>,
        error_code: &'a str,
        error_message: &'a str,
    },
    Retry {
        assignment: Option<(Uuid, i64)>,
        error_code: &'a str,
        error_message: &'a str,
        delay_seconds: i64,
    },
}

#[derive(Debug)]
pub enum RouteWriteError {
    InvalidBatch,
    InvalidIdempotencyKey,
    IdempotencyKeyReused,
    SourceNotFound,
    DestinationNotFound,
    SourceNotValidated,
    DestinationNotValidated,
    StaleRouteRevision { expected: i64, current: i64 },
    ReplacementNotConfirmed,
    InvalidMediaProfile(MediaProfileError),
    InvalidStoredProfile(serde_json::Error),
    InvalidStoredOperation(serde_json::Error),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for RouteWriteError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

pub struct PgRoutes {
    pool: PgPool,
}

impl PgRoutes {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_current(&self) -> Result<Vec<RouteRecord>, sqlx::Error> {
        sqlx::query_as(
            "SELECT DISTINCT ON (route.destination_id) \
                    route.destination_id, \
                    route.source_id, \
                    route.desired_state, \
                    COALESCE(observation.observed_state, 'unknown') AS observed_state, \
                    observation.source_id AS observed_source_id, \
                    route.route_revision AS revision \
             FROM route_instances AS route \
             LEFT JOIN LATERAL ( \
                 SELECT observed_route.source_id, event.observed_state \
                 FROM route_instances AS observed_route \
                 JOIN route_events AS event \
                   ON event.route_instance_id = observed_route.id \
                 WHERE observed_route.destination_id = route.destination_id \
                 ORDER BY event.occurred_at DESC, event.id DESC \
                 LIMIT 1 \
             ) AS observation ON TRUE \
             ORDER BY route.destination_id, route.route_revision DESC",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn set_desired_route(
        &self,
        source_id: Uuid,
        destination_id: Uuid,
        expected_route_revision: i64,
        confirm_replace: bool,
    ) -> Result<RouteRecord, RouteWriteError> {
        let mut transaction = self.pool.begin().await?;
        let route = Self::set_desired_route_in_transaction(
            &mut transaction,
            source_id,
            destination_id,
            expected_route_revision,
            confirm_replace,
        )
        .await?;
        transaction.commit().await?;
        Ok(route)
    }

    pub async fn set_desired_routes_batch(
        &self,
        idempotency_key: Uuid,
        mut changes: Vec<DesiredRouteChange>,
    ) -> Result<RouteBatchOperation, RouteWriteError> {
        if idempotency_key.is_nil() {
            return Err(RouteWriteError::InvalidIdempotencyKey);
        }
        if changes.is_empty()
            || changes.len() > 64
            || changes
                .iter()
                .any(|change| change.expected_route_revision < 0)
        {
            return Err(RouteWriteError::InvalidBatch);
        }
        changes.sort_by_key(|change| change.destination_id);
        if changes
            .windows(2)
            .any(|pair| pair[0].destination_id == pair[1].destination_id)
        {
            return Err(RouteWriteError::InvalidBatch);
        }

        let request = json!({ "routes": changes });
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(idempotency_key.to_string())
            .execute(&mut *transaction)
            .await?;

        let existing = sqlx::query_as::<_, (Value, Value)>(
            "SELECT request, response FROM route_batch_operations \
             WHERE idempotency_key = $1 FOR UPDATE",
        )
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some((stored_request, stored_response)) = existing {
            if stored_request != request {
                return Err(RouteWriteError::IdempotencyKeyReused);
            }
            let operation: RouteBatchOperation = serde_json::from_value(stored_response)
                .map_err(RouteWriteError::InvalidStoredOperation)?;
            transaction.commit().await?;
            return self
                .get_route_batch_operation(operation.operation_id)
                .await?
                .ok_or(RouteWriteError::Database(sqlx::Error::RowNotFound));
        }

        let mut source_ids: Vec<_> = changes.iter().map(|change| change.source_id).collect();
        source_ids.sort_unstable();
        source_ids.dedup();
        sqlx::query(
            "SELECT source.id \
             FROM sources AS source \
             JOIN source_revisions AS revision \
               ON revision.source_id = source.id \
              AND revision.revision = source.current_revision \
             WHERE source.id = ANY($1) \
             ORDER BY source.id \
             FOR SHARE OF source, revision",
        )
        .bind(&source_ids)
        .fetch_all(&mut *transaction)
        .await?;
        let destination_ids: Vec<_> = changes.iter().map(|change| change.destination_id).collect();
        sqlx::query(
            "SELECT destination.id \
             FROM destinations AS destination \
             JOIN destination_revisions AS revision \
               ON revision.destination_id = destination.id \
              AND revision.revision = destination.current_revision \
             WHERE destination.id = ANY($1) \
             ORDER BY destination.id \
             FOR UPDATE OF destination, revision",
        )
        .bind(&destination_ids)
        .fetch_all(&mut *transaction)
        .await?;

        let operation_id = Uuid::new_v4();
        let mut routes = Vec::with_capacity(changes.len());
        let mut results = Vec::with_capacity(changes.len());
        let mut items = Vec::with_capacity(changes.len());
        for change in changes {
            let replacement_assignment_id =
                active_assignment_for_destination(&mut transaction, change.destination_id).await?;
            let route = Self::set_desired_route_in_transaction(
                &mut transaction,
                change.source_id,
                change.destination_id,
                change.expected_route_revision,
                change.confirm_replace,
            )
            .await?;
            let source_revision =
                sqlx::query_scalar::<_, i64>("SELECT current_revision FROM sources WHERE id = $1")
                    .bind(change.source_id)
                    .fetch_one(&mut *transaction)
                    .await?;
            let destination_revision = sqlx::query_scalar::<_, i64>(
                "SELECT current_revision FROM destinations WHERE id = $1",
            )
            .bind(change.destination_id)
            .fetch_one(&mut *transaction)
            .await?;
            let pipeline_id = Uuid::new_v5(&operation_id, change.destination_id.as_bytes());
            let planning_key = Uuid::new_v5(
                &operation_id,
                format!("route-plan:{}", change.destination_id).as_bytes(),
            );
            items.push((
                change.source_id,
                change.destination_id,
                source_revision,
                destination_revision,
                route.revision,
                replacement_assignment_id,
                pipeline_id,
                planning_key,
            ));
            results.push(RouteBatchItemResult {
                source_id: change.source_id,
                destination_id: change.destination_id,
                pipeline_id,
                state: "pending".to_owned(),
                assignment_id: None,
                assignment_generation: None,
                error_code: None,
                error_message: None,
            });
            routes.push(route);
        }
        let operation = RouteBatchOperation {
            operation_id,
            state: "pending".to_owned(),
            routes,
            results,
        };
        let response =
            serde_json::to_value(&operation).map_err(RouteWriteError::InvalidStoredOperation)?;
        sqlx::query(
            "INSERT INTO route_batch_operations \
             (id, idempotency_key, request, response, execution_state) \
             VALUES ($1, $2, $3, $4, 'pending')",
        )
        .bind(operation.operation_id)
        .bind(idempotency_key)
        .bind(request)
        .bind(response)
        .execute(&mut *transaction)
        .await?;
        for (
            source_id,
            destination_id,
            source_revision,
            destination_revision,
            route_revision,
            replacement_assignment_id,
            pipeline_id,
            planning_key,
        ) in items
        {
            sqlx::query(
                "INSERT INTO route_batch_items \
                 (operation_id, source_id, destination_id, source_revision, \
                  destination_revision, route_revision, replacement_assignment_id, \
                  pipeline_id, planning_key) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(operation_id)
            .bind(source_id)
            .bind(destination_id)
            .bind(source_revision)
            .bind(destination_revision)
            .bind(route_revision)
            .bind(replacement_assignment_id)
            .bind(pipeline_id)
            .bind(planning_key)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.get_route_batch_operation(operation.operation_id)
            .await?
            .ok_or(RouteWriteError::Database(sqlx::Error::RowNotFound))
    }

    pub async fn get_route_batch_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<RouteBatchOperation>, RouteWriteError> {
        let row = sqlx::query_as::<_, (String, Value)>(
            "SELECT execution_state, response FROM route_batch_operations WHERE id = $1",
        )
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((state, response)) = row else {
            return Ok(None);
        };
        let mut operation: RouteBatchOperation =
            serde_json::from_value(response).map_err(RouteWriteError::InvalidStoredOperation)?;
        operation.state = state;
        operation.results = sqlx::query_as::<_, RouteBatchItemResult>(
            "SELECT source_id, destination_id, pipeline_id, state, assignment_id, \
             assignment_generation, error_code, error_message \
             FROM route_batch_items WHERE operation_id = $1 ORDER BY destination_id",
        )
        .bind(operation.operation_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(operation))
    }

    pub async fn claim_next_batch_item(
        &self,
    ) -> Result<Option<RouteBatchWorkItem>, RouteWriteError> {
        let lease_token = Uuid::new_v4();
        let mut transaction = self.pool.begin().await?;
        let item = sqlx::query_as::<_, RouteBatchWorkItem>(
            "WITH candidate AS ( \
                 SELECT item.operation_id, item.destination_id \
                 FROM route_batch_items AS item \
                 JOIN route_batch_operations AS operation \
                   ON operation.id = item.operation_id \
                 WHERE operation.execution_state IN ('pending', 'running') \
                   AND ((item.state = 'pending' AND item.next_attempt_at <= clock_timestamp()) \
                     OR (item.state = 'processing' AND item.lease_expires_at <= clock_timestamp())) \
                 ORDER BY operation.created_at, item.destination_id \
                 FOR UPDATE OF item SKIP LOCKED \
                 LIMIT 1 \
             ) \
             UPDATE route_batch_items AS item \
             SET state = 'processing', attempts = item.attempts + 1, \
                 lease_expires_at = clock_timestamp() + INTERVAL '60 seconds', \
                 lease_token = $1 \
             FROM candidate \
             WHERE item.operation_id = candidate.operation_id \
               AND item.destination_id = candidate.destination_id \
             RETURNING item.operation_id, item.source_id, item.destination_id, \
                 item.source_revision, item.destination_revision, item.route_revision, \
             item.replacement_assignment_id, item.pipeline_id, item.planning_key, \
             item.error_code, item.error_message, item.attempts, item.lease_token",
        )
        .bind(lease_token)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(item) = &item {
            sqlx::query(
                "UPDATE route_batch_operations SET execution_state = 'running' \
                 WHERE id = $1 AND execution_state IN ('pending', 'running')",
            )
            .bind(item.operation_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(item)
    }

    pub async fn is_batch_item_current(
        &self,
        item: &RouteBatchWorkItem,
    ) -> Result<bool, RouteWriteError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM route_instances \
             WHERE destination_id = $1 AND source_id = $2 \
               AND route_revision = $3 AND desired_state = 'desired')",
        )
        .bind(item.destination_id)
        .bind(item.source_id)
        .bind(item.route_revision)
        .fetch_one(&self.pool)
        .await
        .map_err(RouteWriteError::Database)
    }

    pub async fn finish_batch_item(
        &self,
        item: &RouteBatchWorkItem,
        completion: RouteBatchItemCompletion<'_>,
    ) -> Result<(), RouteWriteError> {
        let (state, assignment, error_code, error_message, retry_delay) = match completion {
            RouteBatchItemCompletion::Succeeded {
                assignment_id,
                generation,
            } => (
                "succeeded",
                Some((assignment_id, generation)),
                None,
                None,
                None,
            ),
            RouteBatchItemCompletion::Failed {
                assignment,
                error_code,
                error_message,
            } => (
                "failed",
                assignment,
                Some(error_code),
                Some(error_message),
                None,
            ),
            RouteBatchItemCompletion::Retry {
                assignment,
                error_code,
                error_message,
                delay_seconds,
            } => (
                "pending",
                assignment,
                Some(error_code),
                Some(error_message),
                Some(delay_seconds),
            ),
        };
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE route_batch_items SET state = $4, \
                 assignment_id = COALESCE($5, assignment_id), \
                 assignment_generation = COALESCE($6, assignment_generation), \
                 error_code = $7, error_message = $8, \
                 next_attempt_at = CASE WHEN $4 = 'pending' \
                     THEN clock_timestamp() + $9 * INTERVAL '1 second' \
                     ELSE next_attempt_at END, \
                 lease_expires_at = NULL, lease_token = NULL \
             WHERE operation_id = $1 AND destination_id = $2 \
               AND state = 'processing' AND lease_token = $3",
        )
        .bind(item.operation_id)
        .bind(item.destination_id)
        .bind(item.lease_token)
        .bind(state)
        .bind(assignment.map(|(id, _)| id))
        .bind(assignment.map(|(_, generation)| generation))
        .bind(error_code)
        .bind(error_message)
        .bind(retry_delay)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(RouteWriteError::Database(sqlx::Error::RowNotFound));
        }

        sqlx::query(
            "UPDATE route_batch_operations AS operation \
             SET execution_state = CASE \
                 WHEN EXISTS (SELECT 1 FROM route_batch_items WHERE operation_id = $1 \
                              AND state IN ('pending', 'processing')) \
                   THEN CASE WHEN EXISTS (SELECT 1 FROM route_batch_items \
                                          WHERE operation_id = $1 AND state = 'succeeded') \
                             THEN 'running' ELSE 'pending' END \
                 WHEN EXISTS (SELECT 1 FROM route_batch_items WHERE operation_id = $1 \
                              AND state = 'failed') \
                   THEN CASE WHEN EXISTS (SELECT 1 FROM route_batch_items \
                                          WHERE operation_id = $1 AND state = 'succeeded') \
                             THEN 'partial' ELSE 'failed' END \
                 ELSE 'completed' \
             END \
             WHERE operation.id = $1",
        )
        .bind(item.operation_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn set_desired_route_in_transaction(
        transaction: &mut Transaction<'_, Postgres>,
        source_id: Uuid,
        destination_id: Uuid,
        expected_route_revision: i64,
        confirm_replace: bool,
    ) -> Result<RouteRecord, RouteWriteError> {
        let source_configuration = sqlx::query_as::<_, (String, Value)>(
            "SELECT revision.lifecycle, revision.configuration \
             FROM sources AS source \
             JOIN source_revisions AS revision \
               ON revision.source_id = source.id \
              AND revision.revision = source.current_revision \
             WHERE source.id = $1 \
             FOR SHARE OF source, revision",
        )
        .bind(source_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(RouteWriteError::SourceNotFound)?;
        if source_configuration.0 != "validated" {
            return Err(RouteWriteError::SourceNotValidated);
        }

        let destination_configuration = sqlx::query_as::<_, (String, Value)>(
            "SELECT revision.lifecycle, revision.configuration \
             FROM destinations AS destination \
             JOIN destination_revisions AS revision \
               ON revision.destination_id = destination.id \
              AND revision.revision = destination.current_revision \
             WHERE destination.id = $1 \
             FOR UPDATE OF destination, revision",
        )
        .bind(destination_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(RouteWriteError::DestinationNotFound)?;
        if destination_configuration.0 != "validated" {
            return Err(RouteWriteError::DestinationNotValidated);
        }

        let source_profile: SourceMediaProfile = serde_json::from_value(source_configuration.1)
            .map_err(RouteWriteError::InvalidStoredProfile)?;
        let destination_profile: DestinationMediaProfile =
            serde_json::from_value(destination_configuration.1)
                .map_err(RouteWriteError::InvalidStoredProfile)?;
        estimate_pipeline_requirements(&source_profile, &destination_profile)
            .map_err(RouteWriteError::InvalidMediaProfile)?;

        let current = sqlx::query_as::<_, (Uuid, Uuid, String, i64)>(
            "SELECT id, source_id, desired_state, route_revision \
             FROM route_instances \
             WHERE destination_id = $1 \
             ORDER BY route_revision DESC \
             LIMIT 1 \
             FOR UPDATE",
        )
        .bind(destination_id)
        .fetch_optional(&mut **transaction)
        .await?;
        let current_revision = current.as_ref().map_or(0, |route| route.3);
        if expected_route_revision != current_revision {
            return Err(RouteWriteError::StaleRouteRevision {
                expected: expected_route_revision,
                current: current_revision,
            });
        }

        if let Some((_, current_source_id, desired_state, revision)) = &current {
            if desired_state == "desired" && *current_source_id == source_id {
                let observation = latest_observation(transaction, destination_id).await?;
                return Ok(RouteRecord {
                    destination_id,
                    source_id,
                    desired_state: desired_state.clone(),
                    observed_state: observation
                        .as_ref()
                        .map_or_else(|| "unknown".to_owned(), |(_, state)| state.clone()),
                    observed_source_id: observation
                        .map(|(observed_source_id, _)| observed_source_id),
                    revision: *revision,
                });
            }
            if desired_state == "desired" && !confirm_replace {
                return Err(RouteWriteError::ReplacementNotConfirmed);
            }
        }

        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(ROUTE_GLOBAL_REVISION_LOCK_ID)
            .execute(&mut **transaction)
            .await?;
        let global_revision = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(global_revision), 0) + 1 FROM route_instances",
        )
        .fetch_one(&mut **transaction)
        .await?;
        let route_revision = current_revision + 1;
        if let Some((current_id, _, desired_state, _)) = current {
            if desired_state == "desired" {
                sqlx::query("UPDATE route_instances SET desired_state = 'removed' WHERE id = $1")
                    .bind(current_id)
                    .execute(&mut **transaction)
                    .await?;
            }
        }

        sqlx::query(
            "INSERT INTO route_instances \
             (id, source_id, destination_id, route_revision, global_revision, desired_state) \
             VALUES ($1, $2, $3, $4, $5, 'desired')",
        )
        .bind(Uuid::new_v4())
        .bind(source_id)
        .bind(destination_id)
        .bind(route_revision)
        .bind(global_revision)
        .execute(&mut **transaction)
        .await?;
        let observation = latest_observation(transaction, destination_id).await?;

        Ok(RouteRecord {
            destination_id,
            source_id,
            desired_state: "desired".to_owned(),
            observed_state: observation
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), |(_, state)| state.clone()),
            observed_source_id: observation.map(|(observed_source_id, _)| observed_source_id),
            revision: route_revision,
        })
    }
}

async fn active_assignment_for_destination(
    transaction: &mut Transaction<'_, Postgres>,
    destination_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM pipeline_assignments \
         WHERE resource_type = 'pipeline' AND destination_id = $1 \
           AND ((desired_state = 'running' \
                 AND observed_state IN ('starting', 'running', 'failed', 'degraded')) \
             OR (desired_state = 'stopped' AND observed_state = 'stopping')) \
         ORDER BY updated_at DESC, id DESC LIMIT 1",
    )
    .bind(destination_id)
    .fetch_optional(&mut **transaction)
    .await
}

async fn latest_observation(
    transaction: &mut Transaction<'_, Postgres>,
    destination_id: Uuid,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT route.source_id, event.observed_state \
         FROM route_instances AS route \
         JOIN route_events AS event ON event.route_instance_id = route.id \
         WHERE route.destination_id = $1 \
         ORDER BY event.occurred_at DESC, event.id DESC \
         LIMIT 1",
    )
    .bind(destination_id)
    .fetch_optional(&mut **transaction)
    .await
}
