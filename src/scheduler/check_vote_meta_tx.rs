use color_eyre::Result;
use molecule::prelude::Entity;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView,
    api::vote,
    lexicon::{
        proposal::{Proposal, ProposalState},
        timeline::{Timeline, TimelineRow, TimelineType},
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState},
    },
};

pub async fn job(scheduler: &JobScheduler, app: &AppView, cron: &str) -> Result<Job> {
    let app = app.clone();
    let mut job = Job::new_async(cron, move |_uuid, _scheduler| {
        Box::pin({
            let db = app.db.clone();
            let ckb_client = app.ckb_client.clone();
            async move {
                check_vote_meta_tx(db, ckb_client).await;
            }
        })
    })?;

    job.on_removed_notification_add(
        scheduler,
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

pub async fn check_vote_meta_tx(
    db: sqlx::Pool<sqlx::Postgres>,
    ckb_client: ckb_sdk::CkbRpcAsyncClient,
) {
    let (sql, values) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::State).eq(VoteMetaState::Waiting as i32))
        .build_sqlx(PostgresQueryBuilder);

    #[allow(clippy::type_complexity)]
    let rows: Option<Vec<VoteMetaRow>> = sqlx::query_as_with(&sql, values.clone())
        .fetch_all(&db)
        .await
        .map_err(|e| {
            error!("{e}");
            e
        })
        .ok();
    if let Some(rows) = rows {
        for row in rows {
            if let Some(tx_hash) = &row.tx_hash {
                let tx = ckb_client
                    .get_transaction(ckb_types::H256(
                        hex::decode(tx_hash.strip_prefix("0x").unwrap_or(tx_hash))
                            .unwrap()
                            .try_into()
                            .unwrap(),
                    ))
                    .await;
                if let Ok(Some((tx_status, tx))) = tx.map(|t| {
                    t.map(|t| {
                        (
                            t.tx_status,
                            t.transaction.and_then(|t| {
                                if let ckb_jsonrpc_types::Either::Left(tx) = t.inner {
                                    Some(tx)
                                } else {
                                    None
                                }
                            }),
                        )
                    })
                }) {
                    debug!("VoteMeta({}) tx {tx_hash} status: {tx_status:?}", row.id);
                    let meta_state = match tx_status.status {
                        ckb_jsonrpc_types::Status::Committed => {
                            let proposal_hash = ckb_hash::blake2b_256(
                                serde_json::to_vec(&row.proposal_uri).unwrap(),
                            );
                            if let Ok(vote_meta) =
                                vote::build_vote_meta(&db, &row, &proposal_hash).await
                            {
                                let vote_meta_bytes = vote_meta.as_bytes().to_vec();

                                if let Some(tx) = tx {
                                    if tx.inner.outputs_data[0].as_bytes() == vote_meta_bytes {
                                        VoteMetaState::Committed
                                    } else {
                                        VoteMetaState::Changed
                                    }
                                } else {
                                    VoteMetaState::Changed
                                }
                            } else {
                                VoteMetaState::Changed
                            }
                        }
                        ckb_jsonrpc_types::Status::Pending => continue,
                        ckb_jsonrpc_types::Status::Proposed => continue,
                        ckb_jsonrpc_types::Status::Unknown => {
                            if (chrono::Local::now() - row.created) > chrono::Duration::minutes(3) {
                                VoteMetaState::Timeout
                            } else {
                                continue;
                            }
                        }
                        ckb_jsonrpc_types::Status::Rejected => VoteMetaState::Rejected,
                    };
                    let (sql, values) = sea_query::Query::update()
                        .table(VoteMeta::Table)
                        .value(VoteMeta::State, meta_state as i32)
                        .and_where(Expr::col(VoteMeta::Id).eq(row.id))
                        .build_sqlx(PostgresQueryBuilder);
                    sqlx::query_with(&sql, values).execute(&db).await.ok();
                    debug!(
                        "VoteMeta({}) tx {} marked as {:?}",
                        row.id, tx_hash, meta_state
                    );

                    match meta_state {
                        VoteMetaState::Committed => {
                            // update proposal state
                            let lines =
                                Proposal::update_state(&db, &row.proposal_uri, row.proposal_state)
                                    .await
                                    .map_err(|e| error!("update proposal state failed: {e}"))
                                    .unwrap_or(0);
                            if lines > 0 {
                                debug!(
                                    "Proposal({}) marked as {:?}",
                                    row.proposal_uri,
                                    ProposalState::from(row.proposal_state)
                                );
                                let timeline_type = match ProposalState::from(row.proposal_state) {
                                    ProposalState::InitiationVote => TimelineType::InitiationVote,
                                    ProposalState::MilestoneVote => TimelineType::MilestoneVote,
                                    ProposalState::DelayVote => TimelineType::DelayVote,
                                    ProposalState::ReexamineVote => TimelineType::ReexamineVote,
                                    ProposalState::RectificationVote => {
                                        TimelineType::RectificationVote
                                    }
                                    _ => continue,
                                };

                                Timeline::insert(
                                    &db,
                                    &TimelineRow {
                                        id: 0,
                                        timeline_type: timeline_type as i32,
                                        message: format!("{timeline_type:?}"),
                                        target: row.proposal_uri.clone(),
                                        operator: row.creator.clone(),
                                        timestamp: chrono::Local::now(),
                                    },
                                )
                                .await
                                .map_err(|e| error!("insert timeline failed: {e}"))
                                .ok();
                            }
                        }
                        VoteMetaState::Changed => {
                            error!("VoteMeta({}) tx {} output data changed", row.id, tx_hash);
                            let lines = Proposal::update_state(
                                &db,
                                &row.proposal_uri,
                                ProposalState::End as i32,
                            )
                            .await
                            .map_err(|e| error!("update proposal state failed: {e}"))
                            .unwrap_or(0);
                            if lines > 0 {
                                debug!(
                                    "Proposal({}) marked as {:?}",
                                    row.proposal_uri,
                                    ProposalState::End
                                );

                                Timeline::insert(
                                    &db,
                                    &TimelineRow {
                                        id: 0,
                                        timeline_type: TimelineType::VoteMetaTxChanged as i32,
                                        message: format!(
                                            "VoteMeta({}) tx {} output data changed",
                                            row.id, tx_hash
                                        ),
                                        target: row.proposal_uri.clone(),
                                        operator: row.creator.clone(),
                                        timestamp: chrono::Local::now(),
                                    },
                                )
                                .await
                                .map_err(|e| error!("insert timeline failed: {e}"))
                                .ok();
                            }
                        }
                        _ => (),
                    }
                }
            }
        }
    }
}
