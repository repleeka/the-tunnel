//! Two engines keep their shared folders in sync.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{engine, pair};
use tunnel_core::{ChangeKind, Engine};

struct Pair {
    _root: tempfile::TempDir,
    mac: Engine,
    pc: Engine,
    mac_dir: PathBuf,
    pc_dir: PathBuf,
}

async fn setup() -> Pair {
    let root = tempfile::tempdir().unwrap();
    let mac = engine(root.path(), "Mac").await;
    let pc = engine(root.path(), "PC").await;
    pair(&mac, &pc).await;
    let (mac_dir, pc_dir) = (root.path().join("Mac/shared"), root.path().join("PC/shared"));
    Pair { _root: root, mac, pc, mac_dir, pc_dir }
}

impl Pair {
    /// A few rounds in each direction, like the background loop would do.
    /// (A change made while applying a round, such as a conflict copy, is
    /// picked up by the next scan, so convergence can take three rounds.)
    async fn sync(&self) {
        for _ in 0..3 {
            self.mac.sync_now().await.unwrap();
            self.pc.sync_now().await.unwrap();
        }
    }
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn listing(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".thetunnel" {
                continue;
            }
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if e.file_type().unwrap().is_dir() {
                out.push(format!("{rel}/"));
                walk(&e.path(), &rel, out);
            } else {
                out.push(rel);
            }
        }
    }
    walk(dir, "", &mut out);
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn folders_stay_in_sync() {
    let t = setup().await;

    // New files and folders appear on the other device.
    fs::create_dir_all(t.mac_dir.join("Docs")).unwrap();
    fs::create_dir_all(t.mac_dir.join("Empty")).unwrap();
    fs::write(t.mac_dir.join("notes.txt"), "first").unwrap();
    fs::write(t.mac_dir.join("Docs/plan.md"), "# plan").unwrap();
    t.sync().await;
    assert_eq!(listing(&t.pc_dir), listing(&t.mac_dir));
    assert_eq!(read(&t.pc_dir.join("Docs/plan.md")), "# plan");

    // An edit on the PC reaches the Mac.
    fs::write(t.pc_dir.join("notes.txt"), "edited on the pc").unwrap();
    t.sync().await;
    assert_eq!(read(&t.mac_dir.join("notes.txt")), "edited on the pc");

    // A rename is applied as a move, without asking about a deletion.
    fs::rename(t.mac_dir.join("Docs/plan.md"), t.mac_dir.join("Docs/plan-v2.md")).unwrap();
    t.sync().await;
    assert_eq!(listing(&t.pc_dir), listing(&t.mac_dir));
    assert!(t.pc.snapshot().sync.pending_deletions.is_empty());
    assert!(t.pc.snapshot().sync.recent.iter().any(|c| matches!(c.kind, ChangeKind::Renamed) && c.path == "Docs/plan-v2.md"));

    // A deletion is never applied automatically: the PC is asked first.
    fs::remove_file(t.mac_dir.join("notes.txt")).unwrap();
    t.sync().await;
    assert!(t.pc_dir.join("notes.txt").exists(), "deleted without review");
    let pending = t.pc.snapshot().sync.pending_deletions;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].path, "notes.txt");
    assert_eq!(pending[0].device_name, "Mac");

    // "Keep" restores it on the Mac.
    t.pc.resolve_deletions(t.mac.device_id(), vec!["notes.txt".into()], false).await.unwrap();
    t.sync().await;
    assert_eq!(read(&t.mac_dir.join("notes.txt")), "edited on the pc");
    assert!(t.pc.snapshot().sync.pending_deletions.is_empty());

    // Deleting again and accepting removes it on both.
    fs::remove_file(t.mac_dir.join("notes.txt")).unwrap();
    t.sync().await;
    t.pc.resolve_deletions(t.mac.device_id(), vec!["notes.txt".into()], true).await.unwrap();
    t.sync().await;
    assert!(!t.pc_dir.join("notes.txt").exists());
    assert!(!t.mac_dir.join("notes.txt").exists(), "came back after accepted deletion");
    assert!(t.mac.snapshot().sync.pending_deletions.is_empty());

    // A deleted empty folder is removed without asking (there's nothing to lose).
    fs::remove_dir(t.mac_dir.join("Empty")).unwrap();
    t.sync().await;
    assert!(!t.pc_dir.join("Empty").exists());
    assert_eq!(listing(&t.pc_dir), listing(&t.mac_dir));
}

#[tokio::test(flavor = "multi_thread")]
async fn edits_on_both_sides_keep_both_versions() {
    let t = setup().await;
    fs::write(t.mac_dir.join("report.txt"), "original").unwrap();
    t.sync().await;

    fs::write(t.mac_dir.join("report.txt"), "mac edit").unwrap();
    fs::write(t.pc_dir.join("report.txt"), "pc edit, made later").unwrap();
    // Make the PC's edit unambiguously the later one.
    let later = SystemTime::now() + Duration::from_secs(60);
    fs::File::options().write(true).open(t.pc_dir.join("report.txt")).unwrap().set_modified(later).unwrap();
    t.sync().await;

    for dir in [&t.mac_dir, &t.pc_dir] {
        assert_eq!(read(&dir.join("report.txt")), "pc edit, made later");
        let files = listing(dir);
        let copy = files.iter().find(|f| f.starts_with("report (conflict from Mac ")).unwrap_or_else(|| panic!("no conflict copy in {files:?}"));
        assert_eq!(read(&dir.join(copy)), "mac edit");
    }
    assert_eq!(listing(&t.mac_dir), listing(&t.pc_dir));
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_downloads_resume() {
    let t = setup().await;
    let big: Vec<u8> = (0..2_000_000u32).map(|i| (i % 253) as u8).collect();
    fs::write(t.mac_dir.join("big.bin"), &big).unwrap();
    t.mac.sync_now().await.unwrap();

    // Pretend the PC already received the first half before the connection dropped.
    use sha2::Digest;
    let hash = hex::encode(sha2::Sha256::digest(&big));
    let tmp = t.pc_dir.join(".thetunnel/tmp");
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join(format!("{hash}.part")), &big[..1_000_000]).unwrap();

    t.sync().await;
    assert_eq!(fs::read(t.pc_dir.join("big.bin")).unwrap(), big);
    assert!(!tmp.join(format!("{hash}.part")).exists());
}

/// The real background path: discovery, folder watching, notify, and rounds,
/// with no manual `sync_now`. Needs a working local network (broadcast or mDNS),
/// so run it explicitly: `cargo test -p tunnel-core --test sync -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn syncs_in_the_background() {
    use tunnel_core::{EngineConfig, SettingsPatch};
    let root = tempfile::tempdir().unwrap();
    let start = |name: &'static str| {
        let dir = root.path().join(name);
        async move {
            let e = Engine::start(EngineConfig { data_dir: dir.join("data"), port: Some(0), discovery: true, use_trash: false }).await.unwrap();
            e.update_settings(SettingsPatch {
                onboarded: Some(true),
                device_name: Some(name.into()),
                sync_dir: Some(dir.join("shared")),
                receive_dir: Some(dir.join("received")),
                ..Default::default()
            })
            .unwrap();
            e
        }
    };
    let (mac, pc) = (start("BgMac").await, start("BgPC").await);
    pair(&mac, &pc).await;
    let (mac_dir, pc_dir) = (root.path().join("BgMac/shared"), root.path().join("BgPC/shared"));

    async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        for _ in 0..150 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fs::write(mac_dir.join("hello.txt"), "from the mac").unwrap();
    wait_for("file to reach the PC", || fs::read_to_string(pc_dir.join("hello.txt")).is_ok_and(|s| s == "from the mac")).await;

    fs::write(pc_dir.join("hello.txt"), "edited on the pc").unwrap();
    wait_for("edit to reach the Mac", || fs::read_to_string(mac_dir.join("hello.txt")).is_ok_and(|s| s == "edited on the pc")).await;

    fs::remove_file(pc_dir.join("hello.txt")).unwrap();
    wait_for("deletion to be offered for review", || mac.snapshot().sync.pending_deletions.len() == 1).await;
    assert!(mac_dir.join("hello.txt").exists());
}
