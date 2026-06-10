use super::*;

impl ExtensionBodyDef for ServiceProminence<'_> {
    const TAG_EXTENSION: u8 = 0x22;
    const NAME: &'static str = "SERVICE_PROMINENCE";
}

// ===========================================================================
//  Section 0x22 — service_prominence_descriptor (Table 162c, §6.4.18)
// ---------------------------------------------------------------------------
//  SOGI_list_length then a variable-length SOGI loop (unfolded as Vec<SogiEntry>);
//  each entry's target_region sub-loop is kept raw (region_depth-irregular,
//  same precedent as TargetRegion). Trailing private_data_byte run.
// ===========================================================================

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
