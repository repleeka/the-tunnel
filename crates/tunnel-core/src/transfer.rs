//! Streaming files over an established, authenticated connection.
//! Each file is hashed on both ends and only moved into place once the
//! hashes match, so a dropped connection never leaves a corrupt file behind.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::paths::{self, LocalItem, PARTIAL_SUFFIX};
use crate::protocol::{Item, Msg, read_msg, write_msg};

const CHUNK: usize = 256 * 1024;

pub async fn send_files<S>(s: &mut S, items: &[LocalItem], mut progress: impl FnMut(u64)) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut sent = 0u64;
    let mut buf = vec![0u8; CHUNK];
    for (index, local) in items.iter().enumerate() {
        if local.item.dir {
            continue;
        }
        write_msg(s, &Msg::FileStart { index: index as u32 }).await?;
        let mut file = tokio::fs::File::open(&local.source)
            .await
            .with_context(|| format!("can't open {}", local.source.display()))?;
        let mut hasher = Sha256::new();
        let mut remaining = local.item.size;
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                bail!("{} changed while it was being sent", local.source.display());
            }
            hasher.update(&buf[..n]);
            s.write_all(&buf[..n]).await?;
            remaining -= n as u64;
            sent += n as u64;
            progress(sent);
        }
        write_msg(s, &Msg::FileEnd { sha256: hex::encode(hasher.finalize()) }).await?;
    }
    write_msg(s, &Msg::TransferDone { ok: true, message: None }).await?;
    match read_msg(s).await? {
        Msg::TransferDone { ok: true, .. } => Ok(()),
        Msg::TransferDone { message, .. } => bail!(message.unwrap_or_else(|| "the other device couldn't save the files".into())),
        other => Err(other.unexpected()),
    }
}

/// Receive the files described by `items` into `dest_root`.
/// Returns the top-level paths that were created.
pub async fn receive_files<S>(s: &mut S, items: &[Item], dest_root: &Path, progress: impl FnMut(u64)) -> Result<Vec<PathBuf>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut partial: Option<PathBuf> = None;
    let result = receive_inner(s, items, dest_root, progress, &mut partial).await;
    if let Err(e) = &result {
        if let Some(p) = partial {
            let _ = tokio::fs::remove_file(p).await;
        }
        let _ = write_msg(s, &Msg::TransferDone { ok: false, message: Some(e.to_string()) }).await;
    }
    result
}

async fn receive_inner<S>(
    s: &mut S,
    items: &[Item],
    dest_root: &Path,
    mut progress: impl FnMut(u64),
    partial: &mut Option<PathBuf>,
) -> Result<Vec<PathBuf>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let targets = paths::plan_destinations(dest_root, items)?;
    tokio::fs::create_dir_all(dest_root).await.with_context(|| format!("can't create {}", dest_root.display()))?;
    for (item, target) in items.iter().zip(&targets) {
        if item.dir {
            tokio::fs::create_dir_all(target).await.with_context(|| format!("can't create {}", target.display()))?;
        }
    }

    let mut received = vec![false; items.len()];
    let mut total = 0u64;
    let mut buf = vec![0u8; CHUNK];
    loop {
        match read_msg(s).await? {
            Msg::FileStart { index } => {
                let index = index as usize;
                let item = items.get(index).context("file index out of range")?;
                if item.dir || received[index] {
                    bail!("unexpected file in transfer");
                }
                let target = &targets[index];
                if let Some(parent) = target.parent() {
                    tokio::fs::create_dir_all(parent).await.with_context(|| format!("can't create {}", parent.display()))?;
                }
                let tmp = PathBuf::from(format!("{}{PARTIAL_SUFFIX}", target.display()));
                *partial = Some(tmp.clone());
                let mut file = tokio::fs::File::create(&tmp).await.with_context(|| format!("can't write {}", tmp.display()))?;
                let mut hasher = Sha256::new();
                let mut remaining = item.size;
                while remaining > 0 {
                    let want = buf.len().min(remaining as usize);
                    let n = s.read(&mut buf[..want]).await?;
                    if n == 0 {
                        bail!("connection closed mid-file");
                    }
                    hasher.update(&buf[..n]);
                    file.write_all(&buf[..n]).await?;
                    remaining -= n as u64;
                    total += n as u64;
                    progress(total);
                }
                file.flush().await?;
                file.sync_all().await?;
                drop(file);
                match read_msg(s).await? {
                    Msg::FileEnd { sha256 } if sha256 == hex::encode(hasher.finalize()) => {}
                    Msg::FileEnd { .. } => bail!("{} arrived corrupted", item.path),
                    other => return Err(other.unexpected()),
                }
                if let Some(secs) = item.modified {
                    set_modified(&tmp, secs);
                }
                tokio::fs::rename(&tmp, target).await.with_context(|| format!("can't save {}", target.display()))?;
                *partial = None;
                received[index] = true;
            }
            Msg::TransferDone { ok: true, .. } => break,
            Msg::TransferDone { message, .. } => bail!(message.unwrap_or_else(|| "the sender stopped the transfer".into())),
            other => return Err(other.unexpected()),
        }
    }
    if items.iter().zip(&received).any(|(i, r)| !i.dir && !r) {
        bail!("some files were missing from the transfer");
    }
    write_msg(s, &Msg::TransferDone { ok: true, message: None }).await?;

    let mut tops: Vec<PathBuf> = targets
        .iter()
        .filter_map(|t| t.strip_prefix(dest_root).ok()?.components().next().map(|c| dest_root.join(c)))
        .collect();
    tops.dedup();
    Ok(tops)
}

fn set_modified(path: &Path, secs: i64) {
    if secs < 0 {
        return;
    }
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs(secs as u64));
    }
}
