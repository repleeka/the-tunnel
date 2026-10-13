//! Streaming files over an established, authenticated connection.
//!
//! Each file is hashed on both ends and only moved into place once the hashes
//! match, so a dropped connection never leaves a corrupt file behind. Partly
//! received files are kept in a hidden folder so a retry of the same transfer
//! continues where it stopped instead of starting over.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Notify;

use crate::paths::{self, LocalItem};
use crate::protocol::{Item, Msg, read_msg, write_msg};

const CHUNK: usize = 256 * 1024;

/// Hidden folder inside the receive folder holding partly received files.
pub const PARTIAL_DIR: &str = ".tunnel-partial";

/// Lets the user stop a transfer from the UI.
#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolves once `cancel` has been called.
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                break;
            }
            notified.await;
        }
    }

    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            bail!(Cancelled);
        }
        Ok(())
    }
}

/// Error marker for a transfer the user stopped.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_cancelled(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Cancelled>().is_some()
}

/// What the receiver already has from an earlier attempt of the same transfer.
#[derive(Debug, Clone, Default)]
pub struct ResumeInfo {
    /// Files that were fully received.
    pub skip: HashSet<usize>,
    /// Files that were partly received, and how many bytes of each.
    pub offsets: HashMap<usize, u64>,
}

impl ResumeInfo {
    pub fn already_have(&self, items: &[LocalItem]) -> u64 {
        let done: u64 = self.skip.iter().filter_map(|i| items.get(*i)).map(|l| l.item.size).sum();
        done + self.offsets.values().sum::<u64>()
    }
}

pub async fn send_files<S>(s: &mut S, items: &[LocalItem], resume: &ResumeInfo, cancel: &Cancel, mut progress: impl FnMut(u64)) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut sent = resume.already_have(items);
    let mut buf = vec![0u8; CHUNK];
    for (index, local) in items.iter().enumerate() {
        if local.item.dir || resume.skip.contains(&index) {
            continue;
        }
        let offset = resume.offsets.get(&index).copied().unwrap_or(0).min(local.item.size);
        write_msg(s, &Msg::FileStart { index: index as u32, offset }).await?;
        let mut file = tokio::fs::File::open(&local.source)
            .await
            .with_context(|| format!("can't open {}", local.source.display()))?;
        let mut hasher = Sha256::new();

        // The hash covers the whole file, so read (but don't send) what the receiver already has.
        let mut skipped = 0u64;
        while skipped < offset {
            let want = buf.len().min((offset - skipped) as usize);
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                bail!("{} changed since the last attempt", local.source.display());
            }
            hasher.update(&buf[..n]);
            skipped += n as u64;
        }

        let mut remaining = local.item.size - offset;
        while remaining > 0 {
            cancel.check()?;
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

/// Where each item of an incoming transfer goes, and which files already arrived.
/// Kept between attempts so a retry can resume.
#[derive(Debug, Clone)]
pub struct ReceivePlan {
    pub items: Vec<Item>,
    pub dest_root: PathBuf,
    pub targets: Vec<PathBuf>,
    pub done: Vec<bool>,
    pub part_dir: PathBuf,
}

impl ReceivePlan {
    pub fn new(dest_root: &Path, items: Vec<Item>, transfer_id: &str) -> Result<Self> {
        let targets = paths::plan_destinations(dest_root, &items)?;
        let id = paths::sanitize_component(transfer_id)?;
        Ok(ReceivePlan {
            done: vec![false; items.len()],
            items,
            dest_root: dest_root.to_path_buf(),
            targets,
            part_dir: dest_root.join(PARTIAL_DIR).join(id),
        })
    }

    fn part(&self, index: usize) -> PathBuf {
        self.part_dir.join(index.to_string())
    }

    /// What to tell the sender so it only sends what's missing.
    pub fn resume_info(&self) -> ResumeInfo {
        let mut info = ResumeInfo::default();
        for (i, item) in self.items.iter().enumerate() {
            if item.dir {
                continue;
            }
            if self.done[i] {
                info.skip.insert(i);
            } else if let Ok(meta) = std::fs::metadata(self.part(i))
                && meta.len() > 0 && meta.len() <= item.size {
                    info.offsets.insert(i, meta.len());
                }
        }
        info
    }

    pub fn discard(&self) {
        let _ = std::fs::remove_dir_all(&self.part_dir);
        if let Some(parent) = self.part_dir.parent() {
            let _ = std::fs::remove_dir(parent); // only succeeds if empty
        }
    }
}

/// Receive the files described by the plan. Returns the top-level paths created.
/// On failure, partly received files are kept for a later retry.
pub async fn receive_files<S>(s: &mut S, plan: &mut ReceivePlan, cancel: &Cancel, progress: impl FnMut(u64)) -> Result<Vec<PathBuf>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = receive_inner(s, plan, cancel, progress).await;
    if let Err(e) = &result {
        let _ = write_msg(s, &Msg::TransferDone { ok: false, message: Some(e.to_string()) }).await;
    }
    result
}

async fn receive_inner<S>(s: &mut S, plan: &mut ReceivePlan, cancel: &Cancel, mut progress: impl FnMut(u64)) -> Result<Vec<PathBuf>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::fs::create_dir_all(&plan.dest_root)
        .await
        .with_context(|| format!("can't create {}", plan.dest_root.display()))?;
    tokio::fs::create_dir_all(&plan.part_dir).await?;
    for (item, target) in plan.items.iter().zip(&plan.targets) {
        if item.dir {
            tokio::fs::create_dir_all(target).await.with_context(|| format!("can't create {}", target.display()))?;
        }
    }

    let resume = plan.resume_info();
    let mut total: u64 = plan.items.iter().enumerate().filter(|(i, _)| resume.skip.contains(i)).map(|(_, it)| it.size).sum::<u64>()
        + resume.offsets.values().sum::<u64>();
    let mut buf = vec![0u8; CHUNK];
    loop {
        cancel.check()?;
        match read_msg(s).await? {
            Msg::FileStart { index, offset } => {
                let index = index as usize;
                let item = plan.items.get(index).context("file index out of range")?.clone();
                if item.dir || plan.done[index] {
                    bail!("unexpected file in transfer");
                }
                let part = plan.part(index);
                let have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
                if offset != 0 && offset != have {
                    bail!("resume position mismatch for {}", item.path);
                }

                let mut hasher = Sha256::new();
                let mut file = if offset == 0 {
                    tokio::fs::File::create(&part).await.with_context(|| format!("can't write {}", part.display()))?
                } else {
                    let mut f = tokio::fs::OpenOptions::new().read(true).write(true).open(&part).await?;
                    let mut left = offset;
                    while left > 0 {
                        let want = buf.len().min(left as usize);
                        let n = f.read(&mut buf[..want]).await?;
                        if n == 0 {
                            bail!("partial file for {} is shorter than expected", item.path);
                        }
                        hasher.update(&buf[..n]);
                        left -= n as u64;
                    }
                    f.seek(std::io::SeekFrom::Start(offset)).await?;
                    f
                };

                let mut remaining = item.size - offset;
                while remaining > 0 {
                    cancel.check()?;
                    let want = buf.len().min(remaining as usize);
                    let n = s.read(&mut buf[..want]).await?;
                    if n == 0 {
                        file.flush().await?;
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
                    Msg::FileEnd { .. } => {
                        let _ = tokio::fs::remove_file(&part).await;
                        bail!("{} arrived corrupted", item.path);
                    }
                    other => return Err(other.unexpected()),
                }
                if let Some(secs) = item.modified {
                    set_modified(&part, secs);
                }
                // On a resumed transfer something may have appeared here since; never overwrite it.
                if plan.targets[index].exists() {
                    let t = &plan.targets[index];
                    if let (Some(dir), Some(name)) = (t.parent(), t.file_name().and_then(|n| n.to_str())) {
                        plan.targets[index] = dir.join(paths::free_name(dir, name));
                    }
                }
                let target = &plan.targets[index];
                if let Some(parent) = target.parent() {
                    tokio::fs::create_dir_all(parent).await.with_context(|| format!("can't create {}", parent.display()))?;
                }
                tokio::fs::rename(&part, target).await.with_context(|| format!("can't save {}", target.display()))?;
                plan.done[index] = true;
            }
            Msg::TransferDone { ok: true, .. } => break,
            Msg::TransferDone { message, .. } => bail!(message.unwrap_or_else(|| "the sender stopped the transfer".into())),
            other => return Err(other.unexpected()),
        }
    }
    if plan.items.iter().zip(&plan.done).any(|(i, d)| !i.dir && !d) {
        bail!("some files were missing from the transfer");
    }
    write_msg(s, &Msg::TransferDone { ok: true, message: None }).await?;
    plan.discard();

    let mut tops: Vec<PathBuf> = plan
        .targets
        .iter()
        .filter_map(|t| t.strip_prefix(&plan.dest_root).ok()?.components().next().map(|c| plan.dest_root.join(c)))
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
