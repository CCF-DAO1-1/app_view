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
    error::AppError,
    lexicon::meeting::{Meeting, MeetingRow, MeetingState},
};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct ProposalUriQuery {
    #[validate(length(min = 1))]
    pub proposal: String,
}

#[utoipa::path(get, path = "/api/meeting", params(ProposalUriQuery))]
pub async fn get(
    State(state): State<AppView>,
    Query(query): Query<ProposalUriQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;
    let (sql, values) = Meeting::build_select()
        .and_where(Expr::col(Meeting::State).eq(MeetingState::Scheduled as i32))
        .and_where(Expr::col(Meeting::ProposalUri).eq(&query.proposal))
        .order_by(Meeting::Created, Order::Desc)
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<MeetingRow> = sqlx::query_as_with(&sql, values)
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    Ok(ok(rows))
}
