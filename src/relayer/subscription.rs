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
const INITIAL_BACKOFF_MS: u64 = 1000;
const MAX_BACKOFF_MS: u64 = 30000;

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
                self.stream.next(),
            )
            .await;

            match result {
                Ok(Some(Ok(Message::Binary(data)))) => {
                    match Frame::try_from(data.as_ref()) {
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
                            return Err(eyre!("frame decode error {e}"));
                        }
                    }
                }
                Ok(Some(Ok(Message::Ping(_)))) | Ok(Some(Ok(Message::Pong(_)))) => {
                    continue;
                }
                Ok(Some(Ok(Message::Close(frame)))) => {
                    info!("WebSocket closed: {:?}", frame);
                    return Ok(());
                }
                Ok(Some(Ok(_))) => {
                    continue;
                }
                Ok(None) => {
                    info!("WebSocket stream ended");
                    return Ok(());
                }
                Ok(Some(Err(e))) => {
                    return Err(eyre!("websocket error {e}"));
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

pub type LastSeq = Arc<AtomicI64>;

pub fn create_last_seq(initial: i64) -> LastSeq {
    Arc::new(AtomicI64::new(initial))
}

pub async fn run_with_reconnect(relayer: String, handler: impl CommitHandler + Clone) {
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    loop {
        let cursor = {
            let seq = handler.last_seq();
            if seq > 0 { Some(seq) } else { None }
        };
        match RepoSubscription::new(&relayer, cursor).await {
            Ok(mut sub) => {
                backoff_ms = INITIAL_BACKOFF_MS;
                match sub.run(handler.clone()).await {
                    Ok(_) => info!("Subscription ended, reconnecting..."),
                    Err(e) => {
                        error!("{e}");
                    }
                }
            }
            Err(e) => {
                error!("{e}");
            }
        }
        info!("Reconnecting in {}ms...", backoff_ms);
        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}
