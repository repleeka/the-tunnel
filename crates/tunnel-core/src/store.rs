//! Settings and the list of paired devices, persisted as JSON in the app data folder.

use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// What paired devices may browse and copy from this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowseScope {
    /// Every drive plus the home folder, minus hidden and system folders.
    #[default]
    Computer,
    /// The home folder, minus hidden and system folders.
    Home,
    SyncFolder,
    Nobody,
}

/// Bumped when defaults change, so settings saved by older versions can be migrated.
pub const SETTINGS_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub onboarded: bool,
    pub device_name: String,
    pub profile_name: String,
    /// Where files sent to this device are saved.
    pub receive_dir: PathBuf,
    /// The folder kept identical on all paired devices.
    #[serde(alias = "sharedDir")]
    pub sync_dir: PathBuf,
    pub ask_before_receiving: bool,
    pub sync_enabled: bool,
    pub browse_scope: BrowseScope,
    #[serde(default)]
    pub settings_version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
        Self {
            onboarded: false,
            device_name: whoami::devicename().unwrap_or_else(|_| "My computer".into()),
            profile_name: whoami::realname().unwrap_or_default(),
            receive_dir: downloads.clone(),
            sync_dir: downloads.join("The Tunnel"),
            ask_before_receiving: false,
            sync_enabled: true,
            browse_scope: BrowseScope::Home,
            settings_version: SETTINGS_VERSION,
        }
    }
}

impl Settings {
    /// Move settings saved by older versions to the current defaults, but only
    /// where the user never changed them. Returns true if anything changed.
    pub fn migrate(&mut self) -> bool {
        if self.settings_version >= SETTINGS_VERSION {
            return false;
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
        // Version 1: received files go straight to Downloads, and the sync folder lives in Downloads.
        if self.settings_version < 1 {
            if self.receive_dir == downloads.join("The Tunnel") {
                self.receive_dir = downloads.clone();
            }
            if self.sync_dir == home.join("The Tunnel") {
                self.sync_dir = downloads.join("The Tunnel");
            }
        }
        // Version 2: the default became browsing the whole computer, not just the home folder.
        if self.settings_version < 2 && self.browse_scope == BrowseScope::Home {
            self.browse_scope = BrowseScope::Computer;
        }
        self.settings_version = SETTINGS_VERSION;
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub device_id: String,
    pub device_name: String,
    pub profile_name: String,
    pub os: String,
    pub paired_at: u64,
    #[serde(default)]
    pub last_addrs: Vec<SocketAddr>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    pub devices: BTreeMap<String, PairedDevice>,
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Write via a temp file and rename so a crash never leaves a half-written file.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_defaults_move_to_downloads_but_custom_choices_stay() {
        let home = dirs::home_dir().unwrap();
        let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
        let mut old: Settings = serde_json::from_value(serde_json::json!({
            "onboarded": true,
            "receiveDir": downloads.join("The Tunnel"),
            "sharedDir": home.join("The Tunnel"),
        }))
        .unwrap();
        assert_eq!(old.settings_version, 0);
        assert!(old.migrate());
        assert_eq!(old.receive_dir, downloads);
        assert_eq!(old.sync_dir, downloads.join("The Tunnel"));
        assert_eq!(old.browse_scope, BrowseScope::Computer);
        assert!(!old.migrate(), "runs once");

        let mut custom: Settings = serde_json::from_value(serde_json::json!({ "sharedDir": "/Volumes/Work/Sync" })).unwrap();
        custom.migrate();
        assert_eq!(custom.sync_dir, PathBuf::from("/Volumes/Work/Sync"));
        assert_eq!(Settings::default().settings_version, SETTINGS_VERSION);
    }
}
