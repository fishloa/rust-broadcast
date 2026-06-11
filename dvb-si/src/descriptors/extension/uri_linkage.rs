//! URI Linkage Descriptor — ETSI EN 300 468 §6.4.16.1 (tag_extension 0x13).
use super::*;

impl<'a> ExtensionBodyDef<'a> for UriLinkage<'a> {
    const TAG_EXTENSION: u8 = 0x13;
    const NAME: &'static str = "URI_LINKAGE";
}

/// URI linkage type — coded according to ETSI TS 101 162 registry, non-exhaustive.
///
/// Names are best-effort from the TS 101 162 registry; new values may be
/// registered at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UriLinkageType {
    /// Online SDT (Service Discovery & Selection).
    OnlineSdt,
    /// DVB-IPTV SD&S.
    DvbIptvSds,
    /// Material resolution server.
    MaterialResolutionServer,
    /// DVB-I service list.
    DvbIServiceList,
    /// Other / unregistered value.
    Other(u8),
}

impl UriLinkageType {
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => UriLinkageType::OnlineSdt,
            0x01 => UriLinkageType::DvbIptvSds,
            0x02 => UriLinkageType::MaterialResolutionServer,
            0x03 => UriLinkageType::DvbIServiceList,
            other => UriLinkageType::Other(other),
        }
    }

    /// Inverse of `from_u8`; `Self::Other` emits its stored value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            UriLinkageType::OnlineSdt => 0x00,
            UriLinkageType::DvbIptvSds => 0x01,
            UriLinkageType::MaterialResolutionServer => 0x02,
            UriLinkageType::DvbIServiceList => 0x03,
            UriLinkageType::Other(v) => v,
        }
    }

    /// Best-effort human-readable name from the TS 101 162 registry.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            UriLinkageType::OnlineSdt => "online SDT (Service Discovery & Selection)",
            UriLinkageType::DvbIptvSds => "DVB-IPTV SD&S",
            UriLinkageType::MaterialResolutionServer => "material resolution server",
            UriLinkageType::DvbIServiceList => "DVB-I service list",
            UriLinkageType::Other(_) => "other (TS 101 162 registry)",
        }
    }

    /// `true` when this type carries a `min_polling_interval` field per
    /// EN 300 468 Table 159 (types `0x00` and `0x01`).
    #[must_use]
    pub fn has_polling_interval(self) -> bool {
        matches!(self, UriLinkageType::OnlineSdt | UriLinkageType::DvbIptvSds)
    }
}

/// URI_linkage body (Table 159).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct UriLinkage<'a> {
    /// uri_linkage_type(8) — TS 101 162 registry.
    pub uri_linkage_type: UriLinkageType,
    /// Length-delimited `uri_char` text.
    pub uri: crate::text::DvbText<'a>,
    /// min_polling_interval(16), present iff `uri_linkage_type` is 0x00 or 0x01.
    pub min_polling_interval: Option<u16>,
    /// Trailing private_data_byte run.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for UriLinkage<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: sel.len(),
                what: "URI_linkage body",
            });
        }
        let uri_linkage_type = UriLinkageType::from_u8(sel[0]);
        let uri_length = sel[1] as usize;
        let mut pos = 2;
        if sel.len() < pos + uri_length {
            return Err(Error::BufferTooShort {
                need: pos + uri_length,
                have: sel.len(),
                what: "URI_linkage body",
            });
        }
        let uri = crate::text::DvbText::new(&sel[pos..pos + uri_length]);
        pos += uri_length;
        let min_polling_interval = if uri_linkage_type.has_polling_interval() {
            if sel.len() < pos + 2 {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: sel.len(),
                    what: "URI_linkage body",
                });
            }
            let v = u16::from_be_bytes([sel[pos], sel[pos + 1]]);
            pos += 2;
            Some(v)
        } else {
            None
        };
        Ok(UriLinkage {
            uri_linkage_type,
            uri,
            min_polling_interval,
            private_data: &sel[pos..],
        })
    }
}

impl Serialize for UriLinkage<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        2 + self.uri.len()
            + if self.min_polling_interval.is_some() {
                2
            } else {
                0
            }
            + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.uri_linkage_type.to_u8();
        buf[1] = self.uri.len() as u8;
        let mut p = 2;
        buf[p..p + self.uri.len()].copy_from_slice(self.uri.raw());
        p += self.uri.len();
        if let Some(mpi) = self.min_polling_interval {
            buf[p..p + 2].copy_from_slice(&mpi.to_be_bytes());
            p += 2;
        }
        buf[p..p + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_uri_linkage_with_polling() {
        let uri = b"http://x";
        let mut sel = vec![0x00, uri.len() as u8];
        sel.extend_from_slice(uri);
        sel.extend_from_slice(&0x1234u16.to_be_bytes());
        sel.push(0xFE); // private
        let bytes = wrap(0x13, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::UriLinkage(b) => {
                assert_eq!(b.uri_linkage_type, UriLinkageType::OnlineSdt);
                assert_eq!(b.uri.raw(), uri);
                assert_eq!(b.uri.decode(), "http://x");
                assert_eq!(b.min_polling_interval, Some(0x1234));
                assert_eq!(b.private_data, &[0xFE]);
            }
            other => panic!("expected UriLinkage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_uri_linkage_no_polling() {
        // type 0x02 ⇒ no min_polling_interval
        let uri = b"dvb:";
        let mut sel = vec![0x02, uri.len() as u8];
        sel.extend_from_slice(uri);
        let bytes = wrap(0x13, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::UriLinkage(b) => {
                assert_eq!(b.min_polling_interval, None);
                assert_eq!(b.uri.decode(), "dvb:");
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected UriLinkage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_uri_linkage_rejects_overrun() {
        let sel = [0x02, 0x10, 0xAA]; // uri_length 16 but 1 byte present
        let bytes = wrap(0x13, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn uri_linkage_type_roundtrip() {
        for b in 0u8..=0xFF {
            assert_eq!(UriLinkageType::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn uri_linkage_type_name() {
        assert_eq!(
            UriLinkageType::OnlineSdt.name(),
            "online SDT (Service Discovery & Selection)"
        );
        assert_eq!(UriLinkageType::DvbIptvSds.name(), "DVB-IPTV SD&S");
        assert_eq!(
            UriLinkageType::MaterialResolutionServer.name(),
            "material resolution server"
        );
        assert_eq!(UriLinkageType::DvbIServiceList.name(), "DVB-I service list");
        assert_eq!(
            UriLinkageType::Other(0xFF).name(),
            "other (TS 101 162 registry)"
        );
    }

    #[test]
    fn uri_linkage_type_has_polling_interval() {
        assert!(UriLinkageType::OnlineSdt.has_polling_interval());
        assert!(UriLinkageType::DvbIptvSds.has_polling_interval());
        assert!(!UriLinkageType::MaterialResolutionServer.has_polling_interval());
        assert!(!UriLinkageType::DvbIServiceList.has_polling_interval());
        assert!(!UriLinkageType::Other(0x42).has_polling_interval());
    }
}
