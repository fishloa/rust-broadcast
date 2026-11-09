//! An IP or CIDR block a reverse proxy may connect from — the allow-list
//! behind [`crate::Verifier::forwarded_from`].

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::error::Error;

const V4_BITS: u8 = 32;
const V6_BITS: u8 = 128;
const V4_MAPPED_PREFIX_BITS: u8 = V6_BITS - V4_BITS;

/// One trusted proxy address or CIDR block (`192.0.2.7`, `10.0.0.0/8`,
/// `::1`, `2001:db8::/32`). An IPv4-mapped IPv6 peer (`::ffff:a.b.c.d`)
/// matches the IPv4 block it maps to, on either side of the comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustedProxy {
    network: IpAddr,
    prefix_len: u8,
}

impl TrustedProxy {
    /// `127.0.0.0/8`.
    pub const LOOPBACK_V4: TrustedProxy = TrustedProxy {
        network: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)),
        prefix_len: 8,
    };
    /// `::1/128`.
    pub const LOOPBACK_V6: TrustedProxy = TrustedProxy {
        network: IpAddr::V6(Ipv6Addr::LOCALHOST),
        prefix_len: V6_BITS,
    };

    /// The loopback blocks (`127.0.0.0/8` and `::1`): the default trust set,
    /// so a proxy on the same host keeps working with no configuration.
    pub fn loopback() -> Vec<TrustedProxy> {
        vec![Self::LOOPBACK_V4, Self::LOOPBACK_V6]
    }

    /// Whether `peer` lies inside this block.
    pub fn contains(&self, peer: IpAddr) -> bool {
        let (network, prefix_len) = self.canonical();
        match (network, peer.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => masked(
                u32::from(net).into(),
                u32::from(ip).into(),
                prefix_len,
                V4_BITS,
            ),
            (IpAddr::V6(net), IpAddr::V6(ip)) => masked(net.into(), ip.into(), prefix_len, V6_BITS),
            _ => false,
        }
    }

    fn canonical(&self) -> (IpAddr, u8) {
        match self.network.to_canonical() {
            IpAddr::V4(v4) if self.network.is_ipv6() => (
                v4.into(),
                self.prefix_len.saturating_sub(V4_MAPPED_PREFIX_BITS),
            ),
            other => (other, self.prefix_len),
        }
    }
}

fn masked(net: u128, ip: u128, prefix_len: u8, bits: u8) -> bool {
    let host_bits = u32::from(bits - prefix_len);
    match net.checked_shr(host_bits).zip(ip.checked_shr(host_bits)) {
        Some((n, i)) => n == i,
        None => true,
    }
}

impl FromStr for TrustedProxy {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bad = |why: &str| Error::InvalidTrustedProxy(format!("{s:?}: {why}"));
        let (addr, len) = match s.split_once('/') {
            Some((addr, len)) => (addr, Some(len)),
            None => (s, None),
        };
        let network: IpAddr = addr.trim().parse().map_err(|_| bad("not an IP address"))?;
        let max = if network.is_ipv4() { V4_BITS } else { V6_BITS };
        let prefix_len = match len {
            Some(len) => len
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|l| *l <= max)
                .ok_or_else(|| bad("prefix length out of range"))?,
            None => max,
        };
        Ok(TrustedProxy {
            network,
            prefix_len,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> TrustedProxy {
        s.parse().unwrap()
    }
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn single_address_and_block_membership() {
        assert!(p("192.0.2.7").contains(ip("192.0.2.7")));
        assert!(!p("192.0.2.7").contains(ip("192.0.2.8")));
        assert!(p("10.0.0.0/8").contains(ip("10.255.1.2")));
        assert!(!p("10.0.0.0/8").contains(ip("11.0.0.1")));
        assert!(p("2001:db8::/32").contains(ip("2001:db8:1::1")));
        assert!(!p("2001:db8::/32").contains(ip("2001:db9::1")));
        assert!(p("0.0.0.0/0").contains(ip("203.0.113.9")));
    }

    #[test]
    fn v4_mapped_peer_matches_v4_block_and_back() {
        assert!(p("127.0.0.0/8").contains(ip("::ffff:127.0.0.1")));
        assert!(p("::ffff:192.0.2.0/120").contains(ip("192.0.2.9")));
        assert!(!p("::ffff:192.0.2.0/120").contains(ip("192.0.3.9")));
    }

    #[test]
    fn families_never_cross_match() {
        assert!(!p("::1").contains(ip("127.0.0.1")));
        assert!(!p("127.0.0.0/8").contains(ip("::1")));
    }

    #[test]
    fn loopback_default_covers_both_families_only() {
        let set = TrustedProxy::loopback();
        let trusted = |s: &str| set.iter().any(|n| n.contains(ip(s)));
        assert!(trusted("127.9.9.9") && trusted("::1") && trusted("::ffff:127.0.0.1"));
        assert!(!trusted("203.0.113.9") && !trusted("::2"));
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "",
            "nope",
            "10.0.0.0/33",
            "::1/129",
            "10.0.0.0/x",
            "10.0.0.0/",
        ] {
            assert!(bad.parse::<TrustedProxy>().is_err(), "{bad:?}");
        }
    }
}
