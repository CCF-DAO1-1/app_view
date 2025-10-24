use color_eyre::eyre::eyre;
use common_x::restful::{
    axum::{
        Json,
        extract::{Query, State},
        response::IntoResponse,
    },
    ok, ok_simple,
};
use sea_query::{BinOper, Expr, ExprTrait, Func, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Deserialize;
use serde_json::json;
use sqlx::query_as_with;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::{
    AppView,
    api::{ToTimestamp, build_author},
    error::AppError,
    lexicon::proposal::{Proposal, ProposalRow, ProposalView},
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

    debug!("sql: {sql} ({values:?})");

    let rows: Vec<ProposalRow> = query_as_with(&sql, values.clone())
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    let mut views = vec![];
    for row in rows {
        let author = build_author(&state, &row.repo).await;
        views.push(ProposalView::build(row, author));
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

    debug!("sql: {sql} ({values:?})");

    let row: ProposalRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::NotFound
        })?;

    let author = build_author(&state, &row.repo).await;
    let view = ProposalView::build(row, author);

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
