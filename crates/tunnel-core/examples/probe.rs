//! Development aid: start a headless engine with discovery on and list what it can see.
//! Usage: cargo run -p tunnel-core --example probe -- <data-dir> [seconds]

use std::time::Duration;
use tunnel_core::{Engine, EngineConfig, SettingsPatch};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let data_dir = args.next().expect("data dir").into();
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(6);
    let engine = Engine::start(EngineConfig { data_dir, port: Some(0), discovery: true, use_trash: false }).await?;
    engine.update_settings(SettingsPatch { onboarded: Some(true), device_name: Some("Probe".into()), ..Default::default() })?;
    tokio::time::sleep(Duration::from_secs(secs)).await;
    for d in engine.snapshot().devices {
        println!("{:<12} paired={} online={} os={} id={}", d.device_name, d.paired, d.online, d.os, &d.id[..8]);
    }
    Ok(())
}
