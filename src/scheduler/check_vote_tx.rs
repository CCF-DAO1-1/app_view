use chrono::{DateTime, Local};
use color_eyre::Result;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView,
    ckb::get_tx_status,
    lexicon::vote::{Vote, VoteState},
};

pub async fn job(sched: &JobScheduler, app: &AppView, cron: &str) -> Result<Job> {
    let app = app.clone();
    let mut job = Job::new_async(cron, move |_uuid, _scheduler| {
        Box::pin({
            let db = app.db.clone();
            let ckb_client = app.ckb_client.clone();
            async move {
                check_vote_tx(db, ckb_client).await;
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

pub async fn check_vote_tx(db: sqlx::Pool<sqlx::Postgres>, ckb_client: ckb_sdk::CkbRpcAsyncClient) {
    let (sql, values) = sea_query::Query::select()
        .columns([
            (Vote::Table, Vote::Id),
            (Vote::Table, Vote::TxHash),
            (Vote::Table, Vote::Created),
        ])
        .from(Vote::Table)
        .and_where(Expr::col(Vote::State).eq(VoteState::Waiting as i32))
        .build_sqlx(PostgresQueryBuilder);

    #[allow(clippy::type_complexity)]
    let rows: Option<Vec<(i32, Option<String>, DateTime<Local>)>> =
        sqlx::query_as_with(&sql, values.clone())
            .fetch_all(&db)
            .await
            .map_err(|e| {
                error!("{e}");
                e
            })
            .ok();
    if let Some(rows) = rows {
        for (id, tx_hash, created) in rows {
            if let Some(tx_hash) = tx_hash {
                let tx_status = get_tx_status(&ckb_client, &tx_hash).await;
                if let Ok(tx_status) = tx_status {
                    debug!("Vote({id}) tx {tx_hash} status: {tx_status:?}");
                    let meta_state = match tx_status {
                        ckb_jsonrpc_types::Status::Committed => VoteState::Committed,
                        ckb_jsonrpc_types::Status::Pending => continue,
                        ckb_jsonrpc_types::Status::Proposed => continue,
                        ckb_jsonrpc_types::Status::Unknown => {
                            if (chrono::Local::now() - created) > chrono::Duration::minutes(3) {
                                VoteState::Timeout
                            } else {
                                continue;
                            }
                        }
                        ckb_jsonrpc_types::Status::Rejected => VoteState::Rejected,
                    };
                    let (sql, values) = sea_query::Query::update()
                        .table(Vote::Table)
                        .value(Vote::State, meta_state as i32)
                        .and_where(Expr::col(Vote::Id).eq(id))
                        .build_sqlx(PostgresQueryBuilder);
                    sqlx::query_with(&sql, values).execute(&db).await.ok();
                    debug!("Vote({}) tx {} marked as {:?}", id, tx_hash, meta_state);
                }
            }
        }
    }
}
