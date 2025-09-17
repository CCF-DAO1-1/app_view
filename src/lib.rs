pub mod api;
pub mod atproto;
pub mod error;
pub mod lexicon;
pub mod tid;

#[macro_use]
extern crate tracing as logger;

#[derive(Debug, Clone)]
pub struct AppView {
    pub db: sqlx::Pool<sqlx::Postgres>,
    pub pds: String,
    pub whitelist: Vec<String>,
}
