//! Messages exchanged over a TLS connection: a 4-byte big-endian length
//! followed by JSON. File contents are sent as raw bytes after `FileStart`,
//! since the receiver already knows each file's size from the offer.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    pub device_name: String,
    pub profile_name: String,
    pub os: String,
    /// The port this device listens on, so the other side can connect back.
    pub port: u16,
}

/// One file or folder in a transfer. `path` is relative and `/`-separated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub path: String,
    pub size: u64,
    pub dir: bool,
    #[serde(default)]
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
    /// "home" or "drive" for the top-level places; empty for ordinary items.
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Msg {
    Hello(Hello),
    Error { message: String },
    Ping,
    Pong,
    PairRequest { commitment: String },
    PairNonce { nonce: String },
    PairReveal { nonce: String },
    PairDecision { accept: bool },
    Offer { transfer_id: String, items: Vec<Item> },
    OfferReply { accept: bool },
    FileStart { index: u32 },
    FileEnd { sha256: String },
    TransferDone { ok: bool, message: Option<String> },
    List { path: String },
    Listing {
        path: String,
        entries: Vec<RemoteEntry>,
        /// What the top of the listing is, e.g. "Home" or "Sync folder".
        #[serde(default)]
        root: String,
    },
    Pull { paths: Vec<String> },
    // Folder sync
    SyncNotify,
    GetIndex { index_id: Option<String>, seq: u64 },
    IndexUnchanged,
    IndexChunk { index_id: String, seq: u64, entries: Vec<crate::sync::index::Entry>, last: bool },
    GetFile { path: String, hash: String, offset: u64 },
    /// Followed by the file's bytes from the requested offset to the end.
    FileData { size: u64 },
}

impl Msg {
    pub fn error(message: impl Into<String>) -> Self {
        Msg::Error { message: message.into() }
    }

    /// Turn an unexpected reply into an error, keeping the peer's message if it sent one.
    pub fn unexpected(self) -> anyhow::Error {
        match self {
            Msg::Error { message } => anyhow!(message),
            other => anyhow!("unexpected reply from the other device ({:?})", std::mem::discriminant(&other)),
        }
    }
}

pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &Msg) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_MESSAGE {
        bail!("message too large");
    }
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Msg> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_MESSAGE {
        bail!("message too large");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}
