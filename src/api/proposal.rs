use color_eyre::eyre::eyre;
use common_x::restful::{
    axum::{
        Json,
        extract::{Query, State},
        response::IntoResponse,
    },
    ok, ok_simple,
};
use molecule::prelude::Entity;
use sea_query::{BinOper, Expr, ExprTrait, Func, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::query_as_with;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::{
    AppView,
    api::{SignedBody, SignedParam, ToTimestamp, build_author, vote::build_vote_meta},
    ckb::get_vote_time_range,
    error::AppError,
    lexicon::{
        administrator::{Administrator, AdministratorRow},
        proposal::{Proposal, ProposalRow, ProposalSample, ProposalState, ProposalView},
        task::{Task, TaskRow, TaskState, TaskType},
        timeline::{Timeline, TimelineRow, TimelineType},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState, VoteResult, VoteResults},
        vote_whitelist::{VoteWhitelist, VoteWhitelistRow},
    },
};

#[derive(Debug, Validate, Deserialize, ToSchema)]
#[serde(default)]
pub struct ProposalQuery {
    /// pagination cursor (usually timestamp of the last item seen)
    pub cursor: Option<String>,
    /// number of items to return
    pub limit: u64,
    /// search keyword
    pub q: Option<String>,
    /// filter by user's DID
    pub repo: Option<String>,
    /// viewer's DID
    pub viewer: Option<String>,
}

impl Default for ProposalQuery {
    fn default() -> Self {
        Self {
            cursor: Default::default(),
            limit: 20,
            q: Default::default(),
            repo: Default::default(),
            viewer: Default::default(),
        }
    }
}

#[utoipa::path(post, path = "/api/proposal/list")]
pub async fn list(
    State(state): State<AppView>,
    Json(query): Json<ProposalQuery>,
) -> Result<impl IntoResponse, AppError> {
    let (sql, values) = Proposal::build_select(query.viewer)
        .and_where_option(
            query
                .repo
                .map(|repo| Expr::col((Proposal::Table, Proposal::Repo)).eq(repo)),
        )
        .and_where_option(
            query
                .cursor
                .and_then(|cursor| cursor.parse::<i64>().ok())
                .map(|cursor| {
                    Expr::col((Proposal::Table, Proposal::Updated)).binary(
                        BinOper::SmallerThan,
                        Func::cust(ToTimestamp).args([Expr::val(cursor)]),
                    )
                }),
        )
        .order_by(Proposal::Updated, Order::Desc)
        .limit(query.limit)
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<ProposalRow> = query_as_with(&sql, values.clone())
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    let mut views = vec![];
    for row in rows {
        let author = build_author(&state, &row.repo).await;
        views.push(ProposalView::build(row, author, None));
    }
    let cursor = views.last().map(|r| r.updated.timestamp());
    let result = if let Some(cursor) = cursor {
        json!({
            "cursor": cursor.to_string(),
            "proposals": views
        })
    } else {
        json!({
            "proposals": views
        })
    };
    Ok(ok(result))
}

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct UriQuery {
    #[validate(length(min = 1))]
    /// record uri
    pub uri: String,
    /// viewer's DID
    pub viewer: Option<String>,
}

#[utoipa::path(get, path = "/api/proposal/detail", params(UriQuery))]
pub async fn detail(
    State(state): State<AppView>,
    Query(query): Query<UriQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, values) = Proposal::build_select(query.viewer)
        .and_where(Expr::col(Proposal::Uri).eq(query.uri))
        .build_sqlx(PostgresQueryBuilder);

    let row: ProposalRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;
    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(&row.uri))
        .and_where(Expr::col(VoteMeta::ProposalState).eq(row.state))
        .and_where(
            Expr::col(VoteMeta::State)
                .eq(VoteMetaState::Waiting as i32)
                .or(Expr::col(VoteMeta::State).eq(VoteMetaState::Committed as i32))
                .or(Expr::col(VoteMeta::State).eq(VoteMetaState::Finished as i32)),
        )
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row: Option<VoteMetaRow> = query_as_with::<_, VoteMetaRow, _>(&sql, value)
        .fetch_one(&state.db)
        .await
        .ok();

    let author = build_author(&state, &row.repo).await;
    let view = ProposalView::build(row, author, vote_meta_row);

    Ok(ok(view))
}

#[derive(Debug, Default, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct StateQuery {
    #[validate(length(min = 1))]
    /// record uri
    pub uri: String,
    /// proposal state
    pub state: i32,
}

#[utoipa::path(
    post,
    path = "/api/proposal/update_state",
    params(StateQuery),
    description = "方便调试用的，请勿随意调用"
)]
pub async fn update_state(
    State(state): State<AppView>,
    Query(query): Query<StateQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let lines = Proposal::update_state(&state.db, &query.uri, query.state)
        .await
        .map_err(|e| {
            debug!("update_state failed: {e}");
            AppError::NotFound
        })?;

    if lines == 0 {
        return Err(AppError::NotFound);
    }

    Ok(ok_simple())
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct InitiationParams {
    pub proposal_uri: String,
    pub timestamp: i64,
}

impl SignedParam for InitiationParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[test]
fn test_timestamp() {
    let timestamp = chrono::Utc::now() + chrono::Duration::minutes(5);
    println!("timestamp: {}", timestamp);
    let now = chrono::Utc::now();
    println!("now: {}", now);
    let delta = (now - timestamp).abs();
    println!("delta: {}", delta);
    if delta < chrono::Duration::minutes(5) {
        println!("valid");
    } else {
        println!("invalid");
    }
}

#[utoipa::path(
    post,
    path = "/api/proposal/initiation_vote",
    params(StateQuery),
    description = "发起立项投票"
)]
pub async fn initiation_vote(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<InitiationParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let SignedBody::<InitiationParams> { params, did, .. } = body;

    let (sql, values) = Proposal::build_select(None)
        .and_where(Expr::col(Proposal::Uri).eq(&params.proposal_uri))
        .build_sqlx(PostgresQueryBuilder);

    let proposal_row: ProposalRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;

    // check proposal owner
    if proposal_row.repo != did {
        return Err(AppError::ValidateFailed("not proposal owner".to_string()));
    }

    // check proposal state
    if proposal_row.state != (ProposalState::Draft as i32) {
        return Err(AppError::ValidateFailed(
            "proposal state not draft".to_string(),
        ));
    }

    // TODO check AMA completed

    // check proposaler's weight > 10_000_000_000_000
    let ckb_addr = crate::ckb::get_ckb_addr_by_did(&state.ckb_client, &did).await?;
    // TODO: use ckb
    let weight =
        crate::indexer_bind::get_weight(&state.ckb_client, &state.indexer_bind_url, &ckb_addr)
            .await?;
    if weight < 10_000_000_000_000 {
        return Err(AppError::ValidateFailed(
            "not enough weight(At least 100_000 ckb)".to_string(),
        ));
    }

    // create vote_meta
    let proposal_hash = ckb_hash::blake2b_256(serde_json::to_vec(&proposal_row.uri)?);

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(&proposal_row.uri))
        .and_where(Expr::col(VoteMeta::ProposalState).eq(ProposalState::InitiationVote as i32))
        .and_where(Expr::col(VoteMeta::State).eq(VoteMetaState::Waiting as i32))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row = if let Ok(vote_meta_row) = query_as_with::<_, VoteMetaRow, _>(&sql, value)
        .fetch_one(&state.db)
        .await
    {
        vote_meta_row
    } else {
        let (sql, value) = VoteWhitelist::build_select()
            .order_by(VoteWhitelist::Created, Order::Desc)
            .limit(1)
            .build_sqlx(PostgresQueryBuilder);
        let vote_whitelist_row: VoteWhitelistRow = query_as_with(&sql, value)
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                debug!("fetch vote_whitelist failed: {e}");
                AppError::ValidateFailed("vote whitelist not found".to_string())
            })?;
        // TODO
        let time_range = get_vote_time_range(&state.ckb_client, 7).await?;
        let time_range = crate::ckb::test_get_vote_time_range(&state.ckb_client).await?;
        let mut vote_meta_row = VoteMetaRow {
            id: -1,
            proposal_state: ProposalState::InitiationVote as i32,
            state: 0,
            tx_hash: None,
            proposal_uri: params.proposal_uri.clone(),
            whitelist_id: vote_whitelist_row.id,
            candidates: vec![
                "Abstain".to_string(),
                "Agree".to_string(),
                "Against".to_string(),
            ],
            start_time: time_range.0 as i64,
            end_time: time_range.1 as i64,
            creater: did.clone(),
            results: None,
            created: chrono::Local::now(),
        };

        vote_meta_row.id = VoteMeta::insert(&state.db, &vote_meta_row).await?;
        vote_meta_row
    };

    let outputs_data = if vote_meta_row.tx_hash.is_none() {
        let vote_meta = build_vote_meta(&state, &vote_meta_row, &proposal_hash).await?;

        let vote_meta_bytes = vote_meta.as_bytes().to_vec();
        let vote_meta_hex = hex::encode(vote_meta_bytes);

        vec![vote_meta_hex]
    } else {
        vec![]
    };

    Task::complete(&state.db, &proposal_row.uri, TaskType::InitiationVote, &did)
        .await
        .ok();

    Ok(ok(json!({
        "vote_meta": vote_meta_row,
        "outputsData": outputs_data
    })))
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct ReceiverAddrParams {
    pub proposal_uri: String,
    pub receiver_addr: String,
    pub timestamp: i64,
}

impl SignedParam for ReceiverAddrParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(
    post,
    path = "/api/proposal/update_receiver_addr",
    description = "更新项目金库地址"
)]
pub async fn update_receiver_addr(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<ReceiverAddrParams>>,
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

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(body.params.proposal_uri.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("proposal not found: {e}")))?;

    if proposal_sample.state != (ProposalState::InitiationVote as i32) {
        return Err(AppError::ValidateFailed(
            "only InitiationVote state can update receiver addr".to_string(),
        ));
    }

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(&body.params.proposal_uri))
        .and_where(Expr::col(VoteMeta::ProposalState).eq(ProposalState::InitiationVote as i32))
        .and_where(Expr::col(VoteMeta::State).eq(VoteMetaState::Finished as i32))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row: VoteMetaRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("vote meta not found: {e}")))?;

    if vote_result(&vote_meta_row, &proposal_sample) != VoteResult::Agree {
        return Err(AppError::ValidateFailed(
            "only Agree vote result can update receiver addr".to_string(),
        ));
    }

    Proposal::update_receiver_addr(
        &state.db,
        &body.params.proposal_uri,
        &body.params.receiver_addr,
    )
    .await?;

    let admins = Administrator::fetch_all(&state.db)
        .await
        .iter()
        .map(|admin| admin.did.clone())
        .collect();
    Task::insert(
        &state.db,
        &TaskRow {
            id: 0,
            task_type: TaskType::SendInitialFund as i32,
            message: "SendInitialFund".to_string(),
            target: body.params.proposal_uri.clone(),
            operators: admins,
            processor: None,
            deadline: chrono::Local::now() + chrono::Duration::days(7),
            state: TaskState::Unread as i32,
            updated: chrono::Local::now(),
            created: chrono::Local::now(),
        },
    )
    .await
    .map_err(|e| error!("insert task failed: {e}"))
    .ok();

    Task::complete(
        &state.db,
        &body.params.proposal_uri,
        TaskType::UpdateReceiverAddr,
        &body.did,
    )
    .await
    .ok();

    Timeline::insert(
        &state.db,
        &TimelineRow {
            id: 0,
            timeline_type: TimelineType::UpdateReceiverAddr as i32,
            message: json!({
                "receiver_addr": body.params.receiver_addr,
            })
            .to_string(),
            target: body.params.proposal_uri.clone(),
            operator: body.did.clone(),
            timestamp: chrono::Local::now(),
        },
    )
    .await
    .map_err(|e| error!("insert timeline failed: {e}"))
    .ok();

    Ok(ok_simple())
}

pub fn vote_result(vote_meta: &VoteMetaRow, proposal: &ProposalSample) -> VoteResult {
    if let Some(results) = &vote_meta.results
        && let Ok(results) = serde_json::from_value::<VoteResults>(results.clone())
        && let Some(proposal_type) = proposal
            .record
            .pointer("/data/proposalType")
            .and_then(|t| t.as_str())
    {
        return calculate_vote_result(vote_meta.proposal_state, proposal, results, proposal_type);
    }
    VoteResult::Voting
}

pub fn calculate_vote_result(
    proposal_state: i32,
    proposal: &ProposalSample,
    results: VoteResults,
    proposal_type: &str,
) -> VoteResult {
    debug!(
        "calculate_vote_result: proposal_type: {proposal_type}, proposal_state: {proposal_state}",
    );
    match ProposalState::from(proposal_state) {
        ProposalState::InitiationVote | ProposalState::ReexamineVote => {
            if proposal_type == "BudgetProposal" {
                if results.valid_weight_sum >= 1_8500_0000_0000_0000 {
                    let agree = results.candidate_votes[1] as f64 / results.valid_weight_sum as f64;
                    if agree >= 0.67 {
                        return VoteResult::Agree;
                    } else {
                        return VoteResult::Against;
                    }
                } else {
                    return VoteResult::Failed;
                }
            } else if let Some(proposal_budget) = proposal
                .record
                .pointer("/data/budget")
                .and_then(|t| t.as_str())
                .and_then(|t| t.parse::<u64>().ok())
            {
                debug!("proposal_budget: {}", proposal_budget);
                debug!("valid_weight_sum: {}", results.valid_weight_sum);
                if results.valid_weight_sum >= (proposal_budget * 3_0000_0000) {
                    let agree = results.candidate_votes[1] as f64 / results.valid_weight_sum as f64;
                    if agree >= 0.51 {
                        return VoteResult::Agree;
                    } else {
                        return VoteResult::Against;
                    }
                } else {
                    return VoteResult::Failed;
                }
            }
        }
        ProposalState::MilestoneVote | ProposalState::DelayVote | ProposalState::ReviewVote => {
            if proposal_type == "BudgetProposal" {
                if results.valid_weight_sum >= 6200_0000_0000_0000 {
                    let against =
                        results.candidate_votes[2] as f64 / results.valid_weight_sum as f64;
                    if against > 0.67 {
                        return VoteResult::Against;
                    } else {
                        return VoteResult::Agree;
                    }
                } else {
                    return VoteResult::Agree;
                }
            } else if let Some(proposal_budget) = proposal
                .record
                .pointer("/data/budget")
                .and_then(|t| t.as_str())
                .and_then(|t| t.parse::<u64>().ok())
            {
                if results.valid_weight_sum >= (proposal_budget * 1_0000_0000) {
                    let against =
                        results.candidate_votes[2] as f64 / results.valid_weight_sum as f64;
                    if against > 0.51 {
                        return VoteResult::Against;
                    } else {
                        return VoteResult::Agree;
                    }
                } else {
                    return VoteResult::Agree;
                }
            }
        }
        ProposalState::RectificationVote => {
            if proposal_type == "BudgetProposal" {
                if results.valid_weight_sum >= 6200_0000_0000_0000 {
                    let agree = results.candidate_votes[1] as f64 / results.valid_weight_sum as f64;
                    if agree >= 0.67 {
                        return VoteResult::Agree;
                    } else {
                        return VoteResult::Against;
                    }
                } else {
                    return VoteResult::Against;
                }
            } else if let Some(proposal_budget) = proposal
                .record
                .pointer("/data/budget")
                .and_then(|t| t.as_str())
                .and_then(|t| t.parse::<u64>().ok())
            {
                if results.valid_weight_sum >= (proposal_budget * 1_0000_0000) {
                    let agree = results.candidate_votes[1] as f64 / results.valid_weight_sum as f64;
                    if agree >= 0.51 {
                        return VoteResult::Agree;
                    } else {
                        return VoteResult::Against;
                    }
                } else {
                    return VoteResult::Against;
                }
            }
        }
        _ => (),
    }
    VoteResult::Failed
}
