//! The Tunnel's engine: discovery, pairing and file transfer between devices
//! on the same local network. Shared by the desktop app and, later, mobile.

mod discovery;
mod engine;
mod identity;
mod net;
mod pairing;
mod paths;
mod protocol;
mod store;
mod sync;
mod tls;
mod transfer;

pub use engine::{DeviceView, Direction, Engine, EngineConfig, Event, Listing, Me, SettingsPatch, Snapshot, TransferInfo, TransferStatus};
pub use protocol::RemoteEntry;
pub use store::{BrowseScope, Settings};
pub use sync::{ChangeKind, PeerSync, PeerSyncState, PendingDeletion, SyncChange, SyncStatus};
