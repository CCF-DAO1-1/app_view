use color_eyre::{Result, eyre::eyre};
use sea_query::PostgresQueryBuilder;
use sea_query_sqlx::SqlxBinder;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView, ckb, indexer_bind,
    lexicon::{profile::Profile, vote_whitelist::VoteWhitelist},
    smt::{CkbSMT, SMT_VALUE},
};

pub async fn job(scheduler: &JobScheduler, app: &AppView, cron: &str) -> Result<Job> {
    let app = app.clone();
    let mut job = Job::new_async(cron, move |_uuid, _scheduler| {
        Box::pin({
            let db = app.db.clone();
            let ckb_client = app.ckb_client.clone();
            let ckb_net = app.ckb_net;
            let indexer_bind_url = app.indexer_bind_url.clone();
            async move {
                build_vote_whitelist(db, ckb_client, ckb_net, indexer_bind_url)
                    .await
                    .map_err(|e| error!("job run failed: {e}"))
                    .ok();
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

pub async fn build_vote_whitelist(
    db: sqlx::Pool<sqlx::Postgres>,
    ckb_client: ckb_sdk::CkbRpcAsyncClient,
    ckb_net: ckb_sdk::NetworkType,
    indexer_bind_url: String,
) -> Result<()> {
    let (sql, values) = sea_query::Query::select()
        .columns([(Profile::Table, Profile::Did)])
        .from(Profile::Table)
        .build_sqlx(PostgresQueryBuilder);
    let row: Vec<(String,)> = sqlx::query_as_with(&sql, values).fetch_all(&db).await?;
    let block_number = Into::<u64>::into(ckb_client.get_tip_block_number().await?);
    let did_list = row.into_iter().map(|r| r.0).collect::<Vec<String>>();
    let mut vote_whitelist = vec![];
    let mut smt_tree = CkbSMT::default();
    for did in did_list {
        if let Ok(ckb_addr) = ckb::get_ckb_addr_by_did(&ckb_client, &ckb_net, &did).await
            && let Ok(deposit) = indexer_bind::get_weight(
                &ckb_client,
                ckb_net,
                &indexer_bind_url,
                &ckb_addr,
                Some(block_number),
            )
            .await
        {
            if deposit > 0 {
                info!(
                    "DID: {} with CKB address: {} has weight: {}, added to vote whitelist",
                    did, ckb_addr, deposit
                );
                let address = crate::AddressParser::default()
                    .set_network(ckb_net)
                    .parse(&ckb_addr)
                    .map_err(|e| eyre!(e))?;
                let lock_script = ckb_types::packed::Script::from(address.payload());
                let lock_hash_bytes = lock_script.calc_script_hash();
                let lock_hash = hex::encode(lock_hash_bytes.raw_data());
                vote_whitelist.push(lock_hash);
                let key: [u8; 32] = lock_hash_bytes.raw_data().to_vec().as_slice().try_into()?;
                smt_tree.update(key.into(), SMT_VALUE.into()).ok();
            } else {
                info!(
                    "DID: {} with CKB address: {} has weight: {}, not qualified for vote whitelist",
                    did, ckb_addr, deposit
                );
            }
        }
    }
    let smt_root_hash = hex::encode(smt_tree.root().as_slice());
    let id = chrono::Local::now().to_rfc3339();
    info!(
        "Built vote whitelist with {} entries, SMT root hash: {}, id: {}",
        vote_whitelist.len(),
        smt_root_hash,
        id
    );
    VoteWhitelist::insert(
        &db,
        &id,
        vote_whitelist,
        &smt_root_hash,
        block_number as i64,
    )
    .await
}
