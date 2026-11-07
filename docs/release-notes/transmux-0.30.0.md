---
status: DRAFT
---

# transmux 0.30.0

_Released 2026-11-07._

Breaking-class release (0.29 -> 0.30). Dynamic DASH needs an explicit `publishTime`, `serde` is no longer a default feature, the error enum wraps the shared kinds, the DASH and Smooth manifest readers are namespace-aware and bounded (#1708), and the TS-HLS segmenter bumps the PMT `version_number` when its PMT changes (#1699).

Version: 0.29.1 -> 0.30.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `DashPackager::package` (and so `LlDashPackager::package`) returns `Error::InvalidField` (new variant) when `dynamic` is set and `publish_time` is `None`; ISO/IEC 23009-1 §5.3.1.2 Table 3 requires `MPD@publishTime` for `type="dynamic"`. `LlDashPackager` (always dynamic) behaves the same: set `base.publish_time` explicitly and advance it on every update. Static MPDs are unchanged.
- `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).
- error variants `BufferTooShort`, `OutputBufferTooSmall` and `InvalidValue` now wrap `broadcast_common::error::{BufferTooShort, OutputBufferTooSmall, InvalidValue}` (`Error::BufferTooShort(e)`); the hand-written `From` impls are replaced by `#[from]`. `OutputBufferTooSmall` and `InvalidValue` display with the shared message text. `FieldOverflow` now names the `broadcast_common::error` path (same type).
- `rtp::hex_decode`, `smooth_parse::hex_decode` and the `smooth_hex_decode` re-export are now crate-private; both only wrapped the `hex` crate with this crate's own error type. Use `hex::decode`.
- Unused public items `UUID_TYPE_BYTES`, `DashPackager::with_profiles`, `StblOpaque`, `new_vvi1`, `MhaCBox`, `user_data::{MIN_BAR_DATA_LEN, picture_user_data, carriage_for}`, `TrackSpec::with_hdr`/`with_dovi`, `SkippedTrack::with_track_id`, and `ir::sample::NON_SYNC`; `new_avc3`, `new_hev1`, `HdrStaticMetadata::hevc_sei_nal_carries_hdr` and `SENC_OVERRIDE_FIELDS_LEN` are now test-only; the AVS2 FourCC constants are now what `AvsVideoEntryKind` returns.

### `serde` is now opt-in

```toml
# before: serde was on by default
transmux = "0.29"
# after: opt in
transmux = { version = "0.30", features = ["serde"] }
```

Builds that serialize or deserialize these types must name the feature. Builds that only parse and serialize wire bytes get a smaller dependency tree and need no change beyond the version.

### Shared error kinds from `broadcast_common::error`

`broadcast-common` 11 defines the common failure kinds once, as `Copy` structs. This crate's error enum wraps them with `#[error(transparent)] Kind(#[from] broadcast_common::error::Kind)`, so the variants `BufferTooShort` and `OutputBufferTooSmall` (and any other kind named in the list above) changed from struct variants to tuple variants holding the shared struct. The fields are the same, so only the pattern changes, and `From` impls exist, so `?` and `.into()` still build them.

```rust
// before
match err {
    Error::BufferTooShort { need, have, what } => eprintln!("{what}: need {need}, have {have}"),
    _ => {}
}
// after
match err {
    Error::BufferTooShort(e) => eprintln!("{}: need {}, have {}", e.what, e.need, e.have),
    _ => {}
}
```

The display text of these variants now follows the common wording shared by every crate in the wave.

### Renamed and removed items

```text
hex_decode(s)  // rtp / smooth_parse
    -> hex::decode(s)
DashPackager::with_profiles
    -> (removed; unused)
```

### `DashPackager` and `LlDashPackager` require `publishTime` for dynamic MPDs

ISO/IEC 23009-1 section 5.3.1.2 Table 3 requires `MPD@publishTime` for `type="dynamic"`. `DashPackager::package` (and so `LlDashPackager::package`) now returns `Error::InvalidField` when `dynamic` is set and `publish_time` is `None`. `LlDashPackager` is always dynamic and does not default it: set `base.publish_time` and advance it on every updated MPD. Static MPDs are unchanged.

```rust
// before: a dynamic MPD was produced without publishTime
let mut p = DashPackager::new(/* .. */);
p.dynamic = true;
let mpd = p.package(&media)?;
// after
let mut p = DashPackager::new(/* .. */);
p.dynamic = true;
p.publish_time = Some(String::from("2026-11-07T12:00:00Z"));
let mpd = p.package(&media)?;
```

### Fix: PMT `version_number` (#1699, #1702)

`StreamingTsHlsSegmenter` (and the TS muxer's continuing path) increments the PMT `version_number` modulo 32 whenever the PMT content changes, so a track added with `add_track` mid-stream reaches a demuxer that joined earlier. An unchanged PMT keeps its version, and the first segment after `add_track` is flagged `#EXT-X-DISCONTINUITY` (RFC 8216bis section 4.4.4.3: track set changed). Callers that worked around this with a strict PMT order can drop the workaround.

### Fix: namespace-aware, bounded manifest parsing (#1708, #1712)

The DASH MPD and Smooth manifest readers resolve XML namespaces with `quick-xml`'s `NsReader`: a foreign-namespace `<ext:BaseURL>` no longer becomes the MPD base URL, a foreign `<ext:Period>` is no longer a Period, and a DASH default namespace under any prefix (`<d:MPD>`) parses. A root in a wrong namespace is `UnexpectedElement`; an undeclared prefix and content after the root are `Xml` errors. A manifest over 4 MiB, 32 levels deep, 100 000 elements, 8 KiB per attribute value or 256 attributes per element is the new `DashParseError::TooLarge` / `SmoothParseError::TooLarge`.

### Merged crate: `transmux::rtcp`

`transmux::rtcp` re-exports `rtp_packet::rtcp` (the former `rtcp-packet` crate); paths under `transmux::rtcp` are unchanged.

## Added

- `CodecConfig::name()`: one label per codec family; `flv` and `mkv_mux` unsupported-codec errors use it, so MPEG-H, MPEG-2 video and MPEG audio read `MPEG-H 3D Audio`, `MPEG-2 Video`, `MPEG-1/2 Audio` and a subtitle track `Subtitle` in both.
- `box_types::clamped_boxes` / `ClampedBox`: a tolerant top-level box walk that clamps a truncated tail to the buffer end; `frag_offsets::mdat_ranges` and the hls-runtime iframe helpers use it instead of three private copies.

## Fixed

- The DASH MPD and Smooth manifest readers resolve XML namespaces (`quick-xml` `NsReader`) and are bounded (#1708). An element or attribute is read only when it is in the parser's namespace: a foreign-namespace `<ext:BaseURL>` no longer becomes the MPD base URL, a foreign `<ext:Period>` is no longer a Period, and a foreign attribute with a DASH local name is ignored; a DASH default namespace under any prefix (`<d:MPD>`) parses. A root in a wrong namespace is `UnexpectedElement`, an undeclared prefix and content after the root are `Xml` errors, and a manifest over 4 MiB, 32 levels deep, 100 000 elements, 8 KiB per attribute value or 256 attributes per element is the new `DashParseError::TooLarge` / `SmoothParseError::TooLarge` (the `dash-runtime` ceilings; Smooth allows `CodecPrivateData` its own 64 Ki hex cap). A document whose root is unbound keeps parsing as before. `cenc:default_KID` is not read: the MPD model has no `ContentProtection`. transmux has no PlayReady WRMHEADER reader (it only writes one), so there was nothing to change there.
- `StreamingTsHlsSegmenter` (and the TS muxer's continuing path) increments the PMT `version_number` modulo 32 whenever the PMT content changes, so a track added with `add_track` mid-stream reaches a demuxer that joined earlier; an unchanged PMT keeps its version; the first segment after `add_track` is flagged `#EXT-X-DISCONTINUITY` (RFC 8216bis section 4.4.4.3: track set changed) (#1699).
- `WebmDemux` reports an `Info` whose `TimestampScale` is 0 or does not fit an `i64` as a malformed unit (the default scale then applies) instead of applying it: 0 collapsed every pts to 0 and values of 2^63 or more made pts negative.
- The `vvcC` writer no longer indexes the output buffer directly: a short buffer or a short `general_constraint_info` is a typed `BufferTooShort` instead of a panic.

## Other changes

- `transmux::rtcp` re-exports `rtp_packet::rtcp` (the former `rtcp-packet` crate, absorbed by `rtp-packet`); the `serde` feature forwards to `rtp-packet/serde`.
- The WebM `DefaultDuration` to IR-tick conversion and the progressive-MP4 interleave chunk length use `broadcast_common::timescale` (same rounding).
- The `stco`, `co64`, `stss` parse and all six flat FullBox table serializers (`stco`, `co64`, `stss`, `stts`, `ctts`, `elst`) go through one `parse_flat_table` / `serialize_flat_table`; `stts`/`ctts`/`elst` keep their own `parse_body` (they report a truncated entry as an error, the others a count check).
- The in-crate test bit writers of `slice_header` and `display_track`, and the `hdr10plus` integration test, use one `BitWriter` (`test_bits.rs`, `tests/it/support/bit_writer.rs`).
- `pipeline::build_trak` takes its sample entry, media kind and `tkhd` size from one per-codec function (`pipeline/stsd.rs`, `MediaKind` owning the handler and media-header boxes) instead of a 700-line tuple-returning `match`; the seven `init_segment` container boxes (`dinf`, `minf`, `mdia`, `edts`, `trak`, `mvex`, `moov`) serialize through one `serialize_container`.
- `TimelineOrigin`, the progressive-MP4 movie-timescale conversions, the LL-HLS ceil rescale and the tick comparisons use `broadcast_common::timescale`; a zero timescale counts as 1 uniformly.
- Timescale rescaling goes through `broadcast_common::timescale` with the rounding each site already used; a zero timescale counts as 1 instead of returning the input unchanged (RTP, repackage windows) or panicking.
- One `ADTS_HEADER_SIZE` constant shared by `aac_asc`, `ts_demux` and `sample_aes`.
- The four segmenters build `Stage::demand` with `Demand::saturated_if`.
- `ac3` and `dts` import `bitreader::read_bits_checked` instead of declaring forwarding `read_bits` wrappers.
- The `rtcp` module file is replaced by a re-export of `rtp_packet::rtcp`; paths unchanged.
- One `build_chunk` builds the CMAF segment, event segment, LL-DASH chunk and LL-HLS part; `trun` `data_offset` is now range-checked instead of cast.
- VVC reader/writer, AVS3-audio and HDR10+ serializers share `bitreader::{read_bits_checked, write_bits_checked}`; an AVS3-audio `dca3` write overrun is now `BufferTooShort` (was `InvalidInput`).
- `name()` of the AAC/ESDS label types returns `&'static str` (was `&str`; compatible for callers).

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6
- `cc-data` 0.7 -> 0.8
- `container-probe` 0.2 -> 0.3
- `dvb-si` 12 -> 13
- `mp4-emsg` 0.5 -> 0.6
- `mpeg-pes` 0.6 -> 0.7
- `mpeg-ps` 0.6 -> 0.7
- `mpeg-ts` 0.6 -> 0.7
- `rtp-packet` 0.5 -> 0.6

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `transmux-v0.30.0`.
