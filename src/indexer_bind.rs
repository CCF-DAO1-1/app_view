use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::Duration,
};

use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use serde_json::Value;

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

pub async fn query_by_to(url: &str, to: &str) -> Result<Value> {
    http_client()
        .get(format!("{url}/by_to/{to}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<Value>()
        .await
        .map(|r| {
            r.pointer("/data")
                .cloned()
                .ok_or_eyre("missing data field in indexer response")
        })?
}

pub async fn query_by_to_at_height(url: &str, to: &str, height: u64) -> Result<Value> {
    http_client()
        .get(format!("{url}/by_to_at_height/{to}/{height}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<Value>()
        .await
        .map(|r| {
            r.pointer("/data")
                .cloned()
                .ok_or_eyre("missing data field in indexer response")
        })?
}

pub async fn query_by_from(url: &str, from: &str) -> Result<Value> {
    http_client()
        .get(format!("{url}/by_from/{from}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<Value>()
        .await
        .map(|r| {
            r.pointer("/data")
                .cloned()
                .ok_or_eyre("missing data field in indexer response")
        })?
}

pub async fn get_weight(
    ckb_net: ckb_sdk::NetworkType,
    indexer_bind_url: &str,
    indexer_dao_url: &str,
    ckb_addr: &str,
    until_block_number: Option<u64>,
) -> Result<HashMap<String, u64>> {
    let from_list = if let Some(until_block_number) = until_block_number {
        query_by_to_at_height(indexer_bind_url, ckb_addr, until_block_number).await?
    } else {
        query_by_to(indexer_bind_url, ckb_addr).await?
    };
    let mut ckb_addrs: HashSet<String> = from_list
        .as_array()
        .ok_or_eyre("from_list is not an array")?
        .iter()
        .filter_map(|from| {
            from.get("from")
                .and_then(|f| f.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    ckb_addrs.insert(ckb_addr.to_string());
    // PWLock
    if let Some(pw_lock_addr) = crate::ckb::pw_lock(ckb_net, ckb_addr) {
        ckb_addrs.insert(pw_lock_addr.to_string());
    }
    if ckb_addrs.len() > 20 {
        let mut weight_map = HashMap::<String, u64>::new();
        // every 20 addresses in one batch to avoid too long url query
        let ckb_addr_vec: Vec<String> = ckb_addrs.into_iter().collect();
        for ckb_addr_batch in ckb_addr_vec.chunks(20) {
            let batch_weight_map = crate::indexer_dao::query_dao_stake_until_height(
                indexer_dao_url,
                until_block_number,
                ckb_addr_batch,
            )
            .await?;
            weight_map.extend(batch_weight_map);
        }

        Ok(weight_map)
    } else {
        crate::indexer_dao::query_dao_stake_until_height(
            indexer_dao_url,
            until_block_number,
            &ckb_addrs.into_iter().collect::<Vec<_>>(),
        )
        .await
    }
}
