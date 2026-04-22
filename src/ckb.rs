use std::collections::HashMap;

use ckb_sdk::{Address, AddressPayload, CkbRpcAsyncClient, NetworkType};
use ckb_types::{
    bytes::Bytes,
    core::ScriptHashType,
    prelude::{Entity, Pack},
};
use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use serde_json::json;

// CKB contract code hashes
const OMNI_LOCK_MAINNET_CODE_HASH: &str =
    "9b819793a64463aed77c615d6cb226eea5487ccfc0783043a587254cda2b6f26";
const OMNI_LOCK_TESTNET_CODE_HASH: &str =
    "f329effd1c475a2978453c8600e1eaf0bc2087ee093c3ee64cc96ec6847752cb";

const PW_MAINNET_CODE_HASH: &str =
    "bf43c3602455798c1a61a596e0d95278864c552fafe231c063b3fabf97a8febc";
const PW_TESTNET_CODE_HASH: &str =
    "58c5f491aba6d61678b7cf7edf4910b1f5e00ec0cde2f42e0abb4fd9aff25a63";

const VOTE_MAINNET_CODE_HASH: &str =
    "0x38716b429cb139405d32ff86a916827862b2fa819916894848d8460da8953afb";
const VOTE_TESTNET_CODE_HASH: &str =
    "0xb140de2d7d1536cfdcb82da7520475edce5785dff90edae9073c1143d88f50c5";

const DID_MAINNET_CODE_HASH: &str =
    "4a06164dc34dccade5afe3e847a97b6db743e79f5477fa3295acf02849c5984a";
const DID_TESTNET_CODE_HASH: &str =
    "510150477b10d6ab551a509b71265f3164e9fd4137fcb5a4322f49f03092c7c5";

pub fn pw_lock(ckb_net: NetworkType, ckb_addr: &str) -> Option<Address> {
    if let Ok(address) = crate::AddressParser::default()
        .set_network(ckb_net)
        .parse(ckb_addr)
    {
        let lock = ckb_types::packed::Script::from(address.payload());
        let code_hash = lock.code_hash().as_slice().to_vec();
        let omni_code_hash = match ckb_net {
            NetworkType::Mainnet => OMNI_LOCK_MAINNET_CODE_HASH,
            NetworkType::Testnet
            | NetworkType::Dev
            | NetworkType::Staging
            | NetworkType::Preview => OMNI_LOCK_TESTNET_CODE_HASH,
        };
        if Ok(code_hash) == hex::decode(omni_code_hash) {
            let args = hex::encode(lock.args().raw_data());
            if args.starts_with("12") {
                let pw_code_hash = match ckb_net {
                    NetworkType::Mainnet => PW_MAINNET_CODE_HASH,
                    NetworkType::Testnet
                    | NetworkType::Dev
                    | NetworkType::Staging
                    | NetworkType::Preview => PW_TESTNET_CODE_HASH,
                };
                let payload = AddressPayload::Full {
                    hash_type: ScriptHashType::Type,
                    code_hash: ckb_types::packed::Byte32::from_slice(
                        &hex::decode(pw_code_hash).unwrap(),
                    )
                    .unwrap(),
                    args: Bytes::from_owner(lock.args().raw_data()[1..21].to_vec()),
                };
                let address = Address::new(ckb_net, payload.clone(), true);
                return Some(address);
            }
        }
    }
    None
}

pub async fn get_vote_result(
    ckb_client: &CkbRpcAsyncClient,
    ckb_net: NetworkType,
    indexer_bind_url: &str,
    indexer_dao_url: &str,
    vote_meta_tx_hash: &str,
) -> Result<HashMap<String, (usize, u64)>> {
    use ckb_types::prelude::Entity;
    let vote_meta_out_point: ckb_types::packed::OutPoint = ckb_jsonrpc_types::OutPoint {
        tx_hash: ckb_types::H256(
            hex::decode(vote_meta_tx_hash.trim_start_matches("0x"))
                .unwrap()
                .try_into()
                .unwrap(),
        ),
        index: 0.into(),
    }
    .into();
    let pubkey_hash = ckb_hash::blake2b_256(vote_meta_out_point.as_bytes());
    let args = pubkey_hash[0..20].to_vec();
    let args = format!("0x{}", hex::encode(args));
    let vote_code_hash = match ckb_net {
        NetworkType::Mainnet => VOTE_MAINNET_CODE_HASH,
        NetworkType::Testnet | NetworkType::Dev | NetworkType::Staging | NetworkType::Preview => {
            VOTE_TESTNET_CODE_HASH
        }
    };
    let search_key = json!({
        "script": {
            "code_hash": vote_code_hash,
            "hash_type": "type",
            "args": args
        },
        "script_type": "type"
    });
    let search_key: ckb_sdk::rpc::ckb_indexer::SearchKey = serde_json::from_value(search_key)?;
    let r = ckb_client
        .get_cells(
            search_key,
            ckb_sdk::rpc::ckb_indexer::Order::Asc,
            10000.into(),
            None,
        )
        .await?;
    let mut result = HashMap::new();
    for cell in &r.objects {
        if let Some(data) = &cell.output_data {
            let mut bs = String::new();
            for b in data.as_bytes() {
                let b = b.reverse_bits();
                bs.push_str(&format!("{b:08b}"));
            }
            let indices = bs.match_indices('1');
            for (i, _) in indices {
                let payload = AddressPayload::Full {
                    hash_type: ScriptHashType::Type,
                    code_hash: cell.output.lock.code_hash.pack(),
                    args: cell.output.lock.args.clone().into_bytes(),
                };
                let address = Address::new(ckb_net, payload.clone(), true).to_string();
                debug!("address: {}", address);
                let weight = crate::indexer_bind::get_weight(
                    ckb_net,
                    indexer_bind_url,
                    indexer_dao_url,
                    &address,
                    None,
                )
                .await
                .map(|wp| wp.values().sum())
                .unwrap_or(0);
                result.insert(address, (i, weight));
            }
        }
    }
    Ok(result)
}

pub async fn get_ckb_addr_by_did(
    ckb_client: &CkbRpcAsyncClient,
    ckb_net: &NetworkType,
    did: &str,
) -> Result<String> {
    let did = did.trim_start_matches("did:web5:");
    let did = did.trim_start_matches("did:ckb:");
    let did = did.trim_start_matches("did:plc:");
    let code_hash = match ckb_net {
        NetworkType::Mainnet => DID_MAINNET_CODE_HASH,
        NetworkType::Testnet | NetworkType::Dev | NetworkType::Staging | NetworkType::Preview => {
            DID_TESTNET_CODE_HASH
        }
    };
    let r = ckb_client
        .get_cells(
            ckb_sdk::rpc::ckb_indexer::SearchKey {
                script: ckb_jsonrpc_types::Script {
                    code_hash: ckb_types::H256(hex::decode(code_hash).unwrap().try_into().unwrap()),
                    hash_type: ckb_jsonrpc_types::ScriptHashType::Type,
                    args: ckb_jsonrpc_types::JsonBytes::from_vec(
                        base32::decode(base32::Alphabet::Rfc4648Lower { padding: false }, did)
                            .ok_or_eyre("did format is invalid")?,
                    ),
                },
                script_type: ckb_sdk::rpc::ckb_indexer::ScriptType::Type,
                script_search_mode: None,
                filter: None,
                with_data: None,
                group_by_transaction: None,
            },
            ckb_sdk::rpc::ckb_indexer::Order::Asc,
            1.into(),
            None,
        )
        .await?;
    let output: &ckb_jsonrpc_types::CellOutput = &r.objects.first().ok_or_eyre("Not Found")?.output;
    let script: ckb_types::packed::Script = output.lock.clone().into();
    let ckb_addr = ckb_sdk::Address::new(*ckb_net, script.into(), true);
    Ok(ckb_addr.to_string())
}

pub async fn get_tx_status(
    ckb_client: &CkbRpcAsyncClient,
    tx_hash: &str,
) -> Result<ckb_jsonrpc_types::Status> {
    let tx_hash: [u8; 32] = hex::decode(tx_hash.strip_prefix("0x").unwrap_or(tx_hash))?
        .try_into()
        .map_err(|_| eyre!("invalid tx_hash format"))?;
    let tx_status = ckb_client.get_transaction(ckb_types::H256(tx_hash)).await?;
    tx_status
        .ok_or_eyre("get tx error")
        .map(|t| t.tx_status.status)
}
