//! AC-3 and Enhanced AC-3 decoder configuration boxes in ISOBMFF —
//! ETSI TS 102 366 Annex F.
//!
//! | Box | FourCC | Spec | Description |
//! |-----|--------|------|-------------|
//! | [`Ac3SpecificBox`] | `dac3` | §F.4 | AC-3 decoder config |
//! | [`Ec3SpecificBox`] | `dec3` | §F.6 | E-AC-3 decoder config |
//!
//! The syncframe BSI parsers, frame splitters and the `dac3`/`dec3`
//! constructors live in `transmux::ac3_syncframe`.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_core::bits::{BitError, BitReader};
use broadcast_core::error::{BufferTooShort, InvalidValue};
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

/// Width of the `fscod` field.
const FSCOD_BITS: usize = 2;
/// Width of the `bsid` field.
const BSID_BITS: usize = 5;
/// Width of the `bsmod` field.
const BSMOD_BITS: usize = 3;
/// Width of the `acmod` field.
const ACMOD_BITS: usize = 3;

/// Serialized length of the `dac3` box body.
const DAC3_BODY_LEN: usize = 3;
/// Width of `bit_rate_code`.
const BIT_RATE_CODE_BITS: usize = 5;
/// Width of the `dac3` reserved field.
const DAC3_RESERVED_BITS: usize = 5;
/// Width of `num_ind_sub`.
const NUM_IND_SUB_BITS: usize = 3;
/// Width of `data_rate`.
const DATA_RATE_BITS: usize = 13;
/// Serialized length of an `Ec3Substream` without `chan_loc`.
const EC3_SUBSTREAM_LEN: usize = 3;
/// Serialized length of an `Ec3Substream` carrying `chan_loc`.
const EC3_SUBSTREAM_LEN_CHAN_LOC: usize = 4;
/// Byte length of the fixed `dec3` header.
const DEC3_HEADER_LEN: usize = 2;
/// Width of an `Ec3Substream` reserved field.
const EC3_SUBSTREAM_RESERVED_BITS: usize = 3;
/// Width of `num_dep_sub`.
const NUM_DEP_SUB_BITS: usize = 4;
/// Width of `chan_loc`.
const CHAN_LOC_BITS: usize = 9;
/// Number of full-bandwidth channels a coded `acmod` value describes
/// (ETSI TS 102 366 Table 4.5).
pub fn acmod_channels(acmod: u8) -> u8 {
    match acmod {
        0 => 2, // 1+1 (dual mono) → 2 channels
        1 => 1, // 1/0 (mono)
        2 => 2, // 2/0
        3 => 3, // 3/0
        4 => 3, // 2/1
        5 => 4, // 3/1
        6 => 4, // 2/2
        // 3/2 (5.1)
        _ => 5,
    }
}

/// Maps a shared [`BitError`] onto this crate's typed errors.
fn bit_error(what: &'static str, e: BitError) -> Error {
    match e {
        BitError::OutOfBounds { .. } => {
            let mut short: BufferTooShort = e.into();
            short.what = what;
            Error::BufferTooShort(short)
        }
        BitError::TooManyBits { requested } => Error::InvalidValue(InvalidValue {
            field: what,
            value: u64::from(requested),
            reason: "field wider than 64 bits",
        }),
        BitError::ValueTooWide { value, .. } => Error::InvalidValue(InvalidValue {
            field: what,
            value,
            reason: "value does not fit the field width",
        }),
        _ => Error::InvalidValue(InvalidValue {
            field: what,
            value: 0,
            reason: "bit reader error",
        }),
    }
}

/// Read `n` bits MSB-first from `data` at `*bit_pos`, advancing it.
fn read_bits(data: &[u8], bit_pos: &mut usize, n: usize, what: &'static str) -> Result<u64> {
    let n_u32 = u32::try_from(n).map_err(|_| {
        Error::InvalidValue(InvalidValue {
            field: what,
            value: n as u64,
            reason: "bit count > 64",
        })
    })?;
    let mut reader = BitReader::starting_at(data, *bit_pos).map_err(|e| bit_error(what, e))?;
    let value = reader.read_bits(n_u32).map_err(|e| bit_error(what, e))?;
    *bit_pos += n;
    Ok(value)
}

/// Write `n` bits from `val` MSB-first into `buf` at `bit_pos`, advancing it.
fn write_bits(buf: &mut [u8], bit_pos: &mut usize, n: usize, val: u64) {
    for i in (0..n).rev() {
        let byte_idx = *bit_pos / u8::BITS as usize;
        let bit_in_byte = u8::BITS as usize - 1 - (*bit_pos % u8::BITS as usize);
        let bit = ((val >> i) & 1) as u8;
        buf[byte_idx] = (buf[byte_idx] & !(1 << bit_in_byte)) | (bit << bit_in_byte);
        *bit_pos += 1;
    }
}

// ---------------------------------------------------------------------------
// AC3SpecificBox (dac3) — §F.4
// ---------------------------------------------------------------------------

/// AC3SpecificBox (`dac3`) — ETSI TS 102 366 §F.4.
///
/// Wire format (3 bytes after box header):
/// `fscod(2) | bsid(5) | bsmod(3) | acmod(3) | lfeon(1) | bit_rate_code(5) | reserved(5)`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ac3SpecificBox {
    pub fscod: u8,
    pub bsid: u8,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfeon: bool,
    pub bit_rate_code: u8,
}

impl Ac3SpecificBox {
    /// Returns `"ac-3"` per RFC 6381 §3.3.
    pub fn rfc6381(&self) -> &'static str {
        "ac-3"
    }

    /// Number of full-bandwidth channels.
    pub fn channel_count(&self) -> u8 {
        acmod_channels(self.acmod)
    }

    /// Channel count including the LFE channel: [`Self::channel_count`] plus
    /// one when `lfeon` is set (a 3/2 + LFE "5.1" stream is 6).
    pub fn channels_with_lfe(&self) -> u8 {
        self.channel_count() + u8::from(self.lfeon)
    }

    /// Nominal bit rate in kbit/s from `bit_rate_code` — ETSI TS 102 366
    /// Table F.4.1. `None` for a code beyond the table (reserved).
    pub fn nominal_bitrate_kbps(&self) -> Option<u32> {
        AC3_BIT_RATE_KBPS
            .get(self.bit_rate_code as usize)
            .copied()
            .map(u32::from)
    }
}

/// `bit_rate_code` → nominal bit rate in kbit/s — ETSI TS 102 366 Table F.4.1
/// (`private/specs-verified/mpeg/transmux/codec/ac3-eac3-mp4.md`).
const AC3_BIT_RATE_KBPS: [u16; 19] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640,
];

impl<'a> Parse<'a> for Ac3SpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Box body is 3 bytes
        need_in(bytes, DAC3_BODY_LEN, "dac3 body")?;
        let mut bit_pos = 0usize;
        let fscod = read_bits(bytes, &mut bit_pos, FSCOD_BITS, "fscod")? as u8;
        let bsid = read_bits(bytes, &mut bit_pos, BSID_BITS, "bsid")? as u8;
        let bsmod = read_bits(bytes, &mut bit_pos, BSMOD_BITS, "bsmod")? as u8;
        let acmod = read_bits(bytes, &mut bit_pos, ACMOD_BITS, "acmod")? as u8;
        let lfeon = read_bits(bytes, &mut bit_pos, 1, "lfeon")? != 0;
        let bit_rate_code =
            read_bits(bytes, &mut bit_pos, BIT_RATE_CODE_BITS, "bit_rate_code")? as u8;
        let _reserved = read_bits(bytes, &mut bit_pos, DAC3_RESERVED_BITS, "reserved")?;
        Ok(Self {
            fscod,
            bsid,
            bsmod,
            acmod,
            lfeon,
            bit_rate_code,
        })
    }
}

impl Serialize for Ac3SpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        DAC3_BODY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, DAC3_BODY_LEN)?;
        let mut bit_pos = 0usize;
        write_bits(buf, &mut bit_pos, FSCOD_BITS, self.fscod as u64);
        write_bits(buf, &mut bit_pos, BSID_BITS, self.bsid as u64);
        write_bits(buf, &mut bit_pos, BSMOD_BITS, self.bsmod as u64);
        write_bits(buf, &mut bit_pos, ACMOD_BITS, self.acmod as u64);
        write_bits(buf, &mut bit_pos, 1, self.lfeon as u64);
        write_bits(
            buf,
            &mut bit_pos,
            BIT_RATE_CODE_BITS,
            self.bit_rate_code as u64,
        );
        write_bits(buf, &mut bit_pos, DAC3_RESERVED_BITS, 0); // reserved
        Ok(DAC3_BODY_LEN)
    }
}

// ---------------------------------------------------------------------------
// EC3SpecificBox (dec3) — §F.6
// ---------------------------------------------------------------------------

/// Per-substream config fields within [`Ec3SpecificBox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ec3Substream {
    pub fscod: u8,
    pub bsid: u8,
    pub asvc: bool,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfeon: bool,
    pub num_dep_sub: u8,
    /// Present only when `num_dep_sub > 0`.
    pub chan_loc: Option<u16>,
}

/// EC3SpecificBox (`dec3`) — ETSI TS 102 366 §F.6.
///
/// Wire format (variable length after box header):
/// `data_rate(13) | num_ind_sub(3)`, then per-substream fields.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ec3SpecificBox {
    pub data_rate: u16,
    /// Number of independent substreams. Actual count is `num_ind_sub + 1`.
    pub num_ind_sub: u8,
    /// One entry per independent substream; the serializer rejects any other
    /// length, since §F.6.1 derives the count from `num_ind_sub`.
    pub substreams: Vec<Ec3Substream>,
    /// Bytes after the last substream, §F.6.2.14's
    /// "reserved" tail: "Additional reserved bytes may follow at the end of the
    /// EC3SpecificBox … Decoders … should ignore any reserved bytes that are
    /// present". Captured verbatim so the round trip stays byte-exact — this is
    /// where a Dolby Atmos `flag_ec3_extension_type_a`/`complexity_index_type_a`
    /// trailer lives, and dropping it silently lost the Atmos signalling.
    pub reserved_tail: Vec<u8>,
}

impl Ec3SpecificBox {
    /// Returns `"ec-3"` per RFC 6381 §3.3.
    pub fn rfc6381(&self) -> &'static str {
        "ec-3"
    }

    /// Channel count from independent substream 0.
    pub fn channel_count(&self) -> u8 {
        self.substreams
            .first()
            .map(|s| acmod_channels(s.acmod))
            .unwrap_or(0)
    }

    /// Channel count of programme 0 including every LFE: independent
    /// substream 0's `acmod` channels + `lfeon`, plus the extra locations its
    /// dependent substreams declare in `chan_loc` (ETSI TS 102 366 Table
    /// F.6.1: pairs count two, `Cs`/`Ts`/`Cvh` one, `LFE2` one). `0` when
    /// there are no substreams.
    pub fn channels_with_lfe(&self) -> u8 {
        let Some(s) = self.substreams.first() else {
            return 0;
        };
        let extra: u8 = s.chan_loc.map_or(0, |loc| {
            EAC3_CHAN_LOC_CHANNELS
                .iter()
                .enumerate()
                .filter(|(bit, _)| loc & (1 << bit) != 0)
                .map(|(_, n)| *n)
                .sum()
        });
        acmod_channels(s.acmod) + u8::from(s.lfeon) + extra
    }

    /// Data rate of the entire bitstream in kbit/s (`data_rate`, §F.6.2.2:
    /// the sum over all substreams; the maximum for variable-rate streams).
    pub fn nominal_bitrate_kbps(&self) -> u32 {
        u32::from(self.data_rate)
    }
}

/// Channels contributed by each `chan_loc` bit — ETSI TS 102 366 Table F.6.1
/// (`private/specs-verified/mpeg/transmux/codec/eac3-bsi-chanmap.md`), indexed by bit number (bit 0 = `Lc/Rc`
/// pair … bit 8 = `LFE2`).
const EAC3_CHAN_LOC_CHANNELS: [u8; 9] = [
    2, // 0: Lc/Rc pair
    2, // 1: Lrs/Rrs pair
    1, // 2: Cs
    1, // 3: Ts
    2, // 4: Lsd/Rsd pair
    2, // 5: Lw/Rw pair
    2, // 6: Lvh/Rvh pair
    1, // 7: Cvh
    1, // 8: LFE2
];

fn ec3_substream_serialized_len(sub: &Ec3Substream) -> usize {
    // ETSI TS 102 366 §F.6.1: fscod(2)+bsid(5)+reserved(1)+asvc(1)+bsmod(3)+
    // acmod(3)+lfeon(1)+reserved(3)+num_dep_sub(4) = 23 bits, then
    // + chan_loc(9) when num_dep_sub>0 (32 bits = 4 bytes) else
    // + reserved(1) (24 bits = 3 bytes). A dependent substream (7.1 content
    // signalled via chan_loc) is 4 bytes, not always 3 (issue #1055).
    if sub.num_dep_sub > 0 {
        EC3_SUBSTREAM_LEN_CHAN_LOC
    } else {
        EC3_SUBSTREAM_LEN
    }
}

impl<'a> Parse<'a> for Ec3SpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, DEC3_HEADER_LEN, "dec3 body")?;
        let mut bit_pos = 0usize;
        let data_rate = read_bits(bytes, &mut bit_pos, DATA_RATE_BITS, "data_rate")? as u16;
        let num_ind_sub = read_bits(bytes, &mut bit_pos, NUM_IND_SUB_BITS, "num_ind_sub")? as u8;
        let num_sub = num_ind_sub as usize + 1;

        let need_bytes = DEC3_HEADER_LEN
            + num_sub
                * ec3_substream_serialized_len(&Ec3Substream {
                    fscod: 0,
                    bsid: 0,
                    asvc: false,
                    bsmod: 0,
                    acmod: 0,
                    lfeon: false,
                    num_dep_sub: 0,
                    chan_loc: None,
                });
        need_in(bytes, need_bytes, "dec3 substreams")?;

        let mut substreams = Vec::with_capacity(num_sub);
        for _ in 0..num_sub {
            let fscod = read_bits(bytes, &mut bit_pos, FSCOD_BITS, "fscod")? as u8;
            let bsid = read_bits(bytes, &mut bit_pos, BSID_BITS, "bsid")? as u8;
            let _res1 = read_bits(bytes, &mut bit_pos, 1, "reserved")?;
            let asvc = read_bits(bytes, &mut bit_pos, 1, "asvc")? != 0;
            let bsmod = read_bits(bytes, &mut bit_pos, BSMOD_BITS, "bsmod")? as u8;
            let acmod = read_bits(bytes, &mut bit_pos, ACMOD_BITS, "acmod")? as u8;
            let lfeon = read_bits(bytes, &mut bit_pos, 1, "lfeon")? != 0;
            let _res3 = read_bits(bytes, &mut bit_pos, EC3_SUBSTREAM_RESERVED_BITS, "reserved")?;
            let num_dep_sub =
                read_bits(bytes, &mut bit_pos, NUM_DEP_SUB_BITS, "num_dep_sub")? as u8;
            let chan_loc = if num_dep_sub > 0 {
                Some(read_bits(bytes, &mut bit_pos, CHAN_LOC_BITS, "chan_loc")? as u16)
            } else {
                let _res1 = read_bits(bytes, &mut bit_pos, 1, "reserved")?;
                None
            };
            substreams.push(Ec3Substream {
                fscod,
                bsid,
                asvc,
                bsmod,
                acmod,
                lfeon,
                num_dep_sub,
                chan_loc,
            });
        }

        // §F.6.2.14: whatever follows the last substream is reserved; keep it
        // byte-for-byte so a parsed box re-serializes identically.
        let tail_start = (bit_pos.div_ceil(u8::BITS as usize)).min(bytes.len());
        let reserved_tail = bytes[tail_start..].to_vec();

        Ok(Self {
            data_rate,
            num_ind_sub,
            substreams,
            reserved_tail,
        })
    }
}

impl Serialize for Ec3SpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        // 2 bytes header (data_rate+num_ind_sub) + 3 or 4 bytes per substream
        // (4 when it has dependent substreams — see `ec3_substream_serialized_len`),
        // then the reserved tail.
        DEC3_HEADER_LEN
            + self
                .substreams
                .iter()
                .map(ec3_substream_serialized_len)
                .sum::<usize>()
            + self.reserved_tail.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // §F.6.1 derives the independent-substream count from `num_ind_sub`, so
        // a box carrying any other number of substreams would serialize to bytes
        // that parse back differently (or not at all). Fail instead of emitting
        // a misframed box.
        let expected = self.num_ind_sub as usize + 1;
        if self.substreams.len() != expected {
            return Err(Error::InvalidValue(InvalidValue {
                field: "dec3 num_ind_sub",
                value: self.num_ind_sub as u64,
                reason: "substreams.len() must equal num_ind_sub + 1",
            }));
        }
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut bit_pos = 0usize;
        write_bits(buf, &mut bit_pos, DATA_RATE_BITS, self.data_rate as u64);
        write_bits(buf, &mut bit_pos, NUM_IND_SUB_BITS, self.num_ind_sub as u64);
        for sub in &self.substreams {
            write_bits(buf, &mut bit_pos, FSCOD_BITS, sub.fscod as u64);
            write_bits(buf, &mut bit_pos, BSID_BITS, sub.bsid as u64);
            write_bits(buf, &mut bit_pos, 1, 0); // reserved
            write_bits(buf, &mut bit_pos, 1, sub.asvc as u64);
            write_bits(buf, &mut bit_pos, BSMOD_BITS, sub.bsmod as u64);
            write_bits(buf, &mut bit_pos, ACMOD_BITS, sub.acmod as u64);
            write_bits(buf, &mut bit_pos, 1, sub.lfeon as u64);
            write_bits(buf, &mut bit_pos, EC3_SUBSTREAM_RESERVED_BITS, 0); // reserved
            write_bits(buf, &mut bit_pos, NUM_DEP_SUB_BITS, sub.num_dep_sub as u64);
            if sub.num_dep_sub > 0 {
                write_bits(
                    buf,
                    &mut bit_pos,
                    CHAN_LOC_BITS,
                    sub.chan_loc.unwrap_or(0) as u64,
                );
            } else {
                write_bits(buf, &mut bit_pos, 1, 0); // reserved
            }
        }
        // §F.6.2.14 reserved tail, verbatim.
        let tail_start = bit_pos.div_ceil(u8::BITS as usize);
        buf[tail_start..tail_start + self.reserved_tail.len()].copy_from_slice(&self.reserved_tail);
        Ok(need)
    }
}
