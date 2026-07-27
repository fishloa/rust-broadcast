# Changelog

All notable changes to `transmux` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **RTP depacketisation now detects loss/reorder instead of silently
  corrupting reassembly** (issue #779): `RtpHeader.sequence` was parsed but
  never read at any non-test call site, so `RtpStreamDepacketiser` decided
  access-unit boundaries purely from the RTP timestamp and marker bit — a
  dropped FU-A fragment was silently concatenated with its neighbours into a
  malformed access unit. Each track now tracks its expected 16-bit sequence
  number (RFC 3550 §5.1/§A.1, compared with wrapping arithmetic, never `>`):
  an in-order or legally-duplicate packet is unaffected; a reordered packet
  is held in a bounded buffer (`RtpStreamTrack::with_reorder_depth`, default
  `DEFAULT_REORDER_DEPTH`) and replayed once the gap fills, reassembling
  byte-identically to the in-order capture; a genuine gap drops the access
  unit under construction rather than reassembling it from a run missing a
  fragment, and records the new `RtpLossEvent::SequenceGap`
  (drained via `RtpStreamDepacketiser::poll_loss_event`) — a clean capture
  raises zero loss events end to end. The reorder buffer is a hard bound
  (never a fifth unbounded-allocation vector). See
  `transmux/docs/rtp/rtp-sequence-validation.md` for the RFC 3550 §A.1
  transcription this is adapted from, and `rtp_stream`'s module docs for why
  the signal surfaces locally rather than in `ir::DemuxEvent`.

## [0.20.0] - 2026-07-26

**Publish order:** `broadcast-common` 8.7.0 → `transmux` 0.20.0 → `media-doctor` → (steps 4/5: `ll-hls-runtime`, `multimux`, `multimux-cli`).

Media plane step 2: consolidates the IR to carry absolute, unwrapped
timestamps (`Sample::dts`/`pts` as `Option<i64>`, `duration` as `Option<u32>`)
so downstream callers (splice, DVR, pacing, live origin) can work with real
time, and adds track-lifecycle events (`TrackUpdated`/`TrackRemoved`/
`TrackAbandoned`) for mid-stream PMT changes. An aggregate review of the whole
step-2 range then found 11 blocking defects; the five worst are fixed here
(folded into this still-open release rather than shipped separately),
alongside two CENC confidentiality vulnerabilities disclosed under Security
below.

### Added

- **`CodecConfig::Subtitle { format: SubtitleFormat }`** (media plane step 2d):
  `Fmp4Demux` now demuxes `stpp` (TTML/IMSC, ISO/IEC 14496-30 §7.2) and `wvtt`
  (WebVTT, §9.2) ISOBMFF sample entries into this variant instead of silently
  dropping the track — samples stay opaque (never cue-parsed).
  `SubtitleFormat` (`#[non_exhaustive]`, `name()`/`Display` per the #204
  convention) also carries `DvbBitmap`/`Teletext` tokens for the PES-carried
  broadcast subtitle formats (still `CodecConfig::Data` on the TS demux path
  today). There is no re-mux path yet for a `Subtitle` track — `build_trak`
  rejects it with `Error::UnsupportedCodec` (`TODO(#753)`).
- The `ac-4` ISOBMFF sample entry now demuxes to the existing
  `CodecConfig::Ac4` (the mux direction already worked; only the demux arm was
  missing) — a full mux ↔ demux round trip.
- **Shared segmentation primitives in `transmux::segmenter`**, now public
  because all four segmenters use them and their behaviour is observable:
  `MediaClock` (per-track elapsed-media accounting: `duration` when it is a
  real, non-zero span, else the absolute `dts` delta), `choose_anchor` /
  `is_anchor_capable` (first video track of any codec, else the first track
  whose clock can advance; a section-only track set is an error), and
  `MAX_PENDING_SAMPLES_PER_TRACK` (the un-cut buffer bound). Previously each
  module carried its own copy, so the four could — and did — drift apart.
- **`DemuxEvent::TrackUpdated`/`TrackRemoved`/`TrackAbandoned`, PMT version
  diffing** (issue #774, unblocks rust-skyfire#96): `StreamingTsDemux` now
  diffs a PMT's `version_number`/`current_next_indicator` (ISO/IEC 13818-1
  §2.4.4.8) instead of only ever inserting newly-seen PIDs.
  - A version change that no longer lists a previously-declared PID emits
    `TrackRemoved { track_id, provenance }` (only for a PID that had already
    reached `Live` — a real `track_id` a consumer has seen).
  - A version change that alters an existing PID's `es_info_descriptors` or
    reclassifies its `stream_type` emits `TrackUpdated(TrackSpec)` (codec
    config recovery itself stays single-shot and permanent).
  - A track whose codec config never becomes recoverable by end of input, or
    whose probe/parked backlog exceeds its byte budget, emits
    `TrackAbandoned { track_id: Option<u32>, reason: AbandonReason,
    provenance }` (`AbandonReason::ConfigUnrecoverable` /
    `AbandonReason::BudgetExceeded`).
  - A carousel-repeated identical-version PMT section (several times a second
    on a real broadcast) is parsed but never re-diffed — no spurious events on
    an unchanged track set.

### Changed

- **BREAKING — two silent drops are now typed errors** (media plane step 2d,
  landed only after the coverage above, so `stpp`/`wvtt`/`ac-4` — which used
  to hit these paths — no longer do):
  - `Fmp4Demux` no longer silently skips a track whose sample entry it cannot
    reconstruct into a `CodecConfig`; it now returns
    `Error::UnsupportedSampleEntry { fourcc }`, naming the offending sample
    entry.
  - `CmafMux` no longer silently filters `CodecConfig::Data` tracks out of the
    init/media segments; it now returns
    `Error::UnmuxableDataTrack { track_id, stream_type }`. A caller that wants
    the old best-effort behaviour must pre-filter explicitly, e.g.
    `media.select_tracks_by(|t| !matches!(t.spec.config, CodecConfig::Data { .. }))`.
- **BREAKING — `Sample` timing is now absolute and optional** (media plane step
  2c, `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4).
  `Sample` is now `{ data, dts: Option<i64>, pts: Option<i64>, duration:
  Option<u32>, flags: SampleFlags, provenance: Option<Provenance> }`:
  - `dts`/`pts` are **absolute** tick values in the track's own media timescale
    (`TrackSpec::timescale`), replacing the previous model in which a sample's
    time was the running sum of preceding `duration`s anchored on
    `Track::start_decode_time` — an anchor that FLV, WebM, MPEG Program Stream,
    RTMP and RTP all left at `0`. The IR can now address a splice point, rebase
    across sources, and express a send deadline.
  - 33-bit (MPEG-2 Systems, ISO/IEC 13818-1 §2.4.3.7) and 32-bit (RTP,
    RFC 3550 §5.1) rollover is unwrapped **once, at the demux edge**, and never
    re-derived downstream.
  - `Option`, not mandatory: section-carried tracks (SCTE-35 `stream_type`
    `0x86`, DSM-CC, private sections) genuinely have no timestamp, so they keep
    `dts`/`pts`/`duration` of `None` rather than a fabricated value.
  - `composition_offset` is no longer a stored field — it is implied by the
    pair, via the new `Sample::composition_offset()` (`pts - dts`), and still
    round-trips fMP4 `ctts` byte-identically.
  - `is_sync` moved into the `#[non_exhaustive]` `SampleFlags` struct
    (`sample.flags.is_sync`; `SampleFlags::SYNC` / `NON_SYNC` / `new`).
  - `Sample::new`/`from_annexb` take `(data, dts, pts, duration, is_sync)` and
    `from_raw` takes `(data, dts, pts, duration)`.
- **BREAKING — `SourceTiming` deleted.** It was write-only (the crate's own docs
  admitted "all mux paths in this crate ignore this field"). The source
  container's raw, pre-unwrap wire stamps survive as the debug-only
  `Provenance { wire_dts, wire_pts }` side-field (`Sample::with_provenance`), so
  no information is lost — the crate just stops presenting a debug field as a
  timing model.
- **BREAKING — `rebase::unroll_33bit_wraps` and `rebase::MPEG_TS_WRAP` removed.**
  Wrap-unrolling now happens once at the demux edge, so re-folding the IR
  timeline back into 33 bits in order to unwrap it again was exactly the
  anti-pattern this step removes. `rebase_to_zero` / `apply_offset` /
  `insert_discontinuity_gap` now shift every sample's absolute `dts`/`pts` in
  lockstep with `Track::start_decode_time` (and never fabricate a timestamp for
  a `None`-timed sample).
- **BREAKING — `DemuxEvent` reshaped a second time within this release**
  (found by the aggregate review below; applied here rather than deferred to a
  later major bump):
  - `TrackAdded(Track)` → `TrackAdded(TrackSpec)` — drops the always-empty
    `samples` and never-set `encryption` fields that came along with the full
    `Track`; every existing consumer already only read `track.spec`.
  - `Discontinuity { track, provenance }` → `Discontinuity { track, kind:
    DiscontinuityKind, provenance }`, and the variant is now `#[non_exhaustive]`
    with a `DemuxEvent::discontinuity(...)` constructor. `DiscontinuityKind` is
    `Signalled` (an MPEG-2 TS adaptation-field `discontinuity_indicator`),
    `TimelineReanchored` (a live audio track's frame-exact anchor drifted from
    the wire PES clock), or `BudgetExceeded { bytes }` (a per-PID buffer cap
    dropped in-flight data). `abandon_backlog`'s probe/parked-backlog budget
    cap now emits `TrackAbandoned` instead of a `Discontinuity` — it was
    mis-typed before (a budget overflow abandons the track; nothing survives
    to "continue" from).
  - `TracksResolved` → `TracksResolved { generation: u32 }` — fixes a live bug
    where the de-dup key was the known-PID *count*: a removal immediately
    followed by an addition could return the count to a previously-seen
    value, silently suppressing the re-fire a consumer needs. `generation` is
    a monotonic counter bumped once per applied PMT diff (add/update/remove).
  - `ClockReference` is now `#[non_exhaustive]` too, with a matching
    `DemuxEvent::clock_reference(...)` constructor — both non-exhaustive
    variants can grow a field later (e.g. a wall-clock/UTC anchor) without a
    further breaking change.
  - Documented `DemuxEvent`'s event-order guarantee (observation order per
    emission class, not wire order across classes) and its removal semantics
    (no `Sample` for a removed `track_id` ever follows its `TrackRemoved`;
    removal tracks only the PMT-declared set, never a silence timeout).
- **BREAKING — the rest of the IR surface is `#[non_exhaustive]` too**, applied
  in this same release rather than costing a major bump later (`Media`/`Track`
  were already done): the `DemuxEvent::Sample`,
  `TrackRemoved`, `TrackAbandoned` and `TracksResolved` **variants**, and the
  `Provenance`, `PcrSample`, `SkippedTrack`, `TrackEncryption` and
  `FragmentTrackData` structs. `TracksResolved` took a field this release,
  which is exactly the case this prevents recurring. Every affected type gained
  a constructor so none became unconstructible from outside the crate:
  `DemuxEvent::{sample, track_removed, track_abandoned, tracks_resolved}`,
  `Provenance::new`, `PcrSample::new`, `SkippedTrack::new`,
  `TrackEncryption::new`, `FragmentTrackData::new`. Pattern matches on the four
  variants need a trailing `..`. `EventProvenance` stays `Copy + Eq + Default`.
- **BREAKING — `HlsPackager` omits a timestamp-less track instead of emitting
  `#EXTINF:0.000`.** A section-carried track (SCTE-35 `stream_type` `0x86`,
  DSM-CC, private sections) has `duration: None` on every sample, deliberately
  and never fabricated; summing `unwrap_or(0)` over it rendered a zero
  `EXTINF`, which RFC 8216 §4.3.2.1 defines as a real playback duration a
  player would honour. An HLS media playlist is a timeline of playable
  segments and such a track is not one, so it is left out (its content reaches
  an output through the paths built for it — an inband `emsg`, an
  `EXT-X-DATERANGE`). A `Media` whose tracks are *all* timestamp-less is now a
  named `Error::InvalidInput` rather than an empty playlist.

### Fixed

- **`splice::concat`/`splice_insert` now rebase the spliced-in content's
  `dts`/`pts` onto the join (issue #782).** Both used to place a second
  `Media`'s samples after a join point but never shift their absolute
  timestamps onto it — harmless while decode time was reconstructed by
  summing `duration` from `start_decode_time`, but since this release's
  switch to reading absolute `dts` (above) the spliced-in asset simply kept
  whatever timeline it was demuxed with. Two independently-demuxed assets
  have unrelated absolute timelines (each anchored on its own
  `tfdt`/PCR/FLV clock), so a join could jump backwards or forwards by an
  arbitrary offset while the segment structure claimed continuity —
  producing a wrong fragment `tfdt` and wrong PTS/DTS on TS re-mux, and a
  player that stalls or skips at an SSAI ad break (`splice_insert`'s
  advertised use case). Every incoming sample's `dts`/`pts` (when `Some`;
  a section-carried sample with `dts: None` has nothing to rebase and
  fabricates nothing) now shifts by a single offset derived from one
  reference track (video, else the first timed track) and converted into
  each matched track's own timescale — never derived independently per
  track, which would silently re-align tracks relative to each other and
  destroy A/V sync.
- `TsDemux` stored **audio** sample timing in 90 kHz PES-clock ticks while the
  track's timescale is its sample rate, so `dts` deltas (e.g. 2089) disagreed
  with `duration` (1024 AAC samples). Audio `dts`/`pts` — and the audio track's
  `start_decode_time` anchor — are now rescaled into the track's own timescale,
  the same unit as `duration`. Latent before this release only because the old
  `SourceTiming` was never read back.
- `PsDemux` left every AC-3 sample's `duration` at `0`, making the recovered
  audio timeline uninterpretable; it now carries the intrinsic 1536-sample
  syncframe duration (ETSI TS 102 366 §4.1) and absolute time rescaled from the
  PES stamps.
- `RtpDepacketiser` (batch) discarded the RTP timestamp and the per-AU sync
  flag entirely, emitting `duration: 0` / `is_sync: true` for every sample. It
  now carries the unwrapped absolute RTP media clock and the real IDR-derived
  sync flag.
- **CRITICAL — a PMT that reclassifies a PID's codec no longer panics the
  process.** PMT version diffing (above) made PMT application *destructive*,
  which turned several latent weaknesses into live faults; this is the worst
  of them. `apply_pmt_diff` wrote `stream.codec` in place, leaving the
  `ConfigProbe`, the `Carrier`, and any buffered access units built for the
  **old** codec. A version change that reclassified a still-probing PID — DVB's
  routine `stream_type` `0x06` gaining an `AC-3_descriptor`, so
  `Codec::Data(0x06)` becomes `Codec::Ac3` (issue #641) — then reached
  `finalize_probe`'s `unreachable!("ConfigProbe::Data is only created for
  Codec::Data")` and **aborted on ordinary broadcast input**. A codec change is
  a different elementary stream, so the PID is now torn down and re-registered,
  which rebuilds every derived piece of state in one move. That also fixes the
  stale-`Carrier` half of the same bug: ISO/IEC 13818-1 Table 2-34 splits
  `stream_type` into PES- and section-carried families, so a `0x86` → `0x1B`
  reclassification used to feed H.264 PES bytes to a `SectionReassembler` and
  produce silence while the track still claimed to exist. The `unreachable!`
  itself is gone: a probe/codec mismatch now degrades to "unresolved" (and is
  concluded by the existing `TrackAbandoned` paths) rather than aborting a
  `#![forbid(unsafe_code)]` library on remote input. Every remaining
  panic-class site in `ts_demux` reachable from parsed input was converted the
  same way.
- **CRITICAL — PSI `CRC_32` is validated before any PAT/PMT is acted on**
  (ISO/IEC 13818-1 §2.4.4.1, via `broadcast_common::crc32_mpeg2`). Nothing
  checked it before: `CRC32_LEN` was used only to skip the trailer in length
  arithmetic. Now that PMT application tears tracks down, one bit error in a
  version byte or an ES loop destroyed a live track and reassigned its
  `track_id`; and because `process_packet` consults `pmt_reasm` *before*
  `streams`, a corrupt PAT permanently hijacked an elementary PID into PMT
  reassembly, shadowing its stream for the rest of the run. A section failing
  CRC — or clearing `section_syntax_indicator`, which a PAT/PMT never legally
  does — is now dropped silently and disturbs nothing, not even
  `last_applied_version` (bumping that off a corrupt section would have
  swallowed the genuine version that follows).
- **A PAT may remap a PMT PID, and a "next" PAT is not applied.** The
  PAT-derived `program_number` was write-once (`entry().or_insert_with()`) and
  the PAT was applied ignoring `current_next_indicator`, so a legitimate remap
  — or a `cni == 0` "next" PAT — froze the binding and made the defensive
  `program_number` cross-check reject every PMT on that PID **forever**: a
  silent zero-track demux. The `cni == 1` rule PMT application already used now
  gates the PAT too, and a current PAT updates the binding (clearing
  `last_applied_version`, since the version counter belongs to the program, not
  the PID).
- **A removed PID's payload is not replayed into the re-added track.**
  `remove_track` left the dropped PID's traffic flowing into `unattributed` —
  the *pre-registration* replay buffer — so post-removal orphan payloads
  accumulated to the 4 MiB cap and were then delivered as the **re-added**
  track's first samples, anchoring its `start_decode_time` in the past. The
  backlog is now purged on removal and the PID is blacklisted from that buffer
  until a PMT declares it again.
- **An ES PID declared by two programs survives one of them dropping it.**
  `streams`/`es_seen` are global while a PMT's `applied_es` is per-PMT, so a
  shared audio/subtitle component was torn down as soon as *either* program's
  PMT stopped listing it. Removal is now refcounted by declaring PMT PID; only
  the last declarer's drop removes the track.
- **The audio re-anchor threshold is derived from real muxer behaviour, not
  from one sample period.** The B5 anchor's discontinuity threshold was
  `ceil(90000 / sample_rate)` — 3 ticks at 44.1 kHz — while a 1024-sample AAC
  frame is `2089.795…` ticks, so a muxer stamping the rounded constant `2090`
  drifts `+0.204…` ticks per frame *on a perfectly continuous stream* and
  crossed the threshold about every 15 frames; non-frame-aligned MP2 PES (issue
  #638) crossed it on essentially every access unit. `TimelineReanchored` was
  therefore pure noise and the anchor effectively inert. The bound is now
  20 ms of 90 kHz (1800 ticks) — below the lip-sync detectability floor the
  broadcast recommendations work to (ITU-R BT.1359-1; ATSC A/85's ±15 ms), so
  re-anchoring inside it would trade a real `Discontinuity` event for an
  inaudible correction — floored at two intrinsic sample periods for
  degenerate sample rates. `DiscontinuityKind` had no assertion coverage
  anywhere in the crate; it now has both halves of the contract (a
  constant-increment 44.1 kHz stream emits **zero** re-anchors; one genuine gap
  emits exactly one).
- **A gapped/discontinuous fMP4 keeps its gap.** `Fmp4Demux` seeded `next_dts`
  from only the **first** fragment's `tfdt` and then pure-summed `trun`
  durations, so every sample after a gap came out short by exactly the gap —
  re-muxing to a wrong `tfdt`/PES stamp and permanently desyncing A/V. `tfdt`
  is per-fragment and authoritative (ISO/IEC 14496-12:2015 §8.8.12), so every
  fragment that carries one now re-seeds the cursor; `Track::start_decode_time`
  still records the first fragment's anchor. A gapless stream is byte-for-byte
  unaffected.
- **`rescale_to_track` preserves a negative audio anchor.** Its `.max(0) as
  u128` fabricated `dts = 0` for audio where a legitimately negative unwrapped
  anchor (reordering across the 2^33 boundary) is carried through verbatim for
  every other track kind — desyncing the audio track alone.
- **CRITICAL — a `duration` of `Some(0)`/`None` on the anchor track no longer
  stalls segmentation forever** (all four segmenters: `Segmenter`,
  `LlSegmenter`, `LlHlsSegmenter`, `StreamingTsHlsSegmenter`). The anchor
  accumulator advanced only from `Sample::duration`, so a stream carrying
  `Some(0)` never reached the segment target: no part and no segment was ever
  emitted, `pending` grew without bound, and `Stage::demand()` still reported
  "not saturated", inviting a well-behaved driver to feed to exhaustion. This
  was reachable on a shipped path — `StreamingFlvDemux` derives `duration` as
  the forward delta between FLV tag timestamps, so an RTMP publish's first
  sample (and any two tags sharing a timestamp) is `Some(0)`. Since
  `Sample::dts` is absolute, the new shared `segmenter::MediaClock` advances
  on each sample's own `duration` when that is a real, non-zero span and on
  the **`dts` delta** otherwise; a stream with real durations segments exactly
  as before. Per-track `tfdt`/`base_media_decode_time` accounting was the same
  duration sum and is fixed alongside it, so segment decode times no longer
  all collapse to 0. `StreamingTsHlsSegmenter::push` also no longer rejects an
  anchor sample with `duration: None` outright.
- **CRITICAL — a legal single-IDR / infinite-GOP stream is now bounded instead
  of growing without limit** (all four segmenters). With one keyframe at the
  start and none after, no cut is possible — and cutting mid-GOP would break
  CMAF's (and classic HLS's) random-access guarantee — so the pending buffer
  is bounded on **data**, not time (these types are sans-IO and `no_std`): at
  the new `segmenter::MAX_PENDING_SAMPLES_PER_TRACK` un-cut samples,
  `Stage::demand()` reports `saturated` so a cooperative driver stops feeding,
  and `push`/`Stage::feed` then return a named `Error::InvalidInput`. `flush`/
  `finish` closes the trailing partial segment and input flows again.
- **`LlSegmenter`/`LlHlsSegmenter` anchor on any video codec, not just AVC.**
  Both selected the anchor with `matches!(config, CodecConfig::Avc { .. })`,
  so an HEVC-plus-AAC media with audio first anchored on the **audio** track:
  segments did not begin on an IRAP, and since every AAC sample is a sync
  sample every `PartInfo.independent` was `true`, advertising
  `INDEPENDENT=YES` (RFC 8216bis §4.4.4.9) on parts that actually start
  mid-GOP. Both now use the shared `segmenter::choose_anchor`, matching
  `Segmenter`/`ts_hls` as their doc comments already claimed. A track set with
  no anchor-capable track (all section-carried) is a construction error in all
  four rather than a silent stall.
- **`TsHlsPackager` places a timestamped section sample in the right segment.**
  `partition_tracks` advanced its per-track placement clock only from
  `duration`, which a section-carried track never has, so **every** section
  sample landed in segment 0 — an SCTE-35 cue for t=40 s was muxed at t=0 —
  while the streaming path placed it in the segment that was open on arrival.
  Both paths now share `placement_secs`, which falls back to the sample's
  absolute `dts`, restoring the batch/streaming equivalence the module docs
  claim (and now enforced by a test).
- **`ProgressiveDemux` cannot return silently-wrong sample payloads after a
  buffer-cap rejection.** A `Stage::feed` rejected for exceeding `max_bytes`
  discarded that chunk but was neither terminal nor recorded, and `demand()`
  still advertised headroom — so a following smaller chunk was accepted and
  `finish()` parsed a buffer **with a hole** using file-absolute `stbl` chunk
  offsets (ISO/IEC 14496-12:2015 §8.7.5), yielding either a misleading
  `UnexpectedBox` or a `Media` whose samples carried the wrong bytes. The
  rejection now poisons the demuxer permanently: every later `feed` and
  `finish` re-report the original `BufferCapExceeded`, `demand()` stays
  `saturated`, and no parse is attempted. `feed` after `finish` is likewise
  rejected instead of silently appending, and `finish` releases the
  accumulated buffer rather than retaining the whole file alongside a copy of
  every sample.
- **`StreamingFlvDemux::demand()` reports the real want.** It returned a
  constant 11-byte `want_bytes` even mid-tag-body, so a driver sizing its
  reads by `want_bytes` made ~1.5 million `feed` calls to deliver one 16 MiB
  tag. It now returns exactly the bytes still missing for the unit `feed` is
  blocked on.
- **`StreamingTsHlsSegmenter` keeps one ready queue, not two.** Its `Stage`
  adapter had a separate `stage_ready` beside the inherent inline return, so
  delivery routed per call and the two could drift; both now drain a single
  `ready`. The `Stage` impls of `StreamingTsHlsSegmenter` and
  `StreamingFlvDemux` also call their inherent methods by fully-qualified
  path, since `self.finish()` resolved to the inherent one only by
  inherent-over-trait precedence at identical arity — renaming it would have
  turned the delegation into silent infinite recursion.
- **`Segmenter::push` no longer loses the sample that triggered a failing
  cut.** A `build_media_segment` failure (reachable since `new` stopped
  filtering BMFF-unmuxable tracks) propagated *before* the triggering keyframe
  was buffered, punching a hole in the timeline on every subsequent anchor
  keyframe; the sample is now buffered first and the error surfaced after.
- **`trickplay::derive_iframe_track` doc corrections**: dropped a bullet
  naming the removed `composition_offset` field, and the false claim that the
  derived track "covers the same total timeline as the source" (it starts at
  the first sync sample, so it is shorter when `samples[0]` is not one).
- **`TsMux` no longer drops a recognised codec's PMT `ES_info` descriptors**
  (issue #775, closes #775; nullified a shipped rust-skyfire track-picker
  feature). `plan_elementary_streams` only carried a track's inherited
  `TrackSpec::es_info_descriptors` into the re-muxed PMT for an opaque
  `CodecConfig::Data` track — a stale guard from issue #576, written when the
  IR genuinely carried no descriptors for a decoded codec. Since issue #582
  `TsDemux` populates `es_info_descriptors` for **every** track, so a
  recognised codec's audio-language (`ISO_639_language_descriptor`) and DVB
  `subtitling_descriptor` were silently lost on a TS re-mux — a track lost
  information precisely *because* its codec was understood.
  - The new policy (documented with its spec citations in the `ts_mux` module
    doc) is a **deny-list, not an allow-list** — an allow-list would silently
    drop an unknown-but-valid broadcaster-private or newly-registered
    descriptor, which is the same class of bug.
  - **`CA_descriptor` (tag `0x09`, ISO/IEC 13818-1 §2.6.16) is denied.** It
    signals that the elementary stream is scrambled and names the `CA_PID`
    carrying its ECMs; this muxer emits cleartext, so copying it forward would
    falsely advertise the re-mux as encrypted and point at a `CA_PID` absent
    from the new PMT.
  - Inherited descriptors are **de-duplicated against the descriptors the
    muxer synthesises itself** (e.g. the `MPEG-H_3dAudio_descriptor` built
    from the typed `mpegh3daProfileLevelIndication`, issue #579), keeping the
    synthesised copy — emitting both yields a malformed `ES_info` loop with
    contradictory signalling under one tag.
  - Surviving descriptors keep their **source order**.
  - A merged loop over the 12-bit `ES_info_length` field's 4095-byte maximum
    (§2.4.4.8) returns `Error::BufferCapExceeded { what: "PMT ES_info
    descriptor loop", cap }` rather than being silently truncated into a
    malformed PMT.
- **BREAKING — one strictness policy everywhere: DEMUX = lenient but loud,
  MUX = strict but filterable** (B1-B4).
  - `Fmp4Demux` no longer fails the whole file on one track it cannot
    reconstruct (a QuickTime hint/chapter track, `c608`/`c708`, GoPro
    `gpmd`, ...) — it skips that track and records it, named, in the new
    `Media::skipped: Vec<SkippedTrack>`, matching `ProgressiveDemux`'s
    existing per-track leniency (the two used to diverge on identical
    input).
  - `CodecConfig::is_muxable_in_bmff()` (new, `pub`) now covers both the
    opaque `CodecConfig::Data` carriage and `CodecConfig::Subtitle` — B1:
    `CmafMux` (and every other fMP4/CMAF mux entry point) previously had no
    predicate covering `Subtitle`, so a subtitle-bearing CMAF asset that used
    to repackage fine now failed. A caller must pre-filter with
    `media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())` before
    muxing a `Media` that mixes carriable and non-carriable tracks.
  - `Error::UnmuxableSubtitleTrack { track_id, format }` (new): the named
    rejection for a `Subtitle` track, mirroring `UnmuxableDataTrack`.
  - The strict-but-filterable check is now centralized in
    `build_init_segment` itself, so `CmafMux`, `ProgressiveMux`,
    `Segmenter`, `LlSegmenter`, and `LlHlsSegmenter` all reject a
    non-muxable track the same way (previously only `CmafMux` did; the
    other four silently dropped it).
  - The `transmux` CLI (`cli` feature) now filters non-muxable tracks (with
    a stderr warning naming them) before every fMP4/CMAF-based output
    format, so it no longer fails on an ordinary real-world DVB multiplex
    (DVB subtitle/teletext/ANC/SCTE-35 tracks are routine).
- **Audio DTS is frame-exact again (B5)**: `TsDemux` no longer re-derives an
  audio track's dts/pts from the lossy 90 kHz PES clock on every access
  unit (which injected up to ±1 track tick of jitter at every PES boundary,
  since 90000 does not evenly divide a typical sample rate) — it anchors
  once from the first access unit, then advances by the intrinsic per-frame
  duration, and only re-anchors (emitting `DemuxEvent::Discontinuity`) on a
  genuine gap. The same re-anchor-on-every-stamp bug, found via the new
  invariant test below, is fixed identically in `PsDemux`'s AC-3 track
  recovery.
- Added a per-timed-track invariant, checked on real fixtures across every
  demuxer in the crate (`tests/demux_timing_invariant.rs`): a track's
  `start_decode_time` must equal its first sample's `dts`, and
  `sum(sample.duration)` must equal the span from the first to the last
  sample's `dts` plus the last sample's `duration` — the standing guard
  against the whole re-derive-from-a-lossy-clock class of bug.
- **A codec-changed re-registration bypassed the shared-PID refcount.** The
  "removed" branch of `apply_pmt_diff` already refused to tear down a PID
  another PMT still declares (above); the codec-changed branch never got the
  same check, so a PID declared by two programs (an ordinary shared audio/
  subtitle component) had its track torn down and rebuilt — a fresh
  `track_id`, a spurious `TrackRemoved`/`TrackAdded` — the moment *either*
  declaring PMT reclassified its codec, even though the other program's
  declaration was unchanged. The codec-changed branch now consults
  `es_declarers` too: reclassification is refused while any other declarer
  still lists the PID (the existing classification wins; two programs
  declaring one PID under different codecs is a malformed multiplex, and
  last-writer-wins was rejected as letting either program's routine version
  bump flip the shared track back and forth), proceeding only once this PMT
  is the *last* declarer. The same re-registration also now restores the
  PID's original PMT-declaration-order slot in `codec_order`/`data_order`
  instead of losing it to the back of the list, which reordered `TrackAdded`
  emission and could block a later-ranked PID's promotion behind it.
- **`splice::concat`/`splice_insert` read a sample's true absolute `dts` for
  the join/snap position, not a duration-sum reconstruction.**
  `track_end_decode_time`, `snap_to_preceding_sync`, and `splice_insert`'s own
  boundary-decode-time calculation still derived decode time as
  `start_decode_time + Σ duration` — harmless while the two representations
  agreed, but the per-fragment `tfdt` reseed (above) can legitimately leave a
  gap between `start_decode_time` and a sample's true `dts`, which silently
  mis-placed the splice join/snap one step downstream of that fix. All three
  now read the sample's own absolute `dts` directly, falling back to the
  duration sum only for a genuinely timestamp-less (section-carried) sample.
- **The four flagship segmenter doctests (`Segmenter`, `LlSegmenter`,
  `LlHlsSegmenter`, `StreamingTsHlsSegmenter`) actually run now** (#780).
  Each was wrapped in `# if false { … }` with a `spec()` stub returning
  `unimplemented!()`, so rustdoc only type-checked the body and never
  executed it — a user copying the example got code that merely compiled.
  `spec()` now builds a real minimal AVC `TrackSpec`, the `# if false`
  wrapper is gone, and each example carries an assertion tied to the
  segmenter's actual cut/flush behaviour (verified to fail under a mutated
  segmenter, then reverted).

### Security

Four CENC (ISO/IEC 23001-7) defects, two of them a **total loss of
confidentiality** for content encrypted by this crate. Anything encrypted by
`CencEncryptor` with the affected configurations must be re-encrypted with a
fresh content key — the exposure is in the ciphertext already published, not
only in the code.

**Affected releases: 0.16.0 through 0.19.0** (`CencEncryptor` first shipped in
0.16.0); fixed in 0.20.0. **Affected configurations:** the keystream-reuse
defect requires `CencScheme::Cenc` and a `Media` carrying **two or more
tracks** encrypted in one `encrypt` call — the overwhelmingly common
video+audio case. Single-track output, and all `CencScheme::Cbcs` output, are
unaffected by it. Re-keying alone is not sufficient if the same content is
re-encrypted with colliding IVs; take the 0.20.0 IV semantics below.

- **CRITICAL — BREAKING: AES-CTR keystream reuse across tracks (a two-time
  pad).** `CencEncryptor::encrypt` restarted its per-sample IV counter at
  `base` for *every* track while applying one shared content key, so under
  `cenc` (AES-CTR) video sample *i* and audio sample *i* were encrypted with
  the same key **and** the same counter block. XOR-ing the two ciphertexts
  cancels the keystream and yields the XOR of the two plaintexts — both
  disclosed without the key. ISO/IEC 23001-7 §9.2 requires the IV to be unique
  per *key*; "unique per track" is not sufficient when one key covers every
  track. Measured on the two-track `fixtures/ts/h264_aac.ts`: 75 of 206
  per-sample IVs collided. Every pre-existing test narrowed its fixture to a
  single track, which is why a green suite shipped it. An adversarial review
  of the first fix (below) found it incomplete in two further ways, both
  closed in this same 0.20.0 (never separately released): the duplicate-IV
  backstop ran *after* every track had already been keystreamed in place
  (fixed by validating the full planned IV sequence before ciphering a single
  byte), and IV uniqueness was only ever guaranteed *within* one `encrypt`
  call, not across separate calls sharing a key (fixed by moving the running
  counter onto the encryptor instance). See the two follow-up entries below.
  - `IvGen::Counter { base }`'s sample index now runs continuously across the
    whole `Media` in (track, sample) order, and never resets per track.
  - **BREAKING** — `IvGen::Explicit(ivs)` now requires exactly
    `media.tracks.iter().map(|t| t.samples.len()).sum()` IVs, consumed in
    (track, sample) order. Previously it was validated against *each track's*
    count, so one list was replayed verbatim for every track — the same defect
    by a different route. A caller passing a per-track-sized list now gets
    `Error::InvalidInput` instead of silent keystream reuse.
  - `IvGen::Explicit` also rejects a list containing any **duplicate** IV.
  - All IV validation — including the whole-`Media` duplicate check — now
    happens before the first sample is ciphered, so a rejected configuration
    leaves the `Media` untouched instead of half-encrypted (see the follow-up
    entry immediately below for why this is now actually true, not just
    documented as true).
- **CRITICAL — BREAKING (adversarial-review follow-up, F1): the duplicate-IV
  backstop ran after ciphering, not before.** The check above that rejects a
  duplicate per-sample IV used to run against every track's *already-recorded*
  `Track::encryption` — i.e. **after** `encrypt`'s per-track cipher loop had
  already overwritten every sample via a real AES-CTR/CBC pass. Its whole
  purpose is catching a *reintroduced* per-track-reset bug (the defect above),
  so on the one path it exists to guard, `encrypt` returned `Err` while `media`
  was left irreversibly two-time-padded with the plaintext already gone and no
  rollback — exactly contradicting the CHANGELOG claim above ("a rejected
  configuration leaves the Media untouched"), which was therefore false for
  this one path. `CencEncryptor::encrypt` now resolves the **entire** planned
  (track, sample) → IV mapping up front (`plan_sample_ivs`) and validates the
  whole plan for duplicates (`assert_ivs_unique`) *before* any cipher work
  runs; the cipher loop then consumes that exact, already-validated plan
  (never re-resolving), so there is no window in which what was checked can
  drift from what gets recorded. Verified by reintroducing the historical
  per-track reset into `plan_sample_ivs` (Edit-then-revert): `encrypt` returns
  `Err` and every sample's bytes remain byte-identical to the input.
- **CRITICAL — BREAKING (adversarial-review follow-up, F2): IV uniqueness was
  only guaranteed within one `encrypt` call, not per key for all time.**
  AES-CTR requires IV uniqueness per **key**, for the life of that key — not
  merely within a single `encrypt` invocation. `IvGen::Counter { base: 0 }`
  (the `Default`) restarted its counter at `0` on every call, so two
  invocations sharing a key — a video-only + audio-only split of one asset, or
  successive live segments encrypted under one key period — reproduced the
  exact two-time pad the first fix closed *within* one call. `CencEncryptor`
  is now a **stateful** value constructed with its content key
  (`CencEncryptor::new(key)` / `CencEncryptor::resume(key, next_counter)`),
  owning the running `IvGen::Counter` index itself and advancing it after
  every successful call rather than resetting it — reusing *one* instance
  across every call that shares a key continues the counter instead of
  restarting it. Consequences:
  - **BREAKING** — `Encrypt::encrypt` (the `broadcast-common` hub trait) now
    takes `&mut self`, not `&self` — the only production implementor
    (`CencEncryptor`) needs mutable state to own the counter; the only other
    implementor workspace-wide is a trait-usability test fixture inside
    `broadcast-common`'s own test module, updated identically.
  - **BREAKING** — `EncryptConfig` no longer carries a `key` field (the key
    now lives on the `CencEncryptor` instance, so a per-call key could never
    silently pair one running counter with a different key). `IvGen::Counter`
    is now a unit variant (no `base` field); construct a resumed encryptor
    with `CencEncryptor::resume(key, base)` for the equivalent effect.
  - Verified: two successive `encrypt` calls on one `CencEncryptor` instance
    that would previously collide now produce fully disjoint IV sets.
    Constructing a second, *separate* `CencEncryptor::new(key)` with the same
    key still collides (each starts its counter at `0`) — this is
    structurally undetectable from inside the type (it has no way to know
    another instance ever used `key` before) and is documented on
    `CencEncryptor` as the caller's remaining obligation: reuse one instance
    per key, never construct a fresh one for a key already in use.
- **Minor — cipher-core IV lengths tightened to exactly 8 or 16 bytes.**
  `cenc_crypto`'s shared `apply_ctr`/`resolve_cbcs_iv` (used by both the
  encrypt and decrypt paths, so this also hardens decryption of untrusted
  files) accepted any length `1..=16`, silently zero-padding it to a 16-byte
  counter/CBC-seed block — so two differently-invalid-length IVs whose
  non-zero bytes happened to coincide could zero-pad to the *same* block.
  Only 8 and 16 bytes are valid on the wire (ISO/IEC 23001-7 §9.2/§12.2); any
  other length is now `Error::InvalidValue`.
- **CRITICAL — BREAKING: `IvGen::Constant` + `CencScheme::Cenc` produced
  unreadable output encrypted under an all-zero counter.** The `cenc` cipher is
  never handed the track's `tenc`, so `IvGen::Constant`'s 16-byte seed (which
  lives only in `tenc.default_constant_IV`) never reached it: every sample was
  encrypted with an all-zero counter block — one keystream for the entire
  track, and output no conformant decryptor can read. The combination is now
  rejected with `Error::InvalidInput`; a constant IV is fundamentally
  incompatible with CTR mode. `IvGen::Constant` is documented as `cbcs`-only
  (where it is the standard convention) and is unchanged under `cbcs`.
- **`cenc` decrypt no longer "succeeds" over garbage.** A conformant file whose
  `tenc` declares `default_per_sample_iv_size == 0` with a
  `default_constant_IV` yields empty `senc` IVs; the `cenc` path decrypted
  those against an all-zero counter and returned `Ok(())` over rubbish. An
  empty per-sample IV under `cenc` is now a typed error in both directions.
- **`senc` no longer sizes an allocation from an unbounded wire field (remote
  DoS).** `SampleEncryptionBox::parse_body` sized
  `Vec::with_capacity(sample_count)` from the untrusted 32-bit `sample_count`
  before any bounds check: a 20-byte box carrying `FF FF FF FF` requested a
  single **206 GB** allocation (measured). `sample_count` is now bounded
  against the box body's own length via each entry's minimum on-wire size, and
  a `senc` whose entries would carry no bytes at all (no per-sample IV *and* no
  subsample map — a shape whose count the wire cannot corroborate at any
  length) is rejected. `saio`'s `entry_count * offset_size` is likewise
  `checked_mul`'d so it cannot wrap past its length check on a 32-bit target.

### Fixed (CENC, lower severity)

- **A failed cipher pass no longer commits a half-encrypted payload.**
  `cenc_crypto`'s in-place rewrite committed the buffer even when the cipher
  returned `Err`, so a malformed `senc` that overran on its *second* subsample
  left the first subsample keystreamed and stored that back into
  `Sample::data`. Both cipher entry points now validate the entire subsample
  map up front, after which their block loops cannot fail — so an error leaves
  the sample byte-identical, and never leaves it empty.
- **Subsample maps must now cover the whole sample on decrypt too** (ISO/IEC
  23001-7 §9.3, matching the encrypt side): a map declaring 100 bytes of a
  1000-byte sample used to return `Ok(())`, leaving 900 bytes of ciphertext
  presented as plaintext.
- **BREAKING — `CencDecryptor`'s `Decrypt::decrypt` pairs tracks by `track_id`,
  not by position.** It zipped `Media` tracks to crypto records positionally
  while storing the `track_id` for exactly that purpose, so a
  `select_tracks_by`-narrowed `Media` decrypted (say) audio with the *video*
  track's IVs — and when the two tracks' sample counts coincided, the existing
  count check did not catch it either. A media track with no matching record is
  now `Error::InvalidInput`; unmatched *records* remain fine (the narrowed
  case).
- **`iter_length_prefixed_nals` checks its NAL-length addition.** A 4-byte NAL
  length of `0xFFFFFFFF` wrapped `start + len` on a 32-bit `usize`, defeating
  the overrun guard and panicking on the subsequent slice. Reachable from
  `CencEncryptor`'s subsample-map construction on caller-supplied sample data.
- Added a `transmux_cenc_boxes` fuzz target covering the `senc`/`saiz`/`saio`
  parsers (untrusted third-party media; 7.1 M executions clean).
- **(F4) `CencDecryptor` can now decrypt a legitimately `senc`-less fragment.**
  A constant-IV, whole-sample-protected `cbcs` fragment carries none of
  `senc`/`saiz`/`saio` at all (nothing for them to carry — see
  `movie_fragment.rs`'s `build_cenc_fragment_boxes`, issue R3); the decrypt
  side's per-fragment harvesting only ever populated a track's per-sample
  entries *from* a `senc` box, so this shape's `crypto.samples` silently
  stayed empty and `Decrypt::decrypt` always rejected it with a generic
  "sample count mismatch", despite `tenc.default_constant_IV` alone being
  everything a conformant decryptor needs. `harvest_fragment_senc` now
  synthesizes the placeholder per-sample entries this shape needs (one empty
  IV/subsample-map pair per `trun` sample) when `senc` is legitimately absent
  (`default_per_sample_iv_size == 0`); a genuinely missing `senc` on a
  per-sample-IV track is unaffected (still tolerated, still caught downstream
  by the same count check). Found and fixed while adding end-to-end coverage
  for this branch, which had previously been exercised only by this crate's
  own parser, never a real decryptor.
  - That same new coverage found a real, verified interop limitation in
    Bento4's `mp4decrypt` (1.6.0.0): given this exact, ISO/IEC-23001-7-legitimate
    `senc`-less shape, `mp4decrypt` does not decrypt it at all — the output
    `mdat` comes back byte-for-byte unchanged (still ciphertext), no error
    reported. This is not a defect in this crate's cipher math (independently
    re-verified outside this crate: the real ciphertext, AES-128-CBC-decrypted
    with the real key and `tenc.default_constant_IV`, reproduces the true
    plaintext exactly) or in the `saio` anchor (there is no `senc`/`saio` in
    this shape to be wrong about) — it appears `mp4decrypt` requires *some*
    per-sample aux-info structure present in the `traf` before it will attempt
    to decrypt a track, regardless of `tenc`. Documented on the new
    `tests/cenc_encrypt_e2e.rs` test rather than silently masked; the crate's
    own (now-fixed) self round-trip remains that test's hard gate. Tracked as
    a known third-party limitation, not a defect introduced or fixable by this
    story.

## [0.19.0] - 2026-07-26

### Added

- `StreamingFlvDemux` (issue #738): incremental (event-driven) FLV → samples
  demux for live RTMP ingest, the FLV analogue of `StreamingTsDemux`. Feed
  bytes of any size/alignment via `feed` (down to one byte at a time; feed's
  fallible over a hard structural error — bad signature, an implausible
  header `DataOffset`, or a corrupt codec config), drain `DemuxEvent`s
  (`TrackAdded`/`Sample`) one at a time via `poll_event`, and call `finish` to
  flush each track's trailing pending sample — the exact `feed`/`poll_event`/
  `finish` pull idiom `StreamingTsDemux` uses, so a caller can drive both
  demuxers with one uniform drain loop. Reuses `FlvDemux`'s tag-header and
  codec-config (AVC/AAC) parsing verbatim rather than duplicating it.
  Memory-bounded regardless of stream length: the internal buffer never holds
  more than one in-progress tag, plus one pending sample per track; a header
  `DataOffset` above a sane bound is rejected before buffering rather than
  grown toward, closing a remote OOM/DoS a malicious `DataOffset` could
  otherwise trigger. `AVCDecoderConfigurationRecord::parse` (`avc_config.rs`)
  now also rejects an avcC declaring 0 SPS, closing a remote-crash (index
  panic) hole on this same untrusted-ingest path.
- `dash_parse` (issue #758 T1): a hand-rolled MPD parser (`Mpd::parse`), the
  structural inverse of `dash`'s `DashPackager` writer — `no_std`+`alloc`, no
  external XML dependency. Parses `MPD`/`Period`/`AdaptationSet`/
  `Representation`/`SegmentTemplate`/`SegmentTimeline` (ISO/IEC 23009-1
  §5.3/§5.3.9), resolves `SegmentTemplate` inheritance from `AdaptationSet`
  down to `Representation`, and tolerates unmodeled elements
  (`SegmentList`/`SegmentBase`/`ContentProtection`/…) without choking.
  `SegmentTemplate::resolve` substitutes `$RepresentationID$`/`$Number$`/
  `$Time$`/`$Bandwidth$` (with `%0Nd` width and `$$` escaping) into a
  template; `SegmentTimeline::enumerate` expands `<S t= d= r=>` runs into the
  `(number, time)` sequence for `$Time$` addressing, and
  `SegmentTemplate::number_sequence` does the equivalent for constant-
  `@duration` `$Number$` addressing. Also exposes `parse_iso8601_duration`
  for `xs:duration` attributes. Malformed/truncated XML never panics —
  every failure path returns `DashParseError`.
- `smooth_parse` (issue #759 T1): a hand-rolled MS-SSTR **client manifest**
  parser (`SmoothManifest::parse`), the structural inverse of `smooth`'s
  `SmoothPackager` writer — reuses the shared `xml_parse` tokenizer (no
  second XML parser), `no_std`+`alloc`. Parses `SmoothStreamingMedia`/
  `StreamIndex`/`QualityLevel`/`c` (§2.2.2.x), including the live-only
  `IsLive`/`LookAheadFragmentCount`/`DVRWindowLength` attributes (§2.2.2.1).
  `StreamIndex::enumerate_chunks` expands a `c@r` repeat run into the
  `(t, d)` sequence, capped at `MAX_CHUNK_RUN` (100k, mirroring
  `dash_parse::MAX_TIMELINE_SEGMENTS`) against a hostile Manifest's unbounded
  `<c r="...">`; `hex_decode`-ing a `QualityLevel@CodecPrivateData` value is
  likewise capped at `MAX_CODEC_PRIVATE_DATA_HEX_LEN` before any decode
  allocation. `StreamIndex::resolve_fragment_url` substitutes the
  `{bitrate}`/`{start time}` tokens (§2.2.4.1) into the fragment-URL
  template. Because Smooth has no bootstrapping init segment,
  `track_spec_from_quality_level` synthesizes a `pipeline::TrackSpec` from a
  `QualityLevel`'s `CodecPrivateData` (Annex-B SPS/PPS → `avcC` for
  `FourCC="H264"`, raw `AudioSpecificConfig` → `esds` for `FourCC="AACL"`) —
  feed the result to `pipeline::build_init_segment` so `media::Fmp4Demux` can
  absorb the Smooth fragment stream. Also splits `rtp_sdp::avc_config_from_sprop`/
  `aac_config_from_asc_hex` into byte-level building blocks
  (`avc_config_from_sps_pps`/`aac_config_from_asc_bytes`) that a non-SDP
  caller (this module) can reuse directly, rather than duplicating the SPS
  classification / `avcC`/`esds` construction logic.
- **Shared `xml_parse` tokenizer** (issue #758 T1 + #759 T1): both `dash_parse`
  and `smooth_parse` modules extract and reuse a hand-rolled, no_std-capable
  XML token stream parser (`xml_parse::XmlTokenizer`) that neither depends on
  an external XML library nor allocates during parsing — the tokenizer walks
  wire bytes and yields tokens on demand.

### Fixed

- `dash_parse` (issue #758 T1): two critical remote alloc-DoS vectors in the
  MPD parser capped + end-tag name matching to prevent silent truncation:
  1. `SegmentTimeline::enumerate` now returns `Result` and caps total segment
     enumeration at `MAX_TIMELINE_SEGMENTS` (100k), rejecting hostile MPDs with
     unbounded `<S r="...">` repeats instead of allocating/looping unboundedly.
  2. `SegmentTemplate::resolve` clamps `%0Nd` format width to `MAX_FORMAT_WIDTH`
     (20 digits, the max for u64), defending against a hostile `$Number%9999999999d$`
     template allocating/looping unboundedly.
  3. All element parsing functions now validate closing-tag names — a stray
     `</Period>` no longer silently truncates the Period list. `Mpd::parse` +
     callers now return `DashParseError::MismatchedEndTag` on nesting errors.

## [0.18.1] - 2026-07-21

### Added

- `RtpStreamDepacketiser::push_sender_report`/`push_rtcp` + `sync_start_decode_times`
  (issue #722): RTCP Sender Report (RFC 3550 §6.4.1) NTP-wallclock ↔
  RTP-timestamp correlation for cross-track A/V sync, replacing the module's
  `TODO(P5.3)`. Feeding an SR for at least two tracks lets
  `sync_start_decode_times` compute each anchored track's `start_decode_time`
  on one common wallclock, preserving the real inter-track offset that
  independent per-track rebase-to-0 discards. Strictly additive/opt-in: with
  no Sender Reports fed, `sync_start_decode_times` returns an empty `Vec` and
  existing callers keep the unchanged v1 behaviour.

## [0.18.0] - 2026-07-21

### Changed (BREAKING)
- Renamed `RtpPacketiser`, `RtpDepacketiser`, `RtpStreamDepacketiser`, and the
  free functions `packetise_klv`/`depacketise_klv` to British spelling (issue
  #663; all were previously spelled with a "z"). Pure rename —
  behaviour-preserving, no functional change. RFC 6184's SDP `fmtp`
  `mode`-selection attribute key is an external wire-protocol string and was
  deliberately left spelled exactly as the RFC defines it.

### Added

- `hls::MediaPlaylist::parse` + `hls::MasterPlaylist::parse` (issue #717
  slice 1): the symmetric inverse of the existing `to_m3u8()` renderers,
  parsing an m3u8 string back into the same structs — so an LL-HLS client
  can reuse the origin's wire model rather than growing a second one.
  Recognizes `#EXTM3U`/`#EXT-X-VERSION`/`#EXT-X-TARGETDURATION`/
  `#EXT-X-MEDIA-SEQUENCE`/`#EXT-X-DISCONTINUITY-SEQUENCE`/`#EXTINF`/
  `#EXT-X-ENDLIST`/`#EXT-X-DISCONTINUITY`/`#EXT-X-BYTERANGE`/`#EXT-X-MAP`
  plus the LL-HLS client-relevant tags `#EXT-X-PART-INF`,
  `#EXT-X-SERVER-CONTROL` (`CAN-BLOCK-RELOAD`/`PART-HOLD-BACK`/
  `CAN-SKIP-UNTIL`), `#EXT-X-PART` (`DURATION`/`URI`/`INDEPENDENT`/`GAP`/
  `BYTERANGE`), `#EXT-X-PRELOAD-HINT` (`TYPE`/`URI`/byte-range),
  `#EXT-X-RENDITION-REPORT`, `#EXT-X-SKIP` (delta updates), and
  `#EXT-X-STREAM-INF`/`#EXT-X-I-FRAME-STREAM-INF` on the Multivariant side.
  Unrecognized tags are preserved verbatim into `extra_tags` (never an
  error); a malformed known tag (missing required attribute, unparsable
  value) returns the new `Error::HlsParse { line_no, line, reason }`.
  New public types: `ByteRange`, `MapTag`, `PreloadHintType`,
  `RenditionReport`, `SkipInfo`. `MediaSegment` gained `byte_range: Option<ByteRange>`
  and `map: Option<MapTag>`; `PartSpec` gained `byte_range`/`gap`;
  `LowLatencyConfig` gained `can_skip_until`/`preload_hint_type`/
  `preload_hint_byte_range_start`/`preload_hint_byte_range_length`;
  `MediaPlaylist` gained `rendition_reports`/`skip` — all breaking for any
  external struct-literal construction (all five structs now derive
  `Default`, so existing call sites can add `..Default::default()`).
  `to_m3u8()` gained matching render support for every new field
  (`#EXT-X-MAP` dedup/carry-forward, `#EXT-X-BYTERANGE`,
  `#EXT-X-SKIP`, `#EXT-X-RENDITION-REPORT`, `CAN-SKIP-UNTIL`, part
  `BYTERANGE`/`GAP`, preload-hint `TYPE`/byte-range) so the round trip is
  lossless. `#EXT-X-MEDIA` (Multivariant alternate renditions) remains
  unmodeled on both the render and parse side — a documented gap, not a
  silent drop (nothing to preserve it into on `MasterPlaylist`, which has
  no `extra_tags` field).
  - `hls::OpenSegment` (the in-progress LL-HLS segment builder) gained a
    `map: Option<MapTag>` field and an `OpenSegment::with_map` builder, so a
    muxer assembling an open segment can carry forward the Media
    Initialization Section in effect for it, mirroring `MediaSegment::map` on
    the parse side. `OpenSegment::new` still defaults to no map.
  - **Fix** (found while building the `ll-hls-client` tokio adapter, issue
    #717 slice 5): `LowLatencyConfig` gained `can_block_reload: bool` — the
    `CAN-BLOCK-RELOAD` attribute's actual `YES`/`NO` value was previously
    discarded by `parse` (only the tag's *presence* set `low_latency =
    Some(...)`), so a client had no way to distinguish a genuine
    `CAN-BLOCK-RELOAD=NO` origin from one that supports blocking reload —
    both parsed identically. `parse` now reads the real value, defaulting to
    `false` (RFC 8216bis: absent means not supported) when the attribute (or
    the whole `#EXT-X-SERVER-CONTROL` tag) is missing; `to_m3u8()` renders
    the actual value instead of always emitting `YES`.
    `LowLatencyConfig::default()` still defaults to `true` (matching every
    existing in-repo caller that builds one from scratch via
    `..Default::default()`, all of which do support blocking reload) — only
    the *parser's* default differs, deliberately, since it reflects the wire
    rather than a convenience default.

### Security

- **Bounded RTP/TS reassembly buffers** (issue #663 P5.2, audit-ingest
  #4 — memory-exhaustion hardening): two streaming-reassembly buffers
  previously grew without bound against malformed or hostile input.
  `rtp_stream::RtpStreamDepacketiser`'s per-track in-progress access-unit
  buffer (a dropped/corrupted final FU-A fragment, or a marker bit that
  never arrives, previously accumulated RTP packets for the life of the
  session) is now capped at `MAX_AU_BUFFER_BYTES` (4 MiB); on overflow the
  partial AU is dropped and `push` returns the new recoverable
  `Error::BufferCapExceeded`, with internal state already reset so the next
  packet starts a fresh AU. `ts_demux::StreamingTsDemux`'s per-PID PES
  buffer (a `payload_unit_start_indicator` that never recurs — the
  unbounded-video `PES_packet_length = 0` case on a wedged/lossy stream —
  previously accumulated forever) is now capped at `MAX_PES_BUFFER_BYTES`
  (4 MiB); on overflow the partial PES is dropped and a
  `DemuxEvent::Discontinuity` is raised for the PID. Neither cap changes
  behaviour for any well-formed stream (a real access unit/PES is orders of
  magnitude smaller); PSI/private-section buffering needed no equivalent
  change — `mpeg_ts::ts::SectionReassembler` is already inherently bounded
  by `section_length`'s 12-bit field.

## [0.17.0] - 2026-07-15

### Added

- Streaming, timing- and config-aware RTP depayloader `RtpStreamDepacketizer`
  (RFC 6184 H.264 / RFC 3640 AAC): incremental `push`/`flush`, real per-sample
  duration from RTP-timestamp deltas, `is_sync` from IDR detection. v1 assumes
  low-delay H.264 (no B-frame reorder; `composition_offset = 0`), one AAC access
  unit per packet, and in-arrival-order packet feed. `RtpStreamTrack` is
  `#[non_exhaustive]` — construct via `RtpStreamTrack::new`.
- SDP fmtp/rtpmap → `CodecConfig` helpers in `rtp_sdp`, all re-exported at the
  crate root (issue #702): `fmtp_param` — a proper, anchored (no
  substring-match false positives), charset-safe parser for the `a=fmtp:<pt>
  <params>` `;`-separated `key=value` list (RFC 4566 §5.14); full-fmtp-line
  entry points `avc_config_from_fmtp` (RFC 6184 §8.1
  `sprop-parameter-sets` → avcC) and `aac_config_from_fmtp` (RFC 3640 §4.1
  `config` → esds) built on it; and `rtpmap_clock_rate`, parsing the
  companion `a=rtpmap:<pt> <encoding>/<clock>[/<channels>]` clock rate (RFC
  4566 §6). The prior value-level `aac_config_from_fmtp(config_hex)` is
  renamed `aac_config_from_asc_hex` (breaking) to make room for the new
  full-fmtp-line function of the same codec-pair naming; `avc_config_from_sprop`
  is unchanged. This replaces the fragile hand-rolled substring fmtp parsing
  multimux previously needed to do itself.
- `hls::OpenSegment` + `MediaPlaylist::open_segment` (issue #702): renders
  an in-progress, `#EXTINF`-less LL-HLS segment as trailing `#EXT-X-PART`
  lines at the live edge (RFC 8216bis §4.4.4.9), so a live LL-HLS origin can
  publish its still-open trailing segment through the library instead of
  hand-rolling `#EXT-X-PART` lines. Opt-in and gated the same way as the
  existing closed-segment parts: rendered only when `low_latency` is `Some`.
  `MediaPlaylist` gained the `open_segment: Option<OpenSegment>` field
  (breaking for any external struct-literal construction).

## [0.16.0] - 2026-07-14

### Added

- **`CencEncryptor` — CENC/CBCS encrypt path** (issue #564): a new
  `Encrypt` impl that protects a cleartext `Media` in place, the write-side
  counterpart to the existing `CencDecryptor`. Takes an
  `EncryptConfig { scheme, kid, key, iv, pattern, subsample }`: `scheme`
  selects `cenc` (AES-128-CTR, full-block) or `cbcs` (AES-128-CBC,
  `crypt`:`skip` pattern); `iv` is an `IvGen` (`Counter { base }` — an 8-byte
  per-sample counter; `Explicit(Vec<Vec<u8>>)` — one caller-supplied IV per
  sample; `Constant([u8; 16])` — the standard real-world `cbcs` convention,
  `tenc.default_constant_IV` with no per-sample `senc` IV); `subsample`
  selects the per-codec clear/protected byte-range policy (`SubsamplePolicy`).
  Encrypting a track populates `Track::encryption` (`TrackEncryption`
  carrying the resolved `tenc` + a `SampleEncryptionEntry` per sample) for the
  muxer's box emission and the DASH/HLS signalling below to read.
- **`protect_init_segment` / `protect_media_segment` — CENC/CBCS box
  emission** (issue #564): post-processing passes over an already-muxed CMAF
  init/media segment that splice in the CENC/CBCS boxes from a
  `TrackEncryption`, without CENC plumbing through the lower-level
  `TrackSpec`/pipeline: `protect_init_segment` (`init_segment.rs`) rewrites
  the target track's sample entry to `encv`/`enca` + `sinf`
  (`frma`/`schm`/`schi`/`tenc`), recomputing every ancestor box
  (`stsd`/`stbl`/`minf`/`mdia`/`trak`/`moov`) from its typed children;
  `protect_media_segment` (`movie_fragment.rs`, given a `FragmentProtection`
  per protected `traf`) adds the fragment's `senc`/`saiz`/`saio` (`saio`
  anchored `moof`-relative, verified against Bento4's `mp4decrypt` — see the
  e2e entry below). Every other byte, box, and track round-trips unchanged.
- **DASH/HLS CENC/CBCS DRM signalling** (issue #564): both driven straight off
  `Track::encryption` — no DRM logic in either. **DASH**
  (`DashPackager::package`) auto-derives the generic-CENC identification
  element per `AdaptationSet` — `<ContentProtection
  schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"|"cbcs"
  cenc:default_KID="...">` (ISO/IEC 23001-7) — from the set's encrypted
  tracks; `ContentProtectionSystem` gained an optional `pssh: Option<Vec<u8>>`
  field so caller-supplied DRM-system entries
  (`schemeIdUri="urn:uuid:..."`, e.g. built with the `drm` module's pssh
  builders) render a base64 `<cenc:pssh>` child. **HLS** — the new
  `cenc_ext_x_key(scheme, kid, key_uri)` renders
  `#EXT-X-KEY:METHOD=SAMPLE-AES,URI="...",KEYFORMAT="urn:mpeg:dash:mp4protection:2011",KEYFORMATVERSIONS="1",KEYID=0x<kid>`
  for `cbcs`, returning `None` for `cenc` (AES-CTR has no valid HLS `METHOD` —
  `cenc`-protected CMAF is signalled on the DASH side only). A new runnable
  example, `examples/cenc_encrypt.rs`, drives the whole path (demux -> encrypt
  -> mux -> protect -> DASH/HLS signalling) against the real
  `fixtures/ts/h264/main.ts` fixture.
- **CENC/CBCS encrypt-path end-to-end proof** (issue #564): new
  `tests/cenc_encrypt_e2e.rs` exercises the full `CencEncryptor::encrypt` ->
  `CmafMux::package` -> `protect_init_segment`/`protect_media_segment`
  pipeline against `fixtures/ts/h264/main.ts`, then verifies the resulting
  fMP4 two independent ways: a self round-trip through `CencDecryptor`, and a
  golden-interop cross-check against Bento4's real `mp4decrypt` CLI. Both
  `cenc` and `cbcs` pass; the `cenc` case confirms the `saio` moof-relative
  anchor decision against real third-party tooling.

### Fixed

- **`cenc_decrypt`: fragmented CMAF support (`moof`/`traf`/`senc`/`saiz`/`saio`) +
  `cbcs` (AES-CBC pattern) decrypt** (issue #564): `CencDecryptor` previously
  only supported progressive fMP4 (single `moov`/`mdat`, sample layout from
  `stsz`/`stsc`/`stco`) — the real-world fragmented CMAF case (`moov` + one or
  more `moof`/`mdat` pairs, each `traf` carrying its own `senc`/`saiz`/`saio`)
  was entirely unsupported. `CencDecryptor::from_fmp4`/`demux` now walk every
  `moof`, matching each `traf` to its `moov`-declared track by `tfhd.track_id`
  and concatenating every fragment's samples in file order, reusing the
  already-typed fragment parsers in `movie_fragment` (`MovieFragmentBox`,
  `TrackFragmentHeaderBox`, `TrackFragmentRunBox`) rather than a second
  `moof`/`traf`/`trun` walker.
  Also implements the `cbcs` (AES-128-CBC pattern cipher) scheme, previously
  unimplemented (`Decrypt::decrypt` unconditionally rejected any non-`cenc`
  scheme): the `default_crypt_byte_block`:`default_skip_byte_block` pattern is
  applied across a sample's protected bytes, chaining across pattern-skip runs
  within one subsample's protected range and resetting to the sample's seed IV
  at the start of every subsample's protected range (see the next entry — the
  cross-subsample reset rule was corrected after this fix originally shipped),
  with the IV resolved from either the per-sample `senc` entry or the track's
  `tenc.default_constant_IV`.
  New fixtures `fixtures/transmux/h264_cenc.mp4` / `h264_cbcs.mp4` (real,
  fragmented, Bento4 `mp4encrypt`-produced) back
  `tests/cenc_fragmented_fixture.rs`, including a golden-interop cross-check
  against Bento4's own `mp4decrypt`.
- **`cbcs`'s CBC pattern chain now resets per subsample, and `cenc_encrypt`
  gained constant-IV/16-byte-IV support** (issue #564): `cenc_crypto.rs`'s
  `cbcs` pattern cipher (shared by `CencEncryptor` and `CencDecryptor`)
  previously carried its running CBC chain over from one subsample's last
  encrypted block into the *next* subsample's first encrypted block; it now
  resets the chain to the sample's resolved seed IV at the **start of every
  subsample's protected range**, while still chaining correctly *within* one
  subsample's own pattern-skip runs (unchanged, and unchanged for `cenc`'s CTR
  counter, which stays continuous across subsamples as before). Triangulated
  against Bento4's `mp4decrypt` and Shaka Packager (ISO/IEC 23001-7 itself is
  unowned/paid, so the reference implementations are the source of truth): the
  old cross-subsample-continuous chain reproduced only the first protected
  subsample of a multi-subsample sample correctly and silently diverged from
  Bento4 on every later subsample's first crypt block, while still
  round-tripping through this crate's own encrypt/decrypt pair
  (self-consistent, not spec/interop-correct) — undetectable without an
  external oracle. `cenc_encrypt.rs`'s `IvGen` gained a `Constant([u8; 16])`
  variant (the standard real-world `cbcs` convention: `tenc.default_constant_IV`
  + `default_per_sample_iv_size = 0`, no per-sample `senc` IV), and
  `default_per_sample_iv_size` is now derived from the chosen `IvGen` instead
  of a hard-coded `8` (which Bento4's `mp4decrypt` silently no-ops on for
  `cbcs`). `tests/cenc_encrypt_e2e.rs`'s `cbcs` case now uses
  `SubsamplePolicy::Video` (a real per-NAL multi-subsample map — the case that
  exposed the bug) and `IvGen::Constant`, proving both the fix and the
  constant-IV wire convention end to end against the real `mp4decrypt` oracle;
  `tests/cenc_fragmented_fixture.rs`'s existing single-subsample-per-sample
  `h264_cbcs.mp4` regression (which the bug never affected, since it never
  crosses a subsample boundary) remains byte-exact green.

### Security

- **CENC encrypt-path input validation hardening** (issue #564 review): the
  encrypt API previously accepted nonsense configuration and silently shipped
  unprotected or corrupt output instead of erroring. `CencEncryptor::encrypt`
  now rejects: a per-sample `IvGen::Explicit` IV whose length isn't exactly 8
  or 16 bytes (an empty IV previously built an all-zero AES-CTR counter — a
  two-time-pad reusing the same keystream across every sample); a `cbcs`
  pattern with `crypt_byte_block == 0` and a nonzero `skip_byte_block` (the
  shared `cenc_crypto::cbcs_sample`, used by both encrypt and decrypt, now
  guards this — previously it left the "protected" range in cleartext while
  `tenc.default_is_protected` still claimed protection); and a `cbcs` pattern
  component (`crypt_byte_block`/`skip_byte_block`) above 15, which would
  otherwise silently truncate to its low 4 bits when packed into `tenc`
  (ISO/IEC 23001-7 §12.2). Also promotes a `debug_assert` in
  `movie_fragment::protect_media_segment`'s `moof`/`saio`-offset consistency
  check to a real `Error` return (release builds no longer risk silently
  shipping a corrupt `moof`), and tightens that function's doc comment to
  state it protects a single-fragment media segment (one `moof`/`mdat`), which
  is its actual (and the normal CMAF) behaviour.

## [0.15.3] - 2026-07-12

### Added

- **`ssai_ad_stitch` example** (issue #664): a runnable, real-fixture
  end-to-end SSAI (server-side ad insertion) walkthrough wiring
  `scte35-splice` (cue parsing), `timed-metadata` (SCTE-35 -> HLS
  `EXT-X-DATERANGE` / DASH `emsg` conversion), and transmux's own
  `splice_insert` + HLS/DASH packaging together: extracts a hand-built,
  spec-correct `splice_insert` cue from a real MPEG-2 TS PID, converts its
  90 kHz PTS to the base track's own timescale, splices in a stand-in ad
  clip, and renders both an HLS media playlist (`#EXT-X-DISCONTINUITY` +
  `#EXT-X-DATERANGE`) and a DASH MPD (`InbandEventStream` + an inband `emsg`
  box) describing the spliced timeline. `cargo run -p transmux --example
  ssai_ad_stitch`; covered by `transmux/tests/ssai_ad_stitch.rs`, which
  `#[path]`-includes the example and asserts on the exact rendered manifest
  text and a full `emsg` -> SCTE-35 round-trip. Dev-dependency only
  (`scte35-splice`, `timed-metadata`) — no change to transmux's own public
  API or its dependency graph.

### Fixed

- **`splice::splice_insert` mis-scaled the ad/resume cut point on every
  non-anchor track whose media timescale differs from the anchor (video)
  track's** — which is virtually always true for real content (e.g. 90 kHz
  video vs. 44.1/48 kHz audio). The video-timescale tick offset used to be
  passed straight to `sample_index_at_offset` for the audio track without
  any unit conversion, so the audio split landed at the wrong wall-clock
  position (and could run past the end of the audio samples, panicking on
  index-out-of-bounds) on any base media with video and audio at different
  timescales. `splice_insert` now rescales the offset into each track's own
  timescale before searching for its split sample. No existing test exercised
  a multi-track (video + audio) `Media` before this fix — none of
  `transmux/tests/splice.rs`'s cases used more than one track; the new
  `ssai_ad_stitch` example/test (issue #664) exercises a real video+audio
  splice and would have caught this immediately.
- **DASH output emitted every Representation's segments as the full
  multi-track CMAF artefact instead of a genuinely single-track one** (#614).
  `OutputFormat::Dash` muxed one multi-track `CmafMux` blob from all tracks
  and cloned it under every `init-stream{id}.m4s`/`chunk-stream{id}-1.m4s`
  name, so each DASH Representation's segments carried every other track's
  samples too, violating ISO/IEC 23009-1 §5.3.9.1 — invisible to the golden
  gate test's own ffprobe-based checks because they only run against a
  dash-demuxer-capable ffprobe build (rare locally, present on CI, where the
  test was failing). Each track's segments are now muxed from a filtered
  single-track `Media` (`Media::select_tracks_by`), and the golden gate
  `ts_to_dash_mpd_validated` test gained its own single-track segment
  assertion (`assert_single_track_matches_source`) to keep catching this
  class of regression instead of loosening the existing multi-track check.

### Changed

- **Internal only, no public API change**: the RTCP control-packet codec
  (`SenderReport`/`ReceiverReport`/`SourceDescription`/`Bye`/`App`/
  `RtcpPacket`/`CompoundPacket` and friends) moved to the new standalone
  [`rtcp-packet`](https://crates.io/crates/rtcp-packet) crate (issue #654,
  part of epic #653 — the same extraction `rtp-packet` went through in
  #646/#650). `transmux::rtcp` is now a thin `pub use rtcp_packet::*;`
  re-export, so every `transmux::rtcp::*` and crate-root `transmux::*` call
  site keeps working unchanged; existing RTCP tests
  (`transmux/tests/rtcp.rs`) pass byte-for-byte unchanged. Not released as
  its own transmux version bump (same precedent as the rtp-packet
  migration).

## [0.15.2] - 2026-07-09

### Fixed

- **DVB `stream_type 0x06`/`0x15` Dolby/DTS audio never classified past
  opaque data** (#641). AC-3/E-AC-3/DTS carried the standard DVB way --
  `stream_type 0x06` (PES private data) or `0x15` (metadata in PES) plus an
  AC-3 (`0x6A`), enhanced AC-3 (`0x7A`), or DTS (`0x7B`) ES_info descriptor,
  per ETSI EN 300 468 -- fell through to opaque `CodecConfig::Data` and was
  silently dropped from HLS/fMP4 output, exactly like the native
  `0x81`/`0x87`/`0x8*` stream_types would have been recognised. The PMT
  parser now consults the ES_info descriptor loop for those two
  `stream_type`s and reclassifies to the matching audio codec, reaching the
  existing `ConfigProbe::Ac3`/`Eac3`/`Dts` syncframe recovery unchanged.

## [0.15.1] - 2026-07-07

### Fixed

- **MPEG audio / ADTS frame splitting never resynced past a bad sync**
  (#638). `split_mpeg_audio_frames` and `split_adts_frames` assumed a PES
  payload always starts exactly on a frame boundary; a real DVB-S broadcast
  multiplexer routinely splits PES payloads without regard to audio frame
  length, so a misaligned payload silently yielded zero frames (a track
  stuck in `Probing` forever if no buffered PES happened to align, or
  silently dropped samples on an already-live track). Both splitters, and
  the MP2/AAC config-probe backlog scans, now resync forward to the next
  valid frame header instead of bailing on the first byte that isn't one.

## [0.15.0] - 2026-07-06

### Added
- **Late-resolving live tracks: `DemuxEvent::TracksResolved` +
  `StreamingTsHlsSegmenter::add_track`** (#624). A live MPEG-TS feed resolves
  `DemuxEvent::TrackAdded` incrementally per PID (`ts_demux.rs`'s
  `Probing`→`Parked`/`Live` lifecycle), and an audio PID's first frame
  commonly parses after the first video keyframe — a consumer that built
  `StreamingTsHlsSegmenter` at the first video keyframe therefore got a
  permanently video-only segmenter with silently no audio, and no way to fix
  it. `StreamingTsDemux` now emits `DemuxEvent::TracksResolved` once every
  currently-known PMT-declared PID has left `Probing` (no PID stuck
  probing), re-arming (and firing again) if a later PMT version bump adds a
  new PID that itself then resolves — de-duplicated against a
  known-PID-count high-water mark so it never fires once per repeated PMT
  section or per packet on an already-stable track set.
  `StreamingTsHlsSegmenter::add_track(spec)` registers a track after
  construction: errors on a `track_id` collision; otherwise, if nothing has
  been cut or buffered yet (`total_segments == 0`, every track's `pending`
  empty) and the newly-added track is AVC while the current anchor isn't, the
  anchor (and its target-duration tick count) is recomputed to the new video
  track — recovering the construction-time "first video, else first track"
  rule for the case this issue targets — otherwise the existing anchor and
  already-cut segments are left untouched and the track simply joins future
  muxing. Segments cut before `add_track` have no PES data for the new track
  (spec-legal — ISO/IEC 13818-1 §2.4.4.8 permits a PMT to declare a track
  with zero samples in a given segment); every segment cut after `add_track`
  carries it correctly in both the PMT and the PES.
- **Streaming Annex B → access-unit splitter** (`au::AccessUnitSplitter`,
  `au::split_access_units`) (#601). An IP-camera SoC encoder emits a continuous
  Annex B byte stream (ITU-T H.264 Annex B) with no TS/PES framing; the on-camera
  LL-HLS origin path needs to cut it into access units incrementally as bytes
  arrive. The new splitter buffers pushed bytes and emits each complete access
  unit at the next AU boundary — an access-unit delimiter (AVC 9 / HEVC `AUD_NUT`
  35 / VVC `AUD_NUT` 20), a VCL NAL that is the first slice of a new picture
  (AVC `first_mb_in_slice`==0 / HEVC `first_slice_segment_in_pic_flag`==1), or a
  non-VCL NAL following a VCL (H.264 §7.4.1.2.4). Codec-aware (AVC/HEVC; VVC on
  AUD/non-VCL boundaries only), `no_std`, and byte-exact: concatenating the
  emitted units reproduces the input from its first start code. Complements the
  existing per-NAL `annexb::iter_annexb_nals` and the private, one-shot AUD-only
  splitter in `ps_demux`.
- **MPEG-H 3D Audio MPEG-2 TS carriage** (#579): `ts_demux.rs` recognises PMT
  `stream_type 0x2D` (ISO/IEC 13818-1 Table 2-34 / ETSI TS 101 154 §6.8) as
  MPEG-H Audio, parses the MHAS elementary-stream packet framing
  (`transmux::mpegh`'s new packet walker — the three-tier "escaped value"
  header coding, empirically verified against a real Fraunhofer
  MPEG-H-in-TS fixture; see `transmux/docs/codec/mpegh-ts-101154.md`) to
  locate the `PACTYP_MPEGH3DACFG` packet's opaque `mpegh3daConfig()` payload,
  and builds a `CodecConfig::MpegH` track from it — one opaque `Sample` per
  MHAS access unit, `is_sync` set from whether the access unit is a
  random-access point (ETSI TS 101 154 §6.8.4.1). `ts_mux.rs` gains
  `EsKind::MpegH` (stream_type `0x2D`, MHAS passthrough) and synthesizes the
  PMT `MPEG-H_3dAudio_descriptor` (`0x3F` extension descriptor) from the
  track's `mpegh3daProfileLevelIndication`. `referenceChannelLayout` /
  `channel_count` / `sample_rate` are not derivable from TS-layer signalling
  alone (would require decoding the opaque `mpegh3daConfig()` bitstream,
  ISO/IEC 23008-3, paid) and are left as documented `0`/"unspecified"
  placeholders; sample timing is unaffected (durations anchor on the 90 kHz
  TS clock, not an audio sample count). No MPEG-H audio bitstream decode —
  config/sample passthrough only, mirroring the existing AC-3/DTS TS
  carriage and the crate's existing ISOBMFF `mha1`/`mhaC` support. Gated on
  the private `private/fixtures/ts/mpegh-cicp01-baseline.ts` fixture
  (`transmux/tests/mpegh_ts.rs`, skips cleanly when the private submodule
  isn't checked out).
- **`nal::caption_cc_data`** — extract ATSC A/53 caption SEI (in-band
  CEA-608/708 carriage) from an H.264/HEVC access unit (#599, follow-up to
  #595's SEI machinery). Walks every NAL, finds each
  `user_data_registered_itu_t_t35` SEI message (H.264 type 6 / HEVC
  prefix/suffix types 39/40, `payloadType` 4) matching the ATSC A/53 §6.2.3
  signature (country `0xB5`, provider `0x0031`, `user_identifier` `"GA94"`,
  `user_data_type_code` `0x03`), and returns the concatenated
  `MPEG_cc_data()` bytes (`cc_data()` + trailing `marker_bits`) in AU order —
  the same wire form the PES-carried `cc_data()` path already produces, ready
  for `cc_data::CcData::parse`. Reuses `recovery_point_sei`'s SEI RBSP /
  EBSP-unescape / `payloadType`-varint walk (issue #595). Works on IR sample
  bytes (length-prefixed or Annex B); no `ts_demux.rs` change. Validated
  against a real ATSC A/53 caption SEI captured byte-for-byte from
  `samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2885/transformers_EIA608_H264.ts`
  (fetched on demand into `.test-streams/`, see
  `tools/fetch-test-streams.sh transformers-eia608-h264`) plus a full
  `TsDemux` round trip on the same capture.
- **Player-validated golden gate** (#569): `tests/golden_gate.rs` packages the
  real `fixtures/ts/h264_aac.ts` fixture through `transmux::cli::run_bytes`
  (the same code path the `transmux` binary uses) into CMAF/fMP4, progressive
  MP4, classic TS-HLS (segment + `.m3u8` playlist), and DASH MPD, then hands
  each artefact to an **independent** decoder — `ffprobe` — asserting it
  parses cleanly and reports the same track count / codec / dimensions /
  sample-rate as the source fixture's own `ffprobe` identification. Every
  other transmux test proves round-trip symmetry against the crate's own
  parsers only; this closes that self-referential gap with an external
  oracle. DASH falls back to a structural MPD check + per-segment `ffprobe`
  when the local `ffprobe` build lacks the `dash` demuxer (common — it needs
  libxml2); TS-HLS additionally resolves the whole playlist through
  `ffprobe`'s `hls` demuxer when present. `ffprobe` availability (and
  specific demuxers) is probed at runtime and each case skips cleanly with a
  printed reason when the tool is absent, so `cargo test` stays green without
  ffmpeg installed. A `mutated_cmaf_output_fails_the_gate` self-test proves
  the oracle bites (a truncated artefact must fail `ffprobe`, not pass). New
  non-blocking `golden-gate` CI job (`.github/workflows/ci.yml`) installs
  ffmpeg and runs the harness.

### Fixed
- **Reverted the #629 `#EXT-X-DISCONTINUITY-SEQUENCE` change — the original
  eviction-based bookkeeping was correct; the "fix" was not.** #629 diagnosed
  the header as reflecting the window's leading segment's discontinuity-count
  one cut too late, and changed it to stamp each segment's absolute count at
  cut time. A pre-tag audit re-derived RFC 8216 §6.2.1's literal definition —
  *"a segment's Discontinuity Sequence Number is the value of the
  EXT-X-DISCONTINUITY-SEQUENCE tag (or zero if none) plus the number of
  EXT-X-DISCONTINUITY tags in the Playlist preceding the URI line of the
  segment"* — and found the "fix" double-counts: the inline `#EXT-X-
  DISCONTINUITY` tag rendered on a discontinuous segment (while it's still in
  the window) already accounts for that segment's boundary, so advancing the
  header *at the same time* makes every segment still sharing the window with
  it compute a Discontinuity Sequence Number one too high — e.g. window
  `[s2, s3]` with `s2` discontinuous: the "fix" reported both segments' true
  client-computed DSN as `2` instead of `1`. The original eviction-based
  logic (advance the header only once a discontinuous segment rolls *off* the
  window, so its own inline tag stops rendering) is exactly correct per the
  spec's formula and was verified stable at every window state. Restored it.
  The regression test (`transmux/tests/streaming_tshls.rs`) now computes each
  segment's *client-observable* DSN (header + preceding inline tags), not the
  raw header integer in isolation — the isolation-only check is what let the
  wrong "fix" pass its own test. Batch `TsHlsPackager::package` was, and
  remains, unaffected (no rolling window; always emits
  `discontinuity_sequence: 0`).
- **TS mux silently dropped HEVC/MPEG-2-video/MPEG-1/2-audio tracks** (#627).
  `ts_mux::EsKind::from_config` only mapped `CodecConfig::Avc`/`Aac`/`Ac3`/
  `Eac3`/`Dts`/`MpegH`/`Data` to a carriable elementary stream — every other
  codec fell to `None` and `plan_elementary_streams` skipped it ("uncarriable
  codec: skip, never fatal"), so a HEVC/MPEG-2-video/MPEG-1/2-audio track was
  silently absent from TS and TS-HLS output instead of degraded. Added
  `EsKind::Hevc` (stream_type `0x24`), `EsKind::Mpeg2Video` (stream_type
  `0x02`, raw ES passthrough), and `EsKind::MpegAudio` (stream_type `0x03`/
  `0x04` selected from the recovered `esds` `objectTypeIndication`, raw frame
  passthrough) — ISO/IEC 13818-1 Table 2-34. HEVC access units get the same
  independently-decodable guarantee AVC already had: a new
  `build_hevc_annexb_au` (mirroring `build_annexb_au`, HEVC's 2-byte NAL
  header and VPS(32)/SPS(33)/PPS(34) types) prepends the track's VPS/SPS/PPS
  (new `EsPlan::hevc_vps_sps_pps`, recovered from `hvcC` in AU order) to any
  keyframe access unit that does not already carry its own SPS. VVC/AV1/VP9
  are not modeled as `CodecConfig` variants in a way this container layer
  carries into TS today and remain out of scope for this fix (`EsKind` has no
  mapping for them, same as before).
- **Anchor-track selection only recognised AVC as video** (#628).
  `ts_hls::choose_anchor` and `Segmenter::new` both picked the segmentation
  anchor by matching `CodecConfig::Avc` only, so any other video codec (HEVC,
  MPEG-2 video, VVC, AV1, VP9, VP8) was never chosen as the anchor — falling
  back to "first track", which is wrong whenever video isn't track 0. Added
  `CodecConfig::is_video` (mirrors the existing `is_audio`, covering every
  video variant the enum defines) and switched both call sites to it.
  `StreamingTsHlsSegmenter::add_track` (#624) had a *third*, undisclosed
  AVC-only anchor-promotion check that #628 missed — a late-resolving HEVC
  (or any non-AVC video) track added via `add_track` never got promoted to
  anchor, silently reintroducing the audio-anchors-and-video-never-cuts bug
  for exactly the codec #627 exists to carry. Found by a pre-tag audit;
  switched to `is_video()` there too.

## [0.14.0] - 2026-07-04

### Fixed
- **Open-GOP AVC access units now anchor segmentation, not just IDR** (#595).
  Broadcast H.264 is frequently open-GOP — no IDR (NAL type 5) at all; each
  GOP opens instead with SPS(7)/PPS(8) + a non-IDR I-slice, usually announced
  by a `recovery_point` SEI (ITU-T H.264 Annex D.1.7/D.2.7). `is_keyframe_nal`
  only matched IDR, so `TsDemux`/`StreamingTsDemux` set `Sample.is_sync` to
  `false` on every access unit of such a stream, and `Segmenter` never found
  an anchor — it buffered the entire input into one giant segment.
  `transmux::nal` gains `recovery_point_sei` (parses a type-6 SEI NAL's
  `sei_message()` `payloadType`s) and `access_unit_is_rap`, which recognises
  an AVC access unit as a random-access point on IDR **or** a
  `recovery_point` SEI **or** (pragmatic open-GOP fallback) an SPS in the
  access unit; HEVC/VVC keyframe detection is unchanged (their IRAP range
  already covers CRA/BLA). `video_sample_bytes` in `ts_demux.rs` now sets
  `is_sync` via this access-unit-level check for AVC, so both `TsDemux` and
  `StreamingTsDemux` benefit. Segments opened this way are non-IDR — correct
  for open-GOP decode and DASH-IF/CMAF-acceptable, but not a strict
  ISO/IEC 14496-12 "clean" sync sample. `is_keyframe_nal`/
  `access_unit_is_keyframe` keep their strict IDR-only meaning for existing
  callers.

## [0.13.0] - 2026-07-04

### Added
- **DASH MPD generation — `$Time$`/`SegmentTimeline` addressing + live/content
  extensions** (#566), extending the existing `DashPackager`:
  - `Addressing` (`Number` default / `Timeline`) + `DashPackager::segments`
    (`Vec<TrackSegments>`, caller-supplied per-track segment durations, e.g.
    from `Segmenter`): `Addressing::Timeline` emits `$Time$` addressing with
    an explicit `<SegmentTimeline>` of run-length-encoded `<S t= d= r=>`
    entries (ISO/IEC 23009-1 §5.3.9.6); `Addressing::Number` now also accepts
    `segments` to use a real per-segment nominal `@duration` instead of the
    whole-track total, while staying unchanged when `segments` is empty.
  - Live-profile MPD attributes: `DashPackager::publish_time`,
    `time_shift_buffer_depth`, and `suggested_presentation_delay` (alongside
    the existing `availability_start_time`/`minimum_update_period`),
    ISO/IEC 23009-1 §5.3.1.2 Table 3.
  - Every `AdaptationSet` now carries a `Role` (`urn:mpeg:dash:role:2011`,
    `main`, §5.8.5.5) and, when every `Representation` agrees, an inherited
    `@lang` resolved from a TS-sourced audio track's
    `ISO_639_language_descriptor` (ETSI EN 300 468 §6.2.19) in
    `TrackSpec::es_info_descriptors`.
  - `DashPackager::content_protection` (`Vec<ContentProtectionSystem>`) — a
    `<ContentProtection>` hook (§5.8.4.1), optionally carrying
    `cenc:default_KID` (ISO/IEC 23001-7); full CENC `pssh` carriage remains a
    separate epic.
  - `DashPackager::inband_event_streams` (`Vec<InbandEventStream>`) —
    `<InbandEventStream>` (§5.3.3 / §5.10.3.3) on the video `AdaptationSet`
    for an inband `emsg` scheme/value.

## [0.12.0] - 2026-07-04

### Breaking
- `TrackSpec` gains two fields, `source_pid: Option<u16>` and
  `es_info_descriptors: Vec<u8>` (#582) — external struct literals must be
  migrated to the new `TrackSpec::new(track_id, timescale, config)`
  constructor (+ `.with_source(pid, descriptors)` to attach TS provenance).
- `CodecConfig`, `Sample`, and `TrackSpec` are now `#[non_exhaustive]` (#580,
  the crate-wide convention already applied to most other public config/error
  enums). External code can no longer build these with a struct/variant
  literal or exhaustively `match`/destructure without a wildcard arm:
  - `Sample` — construct via `Sample::new(data, duration, is_sync,
    composition_offset)` (the new general-purpose constructor),
    `Sample::from_annexb`, or `Sample::from_raw`, then `.with_source_timing(t)`
    if needed.
  - `TrackSpec` — construct via `TrackSpec::new(...)` /
    `.with_source(...)` (see above).
  - `CodecConfig` — exhaustive external `match`/`if let` sites need a
    trailing `_ =>` (or `other =>`) arm.

### Fixed
- **`avcC` high-profile extension gate + population** (#563, #582): the
  `AVCDecoderConfigurationRecord` extension gate
  (chroma_format/bit_depth_luma_minus8/bit_depth_chroma_minus8) checked
  `profile_idc ∈ {100,110,122,144}`; `144` was a pre-Amendment-3 placeholder,
  finalized by ITU-T H.264 as **profile_idc 244** ("High 4:4:4 Predictive").
  Fixed to the shared `sps::is_high_profile` set. Additionally, `TsDemux` had
  hardcoded those three fields to `None` when recovering `avcC` from a TS AVC
  stream, so High 10/4:2:2/4:4:4 lost chroma/bit-depth end-to-end — now
  populated from the SPS. (Verified against `fixtures/ts/h264/high*.ts`.)

### Added
- **Streaming/incremental classic-HLS segmentation for live input** (#571):
  `TsHlsPackager::package` needs the whole `Media` up front, which does not
  fit an unbounded live feed. `StreamingTsHlsSegmenter` is the incremental
  analogue, mirroring `Segmenter`'s CMAF push/flush model: `push(track_id,
  sample)` buffers one coded sample at a time and returns a finished `.ts`
  `TsSegment` whenever the anchor track crosses a keyframe past the target
  duration (byte-identical to the corresponding `TsHlsPackager::package`
  segment for the same input), `finish()` flushes the trailing partial
  segment, and `playlist()` renders a rolling media playlist over a
  configurable sliding window — advancing `#EXT-X-MEDIA-SEQUENCE` and
  `#EXT-X-DISCONTINUITY-SEQUENCE` as older segments roll off, and omitting
  `#EXT-X-ENDLIST` until `finish` has been called. `mark_discontinuity()`
  marks the next cut as an `#EXT-X-DISCONTINUITY` (e.g. on an upstream
  PID/PCR reset). The batch packager and the streaming segmenter now share
  one anchor-selection/cut-decision/duration implementation so the two paths
  cannot silently drift apart.
- **Origin PID + PMT ES_info descriptors on every TS-demuxed track** (#582):
  a DVB player track-picker can now select/label tracks by PID and by
  ES_info descriptor (ISO-639 language `0x0A`, DVB subtitling `0x59`, E-AC-3
  `0x7A`, …) without running its own parallel PAT/PMT parser.
  - `TrackSpec::source_pid` — the source elementary-stream PID, populated for
    every `StreamingTsDemux`/`TsDemux`-produced track (codec **and** opaque
    `CodecConfig::Data`, not just `Data` as before); `None` for non-TS
    sources (fMP4/FLV/WebM/PS/RTP).
  - `TrackSpec::es_info_descriptors` — the verbatim PMT ES_info
    descriptor-loop bytes (ISO/IEC 13818-1 §2.4.4.8) for that elementary
    stream, for every TS-demuxed track; empty for non-TS sources. transmux
    does not parse these — consumers use `dvb-si`.
  - `TrackSpec::new(track_id, timescale, config)` — the new constructor
    every non-TS demuxer/transform now builds a spec with; `.with_source(pid,
    descriptors)` attaches TS provenance (builder style).
- **Progressive (non-fragmented) MP4 demux** (#561): `ProgressiveDemux`
  (`Unpackage<Input = &[u8]>`) parses a single-file, non-fragmented `.mp4` —
  `moov` sample tables, no `moof` — into the crate's `Media` IR, the
  file-on-disk counterpart to the fragmented `Fmp4Demux` and the demux side
  of the existing `ProgressiveMux`. Reuses `Fmp4Demux`'s `stsd` →
  `CodecConfig` reconstruction verbatim; per-sample decode duration and
  composition offset come from `stts`/`ctts` (ISO/IEC 14496-12:2015
  §8.6.1.2/§8.6.1.3, v0 unsigned / v1 signed), sync flags from `stss`
  (§8.6.2, absent ⇒ all sync), and each sample's coded bytes are sliced
  directly out of the input via `stsc` + `stco`/`co64` chunk offsets
  (§8.7.4/§8.7.5, already file-absolute) and `stsz` sizes (§8.7.3). Verified
  against `fixtures/transmux/h264_aac_prog.mp4` (real interleaved,
  multi-chunk H.264 High + AAC-LC capture) with sample counts/PTS/DTS/sync
  flags cross-checked against `ffprobe -show_packets -ignore_editlist 1`,
  and a demux → `CmafMux` → `Fmp4Demux` round-trip proving byte-identical
  sample data and preserved codec config.

### Internal
- `tests/label_coverage.rs` drift-guard (#580): fails CI if any new public
  spec/field enum in `transmux/src/` lacks a `name()` + `Display` impl (the
  issue #204 convention), mirroring the guard already run in `dvb-si` and
  other crates. `ColourType` (`src/visual_ext.rs`) gained `name()` +
  `Display` as part of closing the existing gap the guard found.
- **H.264/HEVC profile-matrix hardening test** (#563):
  `tests/sps_profile_matrix.rs` — table-driven, ffprobe-oracle-backed
  coverage of `decode_avc_sps`/`decode_hevc_sps` + the `TsDemux` → `CmafMux`
  TS→IR→fMP4 path across Baseline/Main/High/High10/High422/High444/
  High-1080-cropped/interlaced + HEVC Main/Main10 (profile/level/dims/
  chroma/bit-depth/interlace/fps vs real fixtures).

## [0.11.0] - 2026-07-04

### Breaking
- **New public API surface is additive but two changes are source-breaking**
  (0.x minor bump per Cargo SemVer): `CodecConfig::Data` gained a `carriage:
  DataCarriage` field (external construct/match sites must add it), and `Sample`
  gained a `source_timing: Option<SourceTiming>` field (external struct literals
  must set it — use the `Sample::from_*` builders + `with_source_timing`). The
  new `DataCarriage` enum is `#[non_exhaustive]`.

### Added
- **Lossless carriage of ANY MPEG-2 TS elementary stream** (#576): TS→IR→TS
  is no longer limited to a hardcoded stream_type allowlist.
  - `CodecConfig::Data` gains a `carriage: DataCarriage` field
    (`Pes`/`Sections`, with `name()`/`Display`) recording whether the
    elementary stream carries PES packets or PSI/private sections
    (ISO/IEC 13818-1 §2.4.4.8 / Table 2-34).
  - **TS → IR demux**: `Codec::from_stream_type` now returns an opaque
    `CodecConfig::Data` for **every** `stream_type` it does not decode to a
    typed codec (never `None`/dropped). A fixed section-carried set (`0x05`
    private_sections, `0x0A`-`0x0D` DSM-CC, `0x14` DSM-CC synchronized
    download, `0x86` SCTE-35/ANSI-scoped) is reassembled via a
    `mpeg_ts::ts::SectionReassembler` instead of a PES assembler — each
    complete section becomes one `Sample` with no PTS/DTS
    (`source_timing: None`); every other stream_type (including the
    pre-existing `0x06`/`0x15` carriage) is PES-reassembled as before.
  - **IR → TS mux**: `EsKind::Data { stream_type, carriage }` re-emits the
    preserved `stream_type` verbatim; a PES-carried Data track is wrapped in
    a `private_stream_1` (`0xBD`) PES packet, payload pass-through; a
    section-carried Data track's samples are re-emitted directly via
    `mpeg_ts::mux::SectionPacketizer`, never PES-wrapped. `build_pmt_section`
    now writes each ES's preserved `ES_info` descriptor bytes into the PMT
    (previously always empty) so a receiver can identify a carried stream
    (e.g. its DVB subtitling/teletext descriptor); `program_info` stays
    empty. The classic TS-HLS packager (`TsHlsPackager`, built on the same
    `ts_mux` machinery) carries every such stream for free, re-emitting a
    complete PMT (every ES + its descriptors) at the start of every segment.
  - Fixed a latent packet-interleaving bug in `TsMux`/`TsHlsPackager`
    (`ts_mux::mux_tracks_at`) exposed by this change: the global
    packet-interleave sort previously keyed on the on-wire, 33-bit-wrapped
    DTS, which could reorder a single track's own packets against each other
    once its cumulative decode time crossed the wrap point (only reachable
    once an opaque `CodecConfig::Data` track — whose recovered per-sample
    durations are untrusted input — could reach the TS mux path at all). The
    interleave key is now a separate, never-wrapped monotonic value.
  - **fMP4/CMAF mux**: `CodecConfig::Data` tracks have no ISOBMFF sample
    entry, so `build_init_segment` (and therefore `CmafMux`, `ProgressiveMux`,
    `Segmenter`, `LlSegmenter`, `LlHlsSegmenter`) now **skips** them
    gracefully instead of failing the whole mux with `UnsupportedCodec` — a
    TS multiplex mixing carriable and opaque streams now produces a valid
    fMP4/CMAF output for its carriable (video/audio) tracks.
- **DTS Transport-Stream spoke** (#560): DTS is no longer dropped on the TS
  side.
  - **TS → IR demux**: PMT `stream_type` `0x82`/`0x85`/`0x8A` now resolves a
    `CodecConfig::Dts` track instead of being silently skipped. The new
    `dts::DtsCoreFrameInfo::from_es` parses a DTS **core substream** frame
    header (ETSI TS 102 114 §5.3/§5.4, sync `0x7FFE8001`) — sample rate
    (Table 5-5), channel count (Table 5-4 + LFE), samples/frame
    (`32×(NBLKS+1)`) — and `into_ddts()` builds a core-only `ddts`
    `DtsSpecificBox` from it (§E.2.2.3.2, Tables E-2/E-3), mirroring the
    existing AC-3/E-AC-3 recovery path. Each PES access unit is split into
    individual core frames (`dts::split_dts_core_frames`, using each frame's
    own `FSIZE`) and emitted as one `Sample` with interpolated per-frame
    `SourceTiming`, the same pattern as E-AC-3 syncframe splitting (#556).
  - **IR → TS mux**: new `EsKind::Dts` (`stream_type` `0x82`, ETSI TS 101 154
    §G) — a `CodecConfig::Dts` track is now emitted to TS (PES payload
    passthrough) instead of being dropped.
- **Event-driven streaming TS demuxer** (#555): `StreamingTsDemux` is a new
  incremental MPEG-2 TS demux core — `feed()` accepts bytes of any size or
  alignment (down to one byte at a time; resynchronises via
  `mpeg_ts::resync::TsResync`), `poll_event()` drains `DemuxEvent`s
  (`TrackAdded`/`TrackUpdated`/`Sample`/`Pcr`/`Discontinuity`) as they become
  known, and `finish()` flushes trailing partial access units. `TsDemux` is
  now a thin batch wrapper over it (feed the whole buffer, `finish()`, fold
  the event stream into a `Media`) — there is no separate whole-buffer demux
  implementation; every existing `TsDemux` behaviour (per-sample
  `SourceTiming`, AC-3/E-AC-3 syncframe splitting, opaque `Data` tracks, PCR
  collection, 33-bit wrap-unroll) is produced by the streaming core.
  Codec-config recovery is single-shot and incremental (mirrors the old
  whole-file `find_map` scans, applied access-unit-by-access-unit); track IDs
  and `TrackAdded` order still follow PMT declaration order (codec tracks
  first, then data tracks, each group in PMT order), independent of which
  PID's config happens to resolve first. Memory is bounded independent of
  stream length (per-PID PES/PSI reassembly state, one pending sample per
  live video/data track, small per-PID config-recovery backlogs, and a
  FIFO-capped pre-PMT `unattributed`-payload buffer for PIDs whose own packets
  arrive before their PMT registration completes — so a full-multiplex live
  feed with unrelated-service PIDs that never appear in the followed PMT stays
  bounded regardless of stream length; see the `StreamingTsDemux` doc comment
  for details.
- **Per-sample source-container timing** (#556): `Sample` gains
  `source_timing: Option<SourceTiming>` (`SourceTiming { pts, dts }`, the
  33-bit-unwrapped 90 kHz PES clock for TS sources) and a
  `with_source_timing` builder; every in-crate `Sample` constructor/literal
  updated. All mux paths (`build_media_segment`/`CmafMux`) ignore the field —
  fMP4 output timing stays duration-based.
  - `TsDemux` now sets `source_timing` on every video (H.264/HEVC/MPEG-2) and
    audio (AC-3/E-AC-3/AAC/MPEG audio) sample it emits: the first frame taken
    from a PES access unit carries that PES's unwrapped PTS/DTS exactly;
    subsequent frames split out of the same PES payload get interpolated
    timing (`pes_pts + i * samples_per_frame * 90000 / sample_rate`, floored,
    `u128` math).
  - **AC-3 syncframe splitting**: `ts_demux` now splits each PES payload into
    individual AC-3 syncframes (`ac3::split_ac3_syncframes`, using the
    Table 4.13 frame-size table — ETSI TS 102 366 §4.4.1.4 — via the new
    `Ac3SyncframeInfo::frame_len_bytes()`) instead of emitting one
    zero-duration `Sample` per PES access unit. Every syncframe gets
    `duration = AC3_SAMPLES_PER_SYNCFRAME` (1536 = 6 blocks × 256
    samples/block, §4.3.0 `syncframe()`).
  - **E-AC-3 syncframe splitting**: `ac3::split_eac3_syncframes` splits each
    PES payload into access units; a dependent-substream syncframe
    (`strmtyp == 0x1`, Annex E §E.1.2.2 `bsi()`) is concatenated into the
    preceding independent syncframe's access unit. `duration = numblks * 256`
    from the independent frame.
  - A previously TS-sourced AC-3/E-AC-3 track muxed through `CmafMux` no
    longer produces all-zero `trun` sample durations.
- **Opaque PES data tracks** (#557): PMT `stream_type` 0x06 (PES private
  data — DVB subtitles/teletext/SMPTE 2038/etc.) and 0x15 (metadata in PES)
  are now carried into the IR as a new `CodecConfig::Data { stream_type,
  descriptors }` variant (`descriptors` is the raw PMT ES_info
  descriptor-loop bytes) instead of being silently dropped. Mirrors the
  existing `CodecConfig::Vp8`/`Vorbis` WebM-only precedent: carried in the IR
  for inspection / `{TS} → IR → {TS}`, but has no ISOBMFF sample entry, so
  `build_trak`/`CmafMux` reject it with the same `Error::UnsupportedCodec`
  every other site that dispatches exhaustively on `CodecConfig` (`dash.rs`
  RFC 6381 codec string, `flv.rs` codec name, `splice.rs` codec kind) gained
  a matching `Data` arm.
  - `TsDemux` builds one Data track per opaque PES elementary stream: one
    `Sample` per PES access unit (verbatim payload bytes), `timescale =
    90_000`, `is_sync = true`, `composition_offset = 0`, `source_timing` from
    the unwrapped PES PTS/DTS, `duration` = the delta to the next access
    unit's unwrapped PTS (last sample reuses the previous duration). Data
    tracks are ordered after every codec track, in PMT order.
- **PCR timeline** (#557): `Media` gains `pub pcr: Vec<PcrSample>`
  (`PcrSample { pcr_27mhz, pid, packet_index, discontinuity }`, ISO/IEC
  13818-1 §2.4.3.4/§2.4.3.5) and a `with_pcr` builder; empty for every
  demuxer except `TsDemux`, which now collects every PCR observation from
  every TS packet's adaptation field (via `mpeg_ts::ts::TsPacket::
  adaptation_field()`), in packet order.
### Fixed
- `TsDemux`'s 33-bit PES-clock wrap-unroll (`unwrap_ts`/`decode_order`/the
  new `unwrap_all`) could misread a stream's first *genuine* PTS/DTS as a
  spurious backward wrap when an earlier access unit on the same elementary
  stream carried no PES header timing at all (its PTS/DTS default to a
  placeholder `0`) — most commonly hit by opaque PES data streams (#557),
  whose access units are sometimes sparse "heartbeat" PES packets with no
  timing. `push_access_unit` now falls back to the *previous* access unit's
  resolved timestamps (rather than a hardcoded `0`) when a PES carries
  neither PTS nor DTS, and the wrap-unroll itself defers wrap-jump detection
  until each of the PTS/DTS channels has seen its first genuine (non-`0`)
  value.

## [0.10.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.
### Added
- **HEVC SPS VUI timing fields** (#546): `HevcSpsInfo` gains three new fields —
  `num_units_in_tick: Option<u32>`, `time_scale: Option<u32>`, and
  `fps: Option<f32>` — mirroring the AVC equivalents added in #523.
  `decode_hevc_sps` now walks the full HEVC SPS syntax (ITU-T H.265 §7.3.2.2.1)
  past the mandatory fields to parse `vui_parameters()` (§E.2.1) and extract
  `vui_num_units_in_tick`/`vui_time_scale`.  Frame rate is derived as
  `vui_time_scale / vui_num_units_in_tick` (no factor-of-2 — HEVC, unlike H.264,
  expresses the tick rate directly).  All three fields are `None` when
  `vui_parameters_present_flag` or `vui_timing_info_present_flag` is 0, or when
  the SPS is truncated before the VUI.  The `Eq` derive is dropped from
  `HevcSpsInfo` (now `PartialEq` only) to accommodate the `f32` field, matching
  the pattern established for `AvcSpsInfo`.
- **HLS Sample-AES + full-segment AES-128 encryption** (#479): new `sample_aes`
  module (feature `sample-aes`) implementing Apple's HLS-native content
  protection — distinct from CENC — per Apple's "MPEG-2 Stream Encryption Format
  for HTTP Live Streaming" (`transmux/docs/drm/hls-sample-aes.md`) and RFC 8216
  §4.3.2.4. All crypto stays feature-gated; the default `no_std` core build pulls
  none.
  - **AES-128 full segment** (`METHOD=AES-128`): `aes128_encrypt_segment` /
    `aes128_decrypt_segment` — AES-128-CBC over the whole segment with PKCS#7
    padding.
  - **H.264 SAMPLE-AES**: `h264_encrypt_nal` / `h264_decrypt_nal` — encrypts only
    NAL types 1 and 5 longer than 48 bytes, with a 32-byte clear leader and the
    16-byte-block / ≤144-byte-skip (~10%) pattern; emulation-prevention bytes are
    stripped before encryption and re-inserted after; IV reset per NAL.
  - **AAC / AC-3 / E-AC-3 SAMPLE-AES**: `aac_encrypt_frame`/`aac_decrypt_frame`
    (ADTS header + 16-byte leader clear) and `ac3_encrypt_frame`/
    `ac3_decrypt_frame` (16-byte leader clear), 16-byte CBC blocks, `<16` trailer
    clear.
  - **`EXT-X-KEY` rendering**: `ExtXKey` (`METHOD`/`URI`/`IV`/`KEYFORMAT`/
    `KEYFORMATVERSIONS`) with `HlsEncryptionMethod` (`AES-128` / `SAMPLE-AES`),
    `ExtXKey::fairplay_sample_aes` (`skd://` + `com.apple.streamingkeydelivery`),
    `ExtXKey::aes128`, and `iv_from_sequence_number` (implicit IV = media
    sequence number as a 128-bit big-endian integer).
  - AES-128-CBC pinned by a NIST SP 800-38A F.2 known-answer test; the block
    cipher is the RustCrypto `aes` crate driven by the new `cbc` mode crate (the
    only added dependency).
- **Multi-DRM `pssh` init-data generation** (#480): new `drm` module building
  the system-specific `Data` payloads and convenience `pssh`-box builders on top
  of the existing `ProtectionSystemSpecificHeaderBox` (ISO/IEC 23001-7 §12.1).
  - DRM system-ID UUID consts: `WIDEVINE_SYSTEM_ID`, `PLAYREADY_SYSTEM_ID`,
    `FAIRPLAY_SYSTEM_ID`, `COMMON_SYSTEM_ID`.
  - **PlayReady**: `playready_wrmheader` (WRMHEADER v4.2.0.0 XML, UTF-16LE at
    emit), `playready_pro` (the PlayReady Object: `u32` LE length, `u16` LE
    record count, type-`0x0001` header record), and `playready_pssh`. The
    critical CENC-UUID ↔ PlayReady LE-GUID byte-swap is exposed as
    `cenc_kid_to_playready` / `playready_kid_to_cenc`.
  - **Widevine**: `widevine_pssh_data` (hand-encoded `WidevineCencHeader`
    protobuf — repeated `key_id`, `provider`, `protection_scheme`; minimal
    varint + length-delimited encoding, no protobuf crate) and `widevine_pssh`.
  - **FairPlay**: `fairplay_pssh_data` / `fairplay_pssh` (the `skd://` URI as
    UTF-8 `Data` — packager convention, not a formal spec).
  - `ProtectionSystemSpecificHeaderBox` gains `parse_box` (full-box parse) and
    `to_vec` (whole-box serialize, lengths rebuilt from fields).
  - No new dependency: base64/UTF-16LE/protobuf helpers are hand-rolled.
- **KLV timed metadata + KLV-over-RTP** (#478): a new `klv` module implements
  SMPTE ST 336 KLV framing (via MISB ST 0601 + RFC 6597) and the MISB ST 0601
  UAS Datalink Local Set.
  - **BER length** codec (`encode_ber_length` / `ber_length`): short form
    (`< 128`) and long form (`0x80 | N` + `N` big-endian bytes), round-trippable;
    indefinite form rejected.
  - **BER-OID tag** codec (`encode_ber_oid` / `ber_oid`) for Local Set item keys.
  - **`KlvItem`** — a 16-byte `UniversalLabel` key + value, with `Parse`/
    `Serialize` (the wire length is *computed*, never echoed).
  - **`UasLocalSet`** / **`LocalSetItem`** — the MISB ST 0601 packet (`UAS_LS_KEY`
    Universal Label wrapping BER-OID-tagged items): `precision_timestamp()`
    (tag 2, u64 BE µs since the POSIX epoch), `serialize_with_checksum()` +
    `verify_checksum()` (tag 1 = CRC-16/CCITT, poly `0x1021`, init `0xFFFF`, over
    the whole packet incl. the UL key), and `crc16_ccitt`.
  - **KLV-over-RTP** (`rtp::packetize_klv` / `rtp::depacketize_klv`, RFC 6597,
    `smpte336m`): a KLV unit placed directly after the fixed header (no payload
    header), fragmented across sequential packets sharing one timestamp with the
    marker bit on the final fragment; new `DEFAULT_KLV_PT` / `KLV_ENCODING_NAME`.

## [0.9.0] - 2026-07-03
### Added
- Release-audit fixes: `rtcp` length uses `saturating_sub` (latent underflow);
  `avc1` bare-parse error `need` matches its guard; `cli::Container`/`OutputFormat`
  gain `#[non_exhaustive]`; new `RtmpError::UnknownControlMsgType` (was a misleading
  `BadControlLength{need:0}`); added an `RtmpMux` full-chain IR round-trip test.
- **Trick-play manifest signalling — HLS + DASH** (#477): playlist and MPD
  APIs for signalling an I-frame-only (trick-play / scrubbing) rendition
  derived by `derive_iframe_track`.
  - **HLS `#EXT-X-I-FRAME-STREAM-INF`** (RFC 8216 §4.3.4.2): new
    `IFrameVariant` struct + `MasterPlaylist::iframe_variants: Vec<IFrameVariant>`;
    `to_m3u8` renders each as a single `#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=…,URI="…"`
    tag line (URI is an attribute, not a following line — unlike `EXT-X-STREAM-INF`).
    Zero iframe variants → no tag emitted (strict opt-in).
  - **HLS `#EXT-X-I-FRAMES-ONLY`** (RFC 8216 §4.3.3.6): new
    `MediaPlaylist::iframes_only: bool`; when `true`, emits the tag in the
    header block and bumps the rendered version to ≥ 4 as required by the spec.
    Defaults `false` — existing playlists are byte-for-byte unchanged.
  - **DASH trick-mode `AdaptationSet`** (ISO/IEC 23009-1 §5.8.5.8): new
    `TrickModeAdaptationSet` + `TrickModeRepr` structs, `DashPackager::trick_mode:
    Option<TrickModeAdaptationSet>`. When set, `package()` emits an additional
    `AdaptationSet` with `<SupplementalProperty
    schemeIdUri="urn:mpeg:dash:trickmode:2016" value="<main-id>"/>` and
    `maxPlayoutRate`/`codingDependency="false"`. The scheme URI is the named
    constant `TRICKMODE_SCHEME`. Defaults `None` — existing MPDs unchanged.
  - All changes are additive; no existing public API is modified.
- **HEVC (H.265) elementary streams TS → IR** (#467): `TsDemux` now carries
  `stream_type 0x24` HEVC video into the neutral `Media` IR. The in-band
  VPS/SPS/PPS NAL units are gathered from the Annex-B access units, the SPS is
  decoded (`decode_hevc_sps`) for coded geometry + profile/tier/level/chroma/
  bit-depth, and an `hvcC` `HEVCDecoderConfigurationRecord` is assembled into a
  `hvc1` `CodecConfig::Hevc` track — identical to the config `Fmp4Demux`
  recovers from an fMP4 `hvcC`, so `{HEVC-in-TS} → IR → {any}` composes.
  Per-sample `is_sync` marks IRAP access units (HEVC NAL types 16..=23). Both
  8-bit (Main) and 10-bit (Main 10) streams are supported. DTS-from-TS remains
  unimplemented (no `CodecConfig` DTS-from-ES variant). Additive change.
- **`AvcSpsInfo` VUI timing fields** (#523): `decode_avc_sps` now parses the H.264
  VUI `timing_info` block (ITU-T H.264 §E.1.1) and exposes three new optional
  fields on `AvcSpsInfo` — `num_units_in_tick: Option<u32>`, `time_scale:
  Option<u32>`, and `fps: Option<f32>` (= `time_scale / (2 × num_units_in_tick)`).
  All three are `None` when `vui_parameters_present_flag` or
  `timing_info_present_flag` is 0.  The VUI is walked in syntax order
  (aspect_ratio_info → overscan_info → video_signal_type → chroma_loc_info →
  timing_info) with no new dependencies.  Additive change; existing callers are
  unaffected.
- **`transmux` command-line packager + `cli` feature** (#482): a new opt-in
  `cli` feature (`clap` + `std`) builds a `transmux` binary that wires the
  existing demux and mux spokes into an any-to-any packager — `transmux <in>
  -o <out> -f <format>`. The input container is autodetected from its leading
  bytes (MPEG-TS `0x47`+188, MP4/CMAF `ftyp`/`styp`/`moov`/`moof`, MPEG-PS
  `00 00 01 BA`, WebM/EBML `1A 45 DF A3`, FLV `"FLV"`), demuxed to the neutral
  [`Media`] IR, then packaged into `cmaf` / `hls` / `ts-hls` / `dash` / `ts` /
  `progressive` (selected by `-f/--format` or inferred from the output
  extension). Flags: positional `<IN>` or `-i/--input`, `-o/--output`,
  `-f/--format`, `--segment-duration`, `--ll` (LL-DASH), `--tracks`, and (under
  the `cenc` feature) `--decrypt`/`--key`. Follows `docs/CLI-STANDARD.md` (clap
  derive, named flags, auto `--help`/`--version`). The library itself stays
  `no_std` and gains no dependencies; only the `cli` feature/binary pulls
  `clap`+`std`. New public module [`cli`] with a testable
  [`cli::run_bytes`] core and [`cli::detect_container`].
- **Low-Latency HLS — partial segments + preload hints** (#454, RFC 8216bis):
  a new [`ll_hls`] module with [`LlHlsSegmenter`], a segmenter that emits each
  segment's **partial segments** ("parts", RFC 8216bis §4.4.4.9) — independent
  CMAF `moof`+`mdat` fragments covering a configurable `part_target` sub-duration
  — before the parent segment closes. [`LlHlsSegmenter::with_part_target`]
  configures the part target (ms) alongside the segment target;
  [`LlHlsSegmenter::take_ready_parts`] drains ready [`PartInfo`]s (bytes,
  duration, `independent`, `segment_seq`, `part_index`) distinct from the full
  segments drained by [`LlHlsSegmenter::take_ready_segments`]. A part is flagged
  independent when it begins on a sync sample; a segment's parts concatenate to
  exactly the whole-segment [`build_media_segment`] media. The playlist model
  gains an opt-in [`hls::MediaPlaylist::low_latency`] config
  ([`hls::LowLatencyConfig`]) that renders the LL-HLS directives —
  `#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=<sec>`
  (§4.4.3.8, PART-HOLD-BACK held to ≥ 3× part-target),
  `#EXT-X-PART-INF:PART-TARGET=<sec>` (§4.4.3.7),
  `#EXT-X-PART:DURATION=<sec>,URI="…"[,INDEPENDENT=YES]` (§4.4.4.9,
  per [`hls::PartSpec`]), and `#EXT-X-PRELOAD-HINT:TYPE=PART,URI="…"` (§4.4.5.3).
  A plain playlist (no `low_latency`) renders none of these — LL-HLS is strictly
  opt-in.
- **IR timeline conditioning — PTS/DTS rebase & anchor** (#476): new `rebase`
  module of transforms over the `Media` IR, plus the absolute decode-time anchor
  they operate on. `rebase_to_zero` re-origins each track to decode time 0 (per
  track); `apply_offset(delta_ticks)` shifts every track's anchor by a signed
  delta (saturating at 0 on underflow); `unroll_33bit_wraps` undoes MPEG-2
  Systems 33-bit timestamp wraps (ISO/IEC 13818-1 §2.4.3.6; `MPEG_TS_WRAP` =
  `2^33`) so a timeline crossing the boundary is monotonic; and
  `insert_discontinuity_gap(track, at, gap_ticks)` extends the timeline by a gap
  for splice/gap conditioning. `Fmp4Demux` now populates the anchor from the
  first movie fragment's `tfdt` `baseMediaDecodeTime` (ISO/IEC 14496-12:2015
  §8.8.12) and `TsDemux` from the first sample's DTS (rescaled into each track's
  media timescale); FLV/WebM/MPEG-PS/RTMP/RTP carry no absolute anchor and leave
  it 0. Pairs with #475 (splice/concat) as the next consumer. All four transforms
  and the muxer wiring are re-exported from the crate root.
- **IR timeline splice / concatenation → SSAI** (#475): new `splice` module
  joining two `Media` timelines into one monotonic decode timeline for
  server-side ad insertion. `concat(a, b)` appends `b` after `a` on a shared
  timeline — matching tracks pairwise (by `track_id`, else by index; errors on
  incompatible track sets / codecs / timescales), rebasing each `b` track so its
  first sample's decode time meets `a`'s end decode time
  (`start_decode_time + Σ durations`), contiguous with no gap or overlap, sample
  bytes preserved verbatim. `splice_insert(base, ad, at_ticks)` plays `base` up
  to the splice, inserts `ad`, then resumes the base shifted forward by `ad`'s
  duration. A splice boundary must fall on a random-access point: the inserted
  content's first sample must be a sync sample, and `splice_insert` snaps
  `at_ticks` to the nearest **preceding** sync sample of the base video track via
  the testable `snap_to_preceding_sync` helper. Both return a `SpliceResult`
  (`media` + `discontinuity_points: Vec<SplicePoint>` — track id, sample index,
  and presentation time of each join) so a downstream HLS packager / `Segmenter`
  can emit `#EXT-X-DISCONTINUITY` (RFC 8216 §4.3.4.3) on exactly the join
  segments. Timeline model cites the ISO/IEC 14496-12 §8.8.12 `tfdt`
  `baseMediaDecodeTime`. SCTE-35-driven point *selection* (deciding where to
  splice from cue messages) is a follow-up. `concat`, `splice_insert`,
  `snap_to_preceding_sync`, `SplicePoint`, and `SpliceResult` are re-exported
  from the crate root.
- **`emsg` emission in media segments** (#455): [`build_media_segment_with_events`]
  emits one or more MPEG-DASH Event Message Boxes (`emsg`, ISO/IEC 14496-12 §8.8 /
  ISO/IEC 23009-1 §5.10.3.3) at the start of each media segment, after `styp` and
  before `moof` (DASH-IF IOP Part 10 §6.1 placement). Both version 0
  (`PresentationTime::Delta`, segment-relative) and version 1
  (`PresentationTime::Absolute`, representation-relative) are supported. The
  primary consumer is SCTE 35 in-band splice signalling (`urn:scte:scte35:2013:bin`,
  ANSI/SCTE 214-3). [`EmsgBox`], [`PresentationTime`], and [`EmsgVersion`] from the
  workspace `mp4-emsg` crate are re-exported from the `transmux` crate root so
  callers need no additional dependency. [`build_media_segment`] delegates to the
  new function with an empty event slice (byte-identical output).
- **fMP4/CMAF conformance validator** (#481): new `validate` module — the fMP4
  analogue of a TR 101 290 monitor. `validate_init_segment`,
  `validate_media_segment`, and `validate_cmaf_track` (cross-segment) walk the
  ISOBMFF box tree and return `Vec<ConformanceIssue>` (`Severity::Error` /
  `Warning`, each with a stable dotted `code` + spec-cited message) against
  ISO/IEC 14496-12 (box presence/order, `ftyp`/`moov`/`mvhd`/`trak` tree,
  `mvex`/`trex` fragmentation marker, `styp`/`moof`/`mfhd`/`traf`/`tfhd`/`tfdt`/
  `trun`, moof↔mdat pairing, `trun` sample-size/`data_offset` mdat-bounds,
  zero-duration samples) and ISO/IEC 23000-19 (CMAF — segment brands,
  single-track fragments, required `tfdt`, contiguous decode timeline, strictly
  increasing `mfhd.sequence_number`). Malformed input yields issues, never a
  panic.
- **HEVC SPS decode verified against real fixture** (#516): `decode_hevc_sps`
  proven correct on the committed `hevc_frag.mp4` hvcC record — asserts exact
  ffprobe oracle values (320×240, Main profile idc=1, 4:2:0, 8-bit, level 60).
  Truncated-input negative tests added. `decode_hevc_sps` doc now cites
  ITU-T H.265 §7.3.2.2.1 (syntax) + §7.4.3.2.1 (conformance-window semantics).
- **HLS discontinuity support** (#453): `MediaSegment::discontinuous` flag and
  `MediaPlaylist::discontinuity_sequence` field; `MediaPlaylist::to_m3u8()` emits
  `#EXT-X-DISCONTINUITY` immediately before every flagged segment (RFC 8216 §4.3.4.3)
  and `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` in the playlist header when `n > 0`
  (RFC 8216 §4.3.3.3). `Segmenter::mark_discontinuity()` marks the next segment cut
  as discontinuous (explicit API); `Segmenter::take_ready_with_meta()` returns
  `(Vec<u8>, SegmentMeta)` pairs carrying the discontinuity flag. Auto-detection of
  init-segment changes is available via the new
  `mark_init_discontinuities(entries: &mut [(&[u8], &mut MediaSegment)])` helper,
  which compares consecutive init bytes and sets the flag where they differ.
- **RTMP transport spoke** (#515): `RtmpDemux` (`Unpackage`) / `RtmpMux` (`Package`) ⇄ IR,
  Adobe RTMP 1.0. De/frames the chunk stream (basic + message headers, all four `fmt`
  types incl. 2-/3-byte csid and extended timestamp — §5.3.1), reassembles multi-chunk
  messages honouring Set Chunk Size, and routes Audio (type 8) / Video (type 9) message
  bodies — which ARE FLV tag bodies — through the FLV spoke (`FlvDemux`/`FlvMux`) to the
  IR. Also typed `Handshake0/1/2` (§5.2), `ProtocolControl` (SetChunkSize/Abort/Ack/
  WindowAckSize/SetPeerBandwidth — §5.4), and AMF0 `AmfValue`/`Command` for
  `connect`/`publish`/`play`/`createStream`/`onMetaData` (§7). AMF0 only (AMF3 noted as
  out of scope). `no_std` + `alloc`.
- **I-frame-only trick-play track derivation** (#477): `trickplay::derive_iframe_track(&Track) -> Result<Track>` — retains only sync samples from a video track and folds each kept sample's duration to span the gap to the next keyframe, conserving the total timeline. `append_iframe_track(&mut Media, usize)` is a convenience that appends the derived track to an existing `Media`. Returns `Error::InvalidInput` when the source has no sync samples. Codec/container-agnostic; works with any `CodecConfig`. Downstream signalling (`EXT-X-I-FRAME-STREAM-INF` / DASH trick-mode) is deferred to a follow-up issue.
- **RTCP control packets** (#514): typed `Parse`/`Serialize` for the RFC 3550 §6 set —
  `SenderReport` (PT 200), `ReceiverReport` (201), `SourceDescription` (202, with
  `SdesChunk`/`SdesItem`/`SdesItemType`), `Bye` (203), `App` (204), the shared
  `ReportBlock` (24-bit sign-extended cumulative-lost), `CommonHeader`, and a
  `CompoundPacket` that enforces the §6.1 "first packet must be SR/RR" rule on
  construction, parse, and serialize. Dispatch via `RtcpPacket` / `RtcpPacketType`
  (`name()` + `impl_spec_display!`). RTP companion to `rtp.rs`; not a hub spoke.
- **Public NAL keyframe helper** (#517): `nal_unit_type` / `is_keyframe_nal` /
  `access_unit_is_keyframe` + `NalCodec` (Avc/Hevc/Vvc) — Annex-B and 4-byte
  length-prefixed, spec-cited to H.264/H.265/H.266 §7.3.1. `ts_demux` IDR detection
  now delegates to it (behaviour byte-identical).
- **FLV container spoke** (#513): `FlvDemux` (`Unpackage`) / `FlvMux` (`Package`) ⇄ IR,
  Adobe FLV v10.1 Annex E. H.264 (AVCVIDEOPACKET, avcC seq-header, CompositionTime →
  composition offset) + AAC (AACAUDIODATA, ASC seq-header); reuses `CodecConfig::Avc`/`Aac`.
  ms timescale, lossless timing round-trip. Skips spurious empty sequence-header tags;
  trusts the ASC over contradictory `onMetaData`.

### Fixed
- **HEVC sample-entry visual dimensions** (#467): `HEVCSampleEntry::bare_parse`
  read `width`/`height` (and the following visual fields) from the wrong byte
  offsets, so `Fmp4Demux` recovered `0×0` dimensions for `hvc1`/`hev1` tracks.
  Corrected to the ISO/IEC 14496-12 §12.1.3 `VisualSampleEntry` layout (width at
  `body[24]`), matching the AVC entry and `VisualSampleEntryFields::parse_body`.

### Changed
- **`Track` gains a `start_decode_time: u64` field** (#476): the absolute decode
  time of the track's first sample, in the track's media timescale — the
  fragment `tfdt` `baseMediaDecodeTime` (ISO/IEC 14496-12:2015 §8.8.12) anchor
  that `Sample` relative timing lacked. `Track::new` still defaults it to 0 (all
  existing callers compile); `Track::new_at(spec, samples, start)` and
  `Track::with_start_decode_time(start)` set it. This is an additive struct-field
  change → a **minor** version bump.
- **`CmafMux` now writes `Track::start_decode_time` as the first segment's
  `base_media_decode_time`** (#476), replacing the previously hardcoded `0`. A
  rebase/offset transform is therefore observable in the muxed `tfdt`.

## [0.8.0] — 2026-07-02
### Added
- **Any-to-any hub** (#466): the container-agnostic IR (`Media` / `Track`, thin wrappers
  over `TrackSpec`/`Sample`) + implementations of the new **broadcast-common 8.2.0**
  traits — `CmafMux` / `HlsPackager` (`Package`) and `Fmp4Demux` (`Unpackage`, fMP4 →
  `Media`). Every demux/mux spoke now targets one hub API; `Unpackage`⇄`Package` and
  `Encrypt`⇄`Decrypt` are inverse pairs mirroring `Parse`/`Serialize`. Additive —
  `build_init_segment`/`build_media_segment`/`Segmenter`/`TrackSpec`/`Sample` unchanged.
- Requires broadcast-common ≥ 8.2.0 (the trait definitions).
- **MPEG-H 3D Audio** input (promotion): `Fmp4Demux` now reconstructs
  `CodecConfig::MpegH` from `mha1`/`mha2`/`mhm1`/`mhm2` sample entries (re-parsing
  the `mhaC` record, ISO/IEC 23008-3 §20) — MPEG-H was previously output-only.
  Verified byte-exact against a real Fraunhofer/DASH-IF MPEG-H bitstream. This
  makes the codec set demux+mux complete across the hub.
- **VVC / H.266** (#474): `CodecConfig::Vvc` + `vvc1`/`vvcC` (VvcDecoderConfiguration-
  Record as a FullBox, byte-exact Parse/Serialize) mirroring HEVC. `decode_vvc_sps`
  (H.266 §7.3.2.4/§7.3.3.1) recovers dims/profile/tier/level; `Fmp4Demux` reconstructs
  `Vvc` from `vvc1`/`vvi1`; `CmafMux` emits `vvc1`. Byte-verified against a real
  vvenc+ffmpeg fixture. vvcC layout doc grounded in the FFmpeg reference (§11).
- **VP8 + Vorbis** (WebM): `CodecConfig::Vp8` (dims from the RFC 6386 key-frame
  header) + `CodecConfig::Vorbis` (channels/sample_rate + verbatim `CodecPrivate`
  from the Vorbis I identification header). `WebmDemux` now covers all four WebM
  codecs (VP9/VP8 video, Opus/Vorbis audio). WebM-native (no mp4 mux path).
- **MPEG-2 video (H.262) + MPEG-1/2 audio (MP1/2/3)** codecs: `CodecConfig::Mpeg2Video`
  + `MpegAudio`. `Fmp4Demux` reconstructs `mp4v`/esds (OTI 0x60–0x65) → `Mpeg2Video`
  (dims from the in-band sequence header, ISO 13818-2 §6.2.2.1) and `mp4a` OTI
  0x69/0x6B → `MpegAudio` (layer/rate/channels from the frame header, ISO 11172-3
  §2.4.1.3). `TsDemux` handles `stream_type` 0x02/0x03/0x04 — the classic broadcast
  pair now round-trips through both fMP4 and TS. New `Mp4vSampleEntry` +
  `MpegAudioLayer` enum.
- **`CodecConfig::Hevc`** + **complete `Fmp4Demux` codec-config reconstruction**
  (#467 codec tail): `Fmp4Demux` now reconstructs the IR codec config for every
  codec the crate can output — `hvc1`/`hev1`→`Hevc`, `av01`→`Av1`, `vp09`→`Vp9`,
  `Opus`→`Opus`, `fLaC`→`Flac`, `dac3`→`Ac3`, `dec3`→`Eac3`, `ddts`→`Dts` (plus
  the existing `avc1`/`mp4a`) — was previously deferred to AVC/AAC only. New
  `Hevc` variant muxes to an `hvc1`+`hvcC` sample entry. Every codec round-trips
  byte-identically (config box + coded samples) via fragmented-mp4 fixtures.
  Unknown sample entries skip the track rather than erroring.
- **RTP spoke** (#469): `RtpPacketizer` (`Package`) and `RtpDepacketizer`
  (`Unpackage`) — de/packetize the `Media` IR ⇄ RTP. H.264 single-NAL / STAP-A
  (SPS+PPS) / **FU-A** fragmentation at MTU (RFC 6184), AAC `AAC-hbr` AU-headers
  (RFC 3640), RTP fixed header with marker/seq/90 kHz timestamps (RFC 3550), and
  SDP generation (`rtpmap`/`fmtp` with `sprop-parameter-sets` + AAC `config`,
  RFC 4566). Round-trips byte-identically through the real demuxed NALs; no new
  dependency (hand-rolled base64/hex). New `RtpMediaKind` enum.
- **Microsoft Smooth Streaming output** (#473): `SmoothPackager` implements the hub
  `broadcast_common::Package` trait — `Media` IR → a Smooth client Manifest
  (`SmoothStreamingMedia`>`StreamIndex`>`QualityLevel`+`c`) + Smooth fragment-MP4
  fragments (`moof` with the `tfxd` `uuid` box + `mdat`). FourCC `H264`/`AACL`,
  `CodecPrivateData` = start-code SPS+PPS / raw ASC; TimeScale 10 MHz. New
  `TfxdBox` uuid type + `SmoothStreamType` enum. Fragments round-trip losslessly
  via `Fmp4Demux`. Cites [MS-SSTR] + ISO/IEC 14496-12.
- **Low-latency DASH** (#461): `LlSegmenter` (chunked CMAF — each segment
  subdivided into `moof`+`mdat` chunks, first chunk `styp`-prefixed, contiguous
  `tfdt`/sequence numbers) + `LlDashPackager` (LL-DASH MPD: `type="dynamic"`,
  `SegmentTemplate@availabilityTimeComplete="false"` + `@availabilityTimeOffset`,
  `<ServiceDescription><Latency>`). Both `impl broadcast_common::Package`. Chunks
  concatenate losslessly to a whole segment (verified via `Fmp4Demux`). ISO/IEC
  14496-12 chunk structure + ISO/IEC 23009-1 / DASH-IF LL IOP signalling.
- **WebM / Matroska demuxer** (#471): `WebmDemux` implements the hub
  `broadcast_common::Unpackage` trait — WebM (EBML) → `Media` IR, the fourth hub
  input (TS / fMP4 / MPEG-PS / WebM). Hand-written EBML/VINT tree walker
  (RFC 8794 framing, RFC 9559 element IDs); maps `V_VP9`→`CodecConfig::Vp9`
  (synthesised `vpcC`) and `A_OPUS`→`CodecConfig::Opus` (`dOps` from the CodecPrivate
  `OpusHead`); (Simple)Block timestamps in a millisecond IR timescale, Opus
  pre-skip codec delay applied. Gated against a per-frame **size-column** ffprobe
  oracle + a CMAF output round-trip (vp09/`vpcC` + Opus/`dOps`).
- **CENC decrypt** (#465): `CencDecryptor` implements the hub
  `broadcast_common::Decrypt` trait — unprotect a CENC (`cenc` / AES-128-CTR) fMP4
  given the content key. Reuses the existing `cenc.rs` box parsers
  (`tenc`/`senc`/`saiz`/`saio`/`sinf`/`frma`); subsample-aware (clear ranges
  skipped, CTR streams across protected ranges), IV left-justified to 16
  (ISO/IEC 23001-7 §10.1). AES via RustCrypto `aes`/`ctr` behind an optional
  `cenc` feature (`--no-default-features` drops it). `cbcs` documented
  unsupported. Verified by decrypting a real ffmpeg-encrypted fixture to
  byte-identical cleartext (+ wrong-key negative). New `CencScheme` enum.
- **MPEG-2 Program Stream demuxer** (#470): `PsDemux` implements the hub
  `broadcast_common::Unpackage` trait — MPEG-2 PS (`.ps`/VOB-style) → `Media` IR,
  the third hub input alongside TS and fMP4. Parses packs/system-header via
  `mpeg-ps`, maps elementary streams by `stream_id` (H.264 0xE0–0xEF; AC-3 in
  `private_stream_1` 0xBD), reassembles PES across packs, recovers H.264 `avcC`
  (in-band SPS/PPS) + AC-3 `dac3` (syncframe BSI). Gated against ffprobe timing +
  byte-identical `avcC`/video-NAL oracles (ISO/IEC 13818-1 §2.5).
- **Classic HLS (MPEG-2 TS segments)** (#472): `TsHlsPackager` implements the hub
  `broadcast_common::Package` trait (`Output = TsHlsOutput { segments, playlist }`),
  segmenting the `TsMux` output at keyframe boundaries into independently-decodable
  `.ts` segments (each re-emits PAT+PMT + a keyframe-aligned PES) plus an RFC 8216
  HLS media playlist (`#EXTINF` + `.ts` URIs, no `#EXT-X-MAP`). Per-segment base DTS
  keeps one monotonic timeline across boundaries — the concatenated segments
  round-trip losslessly through `TsDemux`.
- **DASH `.mpd` output** (#464): `DashPackager` implements the hub
  `broadcast_common::Package` trait (`Output = String`), emitting a DASH MPD
  (ISO/IEC 23009-1) alongside the HLS playlists from one CMAF —
  MPD→Period→AdaptationSet(`video/mp4`,`audio/mp4`)→Representation with
  `SegmentTemplate` (`$Number$`/`$RepresentationID$`). `codecs=` from the crate's
  own rfc6381 builders; `@width`/`@height` from the SPS, `@audioSamplingRate` from
  the ASC, integer `@bandwidth`; VOD (`static`) + `dynamic` (live). Dependency-free
  XML writer; integer-only arithmetic (`no_std`-clean).
- **fMP4/CMAF repackage** (#462): `Repackage` + `Media` IR transforms —
  `select_tracks` (track subset), `trim` (half-open presentation window, snapped
  back to the preceding sync sample per CMAF ISO/IEC 23000-19 §7.3.2.3), and
  resegment (via the existing `Segmenter`) to a new target duration. Composes
  demux → transform → mux with no new box parsers; lossless (byte-identical coded
  samples across identity repackage, verified against the `TsDemux` oracle).
- **TS muxer** (#460): `TsMux` implements the hub `broadcast_common::Package`
  trait — `Media` IR → a whole-188-byte-packet MPEG-2 TS, the byte-level inverse
  of `TsDemux`. Emits PAT→PMT (CRC-32/MPEG-2 sections), `stream_type` per codec,
  PCR on the first video PID; per-sample PES (PTS always, DTS when differing),
  video length-prefixed→Annex B with SPS/PPS re-injection on keyframes, AAC
  re-wrapped in ADTS from the `esds` ASC (ISO/IEC 13818-1 §2.4.3/§2.4.4). With
  `TsDemux` this closes the loop: `{fMP4/CMAF} → IR → {TS}` and byte-fidelity
  `TS → IR → TS` round-trips.
- **Progressive MP4 output** (#463): `ProgressiveMux` implements the hub
  `broadcast_common::Package` trait, muxing the `Media` IR into a single-file,
  non-fragmented `.mp4` (ftyp + one moov with full `stbl` sample tables + one
  mdat) — the VOD/download counterpart to `CmafMux`. Builds `stts`/`ctts`/
  `stsc`/`stsz`/`stco`|`co64`/`stss` from the sample stream (ISO/IEC 14496-12
  §8.5–§8.7); `faststart: bool` writes moov-before-mdat via a two-pass
  chunk-offset fixup. Adds typed `co64`/`stss` boxes. Gated against the ffmpeg
  faststart ref mp4 (byte-identical video samples + `avcC`).
- **TS demuxer** (#467, partial — H.264 + AAC): `TsDemux` implements the hub
  `broadcast_common::Unpackage` trait, turning MPEG-2 TS bytes into the `Media`
  IR — the input side of the any-to-any hub, so `{TS} → IR → {any}` works
  in-crate. Follows PAT→PMT, maps `stream_type`→codec, per-PID PES reassembly
  (PTS/DTS 33-bit unwrap), and recovers codec config from in-band parameters
  (H.264 SPS/PPS → `avcC`; AAC ADTS → ASC/`esds`; AC-3/E-AC-3 syncframe →
  `dac3`/`dec3`). Verified against ffprobe timestamp and ffmpeg `-c copy` byte
  oracles. HEVC/DTS are recognised in the PMT but not yet emitted (no IR
  HEVC-video variant / DTS-ES parser) — tracked on #467.
- `mpeg-ts` / `mpeg-pes` are now regular dependencies (were dev-deps).

### Fixed
- **ADTS `channel_configuration` decode** (`aac_asc`): the 3-bit field was split
  wrong (`byte2[0]<<3 | byte3[7:5]`); the correct ISO/IEC 13818-7 §6.2 layout is
  `byte2[0]<<2 | byte3[7:6]`. Build+parse were self-consistent so round-trip
  tests passed, but a real mono ADTS stream was misread as stereo. Both
  directions corrected.

### Fixed
- `TsDemux` now decodes AVC `width`/`height` from the in-band SPS (was left at 0).

## [0.6.0] — 2026-07-02
### Added
- **DTS** fMP4 carriage (#437, ETSI TS 102 114 §E.2): `dtsc`/`dtsh`/`dtsl`/`dtse`
  sample entries + `ddts` (DTSSpecificBox — DTSSamplingFrequency, max/avg bitrate,
  pcmSampleDepth, FrameDuration, StreamConstruction, channel layout, …) + a
  `CodecConfig::Dts` variant + `rfc6381()`. Typed Parse/Serialize with a spec-vector
  byte-exact round-trip + `build_init_segment` moov round-trip (ffmpeg has no `ddts`
  encoder, so the real-fixture gate is deferred).
### Changed
- `hvcC` value-verified against the ISO 14496-15:2017 §8.3.3.1 text (recovered via
  marker OCR of the scanned edition), matching FFmpeg movenc + the byte-exact oracle
  (#394). Docs only.

## [0.5.0] — 2026-07-01
### Added — fMP4 gap tier (real codecs + container completeness)
- **Codec sample entries + config boxes** (container-level; header parse only, samples
  pass through opaque): AV1 (`av01`/`av1C`, #436), VP9 (`vp09`/`vpcC`), Opus (`Opus`/
  `dOps`), FLAC (`fLaC`/`dfLa`) (#437), AC-4 (`ac-4`/`dac4`, #431), MPEG-H 3D Audio
  (`mha1`/`mhm1`/`mhaC`, #433), and HE-AAC SBR/PS AudioSpecificConfig signaling →
  `mp4a.40.5`/`mp4a.40.29` (#432). Each with a `CodecConfig` variant + `rfc6381()`.
- **CENC per-sample encryption** (#429, ISO/IEC 23001-7): `tenc`/`senc`/`saiz`/`saio`/
  `pssh` + `sinf`/`frma`/`schm`/`schi` + `enca`/`encv` sample entries.
- **Subtitle carriage** (#430, ISO/IEC 14496-30): `stpp` (TTML/IMSC) + `wvtt` (WebVTT +
  `vttC`/`vtte`/`vttc`/`payl`/`sttg`/`iden`).
- **Sample-entry extensions** (#434): `colr` (nclx — HDR/wide-gamut), `pasp`, `clap`.
- **Timing / grouping** (#435): `prft` (ProducerReferenceTimeBox), `sgpd`/`sbgp`
  (sample groups incl. `roll`), `subs` (sub-sample info).
- **avcC/hvcC value-verification** (#441/#394): byte-exact round-trip against real
  ffmpeg-muxer boxes; avcC now grounded on the text-layer 14496-15:2012.

All new boxes are typed with symmetric `Parse`/`Serialize` and byte-exact round-trip
tests against real ffmpeg-authored fixtures (config-box oracles = ffmpeg's own muxer
output); MPEG-H uses a spec vector (no redistributable fixture/encoder).

## [0.4.1] — 2026-07-01
### Changed
- Value-verified the `esds` / `mp4a` descriptor layout against the vendored
  ISO/IEC 14496-1 §7.2.6 (transcribed to `docs/codec/es-descriptor-14496-1.md`)
  and added a **byte-exact round-trip test on a real ffmpeg-authored `esds`**
  (AAC-LC, 4-byte-expanded descriptor sizes, real max/avg bitrates). No API change.

## [0.4.0] — 2026-07-01
### Added
- AC-3 / E-AC-3 audio in the fMP4 path (ETSI TS 102 366 Annex F):
  - `Ac3SpecificBox` (`dac3`) + `Ec3SpecificBox` (`dec3`) — typed config boxes with
    Parse + symmetric Serialize + round-trip.
  - AC-3 / E-AC-3 syncframe BSI parsers (`0x0B77` syncword → `syncinfo()`+`bsi()` /
    E-AC-3 syncframe): build a `dac3`/`dec3` from an elementary stream.
  - `CodecConfig::Ac3` / `Eac3` + `Ac3SampleEntry` / `Ec3SampleEntry`
    (`SampleEntryVariant::Ac3`/`Ec3`), wired through `build_init_segment` to emit
    `ac-3` / `ec-3` sample entries.
  - `rfc6381()` → `"ac-3"` / `"ec-3"`.
- Gate `tests/dolby.rs`: parses real ffmpeg-encoded AC-3/E-AC-3 fixtures and asserts
  the built `dac3`/`dec3` bytes match ffmpeg's own MP4-muxer output byte-for-byte.

## [0.3.0] — 2026-07-01
### Added
- SPS/VPS decode + RFC 6381 codec strings, so transmux no longer needs an external
  parser (e.g. `h264_reader`) to learn what it must put in an `avcC`/`hvcC`:
  - `AvcSps::decode() -> AvcSpsInfo` (ITU-T H.264 §7.3.2.1.1): profile_idc,
    constraint byte, level_idc, chroma_format_idc, bit-depths, `frame_mbs_only`,
    and coded width·height after frame cropping (chroma-dependent CropUnit +
    interlaced height factor). Handles the high-profile branch and skips
    SPS-embedded scaling lists.
  - `HevcNalUnit::decode_sps() -> Option<HevcSpsInfo>` (ITU-T H.265 §7.3.2.2 +
    profile_tier_level §7.3.3): general profile/tier/level, compatibility +
    constraint flags, chroma/bit-depth, conformance-window-cropped dimensions.
  - RFC 6381 strings: `AvcSps::rfc6381()` → `avc1.PPCCLL`;
    `HevcNalUnit::rfc6381()` → `hvc1.…`; `AudioSpecificConfig::rfc6381()` →
    `mp4a.40.<AOT>`. Plus a public `bitreader` (Exp-Golomb + emulation-prevention
    unescape) and `sps` module.
- Gate `tests/codec_config.rs`: decodes real ffmpeg-encoded fixtures across the
  full H.264 profile matrix (baseline/main/high/high10/high422/high444 + interlaced
  + 1080-cropped) and HEVC main/main10, asserting every field against a
  `trace_headers` oracle, plus a scaling-list spec vector and an avcC round-trip.

## [0.2.0] — 2026-07-01
### Added
- `Segmenter`: a stateful streaming CMAF segmenter wrapping `build_init_segment` /
  `build_media_segment`. Feed coded samples in decode order (`push`), pull finished
  media segments (`take_ready`), and `flush` the trailing partial segment at
  end-of-stream. Segments are cut on the anchor track (first video track, else the
  first track) at a keyframe once the target duration is reached, so every video
  segment begins on a random-access point and the concatenation of all segments
  carries the full stream with contiguous per-track `tfdt`. This is the streaming
  state machine `build_media_segment` (a batch box builder) deliberately omits;
  it lets a live remuxer produce a CMAF track without hand-rolling segment cutting.
- `Error::InvalidInput(&'static str)` for caller-precondition violations (empty
  track list, non-positive segment duration, duplicate `track_id`, unknown
  `track_id` on `push`).

## [0.1.0] — 2026-07-01
### Added
- End-to-end `tests/ts_to_cmaf.rs`: demux a real H.264+AAC MPEG-TS
  (`fixtures/ts/h264_aac.ts`), synthesize `avcC` from the stream's SPS/PPS and
  `esds` from the AAC ADTS header, build init + media segments, then re-parse —
  asserting **byte-identical** avcC SPS/PPS + esds AudioSpecificConfig fidelity,
  75 video access units, the computed ADTS frame count, and first-sample
  round-trip. Closes the literal "TS in → CMAF out" acceptance (#408).
- HLS playlist generation (RFC 8216): `MediaPlaylist` / `MasterPlaylist` +
  `Variant` / `MediaSegment` with `to_m3u8()` emitters for the CMAF segments
  produced by the remux pipeline (`#EXTM3U` / `EXT-X-VERSION` / `TARGETDURATION` /
  `MEDIA-SEQUENCE` / `EXTINF` / `ENDLIST`; master `EXT-X-STREAM-INF` with
  bandwidth/codecs/resolution; `extra_tags` for `EXT-X-DATERANGE`). Generated
  playlists validate clean through `media-doctor::check_playlist`.
- Segment-level boxes: `FileTypeBox` (`ftyp`), `SegmentTypeBox` (`styp`),
  `MediaDataBox` (`mdat`, 32-bit + 64-bit largesize).
- Typed `MovieExtendsBox` (`mvex`) + `TrackExtendsBox` (`trex`) on `MovieBox`
  (fragmented-init movies); byte-identical round-trip of a real fragmented moov.
- Annex B ↔ length-prefixed NAL conversion (`annexb_to_length_prefixed`,
  `length_prefixed_to_annexb`, iterators).
- Samples-in TS→CMAF remux pipeline: `build_init_segment` (ftyp + fragmented-init
  moov with empty sample tables + mvex/trex) and `build_media_segment`
  (styp + moof{tfhd,tfdt,trun} + mdat) with `data_offset` computed from the
  finished `moof` and signed composition offsets. `CodecConfig` / `TrackSpec` /
  `Sample` / `FragmentTrackData` API.
- Typed init-segment `moov` box tree (`MovieBox`/`mvhd`/`trak`/`tkhd`/`mdia`/
  `mdhd`/`hdlr`/`minf`/`stbl`/`stsd` + sample descriptions) with byte-identical
  round-trip.
- `avcC`/`hvcC` config boxes, `esds`/ES_Descriptor, AAC AudioSpecificConfig +
  ADTS, movie-fragment boxes (`moof`/`mfhd`/`traf`/`tfhd`/`tfdt`/`trun`),
  timing boxes (`stts`/`ctts`/`stsc`/`stsz`/`stco`/`elst`/`sidx`), and generic
  box framing.

_Unreleased — `transmux` has not yet been published to crates.io._
