//! Two engines on this machine pair, exchange files and pull from each other.

mod common;

use common::{engine, next};
use tokio::sync::broadcast::Receiver;
use tunnel_core::{BrowseScope, Event, SettingsPatch, TransferStatus};

async fn wait_transfer(rx: &mut Receiver<Event>, id: &str) -> tunnel_core::TransferInfo {
    next(rx, |e| match e {
        Event::TransferChanged { transfer } if transfer.id == id && !matches!(transfer.status, TransferStatus::Waiting | TransferStatus::Active) => Some(transfer),
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_send_and_pull() {
    let root = tempfile::tempdir().unwrap();
    let mac = engine(root.path(), "Mac").await;
    let pc = engine(root.path(), "PC").await;
    let (mut mac_rx, mut pc_rx) = (mac.subscribe(), pc.subscribe());

    // Mac finds the PC by address (as if discovery were blocked).
    let found = mac.connect_address(&format!("127.0.0.1:{}", pc.port())).await.unwrap();
    assert_eq!(found.id, pc.device_id());
    assert!(!found.paired);

    // Sending before pairing is refused.
    let file = root.path().join("hello.txt");
    std::fs::write(&file, "hello from the mac").unwrap();
    assert!(mac.send_paths(pc.device_id(), vec![file.clone()]).await.is_err());

    // Pair: both sides see the same code and both confirm.
    mac.pair(pc.device_id());
    let (mac_session, mac_code) = next(&mut mac_rx, |e| match e {
        Event::PairingCode { session_id, code, incoming: false, .. } => Some((session_id, code)),
        _ => None,
    })
    .await;
    let (pc_session, pc_code) = next(&mut pc_rx, |e| match e {
        Event::PairingCode { session_id, code, incoming: true, device, .. } => {
            assert_eq!(device.device_name, "Mac");
            Some((session_id, code))
        }
        _ => None,
    })
    .await;
    assert_eq!(mac_code, pc_code);
    pc.respond_pairing(&pc_session, true);
    mac.respond_pairing(&mac_session, true);
    for rx in [&mut mac_rx, &mut pc_rx] {
        let ok = next(rx, |e| match e {
            Event::PairingFinished { success, message, .. } => Some((success, message)),
            _ => None,
        })
        .await;
        assert!(ok.0, "pairing failed: {:?}", ok.1);
    }
    assert!(mac.snapshot().devices.iter().any(|d| d.id == pc.device_id() && d.paired));
    assert!(pc.snapshot().devices.iter().any(|d| d.id == mac.device_id() && d.paired));

    // Send a file and a folder (with a nested file and a name Windows can't store).
    let folder = root.path().join("Photos");
    std::fs::create_dir_all(folder.join("trip")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(folder.join("trip").join("big.bin"), &big).unwrap();
    // A Mac file name Windows can't store; it can only be created on macOS/Linux.
    if cfg!(not(windows)) {
        std::fs::write(folder.join("notes: draft?.txt"), "draft").unwrap();
    }
    std::fs::write(folder.join(".DS_Store"), "junk").unwrap();
    let id = mac.send_paths(pc.device_id(), vec![file.clone(), folder.clone()]).await.unwrap();
    let done = wait_transfer(&mut mac_rx, &id).await;
    assert_eq!(done.status, TransferStatus::Done, "{:?} / pc: {:?}", done.error, pc.snapshot().transfers);

    let recv = root.path().join("PC").join("received");
    assert_eq!(std::fs::read_to_string(recv.join("hello.txt")).unwrap(), "hello from the mac");
    assert_eq!(std::fs::read(recv.join("Photos/trip/big.bin")).unwrap(), big);
    if cfg!(not(windows)) {
        assert_eq!(std::fs::read_to_string(recv.join("Photos/notes_ draft_.txt")).unwrap(), "draft");
    }
    assert!(!recv.join("Photos/.DS_Store").exists());

    // Sending the same file again doesn't overwrite the first copy.
    let id = mac.send_paths(pc.device_id(), vec![file.clone()]).await.unwrap();
    assert_eq!(wait_transfer(&mut mac_rx, &id).await.status, TransferStatus::Done);
    assert!(recv.join("hello (1).txt").exists());

    // The PC browses and pulls from the Mac's shared folder.
    let shared = root.path().join("Mac").join("shared");
    std::fs::create_dir_all(shared.join("Docs")).unwrap();
    std::fs::write(shared.join("Docs").join("plan.md"), "# plan").unwrap();
    std::fs::write(shared.join("Docs").join(".private"), "hidden").unwrap();
    std::fs::write(shared.join(".secret"), "hidden").unwrap();
    std::fs::write(root.path().join("secret.txt"), "outside").unwrap();

    let listing = pc.list_remote(mac.device_id(), "").await.unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "Docs");
    let inner = pc.list_remote(mac.device_id(), "Docs").await.unwrap();
    assert_eq!(inner.entries[0].path, "Docs/plan.md");

    let id = pc.pull(mac.device_id(), vec!["Docs".into()]).await.unwrap();
    let done = wait_transfer(&mut pc_rx, &id).await;
    assert_eq!(done.status, TransferStatus::Done, "{:?}", done.error);
    assert_eq!(std::fs::read_to_string(recv.join("Docs/plan.md")).unwrap(), "# plan");
    assert!(!recv.join("Docs/.private").exists(), "hidden files must not be copied");
    assert!(pc.pull(mac.device_id(), vec![".secret".into()]).await.is_err());
    assert!(pc.list_remote(mac.device_id(), ".").await.is_err());

    // Browsing the home folder: read-only, and app data / hidden folders stay private.
    mac.update_settings(SettingsPatch { browse_scope: Some(BrowseScope::Home), ..Default::default() }).unwrap();
    let home = pc.list_remote(mac.device_id(), "").await.unwrap();
    assert_eq!(home.root, "Home");
    assert!(home.entries.iter().all(|e| !e.name.starts_with('.') && e.name != "Library" && e.name != "AppData"), "{:?}", home.entries);
    assert!(pc.list_remote(mac.device_id(), "Library").await.is_err());

    // Browsing the whole computer: the top level lists Home and the drives.
    mac.update_settings(SettingsPatch { browse_scope: Some(BrowseScope::Computer), ..Default::default() }).unwrap();
    let top = pc.list_remote(mac.device_id(), "").await.unwrap();
    assert_eq!(top.root, "Mac");
    assert!(top.entries.iter().any(|e| e.path == "Home" && e.kind == "home"), "{:?}", top.entries);
    let drive = top.entries.iter().find(|e| e.kind == "drive").expect("at least one drive");
    let in_drive = pc.list_remote(mac.device_id(), &drive.path).await.unwrap();
    assert!(!in_drive.entries.is_empty());
    assert!(in_drive.entries.iter().all(|e| !e.name.starts_with('.')));
    assert!(pc.list_remote(mac.device_id(), "Home/Library").await.is_err());
    assert!(pc.list_remote(mac.device_id(), "Nowhere").await.is_err());
    assert!(pc.list_remote(mac.device_id(), "Home/../..").await.is_err());
    let err = pc.pull(mac.device_id(), vec!["Home".into()]).await.unwrap_err().to_string();
    assert!(err.contains("whole drive"), "{err}");

    // Browsing can be switched off entirely.
    mac.update_settings(SettingsPatch { browse_scope: Some(BrowseScope::Nobody), ..Default::default() }).unwrap();
    let err = pc.list_remote(mac.device_id(), "").await.unwrap_err().to_string();
    assert!(err.contains("doesn't let other devices browse"), "{err}");

    // Nothing outside the shared folder can be reached.
    assert!(pc.list_remote(mac.device_id(), "..").await.is_err());
    assert!(pc.pull(mac.device_id(), vec!["../secret.txt".into()]).await.is_err());

    // After unpairing, the PC refuses the Mac.
    pc.unpair(mac.device_id());
    let id = mac.send_paths(pc.device_id(), vec![file]).await.unwrap();
    let t = wait_transfer(&mut mac_rx, &id).await;
    assert_eq!(t.status, TransferStatus::Failed);
    assert!(t.error.unwrap().contains("isn't paired"));
}

#[tokio::test(flavor = "multi_thread")]
async fn declined_pairing_is_not_trusted() {
    let root = tempfile::tempdir().unwrap();
    let mac = engine(root.path(), "Mac").await;
    let pc = engine(root.path(), "PC").await;
    let (mut mac_rx, mut pc_rx) = (mac.subscribe(), pc.subscribe());
    mac.connect_address(&format!("127.0.0.1:{}", pc.port())).await.unwrap();

    mac.pair(pc.device_id());
    let pc_session = next(&mut pc_rx, |e| match e {
        Event::PairingCode { session_id, .. } => Some(session_id),
        _ => None,
    })
    .await;
    pc.respond_pairing(&pc_session, false);
    let (success, message) = next(&mut mac_rx, |e| match e {
        Event::PairingFinished { success, message, .. } => Some((success, message)),
        _ => None,
    })
    .await;
    assert!(!success);
    assert_eq!(message.as_deref(), Some("PC declined."));
    assert!(!mac.snapshot().devices.iter().any(|d| d.paired));
    assert!(!pc.snapshot().devices.iter().any(|d| d.paired));
}
