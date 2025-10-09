use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use color_eyre::eyre::{OptionExt, eyre};
use common_x::restful::{
    axum::{Json, extract::State, response::IntoResponse},
    ok,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::{
    AppView,
    atproto::{NSID_LIKE, NSID_PROPOSAL, NSID_REPLY, direct_writes},
    error::AppError,
    lexicon::{like::Like, proposal::Proposal, reply::Reply},
};

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct NewRecord {
    /// user's DID
    repo: String,
    /// record rkey (for update, must be the same as the existing record)
    rkey: String,
    /// record value
    #[schema(
        example = "{\"$type\": \"app.dao.proposal\", \"created\": \"2025-09-24T04:41:17Z\", \"text\": \"Hello, world!\"}"
    )]
    value: Value,
    /// signing key
    signing_key: String,
    /// ckb address
    ckb_addr: String,
    root: Value,
}

#[utoipa::path(post, path = "/api/record/create")]
pub async fn create(
    State(state): State<AppView>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(new_record): Json<NewRecord>,
) -> Result<impl IntoResponse, AppError> {
    let record_type = new_record
        .value
        .get("$type")
        .map(|t| t.as_str())
        .ok_or_eyre("'$type' must be set")?
        .ok_or_eyre("'$type' must be set")?;
    if !state.whitelist.is_empty() && !state.whitelist.contains(&new_record.repo) {
        match record_type {
            NSID_PROPOSAL | NSID_REPLY => {
                return Err(eyre!("Operation is not allowed!").into());
            }
            _ => {}
        }
    }
    let result = direct_writes(
        &state.pds,
        auth.token(),
        &new_record.repo,
        &json!([{
            "$type": "com.atproto.web5.directWrites#create",
            "collection": new_record.value["$type"],
            "rkey": new_record.rkey,
            "value": new_record.value
        }]),
        &new_record.signing_key,
        &new_record.ckb_addr,
        &new_record.root,
    )
    .await
    .map_err(|e| AppError::CallPdsFailed(e.to_string()))?;
    debug!("pds: {}", result);
    let uri = result
        .pointer("/results/0/uri")
        .and_then(|uri| uri.as_str())
        .ok_or(AppError::CallPdsFailed(result.to_string()))?;
    let cid = result
        .pointer("/results/0/cid")
        .and_then(|cid| cid.as_str())
        .ok_or(AppError::CallPdsFailed(result.to_string()))?;
    match record_type {
        NSID_PROPOSAL => {
            Proposal::insert(&state.db, &new_record.repo, new_record.value, uri, cid).await?;
        }
        NSID_REPLY => {
            Reply::insert(&state.db, &new_record.repo, &new_record.value, uri, cid).await?;
        }
        NSID_LIKE => {
            Like::insert(&state.db, &new_record.repo, &new_record.value, uri, cid).await?;
        }
        _ => {}
    }

    Ok(ok(result))
}

#[utoipa::path(post, path = "/api/record/update")]
pub async fn update(
    State(state): State<AppView>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(new_record): Json<NewRecord>,
) -> Result<impl IntoResponse, AppError> {
    let record_type = new_record
        .value
        .get("$type")
        .map(|t| t.as_str())
        .ok_or_eyre("'$type' must be set")?
        .ok_or_eyre("'$type' must be set")?;
    if !state.whitelist.is_empty() && !state.whitelist.contains(&new_record.repo) {
        match record_type {
            NSID_PROPOSAL | NSID_REPLY => {
                return Err(eyre!("Operation is not allowed!").into());
            }
            _ => {}
        }
    }
    let result = direct_writes(
        &state.pds,
        auth.token(),
        &new_record.repo,
        &json!([{
            "$type": "com.atproto.web5.directWrites#update",
            "collection": new_record.value["$type"],
            "rkey": new_record.rkey,
            "value": new_record.value
        }]),
        &new_record.signing_key,
        &new_record.ckb_addr,
        &new_record.root,
    )
    .await
    .map_err(|e| AppError::CallPdsFailed(e.to_string()))?;
    debug!("pds: {}", result);
    let uri = result
        .pointer("/results/0/uri")
        .and_then(|uri| uri.as_str())
        .ok_or(AppError::CallPdsFailed(result.to_string()))?;
    let cid = result
        .pointer("/results/0/cid")
        .and_then(|cid| cid.as_str())
        .ok_or(AppError::CallPdsFailed(result.to_string()))?;
    match record_type {
        NSID_PROPOSAL => {
            Proposal::insert(&state.db, &new_record.repo, new_record.value, uri, cid).await?;
        }
        NSID_REPLY => {
            Reply::insert(&state.db, &new_record.repo, &new_record.value, uri, cid).await?;
        }
        NSID_LIKE => {
            Like::insert(&state.db, &new_record.repo, &new_record.value, uri, cid).await?;
        }
        _ => {}
    }

    Ok(ok(result))
}
