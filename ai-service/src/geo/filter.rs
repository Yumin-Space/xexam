//! The reserved-address filter of the external IP-location lookup (spec 4.1): an address that is private, loopback,
//! link-local, CGNAT, documentation, multicast, reserved or otherwise not a public unicast address is NEVER sent to a
//! provider. The main backend applies the identical rules (`backend/src/geo.rs`) with the same test vectors.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IPv4 ranges that are never looked up: (first address, prefix length).
const V4_RESERVED: [(Ipv4Addr, u32); 18] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    // CGNAT
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    // documentation
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    (Ipv4Addr::new(192, 31, 196, 0), 24),
    (Ipv4Addr::new(192, 52, 193, 0), 24),
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(192, 175, 48, 0), 24),
    // benchmarking
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    // multicast
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    // reserved, including 255.255.255.255
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

fn in_v4(ip: Ipv4Addr, net: Ipv4Addr, prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    u32::from(ip) & mask == u32::from(net) & mask
}

fn public_v4(ip: Ipv4Addr) -> bool {
    !V4_RESERVED.iter().any(|(net, p)| in_v4(ip, *net, *p))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    let (a, b) = (s[0], s[1]);
    // only the global unicast block 2000::/3 can be public (this drops ::, ::1, fc00::/7, fe80::/10, ff00::/8 ...)
    if a & 0xe000 != 0x2000 {
        return false;
    }
    // 2001::/23 (IETF protocol assignments, Teredo, ORCHID ...)
    if a == 0x2001 && b < 0x0200 {
        return false;
    }
    // 2001:db8::/32 documentation
    if a == 0x2001 && b == 0x0db8 {
        return false;
    }
    // 2002::/16 6to4
    if a == 0x2002 {
        return false;
    }
    // 3fff::/20 documentation
    if a == 0x3fff && b & 0xf000 == 0 {
        return false;
    }
    true
}

/// `true` when the address may be sent to a lookup provider. IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are judged as
/// the IPv4 address they carry.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => public_v4(v4),
            None => public_v6(v6),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("address")
    }

    /// (first address, last address) of every reserved IPv4 range, written out by hand.
    const V4_EDGES: [(&str, &str); 18] = [
        ("0.0.0.0", "0.255.255.255"),
        ("10.0.0.0", "10.255.255.255"),
        ("100.64.0.0", "100.127.255.255"),
        ("127.0.0.0", "127.255.255.255"),
        ("169.254.0.0", "169.254.255.255"),
        ("172.16.0.0", "172.31.255.255"),
        ("192.0.0.0", "192.0.0.255"),
        ("192.0.2.0", "192.0.2.255"),
        ("192.31.196.0", "192.31.196.255"),
        ("192.52.193.0", "192.52.193.255"),
        ("192.88.99.0", "192.88.99.255"),
        ("192.168.0.0", "192.168.255.255"),
        ("192.175.48.0", "192.175.48.255"),
        ("198.18.0.0", "198.19.255.255"),
        ("198.51.100.0", "198.51.100.255"),
        ("203.0.113.0", "203.0.113.255"),
        ("224.0.0.0", "239.255.255.255"),
        ("240.0.0.0", "255.255.255.255"),
    ];

    #[test]
    fn every_reserved_ipv4_range_is_refused_at_both_edges() {
        assert_eq!(V4_EDGES.len(), V4_RESERVED.len());
        for (first, last) in V4_EDGES {
            assert!(!is_public(ip(first)), "{first}");
            assert!(!is_public(ip(last)), "{last}");
            // the same address as an IPv4-mapped IPv6 one
            assert!(!is_public(ip(&format!("::ffff:{first}"))), "mapped {first}");
            assert!(!is_public(ip(&format!("::ffff:{last}"))), "mapped {last}");
        }
    }

    #[test]
    fn ipv4_neighbours_of_the_reserved_ranges_are_public() {
        for s in [
            "1.0.0.0",
            "9.255.255.255",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.0",
            "192.0.3.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "198.51.99.255",
            "198.51.101.0",
            "203.0.112.255",
            "203.0.114.0",
            "223.255.255.255",
            "8.8.8.8",
            "1.1.1.1",
        ] {
            assert!(is_public(ip(s)), "{s}");
        }
    }

    #[test]
    fn ipv6_outside_the_global_unicast_block_is_refused() {
        for s in [
            "::",
            "::1",
            "::2",
            "::ffff:0:0",
            "64:ff9b::1",
            "64:ff9b:1::1",
            "100::1",
            "1fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "4000::1",
            "5f00::1",
            "fc00::1",
            "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "fe80::1",
            "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "fec0::1",
            "ff00::1",
            "ff02::1",
            "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        ] {
            assert!(!is_public(ip(s)), "{s}");
        }
    }

    #[test]
    fn ipv6_special_ranges_inside_the_global_block_are_refused_at_both_edges() {
        for (first, last) in [
            ("2001::", "2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("2001:db8::", "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("2002::", "2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            ("3fff::", "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff"),
        ] {
            assert!(!is_public(ip(first)), "{first}");
            assert!(!is_public(ip(last)), "{last}");
        }
        // and their neighbours are public
        for s in [
            "2000::1",
            "2001:200::1",
            "2001:4860:4860::8888",
            "2001:db7:ffff::1",
            "2001:db9::1",
            "2003::1",
            "3ffe::1",
            "3fff:1000::1",
            "2606:4700:4700::1111",
            "3fff:ffff::1",
        ] {
            assert!(is_public(ip(s)), "{s}");
        }
    }

    #[test]
    fn mapped_addresses_follow_the_address_they_carry() {
        assert!(is_public(ip("::ffff:8.8.8.8")));
        assert!(!is_public(ip("::ffff:10.0.0.1")));
        assert!(is_public(ip("::ffff:100.63.255.255")));
        assert!(!is_public(ip("::ffff:100.64.0.1")));
        assert!(!is_public(ip("::ffff:127.0.0.1")));
    }
}
