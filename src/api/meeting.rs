use color_eyre::eyre::eyre;
use common_x::restful::{
    axum::{extract::State, response::IntoResponse},
    ok,
};
use sea_query::{Expr, ExprTrait, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;

use crate::{
    AppView,
    error::AppError,
    lexicon::meeting::{Meeting, MeetingRow, MeetingState},
};

#[utoipa::path(get, path = "/api/meeting")]
pub async fn get(State(state): State<AppView>) -> Result<impl IntoResponse, AppError> {
    let (sql, values) = Meeting::build_select()
        .and_where(Expr::col(Meeting::State).eq(MeetingState::Scheduled as i32))
        .order_by(Meeting::Created, Order::Desc)
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<MeetingRow> = sqlx::query_as_with(&sql, values)
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    Ok(ok(rows))
}
