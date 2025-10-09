#[macro_use]
extern crate tracing as logger;

use std::time::Duration;

use clap::Parser;
use color_eyre::{Result, eyre::eyre};
use common_x::restful::axum::routing::get;
use common_x::restful::axum::{Router, routing::post};
use dao::api::ApiDoc;
use dao::{AppView, api};
use sqlx::postgres::PgPoolOptions;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;

use dao::lexicon::like::Like;
use dao::lexicon::proposal::Proposal;
use dao::lexicon::reply::Reply;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

#[derive(Parser, Debug, Clone)]
#[command(author, version)]
pub struct Args {
    #[clap(short, long, default_value = "info")]
    log_filter: String,
    #[clap(long, default_value = "8080")]
    port: u16,
    #[clap(short, long)]
    db_url: String,
    #[clap(short, long)]
    pds: String,
    #[clap(short, long, default_value = "")]
    whitelist: String,
    #[clap(short, long, default_value = "false")]
    apidoc: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    common_x::log::init_log_filter(&args.log_filter);
    info!("args: {:?}", args);
    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect(&args.db_url)
        .await?;

    // initialize the database
    Proposal::init(&db).await?;
    Reply::init(&db).await?;
    Like::init(&db).await?;

    let dao = AppView {
        db,
        pds: args.pds.clone(),
        whitelist: args
            .whitelist
            .split(',')
            .filter_map(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_owned())
                }
            })
            .collect(),
    };

    let router = if args.apidoc {
        Router::new()
            // openapi docs
            .merge(Scalar::with_url("/apidoc", ApiDoc::openapi()))
    } else {
        Router::new()
    };
    let router = router
        // api routes
        .route("/api/record/create", post(api::record::create))
        .route("/api/record/update", post(api::record::update))
        .route("/api/repo/profile", get(api::repo::profile))
        .route("/api/proposal/list", post(api::proposal::list))
        .route("/api/proposal/detail", get(api::proposal::detail))
        .route(
            "/api/proposal/update_state",
            post(api::proposal::update_state),
        )
        .route("/api/reply/list", post(api::reply::list))
        .route("/api/like/list", post(api::like::list))
        .layer((TimeoutLayer::new(Duration::from_secs(10)),))
        .layer(CorsLayer::permissive())
        .with_state(dao);
    common_x::restful::http_serve(args.port, router)
        .await
        .map_err(|e| eyre!("{e}"))
}
