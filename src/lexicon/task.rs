use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, ColumnType, Expr, ExprTrait, Iden, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, Row, query};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Default, ToSchema)]
pub enum TaskType {
    #[default]
    Default = 0,

    /// 1 组织AMA
    CreateAMA,

    /// 2 提交AMA报告
    SubmitAMAReport,

    /// 3 发起立项投票
    InitiationVote,

    /// 4 维护项目金库地址
    UpdateReceiverAddr,

    /// 5 发送启动金
    SendInitialFund,

    /// 6 提交里程碑报告
    SubmitReport,

    /// 7 提交验收报告
    SubmitAcceptanceReport,

    /// 8 组织复核会议
    CreateReexamineMeeting,

    /// 9 发起复核投票
    ReexamineVote,

    /// 10 发起最终整改投票
    RectificationVote,

    /// 11 提交最终整改报告
    SubmitRectificationReport,
}

#[derive(Debug, Clone, Copy, Default, ToSchema)]
pub enum TaskState {
    /// 0 未读
    #[default]
    Unread = 0,
    /// 1 已读
    Read,
    /// 2 已完成
    Completed,
}

#[derive(Iden, Debug, Clone, Copy)]
pub enum Task {
    Table,
    Id,
    TaskType,
    Message,
    Target,
    Operators,
    Processor,
    Deadline,
    State,
    Updated,
    Created,
}

impl Task {
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
                ColumnDef::new(Self::TaskType)
                    .integer()
                    .not_null()
                    .default(TaskType::default() as i32),
            )
            .col(ColumnDef::new(Self::Message).string().not_null())
            .col(ColumnDef::new(Self::Target).string().not_null())
            .col(ColumnDef::new(Self::Operators).array(ColumnType::String(Default::default())))
            .col(ColumnDef::new(Self::Processor).string())
            .col(
                ColumnDef::new(Self::Deadline)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(
                ColumnDef::new(Self::State)
                    .integer()
                    .not_null()
                    .default(TaskState::default() as i32),
            )
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

    pub async fn insert(db: &Pool<Postgres>, row: &TaskRow) -> Result<i32> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::TaskType,
                Self::Message,
                Self::Target,
                Self::Operators,
                Self::Processor,
                Self::Deadline,
                Self::State,
                Self::Updated,
                Self::Created,
            ])
            .values([
                row.task_type.into(),
                row.message.clone().into(),
                row.target.clone().into(),
                row.operators.clone().into(),
                row.processor.clone().into(),
                row.deadline.into(),
                row.state.into(),
                Expr::current_timestamp(),
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

    pub async fn complete(
        db: &Pool<Postgres>,
        target: &str,
        t: TaskType,
        processor: &str,
    ) -> Result<i32> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::State, (TaskState::Completed as i32).into()),
                (Self::Updated, Expr::current_timestamp()),
                (Self::Processor, processor.into()),
            ])
            .and_where(Expr::col(Self::Target).eq(target))
            .and_where(Expr::col(Self::TaskType).eq(t as i32))
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
pub struct TaskRow {
    pub id: i32,
    pub task_type: i32,
    pub message: String,
    pub target: String,
    pub operators: Vec<String>,
    pub processor: Option<String>,
    pub deadline: DateTime<Local>,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub created: DateTime<Local>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct TaskView {
    pub id: i32,
    pub task_type: i32,
    pub message: String,
    pub target: Value,
    pub operators: Vec<String>,
    pub processor: Value,
    pub deadline: DateTime<Local>,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub created: DateTime<Local>,
}
