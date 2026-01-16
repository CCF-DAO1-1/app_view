use chrono::{DateTime, Local};
use color_eyre::{Result, eyre::eyre};
use sea_query::{ColumnDef, Expr, ExprTrait, Iden, OnConflict, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, query, query_as_with, query_with};
use utoipa::ToSchema;

use crate::lexicon::vote_meta::VoteMetaRow;

#[derive(Debug, Clone, Copy, Default, ToSchema)]
pub enum ProposalState {
    End = 0,

    /// 1 草稿
    #[default]
    Draft,

    /// 2 立项投票
    InitiationVote,

    /// 3 等待启动金
    WaitingForStartFund,

    /// 4 项目执行中：里程碑过程
    InProgress,

    /// 5 里程碑验收投票
    MilestoneVote,

    /// 6 延期投票
    DelayVote,

    /// 7 等待启动金
    WaitingForMilestoneFund,

    /// 8 等待验收报告
    WaitingForAcceptanceReport,

    /// 9 项目完成
    Completed,

    /// 10 等待复核
    WaitingReexamine,

    /// 11 复核投票
    ReexamineVote,

    /// 12 整改投票
    RectificationVote,

    /// 13 等待整改
    WaitingRectification,
}

impl ProposalState {
    pub const fn from(value: i32) -> Self {
        match value {
            0 => ProposalState::End,
            1 => ProposalState::Draft,
            2 => ProposalState::InitiationVote,
            3 => ProposalState::WaitingForStartFund,
            4 => ProposalState::InProgress,
            5 => ProposalState::MilestoneVote,
            6 => ProposalState::DelayVote,
            7 => ProposalState::WaitingForMilestoneFund,
            8 => ProposalState::WaitingForAcceptanceReport,
            9 => ProposalState::Completed,
            10 => ProposalState::WaitingReexamine,
            11 => ProposalState::ReexamineVote,
            12 => ProposalState::RectificationVote,
            13 => ProposalState::WaitingRectification,
            _ => ProposalState::Draft,
        }
    }
}

#[derive(Iden, Debug, Clone, Copy)]
pub enum Proposal {
    Table,
    Uri,
    Cid,
    Repo,
    Record,
    State,
    Progress,
    Updated,
    ReceiverAddr,
}

impl Proposal {
    pub async fn init(db: &Pool<Postgres>) -> Result<()> {
        let sql = sea_query::Table::create()
            .table(Self::Table)
            .if_not_exists()
            .col(ColumnDef::new(Self::Uri).string().not_null().primary_key())
            .col(ColumnDef::new(Self::Cid).string().not_null())
            .col(ColumnDef::new(Self::Repo).string().not_null())
            .col(ColumnDef::new(Self::Record).json_binary().default("{}"))
            .col(
                ColumnDef::new(Self::State)
                    .integer()
                    .not_null()
                    .default(ProposalState::Draft as i32),
            )
            .col(
                ColumnDef::new(Self::Progress)
                    .integer()
                    .not_null()
                    .default(0),
            )
            .col(
                ColumnDef::new(Self::Updated)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(ColumnDef::new(Self::ReceiverAddr).string())
            .build(PostgresQueryBuilder);
        db.execute(query(&sql)).await?;

        let sql = sea_query::Table::alter()
            .table(Self::Table)
            .add_column_if_not_exists(
                ColumnDef::new(Self::Progress)
                    .integer()
                    .not_null()
                    .default(0),
            )
            .build(PostgresQueryBuilder);
        db.execute(query(&sql)).await?;
        Ok(())
    }

    pub async fn insert(
        db: &Pool<Postgres>,
        repo: &str,
        record: Value,
        uri: &str,
        cid: &str,
    ) -> Result<()> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::Uri,
                Self::Cid,
                Self::Repo,
                Self::Record,
                Self::Updated,
            ])
            .values([
                uri.into(),
                cid.into(),
                repo.into(),
                record.into(),
                Expr::current_timestamp(),
            ])?
            .returning_col(Self::Uri)
            .on_conflict(
                OnConflict::column(Self::Uri)
                    .update_columns([Self::Cid, Self::Repo, Self::Record, Self::Updated])
                    .to_owned(),
            )
            .build_sqlx(PostgresQueryBuilder);

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub async fn update(db: &Pool<Postgres>, record: Value, uri: &str, cid: &str) -> Result<()> {
        let (sql, values) = Proposal::build_select(None)
            .and_where(Expr::col(Proposal::Uri).eq(uri))
            .build_sqlx(PostgresQueryBuilder);

        let proposal_row: ProposalRow = query_as_with(&sql, values.clone()).fetch_one(db).await?;

        // check proposal state
        if proposal_row.state != (ProposalState::Draft as i32)
            && proposal_row.state != (ProposalState::WaitingRectification as i32)
        {
            return Err(eyre!(format!(
                "proposal cannot be updated in {}",
                proposal_row.state
            ),));
        }

        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::Cid, cid.into()),
                (Self::Record, record.into()),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Uri).eq(uri))
            .returning_col(Self::Uri)
            .build_sqlx(PostgresQueryBuilder);

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub fn build_select(viewer: Option<String>) -> sea_query::SelectStatement {
        sea_query::Query::select()
        .columns([
            (Proposal::Table, Proposal::Uri),
            (Proposal::Table, Proposal::Cid),
            (Proposal::Table, Proposal::Repo),
            (Proposal::Table, Proposal::Record),
            (Proposal::Table, Proposal::Progress),
            (Proposal::Table, Proposal::State),
            (Proposal::Table, Proposal::Updated),
            (Proposal::Table, Proposal::ReceiverAddr),
        ])
        .expr(Expr::cust("(select count(\"like\".\"uri\") from \"like\" where \"like\".\"to\" = \"proposal\".\"uri\") as like_count"))
        .expr(if let Some(viewer) = viewer {
            Expr::cust(format!("((select count(\"like\".\"uri\") from \"like\" where \"like\".\"repo\" = '{viewer}' and \"like\".\"to\" = \"proposal\".\"uri\" ) > 0) as liked"))
        } else {
            Expr::cust("false as liked".to_string())
        })
        .from(Proposal::Table)
        .take()
    }

    pub fn build_sample() -> sea_query::SelectStatement {
        sea_query::Query::select()
            .columns([
                (Proposal::Table, Proposal::Uri),
                (Proposal::Table, Proposal::Cid),
                (Proposal::Table, Proposal::Repo),
                (Proposal::Table, Proposal::Record),
                (Proposal::Table, Proposal::Progress),
                (Proposal::Table, Proposal::State),
                (Proposal::Table, Proposal::ReceiverAddr),
                (Proposal::Table, Proposal::Updated),
            ])
            .from(Proposal::Table)
            .take()
    }

    pub async fn update_state(db: &Pool<Postgres>, uri: &str, state: i32) -> Result<u64> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::State, state.into()),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Uri).eq(uri))
            .build_sqlx(PostgresQueryBuilder);

        let lines = db.execute(query_with(&sql, values)).await?.rows_affected();
        Ok(lines)
    }

    pub async fn update_progress(
        db: &Pool<Postgres>,
        uri: &str,
        state: i32,
        progress: i32,
    ) -> Result<u64> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::State, state.into()),
                (Self::Progress, progress.into()),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Uri).eq(uri))
            .build_sqlx(PostgresQueryBuilder);

        let lines = db.execute(query_with(&sql, values)).await?.rows_affected();
        Ok(lines)
    }

    pub async fn update_receiver_addr(
        db: &Pool<Postgres>,
        uri: &str,
        receiver_addr: &str,
    ) -> Result<u64> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::ReceiverAddr, receiver_addr.into()),
                (
                    Self::State,
                    (ProposalState::WaitingForStartFund as i32).into(),
                ),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Uri).eq(uri))
            .build_sqlx(PostgresQueryBuilder);

        let lines = db.execute(query_with(&sql, values)).await?.rows_affected();
        Ok(lines)
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct ProposalSample {
    pub uri: String,
    pub cid: String,
    pub repo: String,
    pub record: Value,
    pub progress: i32,
    pub state: i32,
    pub receiver_addr: Option<String>,
    pub updated: DateTime<Local>,
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct ProposalRow {
    pub uri: String,
    pub cid: String,
    pub repo: String,
    pub record: Value,
    pub progress: i32,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub receiver_addr: Option<String>,
    pub like_count: i64,
    pub liked: bool,
}

#[derive(Debug, Serialize)]
pub struct ProposalView {
    pub uri: String,
    pub cid: String,
    pub author: Value,
    pub record: Value,
    pub progress: i32,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub receiver_addr: Option<String>,
    pub like_count: String,
    pub liked: bool,
    pub vote_meta: Option<VoteMetaRow>,
}

impl ProposalView {
    pub fn build(row: ProposalRow, author: Value, vote_meta: Option<VoteMetaRow>) -> Self {
        Self {
            uri: row.uri,
            cid: row.cid,
            author,
            record: row.record,
            updated: row.updated,
            receiver_addr: row.receiver_addr,
            progress: row.progress,
            state: row.state,
            like_count: row.like_count.to_string(),
            liked: row.liked,
            vote_meta,
        }
    }
}

pub fn has_next_milestone(proposal_sample: &ProposalSample) -> Option<(usize, Value)> {
    if let Some(milestones) = proposal_sample
        .record
        .pointer("/data/milestones")
        .and_then(|m| m.as_array())
    {
        let next_index = proposal_sample.progress as usize + 1;
        return milestones.get(next_index).map(|m| (next_index, m.clone()));
    }
    None
}
