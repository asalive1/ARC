use arc_domain::{
    estimate_pipeline_requirements, DestinationMediaProfile, MediaProfileError, SourceMediaProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

const ROUTE_GLOBAL_REVISION_LOCK_ID: i64 = 0x415243;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum RouteAction {
    #[default]
    Start,
    Stop,
}

impl RouteAction {
    fn is_start(&self) -> bool {
        *self == Self::Start
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

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
    #[serde(default, skip_serializing_if = "RouteAction::is_start")]
    pub action: RouteAction,
}

#[derive(Clone, Debug, Deserialize, FromRow, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetRoute {
    pub source_id: Uuid,
    pub destination_id: Uuid,
    #[serde(default, skip_serializing_if = "RouteAction::is_start")]
    pub action: RouteAction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: Uuid,
    pub name: String,
    pub revision: i64,
    pub routes: Vec<PresetRoute>,
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
    #[serde(default, skip_serializing_if = "RouteAction::is_start")]
    pub action: RouteAction,
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
    pub action: String,
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
    PresetNotFound,
    PresetConflict,
    InvalidPreset,
    PresetLimit,
    DestinationBusy,
    RouteNotFound,
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

    pub async fn list_presets(&self) -> Result<Vec<Preset>, RouteWriteError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
            "SELECT id, name, revision FROM route_presets ORDER BY name, id",
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut presets = Vec::with_capacity(rows.len());
        for (id, name, revision) in rows {
            let routes = sqlx::query_as::<_, PresetRoute>(
                "SELECT source_id, destination_id, action FROM preset_routes WHERE preset_id = $1 ORDER BY destination_id",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            presets.push(Preset {
                id,
                name,
                revision,
                routes,
            });
        }
        tx.commit().await?;
        Ok(presets)
    }

    pub async fn save_preset(
        &self,
        id: Option<Uuid>,
        expected_revision: Option<i64>,
        name: String,
        mut routes: Vec<PresetRoute>,
    ) -> Result<Preset, RouteWriteError> {
        let name = name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 100 || routes.is_empty() || routes.len() > 64 {
            return Err(RouteWriteError::InvalidPreset);
        }
        routes.sort_by_key(|route| route.destination_id);
        if routes
            .windows(2)
            .any(|pair| pair[0].destination_id == pair[1].destination_id)
        {
            return Err(RouteWriteError::InvalidPreset);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(0x41524350_i64)
            .execute(&mut *tx)
            .await?;
        let (id, revision) = if let Some(id) = id {
            let revision = sqlx::query_scalar::<_, i64>(
                "SELECT revision FROM route_presets WHERE id = $1 FOR UPDATE",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RouteWriteError::PresetNotFound)?;
            if expected_revision != Some(revision) {
                return Err(RouteWriteError::PresetConflict);
            }
            (id, revision + 1)
        } else {
            let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM route_presets")
                .fetch_one(&mut *tx)
                .await?;
            if count >= 100 {
                return Err(RouteWriteError::PresetLimit);
            }
            (Uuid::new_v4(), 1)
        };
        let mut sources: Vec<_> = routes.iter().map(|route| route.source_id).collect();
        sources.sort_unstable();
        sources.dedup();
        sqlx::query("SELECT id FROM sources WHERE id = ANY($1) ORDER BY id FOR SHARE")
            .bind(&sources)
            .fetch_all(&mut *tx)
            .await?;
        let destinations: Vec<_> = routes.iter().map(|route| route.destination_id).collect();
        sqlx::query("SELECT id FROM destinations WHERE id = ANY($1) ORDER BY id FOR SHARE")
            .bind(&destinations)
            .fetch_all(&mut *tx)
            .await?;
        for route in &routes {
            Self::validate_preset_route(&mut tx, route).await?;
        }
        let duplicate = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM route_presets WHERE name = $1 AND id <> $2)",
        )
        .bind(&name)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            return Err(RouteWriteError::PresetConflict);
        }
        if revision == 1 {
            sqlx::query("INSERT INTO route_presets (id, name, revision) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(&name)
                .bind(revision)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE route_presets SET name = $2, revision = $3 WHERE id = $1")
                .bind(id)
                .bind(&name)
                .bind(revision)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM preset_routes WHERE preset_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        for route in &routes {
            sqlx::query("INSERT INTO preset_routes (preset_id, source_id, destination_id, action) VALUES ($1, $2, $3, $4)")
                .bind(id).bind(route.source_id).bind(route.destination_id).bind(route.action.as_str()).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(Preset {
            id,
            name,
            revision,
            routes,
        })
    }

    pub async fn delete_preset(
        &self,
        id: Uuid,
        expected_revision: i64,
    ) -> Result<(), RouteWriteError> {
        let mut tx = self.pool.begin().await?;
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT revision FROM route_presets WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RouteWriteError::PresetNotFound)?;
        if revision != expected_revision {
            return Err(RouteWriteError::PresetConflict);
        }
        sqlx::query("DELETE FROM route_presets WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
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

    async fn validate_preset_route(
        tx: &mut Transaction<'_, Postgres>,
        route: &PresetRoute,
    ) -> Result<(), RouteWriteError> {
        let source = sqlx::query_as::<_, (String, Value)>(
            "SELECT r.lifecycle, r.configuration FROM sources s JOIN source_revisions r ON r.source_id = s.id AND r.revision = s.current_revision WHERE s.id = $1 FOR SHARE OF s, r",
        ).bind(route.source_id).fetch_optional(&mut **tx).await?.ok_or(RouteWriteError::SourceNotFound)?;
        let destination = sqlx::query_as::<_, (String, Value)>(
            "SELECT r.lifecycle, r.configuration FROM destinations d JOIN destination_revisions r ON r.destination_id = d.id AND r.revision = d.current_revision WHERE d.id = $1 FOR SHARE OF d, r",
        ).bind(route.destination_id).fetch_optional(&mut **tx).await?.ok_or(RouteWriteError::DestinationNotFound)?;
        if route.action == RouteAction::Stop {
            return Ok(());
        }
        if source.0 != "validated" {
            return Err(RouteWriteError::SourceNotValidated);
        }
        if destination.0 != "validated" {
            return Err(RouteWriteError::DestinationNotValidated);
        }
        let source: SourceMediaProfile =
            serde_json::from_value(source.1).map_err(RouteWriteError::InvalidStoredProfile)?;
        let destination: DestinationMediaProfile =
            serde_json::from_value(destination.1).map_err(RouteWriteError::InvalidStoredProfile)?;
        estimate_pipeline_requirements(&source, &destination)
            .map_err(RouteWriteError::InvalidMediaProfile)?;
        Ok(())
    }

    pub async fn set_desired_route(
        &self,
        source_id: Uuid,
        destination_id: Uuid,
        expected_route_revision: i64,
        confirm_replace: bool,
    ) -> Result<RouteRecord, RouteWriteError> {
        let mut transaction = self.pool.begin().await?;
        lock_destination(&mut transaction, destination_id).await?;
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
        changes: Vec<DesiredRouteChange>,
    ) -> Result<RouteBatchOperation, RouteWriteError> {
        self.admit_batch(idempotency_key, changes, None).await
    }

    pub async fn unroute(
        &self,
        key: Uuid,
        destination_id: Uuid,
        expected_revision: i64,
    ) -> Result<RouteBatchOperation, RouteWriteError> {
        if key.is_nil() {
            return Err(RouteWriteError::InvalidIdempotencyKey);
        }
        if expected_revision < 0 {
            return Err(RouteWriteError::InvalidBatch);
        }
        let request = json!({"kind": "unroute", "destinationId": destination_id, "expectedRouteRevision": expected_revision});
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key.to_string())
            .execute(&mut *tx)
            .await?;
        if let Some((stored, response)) = sqlx::query_as::<_, (Value, Value)>(
            "SELECT request, response FROM route_batch_operations WHERE idempotency_key = $1 FOR UPDATE",
        ).bind(key).fetch_optional(&mut *tx).await? {
            if stored != request { return Err(RouteWriteError::IdempotencyKeyReused); }
            let operation: RouteBatchOperation = serde_json::from_value(response).map_err(RouteWriteError::InvalidStoredOperation)?;
            tx.commit().await?;
            return self.get_route_batch_operation(operation.operation_id).await?
                .ok_or(RouteWriteError::Database(sqlx::Error::RowNotFound));
        }
        sqlx::query("SET LOCAL lock_timeout = '10s'")
            .execute(&mut *tx)
            .await?;
        if let Err(error) = lock_destination(&mut tx, destination_id).await {
            if matches!(&error, sqlx::Error::Database(db) if db.code().as_deref() == Some("55P03"))
            {
                return Err(RouteWriteError::DestinationBusy);
            }
            return Err(RouteWriteError::Database(error));
        }
        sqlx::query("SET LOCAL lock_timeout = '0'")
            .execute(&mut *tx)
            .await?;
        let (route, _) = Self::remove_desired_route_in_transaction(
            &mut tx,
            destination_id,
            expected_revision,
            None,
        )
        .await?;
        let source_id = route.source_id;
        let source_revision =
            sqlx::query_scalar::<_, i64>("SELECT current_revision FROM sources WHERE id = $1")
                .bind(source_id)
                .fetch_one(&mut *tx)
                .await?;
        let destination_revision =
            sqlx::query_scalar::<_, i64>("SELECT current_revision FROM destinations WHERE id = $1")
                .bind(destination_id)
                .fetch_one(&mut *tx)
                .await?;
        let revision = route.revision;
        let operation_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v5(&operation_id, destination_id.as_bytes());
        let planning_key = Uuid::new_v5(
            &operation_id,
            format!("route-stop:{destination_id}").as_bytes(),
        );
        let result = RouteBatchItemResult {
            source_id,
            destination_id,
            pipeline_id,
            state: "pending".to_owned(),
            assignment_id: None,
            assignment_generation: None,
            error_code: None,
            error_message: None,
            action: RouteAction::Stop,
        };
        let operation = RouteBatchOperation {
            operation_id,
            state: "pending".to_owned(),
            routes: vec![route],
            results: vec![result],
        };
        let response =
            serde_json::to_value(&operation).map_err(RouteWriteError::InvalidStoredOperation)?;
        sqlx::query("INSERT INTO route_batch_operations (id, idempotency_key, request, response, execution_state) VALUES ($1, $2, $3, $4, 'pending')")
            .bind(operation_id).bind(key).bind(request).bind(response).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO route_batch_items (operation_id, source_id, destination_id, source_revision, destination_revision, route_revision, pipeline_id, planning_key, action) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'stop')")
            .bind(operation_id).bind(source_id).bind(destination_id).bind(source_revision).bind(destination_revision).bind(revision).bind(pipeline_id).bind(planning_key).execute(&mut *tx).await?;
        Self::capture_stop_targets(&mut tx, operation_id, destination_id).await?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn remove_desired_route_in_transaction(
        tx: &mut Transaction<'_, Postgres>,
        destination_id: Uuid,
        expected_revision: i64,
        fallback_source: Option<Uuid>,
    ) -> Result<(RouteRecord, bool), RouteWriteError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT current_revision FROM destinations WHERE id = $1 FOR UPDATE",
        )
        .bind(destination_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RouteWriteError::DestinationNotFound)?;
        ensure_destination_not_executing(tx, destination_id).await?;
        let current = sqlx::query_as::<_, (Uuid, Uuid, i64, String)>(
            "SELECT id, source_id, route_revision, desired_state FROM route_instances \
             WHERE destination_id = $1 ORDER BY route_revision DESC LIMIT 1 FOR UPDATE",
        )
        .bind(destination_id)
        .fetch_optional(&mut **tx)
        .await?;
        let current_revision = current.as_ref().map_or(0, |row| row.2);
        if current_revision != expected_revision {
            return Err(RouteWriteError::StaleRouteRevision {
                expected: expected_revision,
                current: current_revision,
            });
        }
        let assigned_source = sqlx::query_scalar::<_, Uuid>(
            "SELECT source_id FROM pipeline_assignments WHERE destination_id = $1 \
             AND resource_type = 'pipeline' AND desired_state IN ('planned', 'running', 'stopped') \
             AND observed_state NOT IN ('stopped', 'cancelled') ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(destination_id)
        .fetch_optional(&mut **tx)
        .await?;
        let observation = latest_observation(tx, destination_id).await?;
        let already_stopped = fallback_source.is_some()
            && assigned_source.is_none()
            && (current.is_none()
                || (current.as_ref().is_some_and(|row| row.3 == "removed")
                    && observation
                        .as_ref()
                        .is_some_and(|(_, state)| state == "stopped")));
        let source_id = current
            .as_ref()
            .map(|row| row.1)
            .or(assigned_source)
            .or(fallback_source)
            .ok_or(RouteWriteError::RouteNotFound)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(ROUTE_GLOBAL_REVISION_LOCK_ID)
            .execute(&mut **tx)
            .await?;
        let global_revision = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(global_revision), 0) + 1 FROM route_instances",
        )
        .fetch_one(&mut **tx)
        .await?;
        if let Some((id, _, _, desired)) = current {
            if desired == "desired" {
                sqlx::query("UPDATE route_instances SET desired_state = 'removed' WHERE id = $1")
                    .bind(id)
                    .execute(&mut **tx)
                    .await?;
            }
        }
        let id = Uuid::new_v4();
        let revision = current_revision + 1;
        sqlx::query(
            "INSERT INTO route_instances (id, source_id, destination_id, route_revision, global_revision, desired_state) \
             VALUES ($1, $2, $3, $4, $5, 'removed')",
        ).bind(id).bind(source_id).bind(destination_id).bind(revision).bind(global_revision)
            .execute(&mut **tx).await?;
        if already_stopped {
            sqlx::query(
                "INSERT INTO route_events (id, route_instance_id, observed_state, details) \
                 VALUES ($1, $2, 'stopped', '{\"reason\":\"already_unrouted\"}')",
            )
            .bind(Uuid::new_v4())
            .bind(id)
            .execute(&mut **tx)
            .await?;
        }
        Ok((
            RouteRecord {
                source_id,
                destination_id,
                revision,
                desired_state: "removed".to_owned(),
                observed_state: if already_stopped {
                    "stopped".to_owned()
                } else {
                    observation
                        .as_ref()
                        .map_or_else(|| "unknown".to_owned(), |(_, state)| state.clone())
                },
                observed_source_id: if already_stopped {
                    Some(source_id)
                } else {
                    observation.map(|(source, _)| source)
                },
            },
            already_stopped,
        ))
    }

    async fn capture_stop_targets(
        tx: &mut Transaction<'_, Postgres>,
        operation_id: Uuid,
        destination_id: Uuid,
    ) -> Result<(), RouteWriteError> {
        sqlx::query(
            "INSERT INTO route_stop_targets (operation_id, assignment_id, generation) \
             SELECT $1, id, generation FROM pipeline_assignments \
             WHERE destination_id = $2 AND resource_type = 'pipeline' \
               AND ((desired_state = 'planned' AND observed_state = 'not_started') \
                 OR (desired_state = 'running' AND observed_state IN ('starting', 'running', 'failed', 'degraded', 'unknown')) \
                 OR (desired_state = 'stopped' AND observed_state = 'stopping'))",
        ).bind(operation_id).bind(destination_id).execute(&mut **tx).await?;
        Ok(())
    }

    pub async fn execute_preset(
        &self,
        idempotency_key: Uuid,
        changes: Vec<DesiredRouteChange>,
        preset_id: Uuid,
        revision: i64,
    ) -> Result<RouteBatchOperation, RouteWriteError> {
        self.admit_batch(idempotency_key, changes, Some((preset_id, revision)))
            .await
    }

    async fn admit_batch(
        &self,
        idempotency_key: Uuid,
        mut changes: Vec<DesiredRouteChange>,
        preset: Option<(Uuid, i64)>,
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

        let request = match preset {
            Some((id, revision)) => {
                json!({ "kind": "preset", "presetId": id, "expectedRevision": revision, "routes": changes })
            }
            None => json!({ "routes": changes }),
        };
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

        if let Some((preset_id, expected_revision)) = preset {
            let revision = sqlx::query_scalar::<_, i64>(
                "SELECT revision FROM route_presets WHERE id = $1 FOR SHARE",
            )
            .bind(preset_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(RouteWriteError::PresetNotFound)?;
            if revision != expected_revision {
                return Err(RouteWriteError::PresetConflict);
            }
            let saved = sqlx::query_as::<_, PresetRoute>(
                "SELECT source_id, destination_id, action FROM preset_routes WHERE preset_id = $1 ORDER BY destination_id",
            )
            .bind(preset_id)
            .fetch_all(&mut *transaction)
            .await?;
            if saved.len() != changes.len()
                || saved.iter().zip(&changes).any(|(saved, change)| {
                    saved.source_id != change.source_id
                        || saved.destination_id != change.destination_id
                        || saved.action != change.action
                })
            {
                return Err(RouteWriteError::PresetConflict);
            }
        }

        for change in &changes {
            lock_destination(&mut transaction, change.destination_id).await?;
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
            let stopping = change.action == RouteAction::Stop;
            let replacement_assignment_id = if stopping {
                None
            } else {
                active_assignment_for_destination(&mut transaction, change.destination_id).await?
            };
            let (route, already_stopped) = if stopping {
                Self::remove_desired_route_in_transaction(
                    &mut transaction,
                    change.destination_id,
                    change.expected_route_revision,
                    Some(change.source_id),
                )
                .await?
            } else {
                (
                    Self::set_desired_route_in_transaction(
                        &mut transaction,
                        change.source_id,
                        change.destination_id,
                        change.expected_route_revision,
                        change.confirm_replace,
                    )
                    .await?,
                    false,
                )
            };
            let source_revision =
                sqlx::query_scalar::<_, i64>("SELECT current_revision FROM sources WHERE id = $1")
                    .bind(route.source_id)
                    .fetch_one(&mut *transaction)
                    .await?;
            let destination_revision = sqlx::query_scalar::<_, i64>(
                "SELECT current_revision FROM destinations WHERE id = $1",
            )
            .bind(change.destination_id)
            .fetch_one(&mut *transaction)
            .await?;
            let running = if !stopping
                && route.revision == change.expected_route_revision
                && route.observed_state == "running"
                && route.observed_source_id == Some(change.source_id)
            {
                let busy: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM route_batch_items \
                     WHERE destination_id = $1 AND state IN ('pending', 'processing'))",
                )
                .bind(change.destination_id)
                .fetch_one(&mut *transaction)
                .await?;
                if busy {
                    return Err(RouteWriteError::DestinationBusy);
                }
                sqlx::query_as::<_, (Uuid, i64, Uuid)>(
                    "SELECT id, generation, resource_id FROM pipeline_assignments \
                     WHERE id = $1 AND resource_type = 'pipeline' \
                       AND source_id = $2 AND source_revision = $3 \
                       AND destination_revision = $4 \
                       AND desired_state = 'running' AND observed_state = 'running' FOR UPDATE",
                )
                .bind(replacement_assignment_id)
                .bind(change.source_id)
                .bind(source_revision)
                .bind(destination_revision)
                .fetch_optional(&mut *transaction)
                .await?
            } else {
                None
            };
            let pipeline_id = Uuid::new_v5(&operation_id, change.destination_id.as_bytes());
            let planning_key = Uuid::new_v5(
                &operation_id,
                format!("route-plan:{}", change.destination_id).as_bytes(),
            );
            let result = RouteBatchItemResult {
                source_id: route.source_id,
                destination_id: change.destination_id,
                pipeline_id: running.map_or(pipeline_id, |(_, _, pipeline)| pipeline),
                state: if running.is_some() || already_stopped {
                    "succeeded"
                } else {
                    "pending"
                }
                .to_owned(),
                assignment_id: running.map(|(id, _, _)| id),
                assignment_generation: running.map(|(_, generation, _)| generation),
                error_code: None,
                error_message: None,
                action: change.action,
            };
            items.push((
                source_revision,
                destination_revision,
                route.revision,
                replacement_assignment_id,
                pipeline_id,
                planning_key,
                result.clone(),
            ));
            results.push(result);
            routes.push(route);
        }
        let state = if results.iter().all(|result| result.state == "succeeded") {
            "completed"
        } else if results.iter().any(|result| result.state == "succeeded") {
            "running"
        } else {
            "pending"
        };
        let operation = RouteBatchOperation {
            operation_id,
            state: state.to_owned(),
            routes,
            results,
        };
        let response =
            serde_json::to_value(&operation).map_err(RouteWriteError::InvalidStoredOperation)?;
        sqlx::query(
            "INSERT INTO route_batch_operations \
             (id, idempotency_key, request, response, execution_state) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(operation.operation_id)
        .bind(idempotency_key)
        .bind(request)
        .bind(response)
        .bind(state)
        .execute(&mut *transaction)
        .await?;
        for (
            source_revision,
            destination_revision,
            route_revision,
            replacement_assignment_id,
            pipeline_id,
            planning_key,
            result,
        ) in items
        {
            sqlx::query(
                "INSERT INTO route_batch_items \
                 (operation_id, source_id, destination_id, source_revision, \
                  destination_revision, route_revision, replacement_assignment_id, \
                  pipeline_id, planning_key, state, assignment_id, assignment_generation, action) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(operation_id)
            .bind(result.source_id)
            .bind(result.destination_id)
            .bind(source_revision)
            .bind(destination_revision)
            .bind(route_revision)
            .bind(replacement_assignment_id)
            .bind(pipeline_id)
            .bind(planning_key)
            .bind(&result.state)
            .bind(result.assignment_id)
            .bind(result.assignment_generation)
            .bind(result.action.as_str())
            .execute(&mut *transaction)
            .await?;
            if result.action == RouteAction::Stop {
                Self::capture_stop_targets(&mut transaction, operation_id, result.destination_id)
                    .await?;
            }
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
            "SELECT item.source_id, item.destination_id, \
             CASE WHEN item.state = 'succeeded' \
                  THEN COALESCE(assignment.resource_id, item.pipeline_id) \
                  ELSE item.pipeline_id END AS pipeline_id, \
             item.state, item.assignment_id, item.assignment_generation, item.error_code, item.error_message, item.action \
             FROM route_batch_items AS item \
             LEFT JOIN pipeline_assignments AS assignment ON assignment.id = item.assignment_id \
             WHERE item.operation_id = $1 ORDER BY item.destination_id",
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
             item.error_code, item.error_message, item.attempts, item.lease_token, item.action",
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
            "SELECT EXISTS(SELECT 1 FROM route_instances r \
             JOIN route_batch_items item ON item.operation_id = $5 AND item.destination_id = r.destination_id \
             WHERE r.destination_id = $1 AND r.source_id = $2 \
               AND r.route_revision = $3 AND r.desired_state = CASE WHEN item.action = 'stop' THEN 'removed' ELSE 'desired' END \
               AND r.route_revision = (SELECT MAX(route_revision) FROM route_instances WHERE destination_id = $1) \
               AND item.state = 'processing' AND item.lease_token = $4)",
        )
        .bind(item.destination_id)
        .bind(item.source_id)
        .bind(item.route_revision)
        .bind(item.lease_token)
        .bind(item.operation_id)
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

        ensure_destination_not_executing(transaction, destination_id).await?;
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

async fn ensure_destination_not_executing(
    transaction: &mut Transaction<'_, Postgres>,
    destination_id: Uuid,
) -> Result<(), RouteWriteError> {
    let busy: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM route_batch_items WHERE destination_id = $1 \
         AND state IN ('pending', 'processing') AND attempts > 0)",
    )
    .bind(destination_id)
    .fetch_one(&mut **transaction)
    .await?;
    if busy {
        return Err(RouteWriteError::DestinationBusy);
    }
    Ok(())
}

pub async fn lock_destination(
    transaction: &mut Transaction<'_, Postgres>,
    destination_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("arc-route-destination:{destination_id}"))
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub async fn destination_is_removed(
    transaction: &mut Transaction<'_, Postgres>,
    destination_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT desired_state = 'removed' FROM route_instances \
         WHERE destination_id = $1 ORDER BY route_revision DESC LIMIT 1), FALSE)",
    )
    .bind(destination_id)
    .fetch_one(&mut **transaction)
    .await
}

async fn active_assignment_for_destination(
    transaction: &mut Transaction<'_, Postgres>,
    destination_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM pipeline_assignments \
         WHERE resource_type = 'pipeline' AND destination_id = $1 \
           AND ((desired_state = 'running' \
                 AND observed_state IN ('starting', 'running', 'failed', 'degraded', 'unknown')) \
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
