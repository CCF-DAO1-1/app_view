#![allow(dead_code)]

use std::{str::FromStr, time::Duration};

use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const NSID_PROPOSAL: &str = "app.dao.proposal";
pub const NSID_REPLY: &str = "app.dao.reply";
pub const NSID_LIKE: &str = "app.dao.like";
pub const NSID_PROFILE: &str = "app.actor.profile";

pub async fn create_record(
    url: &str,
    auth: &str,
    repo: &str,
    nsid: &str,
    record: &Value,
) -> Result<Value> {
    reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.repo.createRecord"))
        .bearer_auth(auth)
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(
            json!({
                "repo": repo,
                "collection": nsid,
                "validate": false,
                "record": record,
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn get_record(url: &str, repo: &str, nsid: &str, rkey: &str) -> Result<Value> {
    reqwest::Client::new()
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

pub async fn put_record(
    url: &str,
    auth: &str,
    repo: &str,
    nsid: &str,
    rkey: &str,
    record: &Value,
) -> Result<Value> {
    reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.repo.putRecord"))
        .bearer_auth(auth)
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(
            json!({
                "repo": repo,
                "collection": nsid,
                "rkey": rkey,
                "validate": false,
                "record": record,
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn pre_index_action(url: &str, did: &str, ckb_addr: &str) -> Result<Value> {
    let rsp = reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.web5.preIndexAction"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(
            json!({
                "did": did,
                "ckbAddr": ckb_addr,
                "index": {
                    "$type":"com.atproto.web5.preIndexAction#createSession"
                }
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?;
    debug!("pds rsp: {rsp:?}");
    let body_str = rsp
        .text()
        .await
        .map_err(|e| eyre!("read pds response failed: {e}"))?;
    debug!("pds rsp body: {body_str}");
    Value::from_str(&body_str).map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn index_action(
    url: &str,
    did: &str,
    ckb_addr: &str,
    msg: &str,
    signed_bytes: &str,
    signing_key: &str,
) -> Result<Value> {
    let rsp = reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.web5.indexAction"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(
            json!({
                "did": did,
                "ckbAddr": ckb_addr,
                "index": {
                    "$type":"com.atproto.web5.indexAction#createSession"
                },
                "message": msg,
                "signedBytes": signed_bytes,
                "signingKey": signing_key,
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?;
    debug!("pds rsp: {rsp:?}");
    let body_str = rsp
        .text()
        .await
        .map_err(|e| eyre!("read pds response failed: {e}"))?;
    debug!("pds rsp body: {body_str}");
    Value::from_str(&body_str).map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn pre_direct_writes(url: &str, auth: &str, repo: &str, writes: &Value) -> Result<Value> {
    let body = json!({
        "repo": repo,
        "validate": false,
        "writes": writes,
    });
    debug!(
        "pre_direct_writes body: {}",
        serde_json::to_string_pretty(&body)?
    );
    let rsp = reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.web5.preDirectWrites"))
        .bearer_auth(auth)
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?;
    debug!("pds rsp: {rsp:?}");
    let body_str = rsp
        .text()
        .await
        .map_err(|e| eyre!("read pds response failed: {e}"))?;
    debug!("pds rsp body: {body_str}");
    Value::from_str(&body_str).map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn direct_writes(
    url: &str,
    auth: &str,
    repo: &str,
    writes: &Value,
    signing_key: &str,
    ckb_addr: &str,
    root: &Value,
) -> Result<Value> {
    let body = json!({
        "repo": repo,
        "validate": false,
        "writes": writes,
        "signingKey": signing_key,
        "root": root,
        "ckbAddr": ckb_addr,
    });
    debug!(
        "direct_writes body: {}",
        serde_json::to_string_pretty(&body)?
    );
    let rsp = reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.web5.directWrites"))
        .bearer_auth(auth)
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?;
    debug!("pds rsp: {rsp:?}");
    let body_str = rsp
        .text()
        .await
        .map_err(|e| eyre!("read pds response failed: {e}"))?;
    debug!("pds rsp body: {body_str}");
    Value::from_str(&body_str).map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn index_query(url: &str, did: &str, item: &str) -> Result<Value> {
    let rsp = reqwest::Client::new()
        .post(format!("{url}/xrpc/com.atproto.web5.indexQuery"))
        .header("Content-Type", "application/json; charset=utf-8")
        .timeout(Duration::from_secs(5))
        .body(
            json!({
                "index": {
                    "$type": format!("com.atproto.web5.indexQuery#{}", item),
                    "did": did,
                },
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|e| eyre!("call pds failed: {e}"))?;
    debug!("pds rsp: {rsp:?}");
    let body_str = rsp
        .text()
        .await
        .map_err(|e| eyre!("read pds response failed: {e}"))?;
    debug!("pds rsp body: {body_str}");
    Value::from_str(&body_str).map_err(|e| eyre!("decode pds response failed: {e}"))
}

pub async fn create_session(
    pds_url: &str,
    repo: &str,
    signing_key_hex: &str,
    ckb_addr: &str,
) -> Result<String> {
    use k256::ecdsa::signature::SignerMut;

    let pre_result = pre_index_action(pds_url, repo, ckb_addr).await?;
    debug!("Pre Index Action Response: {:#}", pre_result);

    let mut signing_key = k256::ecdsa::SigningKey::from_slice(&hex::decode(signing_key_hex)?)?;

    let msg = pre_result["message"]
        .as_str()
        .ok_or_eyre("message not found")?;
    let sig: k256::ecdsa::Signature = signing_key.sign(msg.as_bytes());

    let signed_bytes = format!("0x{}", hex::encode(sig.to_vec()));
    let verifying_key = signing_key.verifying_key();

    let signing_key = [
        [0xe7, 0x01].to_vec(),
        verifying_key.to_encoded_point(true).as_bytes().to_vec(),
    ]
    .concat();
    let signing_key = bs58::encode(signing_key).into_string();
    let signing_key = format!("did:key:z{}", signing_key);

    debug!("signed_bytes: {signed_bytes}");
    debug!("signing_key: {signing_key}");
    let r = index_action(
        pds_url,
        repo,
        ckb_addr,
        pre_result["message"]
            .as_str()
            .ok_or_eyre("message not found")?,
        &signed_bytes,
        &signing_key,
    )
    .await?;
    debug!("Index Action Response: {:#}", r);

    Ok(r.pointer("/result/accessJwt")
        .ok_or_eyre("/result/accessJwt not found")?
        .as_str()
        .ok_or_eyre("/result/accessJwt not found")?
        .to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Write {
    pub collection: String,
    pub rkey: String,
    pub value: Value,
}

pub async fn write_to_pds(
    pds_url: &str,
    auth: &str,
    repo: &str,
    write: &Write,
    is_update: bool,
    signing_key_hex: &str,
    ckb_addr: &str,
) -> Result<Value> {
    let signing_key = k256::ecdsa::SigningKey::from_slice(&hex::decode(signing_key_hex)?)?;
    let verifying_key = signing_key.verifying_key();

    debug!(
        "verifying_key: {}",
        hex::encode(verifying_key.to_encoded_point(true).as_bytes())
    );

    let operate = if is_update { "update" } else { "create" };

    let writes = json!([{
        "$type": format!("com.atproto.web5.preDirectWrites#{operate}"),
        "collection": write.collection,
        "rkey": write.rkey,
        "value": write.value
    }]);

    let signing_key_did = [
        [0xe7, 0x01].to_vec(),
        verifying_key.to_encoded_point(true).as_bytes().to_vec(),
    ]
    .concat();
    let signing_key_did = bs58::encode(signing_key_did).into_string();
    let signing_key_did = format!("did:key:z{}", signing_key_did);

    let pre_write = pre_direct_writes(pds_url, auth, repo, &writes).await?;
    debug!("Pre Direct Writes Response: {:#}", pre_write);

    use k256::ecdsa::signature::Signer;
    let sig: k256::ecdsa::Signature = signing_key.sign(
        hex::decode(
            pre_write["unSignBytes"]
                .as_str()
                .ok_or_eyre("unSignBytes not found")?
                .as_bytes(),
        )?
        .as_slice(),
    );
    let signed_bytes = hex::encode(sig.to_vec());
    debug!("signed_bytes: {signed_bytes}");

    let mut root = json!({
        "did": repo,
        "version": 3,
        "rev": pre_write["rev"],
        "data": pre_write["data"],
        "signedBytes": signed_bytes,
    });
    if let Some(prev) = pre_write.get("prev") {
        root["prev"] = prev.clone();
    }

    direct_writes(
        pds_url,
        auth,
        repo,
        &json!([{
            "$type": format!("com.atproto.web5.directWrites#{operate}"),
            "collection": write.collection,
            "rkey": write.rkey,
            "value": write.value
        }]),
        &signing_key_did,
        ckb_addr,
        &root,
    )
    .await
}
