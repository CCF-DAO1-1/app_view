use common_x::restful::{
    axum::{
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::IntoParams;
use validator::Validate;

use crate::{AppView, api::build_author, atproto::index_query, error::AppError};

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

#[utoipa::path(get, path = "/api/repo/login_info", params(RepoQuery))]
pub async fn login_info(
    State(state): State<AppView>,
    Query(query): Query<RepoQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let first = index_query(&state.pds, &query.repo, "firstItem")
        .await
        .map_err(|e| AppError::CallPdsFailed(e.to_string()))?;
    let first = first
        .pointer("/result/result")
        .cloned()
        .and_then(|i| i.as_u64())
        .ok_or(AppError::CallPdsFailed(first.to_string()))?;
    let second = index_query(&state.pds, &query.repo, "secondItem")
        .await
        .map_err(|e| AppError::CallPdsFailed(e.to_string()))?;
    let second = second
        .pointer("/result/result")
        .cloned()
        .and_then(|i| i.as_u64())
        .ok_or(AppError::CallPdsFailed(second.to_string()))?;
    let third = index_query(&state.pds, &query.repo, "thirdItem")
        .await
        .map_err(|e| AppError::CallPdsFailed(e.to_string()))?;
    let third = third
        .pointer("/result/result")
        .cloned()
        .and_then(|i| i.as_u64())
        .ok_or(AppError::CallPdsFailed(third.to_string()))?;

    Ok(ok(json!({
        "firstItem": first,
        "secondItem": second,
        "thirdItem": third,
    })))
}
