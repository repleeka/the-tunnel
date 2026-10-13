//! The engine ties everything together: it listens for connections from other
//! devices, keeps track of who is nearby, and runs pairing and transfers. The UI
//! talks to it through the public methods and listens to `Event`s.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot};
use tokio::time::timeout;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

use crate::identity::Identity;
use crate::net::{is_local, local_addresses};
use crate::paths::{self, LocalItem};
use crate::protocol::{Hello, Item, Msg, PROTOCOL_VERSION, RemoteEntry, read_msg, write_msg};
use crate::store::{BrowseScope, PairedDevice, Settings, TrustStore, load_json, save_json};
use crate::sync::{SyncState, SyncStatus};
use crate::transfer::{Cancel, ReceivePlan, ResumeInfo};
use crate::{discovery, pairing, tls, transfer};

type Stream = TlsStream<TcpStream>;

pub const DEFAULT_PORT: u16 = 47800;
const ONLINE_WINDOW: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const DECISION_TIMEOUT: Duration = Duration::from_secs(120);
const OFFER_TIMEOUT: Duration = Duration::from_secs(300);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
const MAX_TRANSFERS_KEPT: usize = 50;
const MAX_ITEMS: usize = 500_000;

pub struct EngineConfig {
    pub data_dir: PathBuf,
    /// `None` tries the default port and falls back to any free one.
    pub port: Option<u16>,
    /// Run background work: discovering devices and syncing the folder.
    /// Tests turn this off and drive sync explicitly with `sync_now`.
    pub discovery: bool,
    /// Move files deleted through review to the Trash / Recycle Bin (off in tests).
    pub use_trash: bool,
}

// ---------- Views and events sent to the UI ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub device_id: String,
    pub device_name: String,
    pub profile_name: String,
    pub os: String,
    pub port: u16,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub device_name: String,
    pub profile_name: String,
    pub os: String,
    pub paired: bool,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Waiting,
    Active,
    Done,
    Declined,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferInfo {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub direction: Direction,
    pub label: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub status: TransferStatus,
    pub error: Option<String>,
    /// Where received items were saved (the first top-level item).
    pub location: Option<String>,
    pub started_at: u64,
    /// A failed or cancelled send/copy that can be resumed.
    pub can_retry: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub me: Me,
    pub settings: Settings,
    pub devices: Vec<DeviceView>,
    pub transfers: Vec<TransferInfo>,
    pub sync: SyncStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    pub entries: Vec<RemoteEntry>,
    pub root: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub onboarded: Option<bool>,
    pub device_name: Option<String>,
    pub profile_name: Option<String>,
    pub receive_dir: Option<PathBuf>,
    pub sync_dir: Option<PathBuf>,
    pub ask_before_receiving: Option<bool>,
    pub sync_enabled: Option<bool>,
    pub browse_scope: Option<BrowseScope>,
    pub launch_at_login: Option<bool>,
    pub context_menu: Option<bool>,
    pub notifications: Option<bool>,
    pub tray_hint_shown: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Event {
    DevicesChanged,
    SettingsChanged,
    PairingCode { session_id: String, device: DeviceView, code: String, incoming: bool },
    PairingFinished { session_id: String, device_id: String, success: bool, message: Option<String> },
    IncomingOffer { transfer_id: String, device_name: String, label: String, file_count: usize, total_bytes: u64 },
    TransferChanged { transfer: TransferInfo },
    SyncChanged,
}

// ---------- Engine ----------

struct Nearby {
    device_name: String,
    profile_name: String,
    os: String,
    addrs: Vec<SocketAddr>,
    last_seen: Instant,
    via_mdns: bool,
}

impl Nearby {
    fn online(&self) -> bool {
        self.via_mdns || self.last_seen.elapsed() < ONLINE_WINDOW
    }
}

#[derive(Clone)]
enum RetryKind {
    Send { device_id: String, selection: Vec<PathBuf> },
    Pull { device_id: String, wires: Vec<String> },
}

struct Inner {
    identity: Identity,
    data_dir: PathBuf,
    port: u16,
    acceptor: TlsAcceptor,
    connector: TlsConnector,
    settings: RwLock<Settings>,
    trust: RwLock<TrustStore>,
    nearby: RwLock<HashMap<String, Nearby>>,
    pairings: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    offers: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    transfers: Mutex<Vec<TransferInfo>>,
    events: broadcast::Sender<Event>,
    mdns: Mutex<Option<mdns_sd::ServiceDaemon>>,
    sync: SyncState,
    use_trash: bool,
    cancels: Mutex<HashMap<String, Arc<Cancel>>>,
    retries: Mutex<HashMap<String, RetryKind>>,
    /// Partly received transfers, by the sender's transfer id, with the sending device.
    partials: Mutex<HashMap<String, (String, ReceivePlan)>>,
    /// Sender transfer ids currently being received, so a retry can't overlap the old attempt.
    receiving: Mutex<std::collections::HashSet<String>>,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    pub async fn start(config: EngineConfig) -> Result<Engine> {
        std::fs::create_dir_all(&config.data_dir).context("creating app data folder")?;
        let identity = Identity::load_or_create(&config.data_dir)?;
        let mut settings: Settings = load_json(&config.data_dir.join("settings.json"));
        if settings.migrate() {
            save_json(&config.data_dir.join("settings.json"), &settings)?;
        }
        let trust: TrustStore = load_json(&config.data_dir.join("devices.json"));

        let listener = match config.port {
            Some(port) => TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?,
            None => match TcpListener::bind((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT)).await {
                Ok(l) => l,
                Err(_) => TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?,
            },
        };
        let port = listener.local_addr()?.port();
        // Partial files from a previous run can't be resumed (that state lived in memory).
        let _ = std::fs::remove_dir_all(settings.receive_dir.join(transfer::PARTIAL_DIR));

        let engine = Engine {
            inner: Arc::new(Inner {
                acceptor: tls::acceptor(&identity)?,
                connector: tls::connector(&identity)?,
                identity,
                data_dir: config.data_dir,
                port,
                settings: RwLock::new(settings),
                trust: RwLock::new(trust),
                nearby: RwLock::new(HashMap::new()),
                pairings: Mutex::new(HashMap::new()),
                offers: Mutex::new(HashMap::new()),
                transfers: Mutex::new(Vec::new()),
                events: broadcast::channel(1024).0,
                mdns: Mutex::new(None),
                sync: SyncState::default(),
                use_trash: config.use_trash,
                cancels: Mutex::new(HashMap::new()),
                retries: Mutex::new(HashMap::new()),
                partials: Mutex::new(HashMap::new()),
                receiving: Mutex::new(std::collections::HashSet::new()),
            }),
        };

        let accepting = engine.clone();
        tokio::spawn(async move { accepting.accept_loop(listener).await });
        if config.discovery {
            discovery::start(engine.clone());
            engine.start_sync();
        }
        Ok(engine)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn device_id(&self) -> &str {
        &self.inner.identity.device_id
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().unwrap().clone()
    }

    pub(crate) fn sync_state(&self) -> &SyncState {
        &self.inner.sync
    }

    pub(crate) fn use_trash(&self) -> bool {
        self.inner.use_trash
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub(crate) fn emit(&self, event: Event) {
        let _ = self.inner.events.send(event);
    }

    fn hello(&self) -> Hello {
        let s = self.inner.settings.read().unwrap();
        Hello {
            protocol: PROTOCOL_VERSION,
            device_name: s.device_name.clone(),
            profile_name: s.profile_name.clone(),
            os: std::env::consts::OS.into(),
            port: self.inner.port,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let settings = self.settings();
        let me = Me {
            device_id: self.device_id().to_string(),
            device_name: settings.device_name.clone(),
            profile_name: settings.profile_name.clone(),
            os: std::env::consts::OS.into(),
            port: self.inner.port,
            addresses: local_addresses().iter().map(|a| a.to_string()).collect(),
        };
        Snapshot { me, settings, devices: self.devices(), transfers: self.inner.transfers.lock().unwrap().clone(), sync: self.sync_status() }
    }

    pub(crate) fn devices(&self) -> Vec<DeviceView> {
        let trust = self.inner.trust.read().unwrap();
        let nearby = self.inner.nearby.read().unwrap();
        let mut out: Vec<DeviceView> = trust
            .devices
            .values()
            .map(|d| {
                let near = nearby.get(&d.device_id);
                DeviceView {
                    id: d.device_id.clone(),
                    device_name: near.map_or(&d.device_name, |n| &n.device_name).clone(),
                    profile_name: near.map_or(&d.profile_name, |n| &n.profile_name).clone(),
                    os: d.os.clone(),
                    paired: true,
                    online: near.is_some_and(Nearby::online),
                }
            })
            .collect();
        out.extend(nearby.iter().filter(|(id, n)| !trust.devices.contains_key(*id) && n.online()).map(|(id, n)| DeviceView {
            id: id.clone(),
            device_name: n.device_name.clone(),
            profile_name: n.profile_name.clone(),
            os: n.os.clone(),
            paired: false,
            online: true,
        }));
        out.sort_by(|a, b| b.paired.cmp(&a.paired).then(b.online.cmp(&a.online)).then(a.device_name.cmp(&b.device_name)));
        out
    }

    fn device_view(&self, id: &str, hello: &Hello) -> DeviceView {
        DeviceView {
            id: id.to_string(),
            device_name: hello.device_name.clone(),
            profile_name: hello.profile_name.clone(),
            os: hello.os.clone(),
            paired: self.is_paired(id),
            online: true,
        }
    }

    fn is_paired(&self, id: &str) -> bool {
        self.inner.trust.read().unwrap().devices.contains_key(id)
    }

    fn device_name(&self, id: &str) -> String {
        if let Some(n) = self.inner.nearby.read().unwrap().get(id) {
            return n.device_name.clone();
        }
        self.inner.trust.read().unwrap().devices.get(id).map(|d| d.device_name.clone()).unwrap_or_else(|| "Unknown device".into())
    }

    pub fn update_settings(&self, patch: SettingsPatch) -> Result<Settings> {
        let updated = {
            let mut s = self.inner.settings.write().unwrap();
            if let Some(v) = patch.onboarded { s.onboarded = v }
            if let Some(v) = patch.device_name { s.device_name = clean_name(&v, 64)? }
            if let Some(v) = patch.profile_name { s.profile_name = clean_name(&v, 64).unwrap_or_default() }
            if let Some(v) = patch.receive_dir { s.receive_dir = v }
            if let Some(v) = patch.sync_dir {
                validate_sync_dir(&v, &self.inner.data_dir)?;
                s.sync_dir = v;
            }
            if let Some(v) = patch.browse_scope { s.browse_scope = v }
            if let Some(v) = patch.launch_at_login { s.launch_at_login = v }
            if let Some(v) = patch.context_menu { s.context_menu = v }
            if let Some(v) = patch.notifications { s.notifications = v }
            if let Some(v) = patch.tray_hint_shown { s.tray_hint_shown = v }
            if let Some(v) = patch.ask_before_receiving { s.ask_before_receiving = v }
            if let Some(v) = patch.sync_enabled { s.sync_enabled = v }
            s.clone()
        };
        save_json(&self.inner.data_dir.join("settings.json"), &updated)?;
        if updated.onboarded {
            let _ = std::fs::create_dir_all(&updated.sync_dir);
        }
        discovery::refresh_mdns(self);
        self.inner.sync.wake_now();
        self.emit(Event::SettingsChanged);
        Ok(updated)
    }

    fn save_trust(&self) {
        let trust = self.inner.trust.read().unwrap().clone();
        if let Err(e) = save_json(&self.inner.data_dir.join("devices.json"), &trust) {
            tracing::warn!("couldn't save paired devices: {e:#}");
        }
    }

    pub fn unpair(&self, device_id: &str) {
        self.inner.trust.write().unwrap().devices.remove(device_id);
        self.save_trust();
        self.emit(Event::DevicesChanged);
    }

    // ---------- Discovery bookkeeping (called by the discovery module) ----------

    pub(crate) fn mdns_slot(&self) -> &Mutex<Option<mdns_sd::ServiceDaemon>> {
        &self.inner.mdns
    }

    /// Record that a device was seen at `addr`. Returns nothing; emits a change if it's new.
    pub(crate) fn note_device(&self, id: &str, hello: &Hello, addr: SocketAddr, via_mdns: bool) {
        if id == self.device_id() || !is_local(&addr.ip()) {
            return;
        }
        let changed = {
            let mut nearby = self.inner.nearby.write().unwrap();
            match nearby.get_mut(id) {
                Some(n) => {
                    let was_online = n.online();
                    let renamed = n.device_name != hello.device_name || n.profile_name != hello.profile_name;
                    n.device_name = hello.device_name.clone();
                    n.profile_name = hello.profile_name.clone();
                    n.os = hello.os.clone();
                    n.last_seen = Instant::now();
                    n.via_mdns |= via_mdns;
                    n.addrs.retain(|a| *a != addr);
                    n.addrs.insert(0, addr);
                    n.addrs.truncate(4);
                    !was_online || renamed
                }
                None => {
                    nearby.insert(
                        id.to_string(),
                        Nearby {
                            device_name: hello.device_name.clone(),
                            profile_name: hello.profile_name.clone(),
                            os: hello.os.clone(),
                            addrs: vec![addr],
                            last_seen: Instant::now(),
                            via_mdns,
                        },
                    );
                    true
                }
            }
        };
        if changed {
            self.emit(Event::DevicesChanged);
        }
    }

    pub(crate) fn mdns_lost(&self, id: &str) {
        if let Some(n) = self.inner.nearby.write().unwrap().get_mut(id) {
            n.via_mdns = false;
        }
        self.emit(Event::DevicesChanged);
    }

    /// Drop devices that stopped announcing themselves.
    pub(crate) fn prune_nearby(&self) {
        let removed = {
            let mut nearby = self.inner.nearby.write().unwrap();
            let before = nearby.len();
            nearby.retain(|_, n| n.online() || n.last_seen.elapsed() < ONLINE_WINDOW * 2);
            let went_offline = nearby.values().any(|n| !n.online() && n.last_seen.elapsed() < ONLINE_WINDOW + Duration::from_secs(3));
            before != nearby.len() || went_offline
        };
        if removed {
            self.emit(Event::DevicesChanged);
        }
    }

    // ---------- Connections ----------

    async fn accept_loop(self, listener: TcpListener) {
        loop {
            let Ok((tcp, addr)) = listener.accept().await else { continue };
            if !is_local(&addr.ip()) {
                tracing::warn!("refused connection from non-local address {addr}");
                continue;
            }
            let engine = self.clone();
            tokio::spawn(async move {
                if let Err(e) = engine.handle_incoming(tcp, addr).await {
                    tracing::debug!("connection from {addr} ended: {e:#}");
                }
            });
        }
    }

    fn candidate_addrs(&self, device_id: &str) -> Vec<SocketAddr> {
        let mut addrs = self.inner.nearby.read().unwrap().get(device_id).map(|n| n.addrs.clone()).unwrap_or_default();
        if let Some(d) = self.inner.trust.read().unwrap().devices.get(device_id) {
            for a in &d.last_addrs {
                if !addrs.contains(a) {
                    addrs.push(*a);
                }
            }
        }
        addrs
    }

    /// Open an authenticated connection to a known device.
    pub(crate) async fn connect(&self, device_id: &str) -> Result<(Stream, Hello)> {
        let addrs = self.candidate_addrs(device_id);
        if addrs.is_empty() {
            bail!("{} isn't on this network right now", self.device_name(device_id));
        }
        let mut last_err = None;
        for addr in addrs {
            match self.connect_addr(addr).await {
                Ok((stream, id, hello)) if id == device_id => return Ok((stream, hello)),
                Ok(_) => last_err = Some(anyhow!("a different device answered at {addr}")),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("couldn't reach the device")))
            .with_context(|| format!("couldn't reach {}", self.device_name(device_id)))
    }

    async fn connect_addr(&self, addr: SocketAddr) -> Result<(Stream, String, Hello)> {
        if !is_local(&addr.ip()) {
            bail!("The Tunnel only connects to devices on your local network");
        }
        let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await.context("timed out")??;
        tcp.set_nodelay(true)?;
        let tls = timeout(HANDSHAKE_TIMEOUT, self.inner.connector.connect(tls::server_name(), tcp)).await.context("timed out")??;
        let mut stream = TlsStream::Client(tls);
        let peer_id = tls::peer_id(stream.get_ref().1.peer_certificates())?;
        if peer_id == self.device_id() {
            bail!("that address is this device");
        }
        write_msg(&mut stream, &Msg::Hello(self.hello())).await?;
        let hello = match timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await.context("timed out")?? {
            Msg::Hello(h) => h,
            other => return Err(other.unexpected()),
        };
        check_protocol(&hello)?;
        self.note_device(&peer_id, &hello, addr, false);
        self.remember_addr(&peer_id, addr);
        Ok((stream, peer_id, hello))
    }

    fn remember_addr(&self, device_id: &str, addr: SocketAddr) {
        let mut trust = self.inner.trust.write().unwrap();
        if let Some(d) = trust.devices.get_mut(device_id)
            && d.last_addrs.first() != Some(&addr) {
                d.last_addrs.retain(|a| *a != addr);
                d.last_addrs.insert(0, addr);
                d.last_addrs.truncate(4);
                drop(trust);
                self.save_trust();
            }
    }

    /// Reach a device by IP address when automatic discovery is blocked.
    pub async fn connect_address(&self, input: &str) -> Result<DeviceView> {
        let addr = parse_address(input)?;
        let (mut stream, id, hello) = self.connect_addr(addr).await?;
        write_msg(&mut stream, &Msg::Ping).await?;
        let _ = timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await;
        Ok(self.device_view(&id, &hello))
    }

    async fn handle_incoming(&self, tcp: TcpStream, addr: SocketAddr) -> Result<()> {
        tcp.set_nodelay(true)?;
        let tls = timeout(HANDSHAKE_TIMEOUT, self.inner.acceptor.accept(tcp)).await.context("handshake timed out")??;
        let mut s = TlsStream::Server(tls);
        let peer_id = tls::peer_id(s.get_ref().1.peer_certificates())?;
        let hello = match timeout(HANDSHAKE_TIMEOUT, read_msg(&mut s)).await.context("timed out")?? {
            Msg::Hello(h) => h,
            other => return Err(other.unexpected()),
        };
        write_msg(&mut s, &Msg::Hello(self.hello())).await?;
        if let Err(e) = check_protocol(&hello) {
            write_msg(&mut s, &Msg::error(e.to_string())).await?;
            return Ok(());
        }
        let callback = SocketAddr::new(addr.ip(), hello.port);
        self.note_device(&peer_id, &hello, callback, false);

        let request = timeout(HANDSHAKE_TIMEOUT, read_msg(&mut s)).await.context("timed out")??;
        let paired = self.is_paired(&peer_id);
        if paired {
            self.update_paired_details(&peer_id, &hello, callback);
        }
        match request {
            Msg::Ping => write_msg(&mut s, &Msg::Pong).await,
            Msg::PairRequest { commitment } => self.respond_pair(s, peer_id, hello, commitment).await,
            _ if !paired => write_msg(&mut s, &Msg::error("This device isn't paired with you. Pair first.")).await,
            Msg::Offer { transfer_id, items } => self.receive_offer(s, peer_id, hello, transfer_id, items).await,
            Msg::List { path } => self.serve_listing(s, path).await,
            Msg::Pull { paths, transfer_id } => self.serve_pull(s, peer_id, hello, paths, transfer_id).await,
            req @ (Msg::SyncNotify | Msg::GetIndex { .. } | Msg::GetFile { .. }) => self.serve_sync(&mut s, &peer_id, req).await,
            other => Err(other.unexpected()),
        }
    }

    fn update_paired_details(&self, id: &str, hello: &Hello, addr: SocketAddr) {
        {
            let mut trust = self.inner.trust.write().unwrap();
            let Some(d) = trust.devices.get_mut(id) else { return };
            d.device_name = hello.device_name.clone();
            d.profile_name = hello.profile_name.clone();
            d.os = hello.os.clone();
        }
        self.remember_addr(id, addr);
        self.save_trust();
    }

    // ---------- Pairing ----------

    /// Start pairing with a nearby device. Both screens will show a code to compare.
    pub fn pair(&self, device_id: &str) -> String {
        let session = uuid::Uuid::new_v4().to_string();
        let (engine, id, sid) = (self.clone(), device_id.to_string(), session.clone());
        tokio::spawn(async move {
            if let Err(e) = engine.initiate_pair(&sid, &id).await {
                engine.emit(Event::PairingFinished { session_id: sid, device_id: id, success: false, message: Some(format!("{e:#}")) });
            }
        });
        session
    }

    async fn initiate_pair(&self, session: &str, device_id: &str) -> Result<()> {
        let (mut s, hello) = self.connect(device_id).await?;
        let mine = pairing::nonce();
        write_msg(&mut s, &Msg::PairRequest { commitment: pairing::commitment(&mine) }).await?;
        let theirs = match timeout(HANDSHAKE_TIMEOUT, read_msg(&mut s)).await.context("timed out")?? {
            Msg::PairNonce { nonce } => pairing::decode_nonce(&nonce)?,
            other => return Err(other.unexpected()),
        };
        write_msg(&mut s, &Msg::PairReveal { nonce: hex::encode(mine) }).await?;
        let code = pairing::code(self.device_id(), device_id, &mine, &theirs);
        self.confirm_pairing(s, session.to_string(), device_id.to_string(), hello, code, false).await
    }

    async fn respond_pair(&self, mut s: Stream, peer_id: String, hello: Hello, commitment: String) -> Result<()> {
        let mine = pairing::nonce();
        write_msg(&mut s, &Msg::PairNonce { nonce: hex::encode(mine) }).await?;
        let theirs = match timeout(HANDSHAKE_TIMEOUT, read_msg(&mut s)).await.context("timed out")?? {
            Msg::PairReveal { nonce } => pairing::decode_nonce(&nonce)?,
            other => return Err(other.unexpected()),
        };
        if pairing::commitment(&theirs) != commitment {
            bail!("pairing commitment mismatch");
        }
        let code = pairing::code(&peer_id, self.device_id(), &theirs, &mine);
        self.confirm_pairing(s, uuid::Uuid::new_v4().to_string(), peer_id, hello, code, true).await
    }

    /// Show the code, then wait until both people have confirmed (or one declines).
    async fn confirm_pairing(&self, s: Stream, session: String, peer_id: String, hello: Hello, code: String, incoming: bool) -> Result<()> {
        let (tx, mut local_rx) = oneshot::channel();
        self.inner.pairings.lock().unwrap().insert(session.clone(), tx);
        self.emit(Event::PairingCode { session_id: session.clone(), device: self.device_view(&peer_id, &hello), code, incoming });

        let (mut rd, mut wr) = tokio::io::split(s);
        let remote = read_msg(&mut rd);
        tokio::pin!(remote);
        let deadline = tokio::time::sleep(DECISION_TIMEOUT);
        tokio::pin!(deadline);

        let (mut local_ok, mut remote_ok) = (None, None);
        let outcome: Result<(), String> = loop {
            tokio::select! {
                d = &mut local_rx, if local_ok.is_none() => {
                    let accept = d.unwrap_or(false);
                    local_ok = Some(accept);
                    let _ = write_msg(&mut wr, &Msg::PairDecision { accept }).await;
                    if !accept { break Err("Pairing cancelled.".into()) }
                }
                m = &mut remote, if remote_ok.is_none() => match m {
                    Ok(Msg::PairDecision { accept: true }) => remote_ok = Some(true),
                    Ok(Msg::PairDecision { accept: false }) => break Err(format!("{} declined.", hello.device_name)),
                    _ => break Err(format!("Lost connection to {}.", hello.device_name)),
                },
                _ = &mut deadline => break Err("Pairing timed out.".into()),
            }
            if local_ok == Some(true) && remote_ok == Some(true) {
                break Ok(());
            }
        };
        self.inner.pairings.lock().unwrap().remove(&session);

        let success = outcome.is_ok();
        if success {
            let device = PairedDevice {
                device_id: peer_id.clone(),
                device_name: hello.device_name.clone(),
                profile_name: hello.profile_name.clone(),
                os: hello.os.clone(),
                paired_at: now_secs(),
                last_addrs: self.candidate_addrs(&peer_id),
            };
            self.inner.trust.write().unwrap().devices.insert(peer_id.clone(), device);
            self.save_trust();
            self.emit(Event::DevicesChanged);
        }
        self.emit(Event::PairingFinished { session_id: session, device_id: peer_id, success, message: outcome.err() });
        Ok(())
    }

    pub fn respond_pairing(&self, session_id: &str, accept: bool) {
        if let Some(tx) = self.inner.pairings.lock().unwrap().remove(session_id) {
            let _ = tx.send(accept);
        }
    }

    // ---------- Transfers ----------

    /// Add a transfer to the activity list. Reusing an id (a retry) replaces the old row.
    fn new_transfer(&self, id: Option<String>, device_id: &str, direction: Direction, items: &[Item], status: TransferStatus) -> TransferInfo {
        let t = TransferInfo {
            id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            device_id: device_id.to_string(),
            device_name: self.device_name(device_id),
            direction,
            label: label_for(items),
            file_count: items.iter().filter(|i| !i.dir).count(),
            total_bytes: items.iter().map(|i| i.size).sum(),
            done_bytes: 0,
            status,
            error: None,
            location: None,
            started_at: now_secs(),
            can_retry: false,
        };
        let mut list = self.inner.transfers.lock().unwrap();
        list.retain(|x| x.id != t.id);
        list.insert(0, t.clone());
        list.truncate(MAX_TRANSFERS_KEPT);
        drop(list);
        self.emit(Event::TransferChanged { transfer: t.clone() });
        t
    }

    fn update_transfer(&self, id: &str, f: impl FnOnce(&mut TransferInfo)) {
        let updated = {
            let mut list = self.inner.transfers.lock().unwrap();
            let Some(t) = list.iter_mut().find(|t| t.id == id) else { return };
            f(t);
            t.clone()
        };
        self.emit(Event::TransferChanged { transfer: updated });
    }

    fn finish_transfer(&self, id: &str, result: Result<Option<PathBuf>>) {
        let can_retry = self.inner.retries.lock().unwrap().contains_key(id);
        if result.is_ok() {
            self.inner.retries.lock().unwrap().remove(id);
        }
        self.update_transfer(id, |t| match result {
            Ok(location) => {
                t.status = TransferStatus::Done;
                t.done_bytes = t.total_bytes;
                t.location = location.map(|p| p.display().to_string());
                t.can_retry = false;
            }
            Err(e) if transfer::is_cancelled(&e) => {
                t.status = TransferStatus::Cancelled;
                t.can_retry = can_retry;
            }
            Err(e) => {
                t.status = TransferStatus::Failed;
                t.error = Some(format!("{e:#}"));
                t.can_retry = can_retry;
            }
        });
    }

    /// Progress callback that throttles UI updates.
    fn progress(&self, id: &str) -> impl FnMut(u64) + use<> {
        let (engine, id) = (self.clone(), id.to_string());
        let mut last = Instant::now() - PROGRESS_INTERVAL;
        move |done| {
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                engine.update_transfer(&id, |t| t.done_bytes = done);
            }
        }
    }

    fn register_cancel(&self, id: &str) -> Arc<Cancel> {
        let cancel = Arc::new(Cancel::default());
        self.inner.cancels.lock().unwrap().insert(id.to_string(), cancel.clone());
        cancel
    }

    fn drop_cancel(&self, id: &str) {
        self.inner.cancels.lock().unwrap().remove(id);
    }

    /// Stop a transfer that is waiting or in progress.
    pub fn cancel_transfer(&self, id: &str) {
        self.respond_offer(id, false);
        if let Some(c) = self.inner.cancels.lock().unwrap().get(id) {
            c.cancel();
        }
    }

    /// Start a failed or cancelled transfer again, resuming where it stopped.
    pub async fn retry_transfer(&self, id: &str) -> Result<()> {
        let kind = self.inner.retries.lock().unwrap().get(id).cloned().context("This transfer can't be retried.")?;
        match kind {
            RetryKind::Send { device_id, selection } => {
                let locals = tokio::task::spawn_blocking(move || paths::collect(&selection, &Default::default())).await??;
                if locals.is_empty() {
                    bail!("The files no longer exist.");
                }
                let items: Vec<Item> = locals.iter().map(|l| l.item.clone()).collect();
                self.new_transfer(Some(id.to_string()), &device_id, Direction::Send, &items, TransferStatus::Waiting);
                self.spawn_send(device_id, id.to_string(), items, locals);
            }
            RetryKind::Pull { device_id, wires } => {
                self.start_pull(&device_id, wires, Some(id.to_string())).await?;
            }
        }
        Ok(())
    }

    /// Send files and folders to a paired device. Returns the transfer id.
    pub async fn send_paths(&self, device_id: &str, selection: Vec<PathBuf>) -> Result<String> {
        if !self.is_paired(device_id) {
            bail!("Pair with this device before sending files.");
        }
        let chosen = selection.clone();
        let locals = tokio::task::spawn_blocking(move || paths::collect(&selection, &Default::default())).await??;
        if locals.is_empty() {
            bail!("Nothing to send.");
        }
        let items: Vec<Item> = locals.iter().map(|l| l.item.clone()).collect();
        let t = self.new_transfer(None, device_id, Direction::Send, &items, TransferStatus::Waiting);
        self.inner.retries.lock().unwrap().insert(t.id.clone(), RetryKind::Send { device_id: device_id.to_string(), selection: chosen });
        self.spawn_send(device_id.to_string(), t.id.clone(), items, locals);
        Ok(t.id)
    }

    fn spawn_send(&self, device_id: String, id: String, items: Vec<Item>, locals: Vec<LocalItem>) {
        let engine = self.clone();
        let cancel = self.register_cancel(&id);
        tokio::spawn(async move {
            let result = engine.run_send(&device_id, &id, items, locals, &cancel).await;
            engine.drop_cancel(&id);
            engine.finish_send(&id, result);
        });
    }

    fn finish_send(&self, id: &str, result: Result<bool>) {
        match result {
            Ok(true) => self.finish_transfer(id, Ok(None)),
            Ok(false) => self.update_transfer(id, |t| t.status = TransferStatus::Declined),
            Err(e) => self.finish_transfer(id, Err(e)),
        }
    }

    async fn run_send(&self, device_id: &str, id: &str, items: Vec<Item>, locals: Vec<LocalItem>, cancel: &Cancel) -> Result<bool> {
        let (mut s, _) = tokio::select! {
            r = self.connect(device_id) => r?,
            _ = cancel.cancelled() => bail!(transfer::Cancelled),
        };
        write_msg(&mut s, &Msg::Offer { transfer_id: id.to_string(), items }).await?;
        let reply = tokio::select! {
            r = timeout(OFFER_TIMEOUT, read_msg(&mut s)) => r.context("no answer from the other device")??,
            _ = cancel.cancelled() => bail!(transfer::Cancelled),
        };
        let Some(resume) = resume_from_reply(reply)? else { return Ok(false) };
        let already = resume.already_have(&locals);
        self.update_transfer(id, |t| {
            t.status = TransferStatus::Active;
            t.done_bytes = already;
        });
        transfer::send_files(&mut s, &locals, &resume, cancel, self.progress(id)).await?;
        Ok(true)
    }

    /// Reuse what an earlier attempt of the same transfer already received, if anything.
    fn receive_plan(&self, peer_id: &str, transfer_id: &str, items: &[Item]) -> Result<ReceivePlan> {
        if self.inner.receiving.lock().unwrap().contains(transfer_id) {
            bail!("The previous attempt is still finishing. Try again in a moment.");
        }
        if let Some((peer, plan)) = self.inner.partials.lock().unwrap().remove(transfer_id) {
            if peer == peer_id && plan.items == items {
                return Ok(plan);
            }
            plan.discard();
        }
        ReceivePlan::new(&self.settings().receive_dir, items.to_vec(), transfer_id)
    }

    /// Receive into `plan`, keeping partial files for a retry unless the user cancelled.
    async fn receive_into(&self, s: &mut Stream, peer_id: &str, transfer_id: &str, local_id: &str, mut plan: ReceivePlan) {
        let cancel = self.register_cancel(local_id);
        self.inner.receiving.lock().unwrap().insert(transfer_id.to_string());
        let result = transfer::receive_files(s, &mut plan, &cancel, self.progress(local_id)).await;
        self.drop_cancel(local_id);
        match &result {
            Err(e) if !transfer::is_cancelled(e) => {
                self.inner.partials.lock().unwrap().insert(transfer_id.to_string(), (peer_id.to_string(), plan));
            }
            Err(_) => plan.discard(),
            Ok(_) => {}
        }
        self.inner.receiving.lock().unwrap().remove(transfer_id);
        self.finish_transfer(local_id, result.map(|tops| tops.into_iter().next()));
    }

    async fn receive_offer(&self, mut s: Stream, peer_id: String, hello: Hello, remote_id: String, items: Vec<Item>) -> Result<()> {
        if items.is_empty() || items.len() > MAX_ITEMS {
            return write_msg(&mut s, &Msg::error("invalid transfer")).await;
        }
        let t = self.new_transfer(None, &peer_id, Direction::Receive, &items, TransferStatus::Waiting);
        let accept = if self.settings().ask_before_receiving {
            let (tx, rx) = oneshot::channel();
            self.inner.offers.lock().unwrap().insert(t.id.clone(), tx);
            self.emit(Event::IncomingOffer {
                transfer_id: t.id.clone(),
                device_name: hello.device_name.clone(),
                label: t.label.clone(),
                file_count: t.file_count,
                total_bytes: t.total_bytes,
            });
            let answer = timeout(OFFER_TIMEOUT, rx).await.ok().and_then(|r| r.ok()).unwrap_or(false);
            self.inner.offers.lock().unwrap().remove(&t.id);
            answer
        } else {
            true
        };
        if !accept {
            write_msg(&mut s, &Msg::OfferReply { accept: false, skip: vec![], resume: vec![] }).await?;
            self.update_transfer(&t.id, |t| t.status = TransferStatus::Declined);
            return Ok(());
        }
        let plan = match self.receive_plan(&peer_id, &remote_id, &items) {
            Ok(p) => p,
            Err(e) => {
                write_msg(&mut s, &Msg::error(format!("{e:#}"))).await?;
                self.finish_transfer(&t.id, Err(e));
                return Ok(());
            }
        };
        let (reply, already) = reply_for(&plan);
        write_msg(&mut s, &reply).await?;
        self.update_transfer(&t.id, |t| {
            t.status = TransferStatus::Active;
            t.done_bytes = already;
        });
        self.receive_into(&mut s, &peer_id, &remote_id, &t.id, plan).await;
        Ok(())
    }

    pub fn respond_offer(&self, transfer_id: &str, accept: bool) {
        if let Some(tx) = self.inner.offers.lock().unwrap().remove(transfer_id) {
            let _ = tx.send(accept);
        }
    }

    // ---------- Browsing and pulling from the other device's shared folder ----------

    /// A path inside the sync folder (used by sync, which may include dot-files).
    pub(crate) fn resolve_sync(&self, wire: &str) -> Result<PathBuf> {
        let root = self.settings().sync_dir;
        std::fs::create_dir_all(&root)?;
        resolve_within(&root.canonicalize()?, wire, false, self.data_dir())
    }

    /// What the top of a browse listing is called, or an error if browsing is off.
    fn browse_label(&self) -> Result<String> {
        let s = self.settings();
        match s.browse_scope {
            BrowseScope::Computer => Ok(s.device_name),
            BrowseScope::Home => Ok("Home".into()),
            BrowseScope::SyncFolder => Ok("Sync folder".into()),
            BrowseScope::Nobody => bail!(
                "{0} doesn't let other devices browse its files. You can change this in Settings on {0}.",
                s.device_name
            ),
        }
    }

    /// Turn a browse path from another device into a local path, per the browse setting.
    fn resolve_browse(&self, wire: &str) -> Result<PathBuf> {
        let s = self.settings();
        let wire = wire.trim_matches('/');
        let (root, rest) = match s.browse_scope {
            BrowseScope::Computer => {
                let (first, rest) = wire.split_once('/').unwrap_or((wire, ""));
                let place = places().into_iter().find(|p| p.id == first).context("that location isn't available")?;
                (place.root, rest)
            }
            BrowseScope::Home => (dirs::home_dir().context("no home folder")?, wire),
            BrowseScope::SyncFolder => {
                std::fs::create_dir_all(&s.sync_dir)?;
                (s.sync_dir, wire)
            }
            BrowseScope::Nobody => bail!(self.browse_label().unwrap_err()),
        };
        resolve_within(&root.canonicalize()?, rest, true, self.data_dir())
    }

    async fn serve_listing(&self, mut s: Stream, wire: String) -> Result<()> {
        let listing = || -> Result<(String, Vec<RemoteEntry>)> {
            let label = self.browse_label()?;
            if self.settings().browse_scope == BrowseScope::Computer && wire.trim_matches('/').is_empty() {
                let entries = places()
                    .into_iter()
                    .map(|p| RemoteEntry { name: p.label, path: p.id, dir: true, size: 0, modified: None, kind: p.kind.into() })
                    .collect();
                return Ok((label, entries));
            }
            Ok((label, list_dir(&self.resolve_browse(&wire)?, &wire)?))
        };
        let reply = match listing() {
            Ok((root, entries)) => Msg::Listing { path: wire, entries, root },
            Err(e) => Msg::error(format!("{e:#}")),
        };
        write_msg(&mut s, &reply).await
    }

    pub async fn list_remote(&self, device_id: &str, path: &str) -> Result<Listing> {
        let (mut s, _) = self.connect(device_id).await?;
        write_msg(&mut s, &Msg::List { path: path.to_string() }).await?;
        match timeout(HANDSHAKE_TIMEOUT, read_msg(&mut s)).await.context("timed out")?? {
            Msg::Listing { path, entries, root } => Ok(Listing { path, entries, root }),
            other => Err(other.unexpected()),
        }
    }

    async fn serve_pull(&self, mut s: Stream, peer_id: String, _hello: Hello, wires: Vec<String>, requested_id: Option<String>) -> Result<()> {
        let whole_place = self.settings().browse_scope == BrowseScope::Computer && wires.iter().any(|w| !w.trim_matches('/').contains('/'));
        let selection: Result<Vec<PathBuf>> = if whole_place {
            Err(anyhow!("Copying a whole drive or home folder isn't supported. Open it and choose the folders you need."))
        } else {
            wires.iter().map(|w| self.resolve_browse(w)).collect()
        };
        let rules = paths::CollectRules { skip_hidden: true, exclude: self.data_dir().canonicalize().ok() };
        let locals = match selection {
            Ok(sel) => tokio::task::spawn_blocking(move || paths::collect(&sel, &rules)).await?,
            Err(e) => Err(e),
        };
        let locals = match locals {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => return write_msg(&mut s, &Msg::error("Nothing to send.")).await,
            Err(e) => return write_msg(&mut s, &Msg::error(format!("{e:#}"))).await,
        };
        let items: Vec<Item> = locals.iter().map(|l| l.item.clone()).collect();
        let id = requested_id.filter(|id| id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        let t = self.new_transfer(id, &peer_id, Direction::Send, &items, TransferStatus::Waiting);
        write_msg(&mut s, &Msg::Offer { transfer_id: t.id.clone(), items }).await?;
        let cancel = self.register_cancel(&t.id);
        let result = async {
            let reply = timeout(HANDSHAKE_TIMEOUT, read_msg(&mut s)).await.context("timed out")??;
            let Some(resume) = resume_from_reply(reply)? else { return Ok(false) };
            let already = resume.already_have(&locals);
            self.update_transfer(&t.id, |t| {
                t.status = TransferStatus::Active;
                t.done_bytes = already;
            });
            transfer::send_files(&mut s, &locals, &resume, &cancel, self.progress(&t.id)).await?;
            Ok(true)
        }
        .await;
        self.drop_cancel(&t.id);
        self.finish_send(&t.id, result);
        Ok(())
    }

    /// Copy items from a paired device to this device's receive folder.
    pub async fn pull(&self, device_id: &str, wires: Vec<String>) -> Result<String> {
        self.start_pull(device_id, wires, None).await
    }

    async fn start_pull(&self, device_id: &str, wires: Vec<String>, retry_id: Option<String>) -> Result<String> {
        if !self.is_paired(device_id) {
            bail!("Pair with this device first.");
        }
        let id = retry_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let (mut s, _) = self.connect(device_id).await?;
        write_msg(&mut s, &Msg::Pull { paths: wires.clone(), transfer_id: Some(id.clone()) }).await?;
        let items = match timeout(OFFER_TIMEOUT, read_msg(&mut s)).await.context("timed out")?? {
            Msg::Offer { items, .. } if !items.is_empty() && items.len() <= MAX_ITEMS => items,
            other => return Err(other.unexpected()),
        };
        self.new_transfer(Some(id.clone()), device_id, Direction::Receive, &items, TransferStatus::Active);
        self.inner.retries.lock().unwrap().insert(id.clone(), RetryKind::Pull { device_id: device_id.to_string(), wires });
        let plan = self.receive_plan(device_id, &id, &items)?;
        let (reply, already) = reply_for(&plan);
        write_msg(&mut s, &reply).await?;
        self.update_transfer(&id, |t| t.done_bytes = already);
        let (engine, device_id, transfer_id) = (self.clone(), device_id.to_string(), id.clone());
        tokio::spawn(async move {
            engine.receive_into(&mut s, &device_id, &transfer_id, &transfer_id, plan).await;
        });
        Ok(id)
    }

    pub fn clear_finished_transfers(&self) {
        let mut list = self.inner.transfers.lock().unwrap();
        list.retain(|t| matches!(t.status, TransferStatus::Waiting | TransferStatus::Active));
        let kept: std::collections::HashSet<_> = list.iter().map(|t| t.id.clone()).collect();
        self.inner.retries.lock().unwrap().retain(|id, _| kept.contains(id));
    }
}

// ---------- Helpers ----------

/// Interpret the receiver's answer to an offer: `None` if declined.
fn resume_from_reply(reply: Msg) -> Result<Option<ResumeInfo>> {
    match reply {
        Msg::OfferReply { accept: false, .. } => Ok(None),
        Msg::OfferReply { accept: true, skip, resume } => Ok(Some(ResumeInfo {
            skip: skip.into_iter().map(|i| i as usize).collect(),
            offsets: resume.into_iter().map(|(i, o)| (i as usize, o)).collect(),
        })),
        other => Err(other.unexpected()),
    }
}

/// The accepting reply for a receive plan, and how many bytes are already here.
fn reply_for(plan: &ReceivePlan) -> (Msg, u64) {
    let info = plan.resume_info();
    let already = info.skip.iter().map(|i| plan.items[*i].size).sum::<u64>() + info.offsets.values().sum::<u64>();
    let mut skip: Vec<u32> = info.skip.iter().map(|i| *i as u32).collect();
    skip.sort();
    let resume = info.offsets.iter().map(|(i, o)| (*i as u32, *o)).collect();
    (Msg::OfferReply { accept: true, skip, resume }, already)
}

fn check_protocol(hello: &Hello) -> Result<()> {
    if hello.protocol != PROTOCOL_VERSION {
        bail!("The other device runs a different version of The Tunnel. Update both devices.");
    }
    Ok(())
}

/// A top-level location offered when browsing the whole computer.
struct Place {
    /// Used as the first path component on the wire.
    id: String,
    label: String,
    root: PathBuf,
    kind: &'static str,
}

/// The home folder plus every drive (C:, D:, … on Windows; the startup disk and
/// mounted volumes on macOS).
fn places() -> Vec<Place> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(Place { id: "Home".into(), label: "Home".into(), root: home, kind: "home" });
    }
    #[cfg(windows)]
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if std::fs::metadata(&root).is_ok() {
            let l = letter as char;
            out.push(Place { id: format!("Drive {l}"), label: format!("{l}: drive"), root, kind: "drive" });
        }
    }
    #[cfg(target_os = "macos")]
    {
        let root = PathBuf::from("/");
        let volumes: Vec<(String, PathBuf)> = std::fs::read_dir("/Volumes")
            .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path()))).collect())
            .unwrap_or_default();
        // The startup disk appears in /Volumes as a link back to "/".
        let startup = volumes.iter().find(|(_, p)| p.canonicalize().ok().as_deref() == Some(root.as_path())).map(|(n, _)| n.clone());
        let name = startup.unwrap_or_else(|| "Macintosh HD".into());
        out.push(Place { id: name.clone(), label: name, root: root.clone(), kind: "drive" });
        for (name, path) in volumes {
            if name.starts_with('.') || path.canonicalize().ok().as_deref() == Some(root.as_path()) {
                continue;
            }
            out.push(Place { id: name.clone(), label: name, root: path, kind: "drive" });
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    out.push(Place { id: "Computer".into(), label: "Computer".into(), root: PathBuf::from("/"), kind: "drive" });

    // Ids must be unique and usable as a single path component.
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| paths::requested_relative(&p.id).is_ok_and(|r| r.components().count() == 1) && seen.insert(p.id.clone()));
    out
}

fn list_dir(dir: &Path, wire: &str) -> Result<Vec<RemoteEntry>> {
    let prefix = wire.trim_matches('/');
    // Hide Library / AppData in this user's home folder and in any other home folder.
    let in_home = dirs::home_dir().and_then(|h| h.canonicalize().ok()).is_some_and(|h| h == dir) || dir.parent().is_some_and(|p| p.ends_with("Users"));
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if paths::is_ignored(&name) || (in_home && paths::is_private_top_level(&name)) {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() && !ft.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if paths::is_hidden(&name, &meta) {
            continue;
        }
        entries.push(RemoteEntry {
            path: if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") },
            name,
            dir: ft.is_dir(),
            size: if ft.is_dir() { 0 } else { meta.len() },
            modified: paths::modified_secs(&meta),
            kind: String::new(),
        });
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

/// Resolve a requested path inside `root`, refusing anything that escapes it,
/// The Tunnel's own data, and (when `visible_only`) hidden or system items.
fn resolve_within(root: &Path, wire: &str, visible_only: bool, data_dir: &Path) -> Result<PathBuf> {
    let rel = paths::requested_relative(wire)?;
    if visible_only {
        let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
        let mut cur = root.to_path_buf();
        for comp in rel.components() {
            let name = comp.as_os_str().to_string_lossy();
            // Library / AppData inside any home folder hold app data and passwords.
            let in_home = home.as_deref() == Some(cur.as_path()) || cur.parent().is_some_and(|p| p.ends_with("Users"));
            cur.push(comp);
            let hidden = std::fs::symlink_metadata(&cur).is_ok_and(|m| paths::is_hidden(&name, &m));
            if hidden || (in_home && paths::is_private_top_level(&name)) {
                bail!("that item isn't available");
            }
        }
    }
    let full = root.join(rel).canonicalize().context("that item no longer exists")?;
    if !full.starts_with(root) {
        bail!("that item is outside the folder you can browse");
    }
    if let Ok(data) = data_dir.canonicalize()
        && full.starts_with(&data) {
            bail!("that item isn't available");
        }
    Ok(full)
}

/// A sync folder must be an ordinary folder: not a whole drive, not the entire
/// home folder, and not somewhere that contains The Tunnel's own data.
fn validate_sync_dir(dir: &Path, data_dir: &Path) -> Result<()> {
    if !dir.is_absolute() {
        bail!("Choose a full folder path.");
    }
    if dir.parent().is_none_or(|p| p.as_os_str().is_empty()) {
        bail!("Choose a folder, not a whole drive.");
    }
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let dir = canon(dir);
    if dirs::home_dir().is_some_and(|h| canon(&h) == dir) {
        bail!("Choose a folder inside your home folder, not the whole home folder.");
    }
    if canon(data_dir).starts_with(&dir) {
        bail!("That folder contains The Tunnel's own settings. Choose another one.");
    }
    Ok(())
}

fn label_for(items: &[Item]) -> String {
    let tops: Vec<&str> = items.iter().filter(|i| !i.path.contains('/')).map(|i| i.path.as_str()).collect();
    match tops.as_slice() {
        [] => "Files".into(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    }
}

fn clean_name(raw: &str, max: usize) -> Result<String> {
    let name: String = raw.trim().chars().filter(|c| !c.is_control()).take(max).collect();
    if name.is_empty() {
        bail!("Name can't be empty.");
    }
    Ok(name)
}

fn parse_address(input: &str) -> Result<SocketAddr> {
    let input = input.trim();
    if let Ok(addr) = input.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let ip: IpAddr = input.parse().map_err(|_| anyhow!("Enter an address like 192.168.1.5"))?;
    Ok(SocketAddr::new(ip, DEFAULT_PORT))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_folder_must_be_an_ordinary_folder() {
        let home = dirs::home_dir().unwrap();
        let data = home.join("Library/Application Support/com.thetunnel.app");
        assert!(validate_sync_dir(&home.join("Downloads/The Tunnel"), &data).is_ok());
        assert!(validate_sync_dir(&home, &data).is_err(), "whole home folder");
        assert!(validate_sync_dir(Path::new("/"), &data).is_err(), "whole drive");
        assert!(validate_sync_dir(&home.join("Library"), &data).is_err(), "contains app data");
        assert!(validate_sync_dir(Path::new("relative/path"), &data).is_err());
    }
}
