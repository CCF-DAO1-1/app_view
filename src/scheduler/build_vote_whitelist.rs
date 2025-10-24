use color_eyre::Result;
use sea_query::PostgresQueryBuilder;
use sea_query_sqlx::SqlxBinder;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView, ckb,
    lexicon::{profile::Profile, vote_whitelist::VoteWhitelist},
    smt::{CkbSMT, SMT_VALUE},
};

pub async fn build_vote_whitelist_job(sched: &JobScheduler, app: &AppView) -> Result<Job> {
    let app = app.clone();
    let mut job = Job::new_async("0 0 0 * * *", move |uuid, mut l| {
        Box::pin({
            let db = app.db.clone();
            let ckb_client = app.ckb_client.clone();
            async move {
                info!("Job ID: {uuid} run async every day at 0am UTC");

                build_vote_whitelist(db, ckb_client).await;

                let next_tick = l.next_tick_for_job(uuid).await;
                info!("Next time for job is {:?}", next_tick);
            }
        })
    })?;
    job.on_start_notification_add(
        sched,
        Box::new(|job_id, notification_id, type_of_notification| {
            Box::pin(async move {
                info!(
                    "Job {:?} was started, notification {:?} ran ({:?})",
                    job_id, notification_id, type_of_notification
                );
            })
        }),
    )
    .await?;
    job.on_stop_notification_add(
        sched,
        Box::new(|job_id, notification_id, type_of_notification| {
            Box::pin(async move {
                info!(
                    "Job {:?} was completed, notification {:?} ran ({:?})",
                    job_id, notification_id, type_of_notification
                );
            })
        }),
    )
    .await?;
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

pub async fn build_vote_whitelist(
    db: sqlx::Pool<sqlx::Postgres>,
    ckb_client: ckb_sdk::CkbRpcAsyncClient,
) {
    let (sql, values) = sea_query::Query::select()
        .columns([(Profile::Table, Profile::Did)])
        .from(Profile::Table)
        .build_sqlx(PostgresQueryBuilder);
    debug!("build_author exec sql: {sql}");
    let row: Vec<(String,)> = sqlx::query_as_with(&sql, values)
        .fetch_all(&db)
        .await
        .unwrap_or(vec![]);
    let did_list = row.into_iter().map(|r| r.0).collect::<Vec<String>>();
    let mut vote_whitelist = vec![];
    let mut smt_tree = CkbSMT::default();
    for did in did_list {
        if let Ok(ckb_addr) = ckb::get_ckb_addr_by_did(&ckb_client, &did).await
            && let Ok(deposit) = ckb::get_nervos_dao_deposit(&ckb_client, &ckb_addr).await
        {
            if deposit > 0 {
                info!(
                    "DID: {} with CKB address: {} has deposit: {} shannons, added to vote whitelist",
                    did, ckb_addr, deposit
                );
                let address = crate::AddressParser::default()
                    .set_network(ckb_sdk::NetworkType::Testnet)
                    .parse(&ckb_addr)
                    .unwrap();
                let lock_script = ckb_types::packed::Script::from(address.payload());
                let lock_hash_bytes = lock_script.calc_script_hash();
                let lock_hash = hex::encode(lock_hash_bytes.raw_data());
                vote_whitelist.push(lock_hash);
                let key: [u8; 32] = lock_hash_bytes
                    .raw_data()
                    .to_vec()
                    .as_slice()
                    .try_into()
                    .unwrap();
                smt_tree.update(key.into(), SMT_VALUE.into()).ok();
            } else {
                info!(
                    "DID: {} with CKB address: {} has deposit: {} shannons, not qualified for vote whitelist",
                    did, ckb_addr, deposit
                );
            }
        }
    }
    let smt_root_hash = hex::encode(smt_tree.root().as_slice());
    let id = chrono::Utc::now().format("%Y-%m-%d").to_string();
    info!(
        "Built vote whitelist with {} entries, SMT root hash: {}, id: {}",
        vote_whitelist.len(),
        smt_root_hash,
        id
    );
    VoteWhitelist::insert(&db, &id, vote_whitelist, &smt_root_hash)
        .await
        .ok();
}
