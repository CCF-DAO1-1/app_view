use ckb_sdk::CkbRpcAsyncClient;
use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};

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
                    script: Some(ckb_jsonrpc_types::Script::from(lock_hash)),
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
            1000.into(),
            None,
        )
        .await?;
    let mut total_capacity = 0;
    for cell in &r.objects {
        let output: &ckb_jsonrpc_types::CellOutput = &cell.output;
        total_capacity += output.capacity.value();
    }
    Ok(total_capacity)
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
                    hex::decode("db189a3e2106f7a1b0373d6365571bae14c9af17d0d21290a47d428f570ad0a7")
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
async fn get_cells() {
    let ckb_client = ckb_sdk::CkbRpcAsyncClient::new("https://testnet.ckb.dev/");
    let ckb_addr = "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqtyy4lspd4k86v8vz06n03dpjrdx5gzp7cxulwv8";
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
