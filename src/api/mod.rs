pub mod like;
pub mod proposal;
pub mod record;
pub mod reply;
pub mod repo;

use color_eyre::eyre::OptionExt;
use serde_json::{Value, json};
use utoipa::{
    Modify, OpenApi,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};

use crate::{
    AppView,
    atproto::{NSID_PROFILE, get_record},
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
        reply::list,
        like::list,
    ),
    components(schemas(
        record::NewRecord,
        proposal::ProposalQuery,
        reply::ReplyQuery,
        like::LikeQuery,
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
    // Get profile
    let mut author = get_record(&state.pds, repo, NSID_PROFILE, "self")
        .await
        .and_then(|row| row.get("value").cloned().ok_or_eyre("NOT_FOUND"))
        .unwrap_or(json!({
            "did": repo
        }));
    author["did"] = Value::String(repo.to_owned());
    author
}
