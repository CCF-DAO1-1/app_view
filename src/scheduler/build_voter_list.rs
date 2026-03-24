use std::collections::{BTreeSet, HashSet};

use color_eyre::{Result, eyre::eyre};
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    AppView, indexer_bind,
    lexicon::voter_list::VoterList,
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
            let indexer_dao_url = app.indexer_dao_url.clone();
            async move {
                build_voter_list(db, ckb_client, ckb_net, indexer_bind_url, indexer_dao_url)
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

pub async fn build_voter_list(
    db: sqlx::Pool<sqlx::Postgres>,
    ckb_client: ckb_sdk::CkbRpcAsyncClient,
    ckb_net: ckb_sdk::NetworkType,
    indexer_bind_url: String,
    indexer_dao_url: String,
) -> Result<()> {
    let block_number = Into::<u64>::into(ckb_client.get_tip_block_number().await?);
    let did_set = crate::indexer_did::did_set(&indexer_bind_url, block_number).await?;
    let ckb_addrs: HashSet<String> = did_set.values().cloned().collect();
    let mut voter_btree_set = BTreeSet::new();
    for ckb_addr in ckb_addrs {
        if let Ok(deposit) = indexer_bind::get_weight(
            ckb_net,
            &indexer_bind_url,
            &indexer_dao_url,
            &ckb_addr,
            Some(block_number),
        )
        .await
        .map(|wp| wp.values().sum::<u64>())
        {
            if deposit > 0 {
                info!(
                    "CKB address: {} has weight: {}, added to voter list",
                    ckb_addr, deposit
                );
                let address = crate::AddressParser::default()
                    .set_network(ckb_net)
                    .parse(&ckb_addr)
                    .map_err(|e| eyre!(e))?;
                let lock_script = ckb_types::packed::Script::from(address.payload());
                let lock_hash_bytes = lock_script.calc_script_hash();
                voter_btree_set.insert(lock_hash_bytes);
            } else {
                info!(
                    "CKB address: {} has weight: {}, not qualified for voter list",
                    ckb_addr, deposit
                );
            }
        }
    }

    let mut voter_list = vec![];
    let mut smt_tree = CkbSMT::default();
    for lock_hash_bytes in voter_btree_set.iter() {
        let key: [u8; 32] = lock_hash_bytes.raw_data().to_vec().as_slice().try_into()?;
        smt_tree
            .update(key.into(), SMT_VALUE.into())
            .map_err(|e| eyre!(e))?;
        let lock_hash = hex::encode(key);
        voter_list.push(lock_hash);
    }

    let smt_root_hash = hex::encode(smt_tree.root().as_slice());
    let id = chrono::Local::now().to_rfc3339();
    info!(
        "Built voter list with {} entries, SMT root hash: {}, id: {}",
        voter_list.len(),
        smt_root_hash,
        id
    );
    VoterList::insert(&db, &id, voter_list, &smt_root_hash, block_number as i64).await
}
