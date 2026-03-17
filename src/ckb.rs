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

pub async fn get_nervos_dao_deposit(
    ckb_client: &CkbRpcAsyncClient,
    ckb_net: NetworkType,
    ckb_addr: &str,
) -> Result<u64> {
    let address = crate::AddressParser::default()
        .set_network(ckb_net)
        .parse(ckb_addr)
        .map_err(|e| eyre!(e))?;
    let lock_hash = ckb_types::packed::Script::from(address.payload());
    let r = ckb_client
        .get_cells(
            ckb_sdk::rpc::ckb_indexer::SearchKey {
                script: ckb_jsonrpc_types::Script {
                    code_hash: ckb_types::H256(
                        hex::decode(
                            "82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e",
                        )
                        .unwrap()
                        .try_into()
                        .unwrap(),
                    ),
                    hash_type: ckb_jsonrpc_types::ScriptHashType::Type,
                    args: ckb_jsonrpc_types::JsonBytes::default(),
                },
                script_type: ckb_sdk::rpc::ckb_indexer::ScriptType::Type,
                script_search_mode: None,
                filter: Some(ckb_sdk::rpc::ckb_indexer::SearchKeyFilter {
                    script: Some(ckb_jsonrpc_types::Script::from(lock_hash.clone())),
                    script_len_range: None,
                    output_data: None,
                    output_data_filter_mode: None,
                    output_data_len_range: None,
                    output_capacity_range: None,
                    block_range: None,
                }),
                with_data: None,
                group_by_transaction: None,
            },
            ckb_sdk::rpc::ckb_indexer::Order::Asc,
            10000.into(),
            None,
        )
        .await?;
    let mut total_capacity = 0;
    for cell in &r.objects {
        let output: &ckb_jsonrpc_types::CellOutput = &cell.output;
        total_capacity += output.capacity.value();
    }

    if let Ok(c) = pw_lock_capacity(ckb_client, ckb_net, &lock_hash).await {
        total_capacity += c;
    }

    Ok(total_capacity)
}

async fn pw_lock_capacity(
    ckb_client: &CkbRpcAsyncClient,
    ckb_net: NetworkType,
    lock: &ckb_types::packed::Script,
) -> Result<u64> {
    let mut total_capacity = 0;
    let code_hash = lock.code_hash().as_slice().to_vec();
    let l_code_hash = match ckb_net {
        NetworkType::Mainnet => "9b819793a64463aed77c615d6cb226eea5487ccfc0783043a587254cda2b6f26",
        NetworkType::Testnet | NetworkType::Dev | NetworkType::Staging | NetworkType::Preview => {
            "f329effd1c475a2978453c8600e1eaf0bc2087ee093c3ee64cc96ec6847752cb"
        }
    };
    if Ok(code_hash) == hex::decode(l_code_hash) {
        let args = hex::encode(lock.args().raw_data());
        if args.starts_with("12") {
            let pw_code_hash = match ckb_net {
                NetworkType::Mainnet => {
                    "bf43c3602455798c1a61a596e0d95278864c552fafe231c063b3fabf97a8febc"
                }
                NetworkType::Testnet
                | NetworkType::Dev
                | NetworkType::Staging
                | NetworkType::Preview => {
                    "58c5f491aba6d61678b7cf7edf4910b1f5e00ec0cde2f42e0abb4fd9aff25a63"
                }
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
            let r = ckb_client
                .get_cells(
                    ckb_sdk::rpc::ckb_indexer::SearchKey {
                        script: ckb_jsonrpc_types::Script {
                            code_hash: ckb_types::H256(
                                hex::decode("82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e").unwrap().try_into().unwrap(),
                            ),
                            hash_type: ckb_jsonrpc_types::ScriptHashType::Type,
                            args: ckb_jsonrpc_types::JsonBytes::default(),
                        },
                        script_type: ckb_sdk::rpc::ckb_indexer::ScriptType::Type,
                        script_search_mode: None,
                        filter: Some(ckb_sdk::rpc::ckb_indexer::SearchKeyFilter {
                            script: Some(ckb_jsonrpc_types::Script::from(
                                ckb_types::packed::Script::from(address.payload()),
                            )),
                            script_len_range: None,
                            output_data: None,
                            output_data_filter_mode: None,
                            output_data_len_range: None,
                            output_capacity_range: None,
                            block_range: None,
                        }),
                        with_data: None,
                        group_by_transaction: None,
                    },
                    ckb_sdk::rpc::ckb_indexer::Order::Asc,
                    10000.into(),
                    None,
                )
                .await?;
            for cell in &r.objects {
                let output: &ckb_jsonrpc_types::CellOutput = &cell.output;
                total_capacity += output.capacity.value();
            }
        }
    }
    Ok(total_capacity)
}

pub async fn get_vote_result(
    ckb_client: &CkbRpcAsyncClient,
    ckb_net: NetworkType,
    indexer_bind_url: &str,
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
        NetworkType::Mainnet => {
            "0x38716b429cb139405d32ff86a916827862b2fa819916894848d8460da8953afb"
        }
        NetworkType::Testnet | NetworkType::Dev | NetworkType::Staging | NetworkType::Preview => {
            "0xb140de2d7d1536cfdcb82da7520475edce5785dff90edae9073c1143d88f50c5"
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
                    ckb_client,
                    ckb_net,
                    indexer_bind_url,
                    &address,
                )
                .await
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
        NetworkType::Mainnet => "4a06164dc34dccade5afe3e847a97b6db743e79f5477fa3295acf02849c5984a",
        NetworkType::Testnet | NetworkType::Dev | NetworkType::Staging | NetworkType::Preview => {
            "510150477b10d6ab551a509b71265f3164e9fd4137fcb5a4322f49f03092c7c5"
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
