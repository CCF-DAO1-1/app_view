use color_eyre::{Result, eyre::eyre};
use common_x::restful::{
    axum::{
        Json,
        extract::{Query, State},
        response::IntoResponse,
    },
    ok, ok_simple,
};
use molecule::prelude::{Builder, Entity};
use sea_query::{Expr, ExprTrait, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sparse_merkle_tree::H256;
use sqlx::query_as_with;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::{
    AppView,
    api::{SignedBody, SignedParam},
    ckb::{get_ckb_addr_by_did, get_vote_result, get_vote_time_range},
    error::AppError,
    lexicon::{
        administrator::{Administrator, AdministratorRow},
        proposal::{Proposal, ProposalSample},
        vote::{Vote, VoteRow},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState},
        vote_whitelist::{VoteWhitelist, VoteWhitelistRow},
    },
    molecules::{self, VoteProof},
    smt::{Blake2bHasher, CkbSMT, SMT_VALUE},
};

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct CkbAddrQuery {
    #[validate(length(min = 1))]
    pub ckb_addr: String,
}

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct DidQuery {
    #[validate(length(min = 1))]
    pub did: String,
}

#[utoipa::path(get, path = "/api/vote/bind_list", params(DidQuery))]
pub async fn bind_list(
    State(state): State<AppView>,
    Query(query): Query<DidQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let ckb_addr = crate::ckb::get_ckb_addr_by_did(
        &state.ckb_client,
        query
            .did
            .strip_prefix("did:web5")
            .unwrap_or(&query.did)
            .strip_prefix("did:ckb")
            .unwrap_or(&query.did)
            .strip_prefix("did:plc")
            .unwrap_or(&query.did),
    )
    .await?;

    let from_list = crate::indexer_bind::query_by_to(&state.indexer_bind_url, &ckb_addr).await?;

    Ok(ok(from_list))
}

#[utoipa::path(get, path = "/api/vote/weight", params(CkbAddrQuery))]
pub async fn weight(
    State(state): State<AppView>,
    Query(query): Query<CkbAddrQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let weight = crate::indexer_bind::get_weight(
        &state.ckb_client,
        &state.indexer_bind_url,
        &query.ckb_addr,
    )
    .await?;
    Ok(ok(json!({ "weight": weight })))
}

#[utoipa::path(get, path = "/api/vote/whitelist")]
pub async fn whitelist(State(state): State<AppView>) -> Result<impl IntoResponse, AppError> {
    let id = chrono::Local::now().format("%Y-%m-%d").to_string();

    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(id))
        .build_sqlx(PostgresQueryBuilder);

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
    get_proof(&state, &query.whitelist_id, &query.ckb_addr)
        .await
        .map(|r| {
            ok(json!({
                "smt_root_hash": hex::encode(r.0),
                "smt_proof": hex::encode(r.1),
            }))
        })
        .map_err(|e| AppError::ValidateFailed(e.to_string()))
}

async fn get_proof(
    state: &AppView,
    whitelist_id: &str,
    ckb_addr: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(whitelist_id))
        .build_sqlx(PostgresQueryBuilder);

    let row: VoteWhitelistRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| eyre!(e))?;

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

    let address = crate::AddressParser::default()
        .set_network(ckb_sdk::NetworkType::Testnet)
        .parse(ckb_addr)
        .map_err(|e| eyre!(e))?;
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
        Ok((smt_root_hash.as_slice().to_vec(), compiled_proof.0))
    } else {
        Err(eyre!("Not in the whitelist"))
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
    pub timestamp: i64,
}

impl SignedParam for CreateVoteMetaParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(post, path = "/api/vote/create_vote_meta")]
pub async fn create_vote_meta(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<CreateVoteMetaParams>>,
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

    body.verify_signature(&state.indexer_did_url)
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
    let proposal_hash = ckb_hash::blake2b_256(serde_json::to_vec(&proposal_sample.uri)?);

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(body.params.proposal_uri.clone()))
        .and_where(Expr::col(VoteMeta::State).eq(0))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row = if let Ok(vote_meta_row) = query_as_with::<_, VoteMetaRow, _>(&sql, value)
        .fetch_one(&state.db)
        .await
    {
        vote_meta_row
    } else {
        // TODO: 7 days
        let time_range = get_vote_time_range(&state.ckb_client, 7).await?;
        let time_range = crate::ckb::test_get_vote_time_range(&state.ckb_client).await?;
        let mut vote_meta_row = VoteMetaRow {
            id: -1,
            proposal_state: proposal_sample.state,
            state: 0,
            tx_hash: None,
            proposal_uri: body.params.proposal_uri.clone(),
            whitelist_id: chrono::Local::now().format("%Y-%m-%d").to_string(),
            candidates: body.params.candidates.clone(),
            start_time: time_range.0 as i64,
            end_time: time_range.1 as i64,
            creater: body.did.clone(),
            results: None,
            created: chrono::Local::now(),
        };

        vote_meta_row.id = VoteMeta::insert(&state.db, &vote_meta_row).await?;
        vote_meta_row
    };

    let vote_meta = build_vote_meta(&state, &vote_meta_row, &proposal_hash).await?;

    let vote_meta_bytes = vote_meta.as_bytes().to_vec();
    let vote_meta_hex = hex::encode(vote_meta_bytes);

    let outputs_data = vec![vote_meta_hex];

    Ok(ok(json!({
        "vote_meta": vote_meta_row,
        "outputsData": outputs_data
    })))
}

#[test]
fn test() {
    let hex = "89000000180000003c000000550000005d0000006500000020000000c11b4ffff3879b547f6875594ad60efa73422caa470bf46181dd558c48f6c4ea190000000c0000001300000003000000796573020000006e6f000000006908753c00000000690c69bc20000000953dc36641f25d4ca206f8464a53242d2cbdcac72ef2b0eb87ccdb95aa93c8b9";
    let bs = hex::decode(hex).unwrap();
    let vm = molecules::VoteMeta::from_slice(&bs).unwrap();
    println!("vm: {}", vm);
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct UpdateTxParams {
    pub id: i32,
    pub tx_hash: String,
    pub timestamp: i64,
}

impl SignedParam for UpdateTxParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(post, path = "/api/vote/update_meta_tx_hash")]
pub async fn update_meta_tx_hash(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<UpdateTxParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::Id).eq(body.params.id))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row: VoteMetaRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;

    if vote_meta_row.creater != body.did {
        return Err(AppError::ValidateFailed("not creater".to_string()));
    }

    VoteMeta::update_tx_hash(&state.db, body.params.id, &body.params.tx_hash)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("update vote_meta tx_hash failed: {e}")))?;

    Ok(ok_simple())
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct UpdateVoteTxParams {
    pub id: i32,
    pub tx_hash: String,
    pub candidates_index: i32,
    pub timestamp: i64,
}

impl SignedParam for UpdateVoteTxParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(post, path = "/api/vote/update_vote_tx_hash")]
pub async fn update_vote_tx_hash(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<UpdateVoteTxParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let mut vote_row = VoteRow {
        id: -1,
        state: 0,
        tx_hash: Some(body.params.tx_hash),
        vote_meta_id: body.params.id,
        candidates_index: body.params.candidates_index,
        voter: body.did.clone(),
        created: chrono::Local::now(),
    };
    vote_row.id = Vote::insert(&state.db, &vote_row).await?;

    Ok(ok(vote_row))
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct CreateVoteParams {
    pub vote_meta_id: i32,
    pub candidates_index: i32,
    pub timestamp: i64,
}

impl SignedParam for CreateVoteParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

// #[utoipa::path(post, path = "/api/vote/create_vote")]
pub async fn _create_vote(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<CreateVoteParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let mut vote_row = VoteRow {
        id: -1,
        state: 0,
        tx_hash: None,
        vote_meta_id: body.params.vote_meta_id,
        candidates_index: body.params.candidates_index,
        voter: body.did.clone(),
        created: chrono::Local::now(),
    };
    vote_row.id = Vote::insert(&state.db, &vote_row).await?;

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::Id).eq(body.params.vote_meta_id))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row: VoteMetaRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not vote_meta: {e}")))?;

    // TODO build vote row tx
    let vote_addr = get_ckb_addr_by_did(&state.ckb_client, &body.did).await?;
    let address = crate::AddressParser::default()
        .set_network(ckb_sdk::NetworkType::Testnet)
        .parse(&vote_addr)
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;
    let lock_script = ckb_types::packed::Script::from(address.payload());
    let proof = get_proof(&state, &vote_meta_row.whitelist_id, &vote_addr).await?;

    let _vote_proof = VoteProof::new_builder()
        .lock_script_hash::<Vec<u8>>(lock_script.calc_script_hash().raw_data().to_vec())
        .smt_proof::<Vec<u8>>(proof.1)
        .build();

    Ok(ok(json!({
        "row_tx": {
            "cellDeps": [],
            "outputs": [],
            "outputsData": [],
            "witnesses": [],
        }
    })))
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct PrepareBody {
    pub vote_meta_id: i32,
    pub did: String,
}

#[utoipa::path(post, path = "/api/vote/prepare")]
pub async fn prepare(
    State(state): State<AppView>,
    Json(body): Json<PrepareBody>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::Id).eq(body.vote_meta_id))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row: VoteMetaRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not vote_meta: {e}")))?;

    if vote_meta_row.state != (VoteMetaState::Committed as i32) {
        return Err(AppError::ValidateFailed(format!(
            "vote_meta not aready: {}",
            vote_meta_row.state
        )));
    }

    let vote_addr = get_ckb_addr_by_did(&state.ckb_client, &body.did).await?;

    let proof = get_proof(&state, &vote_meta_row.whitelist_id, &vote_addr).await?;

    Ok(ok(json!({
        "vote_meta": vote_meta_row,
        "did": body.did,
        "vote_addr": vote_addr,
        "proof": proof.1
    })))
}

#[utoipa::path(post, path = "/api/vote/status")]
pub async fn status(
    State(state): State<AppView>,
    Json(body): Json<PrepareBody>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Vote::build_select()
        .and_where(Expr::col(Vote::VoteMetaId).eq(body.vote_meta_id))
        .and_where(Expr::col(Vote::Voter).eq(body.did))
        .order_by(Vote::Created, Order::Desc)
        .build_sqlx(PostgresQueryBuilder);
    let vote_row_vec: Vec<VoteRow> = query_as_with(&sql, value)
        .fetch_all(&state.db)
        .await
        .ok()
        .unwrap_or(vec![]);

    Ok(ok(vote_row_vec))
}

#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct VoteResult {
    pub ckb_addr: String,
    pub candidates_index: usize,
    pub weight: u64,
}

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct DetailQuery {
    pub id: i32,
}

#[utoipa::path(get, path = "/api/vote/detail", params(DetailQuery))]
pub async fn detail(
    State(state): State<AppView>,
    Query(query): Query<DetailQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::Id).eq(query.id))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row: VoteMetaRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;

    if vote_meta_row.state != (VoteMetaState::Committed as i32)
        && vote_meta_row.state != (VoteMetaState::Finished as i32)
    {
        return Err(AppError::ValidateFailed(format!(
            "vote_meta not aready: {}",
            vote_meta_row.state
        )));
    }

    let votes = if let Some(tx_hash) = &vote_meta_row.tx_hash {
        get_vote_result(&state.ckb_client, &state.indexer_bind_url, tx_hash).await?
    } else {
        return Err(AppError::ValidateFailed(
            "vote_meta have not tx_hash".to_string(),
        ));
    };
    let vote_sum = votes.len();
    let mut valid_vote_sum = 0;
    let mut weight_sum = 0;
    let mut valid_weight_sum = 0;
    let mut candidate_votes = vec![(0, 0); vote_meta_row.candidates.len()];
    for vote in votes {
        weight_sum += vote.1.1;
        if let Some(candidate_vote) = candidate_votes.get_mut(vote.1.0) {
            valid_vote_sum += 1;
            candidate_vote.0 += 1;
            valid_weight_sum += vote.1.1;
            candidate_vote.1 += vote.1.1;
        }
    }

    Ok(ok(json!({
        "vote_meta": vote_meta_row,
        "vote_sum": vote_sum,
        "valid_vote_sum": valid_vote_sum,
        "weight_sum": weight_sum,
        "valid_weight_sum": valid_weight_sum,
        "candidate_votes": candidate_votes
    })))
}

#[test]
fn test_unsigned_bytes() {
    let msg = CreateVoteMetaParams {
        proposal_uri: "".to_string(),
        candidates: vec![],
        start_time: 1,
        end_time: 1,
        timestamp: chrono::Utc::now().timestamp(),
    };
    let unsigned_bytes = serde_ipld_dagcbor::to_vec(&msg).unwrap();
    println!("unsigned_bytes: {:?}", unsigned_bytes);
    println!("unsigned_bytes: {}", hex::encode(&unsigned_bytes));
}

pub async fn build_vote_meta(
    state: &AppView,
    vote_meta_row: &VoteMetaRow,
    proposal_hash: &[u8],
) -> Result<molecules::VoteMeta> {
    let (sql, values) = VoteWhitelist::build_select()
        .and_where(Expr::col(VoteWhitelist::Id).eq(vote_meta_row.whitelist_id.clone()))
        .build_sqlx(PostgresQueryBuilder);

    let vote_whitelist_row: VoteWhitelistRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await?;

    let mut smt_tree = CkbSMT::default();
    for lock_hash in vote_whitelist_row.list.iter() {
        if let Ok(lock_hash) = hex::decode(lock_hash)
            && let Ok(key) = TryInto::<[u8; 32]>::try_into(lock_hash.as_slice())
        {
            smt_tree
                .update(key.into(), crate::smt::SMT_VALUE.into())
                .ok();
        }
    }

    let smt_root = smt_tree.root().as_slice();
    let smt_root_hash: [u8; 32] = smt_root.try_into()?;

    Ok(molecules::VoteMeta::new_builder()
        .candidates(molecules::StringVec::from(
            vote_meta_row
                .candidates
                .iter()
                .map(|c| molecules::String::from(c.as_bytes().to_vec()))
                .collect::<Vec<molecules::String>>(),
        ))
        .smt_root_hash(
            molecules::Bytes32Opt::new_builder()
                .set(Some(smt_root_hash.into()))
                .build(),
        )
        .start_time(
            molecules::Uint64::new_builder()
                .set::<[molecule::prelude::Byte; 8]>(
                    vote_meta_row.start_time.to_be_bytes().map(|b| b.into()),
                )
                .build(),
        )
        .end_time(
            molecules::Uint64::new_builder()
                .set::<[molecule::prelude::Byte; 8]>(
                    vote_meta_row.end_time.to_be_bytes().map(|b| b.into()),
                )
                .build(),
        )
        .extra(
            molecules::BytesOpt::new_builder()
                .set(Some(proposal_hash.to_vec().into()))
                .build(),
        )
        .build())
}
