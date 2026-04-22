use std::{sync::OnceLock, time::Duration};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

pub const NSID_PROPOSAL: &str = "app.dao.proposal";
pub const NSID_REPLY: &str = "app.dao.reply";
pub const NSID_LIKE: &str = "app.dao.like";
pub const NSID_PROFILE: &str = "app.actor.profile";

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

pub async fn get_record(url: &str, repo: &str, nsid: &str, rkey: &str) -> Result<Value> {
    http_client()
        .get(format!("{url}/xrpc/com.atproto.repo.getRecord"))
        .query(&[("repo", repo), ("collection", nsid), ("rkey", rkey)])
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| eyre!("decode pds response failed: {e}"))
}
