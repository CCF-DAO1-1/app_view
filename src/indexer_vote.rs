use std::{sync::OnceLock, time::Duration};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

pub async fn all_votes(
    url: &str,
    args: &str,
    epoch_number: i64,
    epoch_index: i64,
    epoch_length: i64,
) -> Result<Value> {
    let rsp = http_client()
        .get(format!("{url}/all-votes"))
        .query(&[
            ("args", args),
            ("epoch_number", &epoch_number.to_string()),
            ("epoch_index", &epoch_index.to_string()),
            ("epoch_length", &epoch_length.to_string()),
        ])
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?;
    debug!("all_votes rsp: {:?}", rsp);
    let text = rsp.text().await?;
    debug!("all_votes rsp text: {:?}", text);
    let json: Value =
        serde_json::from_str(&text).map_err(|e| eyre!("decode indexer response failed: {e}"))?;
    Ok(json)
}

pub async fn address_vote(
    url: &str,
    args: &str,
    ckb_addr: &str,
    epoch_number: i64,
    epoch_index: i64,
    epoch_length: i64,
) -> Result<Value> {
    http_client()
        .get(format!("{url}/address-vote"))
        .query(&[
            ("args", args),
            ("ckb_addr", ckb_addr),
            ("epoch_number", &epoch_number.to_string()),
            ("epoch_index", &epoch_index.to_string()),
            ("epoch_length", &epoch_length.to_string()),
        ])
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| eyre!("decode indexer response failed: {e}"))
}
