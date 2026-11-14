//! Datagram socket options: the one description of a UDP bind and its multicast
//! join that every crate in the workspace shares (`docs/API-CONVENTIONS.md`,
//! "One shared type per concept across crates (timeouts, backoff, UDP options)").
//!
//! [`UdpOptions`] is plain data over `core::net`, so it is `no_std` and can ride
//! in a sans-IO action (`broadcast_client::Action::UdpBind`) as well as in a
//! config file. Each crate's own bind adapter (`dvb_stream::udp::bind_udp` is
//! the socket2 one) applies it; no crate keeps a second copy of these fields.

use core::net::{IpAddr, Ipv4Addr};

/// Socket options for one UDP datagram bind.
///
/// Every option is a *request*: the kernel may clamp the receive buffer, and
/// `reuse_port` is applied only where the platform provides it.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct UdpOptions {
    /// Requested `SO_RCVBUF` in bytes; `None` keeps the OS default. A larger
    /// buffer absorbs a burst that arrives between two reads.
    pub recv_buffer_bytes: Option<usize>,
    /// Set `SO_REUSEADDR` before the bind. Default `false`.
    pub reuse_address: bool,
    /// Set `SO_REUSEPORT` before the bind, where the platform has it (Unix,
    /// minus solaris/illumos/cygwin/nuttx/wasi); ignored elsewhere. Default
    /// `false`.
    pub reuse_port: bool,
    /// Join a multicast group after the bind; `None` leaves the socket
    /// unicast-only.
    pub join: Option<MulticastJoin>,
}

impl UdpOptions {
    /// A bind with no option set (the OS defaults apply).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recv_buffer_bytes: None,
            reuse_address: false,
            reuse_port: false,
            join: None,
        }
    }

    /// Request an `SO_RCVBUF` of `bytes` (the kernel may round or clamp it).
    #[must_use]
    pub const fn with_recv_buffer_bytes(mut self, bytes: usize) -> Self {
        self.recv_buffer_bytes = Some(bytes);
        self
    }

    /// Enable or disable `SO_REUSEADDR`.
    #[must_use]
    pub const fn with_reuse_address(mut self, on: bool) -> Self {
        self.reuse_address = on;
        self
    }

    /// Enable or disable `SO_REUSEPORT`.
    #[must_use]
    pub const fn with_reuse_port(mut self, on: bool) -> Self {
        self.reuse_port = on;
        self
    }

    /// Join a multicast group after the bind.
    #[must_use]
    pub fn with_join(mut self, join: MulticastJoin) -> Self {
        self.join = Some(join);
        self
    }

    /// The join interface is one value (`Any`, an IPv4 address, an IPv6 index,
    /// or a name the host resolves), so the former "interface address and
    /// interface index never set together" rule holds by construction. A name
    /// cannot be checked against the family here; it is the host's when it
    /// resolves the name.
    ///
    /// # Errors
    ///
    /// [`JoinMismatch::Group`] when the joined address is neither a multicast
    /// group nor the unspecified address, [`JoinMismatch::Family`] when the interface kind cannot join the
    /// group's address family, or [`JoinMismatch::SourceFamily`] when the
    /// source-specific source is of the other family or is itself a group.
    pub fn validate(&self) -> Result<(), JoinMismatch> {
        let Some(join) = &self.join else {
            return Ok(());
        };
        let group = join.group;
        let family_ok = match (group, &join.interface) {
            (IpAddr::V4(_), MulticastInterface::V4(_) | MulticastInterface::Any)
            | (IpAddr::V4(_), MulticastInterface::Name(_)) => true,
            (IpAddr::V6(_), MulticastInterface::V6Index(_) | MulticastInterface::Any)
            | (IpAddr::V6(_), MulticastInterface::Name(_)) => true,
            (IpAddr::V4(_), MulticastInterface::V6Index(_))
            | (IpAddr::V6(_), MulticastInterface::V4(_)) => false,
        };
        if !family_ok {
            return Err(JoinMismatch::Family);
        }
        if !group.is_multicast() && !group.is_unspecified() {
            return Err(JoinMismatch::Group);
        }
        match join.source {
            Some(source) if source.is_ipv4() != group.is_ipv4() => Err(JoinMismatch::SourceFamily),
            Some(source) if source.is_multicast() => Err(JoinMismatch::SourceFamily),
            _ => Ok(()),
        }
    }
}

/// Multicast membership a [`UdpOptions`] bind establishes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MulticastJoin {
    /// The group to receive; always a multicast address.
    pub group: IpAddr,
    /// The interface the join goes out on.
    pub interface: MulticastInterface,
    /// Source address of a source-specific join (RFC 4607); `None` joins
    /// any-source.
    pub source: Option<IpAddr>,
}

impl MulticastJoin {
    /// An any-source join of `group` on the interface the OS picks.
    ///
    /// A group the bind address already names can be left unspecified: a
    /// host that applies the options joins the bind address's group when this
    /// one is unspecified. Anything else is checked by
    /// [`UdpOptions::validate`] (a bind target that is unspecified and a join
    /// group that is not multicast are both refused).
    #[must_use]
    pub const fn new(group: IpAddr) -> Self {
        Self {
            group,
            interface: MulticastInterface::Any,
            source: None,
        }
    }

    /// Join on `interface`.
    #[must_use]
    pub fn with_interface(mut self, interface: MulticastInterface) -> Self {
        self.interface = interface;
        self
    }

    /// Join `group` instead of the one named so far.
    #[must_use]
    pub fn with_group(mut self, group: IpAddr) -> Self {
        self.group = group;
        self
    }

    /// Join the `source`-specific multicast group instead of any source.
    #[must_use]
    pub const fn with_source(mut self, source: IpAddr) -> Self {
        self.source = Some(source);
        self
    }
}

/// The interface a multicast join goes out on: exactly one of "let the OS
/// pick", an IPv4 interface address, an IPv6 interface index, or a system
/// interface *name* (a config file names `eth0`, and which interface that is
/// is the host's to resolve, when it applies the options).
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum MulticastInterface {
    /// Let the OS choose (the unspecified interface).
    #[default]
    Any,
    /// An IPv4 interface address, for an IPv4 group.
    V4(Ipv4Addr),
    /// An IPv6 interface index (`0` = the system default), for an IPv6 group.
    V6Index(u32),
    /// A system interface name, resolved by the host that applies the
    /// options.
    Name(alloc::string::String),
}

impl MulticastInterface {
    /// Parses an interface written as text: an IPv4 address, else an IPv6
    /// interface index (a decimal number), else a system interface name.
    ///
    /// # Errors
    ///
    /// [`InterfaceError`] when the text is a malformed IPv4 address (non-empty,
    /// all digits and dots, not an address) or a number too large for a `u32`
    /// interface index. Anything else is a system interface name.
    pub fn parse(text: &str) -> Result<Self, InterfaceError> {
        if let Ok(address) = text.parse::<Ipv4Addr>() {
            return Ok(Self::V4(address));
        }
        let numeric = !text.is_empty() && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'.'));
        if !numeric {
            return Ok(Self::Name(alloc::string::String::from(text)));
        }
        match text.parse::<u32>() {
            Ok(index) => Ok(Self::V6Index(index)),
            Err(_) if text.bytes().all(|b| b.is_ascii_digit()) => {
                Err(InterfaceError::Index(text.into()))
            }
            Err(_) => Err(InterfaceError::Address(text.into())),
        }
    }

    /// The interface as the text [`MulticastInterface::parse`] reads back.
    #[must_use]
    pub fn as_str(&self) -> alloc::borrow::Cow<'_, str> {
        use alloc::string::ToString as _;
        match self {
            Self::Any => "any".into(),
            Self::V4(address) => address.to_string().into(),
            Self::V6Index(index) => index.to_string().into(),
            Self::Name(name) => name.as_str().into(),
        }
    }

    /// The IPv4 interface address a join uses, with
    /// [`Ipv4Addr::UNSPECIFIED`] for "any".
    #[must_use]
    pub const fn ipv4(&self) -> Ipv4Addr {
        match self {
            Self::V4(address) => *address,
            Self::Any | Self::V6Index(_) | Self::Name(_) => Ipv4Addr::UNSPECIFIED,
        }
    }

    /// The IPv6 interface index a join uses, with `0` (the system default)
    /// for "any".
    #[must_use]
    pub const fn ipv6_index(&self) -> u32 {
        match self {
            Self::V6Index(index) => *index,
            Self::Any | Self::V4(_) | Self::Name(_) => 0,
        }
    }
}

/// Why [`MulticastInterface::parse`] could not read its text.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceError {
    /// The text looked like an IPv4 address but is not one.
    Address(alloc::string::String),
    /// The text was a number too large for an interface index.
    Index(alloc::string::String),
}

impl InterfaceError {
    /// The reason in words, without the offending text.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Address(_) => "not an IPv4 address (and not an interface name or index)",
            Self::Index(_) => "not an interface index (above the u32 range)",
        }
    }
}

impl core::fmt::Display for InterfaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())?;
        f.write_str(": ")?;
        match self {
            Self::Address(text) | Self::Index(text) => f.write_str(text),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for InterfaceError {}

/// Why a [`UdpOptions`] multicast join cannot be performed for its group.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinMismatch {
    /// The joined group is not a multicast address.
    Group,
    /// The join interface is an IPv4 address for an IPv6 group, or an IPv6
    /// index for an IPv4 group.
    Family,
    /// The source-specific source is of the group's other address family, or
    /// is itself a multicast address.
    SourceFamily,
}

impl JoinMismatch {
    /// A message naming the mismatch (for a caller wrapping it in its own
    /// error type).
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Group => "the joined address is not a multicast group",
            Self::Family => "multicast interface kind does not match the group's address family",
            Self::SourceFamily => {
                "multicast source must be a unicast address of the group's family"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::net::Ipv6Addr;

    const GROUP4: IpAddr = IpAddr::V4(Ipv4Addr::new(239, 255, 42, 42));
    const GROUP6: IpAddr = IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1));

    #[test]
    fn defaults_request_nothing() {
        let options = UdpOptions::default();
        assert_eq!(options, UdpOptions::new());
        assert_eq!(options.recv_buffer_bytes, None);
        assert!(!options.reuse_address);
        assert!(!options.reuse_port);
        assert_eq!(options.join, None);
    }

    #[test]
    fn the_setters_carry_each_option() {
        let join = MulticastJoin::new(GROUP4)
            .with_interface(MulticastInterface::V4(Ipv4Addr::new(192, 168, 1, 10)))
            .with_source(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)));
        let options = UdpOptions::new()
            .with_recv_buffer_bytes(4 << 20)
            .with_reuse_address(true)
            .with_reuse_port(false)
            .with_join(join.clone());
        assert_eq!(options.recv_buffer_bytes, Some(4 << 20));
        assert!(options.reuse_address);
        assert!(!options.reuse_port);
        assert_eq!(options.join, Some(join.clone()));
        assert_eq!(join.group, GROUP4);
        assert_eq!(
            join.interface.ipv4(),
            Ipv4Addr::new(192, 168, 1, 10),
            "an IPv4 address joins with itself"
        );
    }

    #[test]
    fn one_interface_is_either_any_an_address_an_index_or_a_name() {
        assert_eq!(MulticastInterface::Any.ipv4(), Ipv4Addr::UNSPECIFIED);
        assert_eq!(MulticastInterface::Any.ipv6_index(), 0);
        assert_eq!(MulticastInterface::V6Index(3).ipv4(), Ipv4Addr::UNSPECIFIED);
        assert_eq!(MulticastInterface::V6Index(3).ipv6_index(), 3);
        assert_eq!(MulticastInterface::V4(Ipv4Addr::LOCALHOST).ipv6_index(), 0);
        assert_eq!(
            MulticastInterface::Name("eth0".into()).ipv4(),
            Ipv4Addr::UNSPECIFIED
        );
        assert_eq!(MulticastInterface::Name("eth0".into()).ipv6_index(), 0);
    }

    #[test]
    fn an_interface_parses_from_text_as_address_then_index_then_name() {
        for (text, want) in [
            (
                "192.168.1.10",
                MulticastInterface::V4(Ipv4Addr::new(192, 168, 1, 10)),
            ),
            ("3", MulticastInterface::V6Index(3)),
            ("eth0", MulticastInterface::Name("eth0".into())),
            ("en0.100", MulticastInterface::Name("en0.100".into())),
        ] {
            let interface = MulticastInterface::parse(text).unwrap();
            assert_eq!(interface, want, "{text}");
            assert_eq!(interface.as_str(), text, "{text} round-trips");
        }
        assert_eq!(MulticastInterface::Any.as_str(), "any");
        for bad in ["300.1.1.1", "4294967296"] {
            assert!(MulticastInterface::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_matching_join_validates() {
        for (group, interface) in [
            (GROUP4, MulticastInterface::Any),
            (GROUP6, MulticastInterface::Any),
            (GROUP4, MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
            (GROUP6, MulticastInterface::V6Index(1)),
            (GROUP4, MulticastInterface::Name("eth0".into())),
            (GROUP6, MulticastInterface::Name("eth0".into())),
        ] {
            let options = UdpOptions::new()
                .with_join(MulticastJoin::new(group).with_interface(interface.clone()));
            assert_eq!(options.validate(), Ok(()), "{group} on {interface:?}");
        }
        assert_eq!(UdpOptions::default().validate(), Ok(()));
    }

    #[test]
    fn a_join_of_the_wrong_family_is_refused() {
        let v6_index = UdpOptions::new()
            .with_join(MulticastJoin::new(GROUP4).with_interface(MulticastInterface::V6Index(1)));
        assert_eq!(v6_index.validate(), Err(JoinMismatch::Family));
        let v4_address = UdpOptions::new().with_join(
            MulticastJoin::new(GROUP6).with_interface(MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
        );
        assert_eq!(v4_address.validate(), Err(JoinMismatch::Family));
    }

    #[test]
    fn a_unicast_join_target_is_refused() {
        let unicast = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let options = UdpOptions::new().with_join(MulticastJoin::new(unicast));
        assert_eq!(options.validate(), Err(JoinMismatch::Group));
    }

    #[test]
    fn an_unspecified_join_group_defers_to_the_bind_address() {
        let options =
            UdpOptions::new().with_join(MulticastJoin::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert_eq!(options.validate(), Ok(()));
    }

    #[test]
    fn a_source_specific_join_checks_the_source() {
        let with_source = |source: IpAddr| {
            UdpOptions::new().with_join(MulticastJoin::new(GROUP4).with_source(source))
        };
        assert_eq!(
            with_source(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))).validate(),
            Ok(())
        );
        assert_eq!(
            with_source(IpAddr::V6(Ipv6Addr::LOCALHOST)).validate(),
            Err(JoinMismatch::SourceFamily)
        );
        assert_eq!(
            with_source(GROUP4).validate(),
            Err(JoinMismatch::SourceFamily),
            "a group is not a source"
        );
    }
}
