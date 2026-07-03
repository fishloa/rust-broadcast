//! DTS audio in ISOBMFF — `dtsc`/`dtsh`/`dtsl`/`dtse` AudioSampleEntry + `ddts` config box.
//!
//! ETSI TS 102 114 §E.2.2.3: the `DTSSpecificBox` (`ddts`) carries the DTS
//! stream parameters needed for basic playback. `transmux` is samples-in, so
//! the caller supplies the already-populated [`DtsSpecificBox`]; `transmux`
//! serializes it as the child `ddts` of a [`crate::init_segment::DtsSampleEntry`].
//!
//! The four DTS sample-entry FourCCs and their meaning (ETSI TS 102 114 Table E-1):
//!
//! | FourCC | Description |
//! |--------|-------------|
//! | `dtsc` | DTS core substream only |
//! | `dtsh` | DTS core + extension substream (multiple assets) |
//! | `dtsl` | DTS LBR only |
//! | `dtse` | DTS extension substream only |
//!
//! # DTS core elementary-stream framing (issue #560)
//!
//! [`DtsCoreFrameInfo::from_es`] parses a DTS **core substream** frame header
//! (ETSI TS 102 114 §5.3/§5.4, sync `0x7FFE8001`) from a raw elementary stream
//! (e.g. a TS PES payload) — see `docs/codec/dts-core-frame.md`. It recovers
//! sample rate, channel count, and samples/frame, and [`into_ddts`] builds a
//! core-only [`DtsSpecificBox`] from it (§E.2.2.3.2, Tables E-2/E-3/E-5) —
//! see `docs/codec/dts-isobmff-etsi102114.md`. [`split_dts_core_frames`]
//! splits a concatenated PES payload into individual core frames using each
//! frame's own `FSIZE` (mirrors [`crate::ac3::split_ac3_syncframes`]).
//!
//! [`into_ddts`]: DtsCoreFrameInfo::into_ddts

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// ---------------------------------------------------------------------------
// DTS core substream frame header — ETSI TS 102 114 §5.3/§5.4, Tables 5-1/5-2/
// 5-4/5-5/5-7 (docs/codec/dts-core-frame.md)
// ---------------------------------------------------------------------------

/// Core-substream synchronization word — ETSI TS 102 114 §5.3 (big-endian,
/// 16-bit core; the extension-substream-embedded core sync `0x02B09261` is
/// out of scope — TS carriage of a standalone core always uses this word).
const DTS_CORE_SYNC_WORD: u32 = 0x7FFE_8001;

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

/// Find the byte offset of the [`DTS_CORE_SYNC_WORD`] in a buffer.
fn find_dts_sync(data: &[u8]) -> Result<usize> {
    for i in 0..data.len().saturating_sub(3) {
        let word = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        if word == DTS_CORE_SYNC_WORD {
            return Ok(i);
        }
    }
    Err(Error::InvalidInput(
        "DTS core syncword (0x7FFE8001) not found in elementary stream",
    ))
}

/// Read `n` bits MSB-first from `data` at the current `bit_pos`, advancing it.
fn read_bits(data: &[u8], bit_pos: &mut usize, n: usize, what: &'static str) -> Result<u64> {
    let end = *bit_pos + n;
    let need_bytes = end.div_ceil(8);
    if data.len() < need_bytes {
        return Err(Error::BufferTooShort {
            need: need_bytes,
            have: data.len(),
            what,
        });
    }
    let mut val: u64 = 0;
    for _ in 0..n {
        let byte_idx = *bit_pos / 8;
        let bit_in_byte = 7 - (*bit_pos % 8);
        let bit = ((data[byte_idx] >> bit_in_byte) & 1) as u64;
        val = (val << 1) | bit;
        *bit_pos += 1;
    }
    Ok(val)
}

/// Fields parsed from a DTS core-substream frame header (ETSI TS 102 114
/// §5.3/§5.4, Table 5-1), sufficient to describe the elementary stream and
/// build a core-only [`DtsSpecificBox`] via [`into_ddts`](Self::into_ddts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Parse the first core-substream frame header from a DTS elementary
    /// stream buffer. Scans for the `0x7FFE8001` sync word, then parses the
    /// fixed-position fields `SYNC..HFLAG` (Table 5-1).
    pub fn from_es(data: &[u8]) -> Result<Self> {
        let off = find_dts_sync(data)?;
        Self::parse_at(data, off)
    }

    fn parse_at(data: &[u8], off: usize) -> Result<Self> {
        let es = &data[off..];
        let mut bit_pos = 0usize;

        // SYNC: 32 bits (already matched by the caller).
        bit_pos += 32;
        let _ftype = read_bits(es, &mut bit_pos, 1, "FTYPE")?;
        let _short = read_bits(es, &mut bit_pos, 5, "SHORT")?;
        let _cpf = read_bits(es, &mut bit_pos, 1, "CPF")?;
        let nblks = read_bits(es, &mut bit_pos, 7, "NBLKS")? as u8;
        if nblks < DTS_NBLKS_MIN {
            return Err(Error::InvalidValue {
                field: "NBLKS",
                value: nblks as u64,
                reason: "must be >= 5 (0..=4 reserved — Table 5-1)",
            });
        }
        let fsize = read_bits(es, &mut bit_pos, 14, "FSIZE")? as u16;
        if fsize < DTS_FSIZE_MIN {
            return Err(Error::InvalidValue {
                field: "FSIZE",
                value: fsize as u64,
                reason: "must be >= 95 (0..=94 reserved — Table 5-1)",
            });
        }
        let amode = read_bits(es, &mut bit_pos, 6, "AMODE")? as u8;
        let sfreq = read_bits(es, &mut bit_pos, 4, "SFREQ")? as u8;
        let rate = read_bits(es, &mut bit_pos, 5, "RATE")? as u8;
        let _fixed_bit = read_bits(es, &mut bit_pos, 1, "FixedBit")?;
        let _dynf = read_bits(es, &mut bit_pos, 1, "DYNF")?;
        let _timef = read_bits(es, &mut bit_pos, 1, "TIMEF")?;
        let _auxf = read_bits(es, &mut bit_pos, 1, "AUXF")?;
        let _hdcd = read_bits(es, &mut bit_pos, 1, "HDCD")?;
        let _ext_audio_id = read_bits(es, &mut bit_pos, 3, "EXT_AUDIO_ID")?;
        let _ext_audio = read_bits(es, &mut bit_pos, 1, "EXT_AUDIO")?;
        let _aspf = read_bits(es, &mut bit_pos, 1, "ASPF")?;
        let lff = read_bits(es, &mut bit_pos, 2, "LFF")? as u8;
        let _hflag = read_bits(es, &mut bit_pos, 1, "HFLAG")?;

        let sample_rate = dts_sfreq_hz(sfreq).ok_or(Error::InvalidValue {
            field: "SFREQ",
            value: sfreq as u64,
            reason: "reserved/invalid core sampling-frequency code — Table 5-5",
        })?;
        let chs = dts_amode_channels(amode).ok_or(Error::InvalidValue {
            field: "AMODE",
            value: amode as u64,
            reason: "user-defined channel arrangement (0b010000..=0b111111) not decodable",
        })?;
        let channels = chs + u8::from(lff != 0);

        Ok(Self {
            sample_rate,
            channels,
            samples_per_frame: DTS_SAMPLES_PER_BLOCK * (nblks as u32 + 1),
            frame_size: fsize as usize + 1,
            amode,
            lff,
            nblks,
            fsize,
            rate,
            sfreq,
        })
    }

    /// Build a core-only [`DtsSpecificBox`] (`dtsc`) from the parsed header —
    /// ETSI TS 102 114 §E.2.2.3.2, Tables E-2/E-3/E-5 (docs/codec/dts-core-frame.md
    /// `into_ddts` derivation table).
    ///
    /// `channel_layout` is `0` both when `core_layout` names the arrangement
    /// (channel config is then carried by `CoreLayout`) and for the
    /// `CoreLayout == 31` fallback: Table E-5's full `AMODE`→mask
    /// correspondence is not completely transcribed in
    /// `docs/codec/dts-core-frame.md` (only 5 of its bit roles are cited), so
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
/// samples-per-channel count (mirrors [`crate::ac3::Ec3SplitFrame`] — DTS
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
/// [`crate::ac3::split_ac3_syncframes`]).
pub fn split_dts_core_frames(payload: &[u8]) -> Vec<DtsCoreFrame<'_>> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + 4 <= payload.len() {
        let word = u32::from_be_bytes([
            payload[off],
            payload[off + 1],
            payload[off + 2],
            payload[off + 3],
        ]);
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

/// FourCC of the DTS config box.
pub const DDTS_FOURCC: [u8; 4] = *b"ddts";

/// FourCC of the DTS core-only sample entry (`dtsc` — ETSI TS 102 114 Table E-1).
pub const DTSC_FOURCC: [u8; 4] = *b"dtsc";
/// FourCC of the DTS core+extension multi-asset sample entry (`dtsh`).
pub const DTSH_FOURCC: [u8; 4] = *b"dtsh";
/// FourCC of the DTS LBR-only sample entry (`dtsl`).
pub const DTSL_FOURCC: [u8; 4] = *b"dtsl";
/// FourCC of the DTS extension-substream-only sample entry (`dtse`).
pub const DTSE_FOURCC: [u8; 4] = *b"dtse";

/// Fixed serialized length of the `ddts` box body (20 bytes).
///
/// Layout (ETSI TS 102 114 §E.2.2.3.1):
/// 4 (`DTSSamplingFrequency`) + 4 (`maxBitrate`) + 4 (`avgBitrate`) +
/// 1 (`pcmSampleDepth`) + 4 (packed bits: `FrameDuration`/`StreamConstruction`/
/// `CoreLFEPresent`/`CoreLayout`/`CoreSize`/`StereoDownmix`/`RepresentationType`) +
/// 2 (`ChannelLayout`) + 1 (flags byte: `MultiAssetFlag`/`LBRDurationMod`/
/// `ReservedBoxPresent`/`Reserved`).
pub const DDTS_BODY_LEN: usize = 20;

/// `DTSSpecificBox` (`ddts` box body) — ETSI TS 102 114 §E.2.2.3.
///
/// All fields map directly to the spec syntax. The packed bit fields occupy
/// two multi-byte regions: a 32-bit word and a trailing byte.
///
/// Packed 32-bit word layout (most-significant bit first):
/// - `[31:30]` `FrameDuration` (2 bits)
/// - `[29:25]` `StreamConstruction` (5 bits)
/// - `[24]`    `CoreLFEPresent` (1 bit)
/// - `[23:18]` `CoreLayout` (6 bits)
/// - `[17:4]`  `CoreSize` (14 bits)
/// - `[3]`     `StereoDownmix` (1 bit)
/// - `[2:0]`   `RepresentationType` (3 bits)
///
/// Trailing flags byte layout:
/// - `[7]`     `MultiAssetFlag` (1 bit)
/// - `[6]`     `LBRDurationMod` (1 bit)
/// - `[5]`     `ReservedBoxPresent` (1 bit)
/// - `[4:0]`   `Reserved` (5 bits, always 0)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DtsSpecificBox {
    /// Nominal sampling frequency in Hz — ETSI TS 102 114 §E.2.2.3.1.
    pub dts_sampling_frequency: u32,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: u32,
    /// Average bitrate in bits per second.
    pub avg_bitrate: u32,
    /// PCM sample depth: 16 or 24 bits.
    pub pcm_sample_depth: u8,
    /// `[31:30]` Frame duration code: 0 = 512, 1 = 1024, 2 = 2048, 3 = 4096 samples.
    pub frame_duration: u8,
    /// `[29:25]` Stream construction code (ETSI TS 102 114 Table E-2).
    pub stream_construction: u8,
    /// `[24]` Core LFE channel present: 0 = none, 1 = LFE exists.
    pub core_lfe_present: bool,
    /// `[23:18]` Core channel layout code (ETSI TS 102 114 Table E-3).
    pub core_layout: u8,
    /// `[17:4]` Core substream size in bytes.
    pub core_size: u16,
    /// `[3]` Stereo downmix embedded: 0 = none, 1 = present.
    pub stereo_downmix: bool,
    /// `[2:0]` Representation type (ETSI TS 102 114 Table E-4).
    pub representation_type: u8,
    /// Channel layout bitmask (ETSI TS 102 114 Table E-5).
    pub channel_layout: u16,
    /// `[7]` Multi-asset flag: 0 = single asset, 1 = multiple assets.
    pub multi_asset_flag: bool,
    /// `[6]` LBR duration modifier: 0 = ignore, 1 = special LBR modifier.
    pub lbr_duration_mod: bool,
    /// `[5]` Reserved box present: 0 = no reserved box, 1 = reserved box present.
    pub reserved_box_present: bool,
}

impl DtsSpecificBox {
    /// RFC 6381 codec string derived from the DTS sample-entry FourCC.
    ///
    /// Returns the fourcc as a UTF-8 string. For the four defined DTS FourCCs
    /// (`dtsc`, `dtsh`, `dtsl`, `dtse`) the result is the codec identifier used
    /// in `Content-Type` and HLS `CODECS=` attributes.
    pub fn rfc6381(codec_fourcc: &[u8; 4]) -> &'static str {
        match codec_fourcc {
            b"dtsc" => "dtsc",
            b"dtsh" => "dtsh",
            b"dtsl" => "dtsl",
            b"dtse" => "dtse",
            _ => "dtsc",
        }
    }

    /// Encode the two packed multi-bit regions into a single u32 (big-endian word).
    ///
    /// Bit layout (MSB first):
    /// `[31:30]` `frame_duration`, `[29:25]` `stream_construction`,
    /// `[24]` `core_lfe_present`, `[23:18]` `core_layout`,
    /// `[17:4]` `core_size`, `[3]` `stereo_downmix`,
    /// `[2:0]` `representation_type`.
    fn encode_packed_word(&self) -> u32 {
        let mut w: u32 = 0;
        w |= (u32::from(self.frame_duration) & 0x03) << 30;
        w |= (u32::from(self.stream_construction) & 0x1F) << 25;
        w |= u32::from(self.core_lfe_present) << 24;
        w |= (u32::from(self.core_layout) & 0x3F) << 18;
        w |= (u32::from(self.core_size) & 0x3FFF) << 4;
        w |= u32::from(self.stereo_downmix) << 3;
        w |= u32::from(self.representation_type) & 0x07;
        w
    }

    /// Decode the packed 32-bit word produced by [`encode_packed_word`].
    fn decode_packed_word(w: u32) -> (u8, u8, bool, u8, u16, bool, u8) {
        let frame_duration = ((w >> 30) & 0x03) as u8;
        let stream_construction = ((w >> 25) & 0x1F) as u8;
        let core_lfe_present = ((w >> 24) & 0x01) != 0;
        let core_layout = ((w >> 18) & 0x3F) as u8;
        let core_size = ((w >> 4) & 0x3FFF) as u16;
        let stereo_downmix = ((w >> 3) & 0x01) != 0;
        let representation_type = (w & 0x07) as u8;
        (
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
        )
    }

    /// Encode the trailing flags byte.
    ///
    /// Bit layout: `[7]` `multi_asset_flag`, `[6]` `lbr_duration_mod`,
    /// `[5]` `reserved_box_present`, `[4:0]` Reserved (always 0).
    fn encode_flags_byte(&self) -> u8 {
        let mut b: u8 = 0;
        if self.multi_asset_flag {
            b |= 0x80;
        }
        if self.lbr_duration_mod {
            b |= 0x40;
        }
        if self.reserved_box_present {
            b |= 0x20;
        }
        b
    }
}

impl<'a> Parse<'a> for DtsSpecificBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < DDTS_BODY_LEN {
            return Err(Error::BufferTooShort {
                need: DDTS_BODY_LEN,
                have: bytes.len(),
                what: "ddts",
            });
        }
        let dts_sampling_frequency = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let max_bitrate = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let avg_bitrate = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let pcm_sample_depth = bytes[12];
        let packed = u32::from_be_bytes([bytes[13], bytes[14], bytes[15], bytes[16]]);
        let (
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
        ) = Self::decode_packed_word(packed);
        let channel_layout = u16::from_be_bytes([bytes[17], bytes[18]]);
        let flags_byte = bytes[19];
        let multi_asset_flag = (flags_byte & 0x80) != 0;
        let lbr_duration_mod = (flags_byte & 0x40) != 0;
        let reserved_box_present = (flags_byte & 0x20) != 0;

        Ok(Self {
            dts_sampling_frequency,
            max_bitrate,
            avg_bitrate,
            pcm_sample_depth,
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
            channel_layout,
            multi_asset_flag,
            lbr_duration_mod,
            reserved_box_present,
        })
    }
}

impl Serialize for DtsSpecificBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        DDTS_BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = DDTS_BODY_LEN;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.dts_sampling_frequency.to_be_bytes());
        buf[4..8].copy_from_slice(&self.max_bitrate.to_be_bytes());
        buf[8..12].copy_from_slice(&self.avg_bitrate.to_be_bytes());
        buf[12] = self.pcm_sample_depth;
        buf[13..17].copy_from_slice(&self.encode_packed_word().to_be_bytes());
        buf[17..19].copy_from_slice(&self.channel_layout.to_be_bytes());
        buf[19] = self.encode_flags_byte();
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    /// Spec-derived test vector: hand-computed from §E.2.2.3.1 field layout.
    ///
    /// Fields chosen to exercise every non-zero field and keep mental arithmetic
    /// simple. See `transmux/tests/dts.rs` for the full integration gate.
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

    // -----------------------------------------------------------------
    // DtsCoreFrameInfo — hand-rolled bit writer (independent of production
    // `read_bits`) builds synthetic core headers to exercise the parser.
    // -----------------------------------------------------------------

    fn write_bits(buf: &mut [u8], bit_pos: &mut usize, n: usize, val: u64) {
        for i in (0..n).rev() {
            let byte_idx = *bit_pos / 8;
            let bit_in_byte = 7 - (*bit_pos % 8);
            let bit = ((val >> i) & 1) as u8;
            buf[byte_idx] = (buf[byte_idx] & !(1 << bit_in_byte)) | (bit << bit_in_byte);
            *bit_pos += 1;
        }
    }

    /// Build a synthetic DTS core frame header (Table 5-1), header only (11 bytes).
    fn build_core_header(
        nblks: u8,
        fsize: u16,
        amode: u8,
        sfreq: u8,
        rate: u8,
        lff: u8,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        let mut bp = 0usize;
        write_bits(&mut buf, &mut bp, 32, DTS_CORE_SYNC_WORD as u64);
        write_bits(&mut buf, &mut bp, 1, 1); // FTYPE: normal frame
        write_bits(&mut buf, &mut bp, 5, 31); // SHORT
        write_bits(&mut buf, &mut bp, 1, 0); // CPF
        write_bits(&mut buf, &mut bp, 7, nblks as u64);
        write_bits(&mut buf, &mut bp, 14, fsize as u64);
        write_bits(&mut buf, &mut bp, 6, amode as u64);
        write_bits(&mut buf, &mut bp, 4, sfreq as u64);
        write_bits(&mut buf, &mut bp, 5, rate as u64);
        write_bits(&mut buf, &mut bp, 1, 0); // FixedBit
        write_bits(&mut buf, &mut bp, 1, 0); // DYNF
        write_bits(&mut buf, &mut bp, 1, 0); // TIMEF
        write_bits(&mut buf, &mut bp, 1, 0); // AUXF
        write_bits(&mut buf, &mut bp, 1, 0); // HDCD
        write_bits(&mut buf, &mut bp, 3, 0); // EXT_AUDIO_ID
        write_bits(&mut buf, &mut bp, 1, 0); // EXT_AUDIO
        write_bits(&mut buf, &mut bp, 1, 0); // ASPF
        write_bits(&mut buf, &mut bp, 2, lff as u64); // LFF
        write_bits(&mut buf, &mut bp, 1, 0); // HFLAG
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

    // Fixture-derived parameters (`fixtures/ts/dts/dts_core.ts`): stereo 48 kHz
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
        assert!(DtsCoreFrameInfo::from_es(&junk).is_err());
    }

    #[test]
    fn from_es_rejects_nblks_below_minimum() {
        let hdr = build_core_header(3, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(DtsCoreFrameInfo::from_es(&hdr).is_err());
    }

    #[test]
    fn from_es_rejects_fsize_below_minimum() {
        let hdr = build_core_header(FX_NBLKS, 50, FX_AMODE, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(DtsCoreFrameInfo::from_es(&hdr).is_err());
    }

    #[test]
    fn from_es_rejects_invalid_sfreq() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, 0b0000, FX_RATE, FX_LFF);
        assert!(DtsCoreFrameInfo::from_es(&hdr).is_err());
    }

    #[test]
    fn from_es_rejects_user_defined_amode() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, 0b010000, FX_SFREQ, FX_RATE, FX_LFF);
        assert!(DtsCoreFrameInfo::from_es(&hdr).is_err());
    }

    #[test]
    fn from_es_lfe_adds_a_channel() {
        let hdr = build_core_header(FX_NBLKS, FX_FSIZE, FX_AMODE, FX_SFREQ, FX_RATE, 1);
        let info = DtsCoreFrameInfo::from_es(&hdr).unwrap();
        assert_eq!(info.channels, 3, "LFF != 0 must add one channel");
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
}
