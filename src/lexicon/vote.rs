use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, Expr, ExprTrait, Iden, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use sqlx::{Executor, Pool, Postgres, Row, query, query_with};

#[derive(Iden, Debug, Clone, Copy)]
pub enum Vote {
    Table,
    Id,
    State,
    TxHash,
    VoteMetaId,
    CandidatesIndex,
    Voter,
    Created,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum VoteState {
    #[default]
    Waiting = 0,
    Active = 1,
    Invalid = 2,
}

impl Vote {
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
            .col(ColumnDef::new(Self::State).integer().not_null().default(0))
            .col(ColumnDef::new(Self::TxHash).string())
            .col(ColumnDef::new(Self::VoteMetaId).integer().not_null())
            .col(ColumnDef::new(Self::CandidatesIndex).integer().not_null())
            .col(ColumnDef::new(Self::Voter).string().not_null())
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

    pub async fn insert(db: &Pool<Postgres>, row: &VoteRow) -> Result<i32> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::State,
                Self::TxHash,
                Self::VoteMetaId,
                Self::CandidatesIndex,
                Self::Voter,
                Self::Created,
            ])
            .values([
                row.state.into(),
                row.tx_hash.clone().into(),
                row.vote_meta_id.into(),
                row.candidates_index.into(),
                row.voter.clone().into(),
                Expr::current_timestamp(),
            ])?
            .returning_col(Self::Id)
            .build_sqlx(PostgresQueryBuilder);
        debug!("insert exec sql: {sql}");
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
        debug!("update_tx_hash exec sql: {sql}");

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub fn build_select() -> sea_query::SelectStatement {
        sea_query::Query::select()
            .columns([
                (Self::Table, Self::Id),
                (Self::Table, Self::State),
                (Self::Table, Self::TxHash),
                (Self::Table, Self::VoteMetaId),
                (Self::Table, Self::CandidatesIndex),
                (Self::Table, Self::Voter),
                (Self::Table, Self::Created),
            ])
            .from(Self::Table)
            .take()
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct VoteRow {
    pub id: i32,
    pub state: i32,
    pub tx_hash: Option<String>,
    pub vote_meta_id: i32,
    pub candidates_index: i32,
    pub voter: String,
    pub created: DateTime<Local>,
}
