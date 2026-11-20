//! v1.9 peer allow-list (`XEXAM_FRONTEND_PEERS`): when the frontend runs on another machine, this service's port is
//! reachable from the network, so only the listed frontend addresses may talk to it (loopback always may: the health
//! check). The backend has the same module (`security/peers.rs`); keep the two in step.
//!
//! Unset (or blank) = any peer, exactly as before. Entries are IPv4 / IPv6 addresses or CIDR ranges separated by commas
//! (`, ， 、`), semicolons (`; ；`) or white space; quotes around an entry are stripped. `/0` ranges and the unspecified
//! addresses (`0.0.0.0`, `::`) are refused because they would allow everyone. An invalid entry is skipped with a
//! warning; a value with no valid entry at all allows loopback only (and warns), never everyone.
use std::net::{IpAddr, SocketAddr};

/// The parsed peer list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PeerList {
    /// No restriction (the variable is unset or blank)
    #[default]
    Any,
    /// Loopback and these addresses / ranges: the first `u8` bits of the address
    List(Vec<(IpAddr, u8)>),
    /// The variable is set but has no valid entry: loopback only
    LoopbackOnly,
}

/// An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) as the plain IPv4 address.
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

fn contains(net: (IpAddr, u8), ip: IpAddr) -> bool {
    let bits = u32::from(net.1);
    match (net.0, ip) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            bits == 0 || (u32::from(a) ^ u32::from(b)) >> (32 - bits) == 0
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            bits == 0 || (u128::from(a) ^ u128::from(b)) >> (128 - bits) == 0
        }
        _ => false,
    }
}

impl PeerList {
    /// True when the variable restricts the peers.
    pub fn is_restricted(&self) -> bool {
        !matches!(self, PeerList::Any)
    }

    /// Number of valid entries (0 when unrestricted or loopback only).
    pub fn len(&self) -> usize {
        match self {
            PeerList::List(v) => v.len(),
            _ => 0,
        }
    }

    /// True when there is no valid entry.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// May this connection talk to the service? Unrestricted: yes. No peer address (a router without connect info):
    /// yes. Otherwise loopback and the listed addresses.
    pub fn allows(&self, peer: Option<SocketAddr>) -> bool {
        let Some(peer) = peer else {
            return true;
        };
        let nets = match self {
            PeerList::Any => return true,
            PeerList::LoopbackOnly => &[][..],
            PeerList::List(v) => v.as_slice(),
        };
        let ip = normalize_ip(peer.ip());
        ip.is_loopback() || nets.iter().any(|n| contains(*n, ip))
    }
}

fn is_sep(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | '，' | '、' | ';' | '；')
}

/// An entry cut for a log line (printable, at most 64 characters).
fn shown(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
}

fn parse_entry(tok: &str) -> Result<(IpAddr, u8), String> {
    let (ip_text, prefix_text) = match tok.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (tok, None),
    };
    let ip: IpAddr = ip_text.parse().map_err(|_| {
        format!(
            "ignored entry {:?}: not an IP address or CIDR range",
            shown(tok)
        )
    })?;
    let ip = if prefix_text.is_none() {
        normalize_ip(ip)
    } else {
        ip
    };
    let max: u8 = if ip.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix_text {
        None => max,
        Some(p) => {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || p.len() > 3 {
                return Err(format!("ignored entry {:?}: bad prefix length", shown(tok)));
            }
            match p.parse::<u8>() {
                Ok(n) if n <= max => n,
                _ => return Err(format!("ignored entry {:?}: bad prefix length", shown(tok))),
            }
        }
    };
    if ip.is_unspecified() || prefix == 0 {
        return Err(format!(
            "ignored entry {:?}: it would allow every address",
            shown(tok)
        ));
    }
    Ok((ip, prefix))
}

/// Parses the value of `XEXAM_FRONTEND_PEERS`. Returns the list and the warnings to log (English, one per problem).
pub fn parse_peers(raw: Option<&str>) -> (PeerList, Vec<String>) {
    let Some(raw) = raw else {
        return (PeerList::Any, Vec::new());
    };
    let tokens: Vec<String> = raw
        .split(is_sep)
        .map(|t| t.trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’')))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    if tokens.is_empty() {
        // Blank (for example `XEXAM_FRONTEND_PEERS=` passed by Compose) means unset
        return (PeerList::Any, Vec::new());
    }
    let mut nets: Vec<(IpAddr, u8)> = Vec::new();
    let mut warnings = Vec::new();
    for tok in &tokens {
        match parse_entry(tok) {
            Ok(n) => {
                if !nets.contains(&n) {
                    nets.push(n);
                }
            }
            Err(w) => warnings.push(format!("XEXAM_FRONTEND_PEERS: {w}")),
        }
    }
    if nets.is_empty() {
        warnings.push(
            "XEXAM_FRONTEND_PEERS is set but has no valid entry: only loopback may connect"
                .to_string(),
        );
        return (PeerList::LoopbackOnly, warnings);
    }
    (PeerList::List(nets), warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(s: &str) -> Option<SocketAddr> {
        Some(format!("{s}:40000").parse().unwrap_or_else(|_| {
            // IPv6 needs brackets
            format!("[{s}]:40000").parse().expect("peer")
        }))
    }

    #[test]
    fn unset_or_blank_means_any() {
        for raw in [None, Some(""), Some("  "), Some("\"\""), Some(" , ; ")] {
            let (l, w) = parse_peers(raw);
            assert!(!l.is_restricted(), "{raw:?}");
            assert!(w.is_empty(), "{raw:?}");
            assert!(l.allows(peer("203.0.113.5")));
        }
    }

    #[test]
    fn listed_addresses_and_ranges_are_allowed() {
        let (l, w) = parse_peers(Some(
            "10.9.9.9, 192.0.2.0/24；fd00::/8 、 \"10.1.2.3\"; '198.51.100.7'",
        ));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(l.len(), 5);
        assert!(l.allows(peer("10.9.9.9")));
        assert!(l.allows(peer("192.0.2.77")));
        assert!(l.allows(peer("fd00::1234")));
        assert!(l.allows(peer("10.1.2.3")));
        assert!(l.allows(peer("198.51.100.7")));
        assert!(!l.allows(peer("10.9.9.10")));
        assert!(!l.allows(peer("192.0.3.1")));
        assert!(!l.allows(peer("fe80::1")));
    }

    #[test]
    fn loopback_and_missing_peer_always_pass() {
        let (l, _) = parse_peers(Some("10.9.9.9/32"));
        assert!(l.allows(peer("127.0.0.1")));
        assert!(l.allows(peer("::1")));
        assert!(l.allows(peer("::ffff:127.0.0.1")));
        assert!(l.allows(None));
        assert!(!l.allows(peer("10.1.1.1")));
    }

    #[test]
    fn mapped_ipv4_peers_match_ipv4_entries() {
        let (l, _) = parse_peers(Some("10.9.9.9, ::ffff:10.8.8.8"));
        assert!(l.allows(peer("::ffff:10.9.9.9")));
        assert!(l.allows(peer("10.8.8.8")));
    }

    #[test]
    fn dangerous_and_invalid_entries_are_refused_with_a_warning() {
        let (l, w) = parse_peers(Some(
            "0.0.0.0 ::  10.0.0.0/0 ::/0 10.0.0.1/33 fd00::/129 10.0.0.1/x nope 10.0.0.1/ 10.9.9.9",
        ));
        assert_eq!(l.len(), 1);
        assert_eq!(w.len(), 9, "{w:?}");
        assert!(l.allows(peer("10.9.9.9")));
        assert!(!l.allows(peer("10.0.0.1")));
    }

    #[test]
    fn a_value_without_any_valid_entry_allows_loopback_only() {
        let (l, w) = parse_peers(Some("0.0.0.0/0, nope"));
        assert!(l.is_restricted());
        assert!(l.is_empty());
        assert_eq!(l, PeerList::LoopbackOnly);
        assert!(w.iter().any(|x| x.contains("no valid entry")));
        assert!(l.allows(peer("127.0.0.1")));
        assert!(!l.allows(peer("10.9.9.9")));
    }

    #[test]
    fn long_bad_entries_are_cut_in_the_warning() {
        let long = "x".repeat(300);
        let (_, w) = parse_peers(Some(&long));
        assert!(w[0].len() < 200, "{}", w[0].len());
    }
}
