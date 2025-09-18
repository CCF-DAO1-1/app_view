use common_x::restful::{
    axum::{
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use serde::Deserialize;
use serde_json::Value;
use utoipa::IntoParams;
use validator::Validate;

use crate::{AppView, api::build_author, error::AppError};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct RepoQuery {
    #[validate(length(min = 1))]
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

    let mut author = build_author(&state, &query.repo).await;
    if state.whitelist.is_empty() || state.whitelist.contains(&query.repo) {
        author["highlight"] = Value::String("beta".to_owned());
    }

    Ok(ok(author))
}
