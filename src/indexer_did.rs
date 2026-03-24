use std::{collections::HashMap, time::Duration};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

pub async fn did_set(url: &str, until_height: u64) -> Result<HashMap<String, String>> {
    reqwest::Client::new()
        .get(format!("{url}/did-set?until_height={until_height}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<HashMap<String, String>>()
        .await
        .map_err(|e| eyre!("decode indexer response failed: {e}"))
}

pub async fn did_document(url: &str, did: &str) -> Result<Value> {
    reqwest::Client::new()
        .get(format!("{url}/{did}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| eyre!("decode indexer response failed: {e}"))
}

pub async fn ckb_did(url: &str, ckb_addr: &str) -> Result<Vec<String>> {
    reqwest::Client::new()
        .get(format!("{url}/resolve-ckb-addr/{ckb_addr}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<Vec<String>>()
        .await
        .map_err(|e| eyre!("decode indexer response failed: {e}"))
}
