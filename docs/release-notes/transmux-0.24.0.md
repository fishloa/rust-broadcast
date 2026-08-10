# transmux 0.24.0

**Release date:** 2026-08-10

A panic/DoS hardening release for the fMP4/CMAF/CENC parse paths. All seven
fixes below are reachable from ordinary or malformed third-party media — no
fuzzing required to hit any of them — so anyone demuxing untrusted MP4/CMAF
input should take this release. No public API breaks; one new named error
variant and one behavioural improvement (a previously-silent wrong answer
now fails loudly) are described under Migration.

## What changed

### Fixed — panics and DoS on malformed input

- **`cenc::TrackEncryptionBox::parse_body` panicked** on a `tenc` box body
  of exactly 19 bytes. The minimum-length guard undercounted the four
  1-byte fields ahead of the 16-byte `default_KID` (ISO/IEC 23001-7:2016
  §12.2.2) by one, so a 19-byte body passed the guard and then panicked
  slicing the KID at offset `4..20`. The guard now requires the correct 20
  bytes and returns `Error::BufferTooShort` instead.
- **`sample_entries::find_config_box` panicked** on a config-region child
  box whose wire `size` field (ISO/IEC 14496-12:2015 §4.2) claimed more
  bytes than the region actually holding it had left. The returned slice
  is now clamped to the region's remaining length, matching the bound
  `init_segment::parse_stbl_children` already applied to its own child
  walk.
- **`init_segment::parse_stbl_children` silently discarded malformed
  `stts`/`ctts`/`stsc`/`stsz`/`stco`/`co64`/`stss` boxes**, defaulting each
  to an empty typed box that falsely claimed the sample table had zero
  entries. All seven now get the same treatment `stsd` already had (from
  issue #952): kept as raw bytes (`StblChild::Opaque`) so the real parse
  error is recoverable; a new `progressive_demux::find_stbl_child` helper
  re-parses a matching `Opaque` box to surface that error at the point of
  use, so a corrupt sample table now fails the carrying track loudly
  (visible in `Media::skipped`) instead of silently behaving as absent or
  empty.
- **Seven wire-count-driven `Vec::with_capacity` sites in
  `init_segment.rs`** (`dref`/`stsc`/`stsz`/`stco`/`co64`/`stss`/`stsd`)
  allocated on an untrusted `u32` entry count before validating it — a
  16-byte `co64` box declaring `count = 0xFFFFFFFF` asked for roughly 32 GB
  up front. A new `bounded_entry_count` helper caps the count against what
  the remaining buffer could actually hold, computed before any
  allocation, the same discipline `cenc::SampleEncryptionBox::parse_body`
  already applied to `senc`'s `sample_count`.
- **`ll_hls.rs` divided by the anchor track's timescale with no zero
  guard** in three places (part-target-seconds, whole-segment duration,
  part duration). `ts_hls.rs` already guards the identical computation
  with `.max(1)`; `ll_hls.rs` now does too. A malformed zero `mdhd.timescale`
  previously turned these into `f64::INFINITY`/`NaN`, rendered verbatim
  into `#EXT-X-PART-INF`/`#EXT-X-PART`/`#EXTINF` — a wrong value shipped to
  every LL-HLS client, with no panic to flag it.
- **`repackage::anchor_index` only recognised `CodecConfig::Avc` as a
  video anchor**, so HEVC/AV1/VVC-only media (all otherwise supported by
  this crate) fell through to `unwrap_or(0)` — track 0, which may be
  audio — cutting `Media::trim`/resegment boundaries on an audio "keyframe"
  instead of a real video IDR, on ordinary well-formed input. It now
  delegates to the shared `segmenter::choose_anchor` (first video track of
  any codec, else the first anchor-capable track — the same fix issue #628
  made for `Segmenter`/`ts_hls`), so `Repackage` and `Segmenter` can no
  longer disagree on which track anchors segmentation.
- **`Fmp4Demux` dropped an entire H.264 video track** when a High-profile
  `avcC` omitted its optional ISO/IEC 14496-15:2017 §5.3.3.1.2 trailer
  (`chroma_format`/`bit_depth_luma_minus8`/`bit_depth_chroma_minus8`/
  `sps_ext`) — a real DASH-IF `livesim2` capture does exactly this, and
  ffmpeg reads it without complaint (issue #952). Two defects, both fixed:
  `avc_config::AVCDecoderConfigurationRecord::parse` was reading the
  trailer unconditionally for any High-profile family
  (100/110/122/244) profile, even with zero bytes remaining; it's now read
  only when at least one byte remains, and the fields stay `None` (never
  an invented default) when the encoder omitted them, with `Serialize`
  mirroring this so a trailer-less record round-trips without growing one
  back. Separately, `init_segment::parse_stbl_children`'s `stsd` arm was
  swallowing a parse failure into a blank placeholder that cost the entire
  track and surfaced only a generic error; it now keeps the raw bytes as
  `StblChild::Opaque` and re-parses them via
  `media::track_spec_from_trak` so the real error reaches
  `Media::skipped`'s `SkippedTrack::reason`. `hvcC`/`vvcC` were audited for
  the same shape and don't have it: `hvcC`'s chroma/bit-depth fields are
  unconditionally mandatory (ISO/IEC 14496-15:2017 §8.3.3, no `if` gate),
  and `vvcC`'s optional PTL block is already correctly gated by its
  on-the-wire `ptl_present_flag`.

### Changed — internal consolidation, no behaviour or API change

- `ts_demux`'s 33-bit PTS/DTS wrap-unroll now delegates to
  `broadcast_common::clock33::unwrap_delta`, the shared owner of this math
  also used by `timed-metadata`, `media-doctor`, and `compliance-probe`.
  `transmux`'s own algorithm was already the correct, bidirectional one;
  this consolidation changes nothing about its behaviour.
- Six independent MSB-first bit-extraction loops (RBSP/Exp-Golomb bit
  reader, `mpegh`, `vvc_config`, `ac3`, `dts`, `aac_asc`) now delegate the
  innermost bit extraction to `broadcast_common::bits::BitReader` (already
  reused by `dvb-t2mi`/`rdd29`/`st291`) instead of six separate
  re-implementations. Each module keeps its own bounds-checking, error
  type, and higher-level semantics unchanged.
- MSRV raised to **1.95.0** (issue #949), the workspace-wide MSRV
  unification.

## Migration

No breaking API removal or signature change. Two behaviours consumers
should be aware of:

- **New error path:** a `Media` with no anchor-capable track at all — a
  case the old `anchor_index` silently defaulted to track 0 for — is now a
  named `Error::InvalidInput` instead of an unchecked index-0 fallback. A
  caller matching exhaustively on `transmux::Error` should account for this
  (the type is `#[non_exhaustive]`, so an exhaustive match without a
  wildcard was already a compile error).
- **A stream that previously demuxed with an incomplete/empty sample table,
  or dropped a High-profile-without-trailer H.264 track, will now report
  that track as skipped** (via `Media::skipped`) rather than silently
  demuxing it as empty or absent, and a High-profile `avcC` without the
  optional trailer now demuxes its track correctly instead of being
  dropped. No action required unless your code specifically depended on
  the old silent-failure behaviour.

MSRV requires rustc 1.95.0 or newer.
