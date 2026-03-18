#[macro_use]
extern crate tracing as logger;

use std::time::Duration;

use ckb_sdk::CkbRpcAsyncClient;
use clap::Parser;
use color_eyre::{Result, eyre::eyre};
use common_x::restful::axum::routing::get;
use common_x::restful::axum::{Router, routing::post};
use dao::api::ApiDoc;
use dao::lexicon::administrator::Administrator;
use dao::lexicon::meeting::Meeting;
use dao::lexicon::profile::Profile;
use dao::lexicon::task::Task;
use dao::lexicon::timeline::Timeline;
use dao::lexicon::vote::Vote;
use dao::lexicon::vote_meta::VoteMeta;
use dao::lexicon::voter_list::VoterList;
use dao::relayer::subscription::RepoSubscription;
use dao::{AppView, api, scheduler};
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
    ckb_url: String,
    #[clap(short, long)]
    db_url: String,
    #[clap(short, long)]
    indexer_bind_url: String,
    #[clap(short, long)]
    indexer_did_url: String,
    #[clap(short, long)]
    indexer_vote_url: String,
    #[clap(short, long)]
    relayer: String,
    #[clap(short, long)]
    pds: String,
    #[clap(short, long, default_value = "false")]
    apidoc: bool,
    #[clap(short, long, default_value = "Testnet")]
    ckb_net: String,
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
    Profile::init(&db).await?;
    VoterList::init(&db).await?;
    Administrator::init(&db).await?;
    VoteMeta::init(&db).await?;
    Vote::init(&db).await?;
    Timeline::init(&db).await?;
    Task::init(&db).await?;
    Meeting::init(&db).await?;

    let ckb_client = CkbRpcAsyncClient::new(&args.ckb_url);

    let app = AppView {
        db,
        pds: args.pds.clone(),
        indexer_bind_url: args.indexer_bind_url.clone(),
        indexer_did_url: args.indexer_did_url.clone(),
        indexer_vote_url: args.indexer_vote_url.clone(),
        ckb_client,
        ckb_net: match args.ckb_net.to_lowercase().as_str() {
            "mainnet" => ckb_sdk::NetworkType::Mainnet,
            "testnet" => ckb_sdk::NetworkType::Testnet,
            "dev" => ckb_sdk::NetworkType::Dev,
            _ => {
                error!("Invalid ckb_net, default to Testnet");
                ckb_sdk::NetworkType::Testnet
            }
        },
    };

    // reconnect
    let app_ = app.clone();
    let relayer = args.relayer.clone();
    tokio::spawn(async move {
        loop {
            match RepoSubscription::new(&relayer).await {
                Ok(mut sub) => match sub.run(app_.clone()).await {
                    Ok(_) => info!("Subscription ended successfully."),
                    Err(e) => error!("{e}"),
                },
                Err(e) => error!("{e}"),
            }
            info!("Reconnecting in 1 seconds...");
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });

    scheduler::init_task_scheduler(&app).await?;

    let router = if args.apidoc {
        Router::new()
            // openapi docs
            .merge(Scalar::with_url("/apidoc", ApiDoc::openapi()))
    } else {
        Router::new()
    };
    let router = router
        // api routes
        .route("/api/repo/profile", get(api::repo::profile))
        .route("/api/proposal/list", post(api::proposal::list))
        .route("/api/proposal/detail", get(api::proposal::detail))
        .route(
            "/api/proposal/initiation_vote",
            post(api::proposal::initiation_vote),
        )
        .route(
            "/api/proposal/update_receiver_addr",
            post(api::proposal::update_receiver_addr),
        )
        .route(
            "/api/proposal/receiver_addr",
            get(api::proposal::receiver_addr),
        )
        .route("/api/proposal/status", get(api::proposal::status))
        .route("/api/proposal/list_self", get(api::proposal::list_self))
        .route("/api/proposal/replied", get(api::proposal::replied))
        .route("/api/reply/list", post(api::reply::list))
        .route("/api/like/list", post(api::like::list))
        .route("/api/vote/bind_list", get(api::vote::bind_list))
        .route("/api/vote/weight", get(api::vote::weight))
        .route("/api/vote/voter_list", get(api::vote::voter_list))
        .route("/api/vote/proof", get(api::vote::proof))
        .route(
            "/api/vote/update_meta_tx_hash",
            post(api::vote::update_meta_tx_hash),
        )
        .route("/api/vote/prepare", post(api::vote::prepare))
        .route(
            "/api/vote/update_vote_tx_hash",
            post(api::vote::update_vote_tx_hash),
        )
        .route("/api/vote/status", post(api::vote::status))
        .route("/api/vote/detail", get(api::vote::detail))
        .route("/api/vote/list_self", get(api::vote::list_self))
        .route("/api/timeline", get(api::timeline::get))
        .route("/api/task", get(api::task::get))
        .route("/api/task/send_funds", post(api::task::send_funds))
        .route(
            "/api/task/submit_milestone_report",
            post(api::task::submit_milestone_report),
        )
        .route(
            "/api/task/submit_delay_report",
            post(api::task::submit_delay_report),
        )
        .route(
            "/api/task/submit_meeting_report",
            post(api::task::submit_meeting_report),
        )
        .route("/api/task/create_meeting", post(api::task::create_meeting))
        .route(
            "/api/task/submit_acceptance_report",
            post(api::task::submit_acceptance_report),
        )
        .route(
            "/api/task/rectification_vote",
            post(api::task::rectification_vote),
        )
        .route("/api/task/rectification", post(api::task::rectification))
        .route("/api/meeting", get(api::meeting::get))
        .layer((TimeoutLayer::with_status_code(
            reqwest::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ),))
        .layer(CorsLayer::permissive())
        .with_state(app);
    common_x::restful::http_serve(args.port, router)
        .await
        .map_err(|e| eyre!("{e}"))
}
