use chrono::NaiveDateTime;
use color_eyre::Result;
use sea_query::{ColumnDef, Expr, ExprTrait, Iden, OnConflict, PostgresQueryBuilder, Table};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use sqlx::{Executor, FromRow, Pool, Postgres, query, query_as_with, query_with};

#[derive(Iden, Debug, Clone, Copy)]
pub enum CursorState {
    Table,
    Id,
    Name,
    Seq,
    Updated,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct CursorStateRow {
    pub id: i32,
    pub name: String,
    pub seq: i64,
    pub updated: NaiveDateTime,
}

impl CursorState {
    pub async fn init(db: &Pool<Postgres>) -> Result<()> {
        let sql = Table::create()
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
                ColumnDef::new(Self::Name)
                    .string_len(64)
                    .not_null()
                    .unique_key(),
            )
            .col(
                ColumnDef::new(Self::Seq)
                    .big_integer()
                    .not_null()
                    .default(0),
            )
            .col(
                ColumnDef::new(Self::Updated)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .build(PostgresQueryBuilder);

        db.execute(query(&sql)).await?;

        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([Self::Name, Self::Seq])
            .values(["relayer".into(), 0i64.into()])?
            .on_conflict(OnConflict::column(Self::Name).do_nothing().to_owned())
            .build_sqlx(PostgresQueryBuilder);

        db.execute(query_with(&sql, values)).await?;

        Ok(())
    }

    pub async fn get_seq(db: &Pool<Postgres>, name: &str) -> Result<i64> {
        let (sql, values) = sea_query::Query::select()
            .column(Self::Seq)
            .from(Self::Table)
            .and_where(Expr::col(Self::Name).eq(name))
            .build_sqlx(PostgresQueryBuilder);

        let row: (i64,) = query_as_with(&sql, values).fetch_one(db).await?;

        Ok(row.0)
    }

    pub async fn set_seq(db: &Pool<Postgres>, name: &str, seq: i64) -> Result<()> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::Seq, seq.into()),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Name).eq(name))
            .build_sqlx(PostgresQueryBuilder);

        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }

    pub async fn set_seq_if_threshold(
        db: &Pool<Postgres>,
        name: &str,
        seq: i64,
        threshold: i64,
    ) -> Result<bool> {
        if seq % threshold == 0 {
            Self::set_seq(db, name, seq).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
