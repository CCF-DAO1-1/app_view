use chrono::{DateTime, Local};
use color_eyre::{Result, eyre::OptionExt};
use sea_query::{ColumnDef, Expr, Iden, OnConflict, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, query, query_with};

#[derive(Iden, Debug, Clone, Copy)]
pub enum Reply {
    Table,
    Uri,
    Cid,
    Repo,
    Proposal,
    To,
    Text,
    Updated,
    Created,
}

impl Reply {
    pub async fn init(db: &Pool<Postgres>) -> Result<()> {
        let sql = sea_query::Table::create()
            .table(Self::Table)
            .if_not_exists()
            .col(ColumnDef::new(Self::Uri).string().not_null().primary_key())
            .col(ColumnDef::new(Self::Cid).string().not_null())
            .col(ColumnDef::new(Self::Repo).string().not_null())
            .col(ColumnDef::new(Self::Proposal).string().not_null())
            .col(
                ColumnDef::new(Self::To)
                    .string()
                    .not_null()
                    .default("".to_string()),
            )
            .col(ColumnDef::new(Self::Text).string().not_null())
            .col(
                ColumnDef::new(Self::Updated)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
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
        repo: &str,
        reply: &Value,
        uri: &str,
        cid: &str,
    ) -> Result<()> {
        let proposal = reply["proposal"]
            .as_str()
            .map(|s| s.trim_matches('\"'))
            .ok_or_eyre("error in proposal")?;
        let to = reply["to"]
            .as_str()
            .map(|s| s.trim_matches('\"'))
            .unwrap_or_default();
        let text = reply["text"]
            .as_str()
            .map(|s| s.trim_matches('\"'))
            .ok_or_eyre("error in text")?;
        let created = reply["created"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .ok_or_eyre("error in created")?;
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::Uri,
                Self::Cid,
                Self::Repo,
                Self::Proposal,
                Self::To,
                Self::Text,
                Self::Updated,
                Self::Created,
            ])
            .values([
                uri.into(),
                cid.into(),
                repo.into(),
                proposal.into(),
                to.into(),
                text.into(),
                Expr::current_timestamp(),
                created.into(),
            ])?
            .returning_col(Self::Uri)
            .on_conflict(
                OnConflict::column(Self::Uri)
                    .update_columns([
                        Self::Cid,
                        Self::Repo,
                        Self::Proposal,
                        Self::To,
                        Self::Text,
                        Self::Updated,
                    ])
                    .to_owned(),
            )
            .build_sqlx(PostgresQueryBuilder);
        db.execute(query_with(&sql, values)).await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct ReplyRow {
    pub uri: String,
    pub cid: String,
    pub repo: String,
    pub proposal: String,
    pub to: String,
    pub text: String,
    pub updated: DateTime<Local>,
    pub created: DateTime<Local>,
    pub like_count: i64,
    pub liked: bool,
}

#[derive(Debug, Serialize)]
pub struct ReplyView {
    pub uri: String,
    pub cid: String,
    pub author: Value,
    pub proposal: String,
    pub to: Value,
    pub text: String,
    pub updated: DateTime<Local>,
    pub created: DateTime<Local>,
    pub like_count: String,
    pub liked: bool,
}
