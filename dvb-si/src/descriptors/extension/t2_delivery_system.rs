use super::*;

impl ExtensionBodyDef for T2DeliverySystem<'_> {
    const TAG_EXTENSION: u8 = 0x04;
    const NAME: &'static str = "T2_DELIVERY_SYSTEM";
}

// ===========================================================================
//  Section 0x04 — T2_delivery_system_descriptor (Table 133, §6.4.6.3)
// ---------------------------------------------------------------------------
//  plp_id(8) T2_system_id(16) then, if descriptor_length > 4, a packed flags
//  block (SISO_MISO 2 | bandwidth 4 | reserved 2 ; guard 3 | tx_mode 3 | off 1 |
//  tfs 1) followed by a variable cell loop (cells carry tfs-conditional
//  frequency lists + subcell loops). The cell loop is length-irregular and is
//  kept raw per the SAT precedent; the always-present prefix is typed.
// ===========================================================================
/// T2_delivery_system body (Table 133). `cell_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct T2DeliverySystem<'a> {
    /// PLP identifier.
    pub plp_id: u8,
    /// T2 system identifier.
    pub t2_system_id: u16,
    /// SISO_MISO(2), present iff `descriptor_length > 4` (flags block present).
    pub siso_miso: Option<u8>,
    /// bandwidth(4), present with `siso_miso`.
    pub bandwidth: Option<u8>,
    /// guard_interval(3), present with `siso_miso`.
    pub guard_interval: Option<u8>,
    /// transmission_mode(3), present with `siso_miso`.
    pub transmission_mode: Option<u8>,
    /// other_frequency_flag(1), present with `siso_miso`.
    pub other_frequency_flag: Option<bool>,
    /// tfs_flag(1), present with `siso_miso`.
    pub tfs_flag: Option<bool>,
    /// Raw cell loop (Table 133 inner `for`), kept raw (SAT precedent).
    pub cell_loop: &'a [u8],
}

impl<'a> Parse<'a> for T2DeliverySystem<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < T2_FIXED_PREFIX_LEN {
            return Err(invalid("T2_delivery_system: prefix truncated"));
        }
        let plp_id = sel[0];
        let t2_system_id = u16::from_be_bytes([sel[1], sel[2]]);
        let mut pos = T2_FIXED_PREFIX_LEN;
        // descriptor_length > 4 ⇔ the optional packed flags block is present
        // (the body is plp + system_id = 3 bytes when absent; >3 ⇒ block present).
        let (
            siso_miso,
            bandwidth,
            guard_interval,
            transmission_mode,
            other_frequency_flag,
            tfs_flag,
        ) = if sel.len() > T2_FIXED_PREFIX_LEN {
            if sel.len() < T2_FIXED_PREFIX_LEN + T2_FLAGS_BLOCK_LEN {
                return Err(invalid("T2_delivery_system: flags block truncated"));
            }
            let b0 = sel[pos];
            let b1 = sel[pos + 1];
            pos += T2_FLAGS_BLOCK_LEN;
            (
                Some(b0 >> 6),
                Some((b0 >> 2) & 0x0F),
                Some(b1 >> 5),
                Some((b1 >> 2) & 0x07),
                Some((b1 & 0x02) != 0),
                Some((b1 & 0x01) != 0),
            )
        } else {
            (None, None, None, None, None, None)
        };
        Ok(T2DeliverySystem {
            plp_id,
            t2_system_id,
            siso_miso,
            bandwidth,
            guard_interval,
            transmission_mode,
            other_frequency_flag,
            tfs_flag,
            cell_loop: &sel[pos..],
        })
    }
}

impl Serialize for T2DeliverySystem<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        T2_FIXED_PREFIX_LEN
            + if self.siso_miso.is_some() {
                T2_FLAGS_BLOCK_LEN
            } else {
                0
            }
            + self.cell_loop.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.plp_id;
        buf[1..3].copy_from_slice(&self.t2_system_id.to_be_bytes());
        let mut p = T2_FIXED_PREFIX_LEN;
        if let (Some(sm), Some(bw), Some(gi), Some(tm), Some(off), Some(tfs)) = (
            self.siso_miso,
            self.bandwidth,
            self.guard_interval,
            self.transmission_mode,
            self.other_frequency_flag,
            self.tfs_flag,
        ) {
            buf[p] = (sm << 6) | ((bw & 0x0F) << 2);
            buf[p + 1] = (gi << 5) | ((tm & 0x07) << 2) | (u8::from(off) << 1) | u8::from(tfs);
            p += T2_FLAGS_BLOCK_LEN;
        }
        buf[p..p + self.cell_loop.len()].copy_from_slice(self.cell_loop);
        Ok(len)
    }
}
