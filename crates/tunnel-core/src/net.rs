//! The Tunnel never talks to the internet. Every inbound and outbound
//! connection is checked against this list of local-only address ranges.

use std::net::IpAddr;

pub fn is_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local(&IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80 // link local
        }
    }
}

/// Private IPv4 addresses of this machine, shown to the user for manual connection.
pub fn local_addresses() -> Vec<std::net::Ipv4Addr> {
    let mut out: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if a.ip.is_private() => Some(a.ip),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_ranges_are_allowed() {
        for ok in ["192.168.43.10", "172.20.10.2", "10.0.0.5", "127.0.0.1", "169.254.1.1", "fe80::1", "fd00::1"] {
            assert!(is_local(&ok.parse().unwrap()), "{ok} should be local");
        }
        for bad in ["8.8.8.8", "100.64.0.1", "1.1.1.1", "2001:4860::8888"] {
            assert!(!is_local(&bad.parse().unwrap()), "{bad} should be rejected");
        }
    }
}
