use arc_application::{
    AssignmentGenerationStore, CommandFenceDecision, PortError, PortResult, WorkerCommandIdentity,
};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::worker_registry::error_with;

pub struct PgAssignmentGenerationStore {
    pool: PgPool,
}

impl PgAssignmentGenerationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn validate_assignment(&self, command: WorkerCommandIdentity) -> PortResult<()> {
        let assignment = sqlx::query_as::<_, AssignmentIdentityRow>(
            "SELECT resource_id AS pipeline_id, worker_id, generation \
             FROM pipeline_assignments \
             WHERE id = $1 AND resource_type = 'pipeline'",
        )
        .bind(command.assignment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?
        .ok_or_else(|| PortError {
            code: "assignment_not_found",
            message: "worker command assignment was not found".to_owned(),
        })?;

        if assignment.pipeline_id != command.pipeline_id
            || assignment.worker_id != Some(command.worker_id)
            || assignment.generation != command.generation
        {
            return Err(PortError {
                code: "invalid_command_assignment",
                message: "worker command identity does not match its assignment".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(FromRow)]
struct AssignmentIdentityRow {
    pipeline_id: Uuid,
    worker_id: Option<Uuid>,
    generation: i64,
}

#[derive(FromRow)]
struct FenceRow {
    worker_id: Uuid,
    assignment_id: Uuid,
    generation: i64,
}

#[derive(FromRow)]
struct CommandRow {
    worker_id: Uuid,
    pipeline_id: Uuid,
    assignment_id: Uuid,
    generation: i64,
    status: String,
}

fn validate_command(command: WorkerCommandIdentity) -> PortResult<()> {
    if command.command_id.is_nil()
        || command.worker_id.is_nil()
        || command.pipeline_id.is_nil()
        || command.assignment_id.is_nil()
        || command.generation <= 0
    {
        return Err(PortError {
            code: "invalid_worker_command",
            message: "worker command identities and generation must be valid".to_owned(),
        });
    }
    Ok(())
}

#[async_trait::async_trait]
impl AssignmentGenerationStore for PgAssignmentGenerationStore {
    async fn begin_command(
        &self,
        command: WorkerCommandIdentity,
    ) -> PortResult<CommandFenceDecision> {
        validate_command(command)?;
        self.validate_assignment(command).await?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| error_with("database_error", error))?;
        sqlx::query(
            "INSERT INTO worker_assignment_fences \
             (pipeline_id, worker_id, assignment_id, generation) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (pipeline_id) DO NOTHING",
        )
        .bind(command.pipeline_id)
        .bind(command.worker_id)
        .bind(command.assignment_id)
        .bind(command.generation)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;

        let fence = sqlx::query_as::<_, FenceRow>(
            "SELECT worker_id, assignment_id, generation \
             FROM worker_assignment_fences WHERE pipeline_id = $1 FOR UPDATE",
        )
        .bind(command.pipeline_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;

        if command.generation < fence.generation {
            transaction
                .commit()
                .await
                .map_err(|error| error_with("database_error", error))?;
            return Ok(CommandFenceDecision::Stale {
                current_generation: fence.generation,
            });
        }
        if command.generation == fence.generation
            && (command.worker_id != fence.worker_id
                || command.assignment_id != fence.assignment_id)
        {
            return Err(PortError {
                code: "assignment_generation_conflict",
                message: "one generation cannot identify multiple worker assignments".to_owned(),
            });
        }
        if command.generation > fence.generation {
            sqlx::query(
                "UPDATE worker_assignment_fences \
                 SET worker_id = $2, assignment_id = $3, generation = $4, \
                     updated_at = clock_timestamp() WHERE pipeline_id = $1",
            )
            .bind(command.pipeline_id)
            .bind(command.worker_id)
            .bind(command.assignment_id)
            .bind(command.generation)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;
        }

        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO worker_command_ledger \
             (command_id, worker_id, pipeline_id, assignment_id, generation, status) \
             VALUES ($1, $2, $3, $4, $5, 'processing') \
             ON CONFLICT (command_id) DO NOTHING RETURNING command_id",
        )
        .bind(command.command_id)
        .bind(command.worker_id)
        .bind(command.pipeline_id)
        .bind(command.assignment_id)
        .bind(command.generation)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| error_with("database_error", error))?;

        let decision = if inserted.is_some() {
            CommandFenceDecision::Process
        } else {
            let recorded = sqlx::query_as::<_, CommandRow>(
                "SELECT worker_id, pipeline_id, assignment_id, generation, status \
                 FROM worker_command_ledger WHERE command_id = $1 FOR UPDATE",
            )
            .bind(command.command_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| error_with("database_error", error))?;

            if recorded.worker_id != command.worker_id
                || recorded.pipeline_id != command.pipeline_id
                || recorded.assignment_id != command.assignment_id
                || recorded.generation != command.generation
            {
                return Err(PortError {
                    code: "command_identity_conflict",
                    message: "command ID was already used for a different identity".to_owned(),
                });
            }
            if recorded.status == "completed" {
                CommandFenceDecision::Duplicate
            } else {
                CommandFenceDecision::Process
            }
        };

        transaction
            .commit()
            .await
            .map_err(|error| error_with("database_error", error))?;
        Ok(decision)
    }

    async fn complete_command(&self, command_id: Uuid) -> PortResult<()> {
        let updated = sqlx::query(
            "UPDATE worker_command_ledger \
             SET status = 'completed', completed_at = clock_timestamp() \
             WHERE command_id = $1 AND status = 'processing'",
        )
        .bind(command_id)
        .execute(&self.pool)
        .await
        .map_err(|error| error_with("database_error", error))?;
        if updated.rows_affected() == 1 {
            return Ok(());
        }

        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM worker_command_ledger WHERE command_id = $1")
                .bind(command_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| error_with("database_error", error))?;
        match status.as_deref() {
            Some("completed") => Ok(()),
            Some(_) => Err(PortError {
                code: "command_completion_conflict",
                message: "worker command could not be marked complete".to_owned(),
            }),
            None => Err(PortError {
                code: "command_not_found",
                message: "worker command was not recorded before completion".to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{env, sync::Arc};

    use super::*;
    use crate::postgres::{connect_postgres, migrate};

    async fn insert_worker_and_assignment(
        pool: &PgPool,
        worker_id: Uuid,
        pipeline_id: Uuid,
        assignment_id: Uuid,
        generation: i64,
    ) {
        sqlx::query(
            "INSERT INTO workers \
             (id, stable_name, platform, architecture, version, state, last_heartbeat) \
             VALUES ($1, $2, 'linux', 'x86_64', 'test', 'ready', clock_timestamp())",
        )
        .bind(worker_id)
        .bind(format!("fencing-worker-{worker_id}"))
        .execute(pool)
        .await
        .expect("worker inserts");

        sqlx::query(
            "INSERT INTO pipeline_assignments \
             (id, resource_type, resource_id, worker_id, generation, desired_state, observed_state) \
             VALUES ($1, 'pipeline', $2, $3, $4, 'planned', 'not_started')",
        )
        .bind(assignment_id)
        .bind(pipeline_id)
        .bind(worker_id)
        .bind(generation)
        .execute(pool)
        .await
        .expect("assignment inserts");
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn postgres_store_persists_fences_and_command_deduplication_across_instances() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("migrations apply");

        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let first_assignment_id = Uuid::new_v4();
        let second_assignment_id = Uuid::new_v4();
        insert_worker_and_assignment(&pool, worker_id, pipeline_id, first_assignment_id, 1).await;

        let first_command_id = Uuid::new_v4();
        let first = WorkerCommandIdentity {
            command_id: first_command_id,
            worker_id,
            pipeline_id,
            assignment_id: first_assignment_id,
            generation: 1,
        };
        let store = PgAssignmentGenerationStore::new(pool.clone());
        assert_eq!(
            store
                .begin_command(first)
                .await
                .expect("first command begins"),
            CommandFenceDecision::Process
        );
        store
            .complete_command(first_command_id)
            .await
            .expect("command completes");

        sqlx::query("UPDATE pipeline_assignments SET desired_state = 'cancelled' WHERE id = $1")
            .bind(first_assignment_id)
            .execute(&pool)
            .await
            .expect("first assignment is retired");
        sqlx::query(
            "INSERT INTO pipeline_assignments \
             (id, resource_type, resource_id, worker_id, generation, desired_state, observed_state) \
             VALUES ($1, 'pipeline', $2, $3, 2, 'planned', 'not_started')",
        )
        .bind(second_assignment_id)
        .bind(pipeline_id)
        .bind(worker_id)
        .execute(&pool)
        .await
        .expect("second assignment inserts");

        let restarted_store = PgAssignmentGenerationStore::new(pool.clone());
        assert_eq!(
            restarted_store
                .begin_command(first)
                .await
                .expect("completed retry is recognized"),
            CommandFenceDecision::Duplicate
        );

        let second = WorkerCommandIdentity {
            command_id: Uuid::new_v4(),
            worker_id,
            pipeline_id,
            assignment_id: second_assignment_id,
            generation: 2,
        };
        assert_eq!(
            restarted_store
                .begin_command(second)
                .await
                .expect("new generation begins"),
            CommandFenceDecision::Process
        );
        let stale = WorkerCommandIdentity {
            command_id: Uuid::new_v4(),
            assignment_id: first_assignment_id,
            ..first
        };
        assert_eq!(
            restarted_store
                .begin_command(stale)
                .await
                .expect("stale generation is rejected"),
            CommandFenceDecision::Stale {
                current_generation: 2
            }
        );
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL database; set DATABASE_URL and run ignored tests"]
    async fn postgres_store_serializes_concurrent_generation_updates() {
        let pool = connect_postgres(&env::var("DATABASE_URL").expect("DATABASE_URL is set"))
            .await
            .expect("database connects");
        migrate(&pool).await.expect("migrations apply");

        let worker_id = Uuid::new_v4();
        let pipeline_id = Uuid::new_v4();
        let assignments = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        insert_worker_and_assignment(&pool, worker_id, pipeline_id, assignments[0], 1).await;
        for generation in 2_i64..=3 {
            sqlx::query(
                "UPDATE pipeline_assignments SET desired_state = 'cancelled' \
                 WHERE resource_id = $1 AND resource_type = 'pipeline'",
            )
            .bind(pipeline_id)
            .execute(&pool)
            .await
            .expect("previous assignment is retired");
            let assignment_id = assignments[usize::try_from(generation - 1).expect("index fits")];
            sqlx::query(
                "INSERT INTO pipeline_assignments \
                 (id, resource_type, resource_id, worker_id, generation, desired_state, observed_state) \
                 VALUES ($1, 'pipeline', $2, $3, $4, 'planned', 'not_started')",
            )
            .bind(assignment_id)
            .bind(pipeline_id)
            .bind(worker_id)
            .bind(generation)
            .execute(&pool)
            .await
            .expect("later assignment inserts");
        }

        let store = Arc::new(PgAssignmentGenerationStore::new(pool.clone()));
        let first = WorkerCommandIdentity {
            command_id: Uuid::new_v4(),
            worker_id,
            pipeline_id,
            assignment_id: assignments[1],
            generation: 2,
        };
        let second = WorkerCommandIdentity {
            command_id: Uuid::new_v4(),
            assignment_id: assignments[2],
            generation: 3,
            ..first
        };
        let (first_result, second_result) =
            tokio::join!(store.begin_command(first), store.begin_command(second));
        assert!(first_result.is_ok());
        assert!(second_result.is_ok());
        assert_eq!(
            store
                .begin_command(WorkerCommandIdentity {
                    command_id: Uuid::new_v4(),
                    ..first
                })
                .await
                .expect("older generation is stale"),
            CommandFenceDecision::Stale {
                current_generation: 3
            }
        );
    }
}
