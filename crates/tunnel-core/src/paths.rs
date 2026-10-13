//! Turning local files into transfer items, and incoming item paths back into
//! safe local paths. Incoming paths are never trusted: anything that could
//! escape the destination folder is rejected, and names Windows can't store
//! are rewritten so a Mac file name never breaks a transfer to a PC.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};

use crate::protocol::Item;

/// System clutter that should never be transferred.
const IGNORED: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini", ".localized", crate::sync::index::META_DIR];

pub const PARTIAL_SUFFIX: &str = ".tunnelpart";

pub struct LocalItem {
    pub item: Item,
    pub source: PathBuf,
}

/// Top-level folders in a home directory that hold app data and settings.
const PRIVATE_TOP_LEVEL: &[&str] = &["Library", "AppData", "Application Data", "Local Settings"];

/// Hidden or system items: dot-files, plus the hidden flag on macOS and the
/// hidden/system attributes on Windows (which cover ~/Library and AppData).
pub fn is_hidden(name: &str, meta: &fs::Metadata) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if meta.file_attributes() & (HIDDEN | SYSTEM) != 0 {
            return true;
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        if meta.st_flags() & UF_HIDDEN != 0 {
            return true;
        }
    }
    let _ = meta;
    false
}

pub fn is_private_top_level(name: &str) -> bool {
    PRIVATE_TOP_LEVEL.iter().any(|p| p.eq_ignore_ascii_case(name))
}

/// What to leave out when expanding a selection into files.
#[derive(Default)]
pub struct CollectRules {
    /// Skip hidden and system items (used when another device is copying from us).
    pub skip_hidden: bool,
    /// Never include anything inside this folder (The Tunnel's own data).
    pub exclude: Option<PathBuf>,
}

impl CollectRules {
    fn skips(&self, path: &Path, name: &str, meta: &fs::Metadata) -> bool {
        (self.skip_hidden && is_hidden(name, meta)) || self.exclude.as_ref().is_some_and(|x| path.starts_with(x))
    }
}

pub fn is_ignored(name: &str) -> bool {
    IGNORED.contains(&name) || name.ends_with(PARTIAL_SUFFIX)
}

pub fn modified_secs(meta: &fs::Metadata) -> Option<i64> {
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// Expand the user's selection into a flat list of files and folders.
/// Symlinks are skipped so a transfer can never wander outside what was chosen.
pub fn collect(selection: &[PathBuf], rules: &CollectRules) -> Result<Vec<LocalItem>> {
    let mut out = Vec::new();
    let mut top_names = HashSet::new();
    for path in selection {
        let meta = fs::symlink_metadata(path).with_context(|| format!("can't read {}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("unsupported file name: {}", path.display()))?;
        let name = unique_among(name, &mut top_names);
        if meta.is_dir() {
            out.push(LocalItem { item: Item { path: name.clone(), size: 0, dir: true, modified: modified_secs(&meta) }, source: path.clone() });
            walk(path, &name, rules, &mut out)?;
        } else if meta.is_file() {
            out.push(LocalItem { item: Item { path: name, size: meta.len(), dir: false, modified: modified_secs(&meta) }, source: path.clone() });
        }
    }
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, rules: &CollectRules, out: &mut Vec<LocalItem>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if is_ignored(&name) {
            continue;
        }
        let ft = entry.file_type()?;
        let wire = format!("{prefix}/{name}");
        let meta = entry.metadata()?;
        if rules.skips(&entry.path(), &name, &meta) {
            continue;
        }
        if ft.is_dir() {
            out.push(LocalItem { item: Item { path: wire.clone(), size: 0, dir: true, modified: modified_secs(&meta) }, source: entry.path() });
            walk(&entry.path(), &wire, rules, out)?;
        } else if ft.is_file() {
            out.push(LocalItem { item: Item { path: wire, size: meta.len(), dir: false, modified: modified_secs(&meta) }, source: entry.path() });
        }
    }
    Ok(())
}

fn unique_among(name: &str, taken: &mut HashSet<String>) -> String {
    let mut candidate = name.to_string();
    let mut n = 1;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = numbered(name, n);
        n += 1;
    }
    candidate
}

fn numbered(name: &str, n: usize) -> String {
    let p = Path::new(name);
    match (p.file_stem().and_then(|s| s.to_str()), p.extension().and_then(|s| s.to_str())) {
        (Some(stem), Some(ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

/// Make one received path component safe to create on any OS.
pub fn sanitize_component(raw: &str) -> Result<String> {
    if raw.is_empty() || raw == "." || raw == ".." {
        bail!("invalid path component {raw:?}");
    }
    let mut s: String = raw
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    // Windows silently drops trailing dots and spaces.
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() || s == "." || s == ".." {
        s = "_".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.insert(0, '_');
    }
    if s.len() > 255 {
        bail!("file name too long");
    }
    Ok(s)
}

/// Convert an incoming `/`-separated path into a safe relative path.
pub fn incoming_relative(wire: &str) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in wire.split('/') {
        out.push(sanitize_component(part)?);
    }
    if out.as_os_str().is_empty() {
        bail!("empty path");
    }
    Ok(out)
}

/// Strictly parse a path the other device asked for inside our shared folder.
/// Unlike incoming paths, nothing is rewritten: it must name something that exists.
pub fn requested_relative(wire: &str) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in wire.split('/').filter(|p| !p.is_empty()) {
        let mut comps = Path::new(part).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(c)), None) => out.push(c),
            _ => bail!("invalid path {wire:?}"),
        }
    }
    Ok(out)
}

/// Decide where each top-level item lands so nothing already there is overwritten:
/// "Photos" becomes "Photos (1)" if a "Photos" folder exists.
pub fn plan_destinations(dest_root: &Path, items: &[Item]) -> Result<Vec<PathBuf>> {
    let mut renamed: HashMap<String, String> = HashMap::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let rel = incoming_relative(&item.path)?;
        let mut comps = rel.components();
        let top = comps.next().and_then(|c| c.as_os_str().to_str()).unwrap_or("_").to_string();
        let rest: PathBuf = comps.collect();
        let mapped = renamed
            .entry(top.clone())
            .or_insert_with(|| free_name(dest_root, &top))
            .clone();
        let mut target = dest_root.join(mapped);
        if !rest.as_os_str().is_empty() {
            target.push(rest);
        }
        out.push(target);
    }
    Ok(out)
}

fn free_name(dir: &Path, name: &str) -> String {
    let taken = |n: &str| dir.join(n).exists() || dir.join(format!("{n}{PARTIAL_SUFFIX}")).exists();
    if !taken(name) {
        return name.to_string();
    }
    (1..).map(|n| numbered(name, n)).find(|n| !taken(n)).expect("unbounded search")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escapes() {
        assert!(incoming_relative("../etc/passwd").is_err());
        assert!(incoming_relative("a/../../b").is_err());
        assert!(incoming_relative("/abs").is_err());
        assert!(incoming_relative("").is_err());
        assert!(requested_relative("../x").is_err());
        assert!(requested_relative("a/./b").is_err());
    }

    #[test]
    fn rewrites_names_windows_cannot_store() {
        assert_eq!(sanitize_component("notes: draft?.txt").unwrap(), "notes_ draft_.txt");
        assert_eq!(sanitize_component("CON.txt").unwrap(), "_CON.txt");
        assert_eq!(sanitize_component("trailing. ").unwrap(), "trailing");
        assert_eq!(sanitize_component("..\\evil").unwrap(), ".._evil");
        assert_eq!(sanitize_component("Résumé.pdf").unwrap(), "Résumé.pdf");
    }

    #[test]
    fn avoids_overwriting_existing_items() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("Photos")).unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let items = vec![
            Item { path: "Photos".into(), size: 0, dir: true, modified: None },
            Item { path: "Photos/1.jpg".into(), size: 1, dir: false, modified: None },
            Item { path: "a.txt".into(), size: 1, dir: false, modified: None },
        ];
        let plan = plan_destinations(dir.path(), &items).unwrap();
        assert_eq!(plan[0], dir.path().join("Photos (1)"));
        assert_eq!(plan[1], dir.path().join("Photos (1)").join("1.jpg"));
        assert_eq!(plan[2], dir.path().join("a (1).txt"));
    }
}
