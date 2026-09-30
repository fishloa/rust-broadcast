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
/// `strmtyp` for an independent substream that is *not* converted from AC-3 —
/// the only value that carries the programme-mixing block in `bsi()` (§E.1.2.2).
/// `strmtyp == 0x2` is an independent substream converted from AC-3.
const EAC3_STRMTYP_INDEPENDENT: u8 = 0;

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
    split_ac3_syncframes_resyncing(payload, false)
}

/// As [`split_ac3_syncframes`], but **resynchronises** after a frame that does
/// not parse: rather than stopping, the scan advances one byte and looks for the
/// next sync word that starts a frame whose declared length fits.
///
/// A single corrupted or truncated frame mid-stream is exactly what a real
/// capture contains, and stopping at it discards every later frame — the whole
/// remainder of the stream. Resynchronising costs a rescan of at most one
/// frame's worth of bytes per corruption, and is what a demuxer recovering from
/// a damaged payload must do.
pub fn split_ac3_syncframes_resyncing(payload: &[u8], resync: bool) -> Vec<&[u8]> {
    split_ac3_syncframe_ranges(payload, resync)
        .into_iter()
        .map(|(start, end)| &payload[start..end])
        .collect()
}

/// As [`split_ac3_syncframes_resyncing`], but returning each frame's
/// `(start, end)` byte range instead of a slice.
///
/// The ranges — not just the slices — are what a caller needs: after a resync
/// the frames are **not** contiguous from offset 0, so a caller that rebuilt
/// offsets by summing frame lengths would slice at the wrong places and emit
/// samples that start mid-frame (which is exactly what the PS demuxer did until
/// it was given these ranges).
pub fn split_ac3_syncframe_ranges(payload: &[u8], resync: bool) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut off = 0usize;
    while off + 2 <= payload.len() {
        if u16::from_be_bytes([payload[off], payload[off + 1]]) != AC3_SYNCWORD {
            if !resync {
                break;
            }
            off += 1;
            continue;
        }
        // A sync word is not enough: the header must parse and declare a length
        // the payload actually holds, or this is a false sync inside a frame.
        let ok = Ac3SyncframeInfo::parse_at(payload, off)
            .ok()
            .and_then(|info| info.frame_len_bytes())
            .filter(|&len| len != 0 && off + len <= payload.len());
        match ok {
            Some(len) => {
                ranges.push((off, off + len));
                off += len;
            }
            None if resync => off += 1,
            None => break,
        }
    }
    ranges
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
    /// Custom channel map of a dependent substream (`chanmap`, §E.1.3.1.8),
    /// present in `bsi()` only when `strmtyp == 0x1` and `chanmape == 1`.
    /// `None` means the channel map is defined by `acmod`/`lfeon` instead.
    pub chanmap: Option<u16>,
    /// Bit stream mode from informational metadata (`bsmod`, §E.1.3.1.x);
    /// 0 when the independent substream carries no `infomdate`.
    pub bsmod: u8,
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
        // The rest of `bsi()` (§E.1.2.2): skip to the fields that feed dec3 —
        // a dependent substream's custom channel map (`chanmap`, E.1.3.1.7/.8)
        // and the informational-metadata `bsmod` (E.1.3.1.x).
        let (chanmap, bsmod) =
            parse_eac3_bsi_tail(es, &mut bit_pos, strmtyp, acmod, lfeon, numblks)?;

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
            chanmap,
            bsmod,
        })
    }

    /// Parse the syncframes of the **first access unit** in an E-AC-3
    /// elementary stream: one independent frame plus the dependent frames that
    /// follow it (§E.2.8).
    ///
    /// This — not [`Self::from_es_all`] — is what
    /// [`Ec3SpecificBox::from_syncframes`] wants. A `dec3` describes the
    /// *bitstream's* substream layout, and every access unit of a stream
    /// repeats the same layout, so feeding a whole backlog of them would push
    /// one substream per repeated independent frame (N frames → N substreams,
    /// with `data_rate` summed and `num_ind_sub` taken from the last), which the
    /// serializer then rejects. Stops at the first bad sync word or truncated
    /// tail, like [`split_eac3_syncframes`].
    pub fn from_es_first_au(data: &[u8]) -> Vec<Self> {
        let frames = Self::from_es_all(data);
        // An access unit ends where the next one begins: an independent frame
        // for a substreamid already seen. `strmtyp == 0x2` counts as
        // independent too (an AC-3-derived stream carries no dependent frames).
        let mut seen_independent: Vec<u8> = Vec::new();
        let mut out = Vec::new();
        for f in frames {
            if f.strmtyp != EAC3_STRMTYP_DEPENDENT {
                if seen_independent.contains(&f.substreamid) {
                    break;
                }
                seen_independent.push(f.substreamid);
            }
            out.push(f);
        }
        out
    }

    /// Parse **every** syncframe in an E-AC-3 elementary stream, in order —
    /// the independent frames plus the dependent frames that follow them. Stops
    /// at the first bad sync word or truncated tail, like
    /// [`split_eac3_syncframes`].
    ///
    /// Most callers want [`Self::from_es_first_au`] instead: a whole backlog
    /// spans many access units, and only the first one should reach
    /// [`Ec3SpecificBox::from_syncframes`].
    pub fn from_es_all(data: &[u8]) -> Vec<Self> {
        let mut out = Vec::new();
        let mut off = 0usize;
        while off + 2 <= data.len() {
            if u16::from_be_bytes([data[off], data[off + 1]]) != AC3_SYNCWORD {
                break;
            }
            let Ok(info) = Self::parse_at(data, off) else {
                break;
            };
            let len = (info.frmsiz as usize + 1) * BYTES_PER_WORD;
            if len == 0 || off + len > data.len() {
                break;
            }
            out.push(info);
            off += len;
        }
        out
    }

    /// Build an [`Ec3SpecificBox`] from the parsed syncframe fields.
    /// Calculates `data_rate` per §F.6.2.2 with ceiling division:
    /// `data_rate = ceil((frmsiz + 1) * sample_rate / (numblks * 16 * 1000))`
    ///
    /// `num_ind_sub` is §F.6.2.3's "substreamID value of the last independent
    /// substream" — 0 for a single-programme stream, which is what one parsed
    /// independent syncframe can tell us. Use
    /// [`Ec3SpecificBox::from_syncframes`] when the whole elementary stream is
    /// available, so dependent substreams are folded into `num_dep_sub`/
    /// `chan_loc` instead of being signalled as a plain 5.1 track.
    pub fn into_dec3(self) -> Result<Ec3SpecificBox> {
        Ec3SpecificBox::from_syncframes(&[self])
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
        if info.strmtyp == EAC3_STRMTYP_DEPENDENT
            && let Some(last) = out.last_mut()
        {
            last.data.extend_from_slice(frame_bytes);
            off += len;
            continue;
        }
        out.push(Ec3SplitFrame {
            data: frame_bytes.to_vec(),
            info,
        });
        off += len;
    }
    out
}

/// The `(start, end)` byte range of every E-AC-3 **syncframe** (independent or
/// dependent) in a payload, in stream order.
///
/// Distinct from [`split_eac3_syncframes`], which folds dependent syncframes
/// into their independent access unit. Sample-AES needs the syncframe ranges,
/// not the access units: the protected block is a single syncframe, and the
/// independent oracle (`tests/fixtures/sample_aes_eac3/`) shows the CBC IV is
/// **reset at every syncframe** (`docs/drm/hls-sample-aes.md` §6).
///
/// Stops at the first bad sync word / truncated tail (mirrors
/// [`split_ac3_syncframes`]); the caller then treats the payload as unwalkable.
pub fn split_eac3_syncframe_ranges(payload: &[u8]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
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
        ranges.push((off, off + len));
        off += len;
    }
    ranges
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

    /// Build a `dec3` from the E-AC-3 syncframes of one elementary stream, in
    /// stream order (§F.6 / Annex E §E.2.8).
    ///
    /// Independent frames (`strmtyp` 0 or 2) become substreams; the dependent
    /// frames (`strmtyp` 1) that follow one are folded into its
    /// `num_dep_sub`/`chan_loc`: `num_dep_sub` is the `substreamid` of the last
    /// dependent frame before the next independent one (§F.6.2.12), and
    /// `chan_loc` collects the channel locations of every dependent frame's
    /// `chanmap` (§F.6.2.13). `num_ind_sub` is the `substreamid` of the last
    /// independent frame (§F.6.2.3) — 0 for a single-programme stream.
    ///
    /// The pre-fix builder derived `num_ind_sub` from the frame's own
    /// `substreamid` and always wrote `num_dep_sub == 0`/`chan_loc == None`, so a
    /// 7.1 DD+ stream was declared as N+1 substreams carrying one, and any
    /// dependent-substream content was signalled as plain 5.1.
    ///
    /// `frames` must be the syncframes of **one access unit**
    /// ([`Ec3SyncframeInfo::from_es_first_au`]): `data_rate` is the sum over
    /// that AU's substreams (§F.6.2.2) and `num_ind_sub` its last independent
    /// substream. Passing a whole backlog of access units would repeat each
    /// substream once per AU.
    ///
    /// Errors when `frames` has no independent frame — a dependent-only or empty
    /// slice describes no substream, and the resulting box could not be
    /// serialized at all (r04-W5).
    pub fn from_syncframes(frames: &[Ec3SyncframeInfo]) -> Result<Self> {
        // data_rate is the sum over all substreams (§F.6.2.2), each computed
        // with ceiling division: ceil((frmsiz + 1) * fs / (numblks * 16 * 1000)).
        let mut data_rate: u32 = 0;
        let mut substreams: Vec<Ec3Substream> = Vec::new();
        for f in frames {
            let num = (f.frmsiz as u32 + 1) * f.sample_rate;
            let den = (f.numblks as u32) * 16 * 1000;
            if den > 0 {
                data_rate = data_rate.saturating_add(num.div_ceil(den));
            }
            if f.strmtyp == EAC3_STRMTYP_DEPENDENT {
                // Fold into the preceding independent substream. A dependent
                // frame with no independent predecessor carries no substream of
                // its own, so it is dropped rather than inventing one.
                if let Some(prev) = substreams.last_mut() {
                    prev.num_dep_sub = f.substreamid;
                    prev.chan_loc = Some(match prev.chan_loc {
                        Some(loc) => loc | chan_loc_for_frame(f),
                        None => chan_loc_for_frame(f),
                    });
                }
                continue;
            }
            substreams.push(Ec3Substream {
                fscod: f.fscod,
                bsid: f.bsid,
                asvc: false,
                bsmod: f.bsmod,
                acmod: f.acmod,
                lfeon: f.lfeon,
                num_dep_sub: 0,
                chan_loc: None,
            });
        }
        // §F.6.2.3: the substreamID of the last *independent* substream. §F.6.1
        // derives the substream count from it, so a slice with no independent
        // frame cannot describe a box at all.
        let Some(last_ind) = frames
            .iter()
            .filter(|f| f.strmtyp != EAC3_STRMTYP_DEPENDENT)
            .map(|f| f.substreamid)
            .next_back()
        else {
            return Err(Error::InvalidValue {
                field: "dec3 num_ind_sub",
                value: frames.len() as u64,
                reason: "an E-AC-3 access unit must contain an independent frame",
            });
        };
        // §F.6.1 makes `num_ind_sub` 3 bits and `data_rate` 13: a value that
        // does not fit is a real (if improbable) stream this box cannot
        // describe, so fail rather than wrap.
        let num_ind_sub = broadcast_common::len::fit_bits(last_ind as u64, 3, "num_ind_sub")? as u8;
        let data_rate =
            broadcast_common::len::fit_bits(data_rate as u64, 13, "dec3 data_rate")? as u16;
        Ok(Self {
            data_rate,
            num_ind_sub,
            substreams,
            reserved_tail: Vec::new(),
        })
    }
}

/// The §F.6.1 `chan_loc` bits contributed by one dependent substream, derived
/// from its `chanmap` (§E.1.3.1.8 Table E.1.4) via Table F.6.1.
///
/// Both tables enumerate the same locations in a different order, so the mapping
/// is a per-location bit move rather than a bit permutation: locations that
/// `chanmap` can express but `chan_loc` cannot (`Left`…`LFE`, bits 0-4 and 15 of
/// the chanmap) are the ones the independent substream already carries, and
/// §F.6.2.13 says only the *additional* locations are reflected.
fn chan_loc_for_frame(f: &Ec3SyncframeInfo) -> u16 {
    let Some(chanmap) = f.chanmap else {
        // No custom map: the dependent substream's locations are not signalled
        // beyond acmod/lfeon, which chan_loc has no bits for.
        return 0;
    };
    let mut loc: u16 = 0;
    for (chanmap_bit, loc_bit) in EAC3_CHANMAP_TO_CHAN_LOC {
        // §E.1.3.1.8: "Bit 0, which indicates the presence of the left channel,
        // is stored in the most significant bit of the chanmap field." Table
        // E.1.4 numbers its bits MSB-first, while `chan_loc` (Table F.6.1)
        // numbers from *its* MSB as bit 8. The two tables therefore read in
        // opposite directions and cannot share one shift.
        if chanmap & (1u16 << (EAC3_CHANMAP_BITS - 1 - chanmap_bit)) != 0 {
            loc |= 1 << loc_bit;
        }
    }
    loc
}

/// Width of the `chanmap` field (§E.1.3.1.8): 16 bits, numbered MSB-first.
const EAC3_CHANMAP_BITS: u8 = 16;

/// `(chanmap bit, chan_loc bit)` pairs for the locations both tables share —
/// §E.1.3.1.8 Table E.1.4 (chanmap) and §F.6.2.13 Table F.6.1 (chan_loc).
///
/// `chanmap` declares 16 locations (bit 0 = Left … bit 15 = LFE, counted from the
/// field's MSB) and `chan_loc` 9 (bit 0 = Lc/Rc … bit 8 = LFE2, counted from
/// *that* field's MSB, i.e. bit 0 is the least significant of the nine). Only
/// the locations *beyond* the standard 5.1 set appear in `chan_loc`, which is
/// exactly the set below.
const EAC3_CHANMAP_TO_CHAN_LOC: [(u8, u8); 9] = [
    (5, 0),  // Lc/Rc pair
    (6, 1),  // Lrs/Rrs pair
    (7, 2),  // Cs
    (8, 3),  // Ts
    (9, 4),  // Lsd/Rsd pair
    (10, 5), // Lw/Rw pair
    (11, 6), // Vhl/Vhr pair (chan_loc names it Lvh/Rvh)
    (12, 7), // Vhc (chan_loc names it Cvh)
    (14, 8), // LFE2
];

fn ec3_substream_serialized_len(sub: &Ec3Substream) -> usize {
    // ETSI TS 102 366 §F.6.1: fscod(2)+bsid(5)+reserved(1)+asvc(1)+bsmod(3)+
    // acmod(3)+lfeon(1)+reserved(3)+num_dep_sub(4) = 23 bits, then
    // + chan_loc(9) when num_dep_sub>0 (32 bits = 4 bytes) else
    // + reserved(1) (24 bits = 3 bytes). A dependent substream (7.1 content
    // signalled via chan_loc) is 4 bytes, not always 3 (issue #1055).
    if sub.num_dep_sub > 0 { 4 } else { 3 }
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

        // §F.6.2.14: whatever follows the last substream is reserved; keep it
        // byte-for-byte so a parsed box re-serializes identically.
        let tail_start = (bit_pos.div_ceil(8)).min(bytes.len());
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
        2 + self
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
            return Err(Error::InvalidValue {
                field: "dec3 num_ind_sub",
                value: self.num_ind_sub as u64,
                reason: "substreams.len() must equal num_ind_sub + 1",
            });
        }
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
        // §F.6.2.14 reserved tail, verbatim.
        let tail_start = bit_pos.div_ceil(8);
        buf[tail_start..tail_start + self.reserved_tail.len()].copy_from_slice(&self.reserved_tail);
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// Helpers: bit I/O, scanning, lookup tables
// ---------------------------------------------------------------------------

/// Walk the remainder of E-AC-3 `bsi()` (§E.1.2.2) from just after `bsid`, and
/// return the two fields `dec3` needs: a dependent substream's custom
/// channel map (§E.1.3.1.7/.8) and the informational-metadata bit stream mode
/// (§E.1.3.1.x).
///
/// `bsmod` sits after `infomdate`, which itself follows `mixmdate` and every
/// field it conditionally carries, so there is no way to reach it without
/// decoding the intervening layout. Every field skipped here is length-fixed
/// given `strmtyp`/`acmod`/`lfeon`/`numblks`, so the walk is exact — a truncated
/// frame yields `Err` rather than a silently misread `bsmod`.
///
/// Returns `(chanmap, bsmod)`.
fn parse_eac3_bsi_tail(
    es: &[u8],
    bit_pos: &mut usize,
    strmtyp: u8,
    acmod: u8,
    lfeon: bool,
    numblks: u8,
) -> Result<(Option<u16>, u8)> {
    let dependent = strmtyp == EAC3_STRMTYP_DEPENDENT;

    // dialnorm(5), compre(1) + optional compr(8)
    let _dialnorm = read_bits(es, bit_pos, 5, "dialnorm")?;
    let compre = read_bits(es, bit_pos, 1, "compre")? != 0;
    if compre {
        let _compr = read_bits(es, bit_pos, 8, "compr")?;
    }
    if acmod == 0x0 {
        // 1+1 mode carries a second value for several fields.
        let _dialnorm2 = read_bits(es, bit_pos, 5, "dialnorm2")?;
        let compr2e = read_bits(es, bit_pos, 1, "compr2e")? != 0;
        if compr2e {
            let _compr2 = read_bits(es, bit_pos, 8, "compr2")?;
        }
    }
    // A dependent substream's custom channel map (§E.1.3.1.7).
    let mut chanmap = None;
    if dependent {
        let chanmape = read_bits(es, bit_pos, 1, "chanmape")? != 0;
        if chanmape {
            chanmap = Some(read_bits(es, bit_pos, 16, "chanmap")? as u16);
        }
    }
    // mixmdate block — present in both independent and dependent frames.
    let mixmdate = read_bits(es, bit_pos, 1, "mixmdate")? != 0;
    if mixmdate {
        if acmod > 0x2 {
            let _dmixmod = read_bits(es, bit_pos, 2, "dmixmod")?;
        }
        if (acmod & 0x1) != 0 && acmod > 0x2 {
            let _ltrtcmixlev = read_bits(es, bit_pos, 3, "ltrtcmixlev")?;
            let _lorocmixlev = read_bits(es, bit_pos, 3, "lorocmixlev")?;
        }
        if (acmod & 0x4) != 0 {
            let _ltrtsurmixlev = read_bits(es, bit_pos, 3, "ltrtsurmixlev")?;
            let _lorosurmixlev = read_bits(es, bit_pos, 3, "lorosurmixlev")?;
        }
        if lfeon {
            let lfemixlevcode = read_bits(es, bit_pos, 1, "lfemixlevcode")? != 0;
            if lfemixlevcode {
                let _lfemixlevcod = read_bits(es, bit_pos, 5, "lfemixlevcod")?;
            }
        }
        // §E.1.2.2 gates this block on `strmtyp == 0x0` specifically, not on
        // "not dependent": `strmtyp == 0x2` is an AC-3-derived *independent*
        // stream with no programme-mix block, so `!dependent` would read fields
        // that are not there and desync the rest of the walk.
        if strmtyp == EAC3_STRMTYP_INDEPENDENT {
            let pgmscle = read_bits(es, bit_pos, 1, "pgmscle")? != 0;
            if pgmscle {
                let _pgmscl = read_bits(es, bit_pos, 6, "pgmscl")?;
            }
            if acmod == 0x0 {
                let pgmscl2e = read_bits(es, bit_pos, 1, "pgmscl2e")? != 0;
                if pgmscl2e {
                    let _pgmscl2 = read_bits(es, bit_pos, 6, "pgmscl2")?;
                }
            }
            let extpgmscle = read_bits(es, bit_pos, 1, "extpgmscle")? != 0;
            if extpgmscle {
                let _extpgmscl = read_bits(es, bit_pos, 6, "extpgmscl")?;
            }
            let mixdef = read_bits(es, bit_pos, 2, "mixdef")? as u8;
            match mixdef {
                0x1 => {
                    let _premixcmpsel = read_bits(es, bit_pos, 1, "premixcmpsel")?;
                    let _drcsrc = read_bits(es, bit_pos, 1, "drcsrc")?;
                    let _premixcmpscl = read_bits(es, bit_pos, 3, "premixcmpscl")?;
                }
                0x2 => {
                    let _mixdata = read_bits(es, bit_pos, 12, "mixdata")?;
                }
                0x3 => {
                    let mixdeflen = read_bits(es, bit_pos, 5, "mixdeflen")? as usize;
                    let mixdata2e = read_bits(es, bit_pos, 1, "mixdata2e")? != 0;
                    if mixdata2e {
                        skip_eac3_mixdata2(es, bit_pos)?;
                    }
                    let mixdata3e = read_bits(es, bit_pos, 1, "mixdata3e")? != 0;
                    if mixdata3e {
                        let _spchdat = read_bits(es, bit_pos, 5, "spchdat")?;
                        let addspchdate = read_bits(es, bit_pos, 1, "addspchdate")? != 0;
                        if addspchdate {
                            let _spchdat1 = read_bits(es, bit_pos, 5, "spchdat1")?;
                            let _spchan1att = read_bits(es, bit_pos, 2, "spchan1att")?;
                            let addspchdat1e = read_bits(es, bit_pos, 1, "addspchdat1e")? != 0;
                            if addspchdat1e {
                                let _spchdat2 = read_bits(es, bit_pos, 5, "spchdat2")?;
                                let _spchan2att = read_bits(es, bit_pos, 3, "spchan2att")?;
                            }
                        }
                    }
                    // §E.1.2.2: `mixdata` is `(8 * (mixdeflen + 2))` bits, then
                    // 0..=7 `mixdatafill` bits to the next byte boundary — the
                    // fixed-width part is byte-aligned, so the fill is implicit.
                    skip_eac3_bits(es, bit_pos, 8 * (mixdeflen + 2))?;
                    // `mixdatafill`: "0 - 7" bits — the encoder pads to a byte
                    // boundary, so re-align rather than guessing a count.
                    if !bit_pos.is_multiple_of(8) {
                        *bit_pos += (8 - (*bit_pos).rem_euclid(8)) % 8;
                    }
                }
                _ => {}
            }
            if acmod < 0x2 {
                let paninfoe = read_bits(es, bit_pos, 1, "paninfoe")? != 0;
                if paninfoe {
                    let _panmean = read_bits(es, bit_pos, 8, "panmean")?;
                    let _paninfo = read_bits(es, bit_pos, 6, "paninfo")?;
                }
                if acmod == 0x0 {
                    let paninfo2e = read_bits(es, bit_pos, 1, "paninfo2e")? != 0;
                    if paninfo2e {
                        let _panmean2 = read_bits(es, bit_pos, 8, "panmean2")?;
                        let _paninfo2 = read_bits(es, bit_pos, 6, "paninfo2")?;
                    }
                }
            }
            let frmmixcfginfoe = read_bits(es, bit_pos, 1, "frmmixcfginfoe")? != 0;
            if frmmixcfginfoe {
                if numblks == 1 {
                    // `numblkscod == 0x0` → a single 5-bit `blkmixcfginfo[0]`.
                    let _blkmixcfginfo = read_bits(es, bit_pos, 5, "blkmixcfginfo")?;
                } else {
                    for _ in 0..numblks {
                        let blkmixcfginfoe = read_bits(es, bit_pos, 1, "blkmixcfginfoe")? != 0;
                        if blkmixcfginfoe {
                            let _blkmixcfginfo = read_bits(es, bit_pos, 5, "blkmixcfginfo")?;
                        }
                    }
                }
            }
        }
    }
    // infomdate → bsmod (§E.1.3.1.x). Absent means 0 (§F.6.2.8).
    let infomdate = read_bits(es, bit_pos, 1, "infomdate")? != 0;
    let bsmod = if infomdate {
        read_bits(es, bit_pos, 3, "bsmod")? as u8
    } else {
        0
    };
    Ok((chanmap, bsmod))
}

/// Skip the fixed-width `mixdata2e` sub-block of §E.1.2.2 (mixdef 0x3).
fn skip_eac3_mixdata2(es: &[u8], bit_pos: &mut usize) -> Result<()> {
    let _premixcmpsel = read_bits(es, bit_pos, 1, "premixcmpsel")?;
    let _drcsrc = read_bits(es, bit_pos, 1, "drcsrc")?;
    let _premixcmpscl = read_bits(es, bit_pos, 3, "premixcmpscl")?;
    for what in [
        "extpgmlscle",
        "extpgmcscle",
        "extpgmrscle",
        "extpgmlsscle",
        "extpgmrsscle",
        "extpgmlfescle",
        "dmixscle",
    ] {
        let present = read_bits(es, bit_pos, 1, what)? != 0;
        if present {
            let _value = read_bits(es, bit_pos, 4, what)?;
        }
    }
    let addche = read_bits(es, bit_pos, 1, "addche")? != 0;
    if addche {
        for what in ["extpgmaux1scle", "extpgmaux2scle"] {
            let present = read_bits(es, bit_pos, 1, what)? != 0;
            if present {
                let _value = read_bits(es, bit_pos, 4, what)?;
            }
        }
    }
    Ok(())
}

/// Advance `bit_pos` by `n` bits, erroring if the buffer is too short.
fn skip_eac3_bits(es: &[u8], bit_pos: &mut usize, n: usize) -> Result<()> {
    let end = bit_pos.saturating_add(n);
    if end > es.len() * 8 {
        return Err(Error::BufferTooShort {
            need: end.div_ceil(8),
            have: es.len(),
            what: "E-AC-3 bsi",
        });
    }
    *bit_pos = end;
    Ok(())
}

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
///
/// Bounds are checked here exactly as before; the extraction itself
/// delegates to `broadcast_common::bits::BitReader` (shared with
/// `dvb-t2mi`/`rdd29`/`st291`) so a bit-order/overrun fix there reaches this
/// reader too.
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
    let mut br = broadcast_common::bits::BitReader::new(data);
    br.skip_bits(*bit_pos)
        .expect("bounds already validated above");
    let val = br
        .read_bits(n as u32)
        .expect("bounds already validated above");
    *bit_pos += n;
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
    use alloc::vec;

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

    /// #1055: a substream with `num_dep_sub > 0` is 32 bits (4 bytes) on the
    /// wire, not 24 (3 bytes) — ETSI TS 102 366 §F.6.1 (transcribed at
    /// `docs/codec/ac3-eac3-mp4.md` §F.6.1): the per-substream syntax is
    /// `fscod(2) bsid(5) reserved(1) asvc(1) bsmod(3) acmod(3) lfeon(1)
    /// reserved(3) num_dep_sub(4)` = 23 bits, then `chan_loc(9)` when
    /// `num_dep_sub > 0` (32 bits total) or `reserved(1)` otherwise (24 bits
    /// total).
    ///
    /// No real 7.1 E-AC-3 (dependent-substream) fixture could be produced
    /// locally: ffmpeg's native `eac3` encoder only supports up to 5.1/6
    /// channels (independent substreams only, confirmed via
    /// `ffmpeg -h encoder=eac3` and a failed 8-channel encode attempt), and
    /// no other E-AC-3 encoder is available in this environment. Per the W4
    /// fixture-first rule's fallback, the oracle here is the spec's own
    /// worked bit layout above, hand-encoded independently of this crate's
    /// writer (not derived by calling `serialize_into`).
    const DEC3_DEP_SUB_ORACLE: [u8; 6] = [0x06, 0x00, 0x60, 0x02, 0x02, 0x05];

    #[test]
    fn dec3_dependent_substream_round_trip() {
        // Pre-fix, `ec3_substream_serialized_len` always returned 3, so
        // `serialized_len()` under-counted this 6-byte box as 5 (caught by
        // the assertion below) — and had that under-sized buffer been used
        // as-is, `serialize_into` would write the 9-bit `chan_loc` field
        // past the end of a real (larger, real-world) box, out of bounds.
        // Confirmed by temporarily reverting
        // `ec3_substream_serialized_len` to always return 3: this test then
        // fails at the `serialized_len()` assertion (5 != 6) rather than
        // passing.
        let box1 = Ec3SpecificBox::parse(&DEC3_DEP_SUB_ORACLE).unwrap();
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
        assert_eq!(s0.num_dep_sub, 1, "one dependent substream");
        assert_eq!(s0.chan_loc, Some(0b0_0000_0101), "9-bit chan_loc");

        assert_eq!(
            box1.serialized_len(),
            DEC3_DEP_SUB_ORACLE.len(),
            "serialized_len must count the 4th chan_loc byte for a dependent substream"
        );
        let mut buf = vec![0u8; box1.serialized_len()];
        let n = box1.serialize_into(&mut buf).unwrap();
        assert_eq!(n, DEC3_DEP_SUB_ORACLE.len());
        assert_eq!(
            &buf[..],
            &DEC3_DEP_SUB_ORACLE[..],
            "dec3 dependent-substream round-trip mismatch"
        );
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

    // ── r04-W5: dec3 built from real syncframes ───────────────────────────────

    /// Minimal MSB-first bit writer for building spec-conformant `bsi()` vectors.
    #[derive(Default)]
    struct BitBuf {
        bytes: Vec<u8>,
        bit: usize,
    }

    impl BitBuf {
        fn put(&mut self, n: usize, val: u64) -> &mut Self {
            for i in (0..n).rev() {
                let b = (val >> i) & 1;
                if self.bit.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if b == 1 {
                    let idx = self.bytes.len() - 1;
                    self.bytes[idx] |= 1 << (7 - (self.bit % 8));
                }
                self.bit += 1;
            }
            self
        }
        fn align(&mut self) -> &mut Self {
            while !self.bit.is_multiple_of(8) {
                self.put(1, 0);
            }
            self
        }
        fn take(&mut self) -> Vec<u8> {
            self.bit = 0;
            core::mem::take(&mut self.bytes)
        }
    }

    /// Build an E-AC-3 `bsi()` for a frame, per §E.1.2.2. Only the fields the
    /// crate reads are ever nonzero; `mixmdate` is left 0 so the walk is the
    /// short path (the mixdata variants are exercised by the fixture below).
    #[allow(clippy::too_many_arguments)]
    fn eac3_bsi(
        strmtyp: u8,
        substreamid: u8,
        frmsiz: u16,
        acmod: u8,
        lfeon: bool,
        fscod: u8,
        numblkscod: u8,
        chanmape: Option<u16>,
        bsmod: Option<u8>,
    ) -> Vec<u8> {
        let mut w = BitBuf::default();
        w.put(16, AC3_SYNCWORD as u64);
        w.put(2, strmtyp as u64);
        w.put(3, substreamid as u64);
        w.put(11, frmsiz as u64);
        w.put(2, fscod as u64);
        w.put(2, numblkscod as u64);
        w.put(3, acmod as u64);
        w.put(1, lfeon as u64);
        w.put(5, 16); // bsid = 16 (E-AC-3)
        w.put(5, 31); // dialnorm
        w.put(1, 0); // compre
        if acmod == 0 {
            w.put(5, 31); // dialnorm2
            w.put(1, 0); // compr2e
        }
        if strmtyp == 1 {
            w.put(1, u64::from(chanmape.is_some()));
            if let Some(map) = chanmape {
                w.put(16, map as u64);
            }
        }
        w.put(1, 0); // mixmdate
        w.put(1, u64::from(bsmod.is_some())); // infomdate
        if let Some(m) = bsmod {
            w.put(3, m as u64);
        }
        let _ = numblkscod;
        w.align();
        // Zero-pad to the declared frame size: §E.1.3.1.3 makes `frmsiz` the
        // syncframe length in 16-bit words, and the splitter walks frames by it.
        let mut bytes = w.take();
        let frame_len = (frmsiz as usize + 1) * BYTES_PER_WORD;
        assert!(
            bytes.len() <= frame_len,
            "synthetic bsi ({} bytes) exceeds its declared frame size ({frame_len})",
            bytes.len()
        );
        bytes.resize(frame_len, 0);
        bytes
    }

    /// Format a synthetic E-AC-3 elementary stream for a 7.1 programme: an
    /// independent 5.1 frame followed by a dependent frame whose custom map
    /// carries Lrs/Rrs + Cs (§E.2.8.2). Table E.1.4 bits 6 (Lrs/Rrs) and 7 (Cs)
    /// map to `chan_loc` bits 1 and 2 (§F.6.2.13 Table F.6.1). No real 7.1
    /// E-AC-3 sample exists in the fixtures — the bundled Dolby captures are
    /// 5.1, and ffmpeg's native `eac3` encoder refuses any layout above 5.1 —
    /// so this vector is built directly from the §E.1.2.2 syntax rather than
    /// copied from a capture.
    fn seven_one_eac3_es() -> Vec<u8> {
        let mut es = Vec::new();
        es.extend_from_slice(&eac3_bsi(0, 0, 64, 7, true, 0, 3, None, Some(0)));
        es.extend_from_slice(&eac3_bsi(
            1,
            1,
            64,
            2,
            false,
            0,
            3,
            Some(wire_chanmap(&[6, 7])),
            None,
        ));
        es
    }

    /// r04-W5: `num_ind_sub` is §F.6.2.3's "substreamID of the last independent
    /// substream" (0 here, not the dependent frame's id), the dependent frame is
    /// folded into substream 0's `num_dep_sub`, `chan_loc` is built from its
    /// `chanmap`, and `bsmod` comes from the frame instead of being hardcoded 0.
    /// Unfixed, `num_ind_sub` was the writing frame's own `substreamid` and every
    /// dependent substream was dropped, so a 7.1 stream was signalled as 5.1.
    /// Every syncframe of an elementary stream, in order: the independent frame
    /// plus the dependent frames an access unit carries.
    fn scan_eac3_frames(es: &[u8]) -> Vec<Ec3SyncframeInfo> {
        split_eac3_syncframes(es)
            .iter()
            .flat_map(|au| {
                let mut off = 0usize;
                let mut v = Vec::new();
                while off + 2 <= au.data.len() {
                    match Ec3SyncframeInfo::parse_at(&au.data, off) {
                        Ok(info) => {
                            let len = (info.frmsiz as usize + 1) * BYTES_PER_WORD;
                            v.push(info);
                            off += len;
                        }
                        Err(_) => break,
                    }
                }
                v
            })
            .collect()
    }

    #[test]
    fn dec3_from_syncframes_folds_dependent_substreams() {
        let es = seven_one_eac3_es();
        let frames = scan_eac3_frames(&es);
        assert_eq!(frames.len(), 2, "independent frame + its dependent frame");
        assert_eq!(frames[1].strmtyp, EAC3_STRMTYP_DEPENDENT);
        let bx = Ec3SpecificBox::from_syncframes(&frames).unwrap();
        assert_eq!(bx.num_ind_sub, 0, "last independent substream id");
        assert_eq!(bx.substreams.len(), 1);
        let s0 = bx.substreams[0];
        assert_eq!(s0.num_dep_sub, 1, "the dependent frame's substreamid");
        // chanmap bits 6 (Lrs/Rrs) and 7 (Cs) → chan_loc bits 1 and 2.
        assert_eq!(
            s0.chan_loc,
            Some(0b110),
            "Lrs/Rrs + Cs contribute chan_loc bits 1 and 2"
        );
        // The changed num_dep_sub makes the substream 4 bytes wide, so the box
        // is longer than the 3-byte-per-substream 5.1 form.
        assert_eq!(bx.serialized_len(), 2 + 4);
        let mut buf = vec![0u8; bx.serialized_len()];
        bx.serialize_into(&mut buf).unwrap();
        let back = Ec3SpecificBox::parse(&buf).unwrap();
        assert_eq!(back, bx, "dec3 must round-trip");
    }

    /// r04-W5: a dependent-only (or empty) input describes no substream, so
    /// building a `dec3` from it must be an error. Unfixed, it produced a box
    /// with `num_ind_sub == 0` and no substreams, which the serializer then
    /// rejects — an unserializable value handed back as `Ok`.
    #[test]
    fn dec3_from_dependent_only_input_errors() {
        let es = eac3_bsi(1, 2, 64, 1, false, 0, 3, Some(0x0100), None);
        let frames = scan_eac3_frames(&es);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].strmtyp, EAC3_STRMTYP_DEPENDENT);
        let err = Ec3SpecificBox::from_syncframes(&frames).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "dec3 num_ind_sub",
                    ..
                }
            ),
            "expected InvalidValue for a dependent-only AU, got {err:?}"
        );
        // The empty slice is the same case.
        assert!(Ec3SpecificBox::from_syncframes(&[]).is_err());
        // `into_dec3` on a dependent frame errors too, rather than returning a
        // box that cannot be serialized.
        assert!(frames[0].into_dec3().is_err());
    }

    /// r04-W5 regression: a backlog of **repeated access units** (the shape
    /// `ts_demux` actually feeds) must yield the same `dec3` as one access unit.
    /// Unfixed, `from_syncframes` over a whole backlog pushed one substream per
    /// repeated independent frame (N frames -> N substreams, `data_rate` summed,
    /// `num_ind_sub` from the last), which the serializer rejects.
    #[test]
    fn dec3_from_a_full_backlog_matches_one_access_unit() {
        // Three identical access units, each 1 independent + 1 dependent frame.
        let one_au = seven_one_eac3_es();
        let mut backlog = Vec::new();
        for _ in 0..3 {
            backlog.extend_from_slice(&one_au);
        }
        let expected = Ec3SpecificBox::from_syncframes(&scan_eac3_frames(&one_au)).unwrap();

        // What `ts_demux` now does: the first AU of the first AU-bearing buffer.
        let frames = Ec3SyncframeInfo::from_es_first_au(&backlog);
        assert_eq!(frames.len(), 2, "one AU: independent + dependent frame");
        let bx = Ec3SpecificBox::from_syncframes(&frames).unwrap();
        assert_eq!(bx, expected, "backlog must not inflate the substream list");
        assert_eq!(bx.num_ind_sub, 0);
        assert_eq!(bx.substreams.len(), 1);
        assert_eq!(bx.substreams[0].num_dep_sub, 1);
        // It serializes (the whole point) and round-trips.
        let bytes = bx.try_to_bytes().unwrap();
        assert_eq!(Ec3SpecificBox::parse(&bytes).unwrap(), bx);

        // Feeding all three AUs (the pre-fix behaviour) would not serialize.
        let all = Ec3SyncframeInfo::from_es_all(&backlog);
        assert_eq!(all.len(), 6);
        let bad = Ec3SpecificBox::from_syncframes(&all).unwrap();
        assert_eq!(
            bad.substreams.len(),
            3,
            "one substream per repeated AU frame"
        );
        assert!(
            bad.try_to_bytes().is_err(),
            "a repeated-AU substream list must not serialize as if it were valid"
        );
    }

    /// r04-W5: `bsmod` is read from `infomdate` (§E.1.3.1.x) and lands in the
    /// substream, rather than being written as a hardcoded 0.
    #[test]
    fn dec3_carries_bsmod_from_infomdate() {
        let es = eac3_bsi(0, 0, 64, 7, true, 0, 3, None, Some(7));
        let info = Ec3SyncframeInfo::from_es(&es).unwrap();
        assert_eq!(info.bsmod, 7, "bsmod parsed from bsi()");
        let dec3 = info.into_dec3().unwrap();
        assert_eq!(dec3.substreams[0].bsmod, 7);
        // A frame without infomdate signals bsmod 0 (§F.6.2.8).
        let es0 = eac3_bsi(0, 0, 64, 7, true, 0, 3, None, None);
        assert_eq!(Ec3SyncframeInfo::from_es(&es0).unwrap().bsmod, 0);
    }

    /// The mixdata walk must reach `infomdate` exactly: `mixdef == 1` (fixed
    /// 5 extra bits) and `mixdef == 2` (fixed 12) are position-sensitive, so a
    /// wrong width would land `bsmod` on the wrong bits.
    #[test]
    fn dec3_bsmod_reached_through_mixmdate_variants() {
        for (mixdef, mixdata_bits) in [(1u64, 5usize), (2, 12)] {
            let mut w = BitBuf::default();
            w.put(16, AC3_SYNCWORD as u64);
            w.put(2, 0); // strmtyp independent
            w.put(3, 0); // substreamid
            w.put(11, 64); // frmsiz
            w.put(2, 0); // fscod 48 kHz
            w.put(2, 3); // numblkscod 6 blocks
            w.put(3, 7); // acmod 3/2 (three front channels)
            w.put(1, 1); // lfeon
            w.put(5, 16); // bsid
            w.put(5, 31); // dialnorm
            w.put(1, 0); // compre
            w.put(1, 1); // mixmdate
            w.put(2, 0); // dmixmod
            w.put(3, 0); // ltrtcmixlev
            w.put(3, 0); // lorocmixlev
            w.put(3, 0); // ltrtsurmixlev
            w.put(3, 0); // lorosurmixlev
            w.put(1, 0); // lfemixlevcode
            w.put(1, 0); // pgmscle
            w.put(1, 0); // extpgmscle
            w.put(2, mixdef);
            if mixdef == 1 {
                w.put(1, 0); // premixcmpsel
                w.put(1, 0); // drcsrc
                w.put(3, 0); // premixcmpscl
            } else {
                w.put(mixdata_bits, 0); // mixdata(12)
            }
            w.put(1, 0); // frmmixcfginfoe
            w.put(1, 1); // infomdate
            w.put(3, 5); // bsmod
            w.align();
            let es = w.take();
            let info = Ec3SyncframeInfo::from_es(&es).unwrap();
            assert_eq!(info.bsmod, 5, "mixdef {mixdef}: bsmod landed correctly");
        }
    }

    /// r04-W6: §F.6.2.14's reserved tail must be preserved, so an Atmos
    /// `dec3` (whose `flag_ec3_extension_type_a`/`complexity_index_type_a`
    /// trailer sits in those reserved bytes) survives a parse → serialize
    /// round trip. Unfixed, the tail was dropped and the output was short.
    #[test]
    fn dec3_reserved_tail_is_preserved() {
        // 5.1 oracle body + a 3-byte reserved trailer.
        let mut body = DEC3_ORACLE.to_vec();
        body.extend_from_slice(&[0x01, 0x00, 0x10]);
        let bx = Ec3SpecificBox::parse(&body).unwrap();
        assert_eq!(bx.reserved_tail, vec![0x01, 0x00, 0x10]);
        assert_eq!(bx.serialized_len(), body.len());
        let mut out = vec![0u8; bx.serialized_len()];
        bx.serialize_into(&mut out).unwrap();
        assert_eq!(out, body, "reserved tail must round-trip byte-exactly");
        // A body without a tail still parses to an empty tail.
        assert!(
            Ec3SpecificBox::parse(&DEC3_ORACLE)
                .unwrap()
                .reserved_tail
                .is_empty()
        );
    }

    /// r04-W6: a substream count that disagrees with `num_ind_sub` is rejected
    /// rather than serialized, since §F.6.1 derives the count from the field.
    #[test]
    fn dec3_substream_count_must_match_num_ind_sub() {
        let mut bx = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
        assert_eq!(bx.num_ind_sub, 0);
        bx.substreams.push(bx.substreams[0]);
        let err = bx.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "dec3 num_ind_sub",
                    ..
                }
            ),
            "expected InvalidValue for the count mismatch, got {err:?}"
        );
        // Raising num_ind_sub to match is accepted and round-trips.
        bx.num_ind_sub = 1;
        let bytes = bx.try_to_bytes().unwrap();
        assert_eq!(Ec3SpecificBox::parse(&bytes).unwrap(), bx);
    }

    /// A `chanmap` value written the way §E.1.3.1.8 describes it: `bits` are
    /// Table E.1.4 bit numbers, MSB-first, so Table E.1.4 bit 0 is the value's
    /// most significant bit.
    fn wire_chanmap(bits: &[u8]) -> u16 {
        let mut v: u16 = 0;
        for &b in bits {
            assert!(b < 16, "Table E.1.4 only defines bits 0..=15");
            v |= 1u16 << (15 - b);
        }
        v
    }

    /// §E.1.2.2 gates the programme-mix block on `strmtyp == 0x0`, not on "not
    /// dependent": `strmtyp == 0x2` is an AC-3-derived *independent* stream with
    /// no programme-mix block. Unfixed (`!dependent`), a strmtyp-2 frame read
    /// `pgmscle`/`mixdef` out of fields that are not there and desynced, so
    /// `bsmod` came from the wrong bits.
    #[test]
    fn bsmod_is_reached_for_a_strmtyp_two_frame() {
        // A strmtyp-2 frame with infomdate and a distinctive bsmod, built with
        // no programme-mix block (the independent-without-AC3 layout).
        let mut w = BitBuf::default();
        w.put(16, AC3_SYNCWORD as u64);
        w.put(2, 2); // strmtyp = 2 (converted from AC-3)
        w.put(3, 0); // substreamid
        w.put(11, 64); // frmsiz
        w.put(2, 0); // fscod 48 kHz
        w.put(2, 3); // numblkscod 6
        w.put(3, 2); // acmod 2/0
        w.put(1, 0); // lfeon
        w.put(5, 16); // bsid
        w.put(5, 31); // dialnorm
        w.put(1, 0); // compre
        // strmtyp != 1, so no chanmape/chanmap.
        // mixmdate = 1, exercising the prefix that *is* present for strmtyp 2
        // (dmixmod/mixlevs/lfemixlevcode) — then, because strmtyp != 0x0, no
        // programme-mix block at all: infomdate follows immediately after that
        // prefix. A `!dependent` gate would go looking for pgmscle/mixdef here.
        w.put(1, 1); // mixmdate
        w.put(1, 1); // acmod -> only one front channel means no mixlevs
        w.put(3, 6); // bsmod
        w.align();
        let mut es = w.take();
        es.resize(130, 0);

        let info = Ec3SyncframeInfo::from_es(&es).unwrap();
        assert_eq!(info.strmtyp, 2);
        assert_eq!(
            info.bsmod, 6,
            "strmtyp-2 frame has no programme-mix block, so bsmod follows mixmdate directly"
        );
        // A strmtyp-0 frame with the same trailing bytes still reads its
        // programme-mix block and lands on the same bsmod.
        assert_eq!(
            Ec3SyncframeInfo::from_es(&eac3_bsi(0, 0, 64, 2, false, 0, 3, None, Some(6)))
                .unwrap()
                .bsmod,
            6
        );
    }

    /// §E.1.2.2: `mixdata2e` and `mixdata3e` are *siblings* of `mixdeflen`, and
    /// the `mixdata` payload that follows the whole option-4 block is
    /// `8 * (mixdeflen + 2)` bits plus 0..=7 fill bits. A reader that nested the
    /// flags inside the `mixdeflen` field — or that read the fill as a count —
    /// would land `infomdate` on the wrong bit.
    #[test]
    fn mixdef_option_four_walks_to_the_right_bit() {
        for (mixdata2e, mixdata3e, addspchdate, addspchdat1e) in [
            (false, false, false, false),
            (true, false, false, false),
            (true, true, true, true),
            (false, true, false, false),
        ] {
            for mixdeflen in [0u64, 1, 7] {
                let mut w = BitBuf::default();
                w.put(16, AC3_SYNCWORD as u64);
                w.put(2, 0); // strmtyp independent
                w.put(3, 0);
                w.put(11, 64);
                w.put(2, 0); // fscod
                w.put(2, 3); // numblkscod
                w.put(3, 7); // acmod 3/2 (three front + surrounds)
                w.put(1, 1); // lfeon
                w.put(5, 16); // bsid
                w.put(5, 31); // dialnorm
                w.put(1, 0); // compre
                w.put(1, 1); // mixmdate
                w.put(2, 0); // dmixmod
                w.put(3, 0); // ltrtcmixlev
                w.put(3, 0); // lorocmixlev
                w.put(3, 0); // ltrtsurmixlev
                w.put(3, 0); // lorosurmixlev
                w.put(1, 0); // lfemixlevcode
                w.put(1, 0); // pgmscle
                w.put(1, 0); // extpgmscle
                w.put(2, 3); // mixdef = 0x3 (option 4)
                w.put(5, mixdeflen);
                w.put(1, u64::from(mixdata2e));
                if mixdata2e {
                    w.put(1, 0); // premixcmpsel
                    w.put(1, 0); // drcsrc
                    w.put(3, 0); // premixcmpscl
                    for _ in 0..7 {
                        w.put(1, 0); // the seven *scle flags
                    }
                    w.put(1, 0); // addche
                }
                w.put(1, u64::from(mixdata3e));
                if mixdata3e {
                    w.put(5, 0); // spchdat
                    w.put(1, u64::from(addspchdate));
                    if addspchdate {
                        w.put(5, 0); // spchdat1
                        w.put(2, 0); // spchan1att
                        w.put(1, u64::from(addspchdat1e));
                        if addspchdat1e {
                            w.put(5, 0); // spchdat2
                            w.put(3, 0); // spchan2att
                        }
                    }
                }
                // mixdata: 8 * (mixdeflen + 2) bits, then implicit byte fill.
                for _ in 0..8 * (mixdeflen + 2) {
                    w.put(1, 0);
                }
                w.align(); // mixdatafill
                w.put(1, 0); // frmmixcfginfoe
                w.put(1, 1); // infomdate
                w.put(3, 5); // bsmod
                w.align();
                let mut es = w.take();
                es.resize(130, 0);

                let info = Ec3SyncframeInfo::from_es(&es).unwrap();
                assert_eq!(
                    info.bsmod, 5,
                    "mixdeflen {mixdeflen}, mixdata2e {mixdata2e}, mixdata3e {mixdata3e}:                      bsmod landed on the wrong bit"
                );
            }
        }
    }

    /// The chanmap→chan_loc table must move every location §F.6.1 names, and
    /// leave the standard-5.1 locations out (the independent substream already
    /// carries those).
    #[test]
    fn chanmap_to_chan_loc_covers_the_spec_locations() {
        // All 16 chanmap bits set → every chan_loc location that chanmap can
        // express is present.
        let info = Ec3SyncframeInfo {
            strmtyp: EAC3_STRMTYP_DEPENDENT,
            substreamid: 1,
            frmsiz: 1,
            fscod: 0,
            numblks: 6,
            acmod: 2,
            lfeon: false,
            bsid: 16,
            sample_rate: 48000,
            sample_rate_khz: 48,
            chanmap: Some(0xFFFF),
            bsmod: 0,
        };
        assert_eq!(chan_loc_for_frame(&info), 0b1_1111_1111);
        // Table E.1.4 numbers its bits MSB-first, so on the wire the standard
        // 5.1 locations are bits 0-4 and 15 — which is the *low* end and the
        // *top* of the 16-bit value when it is read MSB-first as the spec
        // describes. Written as a wire-order bit string and converted with the
        // same `15 - bit` shift the code uses, they contribute no `chan_loc`.
        let base = Ec3SyncframeInfo {
            chanmap: Some(wire_chanmap(&[0, 1, 2, 3, 4, 15])),
            ..info
        };
        assert_eq!(
            chan_loc_for_frame(&base),
            0,
            "5.1 locations are not chan_loc"
        );
        // A single extra location, expressed in wire order: Table E.1.4 bit 6
        // (Lrs/Rrs pair) is the 7th bit read, i.e. `1 << (15 - 6)` in the value,
        // and maps to Table F.6.1 `chan_loc` bit 1.
        let lrs_rrs = Ec3SyncframeInfo {
            chanmap: Some(wire_chanmap(&[0, 1, 2, 3, 4, 6, 15])),
            ..info
        };
        assert_eq!(
            chan_loc_for_frame(&lrs_rrs),
            0b10,
            "E.1.4 bit 6 -> F.6.1 bit 1"
        );
        // And the two ends: E.1.4 bit 5 (Lc/Rc) -> chan_loc bit 0; bit 14
        // (LFE2) -> chan_loc bit 8.
        let ends = Ec3SyncframeInfo {
            chanmap: Some(wire_chanmap(&[5, 14])),
            ..info
        };
        assert_eq!(chan_loc_for_frame(&ends), 0b1_0000_0001);
        // A frame with no custom map contributes nothing.
        assert_eq!(
            chan_loc_for_frame(&Ec3SyncframeInfo {
                chanmap: None,
                ..info
            }),
            0
        );
    }
}
