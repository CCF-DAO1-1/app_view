use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{ColumnDef, Expr, ExprTrait, Iden, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::Serialize;
use sqlx::{Executor, Pool, Postgres, Row, query};
use utoipa::ToSchema;

#[derive(Iden, Debug, Clone, Copy)]
pub enum Meeting {
    Table,
    Id,
    Title,
    StartTime,
    EndTime,
    Location,
    Url,
    Description,
    ProposalUri,
    State,
    Report,
    Creater,
    Updated,
    Created,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ToSchema)]
pub enum MeetingState {
    #[default]
    Scheduled = 0,
    Finished,
    Canceled,
}

impl Meeting {
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
            .col(ColumnDef::new(Self::Title).string().not_null())
            .col(
                ColumnDef::new(Self::StartTime)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(
                ColumnDef::new(Self::EndTime)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(
                ColumnDef::new(Self::Location)
                    .string()
                    .not_null()
                    .default(""),
            )
            .col(ColumnDef::new(Self::Url).string().not_null())
            .col(
                ColumnDef::new(Self::Description)
                    .string()
                    .not_null()
                    .default(""),
            )
            .col(ColumnDef::new(Self::ProposalUri).string().not_null())
            .col(
                ColumnDef::new(Self::State)
                    .integer()
                    .not_null()
                    .default(MeetingState::default() as i32),
            )
            .col(ColumnDef::new(Self::Report).string())
            .col(ColumnDef::new(Self::Creater).string().not_null())
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

    pub async fn insert(db: &Pool<Postgres>, row: &MeetingRow) -> Result<i32> {
        let (sql, values) = sea_query::Query::insert()
            .into_table(Self::Table)
            .columns([
                Self::Title,
                Self::StartTime,
                Self::EndTime,
                Self::Location,
                Self::Url,
                Self::Description,
                Self::ProposalUri,
                Self::State,
                Self::Report,
                Self::Creater,
                Self::Updated,
                Self::Created,
            ])
            .values([
                row.title.clone().into(),
                row.start_time.into(),
                row.end_time.into(),
                row.location.clone().into(),
                row.url.clone().into(),
                row.description.clone().into(),
                row.proposal_uri.clone().into(),
                row.state.into(),
                row.report.clone().into(),
                row.creater.clone().into(),
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

    pub fn build_select() -> sea_query::SelectStatement {
        sea_query::Query::select()
            .columns([
                (Self::Table, Self::Id),
                (Self::Table, Self::Title),
                (Self::Table, Self::StartTime),
                (Self::Table, Self::EndTime),
                (Self::Table, Self::Location),
                (Self::Table, Self::Url),
                (Self::Table, Self::Description),
                (Self::Table, Self::ProposalUri),
                (Self::Table, Self::State),
                (Self::Table, Self::Report),
                (Self::Table, Self::Creater),
                (Self::Table, Self::Updated),
                (Self::Table, Self::Created),
            ])
            .from(Self::Table)
            .take()
    }

    pub async fn update_report(db: &Pool<Postgres>, id: i32, report: &str) -> Result<()> {
        let (sql, values) = sea_query::Query::update()
            .table(Self::Table)
            .values([
                (Self::Report, report.into()),
                (Self::State, (MeetingState::Finished as i32).into()),
                (Self::Updated, Expr::current_timestamp()),
            ])
            .and_where(Expr::col(Self::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        db.execute(sqlx::query_with(&sql, values)).await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow, Debug, Serialize)]
pub struct MeetingRow {
    pub id: i32,
    pub title: String,
    pub start_time: DateTime<Local>,
    pub end_time: DateTime<Local>,
    pub location: String,
    pub url: String,
    pub description: String,
    pub proposal_uri: String,
    pub state: i32,
    pub report: Option<String>,
    pub creater: String,
    pub updated: DateTime<Local>,
    pub created: DateTime<Local>,
}
