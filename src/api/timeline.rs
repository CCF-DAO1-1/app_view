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
use utoipa::IntoParams;
use validator::Validate;

use crate::{
    AppView,
    api::build_authors,
    error::AppError,
    lexicon::timeline::{Timeline, TimelineRow, TimelineView},
};
use serde_json::json;

#[derive(Debug, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct TimelineQuery {
    #[validate(length(min = 1))]
    pub uri: String,
    /// number of items to return
    pub limit: u64,
}

impl Default for TimelineQuery {
    fn default() -> Self {
        Self {
            uri: String::new(),
            limit: 50,
        }
    }
}

#[utoipa::path(get, path = "/api/timeline", params(TimelineQuery))]
pub async fn get(
    State(state): State<AppView>,
    Query(query): Query<TimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, values) = sea_query::Query::select()
        .columns([
            (Timeline::Table, Timeline::Id),
            (Timeline::Table, Timeline::TimelineType),
            (Timeline::Table, Timeline::Message),
            (Timeline::Table, Timeline::Target),
            (Timeline::Table, Timeline::Operator),
            (Timeline::Table, Timeline::Timestamp),
        ])
        .from(Timeline::Table)
        .and_where(Expr::col(Timeline::Target).eq(query.uri))
        .order_by(Timeline::Timestamp, Order::Desc)
        .limit(std::cmp::min(query.limit, 100))
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<TimelineRow> = sqlx::query_as_with(&sql, values)
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    // Batch fetch authors to avoid N+1 queries
    let repos: Vec<&str> = rows.iter().map(|r| r.operator.as_str()).collect();
    let authors = build_authors(&state, &repos).await;

    let mut views = vec![];
    for row in rows {
        views.push(TimelineView {
            id: row.id,
            timeline_type: row.timeline_type,
            message: row.message,
            target: row.target,
            operator: authors
                .get(&row.operator)
                .cloned()
                .unwrap_or_else(|| json!({"did": &row.operator})),
            timestamp: row.timestamp,
        });
    }

    Ok(ok(views))
}
