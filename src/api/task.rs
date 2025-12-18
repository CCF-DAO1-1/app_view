use color_eyre::eyre::eyre;
use common_x::restful::{
    axum::{
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use sea_query::{Expr, ExprTrait, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Deserialize;
use serde_json::json;
use sqlx::query_as_with;
use utoipa::IntoParams;
use validator::Validate;

use crate::{
    AppView,
    api::build_author,
    error::AppError,
    lexicon::{
        proposal::{Proposal, ProposalRow},
        task::{Task, TaskRow, TaskState, TaskView},
    },
};

#[derive(Debug, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct TaskQuery {
    #[validate(length(min = 1))]
    pub did: String,
    #[validate(range(min = 1))]
    pub page: u64,
    #[validate(range(min = 1))]
    pub per_page: u64,
}

impl Default for TaskQuery {
    fn default() -> Self {
        Self {
            did: String::new(),
            page: 1,
            per_page: 20,
        }
    }
}

#[utoipa::path(get, path = "/api/task", params(TaskQuery))]
pub async fn get(
    State(state): State<AppView>,
    Query(query): Query<TaskQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let offset = query.per_page * (query.page - 1);

    let (sql, values) = sea_query::Query::select()
        .columns([
            (Task::Table, Task::Id),
            (Task::Table, Task::TaskType),
            (Task::Table, Task::Message),
            (Task::Table, Task::Target),
            (Task::Table, Task::Operators),
            (Task::Table, Task::Processor),
            (Task::Table, Task::Deadline),
            (Task::Table, Task::State),
            (Task::Table, Task::Updated),
            (Task::Table, Task::Created),
        ])
        .from(Task::Table)
        .and_where(Expr::col(Task::State).ne(TaskState::Completed as i32))
        .and_where(Expr::col(Task::Operators).is_null().or(Expr::cust(format!(
            "'{}' = ANY(\"task\".\"operators\")",
            query.did
        ))))
        .order_by(Task::Created, Order::Desc)
        .offset(offset)
        .limit(query.per_page)
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

        let processor = if let Some(processor) = &row.processor {
            build_author(&state, processor).await
        } else {
            serde_json::Value::Null
        };
        views.push(TaskView {
            id: row.id,
            task_type: row.task_type,
            message: row.message,
            target: json!(proposal),
            operators: row.operators,
            processor,
            deadline: row.deadline,
            state: row.state,
            updated: row.updated,
            created: row.created,
        });
    }

    let (sql, values) = sea_query::Query::select()
        .expr(Expr::col((Task::Table, Task::Id)).count())
        .from(Task::Table)
        .and_where(Expr::col(Task::State).ne(TaskState::Completed as i32))
        .and_where(Expr::col(Task::Operators).is_null().or(Expr::cust(format!(
            "'{}' = ANY(\"task\".\"operators\")",
            query.did
        ))))
        .build_sqlx(PostgresQueryBuilder);

    let total: (i64,) = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    Ok(ok(json!({
        "tasks": views,
        "page": query.page,
        "per_page": query.per_page,
        "total":  total.0
    })))
}
