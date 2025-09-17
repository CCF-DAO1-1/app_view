use clap::Parser;
use color_eyre::Result;
use dao::atproto::{Write, create_session, write_to_pds};
use tracing::info;

#[derive(Parser, Debug, Clone)]
#[command(author, version)]
pub struct Args {
    #[clap(short, long)]
    pds: String,
    #[clap(short, long)]
    repo: String,
    #[clap(short, long)]
    ckb_addr: String,
    #[clap(short, long)]
    signing_key: String,
    #[clap(short, long, default_value = "")]
    rkey: String,
    #[clap(short, long)]
    collection: String,
    #[clap(short, long)]
    value: String,
    #[clap(short, long, default_value = "false")]
    is_update: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    common_x::log::init_log_filter("info");

    let rkey = if args.rkey.is_empty() {
        dao::tid::Ticker::new().next(None).to_string()
    } else {
        args.rkey
    };

    let auth = create_session(&args.pds, &args.repo, &args.signing_key, &args.ckb_addr).await?;

    let result = write_to_pds(
        &args.pds,
        &auth,
        &args.repo,
        &Write {
            value: serde_json::from_str(&args.value)?,
            collection: args.collection,
            rkey,
        },
        args.is_update,
        &args.signing_key,
        &args.ckb_addr,
    )
    .await?;
    info!("write result: {}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
