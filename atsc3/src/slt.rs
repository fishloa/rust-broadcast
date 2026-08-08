//! Service List Table (SLT) — ATSC A/331:2025-06 §6.3, Table 6.2.
//!
//! Root element `SLT`, namespace
//! `tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/SLT/1.0/` (not enforced by
//! this parser — the local element/attribute names are matched regardless of
//! namespace prefix, since real encoders vary in whether/how they bind it).
//! Function is analogous to MPEG-2 PAT / ATSC A/153 FIC: the rapid-channel-
//! scan bootstrap, carried gzip-compressed as an [`crate::lls::LlsEnvelope`]
//! payload with `table_id` [`crate::lls_table_id::LlsTableId::Slt`].
//!
//! This is a first pass covering the fields named in the crate's initial
//! scope: `SLT@bsid`, and per `Service`: `serviceId`, `majorChannelNo`,
//! `minorChannelNo`, `serviceCategory`, `shortServiceName`, `hidden`, and the
//! `BroadcastSvcSignaling` child (`slsProtocol` +
//! `slsMajorProtocolVersion`/`slsMinorProtocolVersion` +
//! `slsDestinationIpAddress`/`slsDestinationUdpPort`/`slsSourceIpAddress`).
//! `docs/a331-signalling.md` Table 6.2 documents further `SLT`/`Service`
//! attributes (`globalServiceID`, `sltSvcSeqNum`, `protected`,
//! `hideInGuide`, `SvcInetUrl`, `OtherBsid`/`OtherRf`, …) not yet modeled
//! here — deferred, not fabricated.

use crate::error::{Error, Result};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::Ipv4Addr;

/// `SLT` root element (Table 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Slt {
    /// `SLT@bsid` — Broadcast Stream ID(s); more than one when the Service
    /// is channel-bonded. Matches `L1D_bsid` in physical-layer L1-Detail
    /// signaling (A/322).
    pub bsid: Vec<u16>,
    /// One entry per `Service` element (`Use` = `1..N`).
    pub services: Vec<SltService>,
}

/// One `SLT.Service` element (Table 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SltService {
    /// `Service@serviceId` — uniquely identifies the Service within one set
    /// of bonded PLPs.
    pub service_id: u16,
    /// `Service@majorChannelNo` — major channel number (1-999); absent for
    /// non-user-selectable Services (e.g. ESG).
    pub major_channel_no: Option<u16>,
    /// `Service@minorChannelNo` — minor channel number (1-999).
    pub minor_channel_no: Option<u16>,
    /// `Service@serviceCategory` — Service type (Table 6.4).
    pub service_category: ServiceCategory,
    /// `Service@shortServiceName` — short display name (<=7 chars per spec;
    /// not enforced here).
    pub short_service_name: Option<String>,
    /// `Service@hidden` — not directly channel-surfable/enterable (test
    /// signals, NVOD). Default `false`.
    pub hidden: bool,
    /// `Service.BroadcastSvcSignaling` — broadcast SLS bootstrap info.
    pub broadcast_svc_signaling: Option<BroadcastSvcSignaling>,
}

/// `BroadcastSvcSignaling` — SLS bootstrap info (Table 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BroadcastSvcSignaling {
    /// `@slsProtocol` — Table 6.5 (1=ROUTE, 2=MMTP).
    pub sls_protocol: SlsProtocol,
    /// `@slsMajorProtocolVersion` — default `1`.
    pub sls_major_protocol_version: u8,
    /// `@slsMinorProtocolVersion` — default `0`.
    pub sls_minor_protocol_version: u8,
    /// `@slsDestinationIpAddress` — destination IP of the SLS-carrying LCT
    /// channel/MMTP session.
    pub sls_destination_ip_address: Ipv4Addr,
    /// `@slsDestinationUdpPort` — destination port of the same.
    pub sls_destination_udp_port: u16,
    /// `@slsSourceIpAddress` — source IP; required when `@slsProtocol=1`
    /// (ROUTE), not enforced here.
    pub sls_source_ip_address: Option<Ipv4Addr>,
}

/// `SLT.Service@serviceCategory` code values (Table 6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ServiceCategory {
    /// `1` — Linear A/V Service.
    LinearAv,
    /// `2` — Linear audio-only Service.
    LinearAudioOnly,
    /// `3` — App-Based Service.
    AppBased,
    /// `4` — ESG Service (program guide).
    Esg,
    /// `5` — (Deprecated).
    Deprecated,
    /// `6` — DRM Data Service.
    DrmData,
    /// `7` — Data Service.
    Data,
    /// `0`, `8`-`255` — ATSC Reserved.
    Reserved(u8),
}

impl ServiceCategory {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LinearAv => "Linear A/V Service",
            Self::LinearAudioOnly => "Linear audio-only Service",
            Self::AppBased => "App-Based Service",
            Self::Esg => "ESG Service",
            Self::Deprecated => "Deprecated",
            Self::DrmData => "DRM Data Service",
            Self::Data => "Data Service",
            Self::Reserved(_) => "reserved",
        }
    }

    /// Decode from the wire `serviceCategory` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::LinearAv,
            2 => Self::LinearAudioOnly,
            3 => Self::AppBased,
            4 => Self::Esg,
            5 => Self::Deprecated,
            6 => Self::DrmData,
            7 => Self::Data,
            other => Self::Reserved(other),
        }
    }

    /// Encode back to the wire `serviceCategory` byte.
    #[must_use]
    pub fn to_u8(&self) -> u8 {
        match self {
            Self::LinearAv => 1,
            Self::LinearAudioOnly => 2,
            Self::AppBased => 3,
            Self::Esg => 4,
            Self::Deprecated => 5,
            Self::DrmData => 6,
            Self::Data => 7,
            Self::Reserved(v) => *v,
        }
    }
}

broadcast_common::impl_spec_display!(ServiceCategory, Reserved);

/// `BroadcastSvcSignaling@slsProtocol` code values (Table 6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SlsProtocol {
    /// `1` — ROUTE.
    Route,
    /// `2` — MMTP.
    Mmtp,
    /// `0`, `3`-`255` — ATSC Reserved.
    Reserved(u8),
}

impl SlsProtocol {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Route => "ROUTE",
            Self::Mmtp => "MMTP",
            Self::Reserved(_) => "reserved",
        }
    }

    /// Decode from the wire `slsProtocol` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Route,
            2 => Self::Mmtp,
            other => Self::Reserved(other),
        }
    }

    /// Encode back to the wire `slsProtocol` byte.
    #[must_use]
    pub fn to_u8(&self) -> u8 {
        match self {
            Self::Route => 1,
            Self::Mmtp => 2,
            Self::Reserved(v) => *v,
        }
    }
}

broadcast_common::impl_spec_display!(SlsProtocol, Reserved);

const EL_SLT: &str = "SLT";
const EL_SERVICE: &str = "Service";
const EL_BROADCAST_SVC_SIGNALING: &str = "BroadcastSvcSignaling";

impl Slt {
    /// Parse an `SLT` XML document (A/331 §6.3, Table 6.2).
    ///
    /// # Errors
    /// [`Error::XmlParse`] if `xml` is not well-formed; [`Error::MissingElement`]
    /// if the root isn't `SLT` or no `Service` child is present;
    /// [`Error::MissingAttribute`]/[`Error::InvalidAttribute`] for a missing or
    /// malformed required attribute.
    pub fn parse(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::XmlParse {
            what: EL_SLT,
            reason: e.to_string(),
        })?;

        let root = doc.root_element();
        if root.tag_name().name() != EL_SLT {
            return Err(Error::MissingElement {
                what: "SLT document",
                element: EL_SLT,
            });
        }

        let bsid = parse_u16_list_attr(&root, EL_SLT, "bsid")?;

        let services: Vec<SltService> = root
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == EL_SERVICE)
            .map(SltService::from_node)
            .collect::<Result<_>>()?;

        if services.is_empty() {
            return Err(Error::MissingElement {
                what: EL_SLT,
                element: EL_SERVICE,
            });
        }

        Ok(Self { bsid, services })
    }
}

impl SltService {
    fn from_node(node: roxmltree::Node) -> Result<Self> {
        let service_id = parse_u16_attr(&node, EL_SERVICE, "serviceId")?;
        let major_channel_no = parse_opt_u16_attr(&node, EL_SERVICE, "majorChannelNo")?;
        let minor_channel_no = parse_opt_u16_attr(&node, EL_SERVICE, "minorChannelNo")?;
        let service_category =
            ServiceCategory::from_u8(parse_u8_attr(&node, EL_SERVICE, "serviceCategory")?);
        let short_service_name = node.attribute("shortServiceName").map(str::to_string);
        let hidden = parse_bool_attr(&node, EL_SERVICE, "hidden", false)?;

        let broadcast_svc_signaling = node
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == EL_BROADCAST_SVC_SIGNALING)
            .map(BroadcastSvcSignaling::from_node)
            .transpose()?;

        Ok(Self {
            service_id,
            major_channel_no,
            minor_channel_no,
            service_category,
            short_service_name,
            hidden,
            broadcast_svc_signaling,
        })
    }
}

impl BroadcastSvcSignaling {
    fn from_node(node: roxmltree::Node) -> Result<Self> {
        let sls_protocol = SlsProtocol::from_u8(parse_u8_attr(
            &node,
            EL_BROADCAST_SVC_SIGNALING,
            "slsProtocol",
        )?);
        let sls_major_protocol_version =
            parse_opt_u8_attr(&node, EL_BROADCAST_SVC_SIGNALING, "slsMajorProtocolVersion")?
                .unwrap_or(1);
        let sls_minor_protocol_version =
            parse_opt_u8_attr(&node, EL_BROADCAST_SVC_SIGNALING, "slsMinorProtocolVersion")?
                .unwrap_or(0);
        let sls_destination_ip_address =
            parse_ipv4_attr(&node, EL_BROADCAST_SVC_SIGNALING, "slsDestinationIpAddress")?;
        let sls_destination_udp_port =
            parse_u16_attr(&node, EL_BROADCAST_SVC_SIGNALING, "slsDestinationUdpPort")?;
        let sls_source_ip_address =
            parse_opt_ipv4_attr(&node, EL_BROADCAST_SVC_SIGNALING, "slsSourceIpAddress")?;

        Ok(Self {
            sls_protocol,
            sls_major_protocol_version,
            sls_minor_protocol_version,
            sls_destination_ip_address,
            sls_destination_udp_port,
            sls_source_ip_address,
        })
    }
}

fn required_attr<'a>(
    node: &'a roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<&'a str> {
    node.attribute(attr)
        .ok_or(Error::MissingAttribute { element, attr })
}

fn parse_u8_attr(node: &roxmltree::Node, element: &'static str, attr: &'static str) -> Result<u8> {
    let raw = required_attr(node, element, attr)?;
    raw.parse().map_err(|_| Error::InvalidAttribute {
        element,
        attr,
        reason: alloc::format!("{raw:?} is not a valid unsignedByte"),
    })
}

fn parse_opt_u8_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<Option<u8>> {
    match node.attribute(attr) {
        None => Ok(None),
        Some(raw) => raw.parse().map(Some).map_err(|_| Error::InvalidAttribute {
            element,
            attr,
            reason: alloc::format!("{raw:?} is not a valid unsignedByte"),
        }),
    }
}

fn parse_u16_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<u16> {
    let raw = required_attr(node, element, attr)?;
    raw.parse().map_err(|_| Error::InvalidAttribute {
        element,
        attr,
        reason: alloc::format!("{raw:?} is not a valid unsignedShort"),
    })
}

fn parse_opt_u16_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<Option<u16>> {
    match node.attribute(attr) {
        None => Ok(None),
        Some(raw) => raw.parse().map(Some).map_err(|_| Error::InvalidAttribute {
            element,
            attr,
            reason: alloc::format!("{raw:?} is not a valid unsignedShort"),
        }),
    }
}

fn parse_u16_list_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<Vec<u16>> {
    let raw = required_attr(node, element, attr)?;
    raw.split_whitespace()
        .map(|s| {
            s.parse::<u16>().map_err(|_| Error::InvalidAttribute {
                element,
                attr,
                reason: alloc::format!("{s:?} is not a valid unsignedShort"),
            })
        })
        .collect()
}

fn parse_bool_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
    default: bool,
) -> Result<bool> {
    match node.attribute(attr) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(raw) => Err(Error::InvalidAttribute {
            element,
            attr,
            reason: alloc::format!("{raw:?} is not a valid xsd:boolean"),
        }),
    }
}

fn parse_ipv4_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<Ipv4Addr> {
    let raw = required_attr(node, element, attr)?;
    raw.parse().map_err(|_| Error::InvalidAttribute {
        element,
        attr,
        reason: alloc::format!("{raw:?} is not a valid IPv4 address"),
    })
}

fn parse_opt_ipv4_attr(
    node: &roxmltree::Node,
    element: &'static str,
    attr: &'static str,
) -> Result<Option<Ipv4Addr>> {
    match node.attribute(attr) {
        None => Ok(None),
        Some(raw) => raw.parse().map(Some).map_err(|_| Error::InvalidAttribute {
            element,
            attr,
            reason: alloc::format!("{raw:?} is not a valid IPv4 address"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<SLT xmlns="tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/SLT/1.0/" bsid="1 2">
  <Service serviceId="1" majorChannelNo="11" minorChannelNo="1"
           serviceCategory="1" shortServiceName="KSTN-TV" hidden="false">
    <BroadcastSvcSignaling slsProtocol="1"
                            slsDestinationIpAddress="224.0.1.1"
                            slsDestinationUdpPort="5000"
                            slsSourceIpAddress="10.0.0.1"/>
  </Service>
  <Service serviceId="2" serviceCategory="4" hidden="true"/>
</SLT>"#;

    #[test]
    fn parses_full_slt() {
        let slt = Slt::parse(SLT_XML).unwrap();
        assert_eq!(slt.bsid, alloc::vec![1, 2]);
        assert_eq!(slt.services.len(), 2);

        let svc1 = &slt.services[0];
        assert_eq!(svc1.service_id, 1);
        assert_eq!(svc1.major_channel_no, Some(11));
        assert_eq!(svc1.minor_channel_no, Some(1));
        assert_eq!(svc1.service_category, ServiceCategory::LinearAv);
        assert_eq!(svc1.short_service_name.as_deref(), Some("KSTN-TV"));
        assert!(!svc1.hidden);

        let bss = svc1.broadcast_svc_signaling.as_ref().unwrap();
        assert_eq!(bss.sls_protocol, SlsProtocol::Route);
        assert_eq!(bss.sls_major_protocol_version, 1);
        assert_eq!(bss.sls_minor_protocol_version, 0);
        assert_eq!(bss.sls_destination_ip_address, Ipv4Addr::new(224, 0, 1, 1));
        assert_eq!(bss.sls_destination_udp_port, 5000);
        assert_eq!(bss.sls_source_ip_address, Some(Ipv4Addr::new(10, 0, 0, 1)));

        let svc2 = &slt.services[1];
        assert_eq!(svc2.service_id, 2);
        assert_eq!(svc2.major_channel_no, None);
        assert_eq!(svc2.service_category, ServiceCategory::Esg);
        assert_eq!(svc2.short_service_name, None);
        assert!(svc2.hidden);
        assert!(svc2.broadcast_svc_signaling.is_none());
    }

    #[test]
    fn rejects_non_slt_root() {
        let err = Slt::parse("<NotSLT/>").unwrap_err();
        assert!(matches!(err, Error::MissingElement { element: "SLT", .. }));
    }

    #[test]
    fn rejects_missing_bsid() {
        let err =
            Slt::parse("<SLT><Service serviceId=\"1\" serviceCategory=\"1\"/></SLT>").unwrap_err();
        assert!(matches!(
            err,
            Error::MissingAttribute {
                element: "SLT",
                attr: "bsid"
            }
        ));
    }

    #[test]
    fn rejects_no_services() {
        let err = Slt::parse(r#"<SLT bsid="1"></SLT>"#).unwrap_err();
        assert!(matches!(
            err,
            Error::MissingElement {
                element: "Service",
                ..
            }
        ));
    }

    #[test]
    fn rejects_invalid_service_category() {
        let err = Slt::parse(
            r#"<SLT bsid="1"><Service serviceId="1" serviceCategory="not-a-number"/></SLT>"#,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidAttribute {
                element: "Service",
                attr: "serviceCategory",
                ..
            }
        ));
    }

    #[test]
    fn unknown_service_category_is_reserved() {
        let slt =
            Slt::parse(r#"<SLT bsid="1"><Service serviceId="1" serviceCategory="200"/></SLT>"#)
                .unwrap();
        assert_eq!(
            slt.services[0].service_category,
            ServiceCategory::Reserved(200)
        );
    }

    #[test]
    fn service_category_round_trips_all_bytes() {
        for v in 0u8..=255 {
            assert_eq!(ServiceCategory::from_u8(v).to_u8(), v);
        }
    }

    #[test]
    fn sls_protocol_round_trips_all_bytes() {
        for v in 0u8..=255 {
            assert_eq!(SlsProtocol::from_u8(v).to_u8(), v);
        }
    }
}
