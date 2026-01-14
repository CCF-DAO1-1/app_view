use std::str::FromStr;

use chrono::DateTime;
use color_eyre::eyre::eyre;
use common_x::restful::{
    axum::{
        Json,
        extract::{Query, State},
        response::IntoResponse,
    },
    ok, ok_simple,
};
use sea_query::{Expr, ExprTrait, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::query_as_with;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::{
    AppView,
    api::{SignedBody, SignedParam, build_author, create_vote_tx},
    error::AppError,
    lexicon::{
        administrator::{Administrator, AdministratorRow},
        meeting::{Meeting, MeetingRow, MeetingState},
        proposal::{Proposal, ProposalRow, ProposalSample, ProposalState, has_next_milestone},
        task::{Task, TaskRow, TaskState, TaskType, TaskView},
        timeline::{Timeline, TimelineRow, TimelineType},
    },
};

#[derive(Debug, Validate, Deserialize, IntoParams)]
#[serde(default)]
pub struct TaskQuery {
    #[validate(length(min = 1))]
    pub did: String,
    #[validate(range(min = 1))]
    pub page: u64,
    #[validate(range(min = 1))]
    pub per_page: u64,
}

impl Default for TaskQuery {
    fn default() -> Self {
        Self {
            did: String::new(),
            page: 1,
            per_page: 20,
        }
    }
}

#[utoipa::path(get, path = "/api/task", params(TaskQuery))]
pub async fn get(
    State(state): State<AppView>,
    Query(query): Query<TaskQuery>,
) -> Result<impl IntoResponse, AppError> {
    query
        .validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let offset = query.per_page * (query.page - 1);

    let (sql, values) = sea_query::Query::select()
        .columns([
            (Task::Table, Task::Id),
            (Task::Table, Task::TaskType),
            (Task::Table, Task::Message),
            (Task::Table, Task::Target),
            (Task::Table, Task::Operators),
            (Task::Table, Task::Processor),
            (Task::Table, Task::Deadline),
            (Task::Table, Task::State),
            (Task::Table, Task::Updated),
            (Task::Table, Task::Created),
        ])
        .from(Task::Table)
        .and_where(Expr::col(Task::State).ne(TaskState::Completed as i32))
        .and_where(Expr::col(Task::Operators).is_null().or(Expr::cust(format!(
            "'{}' = ANY(\"task\".\"operators\")",
            query.did
        ))))
        .order_by(Task::Created, Order::Desc)
        .offset(offset)
        .limit(query.per_page)
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<TaskRow> = sqlx::query_as_with(&sql, values)
        .fetch_all(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    let mut views = vec![];
    for row in rows {
        let (sql, values) = Proposal::build_select(None)
            .and_where(Expr::col(Proposal::Uri).eq(row.target))
            .build_sqlx(PostgresQueryBuilder);

        let proposal: ProposalRow = query_as_with(&sql, values.clone())
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                debug!("exec sql failed: {e}");
                AppError::ExecSqlFailed(e.to_string())
            })?;

        let processor = if let Some(processor) = &row.processor {
            build_author(&state, processor).await
        } else {
            serde_json::Value::Null
        };
        views.push(TaskView {
            id: row.id,
            task_type: row.task_type,
            message: serde_json::Value::from_str(&row.message).unwrap_or(json!(row.message)),
            target: json!(proposal),
            operators: row.operators,
            processor,
            deadline: row.deadline,
            state: row.state,
            updated: row.updated,
            created: row.created,
        });
    }

    let (sql, values) = sea_query::Query::select()
        .expr(Expr::col((Task::Table, Task::Id)).count_distinct())
        .from(Task::Table)
        .and_where(Expr::col(Task::State).ne(TaskState::Completed as i32))
        .and_where(Expr::col(Task::Operators).is_null().or(Expr::cust(format!(
            "'{}' = ANY(\"task\".\"operators\")",
            query.did
        ))))
        .build_sqlx(PostgresQueryBuilder);

    let total: (i64,) = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| eyre!("exec sql failed: {e}"))?;

    Ok(ok(json!({
        "tasks": views,
        "page": query.page,
        "per_page": query.per_page,
        "total":  total.0
    })))
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct CreateMeetingParams {
    pub proposal_uri: String,
    pub title: String,
    pub start_time: String,
    pub url: String,
    pub description: String,
    pub timestamp: i64,
}

impl SignedParam for CreateMeetingParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(post, path = "/api/task/create_meeting", description = "组织会议")]
pub async fn create_meeting(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<CreateMeetingParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let admins = Administrator::fetch_all(&state.db)
        .await
        .iter()
        .map(|admin| admin.did.clone())
        .collect::<Vec<_>>();

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(&body.params.proposal_uri))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_row: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("invalid proposal_uri: {e}")))?;

    let meeting_row = MeetingRow {
        id: 0,
        title: body.params.title.clone(),
        start_time: DateTime::from_str(&body.params.start_time)
            .map_err(|e| AppError::ValidateFailed(format!("invalid start_time: {e}")))?,
        end_time: DateTime::from_str(&body.params.start_time)
            .map_err(|e| AppError::ValidateFailed(format!("invalid start_time: {e}")))?,
        location: "".to_string(),
        url: body.params.url.clone(),
        description: body.params.description.clone(),
        proposal_uri: body.params.proposal_uri.clone(),
        proposal_state: proposal_row.state,
        state: 0,
        report: None,
        creator: body.did.clone(),
        updated: chrono::Local::now(),
        created: chrono::Local::now(),
    };

    Meeting::insert(&state.db, &meeting_row).await?;

    match ProposalState::from(proposal_row.state) {
        ProposalState::Draft => {
            Task::insert(
                &state.db,
                &TaskRow {
                    id: 0,
                    task_type: TaskType::SubmitAMAReport as i32,
                    message: "SubmitAMAReport".to_string(),
                    target: body.params.proposal_uri.clone(),
                    operators: admins.clone(),
                    processor: None,
                    deadline: chrono::Local::now() + chrono::Duration::days(7),
                    state: TaskState::Unread as i32,
                    updated: chrono::Local::now(),
                    created: chrono::Local::now(),
                },
            )
            .await?;

            Timeline::insert(
                &state.db,
                &TimelineRow {
                    id: 0,
                    timeline_type: TimelineType::CreateAMA as i32,
                    message: format!("AMA meeting created by {}", body.did),
                    target: body.params.proposal_uri.clone(),
                    operator: body.did.clone(),
                    timestamp: chrono::Local::now(),
                },
            )
            .await?;
        }
        ProposalState::WaitingReexamine => {
            Task::insert(
                &state.db,
                &TaskRow {
                    id: 0,
                    task_type: TaskType::SubmitReexamineReport as i32,
                    message: "SubmitReexamineReport".to_string(),
                    target: body.params.proposal_uri.clone(),
                    operators: admins,
                    processor: None,
                    deadline: chrono::Local::now() + chrono::Duration::days(7),
                    state: TaskState::Unread as i32,
                    updated: chrono::Local::now(),
                    created: chrono::Local::now(),
                },
            )
            .await?;

            Task::complete(
                &state.db,
                &body.params.proposal_uri,
                TaskType::CreateReexamineMeeting,
                &body.did,
            )
            .await
            .ok();

            Timeline::insert(
                &state.db,
                &TimelineRow {
                    id: 0,
                    timeline_type: TimelineType::CreateReexamineMeeting as i32,
                    message: format!("Reexamine meeting created by {}", body.did),
                    target: body.params.proposal_uri.clone(),
                    operator: body.did.clone(),
                    timestamp: chrono::Local::now(),
                },
            )
            .await?;
        }
        _ => {}
    }

    Ok(ok_simple())
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct SubmitMeetingReportParams {
    pub proposal_uri: String,
    pub meeting_id: i32,
    pub report: String,
    pub timestamp: i64,
}

impl SignedParam for SubmitMeetingReportParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(
    post,
    path = "/api/task/submit_meeting_report",
    description = "提交会议报告"
)]
pub async fn submit_meeting_report(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<SubmitMeetingReportParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Meeting::build_select()
        .and_where(Expr::col(Meeting::Id).eq(body.params.meeting_id))
        .build_sqlx(PostgresQueryBuilder);
    let meeting_row: MeetingRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not meeting: {e}")))?;

    if meeting_row.report.is_some() || meeting_row.state == MeetingState::Finished as i32 {
        return Err(AppError::ValidateFailed(
            "meeting report already submitted".to_string(),
        ));
    }

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(&body.params.proposal_uri))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("invalid proposal_uri: {e}")))?;

    Meeting::update_report(&state.db, body.params.meeting_id, &body.params.report).await?;

    match ProposalState::from(proposal_sample.state) {
        ProposalState::Draft => {
            Timeline::insert(
                &state.db,
                &TimelineRow {
                    id: 0,
                    timeline_type: TimelineType::SubmitAMAReport as i32,
                    message: body.params.report.clone(),
                    target: body.params.proposal_uri.clone(),
                    operator: body.did.clone(),
                    timestamp: chrono::Local::now(),
                },
            )
            .await?;
        }
        ProposalState::WaitingReexamine => {
            Timeline::insert(
                &state.db,
                &TimelineRow {
                    id: 0,
                    timeline_type: TimelineType::SubmitAMAReport as i32,
                    message: body.params.report.clone(),
                    target: body.params.proposal_uri.clone(),
                    operator: body.did.clone(),
                    timestamp: chrono::Local::now(),
                },
            )
            .await?;

            // create vote_meta
            let vote_outputs_data = create_vote_tx(
                &state,
                &body.params.proposal_uri,
                ProposalState::ReexamineVote,
                &body.did,
            )
            .await?;

            return Ok(ok(vote_outputs_data));
        }
        _ => {}
    }

    Ok(ok(json!({})))
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct SendFundsParams {
    pub proposal_uri: String,
    pub amount: String,
    pub tx_hash: String,
    pub timestamp: i64,
}

impl SignedParam for SendFundsParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(post, path = "/api/task/send_funds", description = "拨款")]
pub async fn send_funds(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<SendFundsParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(body.params.proposal_uri.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("proposal not found: {e}")))?;

    let admins = Administrator::fetch_all(&state.db)
        .await
        .iter()
        .map(|admin| admin.did.clone())
        .collect::<Vec<_>>();

    match ProposalState::from(proposal_sample.state) {
        ProposalState::WaitingForStartFund => {
            let milestone = proposal_sample
                .record
                .pointer("/data/milestones")
                .and_then(|m| m.as_array())
                .and_then(|m| m.first());
            if let Some(milestone) = milestone {
                Proposal::update_state(
                    &state.db,
                    &body.params.proposal_uri,
                    ProposalState::InProgress as i32,
                )
                .await?;

                Task::insert(
                    &state.db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::SubmitMilestoneReport as i32,
                        message: milestone.to_string(),
                        target: body.params.proposal_uri.clone(),
                        operators: admins.clone(),
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(7),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
                Task::insert(
                    &state.db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::SubmitDelayReport as i32,
                        message: milestone.to_string(),
                        target: body.params.proposal_uri.clone(),
                        operators: admins,
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(7),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
            } else {
                Proposal::update_state(
                    &state.db,
                    &body.params.proposal_uri,
                    ProposalState::WaitingForAcceptanceReport as i32,
                )
                .await?;

                Task::insert(
                    &state.db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::SubmitAcceptanceReport as i32,
                        message: "SubmitAcceptanceReport".to_string(),
                        target: body.params.proposal_uri.clone(),
                        operators: admins,
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(7),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
            }

            Timeline::insert(
                &state.db,
                &TimelineRow {
                    id: 0,
                    timeline_type: TimelineType::SendInitialFund as i32,
                    message: json!({
                        "amount": body.params.amount,
                        "tx_hash": body.params.tx_hash,
                    })
                    .to_string(),
                    target: body.params.proposal_uri.clone(),
                    operator: body.did.clone(),
                    timestamp: chrono::Local::now(),
                },
            )
            .await
            .map_err(|e| error!("insert timeline failed: {e}"))
            .ok();
            Task::complete(
                &state.db,
                &body.params.proposal_uri,
                TaskType::SendInitialFund,
                &body.did,
            )
            .await
            .ok();
        }
        ProposalState::WaitingForMilestoneFund => {
            if let Some((index, next_milestone)) = has_next_milestone(&proposal_sample) {
                Proposal::update_progress(
                    &state.db,
                    &body.params.proposal_uri,
                    ProposalState::InProgress as i32,
                    index as i32,
                )
                .await?;

                Task::insert(
                    &state.db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::SubmitMilestoneReport as i32,
                        message: next_milestone.to_string(),
                        target: body.params.proposal_uri.clone(),
                        operators: admins.clone(),
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(7),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
                Task::insert(
                    &state.db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::SubmitDelayReport as i32,
                        message: next_milestone.to_string(),
                        target: body.params.proposal_uri.clone(),
                        operators: admins,
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(7),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
            } else {
                Proposal::update_state(
                    &state.db,
                    &body.params.proposal_uri,
                    ProposalState::WaitingForAcceptanceReport as i32,
                )
                .await?;

                Task::insert(
                    &state.db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::SubmitAcceptanceReport as i32,
                        message: "SubmitAcceptanceReport".to_string(),
                        target: body.params.proposal_uri.clone(),
                        operators: admins,
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(7),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
            }

            Timeline::insert(
                &state.db,
                &TimelineRow {
                    id: 0,
                    timeline_type: TimelineType::SendMilestoneFund as i32,
                    message: json!({
                        "amount": body.params.amount,
                        "tx_hash": body.params.tx_hash,
                    })
                    .to_string(),
                    target: body.params.proposal_uri.clone(),
                    operator: body.did.clone(),
                    timestamp: chrono::Local::now(),
                },
            )
            .await
            .map_err(|e| error!("insert timeline failed: {e}"))
            .ok();
            Task::complete(
                &state.db,
                &body.params.proposal_uri,
                TaskType::SendMilestoneFund,
                &body.did,
            )
            .await
            .ok();
        }
        _ => {}
    }

    Ok(ok_simple())
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct SubmitReportParams {
    pub proposal_uri: String,

    pub report: String,

    pub timestamp: i64,
}

impl SignedParam for SubmitReportParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(
    post,
    path = "/api/task/submit_milestone_report",
    description = "提交里程碑报告"
)]
pub async fn submit_milestone_report(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<SubmitReportParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(body.params.proposal_uri.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("proposal not found: {e}")))?;

    // create vote_meta
    let vote_outputs_data = create_vote_tx(
        &state,
        &body.params.proposal_uri,
        ProposalState::MilestoneVote,
        &body.did,
    )
    .await?;

    Task::complete(
        &state.db,
        &proposal_sample.uri,
        TaskType::SubmitMilestoneReport,
        &body.did,
    )
    .await
    .ok();

    Task::complete(
        &state.db,
        &proposal_sample.uri,
        TaskType::SubmitDelayReport,
        &body.did,
    )
    .await
    .ok();

    Ok(ok(vote_outputs_data))
}

#[utoipa::path(
    post,
    path = "/api/task/submit_delay_report",
    description = "提交延期报告"
)]
pub async fn submit_delay_report(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<SubmitReportParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(body.params.proposal_uri.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("proposal not found: {e}")))?;

    // create vote_meta
    let vote_outputs_data = create_vote_tx(
        &state,
        &body.params.proposal_uri,
        ProposalState::DelayVote,
        &body.did,
    )
    .await?;

    Task::complete(
        &state.db,
        &proposal_sample.uri,
        TaskType::SubmitDelayReport,
        &body.did,
    )
    .await
    .ok();

    Task::complete(
        &state.db,
        &proposal_sample.uri,
        TaskType::SubmitMilestoneReport,
        &body.did,
    )
    .await
    .ok();

    Ok(ok(vote_outputs_data))
}

#[utoipa::path(
    post,
    path = "/api/task/submit_acceptance_report",
    description = "提交结项报告"
)]
pub async fn submit_acceptance_report(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<SubmitReportParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Proposal::build_sample()
        .and_where(Expr::col(Proposal::Uri).eq(body.params.proposal_uri.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let proposal_sample: ProposalSample = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("proposal not found: {e}")))?;

    if proposal_sample.state != ProposalState::WaitingForAcceptanceReport as i32 {
        return Err(AppError::ValidateFailed(
            "proposal is not waiting for acceptance report".to_string(),
        ));
    }

    Proposal::update_state(
        &state.db,
        &body.params.proposal_uri,
        ProposalState::Completed as i32,
    )
    .await?;

    Timeline::insert(
        &state.db,
        &TimelineRow {
            id: 0,
            timeline_type: TimelineType::SubmitAcceptanceReport as i32,
            message: body.params.report.clone(),
            target: body.params.proposal_uri.clone(),
            operator: body.did.clone(),
            timestamp: chrono::Local::now(),
        },
    )
    .await?;

    Task::complete(
        &state.db,
        &proposal_sample.uri,
        TaskType::SubmitAcceptanceReport,
        &body.did,
    )
    .await
    .ok();

    Ok(ok_simple())
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct RectificationVoteParams {
    pub proposal_uri: String,
    pub timestamp: i64,
}

impl SignedParam for RectificationVoteParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(
    post,
    path = "/api/task/rectification_vote",
    description = "发起整改投票"
)]
pub async fn rectification_vote(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<RectificationVoteParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let SignedBody::<RectificationVoteParams> { params, did, .. } = body;

    let (sql, values) = Proposal::build_select(None)
        .and_where(Expr::col(Proposal::Uri).eq(&params.proposal_uri))
        .build_sqlx(PostgresQueryBuilder);

    let proposal_row: ProposalRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::ExecSqlFailed(e.to_string())
        })?;

    // check proposal state
    if proposal_row.state != (ProposalState::WaitingReexamine as i32) {
        return Err(AppError::ValidateFailed(
            "proposal state not WaitingReexamine".to_string(),
        ));
    }

    //  check AMA completed
    let (sql, values) = Meeting::build_select()
        .and_where(Expr::col(Meeting::ProposalUri).eq(&params.proposal_uri))
        .and_where(Expr::col(Meeting::ProposalState).eq(ProposalState::WaitingReexamine as i32))
        .and_where(Expr::col(Meeting::State).eq(MeetingState::Finished as i32))
        .build_sqlx(PostgresQueryBuilder);
    let _meeting_row: MeetingRow = query_as_with(&sql, values)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("fetch meeting failed: {e}");
            AppError::ValidateFailed("Reexamine meeting not completed".to_string())
        })?;

    // create vote_meta
    let vote_outputs_data = create_vote_tx(
        &state,
        &params.proposal_uri,
        ProposalState::RectificationVote,
        &did,
    )
    .await?;

    Task::complete(
        &state.db,
        &proposal_row.uri,
        TaskType::RectificationVote,
        &did,
    )
    .await
    .ok();

    Ok(ok(vote_outputs_data))
}

#[derive(Debug, Default, Validate, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct RectificationParams {
    pub proposal_uri: String,
    pub progress: i32,
    /// record value
    pub value: Value,
    pub timestamp: i64,
}

impl SignedParam for RectificationParams {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

#[utoipa::path(post, path = "/api/task/rectification")]
pub async fn rectification(
    State(state): State<AppView>,
    Json(body): Json<SignedBody<RectificationParams>>,
) -> Result<impl IntoResponse, AppError> {
    body.validate()
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, value) = Administrator::build_select()
        .and_where(Expr::col(Administrator::Did).eq(body.did.clone()))
        .build_sqlx(PostgresQueryBuilder);
    let _admin_row: AdministratorRow = query_as_with(&sql, value)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::ValidateFailed(format!("not administrator: {e}")))?;

    body.verify_signature(&state.indexer_did_url)
        .await
        .map_err(|e| AppError::ValidateFailed(e.to_string()))?;

    let (sql, values) = Proposal::build_select(None)
        .and_where(Expr::col(Proposal::Uri).eq(&body.params.proposal_uri))
        .build_sqlx(PostgresQueryBuilder);

    let proposal_row: ProposalRow = query_as_with(&sql, values.clone())
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            debug!("exec sql failed: {e}");
            AppError::ExecSqlFailed(e.to_string())
        })?;

    // check proposal state
    if proposal_row.state != (ProposalState::WaitingRectification as i32) {
        return Err(AppError::ValidateFailed(
            "proposal state not WaitingRectification".to_string(),
        ));
    }

    Proposal::update(
        &state.db,
        body.params.value,
        &body.params.proposal_uri,
        &proposal_row.cid,
    )
    .await?;
    Proposal::update_progress(
        &state.db,
        &body.params.proposal_uri,
        ProposalState::InProgress as i32,
        body.params.progress,
    )
    .await?;
    Timeline::insert(
        &state.db,
        &TimelineRow {
            id: 0,
            timeline_type: TimelineType::Rectification as i32,
            message: "Rectification".to_string(),
            target: body.params.proposal_uri.to_string(),
            operator: body.did.clone(),
            timestamp: chrono::Local::now(),
        },
    )
    .await
    .map_err(|e| error!("insert timeline failed: {e}"))
    .ok();

    Ok(ok_simple())
}
