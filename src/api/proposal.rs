use color_eyre::eyre::{OptionExt, eyre};
use common_x::restful::{
    axum::{
        Json,
        extract::{Query, State},
        response::IntoResponse,
    },
    ok,
};
use sea_query::{BinOper, Expr, ExprTrait, Func, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::query_as_with;
use validator::Validate;

use crate::{
    AppView,
    api::{ToTimestamp, build_author},
    error::AppError,
    lexicon::proposal::{Proposal, ProposalRow, ProposalView},
};

#[derive(Debug, Validate, Deserialize)]
#[serde(default)]
pub struct ProposalQuery {
    pub section_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: u64,
    pub q: Option<String>,
    pub repo: Option<String>,
    pub viewer: Option<String>,
}

impl Default for ProposalQuery {
    fn default() -> Self {
        Self {
            section_id: Default::default(),
            cursor: Default::default(),
            limit: 20,
            q: Default::default(),
            repo: Default::default(),
            viewer: Default::default(),
        }
    }
}

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

#[derive(Debug, Default, Validate, Deserialize)]
#[serde(default)]
pub struct TopQuery {
    pub section_id: String,
    pub viewer: Option<String>,
}

pub async fn detail(
    State(state): State<AppView>,
    Query(query): Query<Value>,
) -> Result<impl IntoResponse, AppError> {
    let uri = query
        .get("uri")
        .and_then(|u| u.as_str())
        .ok_or_eyre("uri not be null")?;
    let viewer = query
        .get("viewer")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());

    let (sql, values) = Proposal::build_select(viewer)
        .and_where(Expr::col(Proposal::Uri).eq(uri))
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
