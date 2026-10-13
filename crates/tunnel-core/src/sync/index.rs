//! The index: what this device knows about every file in the synced folder.
//!
//! Each entry carries a version vector: a counter per device that changed the
//! file. Comparing two vectors tells whether one copy is strictly newer than
//! the other or whether both devices changed it independently (a conflict).
//! Deleted files stay in the index as tombstones so deletions can be reviewed
//! on the other device instead of being lost or silently resurrected.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths;

pub type Version = BTreeMap<String, u64>;

/// Folder inside the synced folder that holds partial downloads. Never synced.
pub const META_DIR: &str = ".thetunnel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Equal,
    /// The left version includes everything in the right one, and more.
    Newer,
    Older,
    Concurrent,
}

pub fn compare(a: &Version, b: &Version) -> Order {
    let (mut newer, mut older) = (false, false);
    for key in a.keys().chain(b.keys()) {
        let (x, y) = (a.get(key).copied().unwrap_or(0), b.get(key).copied().unwrap_or(0));
        newer |= x > y;
        older |= x < y;
    }
    match (newer, older) {
        (false, false) => Order::Equal,
        (true, false) => Order::Newer,
        (false, true) => Order::Older,
        (true, true) => Order::Concurrent,
    }
}

pub fn merge(a: &Version, b: &Version) -> Version {
    let mut out = a.clone();
    for (k, v) in b {
        let e = out.entry(k.clone()).or_insert(0);
        *e = (*e).max(*v);
    }
    out
}

pub fn bump(v: &Version, me: &str) -> Version {
    let mut out = v.clone();
    *out.entry(me.to_string()).or_insert(0) += 1;
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub dir: bool,
    pub size: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub mtime: i64,
    /// SHA-256 of the contents; kept on tombstones so renames can be recognised.
    pub hash: String,
    pub deleted: bool,
    pub version: Version,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    /// Random per index file, so peers notice if this device starts over.
    pub index_id: String,
    /// Increases whenever anything in the index changes.
    pub seq: u64,
    pub entries: BTreeMap<String, Entry>,
}

impl Index {
    pub fn load(path: &Path) -> Index {
        let mut index: Index = crate::store::load_json(path);
        if index.index_id.is_empty() {
            index.index_id = uuid::Uuid::new_v4().to_string();
        }
        index
    }
}

pub fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub fn wire_to_path(root: &Path, wire: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in wire.split('/') {
        p.push(part);
    }
    p
}

struct Found {
    dir: bool,
    size: u64,
    mtime: i64,
}

pub struct ScanOutcome {
    /// Something other than timestamps changed, so peers should be told.
    pub changed: bool,
    /// Only timestamps changed: worth saving, not worth announcing.
    pub touched: bool,
    pub issues: Vec<String>,
}

/// Walk the folder and bring the index up to date. The caller must hold the
/// folder guard so this never races with a sync placing files.
pub fn scan(root: &Path, index: &std::sync::Mutex<Index>, me: &str) -> Result<ScanOutcome> {
    fs::create_dir_all(root)?;
    let mut found = BTreeMap::new();
    let mut issues = Vec::new();
    walk(root, "", &mut found, &mut issues)?;

    let snapshot = index.lock().unwrap().entries.clone();

    // Hash new or modified files outside the index lock.
    let mut hashes: HashMap<String, String> = HashMap::new();
    for (wire, f) in &found {
        if f.dir {
            continue;
        }
        let unchanged = snapshot
            .get(wire)
            .is_some_and(|e| !e.deleted && !e.dir && e.size == f.size && e.mtime == f.mtime);
        if !unchanged {
            match hash_file(&wire_to_path(root, wire)) {
                Ok(h) => {
                    hashes.insert(wire.clone(), h);
                }
                Err(e) => issues.push(format!("Couldn't read {wire}: {e}")),
            }
        }
    }

    let mut idx = index.lock().unwrap();
    let mut changed = false;
    let mut touched = false;
    for (wire, f) in &found {
        let existing = idx.entries.get(wire).cloned();
        if f.dir {
            if existing.as_ref().is_some_and(|e| e.dir && !e.deleted) {
                continue;
            }
            let version = bump(&existing.map(|e| e.version).unwrap_or_default(), me);
            idx.entries.insert(wire.clone(), Entry { path: wire.clone(), dir: true, size: 0, mtime: 0, hash: String::new(), deleted: false, version });
            changed = true;
            continue;
        }
        let Some(hash) = hashes.remove(wire) else { continue };
        match existing {
            Some(e) if !e.deleted && !e.dir && e.hash == hash => {
                // Only the timestamp moved (e.g. a file was touched or re-saved unchanged).
                let e = idx.entries.get_mut(wire).unwrap();
                e.size = f.size;
                e.mtime = f.mtime;
                touched = true;
            }
            other => {
                let version = bump(&other.map(|e| e.version).unwrap_or_default(), me);
                idx.entries.insert(wire.clone(), Entry { path: wire.clone(), dir: false, size: f.size, mtime: f.mtime, hash, deleted: false, version });
                changed = true;
            }
        }
    }
    let gone: Vec<String> = idx.entries.values().filter(|e| !e.deleted && !found.contains_key(&e.path)).map(|e| e.path.clone()).collect();
    for wire in gone {
        let e = idx.entries.get_mut(&wire).unwrap();
        e.deleted = true;
        e.version = bump(&e.version, me);
        changed = true;
    }
    if changed {
        idx.seq += 1;
    }
    Ok(ScanOutcome { changed, touched, issues })
}

fn walk(dir: &Path, prefix: &str, found: &mut BTreeMap<String, Found>, issues: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            issues.push(format!("Skipped a file with an unsupported name in {}", if prefix.is_empty() { "the folder" } else { prefix }));
            continue;
        };
        if paths::is_ignored(&name) || (prefix.is_empty() && name == META_DIR) {
            continue;
        }
        let wire = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        // A name Windows can't store would never arrive intact, so leave it out and say so.
        if paths::sanitize_component(&name).ok().as_deref() != Some(name.as_str()) {
            issues.push(format!("“{wire}” isn't synced: its name isn't allowed on Windows."));
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            found.insert(wire.clone(), Found { dir: true, size: 0, mtime: 0 });
            walk(&entry.path(), &wire, found, issues)?;
        } else if ft.is_file() {
            let Ok(meta) = entry.metadata() else { continue };
            found.insert(wire, Found { dir: false, size: meta.len(), mtime: mtime_ms(&meta) });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pairs: &[(&str, u64)]) -> Version {
        pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    #[test]
    fn version_ordering() {
        assert_eq!(compare(&v(&[("a", 1)]), &v(&[("a", 1)])), Order::Equal);
        assert_eq!(compare(&v(&[("a", 2)]), &v(&[("a", 1)])), Order::Newer);
        assert_eq!(compare(&v(&[("a", 1)]), &v(&[("a", 1), ("b", 1)])), Order::Older);
        assert_eq!(compare(&v(&[("a", 2)]), &v(&[("a", 1), ("b", 1)])), Order::Concurrent);
        assert_eq!(merge(&v(&[("a", 2)]), &v(&[("a", 1), ("b", 3)])), v(&[("a", 2), ("b", 3)]));
    }

    #[test]
    fn scan_tracks_adds_edits_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let index = std::sync::Mutex::new(Index::default());
        fs::write(root.join("a.txt"), "one").unwrap();
        fs::create_dir(root.join("docs")).unwrap();
        fs::write(root.join(".DS_Store"), "x").unwrap();
        assert!(scan(root, &index, "me").unwrap().changed);
        {
            let idx = index.lock().unwrap();
            assert_eq!(idx.entries.len(), 2);
            assert_eq!(idx.entries["a.txt"].version, v(&[("me", 1)]));
        }
        assert!(!scan(root, &index, "me").unwrap().changed, "rescan without changes");

        fs::write(root.join("a.txt"), "two!").unwrap();
        scan(root, &index, "me").unwrap();
        assert_eq!(index.lock().unwrap().entries["a.txt"].version, v(&[("me", 2)]));

        fs::remove_file(root.join("a.txt")).unwrap();
        scan(root, &index, "me").unwrap();
        let idx = index.lock().unwrap();
        assert!(idx.entries["a.txt"].deleted);
        assert_eq!(idx.entries["a.txt"].version, v(&[("me", 3)]));
        assert!(!idx.entries["a.txt"].hash.is_empty(), "tombstone keeps the hash");
    }
}
