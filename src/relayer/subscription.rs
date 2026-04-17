use atrium_api::com::atproto::sync::subscribe_repos::Commit;
use color_eyre::{Result, eyre::eyre};
use futures::StreamExt;
use std::future::Future;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use crate::relayer::stream::Frame;

const DEFAULT_HEARTBEAT_TIMEOUT_SECS: u64 = 60;

#[trait_variant::make(HttpService: Send)]
pub trait Subscription {
    async fn next(&mut self) -> Option<Result<Frame>>;
}

pub trait CommitHandler {
    fn handle_commit(&self, commit: &Commit, seq: i64) -> impl Future<Output = Result<()>>;
    fn last_seq(&self) -> i64;
}

pub struct RepoSubscription {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    heartbeat_timeout_secs: u64,
}

impl RepoSubscription {
    pub async fn new(relayer: &str, cursor: Option<i64>) -> Result<Self> {
        let url = if let Some(cursor) = cursor {
            format!("{}?cursor={}", relayer, cursor)
        } else {
            relayer.to_string()
        };
        let (stream, _) = connect_async(&url).await?;
        info!("Connected to relayer at {} (cursor: {:?})", url, cursor);
        Ok(RepoSubscription {
            stream,
            heartbeat_timeout_secs: DEFAULT_HEARTBEAT_TIMEOUT_SECS,
        })
    }

    pub fn with_heartbeat_timeout(mut self, secs: u64) -> Self {
        self.heartbeat_timeout_secs = secs;
        self
    }

    pub async fn run(&mut self, handler: impl CommitHandler) -> Result<()> {
        loop {
            let result = timeout(
                Duration::from_secs(self.heartbeat_timeout_secs),
                self.next(),
            )
            .await;

            match result {
                Ok(Some(message)) => {
                    match message {
                        Ok(Frame::Message(Some(t), message)) => {
                            if t.as_str() == "#commit" {
                                let commit: Commit =
                                    serde_ipld_dagcbor::from_reader(message.body.as_slice())?;
                                let seq = commit.seq;
                                if let Err(err) = handler.handle_commit(&commit, seq).await {
                                    error!("FAILED: {err:?}");
                                }
                            }
                        }
                        Ok(Frame::Message(None, _)) | Ok(Frame::Error(_)) => (),
                        Err(e) => {
                            return Err(eyre!("error {e}"));
                        }
                    }
                }
                Ok(None) => {
                    info!("WebSocket stream ended");
                    return Ok(());
                }
                Err(_) => {
                    return Err(eyre!(
                        "Heartbeat timeout: no message received for {} seconds",
                        self.heartbeat_timeout_secs
                    ));
                }
            }
        }
    }
}

impl Subscription for RepoSubscription {
    async fn next(&mut self) -> Option<Result<Frame>> {
        match self.stream.next().await {
            Some(Ok(Message::Binary(data))) => Some(Frame::try_from(data.iter().as_slice())),
            Some(Ok(_)) | None => None,
            Some(Err(e)) => Some(Err(eyre!(e))),
        }
    }
}

pub type LastSeq = Arc<AtomicI64>;

pub fn create_last_seq(initial: i64) -> LastSeq {
    Arc::new(AtomicI64::new(initial))
}
