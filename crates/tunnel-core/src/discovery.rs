//! Finding other devices on the same network, two ways at once:
//! a small UDP broadcast "beacon" (works on most phone hotspots) and
//! mDNS/Bonjour (works where broadcast is filtered). Neither is trusted:
//! they only provide an address to try, and the TLS connection proves identity.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use crate::engine::Engine;
use crate::net::is_local;
use crate::protocol::{Hello, PROTOCOL_VERSION};

pub const BEACON_PORT: u16 = 47801;
const BEACON_INTERVAL: Duration = Duration::from_secs(2);
const SERVICE_TYPE: &str = "_thetunnel._tcp.local.";
const APP: &str = "the-tunnel";

#[derive(Serialize, Deserialize)]
struct Beacon {
    app: String,
    id: String,
    #[serde(flatten)]
    hello: Hello,
}

pub fn start(engine: Engine) {
    match beacon_socket() {
        Ok(sock) => {
            let sock = Arc::new(sock);
            tokio::spawn(send_beacons(engine.clone(), sock.clone()));
            tokio::spawn(receive_beacons(engine.clone(), sock));
        }
        Err(e) => tracing::warn!("broadcast discovery unavailable: {e:#}"),
    }
    let pruning = engine.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(BEACON_INTERVAL);
        loop {
            tick.tick().await;
            pruning.prune_nearby();
        }
    });
    if let Err(e) = start_mdns(&engine) {
        tracing::warn!("mDNS discovery unavailable: {e:#}");
    }
}

fn beacon_socket() -> Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(unix)]
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, BEACON_PORT)).into())?;
    Ok(UdpSocket::from_std(s.into())?)
}

fn broadcast_targets() -> Vec<SocketAddr> {
    let mut out = vec![SocketAddr::from((Ipv4Addr::BROADCAST, BEACON_PORT))];
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        if let if_addrs::IfAddr::V4(a) = iface.addr
            && let Some(b) = a.broadcast
                && a.ip.is_private() && !a.ip.is_loopback() {
                    out.push(SocketAddr::from((b, BEACON_PORT)));
                }
    }
    out.sort();
    out.dedup();
    out
}

fn beacon_for(engine: &Engine) -> Beacon {
    let s = engine.settings();
    Beacon {
        app: APP.into(),
        id: engine.device_id().to_string(),
        hello: Hello {
            protocol: PROTOCOL_VERSION,
            device_name: s.device_name,
            profile_name: s.profile_name,
            os: std::env::consts::OS.into(),
            port: engine.port(),
        },
    }
}

async fn send_beacons(engine: Engine, sock: Arc<UdpSocket>) {
    let mut tick = tokio::time::interval(BEACON_INTERVAL);
    loop {
        tick.tick().await;
        if !engine.settings().onboarded {
            continue;
        }
        let Ok(payload) = serde_json::to_vec(&beacon_for(&engine)) else { continue };
        for target in broadcast_targets() {
            // Errors are expected when an interface has no route; ignore them.
            let _ = sock.send_to(&payload, target).await;
        }
    }
}

async fn receive_beacons(engine: Engine, sock: Arc<UdpSocket>) {
    let mut buf = vec![0u8; 2048];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf).await else { continue };
        if !is_local(&from.ip()) {
            continue;
        }
        let Ok(beacon) = serde_json::from_slice::<Beacon>(&buf[..n]) else { continue };
        if beacon.app != APP || beacon.hello.protocol != PROTOCOL_VERSION {
            continue;
        }
        engine.note_device(&beacon.id, &beacon.hello, SocketAddr::new(from.ip(), beacon.hello.port), false);
    }
}

fn service_info(engine: &Engine) -> Result<ServiceInfo> {
    let s = engine.settings();
    let id = engine.device_id();
    let instance = format!("tunnel-{}", &id[..12]);
    let props = [
        ("id", id.to_string()),
        ("name", s.device_name),
        ("profile", s.profile_name),
        ("os", std::env::consts::OS.to_string()),
        ("v", PROTOCOL_VERSION.to_string()),
    ];
    Ok(ServiceInfo::new(SERVICE_TYPE, &instance, &format!("{instance}.local."), "", engine.port(), &props[..])?.enable_addr_auto())
}

fn start_mdns(engine: &Engine) -> Result<()> {
    let daemon = ServiceDaemon::new()?;
    if engine.settings().onboarded {
        daemon.register(service_info(engine)?)?;
    }
    let events = daemon.browse(SERVICE_TYPE)?;
    *engine.mdns_slot().lock().unwrap() = Some(daemon);

    let engine = engine.clone();
    tokio::spawn(async move {
        let mut names: HashMap<String, String> = HashMap::new();
        while let Ok(event) = events.recv_async().await {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let Some(id) = info.get_property_val_str("id").map(str::to_owned) else { continue };
                    if info.get_property_val_str("v") != Some(&PROTOCOL_VERSION.to_string()) {
                        continue;
                    }
                    let hello = Hello {
                        protocol: PROTOCOL_VERSION,
                        device_name: info.get_property_val_str("name").unwrap_or("Unknown device").to_string(),
                        profile_name: info.get_property_val_str("profile").unwrap_or_default().to_string(),
                        os: info.get_property_val_str("os").unwrap_or_default().to_string(),
                        port: info.get_port(),
                    };
                    names.insert(info.get_fullname().to_string(), id.clone());
                    for ip in info.get_addresses_v4() {
                        engine.note_device(&id, &hello, SocketAddr::new(IpAddr::V4(ip), hello.port), true);
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    if let Some(id) = names.remove(&fullname) {
                        engine.mdns_lost(&id);
                    }
                }
                _ => {}
            }
        }
    });
    Ok(())
}

/// Re-announce after the device name changes or onboarding finishes.
pub fn refresh_mdns(engine: &Engine) {
    let slot = engine.mdns_slot().lock().unwrap();
    let Some(daemon) = slot.as_ref() else { return };
    if !engine.settings().onboarded {
        return;
    }
    match service_info(engine) {
        Ok(info) => {
            if let Err(e) = daemon.register(info) {
                tracing::warn!("mDNS re-register failed: {e:#}");
            }
        }
        Err(e) => tracing::warn!("mDNS info failed: {e:#}"),
    }
}
