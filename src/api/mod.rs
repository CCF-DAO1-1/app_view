pub(crate) mod proposal;
pub(crate) mod record;
pub(crate) mod reply;
pub(crate) mod repo;
pub(crate) mod like;

use color_eyre::eyre::OptionExt;
use serde_json::{Value, json};

use crate::{
    AppView,
    atproto::{NSID_PROFILE, get_record},
};

pub(crate) struct ToTimestamp;

impl sea_query::Iden for ToTimestamp {
    fn unquoted(&self) -> &str {
        "to_timestamp"
    }
}

pub(crate) async fn build_author(state: &AppView, repo: &str) -> Value {
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
