#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use tokio::sync::broadcast::Receiver;
use tokio::time::timeout;
use tunnel_core::{BrowseScope, Engine, EngineConfig, Event, SettingsPatch};

pub async fn engine(root: &Path, name: &str) -> Engine {
    let e = Engine::start(EngineConfig { data_dir: root.join(name).join("data"), port: Some(0), discovery: false, use_trash: false })
        .await
        .unwrap();
    e.update_settings(SettingsPatch {
        onboarded: Some(true),
        device_name: Some(name.into()),
        profile_name: Some("Arun".into()),
        receive_dir: Some(root.join(name).join("received")),
        sync_dir: Some(root.join(name).join("shared")),
        ask_before_receiving: Some(false),
        sync_enabled: Some(true),
        browse_scope: Some(BrowseScope::SyncFolder),
    })
    .unwrap();
    e
}

pub async fn next<T>(rx: &mut Receiver<Event>, mut f: impl FnMut(Event) -> Option<T>) -> T {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Some(v) = f(rx.recv().await.unwrap()) {
                return v;
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

/// Pair two engines the way two users would: both confirm the matching code.
pub async fn pair(a: &Engine, b: &Engine) {
    let (mut a_rx, mut b_rx) = (a.subscribe(), b.subscribe());
    a.connect_address(&format!("127.0.0.1:{}", b.port())).await.unwrap();
    a.pair(b.device_id());
    let a_session = next(&mut a_rx, |e| match e {
        Event::PairingCode { session_id, .. } => Some(session_id),
        _ => None,
    })
    .await;
    let b_session = next(&mut b_rx, |e| match e {
        Event::PairingCode { session_id, .. } => Some(session_id),
        _ => None,
    })
    .await;
    a.respond_pairing(&a_session, true);
    b.respond_pairing(&b_session, true);
    for rx in [&mut a_rx, &mut b_rx] {
        assert!(next(rx, |e| match e {
            Event::PairingFinished { success, .. } => Some(success),
            _ => None,
        })
        .await);
    }
}
