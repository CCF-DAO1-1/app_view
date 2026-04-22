use std::{collections::HashMap, sync::OnceLock, time::Duration};

use color_eyre::{Result, eyre::eyre};
use serde_json::json;

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

pub async fn query_dao_stake_until_height(
    url: &str,
    until_height: Option<u64>,
    ckb_addrs: &[String],
) -> Result<HashMap<String, u64>> {
    http_client()
        .post(format!("{url}/dao-stake-set"))
        .body(
            json!({
                "ckb_list": ckb_addrs,
                "until_height": until_height
            })
            .to_string(),
        )
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<HashMap<String, u64>>()
        .await
        .map_err(|e| eyre!("decode indexer response failed: {e}"))
}
