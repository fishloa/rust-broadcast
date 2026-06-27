# timed-metadata — design

**Status:** approved design, pre-implementation
**Date:** 2026-06-27
**Crate:** `timed-metadata` (crates.io name confirmed free)
**Versioning:** independent, starts `0.1.0` (like `scte104`); NOT in the DVB lockstep.
**Posture:** `#![no_std]` + `alloc`; `serde` + `chrono` feature-gated; must build `--no-default-features`.

## Purpose

A spec-cited **conversion core** that translates Digital Program Insertion / timed-metadata
signalling between the formats used across broadcast and OTT delivery. It is the first crate of
the project's broadening from "DVB parsers" to "broadcast-grade media signalling, pure Rust."

It does **not** assemble HLS playlists or DASH manifests (that is the caller's job, via
`m3u8-rs` / their MPD writer or a future dedicated crate). It owns the *translation* and the
small self-contained text/box outputs that have no other home.

## Scope

### v0.1 — core 3 edges
- `SCTE-35 → HLS EXT-X-DATERANGE` (media-time → wall-clock; requires a time anchor)
- `SCTE-35 → DASH emsg` (media-time only; no anchor)
- `DASH emsg → SCTE-35` (reverse, for the SCTE-35 carriage scheme)

### Deferred
- v0.2: `SCTE-104 → SCTE-35` ingest (reuses `scte104`)
- v0.3: ID3 timed-metadata emit (new ID3v2 framing surface, no existing crate to lean on)
- Future: additional spokes (WebVTT cues, etc.) — one adapter each, by design.

## Architecture

### Boundary (decided: "conversion core only")
Consumes already-parsed source types and emits structured outputs plus the minimal text/box
serialization:
- **in:** `scte35_splice::SpliceInfoSection`, `mp4_emsg::EventMessageBox`
- **out:** a `DateRange` struct (+ `.to_tag_line()` → the `#EXT-X-DATERANGE:` line), an owned
  emsg box (reusing `mp4-emsg`)

No heavyweight playlist/MPD dependency. Zero overlap with `m3u8-rs` / `dash-mpd`.

### Hub-and-spoke (decided)
One canonical `TimedEvent` in the middle; each format gets `from_*` / `to_*` adapters.
N formats → 2N adapters; a new format is one new adapter, never N². The hub is **lossless by
construction**: it carries the original payload bytes verbatim (see below), so format-specific
bits are never dropped.

**Key spec insight that makes the hub lossless:** both target carriages *embed the entire
`splice_info_section` verbatim* —
- HLS `EXT-X-DATERANGE` `SCTE35-OUT`/`IN`/`CMD` attribute value = the whole splice section, hex.
- DASH `emsg` with scheme `urn:scte:scte35:2013:bin` carries the whole splice section as
  `message_data`.

So conversion = (a) parse SCTE-35 to **extract** the timing/id/duration/kind that drive the
target's typed fields, and (b) carry the **raw splice bytes** through unchanged. The metadata
drives the fields; the bytes guarantee fidelity.

### Two API layers (decided: ship both in v0.1)

**Layer 1 — pure conversions** (the foundation; testable, `no_std`, what audits bite on):
```rust
// media-time only — no anchor needed:
pub fn scte35_to_emsg(splice: &SpliceInfoSection, cfg: &EmsgConfig) -> Result<OwnedEmsg>;
pub fn emsg_to_scte35(b: &EventMessageBox) -> Result<Vec<u8>>; // SCTE-35-scheme emsg only

// wall-clock crossing — anchor REQUIRED by the type signature:
pub fn scte35_to_daterange(splice: &SpliceInfoSection, anchor: &TimeAnchor) -> Result<DateRange>;
```
The anchor is a *required argument* only on conversions that cross into wall-clock, so a UTC
time can never be fabricated without the caller supplying the mapping.

**Layer 2 — stateful `Timeline` session** (the headline API the packager calls):
```rust
pub struct Timeline { /* anchor, last_pts, wrap epoch */ }
impl Timeline {
    pub fn new() -> Self;
    pub fn set_anchor(&mut self, anchor: TimeAnchor);
    pub fn push_scte35(&mut self, bytes: &[u8]) -> Result<TimedEvent>; // parse + unroll wrap + store
    pub fn to_daterange(&self, ev: &TimedEvent) -> Result<DateRange>;  // errors if no anchor set
    pub fn to_emsg(&self, ev: &TimedEvent, cfg: &EmsgConfig) -> Result<OwnedEmsg>;
}
```
The session holds the anchor and unrolls the 33-bit PTS (≈26.5 h) across a stream; internally it
calls Layer 1. README example uses Layer 2; the test suite hammers Layer 1.

## Data model

```rust
pub struct TimedEvent {
    pub id: Option<u32>,           // SCTE-35 splice_event_id / emsg id
    pub kind: EventKind,           // abstracted break/ad/chapter semantics
    pub at: Option<MediaTime>,     // 90 kHz PTS, wrap-unrolled; None = immediate/now
    pub duration: Option<MediaDuration>,
    pub source: SourcePayload,     // lossless verbatim original
}

pub enum EventKind { BreakStart, BreakEnd, ProviderAdStart, ProviderAdEnd, Chapter, Unspecified, /* … */ }
// name() + impl_spec_display!(EventKind)  — per the #204 label convention

pub enum SourcePayload {
    Scte35 { raw: Vec<u8> },
    Emsg   { scheme_id_uri: String, value: String, raw: Vec<u8> },
}

pub struct MediaTime(u64);      // 90 kHz ticks, wrap-unrolled
pub struct MediaDuration(u64);  // 90 kHz ticks

pub struct TimeAnchor {         // media-time ↔ wall-clock mapping (caller-supplied)
    pub pts_90k: u64,           // a known PTS …
    pub utc_epoch_ms: i64,      // … and the UTC it corresponds to
}
```

`EventKind` is derived from the SCTE-35 command (`splice_insert.out_of_network_indicator`) and,
when present, the `segmentation_descriptor.segmentation_type_id`.

### DateRange output
```rust
pub struct DateRange {
    pub id: String,
    pub start_date: String,           // ISO-8601 / RFC3339
    pub class: Option<String>,
    pub duration: Option<f64>,        // seconds
    pub planned_duration: Option<f64>,
    pub scte35: Option<Scte35Attr>,   // OUT | IN | CMD + raw bytes (emitted as hex)
    // (extensible, #[non_exhaustive])
}
impl DateRange {
    pub fn to_tag_line(&self) -> String;          // → "#EXT-X-DATERANGE:..."
    pub fn parse_tag_line(s: &str) -> Result<Self>; // for round-trip discipline
}
```

## Time handling
- **Wrap:** 33-bit PTS unroll in the `Timeline` session — on a backward jump beyond a threshold,
  increment the epoch; absolute = `(epoch << 33) | pts`.
- **Wall-clock:** `MediaTime` → UTC via `TimeAnchor` (linear at 90 kHz). RFC3339 string for
  `START-DATE` produced behind the `chrono` feature; a small self-contained RFC3339 formatter
  from `utc_epoch_ms` provides the `no_std`/no-`chrono` path so the core builds without `chrono`.
- **Durations:** SCTE-35 `break_duration` (90 kHz) → DATERANGE `DURATION` seconds (`/90000.0`);
  → emsg `event_duration` at the emsg `timescale`.

## Error handling
`thiserror`, structured and contextual:
- `MissingAnchor` — a wall-clock conversion attempted with no anchor set
- `UnsupportedScheme { scheme }` — `emsg_to_scte35` on a non-SCTE-35 emsg
- `Scte35(scte35_splice::Error)`, `Emsg(mp4_emsg::Error)` — source parse failures
- `AttrEncode` / `AttrParse` — DATERANGE tag (de)serialization
- `OutputBufferTooSmall` — emsg serialize

## Dependencies
- `scte35-splice` (renamed from `dvb-scte35`) — SCTE-35 parse/serialize
- `mp4-emsg` (renamed from `dvb-emsg`) — emsg box
- `dvb-common` — shared traits / error idioms
- `chrono` (optional, default-on), `serde` (optional, default-on)

> **Sequencing (decided):** land the two dependency renames **before** releasing timed-metadata —
> `dvb-scte35`→`scte35-splice` and `dvb-emsg`→`mp4-emsg` ship first (a partial Wave-1: publish the
> new-named crates + re-export shims on the old names, don't yank). timed-metadata then depends on
> `scte35-splice`/`mp4-emsg` from day one — no dep-name churn after release.

## Testing (the gate — must bite)
- Pure-conversion unit tests for each edge.
- **Round-trip:** `DateRange::parse_tag_line(to_tag_line()) == self` byte-stable; emsg round-trips
  via `mp4-emsg`.
- **Real fixture (ungameable):** reuse the existing real SCTE-35 vectors in
  `dvb-scte35/tests/` (`known_vectors.rs`, `downloaded_scte35.rs`, `spec_samples.rs`). Convert a
  real splice → DATERANGE and → emsg; assert (a) the extracted timing/duration/id match the spec
  decode, and (b) the **raw splice bytes survive verbatim** in the SCTE35-OUT hex and the emsg
  `message_data` (lossless proof — defeats a passthrough-only fake).
- **Lossless round-trip:** SCTE-35 → emsg → SCTE-35 returns the original splice bytes.
- `label_coverage.rs` drift-guard for `EventKind` (+ any new public enum).
- `--no-default-features` (`no_std`) build gate.
- Every conversion module cites its mapping spec in the `//!` doc:
  HLS bis §4.4.5.1 (DATERANGE SCTE-35 attrs), SCTE 214-3 (SCTE-35-in-DASH scheme).

## Fixtures (gathered + verified 2026-06-27)

No new fixtures needed for the emsg edges; one real paired fixture gathered for DATERANGE.

**emsg edges — reuse existing real bins (copy into `timed-metadata/tests/fixtures/`):**
- `dvb-emsg/tests/fixtures/scte35_emsg_v0.bin` (real v0 SCTE-35-scheme emsg)
- `dvb-emsg/tests/fixtures/emsg_v1_scte35_livesim.bin` (real v1, DASH-IF livesim2)
- Enables real interop round-trip: emsg → SCTE-35 → emsg, byte-identical.

**SCTE-35 input — reuse existing real vectors:** `dvb-scte35/tests/` (threefive corpus,
real Russian mux).

**DATERANGE edge — real paired fixtures (verified against the repo's own parser):**
Source: Unified Streaming docs (a real packager). Each line's `SCTE35-OUT` hex IS the splice
input; `START-DATE`/`PLANNED-DURATION` are the golden output.

```
#EXT-X-DATERANGE:ID="2002",START-DATE="2018-10-29T10:38:00Z",PLANNED-DURATION=24,SCTE35-OUT=0xFC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D
#EXT-X-DATERANGE:ID="2004",START-DATE="2018-10-29T10:42:00Z",PLANNED-DURATION=24,SCTE35-OUT=0xFC302100000000000000FFF01005000007D47FEF7F7E0020F580C000000000004F1B1A5F
```
Both parse clean: `SpliceInsert`, `out_of_network_indicator=true`, `break_duration=2160000`
(90 kHz) = **24.0 s**, matching `PLANNED-DURATION=24`. This is the real-world assertion the
duration edge must reproduce (`2160000 / 90000 == 24.0`).

> A third docs example (`ID="72617"`) was **discarded** — `CrcMismatch` (bad transcription).
> The CRC check is exactly why fixtures are parser-verified before use.

**Conversion-model note exposed by the fixtures:** these splices have `splice_time.pts_time =
None` (program/immediate splice). So `START-DATE` does **not** come from the splice PTS — it
comes from the insertion point (segment boundary / `EXT-X-PROGRAM-DATE-TIME`). Confirms the
model: `TimedEvent.at = None` ⇒ caller/anchor supplies the wall-clock position; the crate must
not try to derive `START-DATE` from a missing `pts_time`.

## Specs to vendor
- **draft-pantos-hls-rfc8216bis** + **RFC 8216** — IETF, redistributable; vendor text into `specs/`.
- **SCTE 214-3** — SCTE register-wall; transcribe the mapping tables into the crate's `docs/`
  (per project standard); do not commit the raw PDF if terms are unclear.
- **DASH-IF "Event & Timed Metadata Processing"** — free PDF, cite for cross-check.
- ISO/IEC 23009-1 (core DASH) — **paid, NOT vendored**; not needed (emsg box already owned).

## Out of scope (v0.1)
Playlist/manifest assembly; SCTE-104 ingest; ID3; WebVTT; any container muxing.
