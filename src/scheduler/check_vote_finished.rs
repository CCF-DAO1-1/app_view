use ckb_types::core::EpochNumberWithFraction;
use color_eyre::Result;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde_json::json;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView,
    lexicon::{
        proposal::ProposalState,
        task::{Task, TaskRow, TaskState, TaskType},
        timeline::{Timeline, TimelineRow, TimelineType},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState, VoteResult},
    },
};

pub async fn job(sched: &JobScheduler, app: &AppView, cron: &str) -> Result<Job> {
    let app = app.clone();
    let mut job = Job::new_async(cron, move |_uuid, _scheduler| {
        Box::pin({
            let db = app.db.clone();
            let ckb_client = app.ckb_client.clone();
            async move {
                check_vote_meta_finished(db, ckb_client)
                    .await
                    .map_err(|e| error!("job run failed: {e}"))
                    .ok();
            }
        })
    })?;

    job.on_removed_notification_add(
        sched,
        Box::new(|job_id, notification_id, type_of_notification| {
            Box::pin(async move {
                info!(
                    "Job {:?} was removed, notification {:?} ran ({:?})",
                    job_id, notification_id, type_of_notification
                );
            })
        }),
    )
    .await?;
    Ok(job)
}

pub async fn check_vote_meta_finished(
    db: sqlx::Pool<sqlx::Postgres>,
    ckb_client: ckb_sdk::CkbRpcAsyncClient,
) -> Result<()> {
    let (sql, values) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::State).eq(VoteMetaState::Committed as i32))
        .build_sqlx(PostgresQueryBuilder);

    let rows: Vec<VoteMetaRow> = sqlx::query_as_with(&sql, values.clone())
        .fetch_all(&db)
        .await
        .map_err(|e| {
            error!("{e}");
            e
        })
        .unwrap_or_default();
    let bn: u64 = ckb_client.get_tip_block_number().await?.into();
    let current_epoch = ckb_client.get_current_epoch().await?;
    for VoteMetaRow {
        id,
        proposal_uri,
        proposal_state,
        end_time,
        creater,
        ..
    } in rows
    {
        let end_time = EpochNumberWithFraction::from_full_value(end_time as u64);
        let current_epoch_number: u64 = current_epoch.number.into();
        let current_epoch_length: u64 = current_epoch.length.into();
        let current_epoch_index: u64 = bn - Into::<u64>::into(current_epoch.start_number);
        if end_time.number() < current_epoch_number
            || (end_time.number() == current_epoch_number
                && (end_time.index() as f64 / end_time.length() as f64)
                    < (current_epoch_index as f64 / current_epoch_length as f64))
        {
            continue;
        }

        // TODO: get votes by vote_indexer
        let vote_result = VoteResult::Agree;
        // update vote_meta state
        VoteMeta::update_results(&db, id, json!({})).await?;

        match vote_result {
            VoteResult::Voting => {}
            VoteResult::Agree => {}
            VoteResult::Against => {}
            VoteResult::Failed => {}
        }

        match ProposalState::from(proposal_state) {
            ProposalState::InitiationVote => {
                Task::insert(
                    &db,
                    &TaskRow {
                        id: 0,
                        task_type: TaskType::UpdateReceiverAddr as i32,
                        message: "UpdateReceiverAddr".to_string(),
                        target: proposal_uri.clone(),
                        operators: vec![],
                        processor: None,
                        deadline: chrono::Local::now() + chrono::Duration::days(21),
                        state: TaskState::Unread as i32,
                        updated: chrono::Local::now(),
                        created: chrono::Local::now(),
                    },
                )
                .await
                .map_err(|e| error!("insert task failed: {e}"))
                .ok();
            }
            ProposalState::AcceptanceVote => todo!(),
            ProposalState::DelayVote => todo!(),
            ProposalState::ReviewVote => todo!(),
            ProposalState::ReexamineVote => todo!(),
            ProposalState::RectificationVote => todo!(),
            _ => {}
        }

        Timeline::insert(
            &db,
            &TimelineRow {
                id: 0,
                timeline_type: TimelineType::VoteFinished as i32,
                message: "VoteFinished".to_string(),
                target: proposal_uri.clone(),
                operator: creater,
                timestamp: chrono::Local::now(),
            },
        )
        .await
        .map_err(|e| error!("insert timeline failed: {e}"))
        .ok();
    }
    Ok(())
}
