pub mod like;
pub mod proposal;
pub mod record;
pub mod reply;
pub mod repo;
pub mod vote;

use color_eyre::eyre::OptionExt;
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use serde_json::{Value, json};
use utoipa::{
    Modify, OpenApi,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};

use crate::{
    AppView,
    atproto::{NSID_PROFILE, get_record},
    lexicon::profile::{Profile, ProfileRow},
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
        proposal::update_state,
        reply::list,
        like::list,
        vote::weight,
        vote::whitelist,
        vote::proof,
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
    let (sql, values) = Profile::build_select()
        .and_where(Expr::col(Profile::Did).eq(repo))
        .build_sqlx(PostgresQueryBuilder);
    debug!("build_author exec sql: {sql}");
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
    author
}
