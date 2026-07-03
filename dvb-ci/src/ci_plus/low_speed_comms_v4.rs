//! Low Speed Communication resource version 4 extensions — ETSI TS 103 205
//! V1.4.1 §10, Tables 76-89 (PDF pp. 100-110). See
//! `docs/ts_103_205/low-speed-comms-v4.md`.
//!
//! LSC v4 extends LSC v3 (CI Plus V1.3 \[3\] §14.1) to add source-specific
//! multicast, hybrid connections (response data across the TS interface), a new
//! `comms_info()` APDU, a new `comms_IP_config()` APDU, and a `source_port` in the
//! connection_descriptor. The new APDU tags live in the CI Plus `0x9F8Cxx`
//! namespace.
//!
//! - `comms_info_req` (`0x9F8C07`, Table 76) — CICAM → Host, header-only.
//! - `comms_info_reply` (`0x9F8C08`, Table 77) — Host → CICAM.
//! - `comms_IP_config_req` (`0x9F8C09`, Table 78) — CICAM → Host, header-only.
//! - `comms_IP_config_reply` (`0x9F8C0A`, Table 79) — Host → CICAM.
//! - the Comms Cmd `hybrid_descriptor` (Table 83) and `multicast_descriptor`
//!   (Table 85) descriptor bodies.
//!
//! ## Not wired into `CiPlusApdu` resource dispatch
//!
//! TS 103 205 §10 prints **no** LSC v4 resource-summary table with a single
//! `resource_identifier`. The LSC resource_id and the base `comms_cmd` /
//! `comms_reply` / `comms_send` / `comms_rcv` APDUs are defined in CI Plus V1.3
//! \[3\] §14.1 (proprietary, not reproduced) and are **deferred**. These v4
//! extension APDUs are therefore provided as standalone, directly-constructible /
//! parseable typed structs with a [`LscV4Apdu::parse`] tag-dispatch helper — they
//! are **not** wired into [`crate::ci_plus::CiPlusApdu`] (no invented resource_id),
//! mirroring [`crate::ci_plus::ca_support`].
//!
//! ## Table 80 reserved-range typo
//!
//! `connection_state` is a **2-bit** field; Table 80 prints the Reserved range as
//! `0x10-0x11`, an evident typo. We treat it strictly as 2 bits — values 0..3 —
//! with `0x00` = Disconnected, `0x01` = Connected, and `0x02`/`0x03` Reserved. The
//! literal `0x10-0x11` range is not encoded.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// New LSC v4 `apdu_tag`s (§10), in the `0x9F8Cxx` namespace.
pub mod tag {
    use crate::tag::ApduTag;
    /// `comms_info_req_tag` = `0x9F8C07` (Table 76).
    pub const COMMS_INFO_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x07);
    /// `comms_info_reply_tag` = `0x9F8C08` (Table 77).
    pub const COMMS_INFO_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x08);
    /// `comms_IP_config_req_tag` = `0x9F8C09` (Table 78).
    pub const COMMS_IP_CONFIG_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x09);
    /// `comms_IP_config_reply_tag` = `0x9F8C0A` (Table 79).
    pub const COMMS_IP_CONFIG_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x0A);
}

/// A 128-bit IPv6-format address (IPv4 prefixed `::ffff:0:0/96` or `::0:0/96`).
pub const IP_ADDR_LEN: usize = 16;
/// A 48-bit MAC `physical_address`.
pub const MAC_LEN: usize = 6;

// --- connection_state (Table 80) ---

/// `connection_state` values (Table 80). The field is 2 bits; see the module doc
/// for the `0x10-0x11` reserved-range typo note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ConnectionState {
    /// `0x00` — Disconnected (interface inactive or disconnected).
    Disconnected,
    /// `0x01` — Connected (interface active with a valid IP address).
    Connected,
    /// Reserved 2-bit value (`0x02`–`0x03`).
    Reserved(u8),
}
impl ConnectionState {
    /// Decode a 2-bit `connection_state` value (low 2 bits of `v`).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0x00 => Self::Disconnected,
            0x01 => Self::Connected,
            other => Self::Reserved(other),
        }
    }
    /// The 2-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Disconnected => 0x00,
            Self::Connected => 0x01,
            Self::Reserved(v) => v & 0x03,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connected => "connected",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(ConnectionState, Reserved);

// --- IP_protocol_version (Table 86) ---

/// `IP_protocol_version` values (Table 86), used in [`MulticastDescriptor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IpProtocolVersion {
    /// `0x01` — IPv4.
    Ipv4,
    /// `0x02` — IPv6.
    Ipv6,
    /// Reserved (`0x00`, `0x03`–`0xFF`).
    Reserved(u8),
}
impl IpProtocolVersion {
    /// Decode an `IP_protocol_version` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Ipv4,
            0x02 => Self::Ipv6,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Ipv4 => 0x01,
            Self::Ipv6 => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(IpProtocolVersion, Reserved);

// ---------------------------------------------------------------------------
// comms_info_req (Table 76)
// ---------------------------------------------------------------------------

/// `comms_info_req()` (Table 76): CICAM → Host. Header-only.
///
/// NB: Table 76 prints `length_field() = 1`, but the APDU carries no payload
/// fields, so the on-wire body length is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsInfoReq;

impl<'a> Parse<'a> for CommsInfoReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::COMMS_INFO_REQ, "comms_info_req")?;
        Ok(Self)
    }
}
impl Serialize for CommsInfoReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::COMMS_INFO_REQ, buf)
    }
}

// ---------------------------------------------------------------------------
// comms_info_reply (Table 77)
// ---------------------------------------------------------------------------

/// `comms_info_reply()` (Table 77): Host → CICAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsInfoReply {
    /// `LTS_id` (8) — identifier of the Local TS.
    pub lts_id: u8,
    /// `status` (1) — `true` = a connection has been established.
    pub status: bool,
    /// `source_IPaddress` (128) — IPv6-format source address; all-zero if unknown.
    pub source_ip_address: [u8; IP_ADDR_LEN],
    /// `source_port` (16) — source port; `0x0000` if not aware.
    pub source_port: u16,
    /// `inputDeliveryPID` (13) — TS-interface delivery PID for hybrid connections
    /// (`0x0020`–`0x1FFE`); `0x0000` if not a hybrid connection.
    pub input_delivery_pid: u16,
}

// LTS_id(1) + reserved/status(1) + source_IPaddress(16) + source_port(2) +
// reserved/inputDeliveryPID(2) = 22.
const INFO_REPLY_BODY: usize = 1 + 1 + IP_ADDR_LEN + 2 + 2;
const STATUS_BIT: u8 = 0x01;
const INPUT_DELIVERY_PID_MASK: u16 = 0x1FFF;

impl<'a> Parse<'a> for CommsInfoReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::COMMS_INFO_REPLY, "comms_info_reply")?;
        if body.len() < INFO_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: INFO_REPLY_BODY,
                have: body.len(),
                what: "comms_info_reply",
            });
        }
        let lts_id = body[0];
        let status = body[1] & STATUS_BIT != 0;
        let mut source_ip_address = [0u8; IP_ADDR_LEN];
        source_ip_address.copy_from_slice(&body[2..2 + IP_ADDR_LEN]);
        let p = 2 + IP_ADDR_LEN;
        let source_port = u16::from_be_bytes([body[p], body[p + 1]]);
        let input_delivery_pid =
            u16::from_be_bytes([body[p + 2], body[p + 3]]) & INPUT_DELIVERY_PID_MASK;
        Ok(Self {
            lts_id,
            status,
            source_ip_address,
            source_port,
            input_delivery_pid,
        })
    }
}
impl Serialize for CommsInfoReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(INFO_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::COMMS_INFO_REPLY, INFO_REPLY_BODY, buf)?;
        buf[pos] = self.lts_id;
        // reserved(7)='0000000' + status(1).
        buf[pos + 1] = u8::from(self.status);
        buf[pos + 2..pos + 2 + IP_ADDR_LEN].copy_from_slice(&self.source_ip_address);
        let p = pos + 2 + IP_ADDR_LEN;
        buf[p..p + 2].copy_from_slice(&self.source_port.to_be_bytes());
        // reserved(3)='000' + inputDeliveryPID(13).
        buf[p + 2..p + 4]
            .copy_from_slice(&(self.input_delivery_pid & INPUT_DELIVERY_PID_MASK).to_be_bytes());
        Ok(pos + INFO_REPLY_BODY)
    }
}

// ---------------------------------------------------------------------------
// comms_IP_config_req (Table 78)
// ---------------------------------------------------------------------------

/// `comms_IP_config_req()` (Table 78): CICAM → Host. Header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsIpConfigReq;

impl<'a> Parse<'a> for CommsIpConfigReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        objects::parse_empty_apdu(bytes, tag::COMMS_IP_CONFIG_REQ, "comms_IP_config_req")?;
        Ok(Self)
    }
}
impl Serialize for CommsIpConfigReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::serialize_empty_apdu(tag::COMMS_IP_CONFIG_REQ, buf)
    }
}

// ---------------------------------------------------------------------------
// comms_IP_config_reply (Table 79)
// ---------------------------------------------------------------------------

/// The connected-state IP configuration carried when `connection_state == 0x01`
/// (Table 79).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct IpConfig {
    /// `IP_address` (128) — IPv6 format.
    pub ip_address: [u8; IP_ADDR_LEN],
    /// `network_mask` (128).
    pub network_mask: [u8; IP_ADDR_LEN],
    /// `default_gateway` (128).
    pub default_gateway: [u8; IP_ADDR_LEN],
    /// `DHCP_server_address` (128) — all-zero if no DHCP server.
    pub dhcp_server_address: [u8; IP_ADDR_LEN],
    /// `DNS_server_address` list (loop count `num_DNS_servers`).
    pub dns_server_addresses: Vec<[u8; IP_ADDR_LEN]>,
}

/// `comms_IP_config_reply()` (Table 79): Host → CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsIpConfigReply {
    /// `connection_state` (2) — Table 80.
    pub connection_state: ConnectionState,
    /// `physical_address` (48) — MAC of this IP network adapter.
    pub physical_address: [u8; MAC_LEN],
    /// IP configuration, present iff `connection_state == Connected` (`0x01`).
    pub ip_config: Option<IpConfig>,
}

// connection_state/reserved(1) + physical_address(6).
const IP_CONFIG_PREFIX: usize = 1 + MAC_LEN;
// IP_address+network_mask+default_gateway+DHCP_server_address(4*16) + num_DNS_servers(1).
const IP_CONFIG_FIXED: usize = 4 * IP_ADDR_LEN + 1;
const CONNECTION_STATE_CONNECTED: u8 = 0x01;

impl CommsIpConfigReply {
    fn body_len(&self) -> usize {
        IP_CONFIG_PREFIX
            + match &self.ip_config {
                Some(c) => IP_CONFIG_FIXED + c.dns_server_addresses.len() * IP_ADDR_LEN,
                None => 0,
            }
    }
}

fn read_addr(body: &[u8], pos: usize) -> [u8; IP_ADDR_LEN] {
    let mut a = [0u8; IP_ADDR_LEN];
    a.copy_from_slice(&body[pos..pos + IP_ADDR_LEN]);
    a
}

impl<'a> Parse<'a> for CommsIpConfigReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body =
            objects::parse_apdu_header(bytes, tag::COMMS_IP_CONFIG_REPLY, "comms_IP_config_reply")?;
        if body.len() < IP_CONFIG_PREFIX {
            return Err(Error::BufferTooShort {
                need: IP_CONFIG_PREFIX,
                have: body.len(),
                what: "comms_IP_config_reply",
            });
        }
        // connection_state(2) + reserved(6); take the high 2 bits.
        let connection_state = ConnectionState::from_u8(body[0] >> 6);
        let mut physical_address = [0u8; MAC_LEN];
        physical_address.copy_from_slice(&body[1..1 + MAC_LEN]);
        let ip_config = if connection_state.to_u8() == CONNECTION_STATE_CONNECTED {
            if body.len() < IP_CONFIG_PREFIX + IP_CONFIG_FIXED {
                return Err(Error::BufferTooShort {
                    need: IP_CONFIG_PREFIX + IP_CONFIG_FIXED,
                    have: body.len(),
                    what: "comms_IP_config_reply ip_config",
                });
            }
            let mut p = IP_CONFIG_PREFIX;
            let ip_address = read_addr(body, p);
            p += IP_ADDR_LEN;
            let network_mask = read_addr(body, p);
            p += IP_ADDR_LEN;
            let default_gateway = read_addr(body, p);
            p += IP_ADDR_LEN;
            let dhcp_server_address = read_addr(body, p);
            p += IP_ADDR_LEN;
            let n = body[p] as usize;
            p += 1;
            if body.len() < p + n * IP_ADDR_LEN {
                return Err(Error::BufferTooShort {
                    need: p + n * IP_ADDR_LEN,
                    have: body.len(),
                    what: "comms_IP_config_reply dns_servers",
                });
            }
            let mut dns_server_addresses = Vec::with_capacity(n);
            for _ in 0..n {
                dns_server_addresses.push(read_addr(body, p));
                p += IP_ADDR_LEN;
            }
            Some(IpConfig {
                ip_address,
                network_mask,
                default_gateway,
                dhcp_server_address,
                dns_server_addresses,
            })
        } else {
            None
        };
        Ok(Self {
            connection_state,
            physical_address,
            ip_config,
        })
    }
}
impl Serialize for CommsIpConfigReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        let mut pos = objects::write_apdu_header(tag::COMMS_IP_CONFIG_REPLY, body_len, buf)?;
        // connection_state(2) << 6 + reserved(6)='000000'.
        buf[pos] = self.connection_state.to_u8() << 6;
        buf[pos + 1..pos + 1 + MAC_LEN].copy_from_slice(&self.physical_address);
        pos += IP_CONFIG_PREFIX;
        if let Some(c) = &self.ip_config {
            buf[pos..pos + IP_ADDR_LEN].copy_from_slice(&c.ip_address);
            pos += IP_ADDR_LEN;
            buf[pos..pos + IP_ADDR_LEN].copy_from_slice(&c.network_mask);
            pos += IP_ADDR_LEN;
            buf[pos..pos + IP_ADDR_LEN].copy_from_slice(&c.default_gateway);
            pos += IP_ADDR_LEN;
            buf[pos..pos + IP_ADDR_LEN].copy_from_slice(&c.dhcp_server_address);
            pos += IP_ADDR_LEN;
            buf[pos] = c.dns_server_addresses.len() as u8;
            pos += 1;
            for a in &c.dns_server_addresses {
                buf[pos..pos + IP_ADDR_LEN].copy_from_slice(a);
                pos += IP_ADDR_LEN;
            }
        }
        Ok(pos)
    }
}

// ---------------------------------------------------------------------------
// Comms Cmd hybrid_descriptor (Table 83) — descriptor_tag 0x05
// ---------------------------------------------------------------------------

/// `descriptor_tag` of the hybrid_descriptor (Table 83).
pub const HYBRID_DESCRIPTOR_TAG: u8 = 0x05;
/// `descriptor_tag` of the multicast_descriptor (Table 85).
pub const MULTICAST_DESCRIPTOR_TAG: u8 = 0x06;

/// `IP_connection_type` values (Table 84), selecting the hybrid_descriptor's
/// inner connection descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IpConnectionType {
    /// `0x03` — IP_descriptor.
    IpDescriptor,
    /// `0x04` — hostname_descriptor.
    HostnameDescriptor,
    /// `0x06` — multicast_descriptor.
    MulticastDescriptor,
    /// Reserved/unknown (`0x00`–`0x02`, `0x05`, `0x07`–`0xFF`).
    Reserved(u8),
}
impl IpConnectionType {
    /// Decode an `IP_connection_type` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x03 => Self::IpDescriptor,
            0x04 => Self::HostnameDescriptor,
            0x06 => Self::MulticastDescriptor,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::IpDescriptor => 0x03,
            Self::HostnameDescriptor => 0x04,
            Self::MulticastDescriptor => 0x06,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::IpDescriptor => "ip_descriptor",
            Self::HostnameDescriptor => "hostname_descriptor",
            Self::MulticastDescriptor => "multicast_descriptor",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(IpConnectionType, Reserved);

/// `hybrid_descriptor()` (Table 83). The inner `IP_descriptor()` /
/// `hostname_descriptor()` / `multicast_descriptor()` body is carried verbatim as
/// opaque borrowed bytes — `IP_descriptor`/`hostname_descriptor` syntaxes are
/// deferred to CI Plus V1.3 §14.2.1 (not reproduced); `multicast_descriptor` can
/// be re-parsed with [`MulticastDescriptor::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HybridDescriptor<'a> {
    /// `LTS_id` (8) — Local TS for the TCP/UDP payload delivery.
    pub lts_id: u8,
    /// `IP_connection_type` (8) — Table 84.
    pub ip_connection_type: IpConnectionType,
    /// The inner connection-descriptor body (verbatim, after `IP_connection_type`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub inner: &'a [u8],
}

// data portion (after descriptor_length): LTS_id(1) + IP_connection_type(1) + inner.
const HYBRID_FIXED: usize = 1 + 1;

impl<'a> HybridDescriptor<'a> {
    /// Parse a `hybrid_descriptor` (`descriptor_tag` `0x05` + `descriptor_length` +
    /// body) from the start of `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let data = parse_descriptor_header(bytes, HYBRID_DESCRIPTOR_TAG, "hybrid_descriptor")?;
        if data.len() < HYBRID_FIXED {
            return Err(Error::BufferTooShort {
                need: HYBRID_FIXED,
                have: data.len(),
                what: "hybrid_descriptor",
            });
        }
        Ok(Self {
            lts_id: data[0],
            ip_connection_type: IpConnectionType::from_u8(data[1]),
            inner: &data[HYBRID_FIXED..],
        })
    }
    fn data_len(&self) -> usize {
        HYBRID_FIXED + self.inner.len()
    }
}

impl Serialize for HybridDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        descriptor_len(self.data_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = write_descriptor_header(HYBRID_DESCRIPTOR_TAG, self.data_len(), buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.ip_connection_type.to_u8();
        buf[pos + HYBRID_FIXED..pos + self.data_len()].copy_from_slice(self.inner);
        Ok(pos + self.data_len())
    }
}

// ---------------------------------------------------------------------------
// Comms Cmd multicast_descriptor (Table 85) — descriptor_tag 0x06
// ---------------------------------------------------------------------------

/// `multicast_descriptor()` (Table 85).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MulticastDescriptor {
    /// `IP_protocol_version` (8) — Table 86.
    pub ip_protocol_version: IpProtocolVersion,
    /// `IP_address` (128) — multicast service address (IPv4: first 12 bytes `0x00`).
    pub ip_address: [u8; IP_ADDR_LEN],
    /// `multicast_port` (16).
    pub multicast_port: u16,
    /// `include_sources` (1) — `true` = receive only from listed sources; `false`
    /// = receive from all sources except those listed (only relevant when sources
    /// are present).
    pub include_sources: bool,
    /// `source_address` list (loop count `num_source_addresses`); empty = any source.
    pub source_addresses: Vec<[u8; IP_ADDR_LEN]>,
}

// data portion: IP_protocol_version(1) + IP_address(16) + multicast_port(2) +
// reserved/include_sources(1) + num_source_addresses(1).
const MULTICAST_FIXED: usize = 1 + IP_ADDR_LEN + 2 + 1 + 1;
const INCLUDE_SOURCES_BIT: u8 = 0x01;

impl MulticastDescriptor {
    /// Parse a `multicast_descriptor` (`descriptor_tag` `0x06` + `descriptor_length`
    /// + body) from the start of `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let data =
            parse_descriptor_header(bytes, MULTICAST_DESCRIPTOR_TAG, "multicast_descriptor")?;
        if data.len() < MULTICAST_FIXED {
            return Err(Error::BufferTooShort {
                need: MULTICAST_FIXED,
                have: data.len(),
                what: "multicast_descriptor",
            });
        }
        let ip_protocol_version = IpProtocolVersion::from_u8(data[0]);
        let ip_address = read_addr(data, 1);
        let multicast_port = u16::from_be_bytes([data[1 + IP_ADDR_LEN], data[2 + IP_ADDR_LEN]]);
        let flags_pos = 3 + IP_ADDR_LEN;
        let include_sources = data[flags_pos] & INCLUDE_SOURCES_BIT != 0;
        let n = data[flags_pos + 1] as usize;
        let mut pos = MULTICAST_FIXED;
        if data.len() < pos + n * IP_ADDR_LEN {
            return Err(Error::BufferTooShort {
                need: pos + n * IP_ADDR_LEN,
                have: data.len(),
                what: "multicast_descriptor sources",
            });
        }
        let mut source_addresses = Vec::with_capacity(n);
        for _ in 0..n {
            source_addresses.push(read_addr(data, pos));
            pos += IP_ADDR_LEN;
        }
        Ok(Self {
            ip_protocol_version,
            ip_address,
            multicast_port,
            include_sources,
            source_addresses,
        })
    }
    fn data_len(&self) -> usize {
        MULTICAST_FIXED + self.source_addresses.len() * IP_ADDR_LEN
    }
}

impl Serialize for MulticastDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        descriptor_len(self.data_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = write_descriptor_header(MULTICAST_DESCRIPTOR_TAG, self.data_len(), buf)?;
        buf[pos] = self.ip_protocol_version.to_u8();
        buf[pos + 1..pos + 1 + IP_ADDR_LEN].copy_from_slice(&self.ip_address);
        let p = pos + 1 + IP_ADDR_LEN;
        buf[p..p + 2].copy_from_slice(&self.multicast_port.to_be_bytes());
        // reserved(7)='0000000' + include_sources(1).
        buf[p + 2] = u8::from(self.include_sources);
        buf[p + 3] = self.source_addresses.len() as u8;
        pos += MULTICAST_FIXED;
        for a in &self.source_addresses {
            buf[pos..pos + IP_ADDR_LEN].copy_from_slice(a);
            pos += IP_ADDR_LEN;
        }
        Ok(pos)
    }
}

// --- Shared 2-byte (tag + length) descriptor header helpers (Tables 83/85) ---

// descriptor_tag(1) + descriptor_length(1).
const DESCRIPTOR_HEADER: usize = 2;

fn parse_descriptor_header<'a>(
    bytes: &'a [u8],
    expected_tag: u8,
    what: &'static str,
) -> Result<&'a [u8]> {
    if bytes.len() < DESCRIPTOR_HEADER {
        return Err(Error::BufferTooShort {
            need: DESCRIPTOR_HEADER,
            have: bytes.len(),
            what,
        });
    }
    if bytes[0] != expected_tag {
        return Err(Error::InvalidObject {
            what,
            reason: "unexpected descriptor_tag",
        });
    }
    let len = bytes[1] as usize;
    let end = DESCRIPTOR_HEADER + len;
    if bytes.len() < end {
        return Err(Error::LengthMismatch {
            what,
            declared: len,
            actual: bytes.len().saturating_sub(DESCRIPTOR_HEADER),
        });
    }
    Ok(&bytes[DESCRIPTOR_HEADER..end])
}

fn descriptor_len(data_len: usize) -> usize {
    DESCRIPTOR_HEADER + data_len
}

fn write_descriptor_header(tag: u8, data_len: usize, buf: &mut [u8]) -> Result<usize> {
    let total = descriptor_len(data_len);
    if buf.len() < total {
        return Err(Error::OutputBufferTooSmall {
            need: total,
            have: buf.len(),
        });
    }
    if data_len > u8::MAX as usize {
        return Err(Error::LengthTooLarge(data_len));
    }
    buf[0] = tag;
    buf[1] = data_len as u8;
    Ok(DESCRIPTOR_HEADER)
}

// ---------------------------------------------------------------------------
// Tag-dispatch helper (no resource_id — see module doc)
// ---------------------------------------------------------------------------

/// A parsed LSC v4 extension APDU.
///
/// There is intentionally **no** `resource_id`-keyed entry point: TS 103 205 does
/// not print an LSC v4 resource_id (see the module doc). Dispatch is on the
/// apdu_tag alone, for callers already in an LSC session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LscV4Apdu {
    /// `comms_info_req` (`0x9F8C07`).
    CommsInfoReq(CommsInfoReq),
    /// `comms_info_reply` (`0x9F8C08`).
    CommsInfoReply(CommsInfoReply),
    /// `comms_IP_config_req` (`0x9F8C09`).
    CommsIpConfigReq(CommsIpConfigReq),
}

/// A parsed LSC v4 extension APDU that may carry an allocation
/// (`comms_IP_config_reply` has a DNS-server `Vec`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LscV4ReplyApdu {
    /// `comms_IP_config_reply` (`0x9F8C0A`).
    CommsIpConfigReply(CommsIpConfigReply),
}

impl LscV4Apdu {
    /// Parse a fixed-size LSC v4 extension APDU by its apdu_tag. Returns
    /// `Ok(None)` for `comms_IP_config_reply`, which allocates and is returned by
    /// [`parse_ip_config_reply`].
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "lsc_v4 apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::COMMS_INFO_REQ => Ok(Self::CommsInfoReq(CommsInfoReq::parse(body)?)),
            tag::COMMS_INFO_REPLY => Ok(Self::CommsInfoReply(CommsInfoReply::parse(body)?)),
            tag::COMMS_IP_CONFIG_REQ => Ok(Self::CommsIpConfigReq(CommsIpConfigReq::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::COMMS_INFO_REQ.as_u24(),
                what: "lsc_v4",
            }),
        }
    }
}

/// Parse a `comms_IP_config_reply` (`0x9F8C0A`) APDU.
pub fn parse_ip_config_reply(body: &[u8]) -> Result<CommsIpConfigReply> {
    CommsIpConfigReply::parse(body)
}

impl Serialize for LscV4Apdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::CommsInfoReq(o) => o.serialized_len(),
            Self::CommsInfoReply(o) => o.serialized_len(),
            Self::CommsIpConfigReq(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::CommsInfoReq(o) => o.serialize_into(buf),
            Self::CommsInfoReply(o) => o.serialize_into(buf),
            Self::CommsIpConfigReq(o) => o.serialize_into(buf),
        }
    }
}

impl Serialize for LscV4ReplyApdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::CommsIpConfigReply(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::CommsIpConfigReply(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP_A: [u8; IP_ADDR_LEN] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xC0, 0xA8, 0x01,
        0x0A,
    ];
    const IP_B: [u8; IP_ADDR_LEN] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x08, 0x08, 0x08,
        0x08,
    ];

    #[test]
    fn info_req_round_trips() {
        let bytes = CommsInfoReq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x8C, 0x07, 0x00]);
        assert_eq!(CommsInfoReq::parse(&bytes).unwrap(), CommsInfoReq);
    }

    #[test]
    fn info_reply_round_trips_and_bites() {
        let r = CommsInfoReply {
            lts_id: 0x07,
            status: true,
            source_ip_address: IP_A,
            source_port: 0x1234,
            input_delivery_pid: 0x0100,
        };
        let bytes = r.to_bytes();
        // body=22=0x16. LTS(07) status(01) IP(16) port(12 34) pid(01 00).
        assert_eq!(bytes[0..4], [0x9F, 0x8C, 0x08, 0x16]);
        assert_eq!(bytes[4], 0x07);
        assert_eq!(bytes[5], 0x01); // status
        assert_eq!(&bytes[6..22], &IP_A);
        assert_eq!(&bytes[22..24], &[0x12, 0x34]);
        assert_eq!(&bytes[24..26], &[0x01, 0x00]);
        assert_eq!(CommsInfoReply::parse(&bytes).unwrap(), r);
        let mut other = r;
        other.status = false;
        assert_eq!(other.to_bytes()[5], 0x00);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn info_reply_pid_is_13_bit_masked() {
        // Top 3 bits of the PID word are reserved.
        let bytes = [
            0x9F, 0x8C, 0x08, 0x16, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0x00, 0x00, 0xFF, 0xFE,
        ];
        let r = CommsInfoReply::parse(&bytes).unwrap();
        assert_eq!(r.input_delivery_pid, 0x1FFE);
    }

    #[test]
    fn ip_config_req_round_trips() {
        let bytes = CommsIpConfigReq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x8C, 0x09, 0x00]);
        assert_eq!(CommsIpConfigReq::parse(&bytes).unwrap(), CommsIpConfigReq);
    }

    #[test]
    fn ip_config_reply_disconnected_round_trips() {
        let r = CommsIpConfigReply {
            connection_state: ConnectionState::Disconnected,
            physical_address: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            ip_config: None,
        };
        let bytes = r.to_bytes();
        // body=7: conn_state(00) MAC(6). connection_state 0 in high 2 bits => 0x00.
        assert_eq!(
            bytes,
            [
                0x9F, 0x8C, 0x0A, 0x07, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55
            ]
        );
        assert_eq!(CommsIpConfigReply::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn ip_config_reply_connected_two_dns_round_trips_and_bites() {
        let r = CommsIpConfigReply {
            connection_state: ConnectionState::Connected,
            physical_address: [0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F],
            ip_config: Some(IpConfig {
                ip_address: IP_A,
                network_mask: IP_B,
                default_gateway: IP_A,
                dhcp_server_address: IP_B,
                dns_server_addresses: alloc::vec![IP_A, IP_B],
            }),
        };
        let bytes = r.to_bytes();
        // connection_state Connected(01) in high 2 bits => 0x40.
        assert_eq!(bytes[0..4], [0x9F, 0x8C, 0x0A, (7 + 65 + 32) as u8]);
        assert_eq!(bytes[4], 0x40);
        assert_eq!(&bytes[5..11], &[0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F]);
        // num_DNS_servers at offset 11 + 4*16 = 75.
        assert_eq!(bytes[4 + 7 + 4 * IP_ADDR_LEN], 0x02);
        assert_eq!(CommsIpConfigReply::parse(&bytes).unwrap(), r);
        let mut other = r.clone();
        if let Some(c) = &mut other.ip_config {
            c.dns_server_addresses.pop();
        }
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn hybrid_descriptor_round_trips_and_bites() {
        let h = HybridDescriptor {
            lts_id: 0x05,
            ip_connection_type: IpConnectionType::IpDescriptor,
            inner: &[0xDE, 0xAD],
        };
        let bytes = h.to_bytes();
        // tag(05) len(04) LTS(05) conn_type(03) inner(DE AD).
        assert_eq!(bytes, [0x05, 0x04, 0x05, 0x03, 0xDE, 0xAD]);
        assert_eq!(HybridDescriptor::parse(&bytes).unwrap(), h);
        let mut other = h;
        other.lts_id = 0x06;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn multicast_descriptor_two_sources_round_trips_and_bites() {
        let m = MulticastDescriptor {
            ip_protocol_version: IpProtocolVersion::Ipv4,
            ip_address: IP_A,
            multicast_port: 0x1389,
            include_sources: true,
            source_addresses: alloc::vec![IP_A, IP_B],
        };
        let bytes = m.to_bytes();
        // data = 1 + 16 + 2 + 1 + 1 + 2*16 = 53. total = 55.
        assert_eq!(bytes[0], MULTICAST_DESCRIPTOR_TAG);
        assert_eq!(bytes[1], 53);
        assert_eq!(bytes[2], 0x01); // IPv4
        assert_eq!(&bytes[3..19], &IP_A);
        assert_eq!(&bytes[19..21], &[0x13, 0x89]);
        assert_eq!(bytes[21], 0x01); // include_sources
        assert_eq!(bytes[22], 0x02); // num_source_addresses
        assert_eq!(MulticastDescriptor::parse(&bytes).unwrap(), m);
        let mut other = m.clone();
        other.include_sources = false;
        assert_eq!(other.to_bytes()[21], 0x00);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn multicast_descriptor_any_source() {
        let m = MulticastDescriptor {
            ip_protocol_version: IpProtocolVersion::Ipv6,
            ip_address: IP_B,
            multicast_port: 5004,
            include_sources: false,
            source_addresses: Vec::new(),
        };
        let bytes = m.to_bytes();
        assert_eq!(bytes[1], 21); // data = 1+16+2+1+1 = 21
        assert_eq!(MulticastDescriptor::parse(&bytes).unwrap(), m);
    }

    #[test]
    fn dispatch_routes_fixed_tags() {
        assert!(matches!(
            LscV4Apdu::parse(&CommsInfoReq.to_bytes()).unwrap(),
            LscV4Apdu::CommsInfoReq(_)
        ));
        let reply = CommsInfoReply {
            lts_id: 0,
            status: false,
            source_ip_address: [0; IP_ADDR_LEN],
            source_port: 0,
            input_delivery_pid: 0,
        };
        assert!(matches!(
            LscV4Apdu::parse(&reply.to_bytes()).unwrap(),
            LscV4Apdu::CommsInfoReply(_)
        ));
        assert!(matches!(
            LscV4Apdu::parse(&CommsIpConfigReq.to_bytes()).unwrap(),
            LscV4Apdu::CommsIpConfigReq(_)
        ));
        // comms_IP_config_reply routes via the allocating helper, not LscV4Apdu.
        let cfg = CommsIpConfigReply {
            connection_state: ConnectionState::Disconnected,
            physical_address: [0; MAC_LEN],
            ip_config: None,
        };
        let cb = cfg.to_bytes();
        assert_eq!(parse_ip_config_reply(&cb).unwrap(), cfg);
        // It is NOT a member of the fixed-size dispatch set.
        assert!(matches!(
            LscV4Apdu::parse(&cb),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
