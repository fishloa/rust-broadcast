use super::*;

impl super::sealed::Sealed for Ac4<'_> {}
impl ExtensionBodyDef for Ac4<'_> {
    const TAG_EXTENSION: u8 = 0x15;
    const NAME: &'static str = "AC4";
}

// ===========================================================================
//  Section 0x15 — AC-4_descriptor (annex D, §D.5)
// ---------------------------------------------------------------------------
//  Two flags + a packed config byte (when ac4_config_flag set), a
//  length-delimited TOC, then additional_info bytes. The TOC + extra are kept
//  raw; flags + config are typed.
// ===========================================================================
/// AC-4 body (annex D). `toc` + `additional_info` are raw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Ac4<'a> {
    /// ac4_config_flag(1).
    pub ac4_config_flag: bool,
    /// ac4_toc_flag(1).
    pub ac4_toc_flag: bool,
    /// ac4_dialog_enhancement_enabled(1), present iff `ac4_config_flag`.
    pub ac4_dialog_enhancement_enabled: Option<bool>,
    /// ac4_channel_mode(2), present iff `ac4_config_flag`.
    pub ac4_channel_mode: Option<u8>,
    /// Length-delimited `ac4_toc` bytes, present iff `ac4_toc_flag`.
    ///
    /// Kept opaque by design: EN 300 468 carries the AC-4 TOC as an
    /// `ac4_toc_byte` run whose structure (`ac4_toc()`) is defined in the AC-4
    /// codec spec — ETSI TS 103 190-2 §6.2.1 (vendored at
    /// `specs/etsi_ts_103_190_2_v01.03.01_ac4.pdf`), not in DVB SI. Typing it is
    /// a separate, nested codec-bitstream effort tracked in issue #102.
    pub toc: Option<&'a [u8]>,
    /// Trailing additional_info_byte run.
    pub additional_info: &'a [u8],
}

impl<'a> Parse<'a> for Ac4<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("AC-4: flags byte missing"));
        }
        let flags = sel[0];
        let ac4_config_flag = (flags & 0x80) != 0;
        let ac4_toc_flag = (flags & 0x40) != 0;
        let mut pos = 1;
        let (ac4_dialog_enhancement_enabled, ac4_channel_mode) = if ac4_config_flag {
            if sel.len() < pos + 1 {
                return Err(invalid("AC-4: config byte truncated"));
            }
            let c = sel[pos];
            pos += 1;
            (Some((c & 0x80) != 0), Some((c >> 5) & 0x03))
        } else {
            (None, None)
        };
        let toc = if ac4_toc_flag {
            if sel.len() < pos + 1 {
                return Err(invalid("AC-4: toc length truncated"));
            }
            let toc_len = sel[pos] as usize;
            pos += 1;
            if sel.len() < pos + toc_len {
                return Err(invalid("AC-4: toc overruns body"));
            }
            let t = &sel[pos..pos + toc_len];
            pos += toc_len;
            Some(t)
        } else {
            None
        };
        Ok(Ac4 {
            ac4_config_flag,
            ac4_toc_flag,
            ac4_dialog_enhancement_enabled,
            ac4_channel_mode,
            toc,
            additional_info: &sel[pos..],
        })
    }
}

impl Serialize for Ac4<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + usize::from(self.ac4_config_flag)
            + self.toc.map_or(0, |t| 1 + t.len())
            + self.additional_info.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = (u8::from(self.ac4_config_flag) << 7) | (u8::from(self.ac4_toc_flag) << 6);
        let mut p = 1;
        if self.ac4_config_flag {
            let de = self.ac4_dialog_enhancement_enabled.unwrap_or(false);
            let cm = self.ac4_channel_mode.unwrap_or(0) & 0x03;
            buf[p] = (u8::from(de) << 7) | (cm << 5);
            p += 1;
        }
        if let Some(t) = self.toc {
            buf[p] = t.len() as u8;
            p += 1;
            buf[p..p + t.len()].copy_from_slice(t);
            p += t.len();
        }
        buf[p..p + self.additional_info.len()].copy_from_slice(self.additional_info);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_ac4_full() {
        // config_flag=1, toc_flag=1; config byte de=1 cm=2; toc len 2 = [0x11,0x22]; extra 0x33
        let sel = [0xC0, 0x80 | (0x02 << 5), 0x02, 0x11, 0x22, 0x33];
        let bytes = wrap(0x15, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::Ac4(b) => {
                assert!(b.ac4_config_flag);
                assert!(b.ac4_toc_flag);
                assert_eq!(b.ac4_dialog_enhancement_enabled, Some(true));
                assert_eq!(b.ac4_channel_mode, Some(0x02));
                assert_eq!(b.toc, Some([0x11u8, 0x22].as_slice()));
                assert_eq!(b.additional_info, &[0x33]);
            }
            other => panic!("expected Ac4, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_ac4_minimal() {
        let sel = [0x00]; // no config, no toc, no extra
        let bytes = wrap(0x15, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::Ac4(b) => {
                assert!(!b.ac4_config_flag);
                assert!(!b.ac4_toc_flag);
                assert_eq!(b.toc, None);
                assert!(b.additional_info.is_empty());
            }
            other => panic!("expected Ac4, got {other:?}"),
        }
        round_trip(&d);
    }
}
