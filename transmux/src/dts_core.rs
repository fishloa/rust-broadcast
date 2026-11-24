//! DTS core substream frame headers and frame splitting —
//! ETSI TS 102 114 §5.3/§5.4, Tables 5-1/5-2/5-4/5-5/5-7.
//!
//! [`DtsCoreFrameInfo`] parses the core frame header and builds a `ddts` box
//! via [`DtsCoreFrameInfo::into_ddts`]; [`split_dts_core_frames`] splits a
//! concatenated PES payload into core frames. The `ddts` box itself lives in
//! [`isobmff::dts`].

use crate::bitreader::read_bits_checked as read_bits;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::error::InvalidValue;
use isobmff::dts::DtsSpecificBox;

pub(crate) const DTS_CORE_SYNC_WORD: u32 = 0x7FFE_8001;

/// Samples per PCM sample block (`NBLKS` unit) — ETSI TS 102 114 Table 5-1:
/// "samples per channel = 32 × (NBLKS + 1)".
const DTS_SAMPLES_PER_BLOCK: u32 = 32;

/// Minimum valid `NBLKS` (7-bit field) — Table 5-1: "Valid 5..=127 (0..=4 invalid)".
const DTS_NBLKS_MIN: u8 = 5;

/// Minimum valid `FSIZE` (14-bit field) — Table 5-1: "Valid 95..=16383 (0..=94 invalid)".
const DTS_FSIZE_MIN: u16 = 95;

/// `pcmSampleDepth` for a core-only `ddts` — ETSI TS 102 114 §E.2.2.3.2: the
/// core substream's PCM sample depth is always 16 bits.
const DTS_CORE_PCM_SAMPLE_DEPTH: u8 = 16;

/// `StreamConstruction` for a core-only substream (no extensions) — ETSI TS
/// 102 114 Table E-2, the code mapped to the `dtsc` coding name.
const DTS_STREAM_CONSTRUCTION_CORE_ONLY: u8 = 1;

/// `CoreLayout` value meaning "use `ChannelLayout`" — ETSI TS 102 114 Table E-3,
/// used for any `AMODE` arrangement not directly enumerated in the table.
const DTS_CORE_LAYOUT_USE_CHANNEL_LAYOUT: u8 = 31;

/// `RATE` code for open/variable bit rate — ETSI TS 102 114 Table 5-7 (`0b11101`).
const DTS_RATE_OPEN: u8 = 0b11101;

/// Width of `SHORT`.
const DTS_SHORT_BITS: usize = 5;
/// Width of `NBLKS`.
const DTS_NBLKS_BITS: usize = 7;
/// Width of `FSIZE`.
const DTS_FSIZE_BITS: usize = 14;
/// Width of `AMODE`.
const DTS_AMODE_BITS: usize = 6;
/// Width of `SFREQ`.
const DTS_SFREQ_BITS: usize = 4;
/// Width of `RATE`.
const DTS_RATE_BITS: usize = 5;
/// Width of `EXT_AUDIO_ID`.
const DTS_EXT_AUDIO_ID_BITS: usize = 3;
/// Width of `LFF`.
const DTS_LFF_BITS: usize = 2;

/// DTS core `RATE` field (Table 5-7) → nominal bit rate in bits/s, indices
/// `0..=28`. Verified bit-exact against ffmpeg's `ff_dca_bit_rates` table
/// (`libavcodec/dcadata.c`), which implements the same ETSI-published values
/// (a DTS decoder must reproduce the encoder's declared bit rate). Index
/// [`DTS_RATE_OPEN`] (29) and the two reserved codes above it are not real
/// bit rates — `dts_rate_bps` maps every code `>= 29` to `0` (open/unknown).
#[rustfmt::skip]
const DTS_RATE_BPS: [u32; 29] = [
      32_000,   56_000,   64_000,   96_000,  112_000,  128_000,
     192_000,  224_000,  256_000,  320_000,  384_000,
     448_000,  512_000,  576_000,  640_000,  768_000,
     896_000, 1_024_000, 1_152_000, 1_280_000, 1_344_000,
   1_408_000, 1_411_200, 1_472_000, 1_536_000, 1_920_000,
   2_048_000, 3_072_000, 3_840_000,
];

/// `SFREQ` field → core sampling frequency in Hz — ETSI TS 102 114 Table 5-5.
/// `None` for a reserved/invalid code.
fn dts_sfreq_hz(sfreq: u8) -> Option<u32> {
    Some(match sfreq {
        0b0001 => 8_000,
        0b0010 => 16_000,
        0b0011 => 32_000,
        0b0110 => 11_025,
        0b0111 => 22_050,
        0b1000 => 44_100,
        0b1011 => 12_000,
        0b1100 => 24_000,
        0b1101 => 48_000,
        _ => return None, // 0b0000/0100/0101/1001/1010/1110/1111 reserved
    })
}

/// `AMODE` field → channel count (`CHS`, before LFE) — ETSI TS 102 114 Table
/// 5-4. `None` for a user-defined arrangement (`0b010000..=0b111111`).
fn dts_amode_channels(amode: u8) -> Option<u8> {
    Some(match amode {
        0b000000 => 1,
        0b000001 => 2,
        0b000010 => 2,
        0b000011 => 2,
        0b000100 => 2,
        0b000101 => 3,
        0b000110 => 3,
        0b000111 => 4,
        0b001000 => 4,
        0b001001 => 5,
        0b001010 => 6,
        0b001011 => 6,
        0b001100 => 6,
        0b001101 => 7,
        0b001110 => 8,
        0b001111 => 8,
        _ => return None, // 0b010000..=0b111111: user defined
    })
}

/// `AMODE` field → `ddts` `CoreLayout` — ETSI TS 102 114 Table E-3.
/// Arrangements not enumerated in the table map to
/// [`DTS_CORE_LAYOUT_USE_CHANNEL_LAYOUT`] ("use `ChannelLayout`").
fn dts_amode_core_layout(amode: u8) -> u8 {
    match amode {
        0b000000 => 0, // Mono (1/0)
        0b000010 => 2, // Stereo (2/0)
        0b000100 => 4, // LT/RT (2/0)
        0b000101 => 5, // L,C,R (3/0)
        0b000110 => 6, // L,R,S (2/1)
        0b000111 => 7, // L,C,R,S (3/1)
        0b001000 => 8, // L,R,LS,RS (2/2)
        0b001001 => 9, // L,C,R,LS,RS (3/2)
        _ => DTS_CORE_LAYOUT_USE_CHANNEL_LAYOUT,
    }
}

/// `RATE` field (Table 5-7) → nominal bit rate in bits/s; `0` for the
/// open/variable code and any code beyond the table (see [`DTS_RATE_BPS`]).
fn dts_rate_bps(rate: u8) -> u32 {
    if rate >= DTS_RATE_OPEN {
        return 0; // open/variable (0b11101) or reserved (0b11110/0b11111)
    }
    DTS_RATE_BPS.get(rate as usize).copied().unwrap_or(0)
}

/// Read a big-endian `u32` at `offset`.
fn read_be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        bytes[offset..offset + core::mem::size_of::<u32>()]
            .try_into()
            .expect("u32 length"),
    )
}

/// Find the byte offset of the [`DTS_CORE_SYNC_WORD`] in a buffer.
fn find_dts_sync(data: &[u8]) -> Result<usize> {
    for i in 0..data.len().saturating_sub(core::mem::size_of::<u32>() - 1) {
        let word = read_be_u32(data, i);
        if word == DTS_CORE_SYNC_WORD {
            return Ok(i);
        }
    }
    Err(Error::InvalidInput(
        "DTS core syncword (0x7FFE8001) not found in elementary stream",
    ))
}

/// Fields parsed from a DTS core-substream frame header (ETSI TS 102 114
/// §5.3/§5.4, Table 5-1), sufficient to describe the elementary stream and
/// build a core-only [`DtsSpecificBox`] via [`into_ddts`](Self::into_ddts).
/// The parser is the only producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DtsCoreFrameInfo {
    /// Core sampling frequency in Hz (Table 5-5).
    pub sample_rate: u32,
    /// Total channel count: `CHS` (Table 5-4) + 1 if LFE is present.
    pub channels: u8,
    /// Samples per channel per frame: `32 × (NBLKS + 1)`.
    pub samples_per_frame: u32,
    /// Frame byte size: `FSIZE + 1`.
    pub frame_size: usize,
    /// Raw `AMODE` field (channel arrangement, Table 5-4).
    pub amode: u8,
    /// Raw `LFF` field (low-frequency-effects flag, `0` = none).
    pub lff: u8,
    /// Raw `NBLKS` field.
    pub nblks: u8,
    /// Raw `FSIZE` field.
    pub fsize: u16,
    /// Raw `RATE` field (targeted bit rate, Table 5-7).
    pub rate: u8,
    /// Raw `SFREQ` field (Table 5-5).
    pub sfreq: u8,
}

impl DtsCoreFrameInfo {
    fn with_fields(
        sample_rate: u32,
        channels: u8,
        samples_per_frame: u32,
        frame_size: usize,
        amode: u8,
        lff: u8,
        nblks: u8,
        fsize: u16,
        rate: u8,
        sfreq: u8,
    ) -> Self {
        Self {
            sample_rate,
            channels,
            samples_per_frame,
            frame_size,
            amode,
            lff,
            nblks,
            fsize,
            rate,
            sfreq,
        }
    }

    /// Parse the first core-substream frame header from a DTS elementary
    /// stream buffer. Scans for the `0x7FFE8001` sync word, then parses the
    /// fixed-position fields `SYNC..HFLAG` (Table 5-1).
    pub fn from_es(data: &[u8]) -> Result<Self> {
        let off = find_dts_sync(data)?;
        Self::parse_at(data, off)
    }

    pub(crate) fn parse_at(data: &[u8], off: usize) -> Result<Self> {
        let es = &data[off..];
        let mut bit_pos = 0usize;

        // SYNC: 32 bits (already matched by the caller).
        bit_pos += u32::BITS as usize;
        let _ftype = read_bits(es, &mut bit_pos, 1, "FTYPE")?;
        let _short = read_bits(es, &mut bit_pos, DTS_SHORT_BITS, "SHORT")?;
        let _cpf = read_bits(es, &mut bit_pos, 1, "CPF")?;
        let nblks = read_bits(es, &mut bit_pos, DTS_NBLKS_BITS, "NBLKS")? as u8;
        if nblks < DTS_NBLKS_MIN {
            return Err(Error::InvalidValue(InvalidValue {
                field: "NBLKS",
                value: nblks as u64,
                reason: "must be >= 5 (0..=4 reserved — Table 5-1)",
            }));
        }
        let fsize = read_bits(es, &mut bit_pos, DTS_FSIZE_BITS, "FSIZE")? as u16;
        if fsize < DTS_FSIZE_MIN {
            return Err(Error::InvalidValue(InvalidValue {
                field: "FSIZE",
                value: fsize as u64,
                reason: "must be >= 95 (0..=94 reserved — Table 5-1)",
            }));
        }
        let amode = read_bits(es, &mut bit_pos, DTS_AMODE_BITS, "AMODE")? as u8;
        let sfreq = read_bits(es, &mut bit_pos, DTS_SFREQ_BITS, "SFREQ")? as u8;
        let rate = read_bits(es, &mut bit_pos, DTS_RATE_BITS, "RATE")? as u8;
        let _fixed_bit = read_bits(es, &mut bit_pos, 1, "FixedBit")?;
        let _dynf = read_bits(es, &mut bit_pos, 1, "DYNF")?;
        let _timef = read_bits(es, &mut bit_pos, 1, "TIMEF")?;
        let _auxf = read_bits(es, &mut bit_pos, 1, "AUXF")?;
        let _hdcd = read_bits(es, &mut bit_pos, 1, "HDCD")?;
        let _ext_audio_id = read_bits(es, &mut bit_pos, DTS_EXT_AUDIO_ID_BITS, "EXT_AUDIO_ID")?;
        let _ext_audio = read_bits(es, &mut bit_pos, 1, "EXT_AUDIO")?;
        let _aspf = read_bits(es, &mut bit_pos, 1, "ASPF")?;
        let lff = read_bits(es, &mut bit_pos, DTS_LFF_BITS, "LFF")? as u8;
        let _hflag = read_bits(es, &mut bit_pos, 1, "HFLAG")?;

        let sample_rate = dts_sfreq_hz(sfreq).ok_or(Error::InvalidValue(InvalidValue {
            field: "SFREQ",
            value: sfreq as u64,
            reason: "reserved/invalid core sampling-frequency code — Table 5-5",
        }))?;
        let chs = dts_amode_channels(amode).ok_or(Error::InvalidValue(InvalidValue {
            field: "AMODE",
            value: amode as u64,
            reason: "user-defined channel arrangement (0b010000..=0b111111) not decodable",
        }))?;
        let channels = chs + u8::from(lff != 0);

        Ok(DtsCoreFrameInfo::with_fields(
            sample_rate,
            channels,
            DTS_SAMPLES_PER_BLOCK * (nblks as u32 + 1),
            fsize as usize + 1,
            amode,
            lff,
            nblks,
            fsize,
            rate,
            sfreq,
        ))
    }

    /// Build a core-only [`DtsSpecificBox`] (`dtsc`) from the parsed header —
    /// ETSI TS 102 114 §E.2.2.3.2, Tables E-2/E-3/E-5 (private/specs-verified/mpeg/transmux/codec/dts-core-frame.md
    /// `into_ddts` derivation table).
    ///
    /// `channel_layout` is `0` both when `core_layout` names the arrangement
    /// (channel config is then carried by `CoreLayout`) and for the
    /// `CoreLayout == 31` fallback: Table E-5's full `AMODE`→mask
    /// correspondence is not completely transcribed in
    /// `private/specs-verified/mpeg/transmux/codec/dts-core-frame.md` (only 5 of its bit roles are cited), so
    /// this is deferred rather than guessed for the arrangements it would
    /// cover.
    pub fn into_ddts(&self) -> DtsSpecificBox {
        let frame_duration = match self.samples_per_frame {
            512 => 0,
            1024 => 1,
            2048 => 2,
            4096 => 3,
            _ => 0, // not a valid ddts code (incl. 256) — smallest code per the recipe
        };
        let bit_rate = dts_rate_bps(self.rate);

        DtsSpecificBox {
            dts_sampling_frequency: self.sample_rate,
            max_bitrate: bit_rate,
            avg_bitrate: bit_rate,
            pcm_sample_depth: DTS_CORE_PCM_SAMPLE_DEPTH,
            frame_duration,
            stream_construction: DTS_STREAM_CONSTRUCTION_CORE_ONLY,
            core_lfe_present: self.lff != 0,
            core_layout: dts_amode_core_layout(self.amode),
            core_size: self.frame_size as u16,
            stereo_downmix: false,
            representation_type: 0,
            channel_layout: 0,
            multi_asset_flag: false,
            lbr_duration_mod: false,
            reserved_box_present: false,
        }
    }

    /// Total channel count (`CHS` + LFE) — see [`Self::channels`].
    pub fn channel_count(&self) -> u8 {
        self.channels
    }
}

/// One split DTS core access unit: an independent frame's bytes plus its own
/// samples-per-channel count (mirrors [`crate::ac3_syncframe::Ec3SplitFrame`] — DTS
/// frames can vary in `NBLKS`/duration stream to stream).
#[derive(Debug, Clone, Copy)]
pub struct DtsCoreFrame<'a> {
    /// Coded frame bytes (`FSIZE + 1` long), sync word included.
    pub data: &'a [u8],
    /// Samples per channel this frame decodes to (`32 × (NBLKS + 1)`).
    pub samples: u32,
}

/// Split a concatenated DTS core PES payload into individual frames, using
/// each frame's own `FSIZE` (Table 5-1) rather than assuming one PES payload
/// equals one frame. Stops at the first bad sync word / truncated tail so a
/// partial trailing frame does not lose the earlier ones (mirrors
/// [`crate::ac3_syncframe::split_ac3_syncframes`]).
pub fn split_dts_core_frames(payload: &[u8]) -> Vec<DtsCoreFrame<'_>> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + core::mem::size_of::<u32>() <= payload.len() {
        let word = read_be_u32(payload, off);
        if word != DTS_CORE_SYNC_WORD {
            break;
        }
        let Ok(info) = DtsCoreFrameInfo::parse_at(payload, off) else {
            break;
        };
        let len = info.frame_size;
        if len == 0 || off + len > payload.len() {
            break;
        }
        frames.push(DtsCoreFrame {
            data: &payload[off..off + len],
            samples: info.samples_per_frame,
        });
        off += len;
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bits::BitWriter;
    use broadcast_common::{Parse, Serialize};
    use isobmff::dts::*;

    /// Spec-derived test vector: hand-computed from §E.2.2.3.1 field layout.
    ///
    /// Fields chosen to exercise every non-zero field and keep mental arithmetic
    /// simple. See `transmux/tests/it/dts.rs` for the full integration gate.
    #[test]
    fn round_trip_unit() {
        // Build a DtsSpecificBox from known field values.
        let orig = DtsSpecificBox {
            dts_sampling_frequency: 48_000,
            max_bitrate: 1_509_000,
            avg_bitrate: 754_500,
            pcm_sample_depth: 24,
            frame_duration: 1,      // 1024 samples
            stream_construction: 2, // Table E-2
            core_lfe_present: true,
            core_layout: 9, // Table E-3
            core_size: 0x1234,
            stereo_downmix: false,
            representation_type: 3, // Table E-4
            channel_layout: 0x000F, // Table E-5
            multi_asset_flag: false,
            lbr_duration_mod: false,
            reserved_box_present: false,
        };

        let mut buf = [0u8; DDTS_BODY_LEN];
        orig.serialize_into(&mut buf).unwrap();

        let parsed = DtsSpecificBox::parse(&buf).unwrap();
        assert_eq!(parsed, orig, "parse(serialize(x)) must equal x");
    }

    // DtsCoreFrameInfo: a bit writer independent of production `read_bits`
    // builds synthetic core headers to exercise the parser.

    /// Build a synthetic DTS core frame header (Table 5-1), header only (11 bytes).
    fn build_core_header(
        nblks: u8,
        fsize: u16,
        amode: u8,
        sfreq: u8,
        rate: u8,
        lff: u8,
    ) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.u(32, DTS_CORE_SYNC_WORD as u64);
        w.u(1, 1); // FTYPE: normal frame
        w.u(5, 31); // SHORT
        w.u(1, 0); // CPF
        w.u(7, nblks as u64);
        w.u(14, fsize as u64);
        w.u(6, amode as u64);
        w.u(4, sfreq as u64);
        w.u(5, rate as u64);
        w.u(1, 0); // FixedBit
        w.u(1, 0); // DYNF
        w.u(1, 0); // TIMEF
        w.u(1, 0); // AUXF
        w.u(1, 0); // HDCD
        w.u(3, 0); // EXT_AUDIO_ID
        w.u(1, 0); // EXT_AUDIO
        w.u(1, 0); // ASPF
        w.u(2, lff as u64); // LFF
        w.u(1, 0); // HFLAG
        let mut buf = w.finish();
        buf.resize(16, 0);
        buf
    }

    /// A synthetic frame padded out to its declared `FSIZE + 1` byte length.
    fn build_padded_frame(
        nblks: u8,
        fsize: u16,
        amode: u8,
        sfreq: u8,
        rate: u8,
        lff: u8,
    ) -> Vec<u8> {
        let mut hdr = build_core_header(nblks, fsize, amode, sfreq, rate, lff);
        hdr.resize(fsize as usize + 1, 0);
        hdr
    }

    // Fixture-derived parameters (`fixtures/ts/dts/dts-core.ts`): stereo 48 kHz
    // core, NBLKS=15 (512 samples/frame), FSIZE=1023 (1024-byte frames),
    // AMODE=0b000010 (stereo), SFREQ=0b1101 (48 kHz), RATE=15 (768 kbps), LFF=0.
    const FX_NBLKS: u8 = 15;
    const FX_FSIZE: u16 = 1023;
    const FX_AMODE: u8 = 0b000010;
    const FX_SFREQ: u8 = 0b1101;
    const FX_RATE: u8 = 15;
    const FX_LFF: u8 = 0;

    #[test]
    fn from_es_parses_core_header() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.samples_per_frame, 512);
        assert_eq!(info.frame_size, 1024);
        assert_eq!(info.amode, FX_AMODE);
        assert_eq!(info.lff, 0);
        assert_eq!(info.channel_count(), 2);
    }

    #[test]
    fn from_es_scans_for_sync_not_at_offset_zero() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let mut padded = vec![0xAAu8; 7];
        padded.extend_from_slice(&hdr);
        let info = DtsCoreFrameInfo::from_es(&padded).unwrap();
        assert_eq!(info.sample_rate, 48_000);
    }

    #[test]
    fn from_es_rejects_missing_sync() {
        let junk = [0u8; 32];
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&junk),
            Err(crate::error::Error::InvalidInput(..))
        ));
    }

    #[test]
    fn from_es_rejects_nblks_below_minimum() {
        let hdr = build_core_header(3, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&hdr),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn from_es_rejects_fsize_below_minimum() {
        let hdr = build_core_header(FX_NBLKS, 50, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&hdr),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn nblks_five_is_accepted_and_four_is_rejected() {
        let hdr = build_core_header(5, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        assert_eq!(info.nblks, 5);
        assert_eq!(info.samples_per_frame, 192);
        let hdr = build_core_header(4, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&hdr),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn fsize_95_is_accepted_and_94_is_rejected() {
        let hdr = build_core_header(FX_NBLKS, 95, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        assert_eq!(info.fsize, 95);
        assert_eq!(info.frame_size, 96);
        let hdr = build_core_header(FX_NBLKS, 94, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&hdr),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn from_es_rejects_invalid_sfreq() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, 0b0000, FX_RATE, FX_LFF);
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&hdr),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn from_es_rejects_user_defined_amode() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, 0b010000, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(matches!(
            DtsCoreFrameInfo::from_es(&hdr),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn from_es_lfe_adds_a_channel() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, 1);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        assert_eq!(info.channels, 3, "LFF != 0 must add one channel");
    }

    #[test]
    fn sfreq_table_matches_table_5_5() {
        for (code, hz) in [
            (0b0001u8, 8_000u32),
            (0b0010, 16_000),
            (0b0011, 32_000),
            (0b0110, 11_025),
            (0b0111, 22_050),
            (0b1000, 44_100),
            (0b1011, 12_000),
            (0b1100, 24_000),
            (0b1101, 48_000),
        ] {
            assert_eq!(dts_sfreq_hz(code), Some(hz), "SFREQ {code:#06b}");
        }
        for code in [0b0000u8, 0b0100, 0b0101, 0b1001, 0b1010, 0b1110, 0b1111] {
            assert_eq!(dts_sfreq_hz(code), None, "SFREQ {code:#06b} is reserved");
        }
    }

    #[test]
    fn amode_table_matches_table_5_4() {
        let chs = [1u8, 2, 2, 2, 2, 3, 3, 4, 4, 5, 6, 6, 6, 7, 8, 8];
        for (amode, &expected) in chs.iter().enumerate() {
            assert_eq!(
                dts_amode_channels(amode as u8),
                Some(expected),
                "AMODE {amode:#08b}"
            );
        }
        for amode in 0b010000..=0b111111u8 {
            assert_eq!(
                dts_amode_channels(amode),
                None,
                "AMODE {amode:#08b} is user defined"
            );
        }
    }

    #[test]
    fn amode_core_layout_matches_table_e_3() {
        for (amode, layout) in [
            (0u8, 0u8),
            (2, 2),
            (4, 4),
            (5, 5),
            (6, 6),
            (7, 7),
            (8, 8),
            (9, 9),
        ] {
            assert_eq!(dts_amode_core_layout(amode), layout, "AMODE {amode:#08b}");
        }
        for amode in [1u8, 3, 10, 11, 12, 13, 14, 15, 16, 63] {
            assert_eq!(
                dts_amode_core_layout(amode),
                DTS_CORE_LAYOUT_USE_CHANNEL_LAYOUT,
                "AMODE {amode:#08b}"
            );
        }
        assert_eq!(DTS_CORE_LAYOUT_USE_CHANNEL_LAYOUT, 31);
    }

    #[test]
    fn into_ddts_core_only_matches_derivation() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        let ddts = info.into_ddts();

        assert_eq!(ddts.dts_sampling_frequency, 48_000);
        assert_eq!(ddts.stream_construction, DTS_STREAM_CONSTRUCTION_CORE_ONLY);
        assert_eq!(
            ddts.core_layout, 2,
            "AMODE=stereo -> CoreLayout 2 (Table E-3)"
        );
        assert!(!ddts.core_lfe_present);
        assert_eq!(ddts.core_size, 1024);
        assert_eq!(ddts.pcm_sample_depth, DTS_CORE_PCM_SAMPLE_DEPTH);
        assert_eq!(ddts.max_bitrate, 768_000, "RATE=15 -> 768 kbps (Table 5-7)");
        assert_eq!(ddts.avg_bitrate, 768_000);
        assert_eq!(ddts.frame_duration, 0, "512 samples/frame -> code 0");
        assert!(!ddts.stereo_downmix);
        assert_eq!(ddts.representation_type, 0);
        assert!(!ddts.multi_asset_flag);
        assert!(!ddts.lbr_duration_mod);
        assert!(!ddts.reserved_box_present);
    }

    #[test]
    fn into_ddts_maps_each_frame_duration_code() {
        for (nblks, samples, code) in [(31u8, 1024u32, 1u8), (63, 2048, 2), (127, 4096, 3)] {
            let hdr = build_core_header(nblks, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
            let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
            assert_eq!(info.samples_per_frame, samples, "NBLKS {nblks}");
            assert_eq!(
                info.into_ddts().frame_duration,
                code,
                "NBLKS {nblks} -> FrameDuration code {code}"
            );
        }
    }

    #[test]
    fn into_ddts_open_rate_yields_zero_bitrate() {
        let hdr = build_core_header(
            FX_NBLKS,
            FX_FSIZE,
            FX_AMODE,
            FX_SFREQ,
            DTS_RATE_OPEN,
            FX_LFF,
        );
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        let ddts = info.into_ddts();
        assert_eq!(ddts.max_bitrate, 0);
        assert_eq!(ddts.avg_bitrate, 0);
    }

    #[test]
    fn into_ddts_fallback_core_layout_for_unlisted_amode() {
        // AMODE 0b000001 (A+B dual mono) is in Table 5-4 (CHS=2) but not in the
        // CoreLayout Table E-3 enumeration -> falls back to 31.
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, 0b000001, FX_SFREQ, FX_RATE, FX_LFF);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        let ddts = info.into_ddts();
        assert_eq!(ddts.core_layout, DTS_CORE_LAYOUT_USE_CHANNEL_LAYOUT);
    }

    #[test]
    fn split_dts_core_frames_splits_concatenated_frames() {
        let frame1 = build_padded_frame(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let frame2 = build_padded_frame(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let mut payload = frame1.clone();
        payload.extend_from_slice(&frame2);

        let frames = split_dts_core_frames(&payload);
        assert_eq!(frames.len(), 2);
        for f in &frames {
            assert_eq!(f.data.len(), 1024);
            assert_eq!(f.samples, 512);
        }
        assert_eq!(frames[0].data, frame1.as_slice());
        assert_eq!(frames[1].data, frame2.as_slice());
    }

    #[test]
    fn the_packed_word_signals_core_lfe_and_stereo_downmix() {
        let ordered = DtsSpecificBox {
            dts_sampling_frequency: 48_000,
            max_bitrate: 768_000,
            avg_bitrate: 768_000,
            pcm_sample_depth: 16,
            frame_duration: 0,
            stream_construction: DTS_STREAM_CONSTRUCTION_CORE_ONLY,
            core_lfe_present: true,
            core_layout: 9,
            core_size: 1024,
            stereo_downmix: true,
            representation_type: 0,
            channel_layout: 0,
            multi_asset_flag: false,
            lbr_duration_mod: false,
            reserved_box_present: false,
        };
        let mut buf = [0u8; DDTS_BODY_LEN];
        ordered.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[13] & 0x01, 0x01, "CoreLFEPresent is bit 24");
        assert_eq!(buf[16] & 0x08, 0x08, "StereoDownmix is bit 3");
        let parsed = DtsSpecificBox::parse(&buf).unwrap();
        assert!(parsed.core_lfe_present);
        assert!(parsed.stereo_downmix);

        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, 2);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        assert!(info.into_ddts().core_lfe_present);
    }

    #[test]
    fn split_dts_core_frames_stops_at_bad_sync() {
        let mut payload =
            build_padded_frame(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        payload.push(0xAA); // trailing garbage: not a valid sync word
        payload.push(0xAA);
        payload.push(0xAA);
        payload.push(0xAA);

        let frames = split_dts_core_frames(&payload);
        assert_eq!(
            frames.len(),
            1,
            "the bad trailing bytes must not be emitted as a frame"
        );
    }

    #[test]
    fn split_dts_core_frames_empty_on_no_sync() {
        let payload = [0u8; 32];
        assert!(split_dts_core_frames(&payload).is_empty());
    }

    #[test]
    fn split_dts_core_frames_stops_at_a_truncated_tail() {
        let frame = build_padded_frame(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        let mut payload = frame.clone();
        payload.extend_from_slice(&frame);
        let mut third = frame.clone();
        third.truncate(500);
        payload.extend_from_slice(&third);
        let frames = split_dts_core_frames(&payload);
        assert_eq!(frames.len(), 2, "the cut third frame is not emitted");
        assert_eq!(frames[0].data, frame.as_slice());
        assert_eq!(frames[1].data, frame.as_slice());

        let mut short = frame.clone();
        short.truncate(1023);
        assert!(
            split_dts_core_frames(&short).is_empty(),
            "a frame one byte short of its declared FSIZE + 1 is not emitted"
        );
    }
}
