//! Cancelling transfers, and resuming an interrupted one where it stopped.

mod common;

use std::fs;
use std::time::Duration;

use common::{engine, pair};
use tokio::sync::broadcast::Receiver;
use tokio::time::timeout;
use tunnel_core::{Event, TransferInfo, TransferStatus};

async fn until(rx: &mut Receiver<Event>, mut f: impl FnMut(&TransferInfo) -> bool) -> TransferInfo {
    timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(Event::TransferChanged { transfer }) = rx.recv().await
                && f(&transfer) {
                    return transfer;
                }
        }
    })
    .await
    .expect("timed out")
}

fn finished(t: &TransferInfo) -> bool {
    !matches!(t.status, TransferStatus::Waiting | TransferStatus::Active)
}

fn big_file(dir: &std::path::Path) -> (std::path::PathBuf, Vec<u8>) {
    let data: Vec<u8> = (0..60_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    let path = dir.join("movie.bin");
    fs::write(&path, &data).unwrap();
    (path, data)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_send_resumes_on_retry() {
    let root = tempfile::tempdir().unwrap();
    let mac = engine(root.path(), "Mac").await;
    let pc = engine(root.path(), "PC").await;
    pair(&mac, &pc).await;
    let (path, data) = big_file(root.path());
    let (mut mac_rx, mut pc_rx) = (mac.subscribe(), pc.subscribe());

    let id = mac.send_paths(pc.device_id(), vec![path]).await.unwrap();
    until(&mut mac_rx, |t| t.id == id && t.status == TransferStatus::Active && t.done_bytes > 5_000_000).await;
    mac.cancel_transfer(&id);
    let t = until(&mut mac_rx, |t| t.id == id && finished(t)).await;
    assert_eq!(t.status, TransferStatus::Cancelled);
    assert!(t.can_retry);
    // The PC sees the interruption and keeps what it has.
    let p = until(&mut pc_rx, finished).await;
    assert_eq!(p.status, TransferStatus::Failed);
    assert!(!root.path().join("PC/received/movie.bin").exists());

    mac.retry_transfer(&id).await.unwrap();
    let resumed = until(&mut mac_rx, |t| t.id == id && t.status == TransferStatus::Active).await;
    assert!(resumed.done_bytes > 0, "retry started from zero instead of resuming");
    let t = until(&mut mac_rx, |t| t.id == id && finished(t)).await;
    assert_eq!(t.status, TransferStatus::Done, "{:?}", t.error);
    assert_eq!(fs::read(root.path().join("PC/received/movie.bin")).unwrap(), data);
    assert!(!root.path().join("PC/received/.tunnel-partial").exists(), "partial files left behind");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_can_cancel() {
    let root = tempfile::tempdir().unwrap();
    let mac = engine(root.path(), "Mac").await;
    let pc = engine(root.path(), "PC").await;
    pair(&mac, &pc).await;
    let (path, _) = big_file(root.path());
    let (mut mac_rx, mut pc_rx) = (mac.subscribe(), pc.subscribe());

    let id = mac.send_paths(pc.device_id(), vec![path]).await.unwrap();
    let incoming = until(&mut pc_rx, |t| t.status == TransferStatus::Active && t.done_bytes > 0).await;
    pc.cancel_transfer(&incoming.id);
    let p = until(&mut pc_rx, |t| t.id == incoming.id && finished(t)).await;
    assert_eq!(p.status, TransferStatus::Cancelled);
    let m = until(&mut mac_rx, |t| t.id == id && finished(t)).await;
    assert_eq!(m.status, TransferStatus::Failed);
    assert!(!root.path().join("PC/received/movie.bin").exists());
    // Nothing half-received is kept after the receiver cancels.
    let partial = root.path().join("PC/received/.tunnel-partial");
    assert!(!partial.exists() || fs::read_dir(&partial).unwrap().next().is_none());
}
