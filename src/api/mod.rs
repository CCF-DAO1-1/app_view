pub mod like;
pub mod meeting;
pub mod proposal;
pub mod record;
pub mod reply;
pub mod repo;
pub mod task;
pub mod timeline;
pub mod vote;

use color_eyre::eyre::{OptionExt, eyre};
use k256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use molecule::prelude::Entity;
use sea_query::{Expr, ExprTrait, Order, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{
    Modify, OpenApi, ToSchema,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};
use validator::Validate;

use crate::{
    AppView,
    atproto::{NSID_PROFILE, get_record},
    ckb::get_vote_time_range,
    lexicon::{
        self,
        profile::{Profile, ProfileRow},
        proposal::ProposalState,
        vote_meta::{VoteMeta, VoteMetaRow, VoteMetaState},
        vote_whitelist::{VoteWhitelist, VoteWhitelistRow},
    },
};

#[derive(OpenApi, Debug, Clone, Copy)]
#[openapi(
    modifiers(&SecurityAddon),
    paths(
        record::create,
        record::update,
        repo::profile,
        proposal::list,
        proposal::detail,
        proposal::initiation_vote,
        proposal::update_state,
        proposal::update_receiver_addr,
        proposal::receiver_addr,
        proposal::list_self,
        proposal::replied,
        reply::list,
        like::list,
        vote::bind_list,
        vote::weight,
        vote::whitelist,
        vote::proof,
        vote::build_whitelist,
        vote::update_meta_tx_hash,
        vote::prepare,
        vote::update_vote_tx_hash,
        vote::status,
        vote::detail,
        vote::list_self,
        timeline::get,
        task::get,
        task::send_funds,
        task::submit_milestone_report,
        task::submit_delay_report,
        task::create_meeting,
        task::submit_meeting_report,
        task::submit_acceptance_report,
        task::rectification_vote,
        task::rectification,
        meeting::get,
    ),
    components(schemas(
        record::NewRecord,
        proposal::ProposalQuery,
        SignedBody<proposal::InitiationParams>,
        SignedBody<proposal::ReceiverAddrParams>,
        reply::ReplyQuery,
        like::LikeQuery,
        SignedBody<vote::CreateVoteParams>,
        SignedBody<vote::UpdateTxParams>,
        SignedBody<vote::UpdateVoteTxParams>,
        vote::PrepareBody,
        SignedBody<task::SendFundsParams>,
        SignedBody<task::SubmitReportParams>,
        SignedBody<task::CreateMeetingParams>,
        SignedBody<task::SubmitMeetingReportParams>,
        SignedBody<task::RectificationVoteParams>,
        SignedBody<task::RectificationParams>,

        // lexicon
        lexicon::proposal::ProposalState,
        lexicon::task::TaskType,
        lexicon::task::TaskState,
        lexicon::timeline::TimelineType,
        lexicon::vote_meta::VoteMetaState,
        lexicon::vote::VoteState,
    ))
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "Authorization",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("Authorization"))),
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ToTimestamp;

impl sea_query::Iden for ToTimestamp {
    fn unquoted(&self) -> &str {
        "to_timestamp"
    }
}

pub async fn build_author(state: &AppView, repo: &str) -> Value {
    let (sql, values) = Profile::build_select()
        .and_where(Expr::col(Profile::Did).eq(repo))
        .build_sqlx(PostgresQueryBuilder);
    let row: Option<ProfileRow> = sqlx::query_as_with(&sql, values)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    let mut author = if let Some(profile) = row {
        profile.profile
    } else if let Ok(profile) = get_record(&state.pds, repo, NSID_PROFILE, "self")
        .await
        .and_then(|row| row.get("value").cloned().ok_or_eyre("NOT_FOUND"))
    {
        Profile::insert(&state.db, repo, profile.clone()).await.ok();
        profile
    } else {
        json!({
            "did": repo
        })
    };
    author["did"] = Value::String(repo.to_owned());
    if let Ok(ckb_addr) = crate::ckb::get_ckb_addr_by_did(
        &state.ckb_client,
        repo.strip_prefix("did:web5")
            .unwrap_or(repo)
            .strip_prefix("did:ckb")
            .unwrap_or(repo)
            .strip_prefix("did:plc")
            .unwrap_or(repo),
    )
    .await
    {
        author["ckb_addr"] = Value::String(ckb_addr);
    }
    author
}

pub trait SignedParam: Default + ToSchema + Serialize + Validate {
    fn timestamp(&self) -> i64;
}
#[derive(Default, ToSchema, Serialize, Deserialize, Validate)]
pub struct SignedBody<SignedParam> {
    pub params: SignedParam,
    pub did: String,
    pub signing_key_did: String,
    pub signed_bytes: String,
}

impl<T: SignedParam> SignedBody<T> {
    pub async fn verify_signature(&self, indexer_did_url: &str) -> color_eyre::Result<()> {
        // verify timestamp
        let timestamp =
            chrono::DateTime::from_timestamp_secs(self.params.timestamp()).unwrap_or_default();
        let now = chrono::Utc::now();
        let delta = (now - timestamp).abs();
        if delta > chrono::Duration::minutes(5) {
            return Err(eyre!("timestamp is invalid"));
        }

        // verify did
        let did_doc = crate::indexer_did::did_document(indexer_did_url, &self.did)
            .await
            .map_err(|e| eyre!("get did doc failed: {e}"))?;

        if self.signing_key_did
            != did_doc
                .pointer("/verificationMethods/atproto")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        {
            return Err(eyre!("signing_key_did not match"));
        }

        // verify signature
        let verifying_key: VerifyingKey = self
            .signing_key_did
            .split_once("did:key:z")
            .and_then(|(_, key)| {
                let bytes = bs58::decode(key).into_vec().ok()?;
                VerifyingKey::from_sec1_bytes(&bytes[2..]).ok()
            })
            .ok_or_eyre("invalid signing_key_did")?;
        let signature = hex::decode(self.signed_bytes.clone())
            .map(|bytes| Signature::from_slice(&bytes).map_err(|e| eyre!(e)))??;

        let unsigned_bytes = serde_ipld_dagcbor::to_vec(&self.params)?;
        verifying_key
            .verify(&unsigned_bytes, &signature)
            .map_err(|e| eyre!("verify signature failed: {e}"))
    }
}

pub async fn create_vote_tx(
    state: &AppView,
    proposal_uri: &str,
    proposal_state: ProposalState,
    creator: &str,
) -> color_eyre::Result<Value> {
    let proposal_hash = ckb_hash::blake2b_256(serde_json::to_vec(proposal_uri)?);

    let (sql, value) = VoteMeta::build_select()
        .and_where(Expr::col(VoteMeta::ProposalUri).eq(proposal_uri))
        .and_where(Expr::col(VoteMeta::ProposalState).eq(proposal_state as i32))
        .and_where(Expr::col(VoteMeta::State).eq(VoteMetaState::Waiting as i32))
        .build_sqlx(PostgresQueryBuilder);
    let vote_meta_row = if let Ok(vote_meta_row) =
        sqlx::query_as_with::<_, VoteMetaRow, _>(&sql, value)
            .fetch_one(&state.db)
            .await
    {
        vote_meta_row
    } else {
        let (sql, value) = VoteWhitelist::build_select()
            .order_by(VoteWhitelist::Created, Order::Desc)
            .limit(1)
            .build_sqlx(PostgresQueryBuilder);
        let vote_whitelist_row: VoteWhitelistRow = sqlx::query_as_with(&sql, value)
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                debug!("fetch vote_whitelist failed: {e}");
                eyre!("vote whitelist not found".to_string())
            })?;
        // TODO
        let time_range = get_vote_time_range(&state.ckb_client, 7).await?;
        let time_range = crate::ckb::test_get_vote_time_range(&state.ckb_client).await?;
        let mut vote_meta_row = VoteMetaRow {
            id: -1,
            proposal_state: proposal_state as i32,
            state: 0,
            tx_hash: None,
            proposal_uri: proposal_uri.to_string(),
            whitelist_id: vote_whitelist_row.id,
            candidates: vec![
                "Abstain".to_string(),
                "Agree".to_string(),
                "Against".to_string(),
            ],
            start_time: time_range.0 as i64,
            end_time: time_range.1 as i64,
            creator: creator.to_string(),
            results: None,
            created: chrono::Local::now(),
        };

        vote_meta_row.id = VoteMeta::insert(&state.db, &vote_meta_row).await?;
        vote_meta_row
    };

    let outputs_data = if vote_meta_row.tx_hash.is_none() {
        let vote_meta = vote::build_vote_meta(state, &vote_meta_row, &proposal_hash).await?;

        let vote_meta_bytes = vote_meta.as_bytes().to_vec();
        let vote_meta_hex = hex::encode(vote_meta_bytes);

        vec![vote_meta_hex]
    } else {
        vec![]
    };
    Ok(json!({
        "vote_meta": vote_meta_row,
        "outputsData": outputs_data
    }))
}
