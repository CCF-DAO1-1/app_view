use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, ColumnType, Expr, Iden, OnConflict, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use sqlx::{Executor, Pool, Postgres, query, query_with};

#[derive(Iden, Debug, Clone, Copy)]
pub enum VoteWhitelist {
    Table,
    Id,
    List,
    RootHash,
    Created,
}

impl VoteWhitelist {
    pub async fn init(db: &Pool<Postgres>) -> Result<()> {
        let sql = sea_query::Table::create()
            .table(Self::Table)
            .if_not_exists()
            .col(ColumnDef::new(Self::Id).string().not_null().primary_key())
            .col(ColumnDef::new(Self::List).array(ColumnType::String(Default::default())))
            .col(ColumnDef::new(Self::RootHash).string().not_null())
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

    pub async fn insert(
        db: &Pool<Postgres>,
        id: &str,
        list: Vec<String>,
        root_hash: &str,
    ) -> Result<()> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([Self::Id, Self::List, Self::RootHash, Self::Created])
            .values([
                id.into(),
                list.into(),
                root_hash.into(),
                Expr::current_timestamp(),
            ])?
            .returning_col(Self::Id)
            .on_conflict(
                OnConflict::column(Self::Id)
                    .update_columns([Self::List, Self::RootHash, Self::Created])
                    .to_owned(),
            )
            .build_sqlx(PostgresQueryBuilder);
        debug!("insert exec sql: {sql}");

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub fn build_select() -> sea_query::SelectStatement {
        sea_query::Query::select()
            .columns([
                (Self::Table, Self::Id),
                (Self::Table, Self::List),
                (Self::Table, Self::RootHash),
                (Self::Table, Self::Created),
            ])
            .from(Self::Table)
            .take()
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct VoteWhitelistRow {
    pub id: String,
    pub list: Vec<String>,
    pub root_hash: String,
    pub created: DateTime<Local>,
}
