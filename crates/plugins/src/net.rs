//! Keeps plugin web requests off your own computer and local network:
//! `localhost`, private, link-local, shared (CGNAT) and other special
//! addresses are refused, both when written in the URL and (in the host's
//! DNS resolver) after a name is looked up.

use std::net::{IpAddr, Ipv4Addr};

/// Whether a plugin must not connect to this address.
pub fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return blocked_v4(v4);
            }
            let s = v6.segments();
            v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] == 0x64 && s[1] == 0xff9b) // NAT64
                || (s[0] == 0x2001 && s[1] == 0xdb8) // documentation
        }
    }
}

fn blocked_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // shared, 100.64.0.0/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking
        || o[0] >= 240 // reserved
}

/// The host of a URL's authority ("example.com:443" -> "example.com",
/// "[::1]:8080" -> "::1"), lower case.
pub(crate) fn host_of(authority: &str) -> String {
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Refuse local names and blocked addresses written in a URL.
pub(crate) fn public_host(host: &str) -> Result<(), String> {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") {
        return Err(format!("'{host}' is on your own computer or network"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if blocked_ip(ip) {
            return Err(format!("'{host}' is on your own computer or network"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_private_addresses_are_blocked() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "255.255.255.255", "::1", "fd00::1", "fe80::1", "::ffff:192.168.0.1", "::"] {
            assert!(blocked_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "23.45.67.89", "100.128.0.1", "2606:4700::1111"] {
            assert!(!blocked_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn urls_naming_local_hosts_are_refused() {
        assert_eq!(host_of("[::1]:8080"), "::1");
        assert_eq!(host_of("Example.COM.:443"), "example.com");
        for h in ["localhost", "a.localhost", "printer.local", "10.0.0.1", "::1", "metadata.google.internal"] {
            assert!(public_host(h).is_err(), "{h}");
        }
        assert!(public_host("api.example.com").is_ok());
        assert!(public_host("1.1.1.1").is_ok());
    }

    #[test]
    fn listing_a_local_host_does_not_let_a_plugin_reach_it() {
        use crate::tests::plugin;
        for url in ["https://localhost/x", "https://127.0.0.1/x", "https://192.168.1.10/", "https://[::1]/"] {
            let mut p = plugin(
                "id = \"loc\"\nname = \"L\"\npermissions = [\"network\"]\nhosts = [\"localhost\", \"127.0.0.1\", \"192.168.1.10\", \"::1\"]\n",
                &format!("fn on_load() {{ http_get(\"{url}\"); }}"),
            );
            p.start(&std::collections::BTreeSet::from([crate::Permission::Network]), Default::default());
            assert!(p.error.as_deref().is_some_and(|e| e.contains("own computer")), "{url}: {:?}", p.error);
            assert!(p.drain().is_empty());
        }
    }
}
