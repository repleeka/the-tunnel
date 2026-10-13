//! Keeping The Tunnel folder identical on paired devices.
//!
//! Each device watches its own folder and keeps an index of it. A "round" with
//! a paired device fetches that device's index, plans what's missing here (see
//! `plan`), and pulls it. Each device only ever changes its own folder, so the
//! two directions are independent. Changes are announced with a small notify
//! message so the other side starts its round promptly.

pub mod index;
pub mod plan;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::engine::{Engine, Event};
use crate::protocol::{Msg, read_msg, write_msg};
use index::{Entry, Index, META_DIR, bump, merge, mtime_ms, wire_to_path};
use plan::Action;

const TICK: Duration = Duration::from_secs(2);
const ROUND_INTERVAL: Duration = Duration::from_secs(20);
const DEBOUNCE: Duration = Duration::from_millis(800);
const RESCAN_INTERVAL: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const INDEX_CHUNK: usize = 2000;
const MAX_RECENT: usize = 40;
const STATUS_INTERVAL: Duration = Duration::from_millis(300);

// ---------- Status shown in the UI ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerSyncState {
    Waiting,
    Syncing,
    Idle,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerSync {
    pub device_id: String,
    pub state: PeerSyncState,
    pub files_left: usize,
    pub bytes_left: u64,
    pub last_synced: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDeletion {
    pub device_id: String,
    pub device_name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Updated,
    Renamed,
    Deleted,
    Kept,
    Conflict,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChange {
    pub at: u64,
    pub device_name: String,
    pub path: String,
    pub from: Option<String>,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub folder: String,
    pub scanning: bool,
    pub peers: Vec<PeerSync>,
    pub pending_deletions: Vec<PendingDeletion>,
    pub issues: Vec<String>,
    pub recent: Vec<SyncChange>,
}

// ---------- Runtime state ----------

struct RemoteIndex {
    index_id: String,
    seq: u64,
    entries: BTreeMap<String, Entry>,
}

struct Peer {
    remote: Option<RemoteIndex>,
    notified: bool,
    running: bool,
    last_round: Option<Instant>,
    status: PeerSync,
    pending: Vec<(PendingDeletion, Entry)>,
}

impl Peer {
    fn new(device_id: &str) -> Self {
        Peer {
            remote: None,
            notified: false,
            running: false,
            last_round: None,
            status: PeerSync { device_id: device_id.into(), state: PeerSyncState::Waiting, files_left: 0, bytes_left: 0, last_synced: None, error: None },
            pending: Vec::new(),
        }
    }
}

pub(crate) struct Folder {
    root: PathBuf,
    index_path: PathBuf,
    index: Mutex<Index>,
    /// Held while scanning and while placing each file, so the two never interleave.
    guard: tokio::sync::Mutex<()>,
    dirty: Arc<AtomicBool>,
    last_event: Arc<Mutex<Instant>>,
    last_scan: Mutex<Option<Instant>>,
    scanning: AtomicBool,
    issues: Mutex<Vec<String>>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Folder {
    fn open(root: &Path, data_dir: &Path) -> Result<Folder> {
        std::fs::create_dir_all(root).with_context(|| format!("can't create {}", root.display()))?;
        let key = hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()));
        let index_path = data_dir.join(format!("sync-index-{}.json", &key[..12]));
        let dirty = Arc::new(AtomicBool::new(true));
        let last_event = Arc::new(Mutex::new(Instant::now()));

        let meta = root.join(META_DIR);
        let (d, l) = (dirty.clone(), last_event.clone());
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if event.paths.iter().all(|p| p.starts_with(&meta)) {
                    return;
                }
                d.store(true, Ordering::SeqCst);
                *l.lock().unwrap() = Instant::now();
            }
        })
        .and_then(|mut w| w.watch(root, RecursiveMode::Recursive).map(|_| w));
        let watcher = match watcher {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("folder watching unavailable, falling back to periodic scans: {e}");
                None
            }
        };

        Ok(Folder {
            root: root.to_path_buf(),
            index: Mutex::new(Index::load(&index_path)),
            index_path,
            guard: tokio::sync::Mutex::new(()),
            dirty,
            last_event,
            last_scan: Mutex::new(None),
            scanning: AtomicBool::new(false),
            issues: Mutex::new(Vec::new()),
            _watcher: watcher,
        })
    }

    fn needs_scan(&self) -> bool {
        match *self.last_scan.lock().unwrap() {
            None => true,
            Some(t) if t.elapsed() > RESCAN_INTERVAL => true,
            Some(_) => self.dirty.load(Ordering::SeqCst) && self.last_event.lock().unwrap().elapsed() > DEBOUNCE,
        }
    }

    fn save(&self) {
        let snapshot = self.index.lock().unwrap().clone();
        if let Err(e) = crate::store::save_json(&self.index_path, &snapshot) {
            tracing::warn!("couldn't save sync index: {e:#}");
        }
    }

    fn tmp_dir(&self) -> PathBuf {
        self.root.join(META_DIR).join("tmp")
    }
}

#[derive(Default)]
pub(crate) struct SyncState {
    folder: Mutex<Option<Arc<Folder>>>,
    peers: Mutex<HashMap<String, Peer>>,
    wake: Notify,
    recent: Mutex<VecDeque<SyncChange>>,
}

impl SyncState {
    pub(crate) fn wake_now(&self) {
        self.wake.notify_one();
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------- Engine integration ----------

impl Engine {
    fn me_short(&self) -> String {
        self.device_id()[..16].to_string()
    }

    fn with_peer<R>(&self, device_id: &str, f: impl FnOnce(&mut Peer) -> R) -> R {
        let mut peers = self.sync_state().peers.lock().unwrap();
        f(peers.entry(device_id.to_string()).or_insert_with(|| Peer::new(device_id)))
    }

    /// The folder for the current settings, reopened if the user picked a different one.
    fn sync_folder(&self) -> Result<Arc<Folder>> {
        let root = self.settings().sync_dir;
        let mut slot = self.sync_state().folder.lock().unwrap();
        if let Some(f) = slot.as_ref().filter(|f| f.root == root) {
            return Ok(f.clone());
        }
        let folder = Arc::new(Folder::open(&root, self.data_dir())?);
        *slot = Some(folder.clone());
        drop(slot);
        // Anything learned about the old folder no longer applies.
        for peer in self.sync_state().peers.lock().unwrap().values_mut() {
            peer.remote = None;
            peer.pending.clear();
            peer.last_round = None;
        }
        Ok(folder)
    }

    pub(crate) fn start_sync(&self) {
        let engine = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(TICK) => {}
                    _ = engine.sync_state().wake.notified() => {}
                }
                if let Err(e) = engine.sync_tick().await {
                    tracing::warn!("sync: {e:#}");
                }
            }
        });
    }

    async fn sync_tick(&self) -> Result<()> {
        let settings = self.settings();
        if !settings.onboarded || !settings.sync_enabled {
            return Ok(());
        }
        let folder = self.sync_folder()?;
        if folder.needs_scan() {
            self.scan(&folder).await?;
        }
        for device in self.devices().into_iter().filter(|d| d.paired && d.online) {
            let due = self.with_peer(&device.id, |p| {
                !p.running && (p.notified || p.last_round.is_none_or(|t| t.elapsed() > ROUND_INTERVAL))
            });
            if due {
                let (engine, folder) = (self.clone(), folder.clone());
                tokio::spawn(async move { engine.round(&folder, &device.id).await });
            }
        }
        Ok(())
    }

    /// Scan now and sync with every paired device, waiting for the result.
    /// Used by tests and by the "Sync now" button.
    pub async fn sync_now(&self) -> Result<()> {
        if !self.settings().sync_enabled {
            bail!("Sync is turned off.");
        }
        let folder = self.sync_folder()?;
        self.scan(&folder).await?;
        let paired: Vec<_> = self.devices().into_iter().filter(|d| d.paired).collect();
        for device in paired {
            self.round(&folder, &device.id).await;
        }
        Ok(())
    }

    async fn scan(&self, folder: &Arc<Folder>) -> Result<()> {
        folder.dirty.store(false, Ordering::SeqCst);
        folder.scanning.store(true, Ordering::SeqCst);
        let me = self.me_short();
        let f = folder.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let _guard = f.guard.blocking_lock();
            index::scan(&f.root, &f.index, &me)
        })
        .await?;
        folder.scanning.store(false, Ordering::SeqCst);
        *folder.last_scan.lock().unwrap() = Some(Instant::now());
        let outcome = outcome?;
        *folder.issues.lock().unwrap() = outcome.issues;
        if outcome.changed || outcome.touched {
            folder.save();
        }
        if outcome.changed {
            self.emit(Event::SyncChanged);
            self.notify_peers();
        }
        Ok(())
    }

    /// Tell online paired devices that something changed here.
    fn notify_peers(&self) {
        for device in self.devices().into_iter().filter(|d| d.paired && d.online) {
            let engine = self.clone();
            tokio::spawn(async move {
                if let Ok((mut s, _)) = engine.connect(&device.id).await
                    && write_msg(&mut s, &Msg::SyncNotify).await.is_ok() {
                        let _ = timeout(Duration::from_secs(5), read_msg(&mut s)).await;
                    }
            });
        }
    }

    fn record_change(&self, device_name: &str, path: &str, from: Option<String>, kind: ChangeKind) {
        let mut recent = self.sync_state().recent.lock().unwrap();
        recent.push_front(SyncChange { at: now_secs(), device_name: device_name.into(), path: path.into(), from, kind });
        recent.truncate(MAX_RECENT);
    }

    pub(crate) fn sync_status(&self) -> SyncStatus {
        let settings = self.settings();
        let state = self.sync_state();
        let folder = state.folder.lock().unwrap().clone();
        let peers = state.peers.lock().unwrap();
        let paired: Vec<_> = self.devices().into_iter().filter(|d| d.paired).collect();
        SyncStatus {
            enabled: settings.sync_enabled,
            folder: settings.sync_dir.display().to_string(),
            scanning: folder.as_ref().is_some_and(|f| f.scanning.load(Ordering::SeqCst)),
            peers: paired
                .iter()
                .map(|d| {
                    let mut s = peers.get(&d.id).map(|p| p.status.clone()).unwrap_or_else(|| Peer::new(&d.id).status);
                    if !d.online && s.state != PeerSyncState::Error {
                        s.state = PeerSyncState::Waiting;
                    }
                    s
                })
                .collect(),
            pending_deletions: peers.values().flat_map(|p| p.pending.iter().map(|(d, _)| d.clone())).collect(),
            issues: folder.map(|f| f.issues.lock().unwrap().clone()).unwrap_or_default(),
            recent: state.recent.lock().unwrap().iter().cloned().collect(),
        }
    }

    // ---------- One sync round with one device ----------

    async fn round(&self, folder: &Arc<Folder>, device_id: &str) {
        let started = self.with_peer(device_id, |p| {
            if p.running {
                return false;
            }
            p.running = true;
            p.notified = false;
            p.status.state = PeerSyncState::Syncing;
            true
        });
        if !started {
            return;
        }
        self.emit(Event::SyncChanged);
        let result = self.round_inner(folder, device_id).await;
        folder.save();
        self.with_peer(device_id, |p| {
            p.running = false;
            p.last_round = Some(Instant::now());
            p.status.files_left = 0;
            p.status.bytes_left = 0;
            match result {
                Ok(()) => {
                    p.status.state = PeerSyncState::Idle;
                    p.status.error = None;
                    p.status.last_synced = Some(now_secs());
                }
                Err(e) => {
                    p.status.state = PeerSyncState::Error;
                    p.status.error = Some(format!("{e:#}"));
                }
            }
        });
        self.emit(Event::SyncChanged);
    }

    async fn round_inner(&self, folder: &Arc<Folder>, device_id: &str) -> Result<()> {
        let (mut s, hello) = self.connect(device_id).await?;
        let peer_name = hello.device_name;

        // 1. Get the other device's index (only if it changed since last time).
        let known = self.with_peer(device_id, |p| p.remote.as_ref().map(|r| (r.index_id.clone(), r.seq)));
        write_msg(&mut s, &Msg::GetIndex { index_id: known.as_ref().map(|k| k.0.clone()), seq: known.map_or(0, |k| k.1) }).await?;
        let mut fresh: Option<RemoteIndex> = None;
        loop {
            match timeout(REQUEST_TIMEOUT, read_msg(&mut s)).await.context("timed out")?? {
                Msg::IndexUnchanged => break,
                Msg::IndexChunk { index_id, seq, entries, last } => {
                    let r = fresh.get_or_insert_with(|| RemoteIndex { index_id, seq, entries: BTreeMap::new() });
                    for e in entries {
                        r.entries.insert(e.path.clone(), e);
                    }
                    if last {
                        break;
                    }
                }
                other => return Err(other.unexpected()),
            }
        }
        let remote = self.with_peer(device_id, |p| {
            if let Some(f) = fresh {
                p.remote = Some(f);
            }
            p.remote.as_ref().map(|r| r.entries.clone()).unwrap_or_default()
        });

        // 2. Decide what to do.
        let local = folder.index.lock().unwrap().entries.clone();
        let actions = plan::plan(&local, &remote);
        let transfers: Vec<&Entry> = actions
            .iter()
            .filter_map(|a| match a {
                Action::Download { remote, .. } => Some(remote),
                _ => None,
            })
            .collect();
        let (mut files_left, mut bytes_left) = (transfers.len(), transfers.iter().map(|e| e.size).sum::<u64>());
        self.with_peer(device_id, |p| {
            p.status.files_left = files_left;
            p.status.bytes_left = bytes_left;
        });

        // 3. Do it. A failure on one file doesn't stop the rest; the next round retries.
        let mut pending = Vec::new();
        let mut issues = Vec::new();
        let mut changed = false;
        let mut last_status = Instant::now();
        for action in actions {
            let result: Result<()> = match action {
                Action::Adopt { remote } => {
                    self.adopt(folder, &remote);
                    Ok(())
                }
                Action::Mkdir { remote } => self.make_dir(folder, &remote).await,
                Action::RemoveEmptyDir { remote } => self.remove_empty_dir(folder, &remote).await,
                Action::ReviewDeletion { remote } => {
                    pending.push((
                        PendingDeletion { device_id: device_id.into(), device_name: peer_name.clone(), path: remote.path.clone(), size: local.get(&remote.path).map_or(0, |e| e.size) },
                        remote,
                    ));
                    Ok(())
                }
                Action::Skip { path, reason } => {
                    issues.push(format!("“{path}” isn't synced: {reason}."));
                    Ok(())
                }
                Action::MoveLocal { from, tombstone, remote } => {
                    let r = self.move_local(folder, &from, &tombstone, &remote).await;
                    if r.is_ok() {
                        changed = true;
                        self.record_change(&peer_name, &remote.path, Some(from), ChangeKind::Renamed);
                    }
                    r
                }
                Action::CopyLocal { from, remote } => {
                    let r = async {
                        let tmp = self.copy_to_tmp(folder, &from, &remote).await?;
                        self.place(folder, &tmp, &remote, false).await
                    }
                    .await;
                    if let Ok(kind) = &r {
                        changed = true;
                        self.record_change(&peer_name, &remote.path, None, *kind);
                    }
                    r.map(|_| ())
                }
                Action::Download { remote, conflict } => {
                    let r = async {
                        let tmp = self
                            .fetch(&mut s, folder, &remote, |n| {
                                if last_status.elapsed() > STATUS_INTERVAL {
                                    last_status = Instant::now();
                                    self.with_peer(device_id, |p| p.status.bytes_left = bytes_left.saturating_sub(n));
                                    self.emit(Event::SyncChanged);
                                }
                            })
                            .await?;
                        self.place(folder, &tmp, &remote, conflict).await
                    }
                    .await;
                    files_left -= 1;
                    bytes_left = bytes_left.saturating_sub(remote.size);
                    self.with_peer(device_id, |p| {
                        p.status.files_left = files_left;
                        p.status.bytes_left = bytes_left;
                    });
                    if let Ok(kind) = &r {
                        changed = true;
                        self.record_change(&peer_name, &remote.path, None, *kind);
                        self.emit(Event::SyncChanged);
                    }
                    r.map(|_| ())
                }
            };
            if let Err(e) = result {
                tracing::debug!("sync step failed: {e:#}");
                if e.downcast_ref::<std::io::Error>().is_some_and(|io| matches!(io.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset)) {
                    return Err(e.context(format!("lost connection to {peer_name}")));
                }
                issues.push(format!("{e:#}"));
            }
        }
        self.with_peer(device_id, |p| p.pending = pending);
        if !issues.is_empty() {
            folder.issues.lock().unwrap().extend(issues);
        }
        if changed {
            folder.dirty.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn adopt(&self, folder: &Folder, remote: &Entry) {
        let mut idx = folder.index.lock().unwrap();
        if let Some(local) = idx.entries.get_mut(&remote.path) {
            local.version = merge(&local.version, &remote.version);
            idx.seq += 1;
        }
    }

    async fn make_dir(&self, folder: &Folder, remote: &Entry) -> Result<()> {
        let _g = folder.guard.lock().await;
        tokio::fs::create_dir_all(wire_to_path(&folder.root, &remote.path)).await?;
        let mut idx = folder.index.lock().unwrap();
        let version = merge(&idx.entries.get(&remote.path).map(|e| e.version.clone()).unwrap_or_default(), &remote.version);
        idx.entries.insert(remote.path.clone(), Entry { deleted: false, version, ..remote.clone() });
        idx.seq += 1;
        Ok(())
    }

    async fn remove_empty_dir(&self, folder: &Folder, remote: &Entry) -> Result<()> {
        let _g = folder.guard.lock().await;
        let dir = wire_to_path(&folder.root, &remote.path);
        if dir.exists() {
            if std::fs::read_dir(&dir)?.next().is_some() {
                return Ok(()); // Still has files (perhaps waiting for review); try again later.
            }
            std::fs::remove_dir(&dir)?;
        }
        let mut idx = folder.index.lock().unwrap();
        if let Some(e) = idx.entries.get_mut(&remote.path) {
            e.deleted = true;
            e.version = merge(&e.version, &remote.version);
            idx.seq += 1;
        }
        Ok(())
    }

    /// Download a file into the temp folder, resuming a previous partial download.
    async fn fetch<S>(&self, s: &mut S, folder: &Folder, remote: &Entry, mut progress: impl FnMut(u64)) -> Result<PathBuf>
    where
        S: tokio::io::AsyncRead + AsyncWrite + Unpin,
    {
        tokio::fs::create_dir_all(folder.tmp_dir()).await?;
        let part = folder.tmp_dir().join(format!("{}.part", remote.hash));
        let mut hasher = Sha256::new();
        let mut offset = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
        if offset > remote.size {
            tokio::fs::remove_file(&part).await?;
            offset = 0;
        }
        if offset > 0 {
            let mut existing = tokio::fs::File::open(&part).await?;
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = existing.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }

        write_msg(s, &Msg::GetFile { path: remote.path.clone(), hash: remote.hash.clone(), offset }).await?;
        match timeout(REQUEST_TIMEOUT, read_msg(s)).await.context("timed out")?? {
            Msg::FileData { size } if size == remote.size => {}
            Msg::FileData { .. } => bail!("{} changed on the other device; will retry", remote.path),
            other => return Err(other.unexpected()),
        }
        let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&part).await?;
        let mut remaining = remote.size - offset;
        let mut buf = vec![0u8; 256 * 1024];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = s.read(&mut buf[..want]).await?;
            if n == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
            remaining -= n as u64;
            progress(remote.size - remaining);
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        if hex::encode(hasher.finalize()) != remote.hash {
            let _ = tokio::fs::remove_file(&part).await;
            bail!("{} arrived corrupted; will retry", remote.path);
        }
        Ok(part)
    }

    async fn copy_to_tmp(&self, folder: &Folder, from: &str, remote: &Entry) -> Result<PathBuf> {
        tokio::fs::create_dir_all(folder.tmp_dir()).await?;
        let tmp = folder.tmp_dir().join(format!("{}.part", remote.hash));
        tokio::fs::copy(wire_to_path(&folder.root, from), &tmp).await?;
        let check = tmp.clone();
        if tokio::task::spawn_blocking(move || index::hash_file(&check)).await?? != remote.hash {
            let _ = tokio::fs::remove_file(&tmp).await;
            bail!("{from} changed while copying");
        }
        Ok(tmp)
    }

    /// Move a verified temp file into place, provided nothing changed locally meanwhile.
    async fn place(&self, folder: &Folder, tmp: &Path, remote: &Entry, conflict: bool) -> Result<ChangeKind> {
        let _g = folder.guard.lock().await;
        let target = wire_to_path(&folder.root, &remote.path);
        let local = folder.index.lock().unwrap().entries.get(&remote.path).cloned();
        ensure_untouched(&target, local.as_ref(), &remote.path)?;
        let existed = target.exists();
        if conflict && existed {
            let copy = wire_to_path(&folder.root, &conflict_name(&remote.path, &self.settings().device_name));
            std::fs::rename(&target, &copy)?;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(tmp, &target)?;
        let (size, mtime) = set_mtime(&target, remote.mtime)?;
        let mut idx = folder.index.lock().unwrap();
        let version = merge(&local.map(|e| e.version).unwrap_or_default(), &remote.version);
        idx.entries.insert(remote.path.clone(), Entry { size, mtime, deleted: false, version, ..remote.clone() });
        idx.seq += 1;
        Ok(if conflict { ChangeKind::Conflict } else if existed { ChangeKind::Updated } else { ChangeKind::Added })
    }

    async fn move_local(&self, folder: &Folder, from: &str, tombstone: &Entry, remote: &Entry) -> Result<()> {
        let _g = folder.guard.lock().await;
        let (source, target) = (wire_to_path(&folder.root, from), wire_to_path(&folder.root, &remote.path));
        let (local_from, local_to) = {
            let idx = folder.index.lock().unwrap();
            (idx.entries.get(from).cloned(), idx.entries.get(&remote.path).cloned())
        };
        ensure_untouched(&source, local_from.as_ref(), from)?;
        ensure_untouched(&target, local_to.as_ref(), &remote.path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&source, &target)?;
        let (size, mtime) = set_mtime(&target, remote.mtime)?;
        let mut idx = folder.index.lock().unwrap();
        if let Some(e) = idx.entries.get_mut(from) {
            e.deleted = true;
            e.version = merge(&e.version, &tombstone.version);
        }
        let version = merge(&local_to.map(|e| e.version).unwrap_or_default(), &remote.version);
        idx.entries.insert(remote.path.clone(), Entry { size, mtime, deleted: false, version, ..remote.clone() });
        idx.seq += 1;
        Ok(())
    }

    // ---------- Deletion review ----------

    /// Apply the user's decision on files the other device deleted.
    /// `delete` moves them to the Trash / Recycle Bin; otherwise they're kept and
    /// restored on the other device.
    pub async fn resolve_deletions(&self, device_id: &str, paths: Vec<String>, delete: bool) -> Result<()> {
        let folder = self.sync_folder()?;
        let me = self.me_short();
        let mut errors = Vec::new();
        for path in paths {
            let item = self.with_peer(device_id, |p| {
                let i = p.pending.iter().position(|(d, _)| d.path == path)?;
                Some(p.pending.remove(i))
            });
            let Some((pending, tombstone)) = item else { continue };
            let result: Result<()> = async {
                let _g = folder.guard.lock().await;
                let target = wire_to_path(&folder.root, &path);
                let local = folder.index.lock().unwrap().entries.get(&path).cloned().ok_or_else(|| anyhow!("{path} is no longer tracked"))?;
                if delete {
                    ensure_untouched(&target, Some(&local), &path)?;
                    move_to_trash(&target, self.use_trash()).with_context(|| format!("couldn't move {path} to the Trash"))?;
                }
                let mut idx = folder.index.lock().unwrap();
                if let Some(e) = idx.entries.get_mut(&path) {
                    let merged = merge(&e.version, &tombstone.version);
                    if delete {
                        e.deleted = true;
                        e.version = merged;
                    } else {
                        // Newer than the deletion, so the other device brings the file back.
                        e.version = bump(&merged, &me);
                    }
                    idx.seq += 1;
                }
                Ok(())
            }
            .await;
            match result {
                Ok(()) => self.record_change(&pending.device_name, &path, None, if delete { ChangeKind::Deleted } else { ChangeKind::Kept }),
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        folder.save();
        self.emit(Event::SyncChanged);
        self.notify_peers();
        if !errors.is_empty() {
            bail!(errors.join("\n"));
        }
        Ok(())
    }

    // ---------- Serving the other device ----------

    pub(crate) async fn serve_sync<S>(&self, s: &mut S, peer_id: &str, first: Msg) -> Result<()>
    where
        S: tokio::io::AsyncRead + AsyncWrite + Unpin,
    {
        let mut request = first;
        loop {
            match request {
                Msg::SyncNotify => {
                    self.with_peer(peer_id, |p| p.notified = true);
                    self.sync_state().wake.notify_one();
                    write_msg(s, &Msg::Pong).await?;
                }
                Msg::GetIndex { index_id, seq } => self.serve_index(s, index_id, seq).await?,
                Msg::GetFile { path, hash, offset } => self.serve_file(s, &path, &hash, offset).await?,
                other => return Err(other.unexpected()),
            }
            request = match timeout(REQUEST_TIMEOUT, read_msg(s)).await {
                Ok(Ok(m)) => m,
                _ => return Ok(()),
            };
        }
    }

    async fn serve_index<S: AsyncWrite + Unpin>(&self, s: &mut S, known_id: Option<String>, known_seq: u64) -> Result<()> {
        if !self.settings().sync_enabled {
            return write_msg(s, &Msg::error(format!("Sync is turned off on {}.", self.settings().device_name))).await;
        }
        let folder = self.sync_folder()?;
        if folder.last_scan.lock().unwrap().is_none() {
            self.scan(&folder).await?;
        }
        let current = {
            let idx = folder.index.lock().unwrap();
            let unchanged = known_id.as_deref() == Some(idx.index_id.as_str()) && known_seq == idx.seq;
            (!unchanged).then(|| (idx.index_id.clone(), idx.seq, idx.entries.values().cloned().collect::<Vec<_>>()))
        };
        let Some((index_id, seq, entries)) = current else {
            return write_msg(s, &Msg::IndexUnchanged).await;
        };
        let chunks: Vec<&[Entry]> = if entries.is_empty() { vec![&[]] } else { entries.chunks(INDEX_CHUNK).collect() };
        let count = chunks.len();
        for (i, chunk) in chunks.into_iter().enumerate() {
            write_msg(s, &Msg::IndexChunk { index_id: index_id.clone(), seq, entries: chunk.to_vec(), last: i + 1 == count }).await?;
        }
        Ok(())
    }

    async fn serve_file<S: AsyncWrite + Unpin>(&self, s: &mut S, path: &str, hash: &str, offset: u64) -> Result<()> {
        let folder = self.sync_folder()?;
        let entry = folder.index.lock().unwrap().entries.get(path).cloned();
        let entry = match entry {
            Some(e) if !e.deleted && !e.dir && e.hash == hash && offset <= e.size => e,
            _ => return write_msg(s, &Msg::error(format!("{path} changed; will retry"))).await,
        };
        let full = self.resolve_sync(path)?;
        let meta = tokio::fs::metadata(&full).await?;
        if meta.len() != entry.size || mtime_ms(&meta) != entry.mtime {
            folder.dirty.store(true, Ordering::SeqCst);
            return write_msg(s, &Msg::error(format!("{path} changed; will retry"))).await;
        }
        write_msg(s, &Msg::FileData { size: entry.size }).await?;
        let mut file = tokio::fs::File::open(&full).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut remaining = entry.size - offset;
        let mut buf = vec![0u8; 256 * 1024];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                bail!("{path} shrank while sending");
            }
            s.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
        s.flush().await?;
        Ok(())
    }
}

// ---------- Helpers ----------

/// Refuse to touch a file the user changed after it was last scanned.
fn ensure_untouched(path: &Path, indexed: Option<&Entry>, wire: &str) -> Result<()> {
    let live = indexed.filter(|e| !e.deleted && !e.dir);
    match (live, std::fs::metadata(path)) {
        (Some(e), Ok(m)) if m.len() == e.size && mtime_ms(&m) == e.mtime => Ok(()),
        (None, Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => bail!("{wire} changed on this device; will sync after the next scan"),
    }
}

fn set_mtime(path: &Path, mtime_ms: i64) -> Result<(u64, i64)> {
    if mtime_ms > 0 {
        let f = std::fs::File::options().write(true).open(path)?;
        let _ = f.set_modified(UNIX_EPOCH + Duration::from_millis(mtime_ms as u64));
    }
    let meta = std::fs::metadata(path)?;
    Ok((meta.len(), index::mtime_ms(&meta)))
}

fn conflict_name(wire: &str, device_name: &str) -> String {
    let (dir, name) = wire.rsplit_once('/').map_or(("", wire), |(d, n)| (d, n));
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M");
    let device = crate::paths::sanitize_component(device_name).unwrap_or_else(|_| "this device".into());
    let tag = format!("conflict from {device} {stamp}");
    let file = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({tag}).{ext}"),
        _ => format!("{name} ({tag})"),
    };
    if dir.is_empty() { file } else { format!("{dir}/{file}") }
}

fn move_to_trash(path: &Path, use_trash: bool) -> Result<()> {
    if !use_trash {
        std::fs::remove_file(path)?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        // Use the system API directly rather than scripting Finder (no extra permission prompt).
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path)?;
    }
    #[cfg(not(target_os = "macos"))]
    trash::delete(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::conflict_name;

    #[test]
    fn conflict_names_keep_extension_and_folder() {
        let n = conflict_name("docs/report.docx", "Arun's PC");
        assert!(n.starts_with("docs/report (conflict from Arun's PC "), "{n}");
        assert!(n.ends_with(").docx"));
        assert!(conflict_name("README", "Mac").starts_with("README (conflict from Mac "));
    }
}
