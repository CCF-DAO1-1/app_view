use color_eyre::{Result, eyre::OptionExt};
use common_x::restful::{
    axum::{
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Deserialize;
use serde_json::json;
use sqlx::query_as_with;
use utoipa::IntoParams;
use validator::Validate;

use crate::{
    AppView,
    ckb::get_nervos_dao_deposit,
    error::AppError,
    lexicon::vote_whitelist::{VoteWhitelist, VoteWhitelistRow},
    smt::CkbSMT,
};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct CkbAddrQuery {
    #[validate(length(min = 1))]
    pub ckb_addr: String,
}

#[utoipa::path(get, path = "/api/vote/weight", params(CkbAddrQuery))]
pub async fn weight(
    State(state): State<AppView>,
    Query(query): Query<CkbAddrQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let from_list = crate::indexer::query_by_to(&state.indexer_url, &query.ckb_addr).await?;
    debug!("from_list: {:?}", from_list);
    let mut weight = get_nervos_dao_deposit(&state.ckb_client, &query.ckb_addr).await?;

    for from in from_list
        .as_array()
        .ok_or_eyre("from_list is not an array")?
    {
        debug!("from: {:?}", from);
        let from = from
            .get("from")
            .and_then(|f| f.as_str())
            .ok_or_eyre("missing from field")?;
        let nervos_dao_deposit = get_nervos_dao_deposit(&state.ckb_client, from).await?;
        weight += nervos_dao_deposit;
    }
    Ok(ok(json!({ "weight": weight })))
}

#[utoipa::path(get, path = "/api/vote/whitelist")]
pub async fn whitelist(State(state): State<AppView>) -> Result<impl IntoResponse, AppError> {
    let id = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(id))
        .build_sqlx(PostgresQueryBuilder);

    debug!("sql: {sql} ({values:?})");

    let row: VoteWhitelistRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;
    Ok(ok(row))
}

#[utoipa::path(get, path = "/api/vote/proof", params(CkbAddrQuery))]
pub async fn proof(
    State(state): State<AppView>,
    Query(query): Query<CkbAddrQuery>,
) -> Result<impl IntoResponse, AppError> {
    let id = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(id))
        .build_sqlx(PostgresQueryBuilder);

    debug!("sql: {sql} ({values:?})");

    let row: VoteWhitelistRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;

    let mut smt_tree = CkbSMT::default();
    for lock_hash in row.list.iter() {
        let lock_hash = hex::decode(lock_hash).unwrap();
        let key: [u8; 32] = lock_hash.as_slice().try_into().unwrap();
        smt_tree
            .update(key.into(), crate::smt::SMT_VALUE.into())
            .ok();
    }

    let smt_root_hash = hex::encode(smt_tree.root().as_slice());

    let address = crate::AddressParser::default()
        .set_network(ckb_sdk::NetworkType::Testnet)
        .parse(&query.ckb_addr)
        .unwrap();
    let lock_script = ckb_types::packed::Script::from(address.payload());
    let lock_hash = lock_script.calc_script_hash();
    let key: [u8; 32] = lock_hash.raw_data().to_vec().as_slice().try_into().unwrap();
    let proof = smt_tree.merkle_proof(vec![key.into()]).unwrap();
    let compiled_proof = proof.clone().compile(vec![key.into()]).unwrap();
    println!("proof: {}", hex::encode(&compiled_proof.0));

    Ok(ok(json!({
        "proof": hex::encode(&compiled_proof.0),
        "root_hash": smt_root_hash,
    })))
}
