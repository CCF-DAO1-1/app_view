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
    api::{ToTimestamp, build_author, vote::build_vote_meta},
    error::AppError,
    lexicon::{
        proposal::{Proposal, ProposalRow, ProposalState, ProposalView},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState, VoteType},
    },
    verify_signature,
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
    let now = chrono::Local::now().timestamp();
    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(&row.uri))
        .and_where(
            Expr::col(VoteMeta::State)
                .eq(VoteMetaState::Waiting as i32)
                .or(Expr::col(VoteMeta::State).eq(VoteMetaState::Committed as i32)),
        )
        .and_where(Expr::col((VoteMeta::Table, VoteMeta::StartTime)).binary(
            BinOper::SmallerThanOrEqual,
            Func::cust(ToTimestamp).args([Expr::val(now)]),
        ))
        .and_where(Expr::col((VoteMeta::Table, VoteMeta::EndTime)).binary(
            BinOper::GreaterThanOrEqual,
            Func::cust(ToTimestamp).args([Expr::val(now)]),
        ))
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
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct InitiationBody {
    pub params: InitiationParams,
    pub did: String,
    #[validate(length(equal = 57))]
    pub signing_key_did: String,
    pub signed_bytes: String,
}

#[utoipa::path(
    post,
    path = "/api/proposal/initiation_vote",
    params(StateQuery),
    description = "发起立项投票"
)]
pub async fn initiation_vote(
    State(state): State<AppView>,
    Json(body): Json<InitiationBody>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let InitiationBody {
        params,
        did,
        signing_key_did,
        signed_bytes,
    } = body;

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
    if proposal_row.state != ProposalState::Draft as i32 {
        return Err(AppError::ValidateFailed(
            "proposal state not draft".to_string(),
        ));
    }

    // TODO check AMA completed

    // verify signature
    verify_signature(
        &did,
        &state.indexer_did_url,
        &signing_key_did,
        &signed_bytes,
        &params,
    )
    .await
    .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    // check proposaler's weight > 10_000_000_000_000
    let ckb_addr = crate::ckb::get_ckb_addr_by_did(&state.ckb_client, &did).await?;
    let weight = crate::indexer_bind::get_weight(&state, &ckb_addr).await?;
    if weight < 10_000_000_000_000 {
        return Err(AppError::ValidateFailed(
            "not enough weight(At least 100_000 ckb)".to_string(),
        ));
    }

    // create vote_meta
    let proposal_hash = ckb_hash::blake2b_256(serde_json::to_vec(&proposal_row.uri)?);

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(&proposal_row.uri))
        .and_where(Expr::col(VoteMeta::VoteType).eq(VoteType::Initiation as i32))
        .and_where(Expr::col(VoteMeta::State).eq(VoteMetaState::Waiting as i32))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row = if let Ok(vote_meta_row) = query_as_with::<_, VoteMetaRow, _>(&sql, value)
        .fetch_one(&state.db)
        .await
    {
        vote_meta_row
    } else {
        let now = chrono::Local::now();
        let mut vote_meta_row = VoteMetaRow {
            id: -1,
            vote_type: 0,
            state: 0,
            tx_hash: None,
            proposal_uri: params.proposal_uri.clone(),
            whitelist_id: now.format("%Y-%m-%d").to_string(),
            candidates: vec![
                "Abstain".to_string(),
                "Agree".to_string(),
                "Against".to_string(),
            ],
            start_time: now,
            end_time: now.checked_add_days(chrono::Days::new(7)).unwrap(),
            creater: did.clone(),
            created: now,
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

    Ok(ok(json!({
        "vote_meta": vote_meta_row,
        "outputsData": outputs_data
    })))
}
