use std::collections::HashMap;

use ckb_sdk::{Address, AddressPayload, CkbRpcAsyncClient, NetworkType};
use ckb_types::{
    bytes::Bytes,
    core::{EpochNumberWithFraction, ScriptHashType},
    prelude::{Entity, Pack},
};
use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use serde_json::json;

pub async fn get_nervos_dao_deposit(ckb_client: &CkbRpcAsyncClient, ckb_addr: &str) -> Result<u64> {
    let address = crate::AddressParser::default()
        .set_network(ckb_sdk::NetworkType::Testnet)
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

    if let Ok(c) = pw_lock_capacity(ckb_client, &lock_hash).await {
        total_capacity += c;
    }

    Ok(total_capacity)
}

async fn pw_lock_capacity(
    ckb_client: &CkbRpcAsyncClient,
    lock: &ckb_types::packed::Script,
) -> Result<u64> {
    let mut total_capacity = 0;
    let code_hash = lock.code_hash().as_slice().to_vec();
    if Ok(code_hash)
        == hex::decode("f329effd1c475a2978453c8600e1eaf0bc2087ee093c3ee64cc96ec6847752cb")
    {
        let args = hex::encode(lock.args().raw_data());
        if args.starts_with("12") {
            let payload = AddressPayload::Full {
                hash_type: ScriptHashType::Type,
                code_hash: ckb_types::packed::Byte32::from_slice(
                    &hex::decode(
                        "58c5f491aba6d61678b7cf7edf4910b1f5e00ec0cde2f42e0abb4fd9aff25a63",
                    )
                    .unwrap(),
                )
                .unwrap(),
                args: Bytes::from_owner(lock.args().raw_data()[1..21].to_vec()),
            };
            let address = Address::new(NetworkType::Testnet, payload.clone(), true);

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
                            script: Some(ckb_jsonrpc_types::Script::from(ckb_types::packed::Script::from(address.payload()))),
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

#[tokio::test]
async fn test_pw_lock() {
    let ckb_addr = "ckt1qrejnmlar3r452tcg57gvq8patctcgy8acync0hxfnyka35ywafvkqgjv3se7nm9mjen690t26r3zfccuxkwzme5qq4q85en";
    let address = crate::AddressParser::default()
        .set_network(ckb_sdk::NetworkType::Testnet)
        .parse(ckb_addr)
        .unwrap();
    println!("{address}");
    let lock = ckb_types::packed::Script::from(address.payload());
    println!("{lock}");

    let b = lock.args().raw_data()[1..21].to_vec();
    println!("0x{}", hex::encode(&b));

    let payload = AddressPayload::Full {
        hash_type: ScriptHashType::Type,
        code_hash: ckb_types::packed::Byte32::from_slice(
            &hex::decode("58c5f491aba6d61678b7cf7edf4910b1f5e00ec0cde2f42e0abb4fd9aff25a63")
                .unwrap(),
        )
        .unwrap(),
        args: Bytes::from_owner(b),
    };
    let address = Address::new(NetworkType::Testnet, payload.clone(), true).to_string();
    println!("{address}");
}

#[test]
fn test_outpoint_to_args() {
    use ckb_types::prelude::Entity;
    let vote_meta_out_point: ckb_types::packed::OutPoint = ckb_jsonrpc_types::OutPoint {
        tx_hash: ckb_types::H256(
            hex::decode(
                "0x5e81c54bc21c321bea4993f4d04464c8cba7a545aae542e755e5b79b1fd12550"
                    .trim_start_matches("0x"),
            )
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
    assert_eq!(args, "0x6aa486510e313005d89dd8b5dbbb1d1110ba2d7b");
}

pub async fn get_vote_result(
    ckb_client: &CkbRpcAsyncClient,
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
    let search_key = json!({
        "script": {
            "code_hash": "0xb140de2d7d1536cfdcb82da7520475edce5785dff90edae9073c1143d88f50c5",
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
                let address = Address::new(NetworkType::Testnet, payload.clone(), true).to_string();
                debug!("address: {}", address);
                let weight =
                    crate::indexer_bind::get_weight(ckb_client, indexer_bind_url, &address)
                        .await
                        .unwrap_or(0);
                result.insert(address, (i, weight));
            }
        }
    }
    Ok(result)
}

#[test]
fn test_bit_flag() {
    let f: u8 = 1 << 2;
    println!("{f}");
    println!("{f:b}");
    let f = f.to_le_bytes();
    println!("{f:?}");
    let f = hex::encode(f);
    println!("{f}");

    let f = hex::decode(f).unwrap();
    let mut bs = String::new();
    for b in f {
        let b = b.reverse_bits();
        bs.push_str(&format!("{b:08b}"));
    }
    println!("{bs}, len: {}", bs.len());
    let indices = bs.match_indices('1');
    for (i, _) in indices {
        println!("index: {i}");
    }
}

#[tokio::test]
async fn test_get_vote_result() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");
    let indexer_bind_url = "";
    let r = get_vote_result(
        &ckb_client,
        indexer_bind_url,
        "0x5e81c54bc21c321bea4993f4d04464c8cba7a545aae542e755e5b79b1fd12550",
    )
    .await
    .unwrap();
    println!("{r:?}");
}

pub async fn get_ckb_addr_by_did(ckb_client: &CkbRpcAsyncClient, did: &str) -> Result<String> {
    let did = did.trim_start_matches("did:web5:");
    let did = did.trim_start_matches("did:ckb:");
    let did = did.trim_start_matches("did:plc:");
    let r = ckb_client
        .get_cells(
            ckb_sdk::rpc::ckb_indexer::SearchKey {
                script: ckb_jsonrpc_types::Script {
                    code_hash: ckb_types::H256(
                        hex::decode(
                            "510150477b10d6ab551a509b71265f3164e9fd4137fcb5a4322f49f03092c7c5",
                        )
                        .unwrap()
                        .try_into()
                        .unwrap(),
                    ),
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
    let ckb_addr = ckb_sdk::Address::new(ckb_sdk::NetworkType::Testnet, script.into(), true);
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

#[tokio::test]
async fn get_live_cell() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");

    let r = ckb_client
        .get_live_cell(
            ckb_jsonrpc_types::OutPoint {
                tx_hash: ckb_types::H256(
                    hex::decode("3071bec564eafa4eb981f56e028e65216af04a788ddbbc93cc2a2d625235b22a")
                        .unwrap()
                        .try_into()
                        .unwrap(),
                ),
                index: 0.into(),
            },
            false,
        )
        .await
        .unwrap();
    println!("{:?}", r);
}

#[tokio::test]
async fn get_tx() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");

    let t = ckb_client
        .get_transaction(ckb_types::H256(
            hex::decode("3071bec564eafa4eb981f56e028e65216af04a788ddbbc93cc2a2d625235b22a")
                .unwrap()
                .try_into()
                .unwrap(),
        ))
        .await
        .unwrap();
    match t.unwrap().transaction.unwrap().inner {
        ckb_jsonrpc_types::Either::Left(tx) => {
            tx.inner.outputs_data.iter().for_each(|d| {
                println!("{:?}", d);
            });
        }
        ckb_jsonrpc_types::Either::Right(bytes) => {
            println!("tx bytes: {:?}", bytes);
        }
    };
}

#[tokio::test]
async fn get_cells() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");
    let ckb_addr = "ckt1qrejnmlar3r452tcg57gvq8patctcgy8acync0hxfnyka35ywafvkqgjv3se7nm9mjen690t26r3zfccuxkwzme5qq4q85en";
    let total_capacity = get_nervos_dao_deposit(&ckb_client, ckb_addr).await.unwrap();
    println!("total capacity: {total_capacity}");
}

#[tokio::test]
async fn test_ckb_addr_by_did() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");
    let did = "wwokkmvehrkudo5jeengd4udqko3slc";
    let ckb_addr = get_ckb_addr_by_did(&ckb_client, did).await.unwrap();
    println!("ckb_addr: {ckb_addr}");
}

#[test]
fn test() {
    let s = "b59ca532a43c5541bba9211a61f283829db92c422eabf054c8fa3ea5adeabbe3";
    let bs = hex::decode(s).unwrap();
    let did = base32::encode(base32::Alphabet::Rfc4648Lower { padding: false }, &bs);
    println!("did: {}", did);
}

#[tokio::test]
async fn get_last() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");

    let r = ckb_client.get_blockchain_info().await.unwrap();
    println!("{:?}", r);

    let r = ckb_client.get_current_epoch().await.unwrap();
    println!("{:?}", r);

    let bn = ckb_client.get_tip_block_number().await.unwrap();
    println!("{:?}", bn);

    let r = EpochNumberWithFraction::new(
        r.number.into(),
        Into::<u64>::into(bn) - Into::<u64>::into(r.start_number),
        r.length.into(),
    );
    r.full_value();
    println!("{:?}", r);
}

pub async fn get_vote_time_range(
    ckb_client: &CkbRpcAsyncClient,
    duration_days: u64,
) -> Result<(u64, u64)> {
    let current_epoch = ckb_client.get_current_epoch().await?;
    let bn = ckb_client.get_tip_block_number().await?;

    let begin = EpochNumberWithFraction::new(
        current_epoch.number.into(),
        Into::<u64>::into(bn) - Into::<u64>::into(current_epoch.start_number),
        current_epoch.length.into(),
    );

    let end = EpochNumberWithFraction::new(
        Into::<u64>::into(current_epoch.number) + (6 * duration_days),
        Into::<u64>::into(bn) - Into::<u64>::into(current_epoch.start_number),
        current_epoch.length.into(),
    );
    Ok((begin.full_value(), end.full_value()))
}

// TODO: for test only, remove it later
pub async fn test_get_vote_time_range(ckb_client: &CkbRpcAsyncClient) -> Result<(u64, u64)> {
    let current_epoch = ckb_client.get_current_epoch().await?;
    let bn = ckb_client.get_tip_block_number().await?;

    let begin = EpochNumberWithFraction::new(
        current_epoch.number.into(),
        Into::<u64>::into(bn) - Into::<u64>::into(current_epoch.start_number),
        current_epoch.length.into(),
    );

    let index = Into::<u64>::into(bn) - Into::<u64>::into(current_epoch.start_number) + 50;
    let add = if index >= current_epoch.length.into() {
        (1, index - Into::<u64>::into(current_epoch.length))
    } else {
        (0, index)
    };

    let end = EpochNumberWithFraction::new(
        Into::<u64>::into(current_epoch.number) + add.0,
        add.1,
        current_epoch.length.into(),
    );
    Ok((begin.full_value(), end.full_value()))
}

#[test]
fn show_epoch() {
    let epoch = 1979140794232921;
    let epoch = EpochNumberWithFraction::from_full_value(epoch);
    println!("{epoch}");
}
