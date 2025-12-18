use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, Expr, Iden, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, Row, query};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Default, ToSchema)]
pub enum TimelineType {
    #[default]
    Default = 0,
    /// 1 创建提案
    ProposalCreated,
    /// 2 编辑提案
    ProposalEdited,
    /// 3 发起立项投票
    InitiationVote,
    /// 4 维护项目金库地址
    UpdateReceiverAddr,
}

#[derive(Iden, Debug, Clone, Copy)]
pub enum Timeline {
    Table,
    Id,
    TimelineType,
    Message,
    Target,
    Operator,
    Timestamp,
}

impl Timeline {
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
                ColumnDef::new(Self::TimelineType)
                    .integer()
                    .not_null()
                    .default(TimelineType::default() as i32),
            )
            .col(ColumnDef::new(Self::Message).string().not_null())
            .col(ColumnDef::new(Self::Target).string().not_null())
            .col(ColumnDef::new(Self::Operator).string().not_null())
            .col(
                ColumnDef::new(Self::Timestamp)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .build(PostgresQueryBuilder);
        db.execute(query(&sql)).await?;
        Ok(())
    }

    pub async fn insert(db: &Pool<Postgres>, row: &TimelineRow) -> Result<i32> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::TimelineType,
                Self::Message,
                Self::Target,
                Self::Operator,
                Self::Timestamp,
            ])
            .values([
                row.timeline_type.into(),
                row.message.clone().into(),
                row.target.clone().into(),
                row.operator.clone().into(),
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
}

#[derive(sqlx::FromRow, Debug, Serialize)]
#[allow(dead_code)]
pub struct TimelineRow {
    pub id: i32,
    pub timeline_type: i32,
    pub message: String,
    pub target: String,
    pub operator: String,
    pub timestamp: DateTime<Local>,
}

#[derive(sqlx::FromRow, Debug, Serialize)]
#[allow(dead_code)]
pub struct TimelineView {
    pub id: i32,
    pub timeline_type: i32,
    pub message: String,
    pub target: String,
    pub operator: Value,
    pub timestamp: DateTime<Local>,
}
