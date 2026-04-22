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
    ckb::get_ckb_addr_by_did,
    error::AppError,
    lexicon::{
        proposal::{Proposal, ProposalSample},
        vote::{Vote, VoteRow},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState},
        voter_list::{VoterList, VoterListRow},
    },
    molecules,
    scheduler::check_vote_finished::{
        build_vote_results, get_vote_end_block_number, get_vote_end_time,
    },
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
        &state.ckb_net,
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

    let weight: u64 = crate::indexer_bind::get_weight(
        state.ckb_net,
        &state.indexer_bind_url,
        &state.indexer_dao_url,
        &query.ckb_addr,
        None,
    )
    .await?
    .values()
    .sum();
    Ok(ok(json!({ "weight": weight })))
}

#[utoipa::path(get, path = "/api/vote/voter_list")]
pub async fn voter_list(State(state): State<AppView>) -> Result<impl IntoResponse, AppError> {
    let (sql, value) = VoterList::build_select()
        .order_by(VoterList::Created, Order::Desc)
        .limit(1)
        .build_sqlx(PostgresQueryBuilder);
    let row: VoterListRow = sqlx::query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("fetch voter_list failed: {e}");
            eyre!("voter list not found".to_string())
        })?;
    Ok(ok(row))
}

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct ProofQuery {
    #[validate(length(min = 1))]
    pub ckb_addr: String,
    pub voter_list_id: String,
}

#[utoipa::path(get, path = "/api/vote/proof", params(ProofQuery))]
pub async fn proof(
    State(state): State<AppView>,
    Query(query): Query<ProofQuery>,
) -> Result<impl IntoResponse, AppError> {
    get_proof(&state, &query.voter_list_id, &query.ckb_addr)
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
    voter_list_id: &str,
    ckb_addr: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let (sql, values) = VoterList::build_select()
        .and_where(Expr::col(VoterList::Id).eq(voter_list_id))
        .build_sqlx(PostgresQueryBuilder);

    let row: VoterListRow = query_as_with(&sql, values.clone())
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
        .set_network(state.ckb_net)
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
        Err(eyre!("Not in the voter_list"))
    }
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
            AppError::ExecSqlFailed(e.to_string())
        })?;

    if vote_meta_row.creator != body.did {
        return Err(AppError::ValidateFailed("not creator".to_string()));
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
            "vote_meta not already: {}",
            vote_meta_row.state
        )));
    }

    let vote_addr = get_ckb_addr_by_did(&state.ckb_client, &state.ckb_net, &body.did).await?;

    let proof = get_proof(&state, &vote_meta_row.voter_list_id, &vote_addr).await?;

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
            AppError::ExecSqlFailed(e.to_string())
        })?;

    if vote_meta_row.state != (VoteMetaState::Committed as i32)
        && vote_meta_row.state != (VoteMetaState::Finished as i32)
    {
        return Err(AppError::ValidateFailed(format!(
            "vote_meta not already: {}",
            vote_meta_row.state
        )));
    }

    let tx_hash = if let Some(tx_hash) = &vote_meta_row.tx_hash {
        tx_hash.clone()
    } else {
        return Err(AppError::ValidateFailed("vote_meta has no tx_hash".into()));
    };

    let block_number = if let Some(block_number) = &vote_meta_row.block_number {
        *block_number as u64
    } else {
        return Err(AppError::ValidateFailed("vote_meta has no tx_hash".into()));
    };

    let end_time = get_vote_end_time(&state, vote_meta_row.proposal_state, block_number).await?;
    let end_block_number = get_vote_end_block_number(&state, end_time).await?;

    let vote_results = build_vote_results(
        &state,
        Some(tx_hash),
        &vote_meta_row.candidates,
        end_time,
        end_block_number,
        false,
    )
    .await?;

    Ok(ok(json!({
        "vote_meta": vote_meta_row,
        "vote_sum": vote_results.vote_sum,
        "valid_vote_sum": vote_results.valid_vote_sum,
        "valid_weight_sum": vote_results.valid_weight_sum,
        "candidate_votes": vote_results.candidate_votes
    })))
}

#[derive(Debug, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct ListSelfQuery {
    pub did: String,
    #[validate(range(min = 1))]
    pub page: u64,
    #[validate(range(min = 1))]
    pub per_page: u64,
}

impl Default for ListSelfQuery {
    fn default() -> Self {
        Self {
            did: "".to_string(),
            page: 1,
            per_page: 20,
        }
    }
}

#[utoipa::path(get, path = "/api/vote/list_self", params(ListSelfQuery))]
pub async fn list_self(
    State(state): State<AppView>,
    Query(query): Query<ListSelfQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;
    let offset = query.per_page * (query.page - 1);
    let (sql, value) = Vote::build_select()
        .and_where(Expr::col(Vote::Voter).eq(query.did.clone()))
        .order_by(Vote::Created, Order::Desc)
        .limit(std::cmp::min(query.per_page, 100))
        .offset(offset)
        .build_sqlx(PostgresQueryBuilder);
    let rows: Vec<VoteRow> = query_as_with(&sql, value)
        .fetch_all(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::ExecSqlFailed(e.to_string())
        })?;

    // Batch fetch vote_meta to avoid N+1 queries
    let vote_meta_ids: Vec<i32> = rows.iter().map(|r| r.vote_meta_id).collect();
    let vote_meta_map = if !vote_meta_ids.is_empty() {
        let (sql, value) = VoteMeta::build_select()
            .and_where(Expr::col(VoteMeta::Id).is_in(vote_meta_ids))
            .build_sqlx(PostgresQueryBuilder);
        query_as_with::<_, VoteMetaRow, _>(&sql, value)
            .fetch_all(&state.db)
            .await
            .map_err(|e| {
                debug!("exec sql failed: {e}");
                AppError::ExecSqlFailed(e.to_string())
            })?
            .into_iter()
            .map(|r| (r.id, r))
            .collect::<std::collections::HashMap<_, _>>()
    } else {
        std::collections::HashMap::new()
    };

    // Batch fetch proposals to avoid N+1 queries
    let proposal_uris: Vec<String> = vote_meta_map
        .values()
        .map(|r| r.proposal_uri.clone())
        .collect();
    let proposal_map = if !proposal_uris.is_empty() {
        let (sql, value) = Proposal::build_sample()
            .and_where(Expr::col(Proposal::Uri).is_in(proposal_uris))
            .build_sqlx(PostgresQueryBuilder);
        query_as_with::<_, ProposalSample, _>(&sql, value)
            .fetch_all(&state.db)
            .await
            .map_err(|e| {
                debug!("exec sql failed: {e}");
                AppError::ExecSqlFailed(e.to_string())
            })?
            .into_iter()
            .map(|r| (r.uri.clone(), r))
            .collect::<std::collections::HashMap<_, _>>()
    } else {
        std::collections::HashMap::new()
    };

    let mut views = vec![];
    for row in &rows {
        let mut view = json!(row);
        if let Some(vote_meta_row) = vote_meta_map.get(&row.vote_meta_id) {
            view["vote_meta"] = json!(vote_meta_row);
            if let Some(proposal_row) = proposal_map.get(&vote_meta_row.proposal_uri) {
                view["proposal"] = json!(proposal_row);
            }
        }
        views.push(view);
    }

    let (sql, value) = sea_query::Query::select()
        .expr(Expr::col((Vote::Table, Vote::Id)).count_distinct())
        .from(Vote::Table)
        .and_where(Expr::col(Vote::Voter).eq(query.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let total: (i64,) = query_as_with(&sql, value.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    Ok(ok(json!({
        "rows": views,
        "page": query.page,
        "per_page": query.per_page,
        "total":  total.0
    })))
}

pub async fn build_vote_meta(
    db: &sqlx::Pool<sqlx::Postgres>,
    vote_meta_row: &VoteMetaRow,
    proposal_hash: &[u8],
) -> Result<molecules::VoteMeta> {
    let (sql, values) = VoterList::build_select()
        .and_where(Expr::col(VoterList::Id).eq(vote_meta_row.voter_list_id.clone()))
        .build_sqlx(PostgresQueryBuilder);

    let voter_list_row: VoterListRow = query_as_with(&sql, values.clone()).fetch_one(db).await?;

    let mut smt_tree = CkbSMT::default();
    for lock_hash in voter_list_row.list.iter() {
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
