# Display geometry metadata in transmux: design

Status: **design only, awaiting owner review.** No production code accompanies
this document. Spec facts are transcribed and page-checked in
[`SPEC-NOTES.md`](SPEC-NOTES.md); this document cites that file as `SN §n`.

Requester: an integrator building an Apple TV player on `transmux` (and
`media-plane`/`multimux` behind it). Owner direction: "we have a lot of useful
notification and metadata plumbing already, so don't bolt on a field". This
design therefore does **not** add a lone `sar` field to `CodecConfig::Avc`; it
gives display geometry one typed model and routes it through the three
existing paths (`TrackSpec`, `DemuxEvent`, `Sample`), the same way
`TrackUpdated`, `ClockReference`/`EventProvenance` and
`Media::degraded_at`/`DegradationRecord` already work. It subsumes the narrow
"just surface SAR" change, which is not being implemented separately.

## 1. What the player needs (requirements)

| # | Requirement | Source |
|---|---|---|
| R1 | Show each picture at its correct shape using the SAR from the SPS VUI (H.264 Table E-1 idc 1..16 and 255 Extended_SAR; H.265 Table E.1 likewise). | integrator |
| R2 | Detect a change mid-stream (SD channels switch 16:9 <-> 4:3 programmes) and respond from the **first affected picture**; pictures decoded before keep the old shape. | integrator |
| R3 | The change notification is delivered **before the first Sample of the affected access unit** and carries **that AU's DTS and PTS** (the player stamps frames at decode-submit in DTS order; PTS is for logging/asserts). | integrator answer (1) |
| R4 | State whether a geometry change can take effect mid-GOP; say what a violating stream does. The player assumes RAP-only. | integrator answer (2) |
| R5 | A re-sent identical SPS emits nothing. An SPS that changes without changing geometry: pick one behaviour and document it. | integrator answer (3) |
| R6 | AFD needs a solid design: many broadcasters keep the SAR fixed and signal 4:3 programmes with AFD alone. | integrator answer (4) |
| R7 | Cheap to read per chunk; travels the same path as `TrackUpdated` / `Media::degraded_at` / `ClockReference`. | integrator, owner |
| R8 | The player consumes `TrackInfo.pixelAspect` (static) + a per-frame PAR, is already built against a synthetic change, and waits for Phase 1. | integrator answer (5) |

## 2. What transmux does today (verified in the code at `origin/main` 514e93fec)

- `transmux/src/sps.rs` walks the VUI only to reach `timing_info`:
  `parse_avc_vui_inner` (line ~435) and `parse_hevc_sps_to_vui_timing_inner`
  (line ~995) read `aspect_ratio_idc`, `sar_width`, `sar_height`,
  `video_format`, `video_full_range_flag`, `colour_primaries`,
  `transfer_characteristics`, `matrix_coefficients`, and (HEVC)
  `def_disp_win_*_offset` and **discard every one of them** (`let _ = ...`).
  The crop offsets are applied to `width`/`height` and then discarded too.
- `CodecConfig::Avc`/`Hevc` carry `config`, `width: u16`, `height: u16` only.
  These are struct variants of a `#[non_exhaustive]` enum, but the **variants
  themselves are not** `#[non_exhaustive]`: `multimux` constructs them with
  struct literals in production code (`multimux/src/source/ts_program.rs:470`,
  `multimux/src/source/whip.rs:1429`) and ~100 test sites do. Adding a field to
  a variant is therefore a **breaking** change (see §9).
- `TrackSpec` **is** `#[non_exhaustive]` (`transmux/src/ir/track.rs`). Adding a
  field is additive. Same for `Sample`, `Media`, `EventProvenance`,
  `DemuxEvent` (adding a variant is additive).
- `StreamingTsDemux` already re-probes a live track when its in-band parameter
  sets change and emits `DemuxEvent::TrackUpdated` (r04-W51, `advance_track`,
  `ts_demux.rs` ~line 3520). Identical re-sends emit nothing (`ParameterSets`:
  SPS/VPS single "current" slot compared byte-for-byte, PPS keyed by id).
- **Existing ordering defect (found while designing this).** Video samples are
  emitted one-behind (`advance_one_behind`, `ts_demux.rs` ~line 2041: the
  pending AU N-1 is emitted when AU N arrives). `advance_track` pushes
  `TrackUpdated` *before* calling `push_live_au` for AU N, so the event lands
  **before the Sample of AU N-1** - the last picture of the old configuration.
  A consumer that applies `TrackUpdated` on receipt mislabels exactly one
  frame. The test `a_changed_sps_raises_exactly_one_update`
  (`ts_demux.rs` ~line 8760) only counts events, so it cannot see this. R3
  cannot be met without fixing it; the fix is part of Phase 1 (§8).
- Batch `TsDemux::demux` folds `TrackUpdated` into `tracks[i].spec = spec`
  (last one wins, `ts_demux.rs` ~line 5917): samples collected under an earlier
  config lose that config. Per-sample tagging (§5.3) is the only way a batch
  `Media` can say which picture had which shape.
- fMP4: `codec_config_from_entry` (`transmux/src/media.rs` ~line 780) takes
  width/height from the `VisualSampleEntry` and drops the entry's sibling
  boxes; `AVCSampleEntry`/`HEVCSampleEntry` capture `pasp`/`colr`/`clap` as
  opaque `extra_boxes`, but `CodecConfig` never carries them, so they are lost
  in the IR. `tfhd.sample_description_index` is parsed but a track has one
  `CodecConfig`. Typed `PixelAspectRatioBox` / `CleanApertureBox` /
  `ColourInformationBox` exist (`transmux/src/visual_ext.rs`) but no mux or
  demux path uses them; `pipeline::build_trak` writes `extra_boxes: vec![]`
  and `tkhd` width/height = coded width/height.
- `transmux/src/nal.rs` already has an SEI message walker (`sei_messages`) and
  a T.35 header matcher for GA94 captions (`append_if_atsc_cc_data`: country
  0xB5, provider 0x0031, user_identifier). AFD is the sibling `DTG1` payload
  (SN §5); no new SEI framing is needed.
- `media-plane` stores `transmux::Sample` in the `Trunk` sample ring and
  translates track changes into a full `TracksChanged { tracks: Vec<TrackSpec> }`
  snapshot (`media-plane/src/ingress.rs`). It does not mirror `TrackUpdated`.
  Anything on `Sample` or `TrackSpec` reaches egress with no media-plane change.

## 3. Scope decision

| Signal | In scope? | Phase | Why |
|---|---|---|---|
| SAR (H.264/H.265 VUI `aspect_ratio_idc`, Extended_SAR) | **yes** | 1 | R1; two real fixtures (16:11 table path, 64:45 extended path). |
| Output crop (H.264 frame cropping, H.265 conformance window) | **yes** | 1 | Needed to compute the displayed shape (TS 101 154 §B.3.2: AFD is interpreted against "picture width, picture height, frame cropping information, and sample aspect ratio"). Already parsed, just discarded. |
| H.265 default display window | **yes** (carried, not applied) | 1 | One more read in an already-walked syntax; semantically a *suggestion* (SN §3), so it is exposed separately, never folded into the crop. |
| `video_signal_type`: video_format, full range, colour_primaries / transfer / matrix | **yes** | 1 | Already walked; full range is a real rendering bug if ignored (real full-range fixture exists); HLS `VIDEO-RANGE` needs transfer (Phase 4). |
| `overscan_appropriate_flag` | yes (carried) | 1 | One bit in the same walk; explains Table E-1's "with horizontal overscan" rows. |
| AFD (`afd_data()` 'DTG1') in AVC/HEVC T.35 SEI | **yes** | 3 | R6. Real fixtures carry it (only value 8 on H.264). |
| AFD in MPEG-2 picture user_data | **yes** | 3 | Same `afd_data()` payload, different wrapper; it is the only carriage with *real* non-trivial AFD values (10, 14) in our fixtures, so it is what lets Phase 3 prove more than one code on real data. |
| Bar data (`GA94` + `user_data_type_code` 0x06) | yes | 3 | Same message family; AFD 0000/0100 are defined *with* bar data (SN §5). Spec-vector only (no fixture). |
| MPEG-2 sequence-level geometry (`aspect_ratio_information` DAR, `sequence_display_extension`) | optional | 1b | Real 64:45 MPEG-2 fixtures exist, but the Apple TV player decodes H.264/H.265; H.262 md exists (`itu_t_h262_mpeg2_video.md`) but is not transcribed for this. Owner question Q6. |
| DVB component descriptor (EN 300 468 Table 26) | **no** (document the consumer path) | - | Lives in SDT/EIT, "editorial characteristics of the event or service ... to inform the user" (SN §8); event-granular, cannot be frame-accurate. `dvb-si` already parses it. |
| HDR SEI (mastering display, content light level, alternative transfer characteristics) | **no** (separate design) | - | Different consumer (tone mapping) and persistence rules. Note: real fixture tsduck `test-010` signals HLG via the alternative-transfer SEI (`preferred_transfer_characteristics` 18 over VUI 14); Phase 4's `VIDEO-RANGE` must not claim SDR from the VUI alone without this. |
| Pan-scan SEI, WSS (EN 300 294), VVC/AV1/VP9 geometry | no | - | Not requested; fields stay `None` for these codecs; the types are codec-neutral so they can be filled later additively. |

## 4. Data model (doc-level API sketch)

New module `transmux::ir::display`, re-exported at the crate root. All types
are `#[non_exhaustive]`; every public enum gets `name()` + 
`broadcast_common::impl_spec_display!` (#204). No raw byte arrays in the public
API (no-raw-byte rule); every wire code point is an enum with a lossless
catch-all.

```rust
/// H.264 Table E-1 / H.265 Table E.1, wire form kept losslessly.
#[non_exhaustive]
pub enum AspectRatioIdc {
    Unspecified,                       // 0
    Square,                            // 1  (1:1)
    Sar12x11, Sar10x11, Sar16x11, Sar40x33, Sar24x11, Sar20x11,
    Sar32x11, Sar80x33, Sar18x11, Sar15x11, Sar64x33, Sar160x99,
    Sar4x3, Sar3x2, Sar2x1,            // 2..=16
    Reserved(u8),                      // 17..=254
    ExtendedSar { sar_width: u16, sar_height: u16 }, // 255
}
impl AspectRatioIdc {
    pub fn from_wire(idc: u8, extended: Option<(u16, u16)>) -> Self;
    pub fn idc(&self) -> u8;                         // back to the wire value
    /// `None` for 0, for 17..=254 (H.265 E.3.1 "interpret as 0", applied to
    /// both codecs), and for Extended_SAR with a zero component (E.2.1/E.3.1).
    pub fn sample_aspect_ratio(&self) -> Option<SampleAspectRatio>;
    pub fn name(&self) -> &'static str;              // "1:1", "16:11", "Extended_SAR", "reserved", "unspecified"
}

/// A *specified* sample (pixel) aspect ratio: both terms non-zero.
/// Unspecified is `Option::None` at every use site, never a 0:0 sentinel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SampleAspectRatio { width: NonZeroU16, height: NonZeroU16 }
impl SampleAspectRatio {
    pub fn new(width: u16, height: u16) -> Option<Self>;
    pub fn width(&self) -> u16;  pub fn height(&self) -> u16;
    /// Reduced form. The spec requires sar_width/height to be relatively
    /// prime already; a non-reduced wire pair is kept as-is in `AspectRatioIdc`
    /// and only reduced here.
    pub fn reduced(&self) -> Self;
}

/// A rectangle in luma samples of the decoded picture, top-left origin.
pub struct PictureRect { pub left: u32, pub top: u32, pub width: u32, pub height: u32 }

pub enum VideoFormat { Component, Pal, Ntsc, Secam, Mac, Unspecified, Reserved(u8) } // Table E-2
pub enum ColourPrimaries          { /* Table E-3 / E.3 rows */ Reserved(u8) }
pub enum TransferCharacteristics  { /* Table E-4 / E.4 rows */ Reserved(u8) }
pub enum MatrixCoefficients       { /* Table E-5 / E.5 rows */ Reserved(u8) }

pub struct ColourDescription {
    pub primaries: ColourPrimaries,
    pub transfer: TransferCharacteristics,
    pub matrix: MatrixCoefficients,
}
pub struct VideoSignalType {
    pub video_format: VideoFormat,
    pub full_range: bool,                    // video_full_range_flag
    pub colour: Option<ColourDescription>,   // None: colour_description_present_flag == 0
}

/// Sequence-level display geometry: everything that can only change when a
/// new SPS becomes active (SN §4). Lives on `TrackSpec`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayGeometry {
    /// Decoded picture size before cropping (H.264 PicWidthInSamplesL x
    /// 16*FrameHeightInMbs; H.265 pic_width/height_in_luma_samples). u32, not
    /// the u16 `CodecConfig` uses.
    pub coded_width: u32,
    pub coded_height: u32,
    /// What the decoder outputs: H.264 frame-cropping rectangle / H.265
    /// conformance window (SN §3). Equals the full picture when no cropping.
    pub output_rect: PictureRect,
    /// H.265 default display window (absolute, i.e. conformance offsets
    /// already added per E-78..E-81). Always `None` for H.264. A suggestion,
    /// never applied to `output_rect`.
    pub default_display_rect: Option<PictureRect>,
    /// `None` when aspect_ratio_info_present_flag == 0 (or VUI absent).
    pub aspect_ratio: Option<AspectRatioIdc>,
    /// `None` when overscan_info_present_flag == 0.
    pub overscan_appropriate: Option<bool>,
    /// `None` when video_signal_type_present_flag == 0. The spec's inferred
    /// defaults (format 5, limited range, colour 2/2/2) are available through
    /// accessors, but the struct records what was *signalled*.
    pub video_signal: Option<VideoSignalType>,
    /// Where these values came from (codec VUI, or a container box that the
    /// spec says overrides it - see §6.3).
    pub source: GeometrySource,
}
impl DisplayGeometry {
    pub fn from_avc_sps(nal: &[u8]) -> Result<Self>;   // NAL incl. header, same input as decode_avc_sps
    pub fn from_hevc_sps(nal: &[u8]) -> Result<Self>;
    /// Derived from the first SPS in an avcC/hvcC; `None` for other codecs.
    pub fn from_codec_config(config: &CodecConfig) -> Option<Self>;
    pub fn sample_aspect_ratio(&self) -> Option<SampleAspectRatio>;
    /// Display aspect of `output_rect` under the SAR, reduced
    /// (width*sar_w : height*sar_h); `None` when the SAR is unspecified.
    pub fn display_aspect_ratio(&self) -> Option<(u32, u32)>;
    pub fn full_range(&self) -> bool;                  // inferred 0 when absent
}
pub enum GeometrySource { CodecParameterSet, SampleEntry /* pasp/colr overlay */ }

/// ETSI TS 101 154 Table B.5 (SN §5), wire form kept losslessly.
#[non_exhaustive]
pub enum ActiveFormatDescription {
    Unknown,                         // 0000 "AFD unknown"
    Box16x9Top,                      // 0010
    Box14x9Top,                      // 0011
    BoxWiderThan16x9Centre,          // 0100
    SameAsCodedFrame,                // 1000
    Centre4x3,                       // 1001
    Centre16x9,                      // 1010
    Centre14x9,                      // 1011
    Centre4x3ShootProtect14x9,       // 1101
    Centre16x9ShootProtect14x9,      // 1110
    Centre16x9ShootProtect4x3,       // 1111
    Reserved(u8),                    // 0001, 0101..0111, 1100
}

/// TS 101 154 Table B.7. Only the signalled pair is `Some`; the 14-bit values
/// are kept as wire values (Table B.8 line numbering not yet transcribed).
pub struct BarData {
    pub top_bar_end_line: Option<u16>,
    pub bottom_bar_start_line: Option<u16>,
    pub left_bar_end_pixel: Option<u16>,
    pub right_bar_start_pixel: Option<u16>,
}

pub enum UserDataCarriage { AvcSei, HevcPrefixSei, Mpeg2PictureUserData }

/// Picture-level "area of interest" state in force for an access unit.
#[non_exhaustive]
pub struct ActiveFormat {
    /// `None` when the last afd_data() had active_format_flag == 0, or no AFD
    /// has been seen.
    pub afd: Option<ActiveFormatDescription>,
    pub bar_data: Option<BarData>,
    pub carriage: UserDataCarriage,
}

/// Everything that determines how one access unit is shown. Shared, never
/// copied per sample: `Arc<DisplayInfo>`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub geometry: Option<DisplayGeometry>,
    pub active_format: Option<ActiveFormat>,
}

/// Which parts of `DisplayInfo` differ from the previous state.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayChangeSet {
    pub initial: bool,            // first state for this track
    pub aspect_ratio: bool,
    pub output_rect: bool,        // includes coded size
    pub default_display_rect: bool,
    pub video_signal: bool,
    pub overscan: bool,
    pub afd: bool,
    pub bar_data: bool,
}
```

Additions to existing types (all additive, §9):

```rust
// TrackSpec (#[non_exhaustive])
pub display: Option<DisplayGeometry>,      // static/sequence level, see §5.1
pub fn with_display(self, d: Option<DisplayGeometry>) -> Self;

// Sample (#[non_exhaustive])
pub display: Option<Arc<DisplayInfo>>,     // per-AU, see §5.3
pub fn with_display(self, d: Arc<DisplayInfo>) -> Self;

// DemuxEvent (#[non_exhaustive])
#[non_exhaustive]
DisplayChanged {
    track_id: TrackId,
    display: Arc<DisplayInfo>,             // full new state, not a delta
    changed: DisplayChangeSet,
    dts: Option<i64>,                      // the affected AU's absolute DTS (track timescale)
    pts: Option<i64>,                      // the affected AU's absolute PTS
    random_access: bool,                   // the affected AU's is_sync
    provenance: EventProvenance,           // pid; packet_index when known
},
pub fn display_changed(...) -> Self;       // constructor, like clock_reference()

// sps.rs (unchanged AvcSpsInfo / HevcSpsInfo: neither is #[non_exhaustive],
// so a field added there would be breaking; geometry is exposed through
// DisplayGeometry::from_{avc,hevc}_sps instead, sharing the one VUI walk).
```

`DisplayInfo` is behind `Arc` (`alloc::sync::Arc`, available in this
`no_std + alloc` crate): one allocation per *change*, a refcount bump per
sample. Reading the per-frame PAR is `sample.display.as_deref()
.and_then(|d| d.geometry.as_ref()).and_then(|g| g.sample_aspect_ratio())` -
no lookup table, no event replay (R7).

## 5. The three surfaces and how they relate

### 5.1 Static: `TrackSpec::display`

- The sequence-level geometry of the track's **current codec config**: what
  `TrackAdded` announces, what each `TrackUpdated` re-announces, what
  `media-plane`'s `TracksChanged` snapshot carries, what an init-segment muxer
  writes (Phase 4). This is the player's `TrackInfo.pixelAspect` (R8).
- Derived in **one place**: `TrackSpec::new` calls
  `DisplayGeometry::from_codec_config(&config)` (decode of the first SPS in the
  avcC/hvcC record). Every demuxer that builds a `TrackSpec` (TS, fMP4, FLV,
  RTP, PS, WebM, HLS-pull, the multimux sources) is covered without per-path
  edits; a demuxer with better information (fMP4 `colr`, §6.3) overrides with
  `with_display`. A placeholder config with no SPS (multimux WHIP) yields
  `None`, never a fabricated value.
- AFD is **not** here: it is picture-level and changes without a codec-config
  change; putting it on `TrackSpec` would force a `TrackUpdated` (= "rebuild
  your init segment") on every AFD flip.

### 5.2 Change notification: `DemuxEvent::DisplayChanged`

- Carries the full new `DisplayInfo` (like `TrackUpdated` carries the full
  `TrackSpec`), the change set, and the affected AU's DTS/PTS/sync flag and
  provenance - the same shape `ClockReference`/`Discontinuity` use.
- **Level-triggered start:** one `DisplayChanged { changed.initial = true }` is
  emitted for every video track that has any geometry or AFD, after its
  `TrackAdded` and before its first `Sample`. A consumer therefore has exactly
  one code path (apply `DisplayChanged`), never "read TrackAdded, then also
  watch events". (Owner question Q2.)
- Does not bump `TracksResolved::generation` (the track set did not change).

### 5.3 Per-AU tag: `Sample::display`

- Every video `Sample` from a demuxer that implements Phase 2 carries
  `Some(Arc<DisplayInfo>)`: the state in force **for that access unit**. The
  first sample after a change carries the *same* `Arc` as the
  `DisplayChanged` that preceded it (`Arc::ptr_eq`).
- Why on the sample and not only an event: (a) the batch `Media` path folds
  `TrackUpdated` into a last-wins spec and has no event list - per-sample is
  the only lossless form there; (b) IR transforms (rebase, splice, trick-play)
  and segmenters drop, reorder and regroup samples - a tag on the sample
  survives that, an index into an event list does not; (c) `media-plane`
  carries `Sample` through the `Trunk` unchanged, so egress gets per-AU
  geometry with no new `SessionEvent` variant.
- `None` means "this demuxer does not track display state", never "unchanged".

## 6. Behaviour

### 6.1 Change detection (R5)

| Situation | `TrackUpdated` | `DisplayChanged` |
|---|---|---|
| Identical SPS/PPS re-sent (every IDR, routine) | no | no |
| SPS bytes change, parsed `DisplayGeometry` equal (e.g. level/HRD/timing edit) | **yes** (existing byte rule, unchanged) | **no** |
| SPS bytes change and `DisplayGeometry` differs (e.g. SAR 16:11 -> 12:11) | yes | yes (`changed` names the fields) |
| Different SPS **id** activated, same geometry | yes if the config record changed | no |
| AFD / bar data value changes in user data | no | yes |
| AFD repeated with the same value (every picture, as on gulli) | no | no |

So for R5's second case the answer is: **`TrackUpdated` without a geometry
event.** Geometry change detection compares the parsed `DisplayGeometry`
(`PartialEq`), never bytes; the codec-config rule stays byte-driven because an
init segment must be rebuilt for *any* config change. A consumer that only
cares about shape listens to `DisplayChanged` alone.

### 6.2 Activation: which AU a change belongs to (R2, R4)

- **Spec rule (SN §4).** In a conforming stream an SPS-derived change takes
  effect only at an **IDR access unit** (H.264: an active SPS's content may only
  change between coded video sequences; SPS activation by buffering-period SEI
  only in an IDR AU) or an **IRAP AU with NoRaslOutputFlag = 1** (H.265: IDR,
  BLA, or a CRA that is first in the bitstream / follows an end-of-sequence NAL).
  Broadcast H.264 open-GOP recovery-point I-pictures (issue #595) are *not*
  IDRs and cannot carry an SPS content change. For H.265, RASL pictures that
  follow such an IRAP in decode order are not output by the decoder.
- **What transmux does.** The geometry state is advanced at the AU whose
  slices first *reference* the changed SPS (slice header `pic_parameter_set_id`
  -> PPS `seq_parameter_set_id` -> SPS; `slice_header.rs` already parses the
  H.264 slice header with `AvcParamSets`; H.265 needs
  `slice_pic_parameter_set_id`). It does **not** use "the AU that carries the
  SPS NAL" (an encoder may legally send an SPS with a *new id* ahead of its
  use) and does **not** track a single "current SPS" slot (today's
  `ParameterSets.sps.current`, which would raise the change early in that case).
- **Contract for R4.** For SPS-derived fields, a conforming stream only ever
  produces `DisplayChanged` with `random_access == true` on an IDR/IRAP AU. A
  non-conforming stream that changes an active SPS's content mid-CVS gets the
  event **at the AU whose slices first reference the new content** (where a
  decoder actually starts using it) with `random_access == false`; transmux
  does not defer it to the next RAP, because that would mislabel pictures
  already decoded with the new SPS. (Owner question Q3: also raise an
  `InputDegraded`?)
- **AFD is not RAP-bound.** AFD/bar data are picture-level user data and may
  change on **any** picture, including mid-GOP non-reference pictures. The
  player's RAP-only assumption holds for SAR/crop/colour, **not** for AFD; it
  must handle `DisplayChanged { changed.afd, random_access: false }`.
- **Decode order.** The event and the tag follow decode order (the order
  samples are emitted), matching the player's DTS-order stamping. For AFD
  inside a GOP with B-pictures, "the AU that carried the new AFD" and "every AU
  after it in decode order" get the new state. Whether AFD persistence is
  meant in decode or output order is not stated by any spec available here
  (open question Q4).

### 6.3 Precedence when several sources disagree

1. ISO BMFF `colr` (`nclx`) over the VUI colour description:
   "this box takes precedence, and over-rides the information in the
   bitstream" (ISO/IEC 14496-12 §12.1.5.1, SN §6). `source = SampleEntry`.
2. `pasp` vs VUI SAR: **unknown.** The §12.1.4.1 definition text that would say
   it is not transcribed (gap G3). Until it is, the VUI SAR is what
   `aspect_ratio` reports and a `pasp` is not read into the model at all;
   Phase 4 settles precedence after the transcription (owner question Q7).
3. AFD never overrides geometry: it is interpreted *against* it (TS 101 154
   §B.3.0 "The AFD itself does not describe the aspect ratio of the coded
   frame"). `DisplayInfo` carries both; the player composes them.
4. EN 300 468 component descriptor: never merged (out of scope, §3).

### 6.4 AFD state machine (R6)

- **Where it is read.**
  H.264: SEI NAL type 6, `payloadType` 4 (`user_data_registered_itu_t_t35`),
  payload `0xB5 0x00 0x31 'DTG1' afd_data()` (TS 101 154 Table B.11 + B.2).
  H.265: **prefix** SEI only (TS 101 154 §B.8a "shall be a prefix SEI
  message"); a suffix-SEI copy is ignored.
  MPEG-2: picture `user_data()` `0x000001B2 'DTG1' afd_data()` after the
  picture header (Table B.10).
  Bar data: same wrappers, `'GA94'` + `user_data_type_code` 0x06 + Table B.7.
  Reuses `nal::sei_messages` and the T.35 header constants already in
  `nal.rs` (move them to a shared `user_data` module; no second SEI walker).
- **Parse policy.** `active_format_flag == 0` => `afd = None` for this picture.
  The fixed '0' bit and the reserved `'00 0001'` / `'1111'` patterns are not
  enforced (TS 101 154 §B.6.2: receivers "silently discard any unrecognized
  video user data"); a payload shorter than the structure is ignored, not an
  error. More than one AFD in one AU: the first wins (MPEG-2 allows at most one
  per picture header per §B.6.2; AVC has no such rule in the available text).
- **Granularity.** Per access unit (per picture). For PAFF field pairs merged
  into one sample (issue #1161) an AFD on the second field applies from that
  merged sample on (the sample is the AU unit transmux exposes).
- **Persistence.**
  MPEG-2 (spec, SN §5): in effect "until the next sequence_header() or until a
  new AFD is introduced"; after a sequence_header() with no AFD the area of
  interest reverts to the coded frame => `afd = None` from that picture.
  H.264/H.265: **no rule in the available specs** (TS 101 154 §B.7/§B.8a define
  carriage only; ATSC A/72 Part 1 and SMPTE ST 2016-1 are not transcribed, gap
  G1/G2). Proposed default, explicitly marked implementation-defined pending
  those documents: **sticky until a new AFD**, with no reset at an IDR. Rationale:
  a reset-at-IDR guess would flap the player to "unknown" on every IDR of a
  stream that sends AFD only on its first picture; a sticky value cannot flap.
  The alternative (reset at a new CVS, the MPEG-2 analogue) is owner question Q5.
- **Bar data persistence.** MPEG-2: until next sequence_header(), new bar data,
  or a new AFD (§B.6.1). H.264/H.265: same gap; same sticky default, except that
  a new AFD without bar data clears bar data (mirrors §B.6.1 item 3).

### 6.5 Event ordering contract (R3)

For one track T and the AU N whose display state differs from AU N-1's, a
consumer draining `poll_event` sees, in this order:

1. every `Sample` of T for AUs before N in decode order (for PAFF, the merged
   field pair counts as one AU);
2. `TrackUpdated(T)` - only if T's codec config bytes changed at N;
3. `DisplayChanged { track_id: T, dts: N.dts, pts: N.pts, random_access:
   N.is_sync, .. }` - only if the parsed `DisplayInfo` changed at N;
4. `Sample` of T for AU N, with `display` `Arc::ptr_eq` to step 3's.

Guarantees: steps 2-4 are adjacent for T (no other T event between them);
`DisplayChanged.dts/pts` are byte-for-byte the following sample's `dts/pts`
(already unwrapped absolute ticks - no new 33-bit arithmetic; `clock33_guard`
unaffected). No guarantee relative to other tracks or to
`ClockReference`/section samples (the existing per-emission-class rule in
`DemuxEvent`'s docs still applies). During parked-backlog replay the same
ordering holds within the replayed burst. `TrackRemoved` is never followed by
a `DisplayChanged` for that track.

Implementation note: the one-behind slot (`PendingOneBehind`) gains the
`Arc<DisplayInfo>` and any pending `TrackUpdated`/`DisplayChanged` for its AU;
they are pushed when that slot is *emitted*, not when the next AU arrives.
This is also the fix for the existing `TrackUpdated` ordering defect (§2).

## 7. Per-container behaviour

| Path | Static (`TrackSpec.display`) | Change event + per-AU tag |
|---|---|---|
| `StreamingTsDemux` (AVC/HEVC) | Phase 1, from the recovered avcC/hvcC via `TrackSpec::new` | Phase 2 (SPS), Phase 3 (AFD) |
| `StreamingTsDemux` (MPEG-2 video) | Phase 1b if approved (Q6) | Phase 3 AFD; Phase 1b sequence geometry |
| Batch `TsDemux` -> `Media` | same (final spec, last wins) | per-AU tags only (no event list in `Media`) |
| `Fmp4Demux` (avc1/hvc1: params out of band) | Phase 1 from avcC/hvcC; `colr` overlay Phase 1, `pasp` Phase 4 (Q7) | a change needs a new sample entry = new init = new `Media`; no in-segment change possible |
| `Fmp4Demux` (avc3/hev1: in-band params) | Phase 1 | Phase 2b: scan in-band SPS in samples, tag samples. `tfhd.sample_description_index` (multiple entries) stays unsupported (existing gap) |
| `StreamingFlvDemux` (RTMP) | Phase 1 via `TrackSpec::new` on each sequence header | Phase 2 must verify FLV's `TrackUpdated` is not also one sample early (not checked here) |
| `rtp_stream` | Phase 1 via `TrackSpec::new` | Phase 2 audit, same check |
| `hls-runtime` client | inherits `Fmp4Demux` / TS demux | inherits |
| `media-plane` / `multimux` | `TracksChanged` snapshot carries `TrackSpec.display` | `Sample.display` flows through the `Trunk` unchanged |

H.265 VPS: carries no display syntax; a VPS-only change never raises
`DisplayChanged`. H.264 subset SPS (MVC/SVC) and H.265 multi-layer SPS
extensions are ignored (base layer only).

## 8. Phased plan

Each phase is independently releasable, additive to `transmux` within the
0.27 line (repo practice: additive-on-`#[non_exhaustive]` ships as a 0.x patch,
e.g. 0.27.4 added `EventProvenance::program_number`), and changes no sibling's
caret epoch, so **no cascade** (§9).

### Phase 1 - static geometry (the player waits for this)

Scope: transcribe H.264 Tables E-2..E-5 / H.265 E.2..E.5 into
`transmux/docs/codec/` (page-checked); `ir::display` types (§4) minus AFD;
`DisplayGeometry::from_{avc,hevc}_sps` sharing the existing VUI walks (stop
discarding the values; crop rect from the CropUnitX/Y derivation already in
`decode_avc_sps_inner`); `from_codec_config`; `TrackSpec.display` auto-derived
in `TrackSpec::new`; fMP4 `colr nclx` overlay; **the `TrackUpdated` one-behind
ordering fix** (§2, §6.5) as a `### Fixed` entry; private submodule gitlink
bumped to a commit that contains the zenith-tv fixtures (`15931b6` or later).

Semver: additive -> transmux 0.27.x. Behaviour change: `TrackUpdated` moves
one sample later (bug fix; Q1).

Acceptance tests (each must fail under the named mutation; re-run the mutation,
prove recompilation, per the repo's mutation discipline):

| Test | Real data | Oracle (independent) | Mutation it kills |
|---|---|---|---|
| melodie: `TrackAdded.display.aspect_ratio == ExtendedSar{64,45}`, SAR 64:45, `video_signal` = format 5, limited, colour 5/5/5; output 720x576 | `private/fixtures/ts/melodie-anamorphic-sar-64-45-zenith-tv.ts` | ffmpeg `trace_headers`: idc 255, sar 64/45, video_format 5, full_range 0, primaries/transfer/matrix 5 | swap sar_width/sar_height reads; drop the Extended_SAR branch; drop colour read |
| krone: `aspect_ratio == Sar16x11` (idc 4), `video_signal == None` | `private/fixtures/ts/krone-anamorphic-sar-16-11-zenith-tv.ts` | trace_headers idc 4, no video_signal_type | off-by-one in the idc table; treat absent VST as inferred-present |
| two tracks, two SARs in one mux: 1440x1080 `Sar4x3` (idc 14) and 720x576 `Sar16x11` | tsduck `test-092.ts` (already mirrored in `private/fixtures/tsduck/input/`) | ffprobe per stream + trace_headers | geometry keyed to the wrong PID (>= 2 entities rule) |
| crop, two CropUnitY values: 1920x1080 from 1088 rows via `frame_crop_bottom_offset` 4 (high_1080_cropped) and via offset 2 (transformers, 1080 reported by ffprobe => CropUnitY 4, i.e. field-capable) | public `fixtures/ts/h264/high_1080_cropped.ts`; `private/fixtures/ts/transformers-eia608-h264.ts` | trace_headers offsets + ffprobe height; confirm `frame_mbs_only_flag` in the test's own oracle note | CropUnitY without the `2 - frame_mbs_only` factor (one of the two files must then fail) |
| full range: `full_range() == true` | `private/fixtures/ts/transformers-eia608-h264.ts` | trace_headers video_full_range_flag 1 | ignore the flag |
| HEVC colour: primaries 9, transfer 16 (PQ), limited | tsduck `test-074.hevc.ts` (must be mirrored into `private/fixtures/tsduck/`, BSD licence per the tsduck PROVENANCE) | trace_headers | HEVC walk reading the H.264 field order |
| HEVC SAR / conformance window / default display window | **none real** | spec vectors from SN §1/§3, plus a derived fixture: real HEVC re-muxed with ffmpeg `hevc_metadata` (`sample_aspect_ratio=`, `crop_*`), provenance recorded | sign/offset errors in E-78..E-81 |
| ordering: on `h264-two-resolutions.ts` the `TrackUpdated` arrives after the last 320x240 Sample and before the first 640x480 one | public `transmux/tests/fixtures/ts/h264-two-resolutions.ts` | its README oracle (keyframe DTS 306000) | move the push back before `push_live_au` |
| edge vectors: idc 0, 17, 254, Extended 0:45, 64:0, crop beyond picture, VUI truncated after SAR | spec vectors (inline, `#[cfg(test)]`) | SN §1/§3 | each guard removed |
| fuzz: `from_avc_sps`/`from_hevc_sps` added to the existing `transmux_ts_demux*` targets' reach + a direct `sps_display` target | - | no panic, bounded | - |

Effort: about 3 working days including the table transcription.

### Phase 2 - change event + per-AU tag (SPS-derived)

Scope: `DemuxEvent::DisplayChanged`, `Sample.display`, `DisplayChangeSet`;
SPS/PPS tracked **by id** with activation from the slice header (§6.2); the
initial level-triggered event (§5.2); the ordering contract (§6.5) for
TS (and the FLV/RTP audit in §7); per-AU tags in batch `Media`; fMP4 in-band
(avc3/hev1) tagging (2b, separable).

Semver: additive (new variant on `#[non_exhaustive]` enum, new field on
`#[non_exhaustive]` struct) -> 0.27.x. Downstream `match`es already need a
wildcard arm; `multimux/src/source/ts_program.rs` (70 `DemuxEvent::` sites)
must be audited for a wildcard that logs "unexpected event" noisily.

Acceptance tests:

| Test | Data | Mutation it kills |
|---|---|---|
| no false change: melodie (SPS re-sent each IDR) yields exactly one `DisplayChanged` (initial) and zero `TrackUpdated` | melodie (real) | per-AU byte compare instead of parsed compare |
| real-encoder change: melodie cut at an IDR, second half re-tagged with ffmpeg `h264_metadata=sample_aspect_ratio=12:11` (SPS VUI rewritten, slices untouched), TS-concatenated; assert one `DisplayChanged {aspect_ratio}` whose dts/pts equal the first IDR of part 2, the preceding Sample carries 64:45, the following 12:11 (`Arc::ptr_eq` with the event) | derived from melodie, provenance in `private/fixtures/ts/PROVENANCE.md` | emit on SPS arrival not activation; emit one sample early; tag the wrong sample |
| geometry-neutral SPS change -> `TrackUpdated` only (second half re-tagged with `h264_metadata=level=`) | derived | `DisplayChanged` raised from byte inequality |
| new-id SPS sent one GOP early, activated at the next IDR -> event at the activating IDR, not at the carrying AU | synthetic from melodie AUs | single-slot SPS tracking |
| mid-CVS content change (non-conforming) -> event at the referencing non-IDR AU with `random_access == false` | synthetic | silently deferring to next RAP |
| batch `Media`: per-sample tags reproduce the streaming sequence | derived fixture | last-wins spec used for all samples |

No real 16:9 <-> 4:3 SAR switch exists in any fixture scanned (§10). Candidate
real sources (integrator): RAI SD (rai-storia, rai-scuola, rai-premium),
krone/stimmungsgarten; the integrator's background recorder that cuts a
fixture when ffprobe sees a SAR change is the right way to get one, and its
output goes to `private/fixtures/ts/` with PROVENANCE. Phase 2 does not wait
for it; when it lands it becomes the headline test.

Effort: about 4-5 days.

### Phase 3 - AFD and bar data

Scope: `ActiveFormatDescription`, `BarData`, `ActiveFormat`; one shared
user-data parser for the three carriages (§6.4); AFD folded into
`DisplayInfo` and `DisplayChanged`; persistence per §6.4 behind a documented
policy.

Semver: additive -> 0.27.x.

Acceptance tests:

| Test | Data | Mutation it kills |
|---|---|---|
| H.264 AFD 8 on every picture -> one initial `DisplayChanged` with `afd == SameAsCodedFrame`, no further event | public `fixtures/ts/gulli-opengop.ts` (293 AFD SEI, all `1000`) | re-emit on every repeat |
| two H.264 PIDs both AFD 8 in one mux, each tagged on its own track | tsduck `test-092.ts` (mirrored) | AFD state shared across tracks |
| MPEG-2 AFD `1010` (16:9 centre) on one PID and `1000` on another | tsduck `test-040.ts` (needs mirroring) | carriage confusion; value table off-by-one |
| MPEG-2 AFD `1110` (16:9 shoot-and-protect 14:9) | tsduck `test-205.ts` (needs mirroring) | label/variant mismatch |
| AFD change mid-GOP: a spec-vector T.35 SEI NAL `06 04 09 B5 00 31 44 54 47 31 41 F9 80` (NAL type 6; payloadType 4; payloadSize 9; country 0xB5; provider 0x0031; 'DTG1'; afd_data `0 1 000001` = 0x41, `1111 1001` = 0xF9 i.e. 4:3 centre; rbsp trailing bits) inserted into real gulli AUs at AU k -> event at k with k's dts/pts and `random_access` as k's | synthetic over real gulli | RAP-only assumption; wrong AU |
| `active_format_flag == 0` clears; reserved code `1100` -> `Reserved(12)`, `Display` renders `reserved(0x0C)` | spec vectors | lossless catch-all |
| HEVC prefix-SEI AFD; suffix SEI ignored | spec vectors (no real HEVC AFD fixture) | accepting suffix SEI |
| bar data pairs | spec vectors only | flag/field order |
| fuzz: user-data parser target | - | - |

Effort: about 3 days (+1 if MPEG-2 picture user_data needs a new hook in the
MPEG-2 video path).

### Phase 4 - mux side and manifests

Scope (needs gaps G3-G5 closed first): `build_trak` writes `pasp` when the SAR
is specified and `colr nclx` when a colour description is present (from
`TrackSpec.display`); preserve demuxed `pasp`/`colr` through the IR; `tkhd`
width/height per §8.3.2 once transcribed; DASH `@sar` on Representations and
`@par` on Adaptation Sets (DVB-DASH rule, SN §7) in `dash.rs`/`ll_dash.rs`;
HLS `VIDEO-RANGE` in `broadcast-hls` from transfer characteristics (+ the
alternative-transfer SEI case, §3); `RESOLUTION` semantics decided (Q8). No
`clap` from SPS cropping: the decoder already applies the SPS crop, a `clap`
derived from it would crop twice (and the default display window is a
suggestion only).

Semver: transmux additive API, but **init-segment and manifest bytes change**
for anamorphic/colour-tagged input. Golden-byte tests in `multimux` /
`hls-runtime` will move. Patch vs minor is an owner call (Q9). `broadcast-hls`
additive.

Acceptance: `mediastreamvalidator` oracle (`media-doctor` test, macOS) on an
anamorphic CMAF-HLS output from melodie; parse-back round trip of
`pasp`/`colr`; DASH MPD `@sar="64:45"` for melodie and `16:11` for krone.

Effort: about 4 days plus transcription.

## 9. Semver and the epoch rule

| Option | Class | Cascade |
|---|---|---|
| Field on `CodecConfig::Avc`/`Hevc` variants | **breaking** (variants are not `#[non_exhaustive]`; multimux builds them with literals) -> transmux 0.28 | changes the transmux caret epoch for multimux, hls-runtime, media-doctor, media-plane, transmux-py, timed-metadata/rtmp-runtime tests -> each takes a major-class bump (0.x minor) under the epoch-purity rule |
| Field on `AvcSpsInfo`/`HevcSpsInfo` | breaking (not `#[non_exhaustive]`) | same |
| **`TrackSpec.display`, `Sample.display`, `DemuxEvent::DisplayChanged`, new `ir::display` types, new `DisplayGeometry::from_*` fns** | **additive** on `#[non_exhaustive]` types | **none** |

The recommendation is entirely in the last row. Making `CodecConfig` variants
`#[non_exhaustive]` is a good idea on its own but is a separate breaking
change, not part of this work.

## 10. Fixtures: what can be proven on real data

Scanned (read-only) every `.ts`/`.m2ts` > 100 kB under `private/fixtures/`
(main checkout, private `main`), public `fixtures/`, and `.test-streams/`:
ffprobe stream/frame SAR and colour, ffmpeg `trace_headers` for SPS VUI
fields, and a per-PID byte scan for `DTG1` AFD (T.35 SEI and MPEG-2 user data)
and `GA94`+0x06 bar data.

| Signal | Real fixture | Proven on real data? |
|---|---|---|
| H.264 Extended_SAR 64:45 + colour 5/5/5, interlaced | `private/fixtures/ts/melodie-anamorphic-sar-64-45-zenith-tv.ts` | yes |
| H.264 table SAR 16:11 (idc 4), no video signal type | `private/fixtures/ts/krone-anamorphic-sar-16-11-zenith-tv.ts` | yes |
| H.264 SAR 4:3 (1440x1080) and 16:11 in one mux | tsduck `test-092.ts` (mirrored) | yes |
| H.264 SAR 12:11 | tsduck `test-084.ts` (mirrored) | yes |
| H.264 frame cropping | public `fixtures/ts/h264/high_1080_cropped.ts`; `transformers-eia608-h264.ts` | yes |
| H.264 full range | `private/fixtures/ts/transformers-eia608-h264.ts` | yes |
| H.264 AFD (value 1000 only) | public `fixtures/ts/gulli-opengop.ts`; tsduck `test-065.m2ts`, `test-092.ts` | yes, one value |
| MPEG-2 AFD 1000 / 1010 / 1110 | tsduck `test-040.ts`, `test-205.ts`, `test-128.*` (mirroring needed) | yes, three values |
| MPEG-2 SAR 64:45 / 16:11 | `private/fixtures/ts/hotbird-mhp-video-head-6MB.ts`, tsduck `test-205.ts` | yes (Phase 1b) |
| HEVC colour PQ / HLG (bt2020) | tsduck `test-074.hevc.ts`, `test-010.ts` (mirroring needed) | yes |
| HEVC SAR != 1:1, conformance window, default display window | none (all HEVC fixtures are 3840x2160, idc 1, no windows) | **spec vector / derived only** |
| HEVC AFD | none | **spec vector only** |
| Bar data | none | **spec vector only** |
| **AFD value change within one PID** | none (every PID carries one constant value) | **synthetic only** |
| **SAR change within one PID (16:9 <-> 4:3)** | none (per-stream transition count 0 on every candidate) | **derived/synthetic only** until the recorder catches one |

All new test references to tsduck files must go through `test_fixtures` and be
mirrored under `private/fixtures/tsduck/` with the pinned SHA (only referenced
files are mirrored today). The worktree's pinned private gitlink (`724383c`)
predates the zenith-tv captures; Phase 1 bumps it.

## 11. Decision table

| # | Alternative | Pros | Cons | Verdict |
|---|---|---|---|---|
| A | SAR field on `CodecConfig::Avc/Hevc` only | smallest diff | breaking + cascade; static only; no change signal; AVC/HEVC only; the "bolted-on field" the owner ruled out | reject |
| B | `TrackSpec.display` only (static) | additive; reaches every path via `TrackSpec::new`; rides `TrackUpdated`/`TracksChanged` | `TrackUpdated` is one frame early today; no AFD; batch loses history | keep, as the static layer |
| C | `DemuxEvent::DisplayChanged` only | precise, ordered, provenance like `ClockReference` | lost in batch `Media`, in transforms, and in `media-plane` (no event mirror) | keep, as the notification layer |
| D | `Sample.display` tag only | survives batch, transforms, Trunk; cheap per chunk | no "it changed" edge for a consumer that wants to react once | keep, as the per-AU layer |
| E | Generation counter on `Sample` + table on `Track` | 4 bytes per sample | desyncs when a transform drops or reorders samples; needs a lookup; `Media` has no event table | reject |
| F | AFD on `TrackSpec` | one place for everything | AFD flips would raise `TrackUpdated` = init rebuild churn; AFD is picture-level | reject |
| **R** | **B + C + D, split sequence-level (TrackSpec) from picture-level (DisplayInfo), Arc-shared** | additive, no cascade; one model; R1-R8 met | three surfaces to keep consistent (pinned by the ordering test) | **recommend** |

## 12. Spec gaps (content-checked, not filename-checked)

Checked by grepping `private/specs-md/*.md` for the defining syntax names
(`PixelAspectRatioBox`, `ColourInformationBox`, `CleanApertureBox`, `23001-8`,
`ST 2016-1`, `afd_identifier`/`DTG1`, `@sar`), not only file names:

| Gap | What is missing | Effect | Where it is only cited |
|---|---|---|---|
| G1 | **SMPTE ST 2016-1** (AFD and bar data, the normative source TS 101 154 refers to as [23]) | no AFD persistence rule for AVC/HEVC; Table B.8 line semantics | cited by TS 101 154 §B.4.2, ATSC A/53-4, ST 377-1; no transcription |
| G2 | **ATSC A/72 Part 1** (AVC video in ATSC, AFD/bar data carriage + persistence for AVC) | same | not present at all |
| G3 | **ISO/IEC 14496-12** as a whole: no `private/specs-md` file. Only an excerpt exists in `transmux/docs/codec/isobmff-visual-ext.md` (§12.1.4.2 syntax, §12.1.4.3, §12.1.5), starting mid-sentence. Missing: §12.1.4.1 (placement, `pasp`/`clap` vs bitstream precedence), §8.3.2 (`tkhd` width/height) | Phase 4 `pasp` precedence and `tkhd` sizing | `PixelAspectRatioBox` appears only in the UWA HDR guide and AVS3 IMF md (other specs quoting it) |
| G4 | **ISO/IEC 23001-8 / ITU-T H.273 (CICP)** | none for VUI (H.264/H.265 Tables E-3..E-5 are sufficient and authoritative for the codec); needed only if `colr` values outside the codec tables must be labelled | cited by A/331, TS 102 366, H.222.0, and rfc8216bis `[CICP]` |
| G5 | **ISO/IEC 23009-1** (MPD `@sar`/`@par` attribute syntax) | Phase 4 DASH output | only ETSI TS 103 285 (DVB-DASH) rules present |
| G6 | H.264 Table E-2 and Tables E-3..E-5 / H.265 E.2..E.5 not yet page-checked into a `docs/` transcription | Phase 1 first task | present in md, unverified |
| G7 | TS 101 154 Table B.8 (bar data line numbering) | bar data coordinates stay wire values | present in md, unverified |
| G8 | CTA CEB16 (AFD receiver recommendations, informative) | none (informative) | cited by A/53-4 and TS 101 154 |

## 13. Edge cases

| # | Case | Behaviour |
|---|---|---|
| E1 | `aspect_ratio_idc` 0, or Extended_SAR with `sar_width` or `sar_height` 0 | `aspect_ratio = Some(..)` (wire kept), `sample_aspect_ratio() = None` ("unspecified", E.2.1/E.3.1) |
| E2 | reserved idc 17..254 | `Reserved(n)`, SAR `None` (H.265 "interpret as 0", applied to both; SN §1) |
| E3 | crop offsets beyond the picture (non-conforming) | today `saturating_sub` yields width 0. Proposed: `output_rect` = full coded picture and the SPS still accepted (the crop is display metadata, the slices still decode); owner question Q10 whether to also raise `InputDegraded` |
| E4 | CropUnitX/Y | from ChromaArrayType (0 when `separate_colour_plane_flag`), SubWidthC/SubHeightC (Table 6-1) and `2 - frame_mbs_only_flag` (SN §3); melodie/krone are `frame_mbs_only_flag = 0` so CropUnitY = 4 for 4:2:0 |
| E5 | VUI truncated mid-walk | fields read before the truncation are kept, later ones `None` (the existing `parse_avc_vui` policy); never a decode error for the SPS |
| E6 | HEVC `field_seq_flag = 1` | SAR applies to field pictures (Table E.1 NOTE 2); geometry describes the coded field; documented for the player, not "corrected" |
| E7 | non-reduced SAR on the wire | kept in `AspectRatioIdc`; `reduced()` on request |
| E8 | multiple SPS ids in one stream | tracked by id; only the activated one counts (§6.2) |
| E9 | AFD + bar data disagree | both carried; ATSC A/53-4 §6.2.5 (informative, MPEG-2): AFD preferred except AFD 0000/0100 where bar data is used; the player composes |
| E10 | PAFF field pair with SPS at the first field | change attributed to the merged sample (first field's dts/pts) |
| E11 | `u16` overflow of `CodecConfig` width/height | `DisplayGeometry` uses `u32`; `CodecConfig` unchanged |

## 14. Guards

- `label_coverage` (transmux): new public enums `AspectRatioIdc`,
  `VideoFormat`, `ColourPrimaries`, `TransferCharacteristics`,
  `MatrixCoefficients`, `ActiveFormatDescription`, `UserDataCarriage`,
  `GeometrySource` all get `name()` + `impl_spec_display!` (byte-bearing
  `Reserved(u8)` arms rendered `"reserved(0x..)"`). `DisplayChangeSet` is a
  struct, not an enum.
- No magic numbers outside `#[cfg(test)]`: idc 255, 0xB5/0x0031/'DTG1'/'GA94',
  0x06, 0x000001B2 are named constants (the T.35 ones already are in `nal.rs`).
- `clock33_guard`: no new timestamp arithmetic (event dts/pts are copied from
  the already-unwrapped sample values).
- `dehandroll_guard`: unaffected (no HTTP/SDP/date code).
- Fuzz: new `transmux_sps_display` and `transmux_user_data` targets plus the
  existing `transmux_ts_demux*` targets reaching the new code (new parser =
  new-crate bar, per the fuzz rule).

## 15. Open questions for the owner

| # | Question | Default if unanswered |
|---|---|---|
| Q1 | Ship the `TrackUpdated` one-behind ordering fix in Phase 1 as a `### Fixed` patch item (behaviour change: the event moves one sample later)? | yes |
| Q2 | Emit an initial level-triggered `DisplayChanged` per video track (one code path for consumers), or rely on `TrackAdded.display` only? | emit |
| Q3 | Non-conforming mid-CVS SPS content change: event at the referencing AU only, or also an `InputDegraded` (new `InputDegradation` variant)? | event only |
| Q4 | AFD persistence order for AVC/HEVC with B-pictures: decode order (what the player's DTS stamping implies) or output order? | decode order |
| Q5 | AVC/HEVC AFD persistence policy pending A/72 / ST 2016-1: sticky-until-new-AFD (proposed) or reset at a new CVS (MPEG-2 analogue)? Also: transcribe A/72 Part 1 and ST 2016-1 into the private repo first? | sticky; transcribe |
| Q6 | Include MPEG-2 video sequence geometry (Phase 1b) given the player is H.264/H.265? | defer; MPEG-2 AFD still in Phase 3 |
| Q7 | `pasp` vs VUI SAR precedence: blocked on transcribing ISO/IEC 14496-12 §12.1.4.1 into `private/specs-md`. Approve that transcription? | VUI wins until then |
| Q8 | HLS `RESOLUTION` for anamorphic video: coded size, cropped size, or display size after SAR? (RFC 8216 says "optimal pixel resolution at which to display".) | needs Apple-side evidence; no change until decided |
| Q9 | Phase 4 changes init-segment/manifest bytes for anamorphic input: patch or minor? | minor-class note in CHANGELOG, still 0.27.x |
| Q10 | Out-of-range crop: accept with full-picture `output_rect`, and raise `InputDegraded`? | accept, no event |
| Q11 | Should `CodecConfig` variants become `#[non_exhaustive]` in the next planned breaking transmux release (separate from this work)? | yes, separately |
