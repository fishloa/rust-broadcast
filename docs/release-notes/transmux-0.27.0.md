---
status: DRAFT
---
# transmux 0.27.0

_Released 2026-10-16._

**Breaking release (typed ids, API changes, behaviour changes and the epoch cascade).** This is the largest release of the wave. The container hub gets typed ids (`TrackId`, `ProgramNumber`, `Pid`), subtitle tracks (`stpp`/`wvtt`) that survive a remux and appear in DASH and Smooth manifests, real `wvtt` producer fidelity (MP4Box and gpac), E-AC-3 7.1 (dependent substream) support verified against a real stream, a reworked `StreamingTsDemux` E-AC-3 / head-of-line path, a complete MPEG-H 3D audio descriptor, DASH `SegmentTimeline::resolve`, ISOBMFF `largesize` boxes, hostile-input bounds (#1130: a 10 GB allocation from four overwritten bytes; a 1.28 GiB allocation from 256 bytes of zero-width `trun`s; a panic on short `largesize` sample entries), AVS2/AVS3 and Audio Vivid boxes, and demuxers that skip one damaged unit instead of aborting (`Media::degraded`). It also rebuilds against eleven moved siblings (`broadcast-common` 10, `dvb-si` 12, `mpeg-ts`/`mpeg-pes`/`mpeg-ps` 0.6 and others), so a consumer's whole dependency graph must move together.

## Why this version

- **Proposed version:** 0.26.0 -> **0.27.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (TrackId/ProgramNumber newtypes, Pid, CodecConfig::Subtitle sample_entry, VttCueBox, `Media` non-exhaustive; behaviour changes in `StreamingTsDemux`, demux damage handling, MPEG-H descriptor).
- **cargo-semver-checks vs live 0.26.0 (all features):** MAJOR (3 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, broadcast-hls, container-probe, dvb-si, mp4-emsg, mpeg-pes, mpeg-ps, mpeg-ts, rtcp-packet, rtp-packet, scte35-splice.
- **Caret epoch:** 0.26 -> 0.27; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-container-probe = "0.1"
+container-probe = "0.2"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mp4-emsg = "0.4"
+mp4-emsg = "0.5"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
-mpeg-ps = "0.5"
+mpeg-ps = "0.6"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-rtcp-packet = "0.5"
+rtcp-packet = "0.6"
-rtp-packet = "0.4"
+rtp-packet = "0.5"
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

New dependencies (no version bump of an existing requirement):

```toml
+memchr  = { version = "2", default-features = false }                                     # Annex B start-code scan; crate stays no_std
+zeroize = { version = "1", default-features = false, features = ["alloc"], optional = true } # `cenc` feature: wipe content keys on drop
+subtle  = { version = "2", default-features = false, optional = true }                    # `cenc` feature: constant-time key comparison
```

`container-probe` is only used by the `cli` feature. `criterion` (a benchmark) and `test-fixtures` are dev-only; neither affects consumers.

## Breaking changes and migration

### 1. Typed ids (#1159)

New `TrackId(u32)` and `ProgramNumber(u16)` (re-exported at the crate root and `ir`; `new`/`value`, no silent `From`; `Debug`/`Display` render the bare number, serde is transparent, so every golden/JSON/manifest is byte-identical). Every public `track_id: u32` is a `TrackId`: `TrackSpec::{track_id,new}`, `Track::track_id()`, `FragmentTrackData`, the `DemuxEvent` track fields, the segmenters' `push(track_id, ..)`, `RtpInputStream`/`RtpTrackInit`, the ISOBMFF box structs (`tkhd`/`tfhd`/`trex`/`mvhd.next_track_id`/`prft.reference_track_id`), `Error::UnmuxableDataTrack`, the Smooth/DASH descriptors, and `TrackSpec::program_number` (`Option<ProgramNumber>`, `with_program(ProgramNumber)`).

```rust
// before
let spec = TrackSpec::new(1, 90_000, config);   seg.push(1, sample);   let id: u32 = track.track_id();
// after
let spec = TrackSpec::new(TrackId::new(1), 90_000, config);   seg.push(TrackId::new(1), sample);   let id: u32 = track.track_id().value();
```

`TrackId::next()` hands out consecutive ids. PIDs: `TrackSpec::source_pid` is `Option<Pid>` and `with_source` takes a `Pid`; `PcrSample::{pid,new}` and `EventProvenance::pid` are `Pid`; `Debug` of the IR renders `Pid(0x0100)` instead of `256`. `TsMux` reports `Error::InvalidInput` if a track would be assigned an elementary PID above 0x1FFF.

### 2. `CodecConfig::Subtitle` carries its sample entry (#1186)

```rust
// before
match codec { CodecConfig::Subtitle { format } => .. }
// after
match codec { CodecConfig::Subtitle { format, .. } => .. }      // gains sample_entry: Option<SubtitleSampleEntry>
```

`Fmp4Demux` fills the `stpp`/`wvtt` entry whole; `build_init_segment` and every packager built on it re-emit the track (entry verbatim, `sthd`/`nmhd` media header, handler `subt`/`text`). `CodecConfig::is_muxable_in_bmff()` is true for a subtitle track with an entry; one without (TS DVB/Teletext) is still `Error::UnmuxableSubtitleTrack`.

### 3. `VttCueBox` (#1249)

Gains `extra_children` (every `vttc` child other than `payl`/`sttg`/`iden`, kept verbatim) and remembers wire order, so a parsed cue serializes to the bytes it was parsed from (MP4Box and gpac write `iden, sttg, payl`). The struct literal can no longer be built outside the crate: use `VttCueBox::new` and set the public fields. New typed `VttAdditionalTextBox` (`vtta`), `WvttSampleBox`, `parse_wvtt_sample`.

### 4. Behaviour: damaged input no longer fails the whole demux (#1144)

`PsDemux`, `Fmp4Demux`, `WebmDemux`, `FlvDemux` and the batch `RtpDepacketiser` skip one damaged unit and report it in the new `Media::degraded` (`InputDegradation::MalformedUnit { position }`; both `#[non_exhaustive]`). Inputs that used to fail now succeed with `degraded` populated: callers that relied on the error as a validity signal must inspect `Media::degraded`.

### 5. Behaviour: `StreamingTsDemux` track announcement and E-AC-3 (changes since 0.26.0)

`StreamingTsDemux` is the one engine behind `TsDemux`, `ts_hls`, `media-plane`, `multimux`, `hls-runtime` and `compliance-probe`. No signature changes, but what it emits and when does:

- **E-AC-3 probe.** A PID's `dec3` is no longer built from the first frame found. The probe prefers an access unit whose frames all pass `crc2` (a damaged or false-sync frame is skipped; `Ec3SyncframeInfo::from_es_first_au_crc_verified`), takes `bsmod` from the first intact frame that carries one (`first_crc_verified_bsmod`), and falls back to the first header-valid frame only when no intact one is held, so lossy captures stay live. When the frame chain breaks after a whole frame (bytes that are not a syncword), that frame is accepted (`Ec3SyncframeInfo::from_es_first_au_or_broken_chain`, an unverified broken-chain fallback; running out of data or a truncated next frame is still waited on). A probe holding 16 chained frames (`EAC3_PROBE_MAX_FRAMES`) without a provably complete first access unit resolves from them. Work per payload is bounded (after 4096 re-walks, `EAC3_PROBE_FULL_WALKS`, tiny payloads are appended without re-walking). On a real French TNT capture (video, three E-AC-3 PIDs signalled as `stream_type` 0x06 + descriptor 0x7A, and a subtitle/data PID) every track is emitted, as in 0.26.0; a live feed, which never calls `finish()`, does not stall behind one E-AC-3 PID that never completes.
- **Dependent substreams and hold-back.** When a programme has dependent substreams (7.1 / Atmos) the final access unit of each PES is held back until the next PES shows whether a dependent frame follows. The hold-back is flushed before `TrackRemoved` (PMT version change), on a continuity gap, and by `finish()`; a held access unit keeps its own timeline across a PTS jump. A dependent frame *observed* on a track promoted from an unverified or broken-chain first frame turns the hold-back on. An orphan dependent frame (no independent frame before it) is dropped rather than emitted as a sample with an independent frame's duration. An AC-3 syncframe (`bsid` <= 8) is recognised as independent substream 0, so real Dolby Digital Plus 7.1 (AC-3 core + dependent frame) yields a track.
- **Head-of-line.** Tracks are still promoted in PMT order, but a head that cannot resolve no longer blocks everything behind it forever. Non-video heads still `Probing` after 512 access units (`PROBE_HEAD_BLOCK_AUS`) are stepped over. A head that has delivered **no payload packet at all** (a declared but absent PID, in any codec, video included) is stepped over once the PIDs behind it hold 2 MiB (`SILENT_HEAD_PARKED_BYTES`, half of the 4 MiB `MAX_PROBE_BACKLOG_BYTES`) of parked backlog in total. A head that has delivered any data keeps blocking exactly as in 0.26.0, so the video keeps track id 1 and consumers that assume video is track 1 (`ts_hls`, `multimux`) are safe; a skipped head that later resolves is still promoted with a `TrackAdded` (a later id) and its whole backlog.
- **`TracksResolved`.** A stepped-over head counts toward `DemuxEvent::TracksResolved` ("nothing more is being waited on"), and a late promotion re-arms the signal, so `multimux` announces the programme and then emits `TracksChanged`. `finish()` no longer double-counts a skipped head (`resolved` and `skipped` stay disjoint).
- **Public golden `m6-single.ts`.** Track 1 (PID 0x82, `stream_type` 0x06 + Enhanced AC-3 descriptor) gains one real E-AC-3 frame (7 -> 8 samples): a `0B 77` pair whose byte 5 read as `bsid` <= 8 used to be parsed as an E-AC-3 header with a garbage length and swallow it; an AC-3-shaped candidate is now judged by its AC-3 CRC. Codec, track ids and other tracks are unchanged; update goldens that pin per-track sample counts of the three `m6-*` captures.
- **Known limitation (unchanged from 0.26.0).** A video PID that delivers data but never yields a parseable SPS keeps blocking every later track until `finish()` or the 4 MiB per-PID backlog cap, exactly as before; scrambled video delivers payload packets and is bounded only by the byte cap. A video that starts later than 2 MiB of other-track data is now reordered (announced with a later id when it arrives), where 0.26.0 would have abandoned the audio at 4 MiB.

### 6. Behaviour: MPEG-H 3D audio descriptor in `TsMux`

The synthesised `MPEG-H_3dAudio_descriptor` ES_info entry is now the complete ISO/IEC 13818-1 Table 2-121 form `3F 04 08 <profile_level> 7F C1` (an `extension_descriptor` carrying `mpegh3daProfileLevelIndication`, `interactivityEnabled`, `compatibleProfileSetsPresent`, `reserved`, `referenceChannelLayout`), built through `dvb-si`'s typed `MpegH3dAudio` rather than hand-assembled constants; the old 4-byte form omitted the two mandatory bytes. `referenceChannelLayout` comes from the `mhaC` record; a value above the 6-bit field (`> 0x3F`) is written as `0` (unspecified) rather than failing the mux. `interactivityEnabled` is written 0 and `compatibleProfileSetsPresent` 1 because the IR carries neither. TS output for MPEG-H tracks changes byte-for-byte; goldens that pin it must be regenerated.

### 7. Smaller API and behaviour changes

- **`Media` is `#[non_exhaustive]`** (BREAKING): a `Media { .. }` literal no longer compiles outside the crate.

  ```rust
  // before
  let m = Media { tracks, movie_timescale, pcr: vec![], skipped: vec![] };
  // after
  let mut m = Media::new(tracks, movie_timescale);   // set the remaining public fields (pcr, skipped, degraded) on it
  ```
- `colr` (`nclx`) `reserved` is a checked `fit_mask` write: an over-range hand-set value returns `FieldOverflow` instead of being masked into a different value. Likewise version-0 `mvhd`/`tkhd`/`mdhd`/`prft` fields above `u32::MAX`, `mp4esds` `bufferSizeDB` over 24 bits and `stsz` sizes over `u32` return `Error::FieldOverflow` instead of wrapping (#1129).
- Zero-width `trun` allocation is budgeted per `moof` (`Error::ZeroWidthTrunBudgetExceeded { limit }`, reported through `Media::degraded`), and a `largesize` sample entry shorter than its header no longer panics.
- `SegmentTimeline::resolve` takes the media-timeline `period_end_ticks`; use the new `resolve_in_period(start_number, presentation_time_offset, period_duration_ticks)` if you hold the Period length. A negative `@r` whose next `S@t` is not after its start is `DashParseError::TimelineNotIncreasing { t, next_t }` instead of silently yielding no segments.
- `vttc` with a repeated `payl`/`sttg`/`iden` child, and `av3c`/`lavc`/`la2c`/`av2c` with trailing bytes after the record, are rejected at parse instead of being mis-serialized or silently truncated.
- `smooth_parse::track_spec_from_quality_level` builds a subtitle `TrackSpec` for the `Type="text"` stream instead of `UnsupportedCodec`.
- ADTS multi-block frames with CRC have the `raw_data_block_position[]` table and CRC words stripped from the AAC sample (#1012); single-block and unprotected frames are byte-identical.
- `KeyMap` and `CencEncryptor` wipe content keys on drop and compare keys in constant time (`cenc` feature).

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-hls` 0.3 -> 0.4, `container-probe` 0.1 -> 0.2, `dvb-si` 11 -> 12, `mp4-emsg` 0.4 -> 0.5, `mpeg-pes` 0.5 -> 0.6, `mpeg-ps` 0.5 -> 0.6, `mpeg-ts` 0.5 -> 0.6, `rtcp-packet` 0.5 -> 0.6, `rtp-packet` 0.4 -> 0.5, `scte35-splice` 3 -> 4). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed (head-of-line bound of the unreleased 512-AU skip: non-video heads only, round-2 audit)
- Nothing here changes a published release: 0.26.0 has no head-of-line skip at all (a never-resolving head blocks every later track until `finish()`, or until the 4 MiB probe-backlog cap abandons it). The 512-access-unit skip (`PROBE_HEAD_BLOCK_AUS`) is introduced by the unreleased E-AC-3 promotion fix below, and as first written it also stepped over a stalled *video* head, so a video with a long GOP could come out as track 3 behind two audio PIDs promoted past it, breaking consumers that assume video is track 1 (`ts_hls`, `multimux`). The skip therefore applies to non-video heads only; a video head blocks until it resolves, the byte cap abandons it, or (round 3, below) it proves entirely silent.

#### Fixed (StreamingTsDemux head-of-line, round-3 audit)
- **A stepped-over head now counts toward `DemuxEvent::TracksResolved`.** With the unreleased 512-AU skip a stalled non-video head stayed unresolved, `resolved.len() == known` never held, `TracksResolved` was never emitted and `multimux`'s `ProgramTracker` never announced the programme (the promoted tracks' samples were dropped by its `track_program` guard). Semantics, now documented on the event: the signal means "nothing more is being waited on", so skipped heads count; a skipped head is not resolved and, if it resolves later, is still promoted with a `TrackAdded` (a later id) and its whole backlog, and that promotion re-arms `TracksResolved` so a consumer learns of it (`multimux` then emits `TracksChanged`). `ts_hls`/`ll_hls`/`media-plane`/`compliance-probe` do not wait on the signal; `multimux` `file_reader` ignores it. Tests: `ts_demux::tests::{stalled_head_counts_toward_tracks_resolved, stepped_over_head_resolving_late_is_promoted_and_rearms_the_signal}` and, end to end through `TsIngestSession` and `IngestDriver`, `multimux` `source::ts_program::tests::stalled_head_does_not_hide_the_programme_from_the_session`.
- **A declared PID that never sends a packet no longer blocks every later track.** A video PID in the PMT that is absent (a radio service with a stale video entry, video dropped upstream) held all later tracks back until end of input, in 0.26.0 as before, because nothing grows toward the backlog cap of the silent PID itself and the 512-AU bound has no access units to count. Criterion (round 4, replacing the first draft's 8192-stream-packet count, which reordered a video that started 0.3 s into a 38 Mbit/s mux and gave it track id 2): a head that has delivered no payload packet at all is stepped over, whatever its codec, once the PIDs behind it hold `SILENT_HEAD_PARKED_BYTES` (2 MiB, half of `MAX_PROBE_BACKLOG_BYTES`) of `Probing`/`Parked` backlog in total. That is a byte count of the other tracks' elementary stream, independent of the mux bitrate (about two minutes of 128 kbit/s audio). Honest comparison with 0.26.0: 0.26.0 never stepped over anything, it held later tracks behind a head that never resolved until the 4 MiB per-PID cap abandoned the *later* PID's backlog (the audio was lost) and, for a silent head, nothing ever resolved it before `finish()`; so this is not a behaviour loss, but a video that starts later than 2 MiB of other-track data is now reordered (announced with a later id when it arrives), where 0.26.0 would instead have abandoned the audio at 4 MiB. A head that has delivered any data keeps blocking exactly as in 0.26.0, so a slow-but-present video keeps the lowest track id, and a video head with data is never stepped over. A silent head that later sends data is still promoted (later id, backlog kept). Limit: scrambled video still delivers payload packets and remains bounded only by the byte cap. Tests: `declared_but_absent_video_does_not_block_audio` (asserts audio is announced exactly when the parked bytes reach the criterion, not earlier), `late_video_after_8192_stream_packets_keeps_track_one`, `slow_but_present_head_keeps_track_one_and_a_late_one_is_still_announced`, `present_but_unresolved_video_head_still_blocks_past_the_silent_bound`.
- `finish()` abandoned a still-`Probing` PID with `resolved.insert` but left it in `skipped` if it had been stepped over, so `resolved.len() + skipped.len()` double-counted it at end of input (spurious or suppressed `TracksResolved`); it is now removed from `skipped`. Test: `finish_abandoning_a_skipped_head_keeps_resolved_and_skipped_disjoint`.
- Removing an E-AC-3 track (PMT version change) while it was holding back its final access unit for a possible dependent substream (#1186) dropped that complete access unit; it is now emitted before `TrackRemoved`, as `finish()` does. Test: `removing_an_eac3_track_flushes_the_held_final_access_unit`.
- The E-AC-3 probe re-walked its whole held window (up to ~64 KiB) for every unresolved PES, so a stream of tiny PES carrying large frames cost O(PES count x 64 KiB) before the 16-frame bound (0.26.0's tail cap was 4 KiB). After 4096 re-walks of a non-empty window (`EAC3_PROBE_FULL_WALKS`) a payload that grows the window by less than an eighth is appended without a walk, which bounds the work at ~4100 walks plus a logarithmic few; any stream that resolves in fewer payloads (every real capture, and the existing frame bound for sane PES sizes) is untouched, and a pathological one defers its fallbacks (broken-chain, unverified, any-frame) until the window has grown by an eighth. Round 4: the walk-length bookkeeping (`probe_walked_len`) was not reset when the held tail was cleared or replaced, so a stale length above ~58 KiB made the gate demand growth past the 64 KiB tail cap, the tail cleared first and the window was never re-walked in full; it now follows every clear or replacement of the tail (test: `eac3_walked_len_follows_the_held_tail`). Test: `eac3_probe_rewalk_is_bounded_for_tiny_pes` (one-byte PES; reverted gate: 31 553 walks, test fails).
- An E-AC-3 track promoted from an unverified or broken-chain first frame (a lossy capture) announces no dependent substream, so it never held back a final access unit and a dependent frame that began a later PES was orphaned and dropped. A dependent frame *observed* folded into an access unit now turns the hold-back on (`dec3` is not changed after announcement; a dependent frame that began a PES before any was observed is still lost). Test: `observed_dependent_substream_enables_the_hold_back`.
- Removed the three `try_to_bytes().unwrap()` calls on `AudioSpecificConfig::from_adts_header` from the demuxer (they degrade to "no config observation" instead). Serializing an ADTS-derived config cannot fail, so this is a code-hygiene change with no output change and no biting test.

#### Fixed (E-AC-3 probe, round-2 audit)
- `Ec3SyncframeInfo::from_es_first_au_or_broken_chain` returned `Some(vec![])` when frame 0 itself failed to parse (a non-`BufferTooShort` error); it now returns `None` when there is no frame. Test bites (reverted: `Some(0)`).
- The `StreamingTsDemux` E-AC-3 probe built the tail+payload window three times per unresolved payload (probe window, broken-chain fallback, next tail); it is now built once and shared, and the fallbacks are skipped for a window too short to hold a frame. Behaviour is unchanged (the full suite is the oracle); the re-walk of the bounded held tail per payload remains.

#### Fixed (MPEG-H TS descriptor, H.222.0 Table 2-121)
- The synthesized `MPEG-H_3dAudio_descriptor` ES_info entry is now the complete Table 2-121 form (`3F 04 08 <profile_level> 7F C1`-shaped: `mpegh3daProfileLevelIndication`, `interactivityEnabled`, `compatibleProfileSetsPresent`, `reserved`, `referenceChannelLayout`), built from `dvb-si`'s typed `MpegH3dAudio` instead of hand-assembled constants. The old 4-byte form omitted the two mandatory bytes. `referenceChannelLayout` now comes from the `mhaC` record; a value above 6 bits cannot be represented and is written as `0` (unspecified), the same marker the demuxer uses (behaviour change from the earlier unreleased draft that returned `Error::InvalidValue`: H.222.0 §2.6.106 requires the descriptor for `stream_type` 0x2D, so omitting it or failing the whole mux was wrong; TS 101 154 §4.1.8.31 only limits it to at most one per loop). `interactivityEnabled` is written 0 and `compatibleProfileSetsPresent` 1 because the IR carries neither (also noted in the README). The previous claim that H.222.0 2.6.106 is "paid and not vendored" was wrong: it is vendored; the table is transcribed (verified on the rendered page) in `docs/codec/mpegh-3daudio-descriptor-h222.md`. Real-fixture cross-check `3F 04 08 10 7F C1` retained.

#### Fixed
- Serialize: the `colr` nclx `reserved` byte was masked silently (`& MASK`), so an over-range hand-set value was written as a different value; it is now a checked `broadcast_common::len::fit_mask` returning `FieldOverflow` (the #1129/#1197 class, found by the extended `serialize_mask_guard` of adversarial review #1268).

#### Fixed (StreamingTsDemux E-AC-3 promotion regression, zenith)
- **`StreamingTsDemux` emitted only the video track of a real French TNT capture** (video + three E-AC-3 PIDs signalled as PMT `stream_type` 0x06 + descriptor 0x7A + a DVB subtitle/data PID); 0.26.0 emitted all of them. Root cause: #1192 (#1186) made the E-AC-3 config probe wait for a *provably complete* first access unit (a following independent frame), and PMT-order promotion (`try_promote_ready`) stops at the first still-`Probing` PID, so one E-AC-3 PID that never completed held back every later track (the other two E-AC-3 PIDs and the subtitle PID sat `Parked`) until `finish()`, which a live feed never calls. The capture has dropped packets, so after the first whole syncframe the bytes are not a frame chain and no next independent frame is ever seen. Fixed at the root, three ways: (1) when no chain-verified first access unit is available in a payload, one whose frame chain *breaks* after a whole frame is accepted (`Ec3SyncframeInfo::from_es_first_au_or_broken_chain`, new, additive) (bytes that are not a syncword; frames are contiguous per ETSI TS 102 366 §E.1.3.1.2, so no dependent frame can follow), running out of data or a truncated next frame is still waited on (the #1186 7.1 behaviour is kept). Behaviour change on the public `m6-single.ts` capture: its second E-AC-3 track's `dec3` `bsmod` is 2 again (the isolated first frame, as 0.26.0) instead of 0 (#1186 waited for a later chain-verified frame); the golden is re-blessed; (2) bounded fallback: an E-AC-3 probe holding 16 chained frames (`EAC3_PROBE_MAX_FRAMES`, counted in frames not PES so a muxer's PES cutting cannot trigger it) without its first access unit being provably complete resolves from them, as end of input does; (3) a `Probing` PID that has seen 512 access units without resolving (`PROBE_HEAD_BLOCK_AUS`) no longer blocks later-ranked `Parked` PIDs, which are promoted past it (below the bound PMT declaration order, hence track ids, is unchanged). Every consumer (`TsDemux`, `ts_hls`, `media-plane`, `multimux`, `hls-runtime`, `compliance-probe`) goes through this one engine. Tests: `tests/streaming_ts_demux_eac3_promotion.rs` (real capture, private submodule), `ts_demux::tests::{stalled_probe_does_not_block_later_pids_forever, slow_probe_still_blocks_later_pids_below_the_bound, eac3_probe_that_never_completes_resolves_after_the_bound}`, `ac3::tests::first_au_completes_on_a_broken_chain_but_not_on_missing_data`.

#### Fixed (StreamingTsDemux follow-ups to the zenith fix: E-AC-3 `bsmod`, CRC, programs)
- **The `m6-single.ts` `bsmod` 0 -> 2 re-bless is the verified truth, and the E-AC-3 probe now chooses its frame by `crc2` and takes `bsmod` from the stream's intact frames.** An independent parse of every intact (CRC-valid, ETSI TS 102 366 §E.2.2) frame's `bsi()` shows PID 0x83 (PMT `component_type` 0xD2 = Visually Impaired; `ffprobe` reports `visual_impaired`) carries `bsmod` 2 in every intact frame that carries it, and PIDs 0x82/0x84 (0xC2 = Complete Main) carry 0; but the encoder sets `infomdate` in some frames only, and `dec3.bsmod` of a frame without it is 0 (§F.6.2.8), so the old 0 was the #1186 bug (a config read from a frame that happened to lack `infomdate`), not a different stream. The probe now (1) prefers an access unit whose frames all pass `crc2` (new `Ec3SyncframeInfo::from_es_first_au_crc_verified`; a damaged or false-sync frame is skipped), falling back to the first header-valid frame only when no intact one is held (liveness on lossy captures, as #1315), and (2) takes `bsmod` from the first intact frame of the window that carries one (new `Ec3SyncframeInfo::first_crc_verified_bsmod`). No golden changes. Tests: `tests/streaming_ts_demux_eac3_bsmod.rs` (independent BSI parse + PMT vs the demuxer's `dec3`, both captures), `ts_demux::tests::{eac3_probe_skips_a_damaged_frame_in_front_of_an_intact_one, eac3_bsmod_comes_from_the_first_intact_frame_that_carries_it, eac3_probe_with_no_intact_frame_still_announces_the_track}`, `ac3::tests::{eac3_bsi_bit_vector_is_hand_computed, crc_verified_first_au_rejects_damaged_frames, first_crc_verified_bsmod_skips_absent_and_damaged}`.
- Documented (no code change; module docs of `ts_demux`): the demuxer follows the PAT only, so the unreferenced PMT for service 0x0601 that shares the capture's PMT PID yields no tracks and loses nothing (its own PIDs have no packets); a PMT-declared PID that never carries a packet (0x8D, HD DVB subtitles) is deliberately not announced from the PMT (it would add an empty `Data` track every ISOBMFF muxer rejects), at the cost of `TracksResolved` waiting for it; and an E-AC-3 track cannot be announced from the PMT `enhanced_AC-3_descriptor` alone because `data_rate`, `fscod`, `acmod`, `lfeon`, `bsmod` and the dependent-substream fields of `dec3` are not in it (field table in the module docs).

#### Fixed (adversarial review of transmux, `review/transmux`)
- **Panic on a short `largesize` sample entry (hostile init segment).** #1147 started a box body after the 16-byte `largesize` header but the minimum-length guards still assumed 8 bytes, so a 19-byte `largesize` `stpp` or an 86-byte `largesize` `av01`/`vp09`/`avs3` indexed out of bounds (also through `SampleDescriptionBox::parse`); `TrackExtendsBox` (`trex`) and the audio sample-entry reader had the same flaw, and the `av01`/`vp09`/`avs3` child region started 8 bytes early for a `largesize` entry. New `box_types::box_body(bytes, min_body, what)` derives every minimum from the real header size; each reader uses it. Tests: `tests/largesize_truncation.rs` (every sample-entry fourcc x both header forms x every body length 0..=160 x 4 fills, plus `trex`/`schi`/`sinf`), `review_findings::short_largesize_sample_entries_panic_instead_of_erroring`, and a new fuzz target `transmux_sample_entries`.
- **Memory blow-up from many zero-width `trun`s.** `MAX_ZERO_WIDTH_TRUN_SAMPLES` applied per `trun`, so 16 sixteen-byte `trun`s (256 input bytes) allocated ~1.28 GiB. The allowance is now aggregate per `moof` (and per run of `moof`s waiting for their `mdat` in `Fmp4Demux`); exceeding it is the typed `Error::ZeroWidthTrunBudgetExceeded { limit }` (a `moof` rejected that way is reported in `Media::degraded`). Tests: `review_trun_aggregate_alloc`, `tests/trun_budget.rs`.
- **E-AC-3 hold-back (#1186) lost or mis-timed an intact access unit.** A continuity gap in the PES after a held access unit threw the held (complete, intact) access unit away with the damaged one; it is now emitted before the carry is dropped, and the cursor advances past it. When the next PES re-anchors the timeline (PTS jump), the held access unit keeps its own timeline (previously it was stamped `new_pts - lead`, moving it by the whole jump). Tests: `review_findings::eac3_cc_gap_in_next_pes_drops_the_held_intact_access_unit`, `eac3_held_access_unit_is_retimed_onto_the_next_pes_timeline_after_a_pts_jump` (real 7.1 capture, private submodule).
- **`SegmentTimeline::resolve` contract.** `S@t` and `$Time$` are on the MEDIA timeline; the Period-relative time is `S@t - @presentationTimeOffset` (ETSI TS 103 285 §10.9.7). `period_end_ticks` is therefore on the same media timeline (`@presentationTimeOffset + Period length`), now stated in the doc comment. New `SegmentTimeline::resolve_in_period(start_number, presentation_time_offset, period_duration_ticks)` adds the offset for a caller that holds the Period length (a bare Period length under-counts by `PTO / d` segments and yields none once PTO exceeds it). A negative `@r` whose next `S@t` is not after its own start is the typed `DashParseError::TimelineNotIncreasing { t, next_t }` instead of silently producing no segments.
- `vttc` with a repeated `payl`/`sttg`/`iden` child is rejected at parse (ISO/IEC 14496-30 `VTTCueBox` lists each once); it used to parse and then serialize to more bytes than `serialized_len()` reported.
- `av3c` and `lavc`/`la2c` (and `av2c`) reject bytes after the configuration record instead of dropping them on re-serialize.
- `smooth_parse::track_spec_from_quality_level` builds a subtitle `TrackSpec` (synthesised `stpp` TTML-namespace / `wvtt` `WEBVTT` sample entry) for the `Type="text"` stream `SmoothPackager` writes, instead of `UnsupportedCodec`; test `tests/smooth_text_roundtrip.rs` (packager -> parse).

#### Fixed (E-AC-3 dependent substreams, a real 7.1 stream, #1255)
- An AC-3 syncframe (`bsid` <= 8) is now an independent substream 0 of an E-AC-3 stream (ETSI TS 102 366 §E.1.3.1.2; Annex J.1.5.2.2 allows `bsid` 6 or 8 with a dependent substream). `Ec3SyncframeInfo`, `ac3::split_eac3_syncframes` and `StreamingTsDemux` recognised E-AC-3 syntax only, so a real Dolby Digital Plus 7.1 stream (AC-3 core + dependent E-AC-3 frame, FFmpeg FATE `the_great_wall_7.1.eac3`) produced no track from MPEG-2 TS (`stream_type` 0x87, or 0x06 + descriptor 0x7A) and no access units.
- `Ec3SpecificBox::from_syncframes`: `num_dep_sub` is the number of dependent substreams (highest dependent `substreamid` + 1). It was the last dependent frame's `substreamid`, which is 0 for the first (dependent substreams are numbered from 0), so a lone dependent substream gave `num_dep_sub` 0 and no `chan_loc`. MP4Box (GPAC 26.07) and ffmpeg 8.1.2 both write 1; the resulting `dec3` for the real stream matches MP4Box's byte for byte (`24 00 0c 0f 02 02`).
- Track counts on the three `m6-*` captures: track 1 (PID 0x82, signalled `stream_type 0x06` + Enhanced AC-3 descriptor) gains one real E-AC-3 frame (7 -> 8 samples): a `0B 77` pair in the payload whose byte 5 read as bsid <= 8 used to be parsed as an E-AC-3 header with a garbage length and swallow it; an AC-3-shaped candidate is now judged by its AC-3 CRC. Codec, track ids and the other tracks are unchanged; plain AC-3 PIDs are unaffected (`track_id_baseline` golden updated with this justification).
- Real-stream tests (`tests/eac3_real_dependent_substreams.rs`; the stream has an unstated licence so it is in the private submodule `private/fixtures/eac3-dep/` and the tests skip loudly without it) against the tools' own output: gpac's per-sample CRC32 of MP4Box's file, MP4Box's `dec3`, MPEG-2 TS from ffmpeg and from gpac (PES boundaries inside access units), both producers' MP4s through demux and CMAF. ffmpeg's `dec3` for the same stream disagrees (data_rate 576, `chan_loc` bit 4); the dependent frame's own `chanmap` (Lrs/Rrs) supports MP4Box.

#### Changed (BREAKING) / Added (real wvtt producers, #1249)
- **BREAKING:** `VttCueBox` gains `extra_children` (every `vttc` child other than `payl`/`sttg`/`iden`, e.g. `vsid`/`ctim`, kept verbatim) and remembers the wire order of its children. Before, a parsed cue was written back `payl, sttg, iden` and unknown children were dropped, so it did not serialize to the bytes it was parsed from; MP4Box and gpac both write `iden, sttg, payl`, which is now also the order of a cue built with `VttCueBox::new` (after any extras). The struct literal can no longer be built outside the crate; use `VttCueBox::new` and set the public fields.
- New `VttAdditionalTextBox` (`vtta`, the `NOTE` text both producers write ahead of the cue after a comment), `WvttSampleBox` (`Cue`/`Empty`/`AdditionalText`/`Other`) and `parse_wvtt_sample`: a `wvtt` media sample as typed boxes, serializing back to the same bytes.
- Real `wvtt` / `stpp` artifacts from MP4Box and gpac (progressive, fragmented, DASH) under `fixtures/subtitles/`, checked against the tools' own parse (gpac `inspect` per-sample dts/cts/dur/size/CRC32; MP4Box `-raw` WebVTT export for cue ids, settings, text and NOTE blocks).

#### Fixed (E-AC-3 orphan dependent substream, #1245)
- An E-AC-3 dependent-substream syncframe with no independent frame before it (the data starts on it at stream start, or after a continuity gap / dropped PES / re-anchor cleared the carry) was emitted as a sample of its own with an independent frame's duration, shifting every following timestamp by one frame. It cannot be decoded alone (ETSI TS 102 366 §E.1.3.1.2: dependent substreams immediately follow their independent substream), so `StreamingTsDemux` and `ac3::split_eac3_syncframes` now drop it; a dependent frame after its independent frame still folds into that access unit.

#### Changed
- The Annex B start-code scan (`find_start_code_prefix`, behind every NAL and access-unit
  splitter) finds each `01` byte with `memchr` instead of testing every offset. Same results,
  `annexb_to_length_prefixed` 2.6–4.7× faster and `StreamingTsDemux` 13–39 % less time on the
  H.264 fixtures (new `annexb_scan` bench). Adds a `memchr` dependency
  (`default-features = false`, so the crate stays `no_std`).
- The H.264 I-slice check behind `StreamingTsDemux`'s random-access-point detection unescapes
  only the first 32 bytes of each slice NAL, enough for the two `ue(v)` slice-header fields it
  reads, instead of copying the whole slice. Same results, 17–44 % less `StreamingTsDemux` time
  on the H.264 fixtures on top of the `memchr` start-code scan.

#### Added (subtitles in manifests, #1243)
- `DashPackager` writes a subtitle track as an `AdaptationSet` with `contentType="text"`, `mimeType="application/mp4"` and `Representation@codecs` `stpp` (TTML/IMSC) or `wvtt` (WebVTT), after the video and audio sets (ETSI TS 103 285 §7.1.1; bare `stpp` is the form that clause allows when no TTML profile is declared, and the `stpp` sample entry carries none). New `MediaKind::Text`. `rfc6381_codec_string` returns `stpp` / `wvtt` for such a track; a `Subtitle` track with no ISOBMFF sample entry (TS DVB / Teletext) is still `UnsupportedCodec`.
- `SmoothPackager` writes a subtitle track as `StreamIndex Type="text" Subtype="SUBT"` with `QualityLevel FourCC="stpp"` / `"wvtt"` and the samples as ordinary Smooth fragments ([MS-SSTR] §2.2.2.3 / §2.2.2.5). New `SmoothStreamType::Text`, `FOURCC_STPP`, `FOURCC_WVTT`, `SUBTYPE_SUBT`. The error for any other unsupported codec now names subtitles as supported.

#### Added (#1234 - AVS2/AVS3 video and Audio Vivid ISOBMFF boxes, GY/T 420-2025 Annex A)
- `avs3`: `Avs3ConfigurationBox` (`av3c`, A.3.2.2) with `LibraryDependencyIdc` (Table A.1), `LAvsConfigurationBox` (`lavc`/`la2c`, A.3.2.3/A.2.2.3, per-layer `TemporalLayerInfo`, typed `AvsFrameRateCode`), `AvsVideoSampleEntry` (`avs3`/`lav3`/`avst`/`lav2`; byte-exact children, secondary layer box decodable), and the `telg`/`lrap`/`a3lg` sample-group entries (`TemporalLayerEntry`, `VisualLrapEntry`, `LibrarySampleGroupEntry`).
- `avs2`: `Avs2ConfigurationBox` (`av2c`, A.2.2.2).
- `avs3_audio`: `Ca3SpecificBox` (`dca3`, A.4.3.1) with the general-bit-rate and lossless `Avs3Audio*SpecificConfig` (bit-packed, typed `content_type`).
- Not modelled: the `lidx` LRAP segment index box and the `av3l` restricted-scheme wrapper (A.3.3.3/A.3.3.6), the `av3a` audio sample entry wiring in the init-segment machinery, and `sequence_header()` decoding (carried verbatim).

#### Added (#1227)
- Caption SEI extraction (`caption_cc_data`) also accepts the China GY/T 270-2013 clause 6.3.3 form: `itu_t_t35_country_code` `0x26` (provider 0x0031, `GA94`, type 0x03), in addition to the ATSC `0xB5`.

#### Added
- `dash::xs_duration(Duration) -> String` is public: the workspace's one ISO 8601 / `xs:duration` spelling (jiff `SpanPrinter`); a duration beyond `i64::MAX` seconds saturates to the largest representable value. `multimux` now calls it instead of its own copy (#1216).

#### Changed
- Internal: the 5-byte marker-bit PTS/DTS/`DTS_next_AU` field is encoded/decoded by `broadcast_common::clock33::{pack_marked, pack_marked_with, unpack_marked}` instead of a local copy (each crate keeps its own prefix / marker validation and error variants); no behaviour change (#1208).

#### Fixed (DASH SegmentTimeline, #1187)

- `SegmentTimeline::resolve(start_number, period_end_ticks)` (new, with `TimelineSegment { number, time, duration }`) implements ISO/IEC 23009-1 §5.3.9.6 in full: absolute first `S@t`, explicit `S@t` jumps (timeline gaps) and a **negative `S@r`**, which repeats to the next `S`'s explicit `@t` (`ceil((t_next - t) / d)`) or to the Period end (`ceil((period_end - t) / d)`), as transcribed by DASH-IF IOP v4.3 §3.10.2.2.4 (`specs/rules/dash-mpd-rules.md`). `enumerate` is kept as the no-Period-end wrapper (a negative `r` with no bound is still a single occurrence).

#### Changed (BREAKING, #1186)

- **BREAKING: `CodecConfig::Subtitle` carries its ISOBMFF sample entry (ex-`TODO(#753)`).** The variant gains `sample_entry: Option<SubtitleSampleEntry>` (new `#[non_exhaustive]` enum `Ttml(XmlSubtitleSampleEntry)` / `WebVtt(WvttSampleEntry)`, re-exported from `ir` and `pipeline`). `Fmp4Demux` fills it from the `stpp`/`wvtt` entry whole (namespace, `vttC` header block, trailing boxes), and `build_init_segment` and every packager built on it now re-emit the track: the entry verbatim, an `sthd` (`stpp`/`subt`) or `nmhd` (`wvtt`/`text`) media header kept first in `minf` (ISO/IEC 14496-12 §6.2.3), handler `subt`/`text`. `CodecConfig::is_muxable_in_bmff()` is now true for a subtitle track that has a sample entry; one without (`sample_entry: None`, e.g. a TS DVB/Teletext subtitle) is still rejected with `Error::UnmuxableSubtitleTrack`. (DASH/Smooth manifest packagers write subtitle tracks since #1243, see "Added (subtitles in manifests, #1243)" below; only a track with no ISOBMFF sample entry is still `UnsupportedCodec`.) Migrate patterns `CodecConfig::Subtitle { format }` to `{ format, .. }`. Tests: `tests/subtitle_remux.rs` (real ffmpeg `stpp` CMAF round trip, `wvtt` spec vector, entry-less rejection).

#### Fixed (TS demux, #1186)

- **E-AC-3 dependent substreams join their access unit across PES boundaries.** A dependent substream (`strmtyp == 1`) that began a PES used to be emitted as a separate sample; when the programme has dependent substreams (7.1 / Atmos), the final access unit of each PES is now held back until the next PES shows whether a dependent frame follows (ETSI TS 102 366 §E.1.3.1.2: dependent substreams follow their independent substream), and released at the next PES or by `finish()`. Programmes without dependent substreams are untouched (nothing is held). A dependent-frame prefix no longer counts as an access unit commencing, so it cannot move the timeline anchor.
- **E-AC-3 config recovery waits for a complete access unit.** The `dec3` was built from the first independent frame found, so a PES ending right after it produced a config without the dependent substreams (7.1 signalled as 5.1). The probe now resolves only once the first access unit is provably complete (the next independent frame has been seen), joining PES payloads up to 16 syncframes, and at end of input accepts whatever the access unit holds. A leading dependent frame is skipped. Tests: `ts_demux::tests::eac3_*` (independent frames from the committed Dolby capture plus a spec-built dependent frame; a real capture with dependent substreams was found later and is covered by #1255, `tests/eac3_real_dependent_substreams.rs`), `tests/eac3_real_capture.rs` (committed captures and the local archive pinned to the pre-change output).
- **PPS `A -> B -> A` is re-reported (follow-up of #1166).** PPS tracking is keyed by `pic_parameter_set_id` (H.264 §7.3.2.2 / H.265 §7.3.2.3.1): re-sending an id with identical bytes is silent however ids interleave (the #1166 rotation), while an id announced with different bytes is a `TrackUpdated` at every switch; the old "never seen before" test missed the return to an already-seen set. Tests: `ts_demux::tests::pps_a_b_a_on_one_id_is_reported_at_every_switch`, `tests/pps_real_capture.rs` (real captures stay at zero updates). The #1166 rotation tests now rotate real distinct ids (their synthetic "rotation" reused one id with different bytes, which is a replacement per the spec).

#### Fixed (no_std builds, #1181)

- `--features cenc` and `--features sample-aes` now build for bare-metal (`thumbv7em-none-eabi`): `cenc_decrypt` and `sample_aes` were missing `alloc` imports (`ToString`, `format!`) that only the `std` prelude supplied. CI now builds these optional features on the no_std target.

#### Changed (#982)

- `cenc_decrypt`'s private box-navigation copies (`find_box`, `iter_child_boxes`, `iter_top_boxes`, `descend`) are replaced by shared `box_types` helpers (`child_boxes`, `find_child_box`, `descend_boxes`) built on `box_slices`. Behaviour is identical on every real fixture (pinned by an agreement test); the shared helpers also read the true container header length, so a `largesize`-form container's children are found where the old hardcoded 8-byte skip missed them.

#### Security (secrets and identity handling, #1142)

- `KeyMap` and `CencEncryptor` wipe their content keys when dropped (`zeroize`), and `KeyMap` equality compares keys in constant time (`subtle`). Both dependencies are pulled in by the `cenc` feature only. New `tests/key_hygiene.rs` and `tests/secret_debug_guard.rs`.

#### Fixed

- **Over-range version-0 box fields are rejected instead of wrapped (#1129).** `mvhd`/`tkhd`/`mdhd` `creation_time`/`modification_time`/`duration` and `prft` `media_time` above `u32::MAX` now return `Error::FieldOverflow` when `version == 0` (they were written as `value as u32`); `mp4esds` `bufferSizeDB` over 24 bits and `progressive` `stsz` sizes/counts over `u32` likewise. `flv_frame_payloads` (the RTMP payload path) now applies the same 32-bit-millisecond and signed-24-bit `CompositionTime` checks as `FlvMux::package` instead of `as u32`/`as i32`. Tests: `tests/serializer_length_truncation.rs` (also a lexical guard against unchecked `.len() as uN`), `tests/flv.rs::frame_payloads_reject_out_of_range_timestamps`.
- **`trun` with no per-sample fields can no longer drive an unbounded allocation (#1130).** Every entry of such a `trun` is zero bytes wide, so the remaining-bytes bound never limited `sample_count`: four overwritten bytes of `fixtures/mp4/llcmaf/ll_chunked.mp4` made `Fmp4Demux` request a 10 GB `Vec`. `sample_count` is now capped at `MAX_ZERO_WIDTH_TRUN_SAMPLES` (2^20) and `bounded_entry_count` never sizes an up-front allocation from a zero-width count. Found by a hostile-count sweep that is now committed (`tests/hostile_count_sweep.rs`, 21 702 inflated-count parses of every committed MP4 fixture under a 128 MiB live-byte cap) next to the pinned repro (`tests/trun_zero_width_alloc_bound.rs`).
- **ADTS multi-block frames with CRC (#1012).** A frame with `protection_absent == 0` and `number_of_raw_data_blocks_in_frame > 0` now has its `raw_data_block_position[]` table and every `crc_check` word (header-level and per-block, ISO/IEC 13818-7:2004 §6.2 Tables 5-7) stripped; the AAC sample is the concatenated `raw_data_block()` payloads. A self-inconsistent position table degrades to stripping only the header CRC and table instead of rejecting the frame. CRC values are not validated. Layout transcribed in `docs/codec/adts-error-check.md`. Single-block and unprotected frames are byte-identical to before.

#### Changed
- `AudioSpecificConfig`'s stack-less fallback no longer panics on an unserializable value (it hands the parser an empty buffer); `smooth` tfxd splice propagates the serializer error.

- **BREAKING (#1159): typed track and program ids.** New `TrackId(u32)` and `ProgramNumber(u16)` newtypes (re-exported at the crate root and from `ir`; `new`/`value`, no silent `From`, `Debug`/`Display` render the bare number and serde is `transparent`, so every golden, JSON and manifest is byte-identical). Every public `track_id: u32` is now `TrackId`: `TrackSpec::{track_id, new}`, `Track::track_id()`, `FragmentTrackData`, `DemuxEvent` track fields (`Sample`, `TrackRemoved`, `TrackAbandoned`, `Discontinuity`, `InputDegraded`), the `push(track_id, ..)` of the segmenters, `RtpInputStream`/`RtpTrackInit`, the ISOBMFF box structs (`tkhd`/`tfhd`/`trex`/`mvhd.next_track_id`/`prft.reference_track_id`), `Error::UnmuxableDataTrack`, the Smooth/DASH track descriptors, and `TrackSpec::program_number` (now `Option<ProgramNumber>`, `with_program(ProgramNumber)`). `TrackId::next()` hands out consecutive ids. Migrate with `TrackId::new(n)` / `.value()`.
- Test: `tests/track_id_baseline.rs` demuxes every committed fixture (and every `.test-streams` capture up to 40 MB when present) and compares per-track ids and sample counts with `tests/golden/track_ids.txt`, captured while `track_id` was a bare `u32`.

#### Changed (BREAKING)

- **BREAKING (#1159, typed PIDs):** `TrackSpec::source_pid` is `Option<Pid>` and `TrackSpec::with_source` takes a `Pid`; `PcrSample::{pid,new}` and `EventProvenance::pid` are `Pid`. `Debug` of the IR now renders a PID as `Pid(0x0100)` instead of `256` (the `sweep_output_golden` hashes normalise this and are unchanged: the demux output itself is identical). `TsMux` reports `Error::InvalidInput` if a track would be assigned an elementary PID above `0x1FFF`.

#### Fixed

- ISOBMFF box sizes past `u32::MAX` use the `size = 1` + 64-bit `largesize` form (ISO/IEC 14496-12 §4.2) instead of silently wrapping (#1147). Every variable-size box writer (sample tables `stts`/`ctts`/`stsc`/`stsz`/`stco`/`co64`/`stss`/`elst`, `stsd` and every sample entry, `hdlr`/`dref`/`url `, `esds`/`avcC`/`hvcC`/`vvcC`, CENC `senc`/`pssh`/`saiz`/`saio`/`schm`/`schi`/`sinf`, `sgpd`/`sbgp`/`subs`, the subtitle entries, `trun`, and the `moov`/`trak`/`mdia`/`minf`/`stbl`/`dinf`/`edts`/`mvex`/`traf`/`moof` containers) now derives its header through one shared helper, and its `serialized_len` counts the extra 8 `largesize` bytes, so the two stay consistent. `OpaqueBox`/`OpaqueChild` switch to `largesize` on their own when their payload no longer fits. Fixed-layout boxes (`mvhd`, `tkhd`, `mdhd`, `vmhd`, `smhd`, `mfhd`, `tfdt`, `prft`, `cslg`, `tenc`, `sidx`) and the brand-list `ftyp`/`styp` and the compact-only sample-entry `OpaqueBox` return a typed `FieldOverflow` error rather than a wrapped size. The typed readers (stsd/dref children, sample-entry config and extra-box scanners, `parse_body` slicing in every typed box) accept the `largesize` form. Output for every real fixture is byte-identical.
- One damaged unit no longer aborts a whole one-shot demux (audit T16, #1144): `PsDemux` resynchronises past a Program Stream pack that fails to parse (via `mpeg_ps::program_stream::scan_packs`; input with no recoverable pack is still an error), `Fmp4Demux` skips a `moof` whose body does not parse, and the batch `RtpDepacketiser` skips a packet with a malformed RTP header, STAP-A, FU-A or AAC AU-header, then carries on with the data after it.

- `WebmDemux` skips a corrupt `Cluster` (resynchronising at the next Cluster ID), `TrackEntry` or (Simple)Block, `Fmp4Demux` skips a `traf` whose `trun` addressing/timing is invalid (committed atomically per `traf`), and `FlvDemux` skips a tag with damaged framing (resynchronising on the next consistent tag chain via StreamID/`DataSize`/following-header checks), a corrupt NAL-length framing, or a corrupt AVC/AAC sequence header; each is reported in `Media::degraded` and the data after it is decoded unchanged. A truncated *final* tag/cluster is still the tail case, not damage. Behaviour change: inputs that used to fail the whole demux on such damage now succeed with `degraded` populated.

#### Added

- **BREAKING (listed here with the addition that caused it):** `Media` is now `#[non_exhaustive]`, so a `Media { .. }` struct literal no longer compiles outside the crate; build one with `Media::new(tracks, movie_timescale)` and set the public fields.
- `Media::degraded` (`Vec<InputDegradation>`; `Media` is `#[non_exhaustive]`) and `InputDegradation::MalformedUnit { position }` (the enum is `#[non_exhaustive]`): the skipped unit is reported, never silent. `position` is the byte offset (PS pack, fMP4 box) or the packet index in the stream (batch RTP).

## Merged changes since `transmux-v0.26.0`

- fix(transmux): round-4 audit (silent-head step-over on parked bytes, E-AC-3 walked_len, finish disjoint sets, push-release-tags dep abort) (#1329)
- fix(transmux): round-3 audit of StreamingTsDemux head-of-line, E-AC-3 hold-back and probe work (#1326)
- fix: round-2 audit (transmux demux ids, MPEG-H descriptor, st291 fixtures, E-AC-3 probe, dvb-t2mi function entries) (#1322)
- fix(transmux): StreamingTsDemux follow-ups: E-AC-3 CRC-verified frame choice and stream-level bsmod, programs/silent PIDs/PMT-only docs (#1319)
- docs: README coverage gaps (audit 24), media-doctor examples doc (#1317)
- fix(transmux): StreamingTsDemux promotes all tracks of lossy E-AC-3 captures (zenith) (#1315)
- docs(transmux): MPEG-H_3dAudio_descriptor is vendored (H.222.0 Table 2-121), fix stale 'unvendored/not reproduced' text (#1313)
- av3a lossless channel_number gap quoted from spec; MPEG-H descriptor from Table 2-121 (#1310)
- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- transmux: fix confirmed adversarial-review defects (supersedes #1269) (#1273)
- Fix the confirmed defects of the dvb-si descriptor review (supersedes #1268) (#1272)
- transmux: AC-3 core as E-AC-3 independent substream, num_dep_sub counts dependents; real 7.1 stream tests (#1255) (#1257)
- tests: consume deterministic excerpts of the france-tnt/hotbird captures instead of the 540 MB LFS originals (#1263) (#1264)
- captions: mux.js/shaka media (unclear licence) moved to the private submodule; tests read private/fixtures/captions (#1261) (#1262)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- transmux: real wvtt/stpp artifacts from MP4Box and gpac; vtta, typed sample parse, vttc child order (#1249) (#1254)
- transmux: drop an orphan E-AC-3 dependent substream instead of emitting it alone (#1245) (#1246)
- perf(transmux): memchr Annex B start-code scan + bounded unescape in avc_is_i_slice (supersedes #1153, #1154) (#1242)
- transmux: stpp/wvtt subtitle tracks in the DASH MPD and Smooth manifest (#1243) (#1244)
- feat(dvb-si,transmux): China closed caption (GY/T 270-2013) (C1) (#1228)
- feat(transmux): AVS2/AVS3 video and Audio Vivid ISOBMFF boxes (C4) (#1239)
- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- Real CEA-608/708 captures: fixtures, tests, and the bugs they exposed (#1220) (#1221)
- transmux: public dash::xs_duration; multimux uses it, copy and test seam removed (#1216) (#1217)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: pack_marked/unpack_marked for the 5-byte marker-bit timestamp; migrate mpeg-pes, mpeg-ts, st291, transmux (#1208) [stacked on #1207] (#1209)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- transmux: subtitle sample entry re-mux, E-AC-3 dependent substreams across PES, PPS A->B->A (#1186) (#1192)
- DASH: SegmentTimeline absolute t / negative r, AST = presentation origin (#1191)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- fix(transmux,dvb-si): no_std optional features build on thumbv7em (cenc, sample-aes, chrono) + CI step; dedupe cenc box-nav helpers (#1181, #982) (#1184)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
