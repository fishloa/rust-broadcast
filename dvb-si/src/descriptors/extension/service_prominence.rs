//! Service Prominence Descriptor — ETSI EN 300 468 §6.4.18 (tag_extension 0x22).
use super::*;

impl super::sealed::Sealed for ServiceProminence<'_> {}
impl ExtensionBodyDef for ServiceProminence<'_> {
    const TAG_EXTENSION: u8 = 0x22;
    const NAME: &'static str = "SERVICE_PROMINENCE";
}

/// service_prominence body (Table 162c). The SOGI loop is unfolded;
/// each entry's target_region loop is kept raw (region entries are
/// region_depth-irregular — same precedent as [`TargetRegion`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ServiceProminence<'a> {
    /// SOGI entries (the `SOGI_list_length`-delimited loop).
    pub sogi_list: Vec<SogiEntry<'a>>,
    /// Trailing `private_data_byte` run.
    pub private_data: &'a [u8],
}

/// One SOGI entry (Table 162c inner loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SogiEntry<'a> {
    /// `SOGI_flag` (1 bit).
    pub sogi_flag: bool,
    /// `target_region_flag` (1 bit) — true iff `target_region_loop` is `Some`.
    pub target_region_flag: bool,
    /// `service_flag` (1 bit) — true iff `service_id` is `Some`.
    pub service_flag: bool,
    /// `SOGI_priority` (12 bits).
    pub sogi_priority: u16,
    /// `service_id` (16 bits), present iff `service_flag`.
    pub service_id: Option<u16>,
    /// Raw `target_region` loop (region_depth-irregular), present iff `target_region_flag`.
    pub target_region_loop: Option<&'a [u8]>,
}

impl<'a> Parse<'a> for ServiceProminence<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("service_prominence: sogi_list_length truncated"));
        }
        let sogi_list_length = sel[0] as usize;
        if sel.len() < 1 + sogi_list_length {
            return Err(invalid("service_prominence: sogi_list overruns body"));
        }
        let sogi_slice = &sel[1..1 + sogi_list_length];
        let mut sogi_list = Vec::new();
        let mut k = 0;
        while k < sogi_slice.len() {
            if sogi_slice.len() - k < 2 {
                return Err(invalid("service_prominence: SOGI entry overruns list"));
            }
            let byte0 = sogi_slice[k];
            let byte1 = sogi_slice[k + 1];
            let sogi_flag = (byte0 >> 7) != 0;
            let target_region_flag = ((byte0 >> 6) & 0x01) != 0;
            let service_flag = ((byte0 >> 5) & 0x01) != 0;
            let sogi_priority = ((u16::from(byte0) & 0x0F) << 8) | u16::from(byte1);
            k += 2;
            let service_id = if service_flag {
                if sogi_slice.len() - k < 2 {
                    return Err(invalid("service_prominence: SOGI entry overruns list"));
                }
                let id = u16::from_be_bytes([sogi_slice[k], sogi_slice[k + 1]]);
                k += 2;
                Some(id)
            } else {
                None
            };
            let target_region_loop = if target_region_flag {
                if sogi_slice.len() - k < 1 {
                    return Err(invalid("service_prominence: SOGI entry overruns list"));
                }
                let region_len = sogi_slice[k] as usize;
                k += 1;
                if sogi_slice.len() - k < region_len {
                    return Err(invalid("service_prominence: SOGI entry overruns list"));
                }
                let region = &sogi_slice[k..k + region_len];
                k += region_len;
                Some(region)
            } else {
                None
            };
            sogi_list.push(SogiEntry {
                sogi_flag,
                target_region_flag,
                service_flag,
                sogi_priority,
                service_id,
                target_region_loop,
            });
        }
        Ok(ServiceProminence {
            sogi_list,
            private_data: &sel[1 + sogi_list_length..],
        })
    }
}

impl Serialize for ServiceProminence<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let sogi_list_length: usize = self
            .sogi_list
            .iter()
            .map(|e| {
                2 + if e.service_flag { 2 } else { 0 }
                    + if e.target_region_flag {
                        1 + e.target_region_loop.map_or(0, |s| s.len())
                    } else {
                        0
                    }
            })
            .sum();
        1 + sogi_list_length + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let sogi_len = len - 1 - self.private_data.len();
        buf[0] = sogi_len as u8;
        let mut p = 1;
        for e in &self.sogi_list {
            buf[p] = ((e.sogi_flag as u8) << 7)
                | ((e.target_region_flag as u8) << 6)
                | ((e.service_flag as u8) << 5)
                | 0x10
                | ((e.sogi_priority >> 8) as u8 & 0x0F);
            buf[p + 1] = e.sogi_priority as u8;
            p += 2;
            if e.service_flag {
                if let Some(id) = e.service_id {
                    buf[p..p + 2].copy_from_slice(&id.to_be_bytes());
                }
                p += 2;
            }
            if e.target_region_flag {
                let region = e.target_region_loop.unwrap_or(&[]);
                buf[p] = region.len() as u8;
                p += 1;
                buf[p..p + region.len()].copy_from_slice(region);
                p += region.len();
            }
        }
        buf[p..p + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_service_prominence_one_entry_service_only() {
        // One SOGI entry: service_flag=1, target_region_flag=0,
        // sogi_priority=0x123, service_id=0x4567, private_data [0xAB]
        let sel = [0x04, 0x21, 0x23, 0x45, 0x67, 0xAB];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ServiceProminence));
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 1);
                let e = &b.sogi_list[0];
                assert!(!e.sogi_flag);
                assert!(!e.target_region_flag);
                assert!(e.service_flag);
                assert_eq!(e.sogi_priority, 0x0123);
                assert_eq!(e.service_id, Some(0x4567));
                assert!(e.target_region_loop.is_none());
                assert_eq!(b.private_data, &[0xAB]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_one_entry_target_region() {
        // One SOGI entry: service_flag=0, target_region_flag=1,
        // sogi_priority=0x001, target_region_loop = [0xAA, 0xBB].
        let sel = [0x05, 0x40, 0x01, 0x02, 0xAA, 0xBB];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 1);
                let e = &b.sogi_list[0];
                assert!(!e.sogi_flag);
                assert!(e.target_region_flag);
                assert!(!e.service_flag);
                assert_eq!(e.sogi_priority, 0x0001);
                assert!(e.service_id.is_none());
                assert_eq!(e.target_region_loop, Some([0xAAu8, 0xBB].as_slice()));
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_two_entries_plus_private() {
        // Two SOGI entries + private_data tail.
        // Entry 0: service_flag=1, sogi_priority=0xABC, service_id=0x1111.
        // Entry 1: target_region_flag=1, sogi_priority=0x345, region=[0xCC].
        let sel = [0x08, 0x2A, 0xBC, 0x11, 0x11, 0x43, 0x45, 0x01, 0xCC, 0xDD];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 2);
                let e0 = &b.sogi_list[0];
                assert!(e0.service_flag);
                assert_eq!(e0.sogi_priority, 0x0ABC);
                assert_eq!(e0.service_id, Some(0x1111));
                assert!(e0.target_region_loop.is_none());
                let e1 = &b.sogi_list[1];
                assert!(!e1.sogi_flag);
                assert!(e1.target_region_flag);
                assert!(!e1.service_flag);
                assert_eq!(e1.sogi_priority, 0x0345);
                assert_eq!(e1.target_region_loop, Some([0xCCu8].as_slice()));
                assert_eq!(b.private_data, &[0xDD]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_empty_list_private_only() {
        // SOGI_list_length=0, private=[0x01, 0x02]
        let sel = [0x00, 0x01, 0x02];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert!(b.sogi_list.is_empty());
                assert_eq!(b.private_data, &[0x01, 0x02]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_rejects_overrun() {
        // SOGI_list_length=5 but only 3 bytes follow
        let sel = [0x05, 0xAA, 0xBB, 0xCC];
        let bytes = wrap(0x22, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    #[test]
    fn parse_service_prominence_rejects_entry_overrun() {
        // SOGI_list_length=3, service_flag=1 but no service_id bytes follow
        let sel = [0x03, 0x20, 0x00, 0x00];
        let bytes = wrap(0x22, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_service_prominence() {
        let d = ExtensionDescriptor {
            tag_extension: 0x22,
            body: ExtensionBody::ServiceProminence(ServiceProminence {
                sogi_list: vec![SogiEntry {
                    sogi_flag: false,
                    target_region_flag: false,
                    service_flag: true,
                    sogi_priority: 0x123,
                    service_id: Some(0x4567),
                    target_region_loop: None,
                }],
                private_data: &[0xAB],
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":34"));
        assert!(json.contains("\"serviceProminence\""));
    }
}
