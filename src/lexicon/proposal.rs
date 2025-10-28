use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, Expr, ExprTrait, Iden, OnConflict, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, query, query_with};

#[derive(Iden, Debug, Clone, Copy)]
pub enum Proposal {
    Table,
    Uri,
    Cid,
    Repo,
    Record,
    State,
    Updated,
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
            .col(ColumnDef::new(Self::State).integer().not_null().default(1))
            .col(
                ColumnDef::new(Self::Updated)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
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
        debug!("insert exec sql: {sql}");

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
            (Proposal::Table, Proposal::State),
            (Proposal::Table, Proposal::Updated),
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

    pub async fn update_state(db: &Pool<Postgres>, uri: &str, state: i32) -> Result<u64> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::State, state.into()),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Uri).eq(uri))
            .build_sqlx(PostgresQueryBuilder);
        debug!("update_state exec sql: {sql}");

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
    pub state: i32,
    pub updated: DateTime<Local>,
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct ProposalRow {
    pub uri: String,
    pub cid: String,
    pub repo: String,
    pub record: Value,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub like_count: i64,
    pub liked: bool,
}

#[derive(Debug, Serialize)]
pub struct ProposalView {
    pub uri: String,
    pub cid: String,
    pub author: Value,
    pub record: Value,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub like_count: String,
    pub liked: bool,
}

impl ProposalView {
    pub fn build(row: ProposalRow, author: Value) -> Self {
        Self {
            uri: row.uri,
            cid: row.cid,
            author,
            record: row.record,
            updated: row.updated,
            state: row.state,
            like_count: row.like_count.to_string(),
            liked: row.liked,
        }
    }
}
