//! AC-3 and Enhanced AC-3 in ISOBMFF — ETSI TS 102 366 Annex F.
//!
//! # Types
//!
//! | Box | FourCC | Spec | Description |
//! |-----|--------|------|-------------|
//! | [`Ac3SpecificBox`] | `dac3` | §F.4 | AC-3 decoder config |
//! | [`Ec3SpecificBox`] | `dec3` | §F.6 | E-AC-3 decoder config |
//!
//! # Syncframe BSI parsers
//!
//! [`Ac3SyncframeInfo::from_es`] parses the BSI fields from an AC-3 syncframe
//! (§4.3.2) and can build an [`Ac3SpecificBox`] via `into_dac3()`. Similarly
//! [`Ec3SyncframeInfo::from_es`] parses the E-AC-3 syncframe (§E.1.2.2/E.1.3.1) and builds an
//! [`Ec3SpecificBox`] via `into_dec3()`.
//!
//! Both parsers scan for the `0x0B77` syncword.
//!
//! # Syncframe splitting (issue #556)
//!
//! [`split_ac3_syncframes`] / [`split_eac3_syncframes`] split a concatenated
//! PES payload (which may carry several syncframes back to back) into
//! individual access units, using the frame length recovered from each
//! syncframe's own bit stream information rather than assuming one PES
//! payload equals one syncframe.

use crate::error::{Error, Result};
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// AC-3 syncword (big-endian).
const AC3_SYNCWORD: u16 = 0x0B77;

/// Bytes per "word" in the AC-3 Table 4.13 frame-size table caption ("1 word
/// = 16 bits") — ETSI TS 102 366 §4.4.1.4 / Table 4.13.
const BYTES_PER_WORD: usize = 2;

/// Samples per AC-3/E-AC-3 audio block (`audblk()`) — ETSI TS 102 366 §3.1
/// (Definitions): "audio block: set of 512 audio samples consisting of 256
/// samples of the preceding audio block, and 256 new time samples. NOTE 1: A
/// new audio block occurs every 256 audio samples." (Verified against the
/// spec; excerpt appended to `docs/codec/ac3-syncframe.md`.)
pub(crate) const SAMPLES_PER_AUDIO_BLOCK: u32 = 256;

/// AC-3 blocks per syncframe — ETSI TS 102 366 §4.3.0 `syncframe()`:
/// `for(blk = 0; blk < 6; blk++) { audblk(); }`.
const AC3_BLOCKS_PER_SYNCFRAME: u32 = 6;

/// Samples per AC-3 syncframe: `AC3_BLOCKS_PER_SYNCFRAME` ×
/// `SAMPLES_PER_AUDIO_BLOCK` = 1536 — stated directly in ETSI TS 102 366
/// §7.2.1.2: "each AC-3 syncframe contains 1 536 samples of audio per
/// channel" (excerpt appended to `docs/codec/ac3-syncframe.md`).
pub const AC3_SAMPLES_PER_SYNCFRAME: u32 = AC3_BLOCKS_PER_SYNCFRAME * SAMPLES_PER_AUDIO_BLOCK;

/// `strmtyp` value marking a dependent-substream E-AC-3 syncframe — ETSI TS
/// 102 366 Annex E §E.1.2.2 `bsi()`: `if(strmtyp == 0x1) /* if dependent
/// stream */`.
const EAC3_STRMTYP_DEPENDENT: u8 = 1;

/// AC-3 frame-size table — ETSI TS 102 366 Table 4.13 ("Frame size code table
/// (1 word = 16 bits)"), indexed by `frmsizecod` (0..=37; 38..=63 reserved).
/// Each row is `[words @ fscod 32 kHz, words @ fscod 44.1 kHz, words @ fscod
/// 48 kHz]`, matching the doc's column order.
#[rustfmt::skip]
const AC3_FRAME_SIZE_WORDS: [[u16; 3]; 38] = [
    [  96,   69,   64], [  96,   70,   64],
    [ 120,   87,   80], [ 120,   88,   80],
    [ 144,  104,   96], [ 144,  105,   96],
    [ 168,  121,  112], [ 168,  122,  112],
    [ 192,  139,  128], [ 192,  140,  128],
    [ 240,  174,  160], [ 240,  175,  160],
    [ 288,  208,  192], [ 288,  209,  192],
    [ 336,  243,  224], [ 336,  244,  224],
    [ 384,  278,  256], [ 384,  279,  256],
    [ 480,  348,  320], [ 480,  349,  320],
    [ 576,  417,  384], [ 576,  418,  384],
    [ 672,  487,  448], [ 672,  488,  448],
    [ 768,  557,  512], [ 768,  558,  512],
    [ 960,  696,  640], [ 960,  697,  640],
    [1152,  835,  768], [1152,  836,  768],
    [1344,  975,  896], [1344,  976,  896],
    [1536, 1114, 1024], [1536, 1115, 1024],
    [1728, 1253, 1152], [1728, 1254, 1152],
    [1920, 1393, 1280], [1920, 1394, 1280],
];

/// Words-per-syncframe for `(fscod, frmsizecod)` — Table 4.13. `fscod == 3`
/// (reserved, Table 4.1) or `frmsizecod > 37` (reserved, Table 4.13) yield
/// `None`.
fn ac3_frame_words(fscod: u8, frmsizecod: u8) -> Option<u16> {
    let row = AC3_FRAME_SIZE_WORDS.get(frmsizecod as usize)?;
    // Table 4.1: fscod 0 = 48 kHz, 1 = 44.1 kHz, 2 = 32 kHz — Table 4.13's
    // columns are ordered 32/44.1/48 kHz, so map fscod to the matching index.
    let col = match fscod {
        0 => 2, // 48 kHz
        1 => 1, // 44.1 kHz
        2 => 0, // 32 kHz
        _ => return None,
    };
    Some(row[col])
}

// ---------------------------------------------------------------------------
// AC-3 syncframe BSI — §4.3.1 syncinfo + §4.3.2 bsi
// ---------------------------------------------------------------------------

/// Fields parsed from an AC-3 `syncinfo()` + `bsi()`, sufficient to build an
/// [`Ac3SpecificBox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ac3SyncframeInfo {
    pub fscod: u8,
    pub frmsizecod: u8,
    pub bsid: u8,
    pub bsmod: u8,
    pub acmod: u8,
    pub lfeon: bool,
    pub sample_rate: u32,
}

impl Ac3SyncframeInfo {
    /// Parse the first syncframe from an AC-3 elementary stream buffer.
    /// Scans for the `0x0B77` syncword, then parses `syncinfo()` + `bsi()`.
    pub fn from_es(data: &[u8]) -> Result<Self> {
        let off = find_syncword(data)?;
        Self::parse_at(data, off)
    }

    fn parse_at(data: &[u8], off: usize) -> Result<Self> {
        // syncword(16) + crc1(16) + fscod(2) + frmsizecod(6) = 40 bits = 5 bytes
        let need = off + 5;
        if data.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: data.len(),
                what: "AC-3 syncinfo",
            });
        }
        // Use BitReader on the raw syncframe bytes (no emulation-prevention
        // in AC-3 — it's a raw bitstream).
        let es = &data[off..];
        // Parse inline with manual bit extraction — the fields are simple
        // enough and we want no_std compat.
        let mut bit_pos = 0usize;

        // syncword: 16 bits (skip — already scanned)
        bit_pos += 16;
        // crc1: 16 bits (skip)
        bit_pos += 16;
        // fscod: 2 bits
        let fscod = read_bits(es, &mut bit_pos, 2, "fscod")? as u8;
        // frmsizecod: 6 bits
        let frmsizecod = read_bits(es, &mut bit_pos, 6, "frmsizecod")? as u8;
        // ---- bsi() starts here ----
        // bsid: 5 bits
        let bsid = read_bits(es, &mut bit_pos, 5, "bsid")? as u8;
        // bsmod: 3 bits
        let bsmod = read_bits(es, &mut bit_pos, 3, "bsmod")? as u8;
        // acmod: 3 bits
        let acmod = read_bits(es, &mut bit_pos, 3, "acmod")? as u8;
        // cmixlev(2) if acmod has 3 front channels
        if (acmod & 0x1) != 0 && acmod != 0x1 {
            bit_pos += 2; // cmixlev
        }
        // surmixlev(2) if surround channel exists
        if (acmod & 0x4) != 0 {
            bit_pos += 2; // surmixlev
        }
        // dsurmod(2) if 2/0 mode
        if acmod == 0x2 {
            bit_pos += 2; // dsurmod
        }
        // lfeon: 1 bit
        let lfeon = read_bits(es, &mut bit_pos, 1, "lfeon")? != 0;

        let sample_rate = ac3_sample_rate(fscod);

        Ok(Self {
            fscod,
            frmsizecod,
            bsid,
            bsmod,
            acmod,
            lfeon,
            sample_rate,
        })
    }

    /// Build an [`Ac3SpecificBox`] from the parsed syncframe fields.
    pub fn into_dac3(self) -> Ac3SpecificBox {
        Ac3SpecificBox {
            fscod: self.fscod,
            bsid: self.bsid,
            bsmod: self.bsmod,
            acmod: self.acmod,
            lfeon: self.lfeon,
            bit_rate_code: self.frmsizecod >> 1,
        }
    }

    /// Number of full-bandwidth channels derived from `acmod` (Table 4.5).
    pub fn channel_count(&self) -> u8 {
        acmod_channels(self.acmod)
    }

    /// Coded length of this syncframe in bytes:
    /// `words_per_syncframe(fscod, frmsizecod) * 2` (Table 4.13 — "1 word =
    /// 16 bits"). `None` for a reserved `fscod`/`frmsizecod`.
    pub fn frame_len_bytes(&self) -> Option<usize> {
        ac3_frame_words(self.fscod, self.frmsizecod).map(|w| w as usize * BYTES_PER_WORD)
    }
}

/// Split a concatenated AC-3 PES payload into individual syncframes, using
/// the frame length recovered from each syncframe's own BSI (Table 4.13)
/// rather than assuming one PES payload equals one syncframe. Stops at the
/// first bad sync word / truncated tail so a partial trailing frame does not
/// lose the earlier ones (mirrors `ts_demux::split_adts_frames`).
pub fn split_ac3_syncframes(payload: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + 2 <= payload.len() {
        if u16::from_be_bytes([payload[off], payload[off + 1]]) != AC3_SYNCWORD {
            break;
        }
        let Ok(info) = Ac3SyncframeInfo::parse_at(payload, off) else {
            break;
        };
        let Some(len) = info.frame_len_bytes() else {
            break;
        };
        if len == 0 || off + len > payload.len() {
            break;
        }
        frames.push(&payload[off..off + len]);
        off += len;
    }
    frames
}

// ---------------------------------------------------------------------------
// E-AC-3 syncframe BSI — §E.1.2.2 / E.1.3.1
// ---------------------------------------------------------------------------

/// Fields parsed from an E-AC-3 syncframe, sufficient to build an
/// [`Ec3SpecificBox`] and calculate `data_rate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ec3SyncframeInfo {
    pub strmtyp: u8,
    pub substreamid: u8,
    pub frmsiz: u16,
    pub fscod: u8,
    pub numblks: u8,
    pub acmod: u8,
    pub lfeon: bool,
    pub bsid: u8,
    pub sample_rate: u32,
    /// Effective sample rate in kHz (as f32) for data_rate calculation.
    pub sample_rate_khz: u32,
}

impl Ec3SyncframeInfo {
    /// Parse the first E-AC-3 syncframe from an elementary stream buffer.
    pub fn from_es(data: &[u8]) -> Result<Self> {
        let off = find_syncword(data)?;
        Self::parse_at(data, off)
    }

    fn parse_at(data: &[u8], off: usize) -> Result<Self> {
        // We need at least syncword(16) + strmtyp(2) + substreamid(3) +
        // frmsiz(11) + fscod(2) + [sr_code2(2) or numblkscod(2)] +
        // acmod(3) + lfeon(1) + bsid(5)
        // = 16 + 2 + 3 + 11 + 2 + 2 + 3 + 1 + 5 = 45 bits minimum → 6 bytes
        let need = off + 6;
        if data.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: data.len(),
                what: "E-AC-3 syncframe",
            });
        }
        let es = &data[off..];
        let mut bit_pos = 0usize;

        // syncword: 16 bits
        bit_pos += 16;
        // strmtyp: 2 bits
        let strmtyp = read_bits(es, &mut bit_pos, 2, "strmtyp")? as u8;
        // substreamid: 3 bits
        let substreamid = read_bits(es, &mut bit_pos, 3, "substreamid")? as u8;
        // frmsiz: 11 bits
        let frmsiz = read_bits(es, &mut bit_pos, 11, "frmsiz")? as u16;
        // fscod: 2 bits
        let fscod = read_bits(es, &mut bit_pos, 2, "fscod")? as u8;

        let (sample_rate, sample_rate_khz, numblks) = if fscod == 3 {
            // sr_code2: 2 bits (half-rate mode)
            let sr_code2 = read_bits(es, &mut bit_pos, 2, "sr_code2")? as u8;
            // numblkscod not present; default 6 blocks
            let sr = eac3_half_sample_rate(sr_code2);
            (sr, eac3_half_sample_rate_khz(sr_code2), 6u8)
        } else {
            // numblkscod: 2 bits
            let numblkscod = read_bits(es, &mut bit_pos, 2, "numblkscod")? as u8;
            let nb = eac3_num_blocks(numblkscod);
            let sr = ac3_sample_rate(fscod);
            (sr, ac3_sample_rate_khz(fscod), nb)
        };
        // acmod: 3 bits
        let acmod = read_bits(es, &mut bit_pos, 3, "acmod")? as u8;
        // lfeon: 1 bit
        let lfeon = read_bits(es, &mut bit_pos, 1, "lfeon")? != 0;
        // bsid: 5 bits
        let bsid = read_bits(es, &mut bit_pos, 5, "bsid")? as u8;

        Ok(Self {
            strmtyp,
            substreamid,
            frmsiz,
            fscod,
            numblks,
            acmod,
            lfeon,
            bsid,
            sample_rate,
            sample_rate_khz,
        })
    }

    /// Build an [`Ec3SpecificBox`] from the parsed syncframe fields.
    /// Calculates `data_rate` per §F.6.2.2 with ceiling division:
    /// `data_rate = ceil((frmsiz + 1) * sample_rate / (numblks * 16 * 1000))`
    pub fn into_dec3(self) -> Ec3SpecificBox {
        let num = (self.frmsiz as u32 + 1) * self.sample_rate;
        let den = (self.numblks as u32) * 16 * 1000;
        let data_rate = if den > 0 { num.div_ceil(den) } else { 0 };
        Ec3SpecificBox {
            data_rate: data_rate as u16,
            num_ind_sub: self.substreamid,
            substreams: vec![Ec3Substream {
                fscod: self.fscod,
                bsid: self.bsid,
                asvc: false,
                bsmod: 0,
                acmod: self.acmod,
                lfeon: self.lfeon,
                num_dep_sub: 0,
                chan_loc: None,
            }],
        }
    }

    /// Number of full-bandwidth channels.
    pub fn channel_count(&self) -> u8 {
        acmod_channels(self.acmod)
    }

    /// Samples encoded by this access unit: `numblks` audio blocks ×
    /// `SAMPLES_PER_AUDIO_BLOCK` samples/block.
    pub fn samples_per_frame(&self) -> u32 {
        self.numblks as u32 * SAMPLES_PER_AUDIO_BLOCK
    }
}

/// One split E-AC-3 access unit: an independent syncframe's bytes, with any
/// immediately-following dependent-substream syncframes (`strmtyp == 0x1`,
/// Annex E §E.1.2.2) concatenated onto it, plus the independent frame's
/// parsed syncframe info (used for the access unit's duration).
#[derive(Debug, Clone)]
pub struct Ec3SplitFrame {
    /// Concatenated coded bytes: the independent frame followed by any
    /// dependent frames belonging to the same access unit.
    pub data: Vec<u8>,
    /// The independent frame's parsed syncframe info.
    pub info: Ec3SyncframeInfo,
}

/// Split a concatenated E-AC-3 PES payload into access units: each
/// independent syncframe (`strmtyp != 0x1`) starts a new access unit; a
/// dependent-substream syncframe (`strmtyp == 0x1`) immediately following is
/// concatenated into that access unit (Annex E §E.1.2.2 `bsi()`). Stops at the
/// first bad sync word / truncated tail (mirrors [`split_ac3_syncframes`]).
///
/// Frame length is `(frmsiz + 1) * 2` bytes — ETSI TS 102 366 §E.1.3.1.3:
/// "The frmsiz field indicates a value one less than the overall size of the
/// coded syncframe in 16-bit words" (excerpt appended to
/// `docs/codec/eac3-syncframe.md`).
///
/// Multi-program E-AC-3 (independent substreams with `substreamid` 1..=7,
/// §E.1.3.1.2) is not disentangled: every independent frame starts a new
/// access unit in stream order (single-program streams are unaffected).
pub fn split_eac3_syncframes(payload: &[u8]) -> Vec<Ec3SplitFrame> {
    let mut out: Vec<Ec3SplitFrame> = Vec::new();
    let mut off = 0usize;
    while off + 2 <= payload.len() {
        if u16::from_be_bytes([payload[off], payload[off + 1]]) != AC3_SYNCWORD {
            break;
        }
        let Ok(info) = Ec3SyncframeInfo::parse_at(payload, off) else {
            break;
        };
        let len = (info.frmsiz as usize + 1) * BYTES_PER_WORD;
        if len == 0 || off + len > payload.len() {
            break;
        }
        let frame_bytes = &payload[off..off + len];
        if info.strmtyp == EAC3_STRMTYP_DEPENDENT {
            if let Some(last) = out.last_mut() {
                last.data.extend_from_slice(frame_bytes);
                off += len;
                continue;
            }
        }
        out.push(Ec3SplitFrame {
            data: frame_bytes.to_vec(),
            info,
        });
        off += len;
    }
    out
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
}

impl<'a> Parse<'a> for Ac3SpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Box body is 3 bytes
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "dac3 body",
            });
        }
        let mut bit_pos = 0usize;
        let fscod = read_bits(bytes, &mut bit_pos, 2, "fscod")? as u8;
        let bsid = read_bits(bytes, &mut bit_pos, 5, "bsid")? as u8;
        let bsmod = read_bits(bytes, &mut bit_pos, 3, "bsmod")? as u8;
        let acmod = read_bits(bytes, &mut bit_pos, 3, "acmod")? as u8;
        let lfeon = read_bits(bytes, &mut bit_pos, 1, "lfeon")? != 0;
        let bit_rate_code = read_bits(bytes, &mut bit_pos, 5, "bit_rate_code")? as u8;
        let _reserved = read_bits(bytes, &mut bit_pos, 5, "reserved")?;
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
        3
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 3 {
            return Err(Error::OutputBufferTooSmall {
                need: 3,
                have: buf.len(),
            });
        }
        let mut bit_pos = 0usize;
        write_bits(buf, &mut bit_pos, 2, self.fscod as u64);
        write_bits(buf, &mut bit_pos, 5, self.bsid as u64);
        write_bits(buf, &mut bit_pos, 3, self.bsmod as u64);
        write_bits(buf, &mut bit_pos, 3, self.acmod as u64);
        write_bits(buf, &mut bit_pos, 1, self.lfeon as u64);
        write_bits(buf, &mut bit_pos, 5, self.bit_rate_code as u64);
        write_bits(buf, &mut bit_pos, 5, 0); // reserved
        Ok(3)
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
    pub substreams: Vec<Ec3Substream>,
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
}

fn ec3_substream_serialized_len(_sub: &Ec3Substream) -> usize {
    // fscod(2)+bsid(5)+reserved(1)+asvc(1)+bsmod(3)+acmod(3)+lfeon(1)+reserved(3)+num_dep_sub(4)
    // + chan_loc(9) if num_dep_sub>0 else reserved(1)
    // = 23 bits + (9 or 1) → 24 bits = 3 bytes per substream
    3
}

impl<'a> Parse<'a> for Ec3SpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "dec3 body",
            });
        }
        let mut bit_pos = 0usize;
        let data_rate = read_bits(bytes, &mut bit_pos, 13, "data_rate")? as u16;
        let num_ind_sub = read_bits(bytes, &mut bit_pos, 3, "num_ind_sub")? as u8;
        let num_sub = num_ind_sub as usize + 1;

        let need_bytes = 2 + num_sub
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
        if bytes.len() < need_bytes {
            return Err(Error::BufferTooShort {
                need: need_bytes,
                have: bytes.len(),
                what: "dec3 substreams",
            });
        }

        let mut substreams = Vec::with_capacity(num_sub);
        for _ in 0..num_sub {
            let fscod = read_bits(bytes, &mut bit_pos, 2, "fscod")? as u8;
            let bsid = read_bits(bytes, &mut bit_pos, 5, "bsid")? as u8;
            let _res1 = read_bits(bytes, &mut bit_pos, 1, "reserved")?;
            let asvc = read_bits(bytes, &mut bit_pos, 1, "asvc")? != 0;
            let bsmod = read_bits(bytes, &mut bit_pos, 3, "bsmod")? as u8;
            let acmod = read_bits(bytes, &mut bit_pos, 3, "acmod")? as u8;
            let lfeon = read_bits(bytes, &mut bit_pos, 1, "lfeon")? != 0;
            let _res3 = read_bits(bytes, &mut bit_pos, 3, "reserved")?;
            let num_dep_sub = read_bits(bytes, &mut bit_pos, 4, "num_dep_sub")? as u8;
            let chan_loc = if num_dep_sub > 0 {
                Some(read_bits(bytes, &mut bit_pos, 9, "chan_loc")? as u16)
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

        Ok(Self {
            data_rate,
            num_ind_sub,
            substreams,
        })
    }
}

impl Serialize for Ec3SpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        // 2 bytes header (data_rate+num_ind_sub) + 3 bytes per substream
        2 + self.substreams.len() * 3
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut bit_pos = 0usize;
        write_bits(buf, &mut bit_pos, 13, self.data_rate as u64);
        write_bits(buf, &mut bit_pos, 3, self.num_ind_sub as u64);
        for sub in &self.substreams {
            write_bits(buf, &mut bit_pos, 2, sub.fscod as u64);
            write_bits(buf, &mut bit_pos, 5, sub.bsid as u64);
            write_bits(buf, &mut bit_pos, 1, 0); // reserved
            write_bits(buf, &mut bit_pos, 1, sub.asvc as u64);
            write_bits(buf, &mut bit_pos, 3, sub.bsmod as u64);
            write_bits(buf, &mut bit_pos, 3, sub.acmod as u64);
            write_bits(buf, &mut bit_pos, 1, sub.lfeon as u64);
            write_bits(buf, &mut bit_pos, 3, 0); // reserved
            write_bits(buf, &mut bit_pos, 4, sub.num_dep_sub as u64);
            if sub.num_dep_sub > 0 {
                write_bits(buf, &mut bit_pos, 9, sub.chan_loc.unwrap_or(0) as u64);
            } else {
                write_bits(buf, &mut bit_pos, 1, 0); // reserved
            }
        }
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// Helpers: bit I/O, scanning, lookup tables
// ---------------------------------------------------------------------------

/// Find the byte offset of the `0x0B77` syncword in a buffer.
fn find_syncword(data: &[u8]) -> Result<usize> {
    for i in 0..data.len().saturating_sub(1) {
        let word = u16::from_be_bytes([data[i], data[i + 1]]);
        if word == AC3_SYNCWORD {
            return Ok(i);
        }
    }
    Err(Error::InvalidInput(
        "AC-3/E-AC-3 syncword (0x0B77) not found in elementary stream",
    ))
}

/// Read `n` bits MSB-first from `data` at the current `bit_pos`, advancing it.
fn read_bits(data: &[u8], bit_pos: &mut usize, n: usize, _what: &'static str) -> Result<u64> {
    if n > 64 {
        return Err(Error::InvalidValue {
            field: _what,
            value: n as u64,
            reason: "bit count > 64",
        });
    }
    let end = *bit_pos + n;
    let need_bytes = end.div_ceil(8);
    if data.len() < need_bytes {
        return Err(Error::BufferTooShort {
            need: need_bytes,
            have: data.len(),
            what: _what,
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

/// Write `n` bits from `val` MSB-first into `buf` at `bit_pos`, advancing it.
fn write_bits(buf: &mut [u8], bit_pos: &mut usize, n: usize, val: u64) {
    for i in (0..n).rev() {
        let byte_idx = *bit_pos / 8;
        let bit_in_byte = 7 - (*bit_pos % 8);
        let bit = ((val >> i) & 1) as u8;
        buf[byte_idx] = (buf[byte_idx] & !(1 << bit_in_byte)) | (bit << bit_in_byte);
        *bit_pos += 1;
    }
}

/// Sample rate lookup for AC-3 fscod (Table 4.3).
fn ac3_sample_rate(fscod: u8) -> u32 {
    match fscod {
        0 => 48000,
        1 => 44100,
        2 => 32000,
        _ => 44100, // reserved → assume 44100
    }
}

/// Sample rate in kHz (integer) for AC-3 fscod.
fn ac3_sample_rate_khz(fscod: u8) -> u32 {
    match fscod {
        0 => 48,
        1 => 44,
        2 => 32,
        _ => 44,
    }
}

/// E-AC-3 half-rate sample rate mapping (sr_code2).
fn eac3_half_sample_rate(sr_code2: u8) -> u32 {
    match sr_code2 {
        0 => 24000,
        1 => 22050,
        2 => 16000,
        _ => 22050,
    }
}

/// E-AC-3 half-rate sample rate in kHz (integer).
fn eac3_half_sample_rate_khz(sr_code2: u8) -> u32 {
    match sr_code2 {
        0 => 24,
        1 => 22,
        2 => 16,
        _ => 22,
    }
}

/// Number of audio blocks per syncframe for E-AC-3 numblkscod.
fn eac3_num_blocks(numblkscod: u8) -> u8 {
    match numblkscod {
        0 => 1,
        1 => 2,
        2 => 3,
        _ => 6,
    }
}

/// Channel count from acmod (Table 4.5).
fn acmod_channels(acmod: u8) -> u8 {
    match acmod {
        0 => 2, // 1+1 (dual mono) → 2 channels
        1 => 1, // 1/0 (mono)
        2 => 2, // 2/0
        3 => 3, // 3/0
        4 => 3, // 2/1
        5 => 4, // 3/1
        6 => 4, // 2/2
        _ => 5, // 3/2 (5.1)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Oracle bytes from DOLBY-ORACLE.md (ffmpeg-generated)
    const DAC3_ORACLE: [u8; 3] = [0x50, 0x09, 0x40];
    const DEC3_ORACLE: [u8; 5] = [0x06, 0x00, 0x60, 0x02, 0x00];

    #[test]
    fn dac3_round_trip() {
        let box1 = Ac3SpecificBox::parse(&DAC3_ORACLE).unwrap();
        assert_eq!(box1.fscod, 1);
        assert_eq!(box1.bsid, 8);
        assert_eq!(box1.bsmod, 0);
        assert_eq!(box1.acmod, 1);
        assert!(!box1.lfeon);
        assert_eq!(box1.bit_rate_code, 10);
        // bit_rate_code=10 maps to 192 kbps per Table F.4.1

        let mut buf = [0u8; 3];
        let n = box1.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf[..], &DAC3_ORACLE[..], "dac3 round-trip mismatch");
    }

    #[test]
    fn dac3_mutate_acmod_changes_bytes() {
        let mut box1 = Ac3SpecificBox::parse(&DAC3_ORACLE).unwrap();
        box1.acmod = 2; // change from mono to stereo
        let mut buf1 = [0u8; 3];
        box1.serialize_into(&mut buf1).unwrap();
        assert_ne!(
            &buf1[..],
            &DAC3_ORACLE[..],
            "mutating acmod did not change bytes"
        );
    }

    #[test]
    fn dec3_round_trip() {
        let box1 = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
        assert_eq!(box1.data_rate, 192);
        assert_eq!(box1.num_ind_sub, 0);
        assert_eq!(box1.substreams.len(), 1);
        let s0 = &box1.substreams[0];
        assert_eq!(s0.fscod, 1);
        assert_eq!(s0.bsid, 16);
        assert!(!s0.asvc);
        assert_eq!(s0.bsmod, 0);
        assert_eq!(s0.acmod, 1);
        assert!(!s0.lfeon);
        assert_eq!(s0.num_dep_sub, 0);
        assert!(s0.chan_loc.is_none());

        let mut buf = vec![0u8; box1.serialized_len()];
        let n = box1.serialize_into(&mut buf).unwrap();
        assert_eq!(n, DEC3_ORACLE.len());
        assert_eq!(&buf[..], &DEC3_ORACLE[..], "dec3 round-trip mismatch");
    }

    #[test]
    fn dec3_mutate_acmod_changes_bytes() {
        let mut box1 = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
        let orig = {
            let mut b = vec![0u8; box1.serialized_len()];
            box1.serialize_into(&mut b).unwrap();
            b
        };
        box1.substreams[0].acmod = 2;
        let mut b2 = vec![0u8; box1.serialized_len()];
        box1.serialize_into(&mut b2).unwrap();
        assert_ne!(&b2[..], &orig[..], "mutating acmod did not change bytes");
    }

    #[test]
    fn rfc6381() {
        let dac3 = Ac3SpecificBox::parse(&DAC3_ORACLE).unwrap();
        assert_eq!(dac3.rfc6381(), "ac-3");
        let dec3 = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
        assert_eq!(dec3.rfc6381(), "ec-3");
    }
}
