use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, ColumnType, Expr, Iden, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Executor, Pool, Postgres, Row, query};

#[derive(Debug, Clone, Copy, Default)]
pub enum TaskType {
    #[default]
    Default = 0,

    // 组织AMA
    CreateAMA,

    // 提交AMA报告
    SubmitAMAReport,

    // 发起立项投票
    InitiationVote,

    // 维护项目金库地址
    UpdateReceiverAddr,

    // 发送启动金
    SendInitialFund,

    // 提交里程碑报告
    SubmitReport,

    // 提交验收报告
    SubmitAcceptanceReport,

    // 组织复核会议
    CreateReexamineMeeting,

    // 发起复核投票
    ReexamineVote,

    // 发起最终整改投票
    RectificationVote,

    // 提交最终整改报告
    SubmitRectificationReport,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TaskState {
    #[default]
    Unread = 0,
    Read,
    Completed,
}

#[derive(Iden, Debug, Clone, Copy)]
pub enum Task {
    Table,
    Id,
    TaskType,
    Importance,
    Message,
    Target,
    Operators,
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
                Self::Importance,
                Self::Message,
                Self::Target,
                Self::Operators,
                Self::Deadline,
                Self::State,
                Self::Updated,
                Self::Created,
            ])
            .values([
                row.task_type.into(),
                row.importance.into(),
                row.message.clone().into(),
                row.target.clone().into(),
                row.operators.clone().into(),
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
}

#[derive(sqlx::FromRow, Debug, Serialize)]
#[allow(dead_code)]
pub struct TaskRow {
    pub id: i32,
    pub task_type: i32,
    pub importance: i32,
    pub message: String,
    pub target: String,
    pub operators: Vec<String>,
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
    pub importance: i32,
    pub message: String,
    pub target: Value,
    pub operators: Vec<String>,
    pub deadline: DateTime<Local>,
    pub state: i32,
    pub updated: DateTime<Local>,
    pub created: DateTime<Local>,
}
