use std::time::Duration;

use ckb_sdk::CkbRpcAsyncClient;
use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use serde_json::Value;

use crate::ckb::get_nervos_dao_deposit;

pub async fn query_by_to(url: &str, to: &str) -> Result<Value> {
    reqwest::Client::new()
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

pub async fn query_by_from(url: &str, from: &str) -> Result<Value> {
    reqwest::Client::new()
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
    ckb_client: &CkbRpcAsyncClient,
    indexer_bind_url: &str,
    ckb_addr: &str,
) -> Result<u64> {
    let from_list = crate::indexer_bind::query_by_to(indexer_bind_url, ckb_addr).await?;
    let mut weight = get_nervos_dao_deposit(ckb_client, ckb_addr).await?;

    for from in from_list
        .as_array()
        .ok_or_eyre("from_list is not an array")?
    {
        debug!("from: {:?}", from);
        let from = from
            .get("from")
            .and_then(|f| f.as_str())
            .ok_or_eyre("missing from field")?;
        if from == ckb_addr {
            continue;
        }
        let nervos_dao_deposit = get_nervos_dao_deposit(ckb_client, from).await?;
        weight += nervos_dao_deposit;
    }
    Ok(weight)
}
