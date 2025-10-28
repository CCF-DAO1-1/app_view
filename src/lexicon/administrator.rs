use color_eyre::Result;
use sea_query::{ColumnDef, Iden, OnConflict, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use sqlx::{Executor, Pool, Postgres, query, query_with};

#[derive(Iden, Debug, Clone, Copy)]
pub enum Administrator {
    Table,
    Did,
    Permission,
}

impl Administrator {
    pub async fn init(db: &Pool<Postgres>) -> Result<()> {
        let sql = sea_query::Table::create()
            .table(Self::Table)
            .if_not_exists()
            .col(ColumnDef::new(Self::Did).string().not_null().primary_key())
            .col(ColumnDef::new(Self::Permission).integer().not_null())
            .build(PostgresQueryBuilder);
        db.execute(query(&sql)).await?;
        Ok(())
    }

    pub async fn insert(db: &Pool<Postgres>, did: &str, permission: i32) -> Result<()> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([Self::Did, Self::Permission])
            .values([did.into(), permission.into()])?
            .returning_col(Self::Did)
            .on_conflict(
                OnConflict::column(Self::Did)
                    .update_columns([Self::Permission])
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
                (Administrator::Table, Administrator::Did),
                (Administrator::Table, Administrator::Permission),
            ])
            .from(Administrator::Table)
            .take()
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct AdministratorRow {
    pub did: String,
    pub permission: i32,
}
