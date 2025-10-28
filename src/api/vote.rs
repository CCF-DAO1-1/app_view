use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use common_x::restful::{
    axum::{
        Json,
        extract::{Query, State},
        response::IntoResponse,
    },
    ok, ok_simple,
};
use k256::ecdsa::signature::Verifier;
use k256::ecdsa::{Signature, VerifyingKey};
use molecule::prelude::{Builder, Entity};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sparse_merkle_tree::H256;
use sqlx::query_as_with;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::{
    AppView,
    ckb::get_nervos_dao_deposit,
    error::AppError,
    lexicon::{
        administrator::{Administrator, AdministratorRow},
        proposal::{Proposal, ProposalSample},
        vote_whitelist::{VoteWhitelist, VoteWhitelistRow},
    },
    molecules,
    smt::{Blake2bHasher, CkbSMT, SMT_VALUE},
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

    let from_list =
        crate::indexer_bind::query_by_to(&state.indexer_bind_url, &query.ckb_addr).await?;
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

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct ProofQuery {
    #[validate(length(min = 1))]
    pub ckb_addr: String,
    pub whitelist_id: String,
}

#[utoipa::path(get, path = "/api/vote/proof", params(ProofQuery))]
pub async fn proof(
    State(state): State<AppView>,
    Query(query): Query<ProofQuery>,
) -> Result<impl IntoResponse, AppError> {
    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(query.whitelist_id))
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
        if let Ok(lock_hash) = hex::decode(lock_hash)
            && let Ok(key) = TryInto::<[u8; 32]>::try_into(lock_hash.as_slice())
        {
            smt_tree
                .update(key.into(), crate::smt::SMT_VALUE.into())
                .ok();
        }
    }

    let smt_root_hash: H256 = *smt_tree.root();
    let smt_root_hash_hex = hex::encode(smt_root_hash.as_slice());

    let address = crate::AddressParser::default()
        .set_network(ckb_sdk::NetworkType::Testnet)
        .parse(&query.ckb_addr)
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;
    let lock_script = ckb_types::packed::Script::from(address.payload());
    let lock_hash = lock_script.calc_script_hash();
    let key: [u8; 32] = lock_hash.raw_data().to_vec().as_slice().try_into()?;
    let proof = smt_tree
        .merkle_proof(vec![key.into()])
        .map_err(|e| eyre!(e))?;
    let compiled_proof = proof
        .clone()
        .compile(vec![key.into()])
        .map_err(|e| eyre!(e))?;

    let proof: Vec<u8> = compiled_proof.0;
    let compiled_proof = sparse_merkle_tree::CompiledMerkleProof(proof);
    let ret = compiled_proof
        .verify::<Blake2bHasher>(&smt_root_hash, vec![(key.into(), SMT_VALUE.into())])
        .unwrap_or(false);

    if ret {
        Ok(ok(json!({
            "proof": hex::encode(&compiled_proof.0),
            "root_hash": smt_root_hash_hex,
        })))
    } else {
        Err(AppError::ValidateFailed("Not in smt".to_string()))
    }
}

#[utoipa::path(
    get,
    path = "/api/vote/build_whitelist",
    description = "方便调试用的，请勿随意调用"
)]
pub async fn build_whitelist(State(state): State<AppView>) -> Result<impl IntoResponse, AppError> {
    tokio::spawn(
        crate::scheduler::build_vote_whitelist::build_vote_whitelist(
            state.db.clone(),
            state.ckb_client.clone(),
        ),
    );
    Ok(ok_simple())
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct CreateVoteMetaParams {
    pub proposal_uri: String,
    pub candidates: Vec<String>,
    pub start_time: u64,
    pub end_time: u64,
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct CreateVoteMetaBody {
    pub params: CreateVoteMetaParams,
    pub did: String,
    #[validate(length(equal = 57))]
    pub signing_key_did: String,
    pub signed_bytes: String,
}

#[utoipa::path(post, path = "/api/vote/create_vote_meta")]
pub async fn create_vote_meta(
    State(state): State<AppView>,
    Json(body): Json<CreateVoteMetaBody>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    verify_signature(
        &body.did,
        &state.indexer_did_url,
        &body.signing_key_did,
        &body.signed_bytes,
        &body.params,
    )
    .await
    .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = sea_query::Query::select()
        .columns([
            (Proposal::Table, Proposal::Uri),
            (Proposal::Table, Proposal::Cid),
            (Proposal::Table, Proposal::Repo),
            (Proposal::Table, Proposal::Record),
            (Proposal::Table, Proposal::State),
            (Proposal::Table, Proposal::Updated),
        ])
        .from(Proposal::Table)
        .and_where(Expr::col(Proposal::Uri).eq(body.params.proposal_uri.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("proposal not found: {e}")))?;
    let proposal_hash = ckb_hash::blake2b_256(serde_json::to_vec(&proposal_sample)?);

    let whitelist_id = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(whitelist_id))
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
        if let Ok(lock_hash) = hex::decode(lock_hash)
            && let Ok(key) = TryInto::<[u8; 32]>::try_into(lock_hash.as_slice())
        {
            smt_tree
                .update(key.into(), crate::smt::SMT_VALUE.into())
                .ok();
        }
    }

    let vote_meta = molecules::VoteMeta::new_builder()
        .candidates(molecules::StringVec::from(
            body.params
                .candidates
                .iter()
                .map(|c| molecules::String::from(c.as_bytes().to_vec()))
                .collect::<Vec<molecules::String>>(),
        ))
        .smt_root_hash(
            molecules::BytesOpt::new_builder()
                .set(Some(smt_tree.root().as_slice().to_vec().into()))
                .build(),
        )
        .start_time(
            molecules::Uint64::new_builder()
                .set::<[molecule::prelude::Byte; 8]>(
                    body.params.start_time.to_be_bytes().map(|b| b.into()),
                )
                .build(),
        )
        .end_time(
            molecules::Uint64::new_builder()
                .set::<[molecule::prelude::Byte; 8]>(
                    body.params.end_time.to_be_bytes().map(|b| b.into()),
                )
                .build(),
        )
        .extra(
            molecules::BytesOpt::new_builder()
                .set(Some(proposal_hash.to_vec().into()))
                .build(),
        )
        .build();

    let vote_meta_bytes = vote_meta.as_bytes().to_vec();
    let vote_meta_hex = hex::encode(vote_meta_bytes);

    let outputs_data = vec![vote_meta_hex];

    Ok(ok(json!({ "outputsData": outputs_data })))
}

async fn verify_signature<T>(
    did: &str,
    indexer_did_url: &str,
    signing_key_did: &str,
    signed_bytes: &str,
    message: &T,
) -> Result<()>
where
    T: Serialize + ?Sized,
{
    // verify did
    let did_doc = crate::indexer_did::did_document(indexer_did_url, did)
        .await
        .map_err(|e| eyre!("get did doc failed: {e}"))?;

    if signing_key_did
        != did_doc
            .pointer("/verificationMethods/atproto")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
    {
        return Err(eyre!("signing_key_did not match"));
    }

    // verify signature
    let verifying_key: VerifyingKey = signing_key_did
        .split_once("did:key:z")
        .and_then(|(_, key)| {
            let bytes = bs58::decode(key).into_vec().ok()?;
            VerifyingKey::from_sec1_bytes(&bytes[2..]).ok()
        })
        .ok_or_eyre("invalid signing_key_did")?;
    let signature = hex::decode(signed_bytes)
        .map(|bytes| Signature::from_slice(&bytes).map_err(|e| eyre!(e)))??;

    let unsigned_bytes = serde_ipld_dagcbor::to_vec(message)?;
    debug!("unsigned_bytes: {}", hex::encode(&unsigned_bytes));
    verifying_key
        .verify(&unsigned_bytes, &signature)
        .map_err(|e| eyre!("verify signature failed: {e}"))
}
