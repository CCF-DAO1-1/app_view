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
    api::build_author,
    error::AppError,
    lexicon::timeline::{Timeline, TimelineRow, TimelineView},
};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct TimelineQuery {
    #[validate(length(min = 1))]
    pub uri: String,
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
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<TimelineRow> = sqlx::query_as_with(&sql, values)
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    let mut views = vec![];
    for row in rows {
        views.push(TimelineView {
            id: row.id,
            timeline_type: row.timeline_type,
            message: row.message,
            target: row.target,
            operator: build_author(&state, &row.operator).await,
            timestamp: row.timestamp,
        });
    }

    Ok(ok(views))
}
