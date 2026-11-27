//! Sans-IO fragmented MP4 writer: the initialization segment and the
//! `moof`+`mdat` fragments that follow it.
//!
//! ISO/IEC 14496-12:2026 §8.8 (movie fragments), §8.16 (CMAF), §8.14.2
//! (`styp`). The [`FragmentedWriter`] owns the per-track sample buffers and
//! hands back one whole `[styp] [emsg]* moof mdat` run per
//! [`flush`](FragmentedWriter::flush); [`fragmented_segment`] is the same one
//! builder for a caller that already has its samples in hand.
//!
//! Every count and size taken from a file is checked against [`Limits`]
//! before anything is allocated, and the buffered fragment is capped by
//! [`Limits::max_fragment_bytes`] before a push grows it: a hostile header can
//! only ever produce a typed error.

use crate::box_support::OpaqueBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE};
use crate::cenc::{CencScheme, SubSampleEntry, TrackEncryptionBox};
use crate::emsg::EmsgBox;
use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox, DataReferenceBox, HandlerBox,
    MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox, MovieExtendsBox, MovieHeaderBox,
    SampleDescriptionBox, SampleEntryVariant, SampleSizeBox, SampleTableBox, SampleToChunkBox,
    SoundMediaHeaderBox, StblChild, TrackBox, TrackExtendsBox, TrackHeaderBox, VideoMediaHeaderBox,
};
use crate::limits::Limits;
use crate::segments::FileTypeBox;
use crate::timing::TimeToSampleBox;
use crate::track_id::TrackId;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_core::Serialize;
use broadcast_core::timescale::Timescale;

pub mod fragmented;
mod index;
pub mod progressive;

pub use fragmented::{FragmentedWriter, RunSample, fragmented_segment, media_fragment_run};
pub use progressive::{Interleave, Layout, MovieHeader, ProgressiveWriter, Sink, Write};

/// ISO-639-2 packed language code `und` (undetermined).
pub(crate) const LANG_UND: u16 = 0x55C4;
/// The ISOBMFF identity transformation matrix (§7.3.2), unity + `[2,30]` fixed.
const IDENTITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
/// `tkhd` flags: track_enabled | in_movie | in_preview.
const TKHD_ENABLED_IN_MOVIE: u32 = 0x0000_0007;
/// `ftyp` minor version this crate writes for its default brand set.
const FTYP_MINOR_VERSION: u32 = 512;
/// `ftyp` minor version a QuickTime `qt  ` brand carries (the value every
/// QuickTime `.mov` fixture in `private/fixtures/mp4/quicktime/` states).
const FTYP_QUICKTIME_MINOR_VERSION: u32 = 512;
/// `mvhd` `rate` for normal playback: 1.0 in 16.16 fixed point (§8.2.2).
const MVHD_RATE_UNITY: u32 = 0x0001_0000;
/// `mvhd` `volume` for full volume: 1.0 in 8.8 fixed point (§8.2.2).
const MVHD_VOLUME_UNITY: u16 = 0x0100;
/// `trex` default sample description index (1-based, §8.8.3).
const TREX_DEFAULT_DESCRIPTION_INDEX: u32 = 1;
/// The one `url ` data entry a self-contained track carries (§8.7.2).
const SELF_CONTAINED_DATA_ENTRY_FLAGS: u32 = 1;
/// The Null Media Header (`nmhd`, §8.4.5.2): the media header for a handler
/// with no header of its own.
const NMHD: &[u8; BOX_TYPE_SIZE] = b"nmhd";

/// The `ftyp` brand set an initialization segment is written under.
///
/// The brand choice belongs to the caller (a CMAF packager writes
/// `iso5`/`iso6`/`mp41`, a Smooth packager something else), so it is a value
/// the writer carries rather than a constant it decides. A QuickTime file uses
/// [`FileType::quicktime`]; the `ftyp` atom is optional in QuickTime
/// (`private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
/// "File type compatibility atom ('ftyp')"), so [`FileType::without_ftyp`]
/// writes a classic brandless file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileType {
    /// Best-use brand.
    pub major_brand: [u8; BOX_TYPE_SIZE],
    /// Minor version of the major brand.
    pub minor_version: u32,
    /// Brands the file is compatible with.
    pub compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
    /// Whether the file starts with an `ftyp` atom.
    pub emit_ftyp: bool,
}

impl FileType {
    /// The QuickTime brand set: major brand `qt  `, the minor version and
    /// compatible brand a QuickTime writer emits.
    #[must_use]
    pub fn quicktime() -> Self {
        Self {
            major_brand: *b"qt  ",
            minor_version: FTYP_QUICKTIME_MINOR_VERSION,
            compatible_brands: vec![*b"qt  "],
            emit_ftyp: true,
        }
    }

    /// The same brands, but with no `ftyp` atom at all (classic QuickTime).
    #[must_use]
    pub fn without_ftyp() -> Self {
        Self {
            emit_ftyp: false,
            ..Self::default()
        }
    }
}

impl Default for FileType {
    fn default() -> Self {
        Self {
            major_brand: *b"iso5",
            minor_version: FTYP_MINOR_VERSION,
            compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
            emit_ftyp: true,
        }
    }
}

/// The `styp` brand set of the fragments a writer flushes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentBrands {
    /// Best-use brand.
    pub major_brand: [u8; BOX_TYPE_SIZE],
    /// Minor version of the major brand.
    pub minor_version: u32,
    /// Brands the segment is compatible with.
    pub compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
}

/// One sample entry as the init segment's `stsd` carries it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum SampleEntry {
    /// A typed entry this crate parses and serializes.
    Typed(SampleEntryVariant),
    /// An entry this crate has no typed form for, kept as its complete box
    /// bytes (header included) so the writer emits it verbatim.
    Raw {
        /// The entry's four-CC.
        fourcc: [u8; BOX_TYPE_SIZE],
        /// The complete sample-entry box, 8-byte header included.
        body: Vec<u8>,
    },
}

impl SampleEntry {
    /// A registered private sample entry as the raw entry the writer carries.
    ///
    /// The writer itself needs no [`BoxRegistry`](crate::registry::BoxRegistry):
    /// a private entry becomes its complete box bytes here, at the call
    /// boundary, and travels through [`SampleEntry::Raw`] unchanged.
    pub fn from_registered(entry: &dyn crate::registry::RegisteredBox) -> Result<Self> {
        let fourcc = entry.fourcc();
        let mut body = vec![0u8; entry.serialized_len()];
        let written = entry.serialize_into(&mut body)?;
        body.truncate(written);
        raw_entry_variant(fourcc, &body)?;
        Ok(Self::Raw { fourcc, body })
    }
}

/// The default sample values one track's `trex` states (§8.8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrexDefaults {
    /// Default `stsd` entry index (1-based).
    pub sample_description_index: u32,
    /// Default sample duration, in the track timescale.
    pub default_sample_duration: u32,
    /// Default sample size, in bytes.
    pub default_sample_size: u32,
    /// Default per-sample flags (§8.8.3 sample-flags layout).
    pub default_sample_flags: u32,
}

impl Default for TrexDefaults {
    fn default() -> Self {
        Self {
            sample_description_index: TREX_DEFAULT_DESCRIPTION_INDEX,
            default_sample_duration: 0,
            default_sample_size: 0,
            default_sample_flags: 0,
        }
    }
}

/// CENC protection applied to a track's sample entry in the init segment
/// (ISO/IEC 14496-12:2026 §13.4, ISO/IEC 23001-7 §12.2).
#[derive(Debug, Clone, PartialEq)]
pub struct SampleEntryProtection {
    /// The protection scheme (`schm.scheme_type`).
    pub scheme: CencScheme,
    /// The track encryption box (`tenc`) that carries the scheme parameters.
    pub tenc: TrackEncryptionBox,
}

/// Everything the init segment needs to know about one track.
///
/// The fields are the `tkhd`/`mdhd`/`hdlr`/`stsd` values the caller already
/// has in its own model; the writer assembles the boxes from them. A track
/// with any four-CC handler and any sample entry can be written, so a track
/// outside the caller's codec set is not dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackSetup {
    /// `tkhd.track_ID` / `tfhd.track_ID` / `trex.track_ID`.
    pub track_id: TrackId,
    /// `hdlr.handler_type` (`vide`, `soun`, `meta`, …).
    pub handler: [u8; BOX_TYPE_SIZE],
    /// `hdlr.name` bytes (NUL-terminated by the caller's own convention).
    pub handler_name: Vec<u8>,
    /// The headerless FullBox kept as the first `minf` child (`sthd`, `nmhd`),
    /// when the handler needs one (§7.3.4).
    pub minf_header: Option<[u8; BOX_TYPE_SIZE]>,
    /// `mdhd.timescale`.
    pub timescale: Timescale,
    /// `mdhd.language`, packed as ISO/IEC 14496-12:2026 §8.4.2.3 defines it
    /// (three 5-bit letters); `und` when the caller has none.
    pub language: u16,
    /// `tkhd.width`, 16.16 fixed point.
    pub width: u32,
    /// `tkhd.height`, 16.16 fixed point.
    pub height: u32,
    /// `tkhd.volume`, 8.8 fixed point.
    pub volume: i16,
    /// The single sample entry of the fragmented init.
    pub entry: SampleEntry,
    /// The track's `trex` defaults; `None` writes the neutral ones.
    pub trex: Option<TrexDefaults>,
    /// CENC protection to wrap the sample entry with.
    pub protection: Option<SampleEntryProtection>,
    /// The QuickTime base media information header (`gmhd`) this `minf`
    /// carries, when the caller builds a QuickTime track.
    pub gmhd: Option<crate::quicktime::GmhdAtom>,
    /// `tref` references the track declares (`tmcd`, `chap`, `avcp`, …).
    pub references: Vec<crate::track_reference::TrackReferenceTypeBox>,
    /// The parsed `trak` this setup was read from, holding every `tkhd`/
    /// `mdhd`/`hdlr` field, extra child (`tref`, `udta`, `gmhd`) and wire order
    /// a [`TrackSetup`] does not model. The writer swaps its own `stbl` in and
    /// writes the rest exactly as the source carried it; `None` builds the
    /// conventional `trak`.
    pub shell: Option<TrackBox>,
}

impl TrackSetup {
    /// A track with the neutral `tkhd` and `trex` values.
    pub fn new(
        track_id: TrackId,
        handler: [u8; BOX_TYPE_SIZE],
        handler_name: Vec<u8>,
        timescale: Timescale,
        entry: SampleEntry,
    ) -> Self {
        Self {
            track_id,
            handler,
            handler_name,
            minf_header: None,
            timescale,
            language: LANG_UND,
            width: 0,
            height: 0,
            volume: 0,
            entry,
            trex: None,
            protection: None,
            gmhd: None,
            references: Vec::new(),
            shell: None,
        }
    }
}

/// The writer's entry point.
pub struct Writer;

impl Writer {
    /// Start a fragmented writer over `tracks`, in the order they appear in
    /// the `moov` and in each `moof`.
    ///
    /// The buffered fragment is capped by `limits`; a push that would exceed
    /// [`Limits::max_fragment_bytes`] fails before the buffer grows.
    pub fn fragmented(
        tracks: Vec<TrackSetup>,
        movie_timescale: Timescale,
        limits: Limits,
    ) -> Result<FragmentedWriter> {
        FragmentedWriter::new(tracks, movie_timescale, limits)
    }

    /// Start a progressive (non-fragmented) writer over a whole sample plan.
    ///
    /// The layout — chunk boundaries, `stco` or `co64`, the `mdat` header form
    /// and every sample's absolute offset — is fixed here from `plan` alone,
    /// before a sample byte is pushed.
    pub fn progressive(
        plan: crate::plan::OwnedPlan,
        layout: Layout,
        file_type: FileType,
        movie: MovieHeader,
        limits: Limits,
    ) -> Result<ProgressiveWriter> {
        ProgressiveWriter::new(plan, layout, file_type, movie, limits)
    }
}

/// Build a fragmented initialization segment (`ftyp` + `moov`) for `tracks`.
///
/// The `moov` carries one empty `stbl` per track (`stsd` plus empty
/// `stts`/`stsc`/`stsz`/`stco`) and an `mvex`/`trex`, marking the movie as
/// fragmented (§8.8).
pub(crate) fn build_init_segment(
    tracks: &[TrackSetup],
    movie_timescale: Timescale,
    file_type: &FileType,
    shell: Option<&crate::plan::MovieShell>,
) -> Result<Vec<u8>> {
    let ftyp = FileTypeBox {
        major_brand: file_type.major_brand,
        minor_version: file_type.minor_version,
        compatible_brands: file_type.compatible_brands.clone(),
    };

    let next_track_id = tracks
        .iter()
        .map(|t| t.track_id)
        .max()
        .map_or(TrackId::new(1), TrackId::next);
    let mvhd = match shell {
        Some(shell) => shell.mvhd.clone(),
        None => MovieHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: movie_timescale,
            duration: 0,
            rate: MVHD_RATE_UNITY,
            volume: MVHD_VOLUME_UNITY,
            reserved: Default::default(),
            matrix: IDENTITY_MATRIX,
            pre_defined: Default::default(),
            next_track_id,
        },
    };

    let mut trak_boxes = Vec::with_capacity(tracks.len());
    let mut trex = Vec::with_capacity(tracks.len());
    for t in tracks {
        trak_boxes.push(build_trak(t)?);
        let defaults = t.trex.unwrap_or_default();
        trex.push(TrackExtendsBox {
            version: 0,
            flags: 0,
            track_id: t.track_id,
            default_sample_description_index: defaults.sample_description_index,
            default_sample_duration: defaults.default_sample_duration,
            default_sample_size: defaults.default_sample_size,
            default_sample_flags: defaults.default_sample_flags,
        });
    }

    let moov = match shell {
        Some(shell) => MovieBox {
            mvhd,
            tracks: trak_boxes,
            mvex: shell
                .mvex
                .clone()
                .or_else(|| Some(MovieExtendsBox::new(trex, vec![]))),
            opaque: shell.opaque.clone(),
            order: shell.order.clone(),
        },
        None => MovieBox::new(
            mvhd,
            trak_boxes,
            Some(MovieExtendsBox::new(trex, vec![])),
            vec![],
        ),
    };

    let mut out = Vec::new();
    if file_type.emit_ftyp {
        out.extend_from_slice(&ftyp.try_to_bytes()?);
    }
    if let Some(shell) = shell {
        for box_ in shell.top_level_at(crate::plan::BoxPosition::AfterFtyp) {
            out.extend_from_slice(&box_.try_to_bytes()?);
        }
    }
    out.extend_from_slice(&moov.try_to_bytes()?);
    if let Some(shell) = shell {
        for box_ in shell.top_level_at(crate::plan::BoxPosition::AfterMoov) {
            out.extend_from_slice(&box_.try_to_bytes()?);
        }
    }
    Ok(out)
}

/// Build one fragmented-init `trak`: empty sample tables plus a single `stsd`
/// entry.
fn build_trak(t: &TrackSetup) -> Result<TrackBox> {
    let entry = track_entry(t)?;
    let template = progressive::stbl_template(t.shell.as_ref());
    let stbl = empty_sample_table(
        empty_sample_description(entry),
        uniform_sample_size(template),
    );
    let stbl = match template {
        Some(template) => SampleTableBox {
            children: progressive::stbl_children(Some(template), stbl.children),
        },
        None => stbl,
    };
    track_box(t, stbl)
}

/// The constant `sample_size` a source `stbl` declares on its `stsz`, or 0 when
/// it carries none. A fragmented init's `stsz` holds no entries, but the
/// constant size it states is still part of the box a rewrite reproduces.
fn uniform_sample_size(template: Option<&[StblChild]>) -> u32 {
    template
        .and_then(|children| {
            children.iter().find_map(|child| match child {
                StblChild::Stsz(size) => Some(size.sample_size),
                _ => None,
            })
        })
        .unwrap_or(0)
}

/// One `trak` from its setup and a finished `stbl`.
///
/// The fragmented init and the progressive writer share this: only the sample
/// table differs between them.
pub(crate) fn track_box(t: &TrackSetup, stbl: SampleTableBox) -> Result<TrackBox> {
    if let Some(shell) = &t.shell {
        let mut trak = shell.clone();
        trak.edts = None;
        let minf = trak
            .mdia
            .as_mut()
            .and_then(|mdia| mdia.minf.as_mut())
            .ok_or(Error::UnexpectedBox { expected: "minf" })?;
        minf.stbl = Some(stbl);
        return Ok(trak);
    }
    let minf = track_media_information(&t.handler, stbl, t.minf_header, t.gmhd.clone());
    let mdia = MediaBox::new(
        Some(track_media_header(t.timescale, t.language)),
        Some(track_handler(t)),
        Some(minf),
        vec![],
    );
    let tkhd = track_header(t);
    let mut trak = TrackBox::new(tkhd, None, Some(mdia), vec![]);
    if !t.references.is_empty() {
        let references = crate::track_reference::TrackReferenceBox {
            references: t.references.clone(),
        };
        let bytes = references.try_to_bytes()?;
        if bytes.len() < BOX_HEADER_MIN_SIZE {
            return Err(Error::InvalidInput("tref box is shorter than its header"));
        }
        trak.opaque.push(OpaqueBox {
            box_type: *b"tref",
            data: bytes[BOX_HEADER_MIN_SIZE..].to_vec(),
            to_end: false,
            largesize: false,
        });
        trak.order.push(*b"tkhd");
        trak.order.push_opaque(*b"tref");
        trak.order.push(*b"mdia");
    }
    Ok(trak)
}

/// The sample entry the track's `stsd` carries, protecting it first when the
/// setup asks for CENC.
fn track_entry(t: &TrackSetup) -> Result<SampleEntryVariant> {
    let entry = sample_entry_variant(&t.entry)?;
    match &t.protection {
        Some(p) => crate::init_segment::protect_sample_entry(&entry, p.scheme, &p.tenc),
        None => Ok(entry),
    }
}

/// One caller-supplied sample entry as the typed variant the box tree writes.
///
/// A raw entry keeps its bytes: the `stsd` entry is written back exactly as the
/// caller supplied it, header included (§8.5.2).
pub(crate) fn sample_entry_variant(entry: &SampleEntry) -> Result<SampleEntryVariant> {
    match entry {
        SampleEntry::Typed(v) => Ok(v.clone()),
        SampleEntry::Raw { fourcc, body } => raw_entry_variant(*fourcc, body),
    }
}

/// A raw entry as the generic passthrough variant, after checking that the
/// stored bytes really are that box.
fn raw_entry_variant(fourcc: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Result<SampleEntryVariant> {
    let (item, _) = crate::box_types::parse_box(body)?;
    if !item.header.box_type.is(&fourcc) {
        return Err(Error::InvalidInput(
            "raw sample entry four-CC does not match its box header",
        ));
    }
    let mut opaque = OpaqueBox::new(fourcc, item.payload().to_vec());
    opaque.to_end = item.to_end();
    opaque.largesize = item.has_largesize();
    Ok(SampleEntryVariant::Unknown(opaque))
}

/// An empty `stsd` holding the track's single sample entry: the init segment
/// declares the codec, and every sample arrives through the fragments.
fn empty_sample_description(entry: SampleEntryVariant) -> SampleDescriptionBox {
    SampleDescriptionBox {
        version: entry.required_stsd_version(),
        flags: 0,
        entries: vec![entry],
    }
}

/// The empty `stbl` of a fragmented init: `stsd` plus the four sample tables a
/// fragmenting writer leaves unpopulated (their entries land in `moof`).
fn empty_sample_table(stsd: SampleDescriptionBox, sample_size: u32) -> SampleTableBox {
    SampleTableBox {
        children: vec![
            StblChild::Stsd(stsd),
            StblChild::Stts(TimeToSampleBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
            StblChild::Stsc(SampleToChunkBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
            StblChild::Stsz(SampleSizeBox {
                version: 0,
                flags: 0,
                sample_size,
                sample_count: 0,
                entries: vec![],
            }),
            StblChild::Stco(ChunkOffsetBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
        ],
    }
}

/// The `dinf` of a self-contained track: one `url ` entry with flags bit 0 set
/// (media in this file).
fn self_contained_data_information() -> DataInformationBox {
    DataInformationBox::new(
        Some(DataReferenceBox {
            version: 0,
            flags: 0,
            entries: vec![DataEntry::Url(DataEntryUrlBox {
                version: 0,
                flags: SELF_CONTAINED_DATA_ENTRY_FLAGS,
                location: vec![],
            })],
        }),
        vec![],
    )
}

/// The `minf` of one track: the media header its handler implies, the shared
/// `dinf`, the empty `stbl`, and any header the handler requires ahead of its
/// children.
fn track_media_information(
    handler: &[u8; BOX_TYPE_SIZE],
    stbl: SampleTableBox,
    minf_header: Option<[u8; BOX_TYPE_SIZE]>,
    gmhd: Option<crate::quicktime::GmhdAtom>,
) -> MediaInformationBox {
    let vmhd = (handler == b"vide").then_some(VideoMediaHeaderBox {
        version: 0,
        flags: 1,
        graphicsmode: 0,
        opcolor: [0, 0, 0],
    });
    let smhd = (handler == b"soun").then_some(SoundMediaHeaderBox {
        version: 0,
        flags: 0,
        balance: 0,
        reserved: Default::default(),
    });
    let mut minf = MediaInformationBox::new(
        vmhd,
        smhd,
        Some(self_contained_data_information()),
        Some(stbl),
        vec![],
    );
    minf.gmhd = gmhd;
    // A handler that implies no media header of its own (`meta`, `text`,
    // `tmcd`, …) gets the null one (§8.4.5.2) unless the caller named another
    // or a QuickTime `gmhd` already is the header.
    let header = minf_header.or_else(|| {
        (handler != b"vide" && handler != b"soun" && minf.gmhd.is_none()).then_some(*NMHD)
    });
    if let Some(header) = header {
        minf.set_leading_opaque_header(OpaqueBox::new(header, vec![0u8; FULLBOX_EXTRA_SIZE]));
    }
    minf
}

/// The `mdhd` of a written track: zero timeline (the caller sets the duration
/// afterwards), the track's timescale and language.
fn track_media_header(timescale: Timescale, language: u16) -> MediaHeaderBox {
    MediaHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale,
        duration: 0,
        language,
        quality: 0,
    }
}

/// The `hdlr` naming the track's handler type.
fn track_handler(t: &TrackSetup) -> HandlerBox {
    HandlerBox {
        version: 0,
        flags: 0,
        pre_defined: 0,
        reserved: [0u8; crate::init_segment::HDLR_RESERVED_LEN],
        handler_type: t.handler,
        name: t.handler_name.clone(),
    }
}

/// The `tkhd` of a fragmented init: enabled in movie, zero timeline, and the
/// presentation size and volume the caller supplied.
fn track_header(t: &TrackSetup) -> TrackHeaderBox {
    TrackHeaderBox {
        version: 0,
        flags: TKHD_ENABLED_IN_MOVIE,
        creation_time: 0,
        modification_time: 0,
        track_id: t.track_id,
        reserved: Default::default(),
        duration: 0,
        reserved_2: Default::default(),
        layer: 0,
        alternate_group: 0,
        volume: t.volume,
        reserved_3: Default::default(),
        matrix: IDENTITY_MATRIX,
        width: t.width,
        height: t.height,
    }
}

/// Per-sample CENC auxiliary information, as `senc` carries it
/// (ISO/IEC 23001-7 §12.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SampleAux {
    /// The sample's initialization vector (`Per_Sample_IV_Size` bytes).
    pub initialization_vector: Vec<u8>,
    /// The clear/protected ranges of a subsample-encrypted sample.
    pub subsamples: Vec<SubSampleEntry>,
}

/// One sample's metadata for the fragment's `trun`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleMeta {
    /// `sample_duration`, in the track timescale.
    pub duration: u32,
    /// `pts - dts` composition offset.
    pub composition_offset: i32,
    /// Whether the sample is a sync sample (random-access point).
    pub sync: bool,
    /// CENC per-sample auxiliary information, when the track is protected.
    pub aux: Option<SampleAux>,
    /// One-based `stsd` entry this sample uses (§8.7.4). A fragment states it
    /// once per track fragment (`tfhd.sample_description_index`), so every
    /// sample of one `traf` must share it.
    pub description_index: u32,
}

impl Default for SampleMeta {
    fn default() -> Self {
        Self {
            duration: 0,
            composition_offset: 0,
            sync: false,
            aux: None,
            description_index: TREX_DEFAULT_DESCRIPTION_INDEX,
        }
    }
}

/// One finished fragment: the bytes to emit, the `moof` length inside them,
/// and where the `moof` itself lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    /// `[styp] [emsg]* moof mdat`, in that order.
    pub bytes: Vec<u8>,
    /// The length of the `moof` box inside [`Fragment::bytes`].
    pub moof_len: usize,
    /// The absolute file offset of the `moof` box, as
    /// [`FragmentOptions::position`] places it.
    pub moof_offset: u64,
}

/// One sample handed to [`fragmented_segment`].
#[derive(Debug, Clone, Copy)]
pub struct SampleData<'a> {
    /// The sample's timing and flags.
    pub meta: &'a SampleMeta,
    /// The coded sample bytes.
    pub data: &'a [u8],
}

/// One track's samples for a single fragment.
#[derive(Debug, Clone, Copy)]
pub struct FragmentTrack<'a> {
    /// The `traf.tfhd.track_ID`.
    pub track_id: TrackId,
    /// The first sample's decode time (`tfdt`).
    pub base_decode_time: u64,
    /// The samples, in decode order.
    pub samples: &'a [SampleData<'a>],
    /// The track's `trex` defaults (`§8.8.3`): what a `tfhd`/`trun` field the
    /// layout omits inherits when the `tfhd` states no default of its own.
    pub trex: TrexDefaults,
}

/// The segment-level choices of one fragment.
#[derive(Debug, Clone, Copy)]
pub struct FragmentOptions<'a> {
    /// `mfhd.sequence_number`.
    pub sequence_number: u32,
    /// The `styp` brands to emit; `None` omits `styp`.
    pub styp: Option<&'a SegmentBrands>,
    /// Event Message Boxes, emitted after `styp` and before `moof`.
    pub events: &'a [EmsgBox<'a>],
    /// The source layout to reproduce; `None` writes this crate's canonical
    /// per-sample form.
    pub layout: Option<&'a crate::frag_layout::FragmentLayout>,
    /// The absolute file offset at which the emitted bytes will be placed, so
    /// a layout that addresses its runs from an explicit `base_data_offset`
    /// can state it.
    pub position: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw entry the wire encoded with the `size == 0` form ("to the end of
    /// the enclosing container", §4.2) is written back with the same form:
    /// spelling the length out instead changes the entry's bytes (fuzz finding
    /// on `isobmff_whole_file`).
    #[test]
    fn a_to_end_raw_entry_keeps_the_size_zero_form() {
        let body = [0u8, 0, 0, 0, b'a', b'c', b'm', b'e', 0xde, 0xad, 0xbe, 0xef];
        let entry = raw_entry_variant(*b"acme", &body).expect("raw entry");
        let bytes = entry.try_to_bytes().expect("bytes");
        assert_eq!(bytes, body);
        assert_eq!(bytes[0..4], [0, 0, 0, 0]);
    }

    /// The same box with an explicit length keeps it.
    #[test]
    fn an_explicit_length_raw_entry_is_unchanged() {
        let body = [
            0u8, 0, 0, 12, b'a', b'c', b'm', b'e', 0xde, 0xad, 0xbe, 0xef,
        ];
        let entry = raw_entry_variant(*b"acme", &body).expect("raw entry");
        assert_eq!(entry.try_to_bytes().expect("bytes"), body);
    }
}
