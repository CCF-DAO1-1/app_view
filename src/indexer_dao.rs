use std::{collections::HashMap, time::Duration};

use color_eyre::{Result, eyre::eyre};

pub async fn query_dao_stake_until_height(
    url: &str,
    until_height: i64,
    ckb_addrs: &str,
) -> Result<HashMap<String, u64>> {
    reqwest::Client::new()
        .get(format!(
            "{url}/dao-stake-set?until_height={until_height}&ckb_list={ckb_addrs}"
        ))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call indexer failed: {e}"))?
        .json::<HashMap<String, u64>>()
        .await
        .map_err(|e| eyre!("decode indexer response failed: {e}"))
}
