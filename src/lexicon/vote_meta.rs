use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, ColumnType, Expr, ExprTrait, Iden, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, Row, query, query_with};
use utoipa::ToSchema;

use crate::lexicon::proposal::ProposalState;

#[derive(Iden, Debug, Clone, Copy)]
pub enum VoteMeta {
    Table,
    Id,
    ProposalState,
    State,
    TxHash,
    ProposalUri,
    WhitelistId,
    Candidates,
    StartTime,
    EndTime,
    Creator,
    Results,
    Created,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ToSchema)]
pub enum VoteMetaState {
    /// 0 等待发送交易
    #[default]
    Waiting = 0,
    /// 1 已提交交易
    Committed = 1,
    /// 2 交易超时
    Timeout = 2,
    /// 3 交易被拒绝
    Rejected = 3,
    /// 4 投票已结束
    Finished = 4,
}

impl VoteMeta {
    pub async fn init(db: &Pool<Postgres>) -> Result<()> {
        let sql = sea_query::Table::create()
            .table(Self::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Self::Id)
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .col(
                ColumnDef::new(Self::ProposalState)
                    .integer()
                    .not_null()
                    .default(ProposalState::InitiationVote as i32),
            )
            .col(ColumnDef::new(Self::State).integer().not_null().default(0))
            .col(ColumnDef::new(Self::TxHash).string())
            .col(ColumnDef::new(Self::ProposalUri).string().not_null())
            .col(ColumnDef::new(Self::WhitelistId).string().not_null())
            .col(ColumnDef::new(Self::Candidates).array(ColumnType::String(Default::default())))
            .col(ColumnDef::new(Self::StartTime).big_integer().not_null())
            .col(ColumnDef::new(Self::EndTime).big_integer().not_null())
            .col(
                ColumnDef::new(Self::Creator)
                    .string()
                    .not_null()
                    .default(""),
            )
            .col(ColumnDef::new(Self::Results).json_binary())
            .col(
                ColumnDef::new(Self::Created)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .build(PostgresQueryBuilder);
        db.execute(query(&sql)).await?;
        Ok(())
    }

    pub async fn insert(db: &Pool<Postgres>, row: &VoteMetaRow) -> Result<i32> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::ProposalState,
                Self::State,
                Self::TxHash,
                Self::ProposalUri,
                Self::WhitelistId,
                Self::Candidates,
                Self::StartTime,
                Self::EndTime,
                Self::Creator,
                Self::Results,
                Self::Created,
            ])
            .values([
                row.proposal_state.into(),
                row.state.into(),
                row.tx_hash.clone().into(),
                row.proposal_uri.clone().into(),
                row.whitelist_id.clone().into(),
                row.candidates.clone().into(),
                row.start_time.into(),
                row.end_time.into(),
                row.creator.clone().into(),
                row.results.clone().into(),
                Expr::current_timestamp(),
            ])?
            .returning_col(Self::Id)
            .build_sqlx(PostgresQueryBuilder);
        sqlx::query_with(&sql, values)
            .fetch_one(db)
            .await
            .and_then(|r| r.try_get(0))
            .map_err(|e| color_eyre::eyre::eyre!(e))
    }

    pub async fn update_tx_hash(db: &Pool<Postgres>, id: i32, tx_hash: &str) -> Result<()> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .value(Self::TxHash, tx_hash)
            .and_where(Expr::col(Self::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub async fn update_results(db: &Pool<Postgres>, id: i32, results: Value) -> Result<()> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::Results, results.into()),
                (Self::State, (VoteMetaState::Finished as i32).into()),
            ])
            .and_where(Expr::col(Self::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub fn build_select() -> sea_query::SelectStatement {
        sea_query::Query::select()
            .columns([
                (Self::Table, Self::Id),
                (Self::Table, Self::ProposalState),
                (Self::Table, Self::State),
                (Self::Table, Self::TxHash),
                (Self::Table, Self::ProposalUri),
                (Self::Table, Self::WhitelistId),
                (Self::Table, Self::Candidates),
                (Self::Table, Self::StartTime),
                (Self::Table, Self::EndTime),
                (Self::Table, Self::Creator),
                (Self::Table, Self::Results),
                (Self::Table, Self::Created),
            ])
            .from(Self::Table)
            .take()
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct VoteMetaRow {
    pub id: i32,
    pub proposal_state: i32,
    pub state: i32,
    pub tx_hash: Option<String>,
    pub proposal_uri: String,
    pub whitelist_id: String,
    pub candidates: Vec<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub creator: String,
    pub results: Option<Value>,
    pub created: DateTime<Local>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VoteResults {
    pub vote_sum: u64,
    pub valid_vote_sum: u64,
    pub weight_sum: u64,
    pub valid_weight_sum: u64,
    pub valid_votes: Vec<Vec<(String, u64)>>,
    pub candidate_votes: Vec<u64>,
    pub result: Option<VoteResult>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub enum VoteResult {
    Voting = 0,
    Agree,
    AgreeLessThan51PCT,
    AgreeLessThan67PCT,
    TotalLessThan185000000CKB,
    TotalLessThan3X,
    AgainstMoreThan51PCT,
    AgainstMoreThan67PCT,
    Failed,
}
