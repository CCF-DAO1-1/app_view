use ckb_types::core::EpochNumberWithFraction;
use ckb_types::prelude::Entity;
use color_eyre::{Result, eyre::OptionExt};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde_json::json;
use sqlx::query_as_with;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView,
    api::proposal::calculate_vote_result,
    indexer_vote::all_votes,
    lexicon::{
        administrator::Administrator,
        proposal::{Proposal, ProposalSample, ProposalState},
        task::{Task, TaskRow, TaskState, TaskType},
        timeline::{Timeline, TimelineRow, TimelineType},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState, VoteResult, VoteResults},
    },
};

pub async fn job(sched: &JobScheduler, app: &AppView, cron: &str) -> Result<Job> {
    let app = app.clone();
    let mut job = Job::new_async(cron, move |_uuid, _scheduler| {
        Box::pin({
            let db = app.db.clone();
            let ckb_client = app.ckb_client.clone();
            let indexer_bind_url = app.indexer_bind_url.clone();
            let indexer_vote_url = app.indexer_vote_url.clone();
            async move {
                check_vote_meta_finished(db, indexer_bind_url, indexer_vote_url, ckb_client)
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
    indexer_bind_url: String,
    indexer_vote_url: String,
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
    debug!(
        "check_vote_meta_finished at block number: {}, current_epoch: {:?}",
        bn, current_epoch
    );
    for VoteMetaRow {
        id,
        proposal_uri,
        proposal_state,
        end_time,
        creater,
        tx_hash,
        candidates,
        ..
    } in rows
    {
        let end_time = EpochNumberWithFraction::from_full_value(end_time as u64);
        let current_epoch_number: u64 = current_epoch.number.into();
        let current_epoch_length: u64 = current_epoch.length.into();
        let current_epoch_index: u64 = bn - Into::<u64>::into(current_epoch.start_number);
        debug!(
            "check vote_meta id: {}, proposal_state: {}, end_time: {}",
            id, proposal_state, end_time
        );
        if end_time.number() > current_epoch_number
            || (end_time.number() == current_epoch_number
                && (end_time.index() as f64 / end_time.length() as f64)
                    > (current_epoch_index as f64 / current_epoch_length as f64))
        {
            continue;
        }

        // TODO: get votes by vote_indexer
        let vote_meta_out_point: ckb_types::packed::OutPoint = ckb_jsonrpc_types::OutPoint {
            tx_hash: ckb_types::H256(
                hex::decode(tx_hash.unwrap().trim_start_matches("0x"))
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
            index: 0.into(),
        }
        .into();
        let pubkey_hash = ckb_hash::blake2b_256(vote_meta_out_point.as_bytes());
        let args = pubkey_hash[0..20].to_vec();
        let args = hex::encode(args);
        debug!("args: {}", args);

        let vote_result = all_votes(
            &indexer_vote_url,
            &args,
            end_time.number() as i64,
            end_time.index() as i64,
            end_time.length() as i64,
        )
        .await?
        .as_array()
        .cloned()
        .ok_or_eyre("vote_result is not array")?;
        let vote_sum = vote_result.len();
        let mut weight_sum = 0;
        let mut valid_vote_sum = 0;
        let mut valid_weight_sum = 0;
        let mut valid_votes = vec![vec![]; candidates.len()];
        let mut candidate_votes = vec![0; candidates.len()];
        for vote in vote_result {
            let ckb_addr = vote
                .get("ckbAddress")
                .and_then(|v| v.as_str())
                .ok_or_eyre("ckb_addr not found")?;
            let vote_index = vote
                .get("voteIndex")
                .and_then(|v| v.as_array())
                .ok_or_eyre("vote_index not found")?
                .first()
                .and_then(|i| i.as_i64())
                .ok_or_eyre("vote_index not found")?;
            let weight = crate::indexer_bind::get_weight(&ckb_client, &indexer_bind_url, ckb_addr)
                .await
                .unwrap_or(0);

            weight_sum += weight;
            if let Some(candidate_vote) = candidate_votes.get_mut(vote_index as usize) {
                valid_vote_sum += 1;
                valid_weight_sum += weight;
                *candidate_vote += weight;
            }
            if let Some(valid_vote) = valid_votes.get_mut(vote_index as usize) {
                valid_vote.push((ckb_addr.to_string(), weight));
            }
        }

        let vote_results = VoteResults {
            vote_sum: vote_sum as u64,
            valid_vote_sum: valid_vote_sum as u64,
            weight_sum,
            valid_weight_sum,
            valid_votes,
            candidate_votes,
        };
        debug!("vote_result: {:?}", vote_results);
        // update vote_meta state
        VoteMeta::update_results(&db, id, json!(vote_results)).await?;

        let (sql, value) = Proposal::build_sample()
            .and_where(Expr::col(Proposal::Uri).eq(proposal_uri.clone()))
            .build_sqlx(PostgresQueryBuilder);
        let proposal_sample: ProposalSample = query_as_with(&sql, value).fetch_one(&db).await?;
        debug!("proposal_sample: {:?}", proposal_sample);
        let proposal_type = proposal_sample
            .record
            .pointer("/data/proposalType")
            .and_then(|t| t.as_str())
            .ok_or_eyre("")?;
        let vote_result = calculate_vote_result(
            proposal_state,
            &proposal_sample,
            vote_results.clone(),
            proposal_type,
        );

        debug!(
            "vote_meta id: {} finished with result: {:?}",
            id, vote_result
        );
        match vote_result {
            VoteResult::Voting => {}
            VoteResult::Agree => match ProposalState::from(proposal_state) {
                ProposalState::InitiationVote => {
                    Proposal::update_state(
                        &db,
                        &proposal_uri,
                        ProposalState::WaitingForStartFund as i32,
                    )
                    .await?;

                    let admins = Administrator::fetch_all(&db)
                        .await
                        .iter()
                        .map(|admin| admin.did.clone())
                        .collect();
                    Task::insert(
                        &db,
                        &TaskRow {
                            id: 0,
                            task_type: TaskType::UpdateReceiverAddr as i32,
                            message: "UpdateReceiverAddr".to_string(),
                            target: proposal_uri.clone(),
                            operators: admins,
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

                    Task::complete(&db, &proposal_uri, TaskType::CreateAMA, "SYSTEM")
                        .await
                        .ok();
                    Task::complete(&db, &proposal_uri, TaskType::SubmitAMAReport, "SYSTEM")
                        .await
                        .ok();
                }
                ProposalState::MilestoneVote => {
                    Proposal::update_state(
                        &db,
                        &proposal_uri,
                        ProposalState::WaitingForMilestoneFund as i32,
                    )
                    .await?;

                    let admins = Administrator::fetch_all(&db)
                        .await
                        .iter()
                        .map(|admin| admin.did.clone())
                        .collect();
                    let milestone = proposal_sample
                        .record
                        .pointer("/data/milestones")
                        .and_then(|m| m.as_array())
                        .and_then(|ms| ms.get(proposal_sample.progress as usize));
                    Task::insert(
                        &db,
                        &TaskRow {
                            id: 0,
                            task_type: TaskType::SendMilestoneFund as i32,
                            message: milestone
                                .map(|m| m.to_string())
                                .unwrap_or("SendMilestoneFund".to_string()),
                            target: proposal_uri.clone(),
                            operators: admins,
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
                ProposalState::DelayVote => {
                    Proposal::update_state(&db, &proposal_uri, ProposalState::InProgress as i32)
                        .await?;
                    let admins: Vec<String> = Administrator::fetch_all(&db)
                        .await
                        .iter()
                        .map(|admin| admin.did.clone())
                        .collect();
                    Task::insert(
                        &db,
                        &TaskRow {
                            id: 0,
                            task_type: TaskType::SubmitMilestoneReport as i32,
                            message: proposal_sample.progress.to_string(),
                            target: proposal_uri.clone(),
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
                        &db,
                        &TaskRow {
                            id: 0,
                            task_type: TaskType::SubmitDelayReport as i32,
                            message: proposal_sample.progress.to_string(),
                            target: proposal_uri.clone(),
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
                ProposalState::ReexamineVote => {
                    error!("VoteResult::Agree -> ProposalState::ReexamineVote not implemented yet");
                }
                ProposalState::RectificationVote => {
                    error!(
                        "VoteResult::Agree -> ProposalState::RectificationVote not implemented yet"
                    );
                }
                _ => {}
            },
            VoteResult::Against => match ProposalState::from(proposal_state) {
                ProposalState::InitiationVote => {
                    Proposal::update_state(&db, &proposal_uri, ProposalState::End as i32).await?;
                }
                ProposalState::MilestoneVote | ProposalState::DelayVote => {
                    error!("VoteResult::Against -> MilestoneVote | DelayVote not implemented yet");
                }
                ProposalState::ReexamineVote => {
                    Proposal::update_state(&db, &proposal_uri, ProposalState::End as i32).await?;
                }
                ProposalState::RectificationVote => {
                    Proposal::update_state(&db, &proposal_uri, ProposalState::End as i32).await?;
                }
                _ => {}
            },
            VoteResult::Failed => match ProposalState::from(proposal_state) {
                ProposalState::InitiationVote => {
                    Proposal::update_state(&db, &proposal_uri, ProposalState::End as i32).await?;
                }
                ProposalState::MilestoneVote | ProposalState::DelayVote => {
                    error!("VoteResult::Failed -> ProposalState::DelayVote not implemented yet");
                }
                ProposalState::ReexamineVote => {
                    error!(
                        "VoteResult::Failed -> ProposalState::ReexamineVote not implemented yet"
                    );
                }
                ProposalState::RectificationVote => {
                    Proposal::update_state(&db, &proposal_uri, ProposalState::End as i32).await?;
                }
                _ => {}
            },
        }

        Timeline::insert(
            &db,
            &TimelineRow {
                id: 0,
                timeline_type: TimelineType::VoteFinished as i32,
                message: json!(vote_results).to_string(),
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
