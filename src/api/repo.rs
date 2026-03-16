use common_x::restful::{
    axum::{
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use serde::Deserialize;
use utoipa::IntoParams;
use validator::Validate;

use crate::{AppView, api::build_author, error::AppError};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct RepoQuery {
    #[validate(length(min = 1))]
    /// user's DID
    pub repo: String,
}

#[utoipa::path(get, path = "/api/repo/profile", params(RepoQuery))]
pub async fn profile(
    State(state): State<AppView>,
    Query(query): Query<RepoQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let author = build_author(&state, &query.repo).await;

    Ok(ok(author))
}
