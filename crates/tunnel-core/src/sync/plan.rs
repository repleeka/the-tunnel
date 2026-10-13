//! Deciding what to do with the other device's index. Pure logic, no I/O.
//!
//! Each device only ever changes its own folder: it pulls what it's missing,
//! and the other device does the same in reverse. Rules:
//! - The other side's copy is strictly newer → take it.
//! - Both changed the same file independently → the later edit keeps the name;
//!   the device holding the earlier edit saves it as a conflict copy.
//! - The other side deleted a file → never delete automatically; ask the user.
//! - A delete plus an add with the same contents is a rename → move locally.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::index::{Entry, Order, compare};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Create a folder that exists on the other device.
    Mkdir { remote: Entry },
    /// Fetch a file. `conflict` means the local copy must first be kept as a conflict copy.
    Download { remote: Entry, conflict: bool },
    /// The contents already exist locally under another name: copy them.
    CopyLocal { from: String, remote: Entry },
    /// The other device renamed `from` to `remote.path`: do the same here.
    MoveLocal { from: String, tombstone: Entry, remote: Entry },
    /// Same contents (or both deleted): just record the other device's version.
    Adopt { remote: Entry },
    /// The other device deleted this file: ask before deleting it here.
    ReviewDeletion { remote: Entry },
    /// The other device deleted this folder: remove it here if it's empty.
    RemoveEmptyDir { remote: Entry },
    /// Something that can't be resolved automatically.
    Skip { path: String, reason: String },
}

pub fn plan(local: &BTreeMap<String, Entry>, remote: &BTreeMap<String, Entry>) -> Vec<Action> {
    let mut by_hash: HashMap<&str, &str> = HashMap::new();
    for e in local.values() {
        if !e.deleted && !e.dir && !e.hash.is_empty() {
            by_hash.entry(e.hash.as_str()).or_insert(e.path.as_str());
        }
    }

    let mut actions = Vec::new();
    let mut deletions = Vec::new();
    let mut moved: HashSet<String> = HashSet::new();

    let fetch = |r: &Entry, conflict: bool, actions: &mut Vec<Action>, moved: &mut HashSet<String>| {
        if !conflict
            && let Some(&src) = by_hash.get(r.hash.as_str())
                && src != r.path {
                    let renamed_away = remote.get(src).filter(|t| {
                        t.deleted && !moved.contains(src) && local.get(src).is_some_and(|l| compare(&l.version, &t.version) == Order::Older)
                    });
                    actions.push(match renamed_away {
                        Some(t) => {
                            moved.insert(src.to_string());
                            Action::MoveLocal { from: src.to_string(), tombstone: t.clone(), remote: r.clone() }
                        }
                        None => Action::CopyLocal { from: src.to_string(), remote: r.clone() },
                    });
                    return;
                }
        actions.push(Action::Download { remote: r.clone(), conflict });
    };

    for (path, r) in remote {
        let Some(l) = local.get(path) else {
            if !r.deleted {
                if r.dir {
                    actions.push(Action::Mkdir { remote: r.clone() });
                } else {
                    fetch(r, false, &mut actions, &mut moved);
                }
            }
            continue;
        };

        let type_clash = !l.deleted && !r.deleted && l.dir != r.dir;
        if type_clash {
            actions.push(Action::Skip { path: path.clone(), reason: "it's a file on one device and a folder on the other".into() });
            continue;
        }

        match compare(&l.version, &r.version) {
            Order::Equal | Order::Newer => {}
            Order::Older => match (l.deleted, r.deleted) {
                (true, true) => actions.push(Action::Adopt { remote: r.clone() }),
                (true, false) if r.dir => actions.push(Action::Mkdir { remote: r.clone() }),
                (true, false) => fetch(r, false, &mut actions, &mut moved),
                (false, true) if l.dir => actions.push(Action::RemoveEmptyDir { remote: r.clone() }),
                (false, true) => deletions.push(r.clone()),
                (false, false) if l.dir || l.hash == r.hash => actions.push(Action::Adopt { remote: r.clone() }),
                (false, false) => fetch(r, false, &mut actions, &mut moved),
            },
            Order::Concurrent => match (l.deleted, r.deleted) {
                (true, true) => actions.push(Action::Adopt { remote: r.clone() }),
                // A change beats a delete: keep ours, and the other device restores it.
                (false, true) => {}
                (true, false) if r.dir => actions.push(Action::Mkdir { remote: r.clone() }),
                (true, false) => fetch(r, false, &mut actions, &mut moved),
                (false, false) if l.dir || l.hash == r.hash => actions.push(Action::Adopt { remote: r.clone() }),
                (false, false) => {
                    // Both edited it. The later edit wins (ties broken by content hash, so
                    // both devices pick the same winner).
                    if (r.mtime, &r.hash) > (l.mtime, &l.hash) {
                        fetch(r, true, &mut actions, &mut moved);
                    }
                }
            },
        }
    }

    for r in deletions {
        if !moved.contains(&r.path) {
            actions.push(Action::ReviewDeletion { remote: r });
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::index::Version;

    fn v(pairs: &[(&str, u64)]) -> Version {
        pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    fn file(path: &str, hash: &str, mtime: i64, version: Version) -> Entry {
        Entry { path: path.into(), dir: false, size: 1, mtime, hash: hash.into(), deleted: false, version }
    }

    fn deleted(mut e: Entry, version: Version) -> Entry {
        e.deleted = true;
        e.version = version;
        e
    }

    fn map(entries: Vec<Entry>) -> BTreeMap<String, Entry> {
        entries.into_iter().map(|e| (e.path.clone(), e)).collect()
    }

    #[test]
    fn new_and_newer_files_are_fetched() {
        let local = map(vec![file("a", "h1", 1, v(&[("mac", 1)]))]);
        let remote = map(vec![file("a", "h2", 2, v(&[("mac", 1), ("pc", 1)])), file("b", "h3", 1, v(&[("pc", 1)]))]);
        let actions = plan(&local, &remote);
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[0], Action::Download { remote, conflict: false } if remote.path == "a"));
        assert!(matches!(&actions[1], Action::Download { remote, conflict: false } if remote.path == "b"));
    }

    #[test]
    fn older_or_equal_remote_does_nothing() {
        let local = map(vec![file("a", "h2", 2, v(&[("mac", 2)]))]);
        let remote = map(vec![file("a", "h1", 1, v(&[("mac", 1)]))]);
        assert!(plan(&local, &remote).is_empty());
        assert!(plan(&local, &local).is_empty());
    }

    #[test]
    fn deletions_need_review() {
        let a = file("a", "h1", 1, v(&[("mac", 1)]));
        let local = map(vec![a.clone()]);
        let remote = map(vec![deleted(a, v(&[("mac", 1), ("pc", 1)]))]);
        assert!(matches!(&plan(&local, &remote)[..], [Action::ReviewDeletion { remote }] if remote.path == "a"));
    }

    #[test]
    fn edit_beats_concurrent_delete() {
        let local = map(vec![file("a", "h2", 5, v(&[("mac", 2)]))]);
        let remote = map(vec![deleted(file("a", "h1", 1, v(&[])), v(&[("mac", 1), ("pc", 1)]))]);
        assert!(plan(&local, &remote).is_empty());
    }

    #[test]
    fn renames_become_local_moves() {
        let a = file("old.txt", "h1", 1, v(&[("mac", 1)]));
        let local = map(vec![a.clone()]);
        let remote = map(vec![deleted(a, v(&[("mac", 1), ("pc", 1)])), file("new.txt", "h1", 1, v(&[("pc", 1)]))]);
        let actions = plan(&local, &remote);
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert!(matches!(&actions[0], Action::MoveLocal { from, remote, .. } if from == "old.txt" && remote.path == "new.txt"));
    }

    #[test]
    fn duplicate_contents_are_copied_locally() {
        let local = map(vec![file("a", "h1", 1, v(&[("mac", 1)]))]);
        let remote = map(vec![file("a", "h1", 1, v(&[("mac", 1)])), file("copy", "h1", 1, v(&[("pc", 1)]))]);
        assert!(matches!(&plan(&local, &remote)[..], [Action::CopyLocal { from, .. }] if from == "a"));
    }

    #[test]
    fn conflicts_pick_the_same_winner_on_both_sides() {
        let mac = map(vec![file("a", "hm", 10, v(&[("mac", 2)]))]);
        let pc = map(vec![file("a", "hp", 20, v(&[("mac", 1), ("pc", 1)]))]);
        // The PC's edit is later: the Mac takes it (keeping its own as a conflict copy) ...
        assert!(matches!(&plan(&mac, &pc)[..], [Action::Download { conflict: true, .. }]));
        // ... and the PC keeps its own.
        assert!(plan(&pc, &mac).is_empty());
    }

    #[test]
    fn identical_concurrent_changes_just_merge() {
        let mac = map(vec![file("a", "same", 10, v(&[("mac", 1)]))]);
        let pc = map(vec![file("a", "same", 20, v(&[("pc", 1)]))]);
        assert!(matches!(&plan(&mac, &pc)[..], [Action::Adopt { .. }]));
    }
}
