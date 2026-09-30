//! MPEG-2 Program Stream demuxer → hub [`Media`] IR.
//!
//! `PsDemux` is an **input** side of the any-to-any container hub: it consumes a
//! raw MPEG-1/2 Program Stream (`.mpg` / `.vob`) and produces the neutral
//! [`Media`] IR (one [`Track`] per elementary stream, coded samples in decode
//! order), implementing the abstract [`broadcast_common::Unpackage`] trait so
//! `{PS} → IR → {any}` composes with the existing
//! [`CmafMux`](crate::media::CmafMux) / [`HlsPackager`](crate::media::HlsPackager)
//! packagers — mirroring [`TsDemux`](crate::TsDemux).
//!
//! Pipeline: PS pack layer ([`mpeg_ps`]) → per-`stream_id` PES reassembly
//! ([`mpeg_pes`]) → codec-config recovery (H.264 in-band SPS/PPS → `avcC`, AC-3
//! syncframe BSI → `dac3`) → length-prefixed video / raw audio samples.
//!
//! Unlike a Transport Stream, a Program Stream carries no PMT here: the fixture
//! has no [`ProgramStreamMap`](mpeg_ps::ProgramStreamMap), so elementary streams
//! are mapped by `stream_id` (ISO/IEC 13818-1 Table 2-22): video from the video
//! range 0xE0–0xEF, audio from `private_stream_1` (0xBD). Unknown `stream_id`s
//! are skipped, never fatal.
//!
//! A `private_stream_1` `stream_id` is a *container*: it multiplexes several
//! independent substreams (AC-3, DTS, LPCM, subpictures), told apart by the
//! `substream_id` byte at the front of each PES payload's 4-byte substream
//! header. Elementary streams are therefore keyed by
//! `(stream_id, substream_id)`. A substream is carried only when its own bytes
//! identify it as AC-3: the syncword is looked for at the header's
//! `first_access_unit_pointer` (so a packet that resumes a frame an earlier
//! packet opened is not mistaken for a non-audio stream), and the DTS
//! (`0x88..=0x8F`), LPCM (`0xA0..=0xA7`) and subpicture (`0x20..=0x3F`)
//! substream ids are skipped explicitly rather than concatenated into the AC-3
//! track beside them.
//!
//! A PS also does not stamp every frame: one PES packet concatenates several
//! access units and carries a PTS/DTS only for the first one. Video access units
//! are therefore recovered from the reassembled Annex B byte stream by
//! [`crate::au::AccessUnitSplitter`], which decides a boundary from
//! `first_mb_in_slice` as well as from an access-unit delimiter — an AUD is
//! *optional* in H.264 and absent from most PS captures. Audio is split into
//! AC-3 syncframes by [`crate::ac3::split_ac3_syncframes`], which walks each
//! frame's declared length rather than scanning for the next syncword. Timing is
//! anchored off the PES-level PTS/DTS that are present and the constant frame
//! duration derived from the stamped decode timestamps.
//!
//! HEVC / DTS / other codecs are not carried here (skipped, never fatal).
//!
//! # Spec
//!
//! - **Program Stream framing (pack header / system header / PSM)**: ISO/IEC
//!   13818-1 (ITU-T H.222.0) §2.5 — via [`mpeg_ps`].
//! - **PES reassembly + PTS/DTS**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7 (via
//!   [`mpeg_pes`], 33-bit @ 90 kHz).
//! - **`stream_id` assignment**: ISO/IEC 13818-1 Table 2-22.
//! - **AC-3 in `private_stream_1`**: ETSI TS 101 154 — the 4-byte substream
//!   header (`substream_id` + `number_of_frames` + `first_access_unit_pointer`)
//!   precedes the AC-3 syncframes.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::Unpackage;
use mpeg_ps::program_stream::parse_all_packs;

use crate::ac3::{AC3_SAMPLES_PER_SYNCFRAME, Ac3SyncframeInfo};
use crate::annexb::iter_annexb_nals;
use crate::au::AccessUnitSplitter;
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::mp4esds::{DecoderConfigDescriptor, ESDescriptor, EsdsBox, SLConfigDescriptor};
use crate::mpeg_legacy::Mpeg2SeqHeader;
use crate::nal::NalCodec;
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::ts_demux::{
    ESDS_VIDEO_ES_ID, MPEG2_PICTURE_START_CODE, OTI_MPEG2_VIDEO_MAIN, STREAM_TYPE_VISUAL,
    mpeg2_is_sync,
};

// ── stream_id → codec (ISO/IEC 13818-1 Table 2-22) ──────────────────────────

/// Low bound of the H.264/video `stream_id` range (`1110 xxxx`, 0xE0–0xEF).
const STREAM_ID_VIDEO_LO: u8 = 0xE0;
/// High bound of the video `stream_id` range.
const STREAM_ID_VIDEO_HI: u8 = 0xEF;
/// `private_stream_1` `stream_id` — carries user-private payloads, in practice
/// AC-3/E-AC-3/DTS/LPCM audio and subpictures (Table 2-22; H.222.0 §2.4.3.7
/// makes its `PES_packet_data_byte`s "user definable").
const STREAM_ID_PRIVATE_1: u8 = 0xBD;

// ── private_stream_1 substream header ────────────────────────────────────────

/// Length of the `private_stream_1` substream header before the substream's own
/// payload: `substream_id`(8) + `number_of_frames`(8) +
/// `first_access_unit_pointer`(16) — the `private_stream_1`-specific
/// `PES_packet_data_byte` layout (H.222.0 §2.4.3.7 leaves its *contents* to the
/// application, but this 4-byte prologue is what the DVD/ATSC private-stream
/// profiles this demuxer targets put there).
const PRIVATE1_HEADER_LEN: usize = 4;
/// Offset of the `substream_id` byte within that header.
const PRIVATE1_SUBSTREAM_ID_OFFSET: usize = 0;
/// Offset of `number_of_frames` within the substream header: how many complete
/// access units start in this PES packet.
const PRIVATE1_NUM_FRAMES_OFFSET: usize = 1;
/// Offset of `first_access_unit_pointer` within the substream header.
const PRIVATE1_FIRST_ACCESS_UNIT_POINTER_OFFSET: usize = 2;
/// Bytes of `number_of_frames`.
const PRIVATE1_NUM_FRAMES_LEN: usize = 1;
/// Bytes of `first_access_unit_pointer` (big-endian).
const PRIVATE1_FIRST_ACCESS_UNIT_POINTER_LEN: usize = 2;

/// The substream header's fields must tile its 4 bytes with nothing left over:
/// `substream_id`(1) at [`PRIVATE1_SUBSTREAM_ID_OFFSET`], then
/// `number_of_frames`(1) at [`PRIVATE1_NUM_FRAMES_OFFSET`], then
/// `first_access_unit_pointer`(2). A mismatch would read one field out of
/// another's bytes. Checked at compile time (`debug_assert!` would be skipped
/// by release builds).
const _: () = assert!(
    PRIVATE1_SUBSTREAM_ID_OFFSET == 0
        && PRIVATE1_NUM_FRAMES_OFFSET == PRIVATE1_SUBSTREAM_ID_OFFSET + 1
        && PRIVATE1_NUM_FRAMES_LEN == 1
        && PRIVATE1_FIRST_ACCESS_UNIT_POINTER_OFFSET
            == PRIVATE1_NUM_FRAMES_OFFSET + PRIVATE1_NUM_FRAMES_LEN
        && PRIVATE1_FIRST_ACCESS_UNIT_POINTER_LEN == 2
        && PRIVATE1_HEADER_LEN == 4
);

// ── private_stream_1 substream_id ranges (DVD/ATSC private-stream profiles) ───
//
// H.222.0 §2.4.3.7 declares a `private_stream_1` PES's data bytes "user
// definable" and specifies nothing about `substream_id`; the assignments below
// are the de-facto DVD-Video / ATSC A/52 streaming convention that ffmpeg's
// `mpeg` demuxer and libavformat follow. They are used only to *skip*
// substreams this demuxer does not carry (with a comment naming what they are)
// — never to assume a substream's codec, which is still probed from its bytes.

/// DTS substream ids (DVD-Video convention): `0x88..=0x8F`. Not carried.
const SUBSTREAM_ID_DTS_RANGE: core::ops::RangeInclusive<u8> = 0x88..=0x8F;
/// LPCM substream ids (DVD-Video convention): `0xA0..=0xA7`. Not carried.
const SUBSTREAM_ID_LPCM_RANGE: core::ops::RangeInclusive<u8> = 0xA0..=0xA7;
/// Subpicture (bitmap subtitle) substream ids (DVD-Video convention):
/// `0x20..=0x3F`. Not carried here (a subtitle track needs its own sample
/// entry, which this demuxer does not build).
const SUBSTREAM_ID_SUBPICTURE_RANGE: core::ops::RangeInclusive<u8> = 0x20..=0x3F;

/// `substream_id` recorded for a stream that is not a `private_stream_1`
/// substream container, so every `(stream_id, _)` pair still has a complete key.
const PRIVATE1_SUBSTREAM_ID_NONE: u8 = 0;

/// AC-3 syncword (`0x0B77`, ETSI TS 102 366 §4.1 "syncword"; "0x0B77" in
/// Annex B's `syncinfo()`) — the first two bytes of every AC-3 syncframe, and
/// the evidence [`Codec::classify_private1`] requires of an audio substream.
const AC3_SYNCWORD: [u8; 2] = [0x0B, 0x77];
/// Length of [`AC3_SYNCWORD`].
const AC3_SYNCWORD_LEN: usize = AC3_SYNCWORD.len();

/// Lowest `substream_id` in the audio block (`0x80..=0xBF`). A substream below
/// it — a subpicture, or the unassigned `0x00..=0x1F` block — is not audio and
/// is skipped without probing.
const SUBSTREAM_ID_AUDIO_LO: u8 = 0x80;

// ── H.264 NAL / config constants (ISO/IEC 14496-10 / 14496-15) ───────────────

/// NAL length-field width for `mdat` samples: 4-byte prefixes → `lengthSizeMinusOne = 3`.
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// H.264 `nal_unit_type` for SPS (Table 7-1).
const H264_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for PPS (Table 7-1).
const H264_NAL_PPS: u8 = 8;
/// H.264 `nal_unit_type` for a coded slice of an IDR picture (Table 7-1).
const H264_NAL_IDR: u8 = 5;
/// Mask for the H.264 5-bit `nal_unit_type` in the NAL header byte.
const H264_NAL_TYPE_MASK: u8 = 0x1F;

// ── Timestamps / timescale ───────────────────────────────────────────────────

/// Video media timescale (90 kHz — the PS/PES timestamp clock).
const VIDEO_TIMESCALE: u32 = 90_000;
/// Audio sample size in bits carried in the sample entry (PCM-equivalent; 16).
const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;
/// 33-bit PTS/DTS modulus, for wrap-around unrolling (§2.4.3.7, 90 kHz clock).
const TS_WRAP: i128 = 1 << 33;
/// Half the 33-bit range — the threshold used to detect a backward wrap.
const TS_WRAP_HALF: i128 = TS_WRAP / 2;
/// Fallback per-frame duration (90 kHz ticks) when only one stamped frame exists.
const DEFAULT_FRAME_DURATION: i128 = 3600;

/// Codec class recovered from a `stream_id`. Data-carrying dispatch discriminant,
/// not a spec label enum — hence no `name()`/`Display` (see the
/// `tests/label_coverage.rs` policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    /// A `stream_id` in the video range (Table 2-22, 0xE0-0xEF). This range
    /// carries H.264 *or* MPEG-2 video (or other video codecs this demuxer
    /// doesn't support) indistinguishably by `stream_id` alone — the actual
    /// codec is decided by probing the reassembled ES (C6, #1009).
    Video,
    /// A `private_stream_1` (0xBD) PES: a container for several substreams,
    /// each identified by the first byte of its substream header. Resolved
    /// per substream by [`Codec::classify_private1`] into [`Codec::Ac3`] or
    /// [`Codec::Skipped`].
    Private,
    Ac3,
    /// A substream this demuxer does not carry (DTS, LPCM, a subpicture, or a
    /// private substream whose payload is not AC-3). Skipped, never fatal —
    /// it produces no track and accumulates no bytes.
    Skipped,
}

impl Codec {
    /// Map a PES `stream_id` to a supported [`Codec`] *without* inspecting the
    /// payload, or `None` for a `stream_id` this demuxer never carries.
    ///
    /// `private_stream_1` maps to [`Codec::Private`] rather than to AC-3: its
    /// substream is only known once the substream header has been read and its
    /// payload probed (see [`Codec::classify_private1`]).
    fn from_stream_id(stream_id: u8) -> Option<Self> {
        match stream_id {
            STREAM_ID_VIDEO_LO..=STREAM_ID_VIDEO_HI => Some(Codec::Video),
            STREAM_ID_PRIVATE_1 => Some(Codec::Private),
            _ => None,
        }
    }

    /// Classify one `private_stream_1` substream from its header and the payload
    /// that follows it.
    ///
    /// Returns the codec to carry the substream as, or `None` to skip it.
    ///
    /// The syncword probe runs at the header's `first_access_unit_pointer`, not
    /// at the start of the payload (r04-W19 review). The pointer is the offset,
    /// from the byte after the pointer field, of the first access unit that
    /// *begins* in this packet — so a packet that merely continues a frame an
    /// earlier packet opened has the syncword somewhere later, or not at all
    /// (pointer 0). Probing at the payload start therefore dropped any substream
    /// whose first surviving packet begins mid-frame, which is routine. A
    /// pointer of 0 means no access unit starts here, so there is no syncword to
    /// find and the substream is classified from what the pointer points at only
    /// when it is non-zero — with nothing to point at, the payload is left for
    /// accumulation and classified on a later packet.
    ///
    /// A substream with no audio `substream_id` is skipped by identity: the
    /// DVD/ATSC convention puts subpictures at `0x20..=0x3F`, DTS at
    /// `0x88..=0x8F` and LPCM at `0xA0..=0xA7`, none of which this demuxer
    /// carries. Those are named ranges rather than a silent fall-through so the
    /// intent is visible, and the AC-3 range is still *probed* rather than
    /// assumed.
    fn classify_private1(
        substream_id: u8,
        header: &Private1Header,
        payload: &[u8],
    ) -> Option<Self> {
        if SUBSTREAM_ID_SUBPICTURE_RANGE.contains(&substream_id)
            || SUBSTREAM_ID_DTS_RANGE.contains(&substream_id)
            || SUBSTREAM_ID_LPCM_RANGE.contains(&substream_id)
        {
            return None;
        }
        if substream_id < SUBSTREAM_ID_AUDIO_LO {
            return None;
        }
        // The pointer is 1-based over the bytes that follow the pointer field:
        // 1 addresses the first of them, so the access unit's first byte is at
        // `pointer - 1`. A pointer of 0 means "no access unit starts in this
        // packet" — which is what a muxer that splits purely by its own frame
        // boundaries writes, so it is a normal variant rather than a corrupt
        // one. Fall back to probing the payload's own start there: the first
        // packet of such a stream carries a frame from its very first byte.
        let pointer = usize::from(header.first_access_unit_pointer);
        let offset = pointer.saturating_sub(1);
        let probe = payload.get(offset..)?;
        if probe.len() < AC3_SYNCWORD_LEN || !probe.starts_with(&AC3_SYNCWORD) {
            return None;
        }
        Some(Codec::Ac3)
    }
}

/// The 4-byte `private_stream_1` substream header that precedes each substream's
/// own bytes ([`PRIVATE1_HEADER_LEN`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Private1Header {
    /// `substream_id` — distinguishes the substreams sharing one 0xBD
    /// `stream_id`.
    substream_id: u8,
    /// `number_of_frames` — how many complete access units start in this PES
    /// packet. Unused beyond validation: this demuxer splits frames by their own
    /// length ([`crate::ac3::split_ac3_syncframes`]) rather than by the count,
    /// but a header claiming frames in a payload too short to hold them is
    /// malformed.
    number_of_frames: u8,
    /// `first_access_unit_pointer` — byte offset, from the byte after this
    /// field, of the first access unit that begins in this packet; 0 when none
    /// does.
    first_access_unit_pointer: u16,
}

impl Private1Header {
    /// Parse the 4-byte substream header at the front of a `private_stream_1`
    /// PES payload, returning it and the substream's own bytes.
    ///
    /// `None` when the payload is too short to hold the header.
    fn parse(payload: &[u8]) -> Option<(Self, &[u8])> {
        if payload.len() < PRIVATE1_HEADER_LEN {
            return None;
        }
        let ptr = PRIVATE1_FIRST_ACCESS_UNIT_POINTER_OFFSET;
        let header = Private1Header {
            substream_id: payload[PRIVATE1_SUBSTREAM_ID_OFFSET],
            number_of_frames: payload[PRIVATE1_NUM_FRAMES_OFFSET],
            first_access_unit_pointer: u16::from_be_bytes([payload[ptr], payload[ptr + 1]]),
        };
        Some((header, &payload[PRIVATE1_HEADER_LEN..]))
    }

    /// True if the payload can hold the access units this header claims: a
    /// header that says frames start here must point at one.
    ///
    /// The pointer is 1-based over the body and may address one past its last
    /// byte (an access unit that begins at the very end of the packet, with no
    /// bytes of its own in it), so the valid non-zero range is `1..=body_len + 1`.
    /// Only a pointer beyond that names a position the packet does not carry.
    ///
    /// This is consulted only while a substream is **undecided** — it says
    /// whether there is anything in this packet to identify the substream by.
    /// Once the substream is known, every byte it carries is frame data and is
    /// kept regardless, so a muxer writing `pointer = 0` on every packet (a
    /// normal variant: the pointer only marks where an access unit *starts*)
    /// loses nothing.
    fn is_consistent(&self, payload_len: usize) -> bool {
        let body_len = payload_len.saturating_sub(PRIVATE1_HEADER_LEN);
        let pointer = usize::from(self.first_access_unit_pointer);
        if self.number_of_frames > 0 && pointer == 0 {
            return false;
        }
        if pointer > body_len.saturating_add(1) {
            return false;
        }
        true
    }
}

/// Identifies one elementary stream inside the program stream.
///
/// `stream_id` alone is not unique for a program stream: `private_stream_1`
/// (0xBD) is a container whose payloads belong to independent *substreams*,
/// told apart by the `substream_id` byte at the front of each PES payload's
/// substream header. Keying the ES map by both keeps them separate (r04-W19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct StreamKey {
    stream_id: u8,
    /// The `private_stream_1` `substream_id`; meaningless (and
    /// [`PRIVATE1_SUBSTREAM_ID_NONE`]) for a `stream_id` that is not a
    /// substream container.
    substream_id: u8,
}

/// One elementary stream discovered by [`StreamKey`], its reassembled elementary
/// byte stream, and the PES-level timestamps captured at each fragment boundary.
struct ElementaryStream {
    codec: Codec,
    /// Concatenated elementary bytes across every PES fragment (Annex B for
    /// video; raw AC-3 syncframes for audio, substream header already stripped).
    es_bytes: Vec<u8>,
    /// PES-level `(byte_offset, pts, dts)` captured at the start of each PES
    /// fragment that carried a PTS. `byte_offset` is the offset into `es_bytes`
    /// at which that fragment's payload begins.
    stamps: Vec<Stamp>,
}

/// A PES-level timestamp anchored at a byte offset within the reassembled ES.
#[derive(Debug, Clone, Copy)]
struct Stamp {
    offset: usize,
    pts: Option<u64>,
    dts: Option<u64>,
}

/// A single recovered access unit with its (optional) presentation/decode
/// timestamps. Video AUs are Annex B; audio "AUs" are raw AC-3 frames.
struct AccessUnit {
    data: Vec<u8>,
    pts: Option<u64>,
    dts: Option<u64>,
}

/// Demux an MPEG-1/2 Program Stream byte slice into a [`Media`].
///
/// Walks the packs, reassembles per-`stream_id` PES into elementary byte
/// streams, splits them into access units (video: Annex B AUD boundaries; audio:
/// AC-3 syncframes), recovers codec config from the in-band headers, and emits
/// length-prefixed video / raw audio samples in decode order.
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes via
/// [`Unpackage::Input`]; construct one per call with [`PsDemux::new`].
#[derive(Debug, Default, Clone)]
pub struct PsDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> PsDemux<'a> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    /// Demux `input` (a whole MPEG-1/2 Program Stream) into a [`Media`].
    ///
    /// This is the inherent form of [`Unpackage::unpackage`]; both produce the
    /// same result. See the type-level docs for the pipeline.
    pub fn demux(&mut self, input: &'a [u8]) -> Result<Media> {
        let (packs, _trailing) = parse_all_packs(input).map_err(Error::Ps)?;

        // ── Pass 1: per-elementary-stream PES reassembly ──────────────────
        // A stream is keyed by `(stream_id, substream_id)`: a
        // `private_stream_1` (0xBD) PES multiplexes several independent
        // substreams — AC-3, DTS, LPCM, subpictures — distinguished by the
        // first byte of its substream header, so keying by `stream_id` alone
        // would concatenate them into one incoherent "AC-3" track (r04-W19).
        // Insertion order is preserved so tracks come out in the order the
        // streams first appear (video before audio, as stored).
        let mut order: Vec<StreamKey> = Vec::new();
        let mut streams: BTreeMap<StreamKey, ElementaryStream> = BTreeMap::new();

        for pack in &packs {
            for pes in &pack.pes_packets {
                let sid = pes.stream_id.0;
                let Some(codec) = Codec::from_stream_id(sid) else {
                    continue;
                };
                // A video payload is the Annex B bytes verbatim; a
                // private_stream_1 payload needs its substream header read and
                // stripped before the substream's own bytes are known.
                let (substream_id, header, payload): (u8, Option<Private1Header>, &[u8]) =
                    match codec {
                        Codec::Video => (PRIVATE1_SUBSTREAM_ID_NONE, None, pes.payload),
                        Codec::Private => {
                            let Some((header, body)) = Private1Header::parse(pes.payload) else {
                                continue;
                            };
                            (header.substream_id, Some(header), body)
                        }
                        // `from_stream_id` never returns these: AC-3 is only
                        // reached via `Codec::Private`, and `Skipped` is only
                        // assigned inside the loop below.
                        Codec::Ac3 | Codec::Skipped => continue,
                    };
                if payload.is_empty() {
                    continue;
                }
                let (pts, dts) = pes
                    .header
                    .as_ref()
                    .map(|h| (h.pts.map(|p| p.0), h.dts.map(|d| d.0)))
                    .unwrap_or((None, None));

                let key = StreamKey {
                    stream_id: sid,
                    substream_id,
                };
                let es = streams.entry(key).or_insert_with(|| {
                    order.push(key);
                    ElementaryStream {
                        codec,
                        es_bytes: Vec::new(),
                        stamps: Vec::new(),
                    }
                });
                if es.codec == Codec::Private {
                    // Still undecided: decide it from this packet's substream
                    // header. The classification probes the syncword at the
                    // header's `first_access_unit_pointer`, so a packet that
                    // merely continues a frame (`pointer == 0`, or a tail that
                    // does not begin with a syncword) leaves the substream
                    // undecided and the bytes are *not* accumulated — the
                    // packets before the first real access-unit start carry no
                    // frame this demuxer could emit anyway, and dropping them
                    // keeps the ES starting on a syncframe.
                    if let Some(c) =
                        header.and_then(|h| Codec::classify_private1(substream_id, &h, payload))
                    {
                        es.codec = c;
                    } else {
                        // A header naming an access unit the payload does not
                        // carry, or claiming frames while pointing nowhere, is
                        // malformed *for classification purposes* — there is
                        // nothing here to identify the substream by. Skipping
                        // the packet keeps a partial frame tail from being
                        // spliced onto an ES that has not started yet.
                        //
                        // This check applies only while the substream is
                        // undecided. Once it is known to be AC-3, every byte it
                        // carries is frame data and must be kept: the pointer
                        // only says where the next access unit starts, and a
                        // bogus or zero one on a mid-stream packet is no reason
                        // to discard real audio (a muxer writing pointer 0
                        // everywhere is a normal variant, not a corrupt file).
                        if !header.is_some_and(|h| h.is_consistent(pes.payload.len())) {
                            continue;
                        }
                    }
                }
                if es.codec == Codec::Private || es.codec == Codec::Skipped {
                    continue;
                }
                // Drop the leading bytes that precede the first access unit in
                // this packet: a packet may open mid-frame (its
                // `first_access_unit_pointer` naming where the next frame
                // starts), and those prefix bytes are the tail of a frame the
                // demuxer never saw the start of. Accumulating them would leave
                // the reassembled ES beginning mid-frame, which no syncword
                // scan can recover from — the AC-3 splitter stops at the first
                // non-syncword byte.
                let payload = match header {
                    Some(header)
                        if es.es_bytes.is_empty() && header.is_consistent(pes.payload.len()) =>
                    {
                        let prefix = usize::from(header.first_access_unit_pointer)
                            .saturating_sub(1)
                            .min(payload.len());
                        &payload[prefix..]
                    }
                    _ => payload,
                };
                if payload.is_empty() {
                    continue;
                }
                let offset = es.es_bytes.len();
                if pts.is_some() || dts.is_some() {
                    es.stamps.push(Stamp { offset, pts, dts });
                }
                es.es_bytes.extend_from_slice(payload);
            }
        }

        // ── Pass 2: build one track per elementary stream, in first-seen order ──
        let mut tracks: Vec<Track> = Vec::new();
        let mut track_id: u32 = 1;
        for key in &order {
            let es = &streams[key];
            let built = match es.codec {
                // C6 (#1009): probe the reassembled ES rather than assuming
                // every 0xE0-0xEF `stream_id` is H.264 — a MPEG-2 sequence
                // header (`0x000001B3`, ISO/IEC 13818-2 §6.2.2.1) is
                // structural evidence no H.264 NAL stream can produce by
                // chance, so it is checked first.
                Codec::Video if Mpeg2SeqHeader::find(&es.es_bytes).is_ok() => {
                    build_mpeg2_track(es, track_id)
                }
                // The H.264 path can fail on a resource limit (a NAL past the
                // splitter's cap); that must surface as an error rather than
                // "no video track" (r04-W21 review).
                Codec::Video => build_h264_track(es, track_id)?,
                Codec::Ac3 => build_ac3_track(es, track_id),
                // A private substream that is not AC-3, and the placeholder an
                // undecided substream is parked at, both produce no track.
                Codec::Private | Codec::Skipped => None,
            };
            if let Some(track) = built {
                tracks.push(track);
                track_id += 1;
            }
        }

        Ok(Media::new(tracks, VIDEO_TIMESCALE))
    }
}

impl<'a> Unpackage for PsDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        self.demux(input)
    }
}

/// Positions of every start code's first `00` (of the trailing `00 00 01`) in an
/// Annex B byte stream. Used to split the reassembled video ES into access units.
fn start_code_positions(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let n = data.len();
    let mut p = 0usize;
    while p + 3 <= n {
        if data[p] == 0 && data[p + 1] == 0 && data[p + 2] == 1 {
            positions.push(p);
            p += 3;
        } else {
            p += 1;
        }
    }
    positions
}

/// The access units recovered from a reassembled Annex B byte stream, together
/// with each unit's `(start, end)` byte range in the input — the ranges
/// [`assign_stamps`] needs to place a PES-level stamp on the unit that begins
/// inside its fragment.
type SplitUnits = (Vec<Vec<u8>>, Vec<(usize, usize)>);

/// A `Result<Option<_>>` whose `Ok(None)` means "this stream has nothing this
/// demuxer can carry" (skip, never fatal) and whose `Err` means the stream was
/// rejected (a resource limit, not a format the demuxer declines). The
/// distinction matters: the two used to be the same value, so a rejection
/// looked like an unsupported stream and the video track vanished silently.
type MaybeTrack = Result<Option<Track>>;

/// Split a reassembled H.264 Annex B byte stream into access units, returning
/// the unit bytes together with their `(start, end)` ranges **into `data`** —
/// the ranges [`assign_stamps`] needs to place each PES-level stamp on the unit
/// that begins inside its fragment.
///
/// The boundaries come from [`crate::au::AccessUnitSplitter`] (r04-W21): a new
/// access unit starts at an access-unit delimiter *or* at the first slice whose
/// `first_mb_in_slice` is 0 (H.264 §7.4.3 — the first VCL NAL of a primary
/// coded picture). Requiring an AUD made every PS capture without one collapse
/// into a single sample.
///
/// Returns `None` if the stream yields no access units at all.
fn split_h264_access_units(data: &[u8]) -> Result<Option<SplitUnits>> {
    let mut splitter = AccessUnitSplitter::new(NalCodec::Avc);
    // The splitter drops everything before the first start code (leading junk,
    // and the zero byte of a 4-byte start code), so the units it returns do not
    // begin at offset 0 of `data`. Starting the offset walk at 0 therefore
    // mismatched on the first unit and the old code returned "no units",
    // silently discarding the whole video track for any stream that does not
    // open exactly on a start code (r04-W21 review). Anchor the walk at the
    // first start code instead.
    let Some(mut cursor) = first_nal_offset(data) else {
        return Ok(None);
    };
    // A push past the splitter's buffered-NAL cap is a resource-limit
    // rejection, not an unparseable stream; propagating it keeps the caller
    // from emitting a `Media` whose video track is silently missing.
    splitter.push(data)?;
    splitter.finish();

    let mut units = Vec::new();
    let mut ranges = Vec::new();
    while let Some(unit) = splitter.pop() {
        // The splitter hands back verbatim, in-order slices of what was pushed,
        // so each unit must sit exactly at the running cursor. This is checked
        // rather than assumed: a mismatch means the offsets are wrong and every
        // unit's timestamps would be mis-stamped, so it is an error — a
        // `debug_assert!` here let release builds emit the wrong samples
        // silently.
        let end = cursor.checked_add(unit.len()).ok_or(Error::InvalidInput(
            "ps: H.264 access-unit offset overflowed",
        ))?;
        if data.get(cursor..end) != Some(unit.as_slice()) {
            return Err(Error::InvalidInput(
                "ps: H.264 access-unit splitter returned bytes that are not a                  slice of its input",
            ));
        }
        ranges.push((cursor, end));
        units.push(unit);
        cursor = end;
    }
    Ok(if units.is_empty() {
        None
    } else {
        Some((units, ranges))
    })
}

/// Offset of the first Annex B start code (`00 00 01`) in `data`, pulled back
/// over **every** leading zero byte so the returned offset is where the NAL's
/// start code really begins.
///
/// A start code may be preceded by any number of zero bytes — `00 00 00 01` is
/// the common 4-byte form, but longer runs are legal padding and are what a PES
/// stuffing tail leaves. Folding back only one of them (as this did) puts the
/// offset inside the run, so every access-unit range the caller derives from it
/// is short by the remaining zeros. Mirrors `au::first_nal_start`.
fn first_nal_offset(data: &[u8]) -> Option<usize> {
    let pos = data.windows(3).position(|w| w == [0, 0, 1])?;
    let mut start = pos;
    while start > 0 && data[start - 1] == 0 {
        start -= 1;
    }
    Some(start)
}

/// Split a reassembled MPEG-2 video byte stream into access units at every
/// `picture_start_code` (ISO/IEC 13818-2 §6.2.3, `0x00000100`) — MPEG-2 has no
/// AUD equivalent, so a new picture is the only reliable per-AU boundary (C6,
/// #1009). Bytes before the first picture (the `sequence_header()` and any
/// `extension`/`user_data`) are attached to the first AU, same convention as
/// [`split_access_units`].
fn split_mpeg2_pictures(data: &[u8]) -> Vec<(usize, usize)> {
    let codes = start_code_positions(data);
    let mut starts: Vec<usize> = Vec::new();
    for &pos in &codes {
        if pos + 3 < data.len() && data[pos + 3] == MPEG2_PICTURE_START_CODE {
            starts.push(pos);
        }
    }
    if starts.is_empty() {
        return if data.is_empty() {
            Vec::new()
        } else {
            alloc::vec![(0, data.len())]
        };
    }
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(starts.len());
    let n = starts.len();
    for i in 0..n {
        let start = if i == 0 { 0 } else { starts[i] };
        let end = if i + 1 < n { starts[i + 1] } else { data.len() };
        ranges.push((start, end));
    }
    ranges
}

/// Extend a running unwrapped timestamp by the delta to the next raw 33-bit
/// value, correcting for a single 90 kHz wrap in either direction (§2.4.3.7).
fn unwrap_ts(prev_unwrapped: i128, prev_raw: u64, raw: u64) -> i128 {
    let mut delta = raw as i128 - prev_raw as i128;
    if delta > TS_WRAP_HALF {
        delta -= TS_WRAP;
    } else if delta < -TS_WRAP_HALF {
        delta += TS_WRAP;
    }
    prev_unwrapped + delta
}

/// Assign each access unit its (optional) PTS/DTS from the PES-level stamps.
///
/// A stamp applies to the first access unit whose byte range begins at or after
/// the stamp's offset — i.e. the first AU that starts in that PES fragment.
/// Timestamps are unwrapped across the 33-bit wrap using stamp (stream) order.
fn assign_stamps(ranges: &[(usize, usize)], stamps: &[Stamp]) -> Vec<(Option<u64>, Option<u64>)> {
    let mut out = alloc::vec![(None, None); ranges.len()];
    // Unwrap the stamp timestamps across the 33-bit wrap, in stamp order.
    let mut si = 0usize;
    let (mut prev_pts_raw, mut prev_pts_uw): (Option<u64>, i128) = (None, 0);
    let (mut prev_dts_raw, mut prev_dts_uw): (Option<u64>, i128) = (None, 0);
    for (ai, &(start, _end)) in ranges.iter().enumerate() {
        // Consume the last stamp whose offset falls at/before this AU's start,
        // preferring the earliest AU that begins in the fragment.
        while si < stamps.len() && stamps[si].offset <= start {
            let s = stamps[si];
            let pts_uw = s.pts.map(|p| match prev_pts_raw {
                Some(pr) => {
                    let uw = unwrap_ts(prev_pts_uw, pr, p);
                    prev_pts_uw = uw;
                    prev_pts_raw = Some(p);
                    uw
                }
                None => {
                    prev_pts_uw = p as i128;
                    prev_pts_raw = Some(p);
                    p as i128
                }
            });
            let dts_uw = s.dts.map(|d| match prev_dts_raw {
                Some(pr) => {
                    let uw = unwrap_ts(prev_dts_uw, pr, d);
                    prev_dts_uw = uw;
                    prev_dts_raw = Some(d);
                    uw
                }
                None => {
                    prev_dts_uw = d as i128;
                    prev_dts_raw = Some(d);
                    d as i128
                }
            });
            // Stamp applies to the AU that begins this fragment (this AU), only
            // if not already stamped (first AU wins for the fragment).
            if out[ai].0.is_none() && out[ai].1.is_none() {
                out[ai] = (pts_uw.map(|v| v as u64), dts_uw.map(|v| v as u64));
            }
            si += 1;
        }
    }
    out
}

/// Recover H.264 config + build video samples (Annex B → length-prefixed).
///
/// Splits the reassembled Annex B stream into access units with
/// [`crate::au::AccessUnitSplitter`] — the crate's single H.264 AU-boundary
/// implementation, which decides a boundary from `first_mb_in_slice`
/// (H.264 §7.4.3) rather than requiring an access-unit delimiter. An AUD is
/// *optional* in H.264 and is absent from most PS/MPEG-2-programme captures, so
/// an AUD-only split left such a stream as a single "access unit" spanning the
/// whole file — one sample, one IDR flag (r04-W21). Each unit is then stamped
/// with the PES-level PTS/DTS at its fragment start and emitted in decode
/// order. Returns `None` if in-band SPS/PPS cannot be found (skip, never fatal).
fn build_h264_track(es: &ElementaryStream, track_id: u32) -> MaybeTrack {
    let Some((units, ranges)) = split_h264_access_units(&es.es_bytes)? else {
        return Ok(None);
    };
    let stamped = assign_stamps(&ranges, &es.stamps);

    // Recover SPS/PPS (first of each) and attach each AU's PES-level stamps.
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let units: Vec<AccessUnit> = units
        .into_iter()
        .enumerate()
        .map(|(i, data)| {
            for nal in iter_annexb_nals(&data) {
                match nal[0] & H264_NAL_TYPE_MASK {
                    H264_NAL_SPS if sps.is_none() => sps = Some(nal.to_vec()),
                    H264_NAL_PPS if pps.is_none() => pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            AccessUnit {
                data,
                pts: stamped[i].0,
                dts: stamped[i].1,
            }
        })
        .collect();
    let (Some(sps), Some(pps)) = (sps, pps) else {
        return Ok(None);
    };
    if sps.len() < 4 {
        return Ok(None);
    }

    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        // profile_idc / constraint_flags / level_idc live at SPS bytes 1..=3
        // (after the 1-byte NAL header) — ISO/IEC 14496-15 §5.3.3.1.
        profile_indication: sps[1],
        profile_compatibility: sps[2],
        level_indication: sps[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps: alloc::vec![AvcSps(sps)],
        pps: alloc::vec![AvcPps(pps)],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: alloc::vec![],
    };
    let config = AVCConfigurationBox::new(record);

    // Fill in DTS for unstamped AUs by anchoring off the stamped ones and the
    // constant frame duration, so every sample carries a decode time.
    let dts = interpolate_dts(&units);
    let pts = interpolate_pts(&units, &dts);

    // Decode order = ascending DTS (stable — preserves stream order for ties).
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| dts[i]);

    // Absolute dts/pts (media plane step 2c): `interpolate_dts`/
    // `interpolate_pts` already yield **unwrapped absolute** 90 kHz values
    // (the PES-level stamps are wrap-unrolled once in `assign_stamps` via
    // `unwrap_ts`, and unstamped AUs are anchored off them), so MPEG Program
    // Stream genuinely recovers an absolute clock rather than leaving the
    // anchor at 0. `composition_offset` folds into the pts/dts pair.
    let samples: Vec<Sample> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let dur = frame_duration(&order, &dts, pos);
            let is_idr = au_is_idr(&units[i].data);
            Sample::from_annexb(
                &units[i].data,
                Some(to_ticks(dts[i])),
                Some(to_ticks(pts[i])),
                Some(dur),
                is_idr,
            )
        })
        .collect();

    // Anchor = the first decode-ordered sample's absolute DTS, kept in
    // lockstep with `samples[0].dts` per the crate-wide IR invariant.
    let anchor = order.first().map(|&i| dts[i].max(0) as u64).unwrap_or(0);
    Ok(Some(Track::new_at(
        TrackSpec::new(
            track_id,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        ),
        samples,
        anchor,
    )))
}

/// Recover MPEG-2 video config (picture geometry from `sequence_header()`) and
/// build one raw sample per picture (C6, #1009). Returns `None` if no
/// `picture_start_code` is found (skip, never fatal) — the sequence header
/// itself was already confirmed present by the caller.
fn build_mpeg2_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let seq = Mpeg2SeqHeader::find(&es.es_bytes).ok()?;
    let ranges = split_mpeg2_pictures(&es.es_bytes);
    if ranges.is_empty() {
        return None;
    }
    let stamped = assign_stamps(&ranges, &es.stamps);

    let mut units: Vec<AccessUnit> = Vec::with_capacity(ranges.len());
    for (i, &(start, end)) in ranges.iter().enumerate() {
        units.push(AccessUnit {
            data: es.es_bytes[start..end].to_vec(),
            pts: stamped[i].0,
            dts: stamped[i].1,
        });
    }

    // `esds` carrying the MPEG-2 Main Visual object type (ISO/IEC 14496-1
    // Table 5, `OTI_MPEG2_VIDEO_MAIN` = 0x61) — the same construction
    // `ts_demux`'s `ConfigProbe::Mpeg2Video` uses for the TS input side.
    let esds = EsdsBox::new(ESDescriptor::new(
        ESDS_VIDEO_ES_ID,
        0,
        Some(DecoderConfigDescriptor::new(
            OTI_MPEG2_VIDEO_MAIN,
            STREAM_TYPE_VISUAL,
            false,
            0,
            0,
            0,
            None,
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));

    let dts = interpolate_dts(&units);
    let pts = interpolate_pts(&units, &dts);
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| dts[i]);

    let samples: Vec<Sample> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let dur = frame_duration(&order, &dts, pos);
            let is_sync = mpeg2_is_sync(&units[i].data);
            Sample::new(
                units[i].data.clone(),
                Some(to_ticks(dts[i])),
                Some(to_ticks(pts[i])),
                Some(dur),
                is_sync,
            )
        })
        .collect();

    let anchor = order.first().map(|&i| dts[i].max(0) as u64).unwrap_or(0);
    Some(Track::new_at(
        TrackSpec::new(
            track_id,
            VIDEO_TIMESCALE,
            CodecConfig::Mpeg2Video {
                esds,
                width: seq.width,
                height: seq.height,
            },
        ),
        samples,
        anchor,
    ))
}

/// Clamp an unwrapped 33-bit-derived `i128` timestamp into the `i64` range
/// [`Sample::dts`]/[`Sample::pts`] carry (mirrors `ts_demux`'s own
/// `to_ticks`): `i128` is used internally purely for wrap-arithmetic
/// headroom, and every real value is a small multiple of the 33-bit range
/// that fits `i64` with centuries of 90 kHz headroom — so this never clamps
/// in practice, it just makes the narrowing checked instead of silent.
fn to_ticks(uw: i128) -> i64 {
    uw.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// True if an Annex B access unit contains an IDR slice NAL (type 5).
fn au_is_idr(au: &[u8]) -> bool {
    iter_annexb_nals(au).any(|nal| (nal[0] & H264_NAL_TYPE_MASK) == H264_NAL_IDR)
}

/// Derive the constant per-frame duration (90 kHz ticks) from the stamped DTS
/// deltas, falling back to [`DEFAULT_FRAME_DURATION`] when fewer than two are
/// known.
fn stamped_frame_duration(units: &[AccessUnit]) -> i128 {
    let stamped: Vec<(usize, i128)> = units
        .iter()
        .enumerate()
        .filter_map(|(i, u)| u.dts.map(|d| (i, d as i128)))
        .collect();
    if stamped.len() < 2 {
        return DEFAULT_FRAME_DURATION;
    }
    let (i0, d0) = stamped[0];
    let (i1, d1) = *stamped.last().unwrap();
    let span_idx = (i1 - i0) as i128;
    let span_dts = d1 - d0;
    if span_idx > 0 && span_dts > 0 {
        (span_dts / span_idx).max(1)
    } else {
        DEFAULT_FRAME_DURATION
    }
}

/// Per-AU decode timestamp: the explicit PES DTS where present, else anchored off
/// the nearest known DTS by the constant frame duration (index distance).
fn interpolate_dts(units: &[AccessUnit]) -> Vec<i128> {
    let dur = stamped_frame_duration(units);
    let n = units.len();
    let mut dts = alloc::vec![0i128; n];
    // Find the first stamped anchor to seed the whole run.
    let anchor = units
        .iter()
        .enumerate()
        .find_map(|(i, u)| u.dts.map(|d| (i, d as i128)));
    let (anchor_idx, anchor_dts) = anchor.unwrap_or((0, 0));
    for (i, slot) in dts.iter_mut().enumerate() {
        *slot = match units[i].dts {
            Some(d) => d as i128,
            None => anchor_dts + (i as i128 - anchor_idx as i128) * dur,
        };
    }
    dts
}

/// Per-AU presentation timestamp: the explicit PES PTS where present, else the
/// AU's decode time (no reordering information available for unstamped frames).
fn interpolate_pts(units: &[AccessUnit], dts: &[i128]) -> Vec<i128> {
    units
        .iter()
        .enumerate()
        .map(|(i, u)| u.pts.map(|p| p as i128).unwrap_or(dts[i]))
        .collect()
}

/// Duration of the sample at decode position `pos`: the gap to the next
/// decode-ordered DTS; the final sample reuses the previous gap.
fn frame_duration(order: &[usize], dts: &[i128], pos: usize) -> u32 {
    let n = order.len();
    let dur = if pos + 1 < n {
        (dts[order[pos + 1]] - dts[order[pos]]).max(0)
    } else if pos > 0 {
        (dts[order[pos]] - dts[order[pos - 1]]).max(0)
    } else {
        DEFAULT_FRAME_DURATION
    };
    dur as u32
}

/// Split a reassembled AC-3 byte stream into individual syncframes and return
/// their `(start, end)` byte ranges.
///
/// The frames are located by [`crate::ac3::split_ac3_syncframes_resyncing`],
/// which walks each syncframe's own `frmsizecod`-derived length (ETSI TS 102 366
/// Table 4.13) instead of hunting for the next `0x0B77`, and resynchronises
/// after a frame that does not parse so a single corrupted frame does not
/// discard the rest of the stream. That distinction is
/// load-bearing: the 16-bit syncword occurs *inside* AC-3 payload by chance
/// (roughly one position in 65 536, so a few percent of 1792-byte frames), and
/// splitting on those false syncs cuts real frames in half and stamps each half
/// with a full 1536-sample duration — corrupt access units and A/V drift
/// against the PES stamps (r04-W20).
fn split_ac3_frames(data: &[u8]) -> Vec<(usize, usize)> {
    // Resyncing: one corrupted frame must not discard every frame after it. The
    // ranges come from the splitter directly — they are *not* contiguous from
    // offset 0 once a resync has skipped bytes, so reconstructing them by
    // summing frame lengths (as this used to) slices mid-frame and emits a
    // sample beginning at the corrupted byte.
    crate::ac3::split_ac3_syncframe_ranges(data, true)
}

/// Recover AC-3 config (syncframe BSI → `dac3`) + one raw sample per syncframe.
/// Returns `None` if no valid AC-3 syncframe is found (skip, never fatal).
fn build_ac3_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let info = Ac3SyncframeInfo::from_es(&es.es_bytes).ok()?;
    let sample_rate = info.sample_rate;
    let channel_count = info.channel_count() as u16;
    let config = info.into_dac3();

    let frames = split_ac3_frames(&es.es_bytes);
    if frames.is_empty() {
        return None;
    }

    // Absolute dts/pts (media plane step 2c). The PES-level stamps are 90 kHz
    // (ISO/IEC 13818-1 §2.4.3.7) while this track's timescale is
    // `sample_rate`, so a stamp needs rescaling into the track clock — but
    // (issue B5 sibling, found via the FIX C invariant test, media plane
    // step-2 fix wave 1) 90000 does not evenly divide a typical sample rate,
    // so re-deriving the dts from EVERY stamped frame's own rescale (as this
    // used to) injects up to ±1 track tick of jitter wherever a stamp
    // happens to be present — exactly the bug `ts_demux::emit_audio_au` was
    // fixed for. The anchor is established once (or re-established on a
    // genuine gap — a stamp drifting from the frame-exact accumulator's
    // predicted position by more than
    // [`crate::ts_demux::audio_discontinuity_threshold_90k`]) and otherwise
    // simply advances by the **intrinsic** AC-3 syncframe duration (1536
    // samples — ETSI TS 102 366 §4.1, `AC3_SAMPLES_PER_SYNCFRAME`), which is
    // exactly the value `ts_demux` uses for the same split and also the
    // per-sample `duration` (previously left at `0`, which made the AC-3
    // timeline uninterpretable).
    let rescale_90k = |t90: u64| -> i64 {
        ((t90 as u128 * sample_rate as u128) / VIDEO_TIMESCALE as u128) as i64
    };
    let stamped = assign_stamps(&frames, &es.stamps);
    let mut samples: Vec<Sample> = Vec::with_capacity(frames.len());
    let mut next_dts: Option<i64> = None;
    // The unwrapped 90 kHz stamp the anchor was last (re-)established from,
    // and how many track ticks have elapsed since — used only to predict the
    // expected wire position for drift detection, never to derive a dts.
    let mut anchor_wire90: i128 = 0;
    let mut ticks_since_anchor: i64 = 0;
    for (i, &(s, e)) in frames.iter().enumerate() {
        // Prefer this frame's own stamp (DTS, else PTS — AC-3 is never
        // reordered, so they coincide) ONLY to (re-)anchor; otherwise
        // continue the running, frame-exact clock.
        let stamp90 = stamped[i].1.or(stamped[i].0);
        let dts = match (stamp90, next_dts) {
            (Some(t90), Some(running)) => {
                let expected_wire90 = anchor_wire90
                    + (ticks_since_anchor as i128 * VIDEO_TIMESCALE as i128)
                        / sample_rate.max(1) as i128;
                let drift = (t90 as i128 - expected_wire90).abs();
                if drift > crate::ts_demux::audio_discontinuity_threshold_90k(sample_rate) {
                    // A genuine gap: re-anchor from the wire stamp.
                    anchor_wire90 = t90 as i128;
                    ticks_since_anchor = 0;
                    rescale_90k(t90)
                } else {
                    running
                }
            }
            (Some(t90), None) => {
                // First stamped frame: establish the anchor.
                anchor_wire90 = t90 as i128;
                ticks_since_anchor = 0;
                rescale_90k(t90)
            }
            (None, Some(running)) => running,
            // No stamp has been seen yet at all: this leading frame precedes
            // the stream's first PES timestamp, so its absolute time is
            // genuinely unknown — `None`, never fabricated.
            (None, None) => {
                samples.push(Sample::from_raw(
                    es.es_bytes[s..e].to_vec(),
                    None,
                    None,
                    Some(AC3_SAMPLES_PER_SYNCFRAME),
                ));
                continue;
            }
        };
        next_dts = Some(dts + AC3_SAMPLES_PER_SYNCFRAME as i64);
        ticks_since_anchor += AC3_SAMPLES_PER_SYNCFRAME as i64;
        samples.push(Sample::from_raw(
            es.es_bytes[s..e].to_vec(),
            Some(dts),
            Some(dts),
            Some(AC3_SAMPLES_PER_SYNCFRAME),
        ));
    }

    // Anchor = the first sample's absolute DTS when known (lockstep with
    // `samples[0].dts`), else 0.
    let anchor = samples
        .first()
        .and_then(|s| s.dts)
        .map(|d| d.max(0) as u64)
        .unwrap_or(0);
    Some(Track::new_at(
        TrackSpec::new(
            track_id,
            sample_rate,
            CodecConfig::Ac3 {
                config,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            },
        ),
        samples,
        anchor,
    ))
}
