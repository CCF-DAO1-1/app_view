use std::str::FromStr;

use ckb_sdk::{Address, AddressPayload, AddressType, CodeHashIndex, NetworkType, OldAddress};
use ckb_types::{H256, prelude::Unpack};
use color_eyre::{
    Result,
    eyre::{OptionExt, eyre},
};
use k256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::Serialize;

pub mod api;
pub mod atproto;
pub mod ckb;
pub mod error;
pub mod indexer_bind;
pub mod indexer_did;
pub mod lexicon;
pub mod molecules;
pub mod scheduler;
pub mod smt;
pub mod tid;

#[macro_use]
extern crate tracing as logger;

#[derive(Clone)]
pub struct AppView {
    pub db: sqlx::Pool<sqlx::Postgres>,
    pub pds: String,
    pub indexer_bind_url: String,
    pub indexer_did_url: String,
    pub ckb_client: ckb_sdk::CkbRpcAsyncClient,
    pub whitelist: Vec<String>,
}

pub enum AddressPayloadOption {
    Short(Option<CodeHashIndex>),
    #[allow(dead_code)]
    Full(Option<H256>),
    #[allow(dead_code)]
    FullData(Option<H256>),
    FullType(Option<H256>),
}

impl Default for AddressPayloadOption {
    fn default() -> AddressPayloadOption {
        AddressPayloadOption::Short(Some(CodeHashIndex::Sighash))
    }
}

#[derive(Default)]
pub struct AddressParser {
    network: Option<NetworkType>,
    payload: Option<AddressPayloadOption>,
}

impl AddressParser {
    pub const fn new(
        network: Option<NetworkType>,
        payload: Option<AddressPayloadOption>,
    ) -> AddressParser {
        AddressParser { network, payload }
    }

    pub const fn new_sighash() -> Self {
        AddressParser {
            network: None,
            payload: Some(AddressPayloadOption::Short(Some(CodeHashIndex::Sighash))),
        }
    }
    pub const fn new_multisig() -> Self {
        AddressParser {
            network: None,
            payload: Some(AddressPayloadOption::Short(Some(CodeHashIndex::Multisig))),
        }
    }

    pub const fn set_network(&mut self, network: NetworkType) -> &mut Self {
        self.network = Some(network);
        self
    }

    pub const fn set_network_opt(&mut self, network: Option<NetworkType>) -> &mut Self {
        self.network = network;
        self
    }

    pub const fn set_short(&mut self, code_hash_index: CodeHashIndex) -> &mut Self {
        self.payload = Some(AddressPayloadOption::Short(Some(code_hash_index)));
        self
    }

    #[allow(dead_code)]
    pub const fn set_full(&mut self, code_hash: H256) -> &mut Self {
        self.payload = Some(AddressPayloadOption::Full(Some(code_hash)));
        self
    }
    #[allow(dead_code)]
    pub const fn set_full_data(&mut self, code_hash: H256) -> &mut Self {
        self.payload = Some(AddressPayloadOption::FullData(Some(code_hash)));
        self
    }
    pub const fn set_full_type(&mut self, code_hash: H256) -> &mut Self {
        self.payload = Some(AddressPayloadOption::FullType(Some(code_hash)));
        self
    }
}

impl AddressParser {
    fn parse(&self, input: &str) -> Result<Address, String> {
        fn check_code_hash(
            payload: &AddressPayload,
            code_hash_opt: Option<&H256>,
        ) -> Result<(), String> {
            if let Some(code_hash) = code_hash_opt {
                let payload_code_hash: H256 = payload.code_hash(None).unpack();
                if code_hash != &payload_code_hash {
                    return Err(format!(
                        "Invalid code hash: {:#x}, expected: {:#x}",
                        payload_code_hash, code_hash
                    ));
                }
            }
            Ok(())
        }

        if let Ok(address) = Address::from_str(input) {
            if let Some(network) = self.network
                && address.network().to_prefix() != network.to_prefix()
            {
                return Err(format!(
                    "Invalid network: {}, expected: {}",
                    address.network().to_prefix(),
                    network.to_prefix(),
                ));
            }
            if let Some(payload_option) = self.payload.as_ref() {
                let payload = address.payload();
                match payload_option {
                    AddressPayloadOption::Short(index_opt) => match payload {
                        AddressPayload::Short { index, .. } => {
                            if let Some(expected_index) = index_opt
                                && index != expected_index
                            {
                                return Err(format!(
                                    "Invalid address code hash index: {:?}, expected: {:?}",
                                    index, expected_index,
                                ));
                            }
                        }
                        _ => {
                            return Err(format!(
                                "Invalid address type: {:?}, expected: {:?}",
                                payload.ty(true),
                                AddressType::Short,
                            ));
                        }
                    },
                    AddressPayloadOption::Full(code_hash_opt) => {
                        if payload.ty(true) == AddressType::Short {
                            return Err(format!(
                                "Unexpected address type: {:?}",
                                AddressType::Short
                            ));
                        }
                        check_code_hash(payload, code_hash_opt.as_ref())?;
                    }
                    AddressPayloadOption::FullData(code_hash_opt) => {
                        if payload.ty(true) != AddressType::FullData {
                            return Err(format!(
                                "Unexpected address type: {:?}, expected: {:?}",
                                payload.ty(true),
                                AddressType::FullData
                            ));
                        }
                        check_code_hash(payload, code_hash_opt.as_ref())?;
                    }
                    AddressPayloadOption::FullType(code_hash_opt) => {
                        if payload.ty(true) != AddressType::FullType {
                            return Err(format!(
                                "Unexpected address type: {:?}, expected: {:?}",
                                payload.ty(true),
                                AddressType::FullType
                            ));
                        }
                        check_code_hash(payload, code_hash_opt.as_ref())?;
                    }
                }
            }
            return Ok(address);
        }

        // Fallback to old format address (TODO: move this logic to upper level)
        let prefix = input.chars().take(3).collect::<String>();
        let network = NetworkType::from_prefix(prefix.as_str())
            .ok_or_else(|| format!("Invalid address prefix: {}", prefix))?;
        let old_address = OldAddress::from_input(network, input)?;
        let payload = AddressPayload::from_pubkey_hash(old_address.hash().clone());
        Ok(Address::new(NetworkType::Testnet, payload, true))
    }
}

pub async fn get_network_type(rpc_client: &ckb_sdk::CkbRpcAsyncClient) -> Result<NetworkType> {
    let chain_info = rpc_client.get_blockchain_info().await?;
    NetworkType::from_raw_str(chain_info.chain.as_str())
        .ok_or_else(|| eyre!("Unsupported network type: {}", chain_info.chain))
}

pub async fn verify_signature<T>(
    did: &str,
    indexer_did_url: &str,
    signing_key_did: &str,
    signed_bytes: &str,
    message: &T,
) -> Result<()>
where
    T: Serialize + ?Sized,
{
    // verify did
    let did_doc = crate::indexer_did::did_document(indexer_did_url, did)
        .await
        .map_err(|e| eyre!("get did doc failed: {e}"))?;

    if signing_key_did
        != did_doc
            .pointer("/verificationMethods/atproto")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
    {
        return Err(eyre!("signing_key_did not match"));
    }

    // verify signature
    let verifying_key: VerifyingKey = signing_key_did
        .split_once("did:key:z")
        .and_then(|(_, key)| {
            let bytes = bs58::decode(key).into_vec().ok()?;
            VerifyingKey::from_sec1_bytes(&bytes[2..]).ok()
        })
        .ok_or_eyre("invalid signing_key_did")?;
    let signature = hex::decode(signed_bytes)
        .map(|bytes| Signature::from_slice(&bytes).map_err(|e| eyre!(e)))??;

    let unsigned_bytes = serde_ipld_dagcbor::to_vec(message)?;
    debug!("unsigned_bytes: {}", hex::encode(&unsigned_bytes));
    verifying_key
        .verify(&unsigned_bytes, &signature)
        .map_err(|e| eyre!("verify signature failed: {e}"))
}
