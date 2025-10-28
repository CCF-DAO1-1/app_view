use std::time::Duration;

use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use serde_json::Value;

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
