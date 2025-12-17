use color_eyre::eyre::eyre;
use common_x::restful::{
    axum::{
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use sea_query::{Expr, ExprTrait, Order, PostgresQueryBuilder, extension::postgres::PgExpr};
use sea_query_sqlx::SqlxBinder;
use serde::Deserialize;
use serde_json::json;
use sqlx::query_as_with;
use utoipa::IntoParams;
use validator::Validate;

use crate::{
    AppView,
    error::AppError,
    lexicon::{
        proposal::{Proposal, ProposalRow},
        task::{Task, TaskRow, TaskState, TaskView},
    },
};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct TaskQuery {
    #[validate(length(min = 1))]
    pub did: String,
}

#[utoipa::path(get, path = "/api/task", params(TaskQuery))]
pub async fn get(
    State(state): State<AppView>,
    Query(query): Query<TaskQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, values) = sea_query::Query::select()
        .columns([
            (Task::Table, Task::Id),
            (Task::Table, Task::TaskType),
            (Task::Table, Task::Message),
            (Task::Table, Task::Target),
            (Task::Table, Task::Operators),
            (Task::Table, Task::Deadline),
            (Task::Table, Task::State),
            (Task::Table, Task::Updated),
            (Task::Table, Task::Created),
        ])
        .from(Task::Table)
        .and_where(Expr::col(Task::State).ne(TaskState::Completed as i32))
        .and_where(
            Expr::col(Task::Operators)
                .is_null()
                .or(Expr::col(Task::Operators).contains(query.did)),
        )
        .order_by(Task::Created, Order::Desc)
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<TaskRow> = sqlx::query_as_with(&sql, values)
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    let mut views = vec![];
    for row in rows {
        let (sql, values) = Proposal::build_select(None)
            .and_where(Expr::col(Proposal::Uri).eq(row.target))
            .build_sqlx(PostgresQueryBuilder);

        let proposal: ProposalRow = query_as_with(&sql, values.clone())
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                debug!("exec sql failed: {e}");
                AppError::NotFound
            })?;

        views.push(TaskView {
            id: row.id,
            task_type: row.task_type,
            importance: row.importance,
            message: row.message,
            target: json!(proposal),
            operators: row.operators,
            deadline: row.deadline,
            state: row.state,
            updated: row.updated,
            created: row.created,
        });
    }

    Ok(ok(views))
}
