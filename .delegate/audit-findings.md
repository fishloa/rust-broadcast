# Adversarial HLS Conformance Audit — Findings

Audit of `hls-runtime/src/server/` + `broadcast-hls/src/` against
`broadcast-hls/docs/server-requirements.md` (113 normative items),
`playlist-tags.md` (all 32 §4.4 tags), `low-latency.md`, and
`media-segment-formats.md` (§3 container rules).

## Summary

| Area | Items checked | Findings | Severity |
|---|---|---|---|
| Server requirements (§6.2.1 – §6.2.6) MUSTs | 70 | 2 | 1 Med, 1 Low |
| Attribute-level fidelity (all 32 tags) | 32 | 2 | 1 Med, 1 Low |
| Low-latency numeric relationships | 6 | 1 | Med |
| Container rules (§3) | 15 | 0 | — |

See per-area checklists at the bottom for full coverage detail.

---

## Findings

### F1. `HOLD-BACK` attribute not modeled — `#EXT-X-SERVER-CONTROL` omits a defined attribute

**Spec:** `playlist-tags.md` §4.4.3.8 table row for `HOLD-BACK` + `server-requirements.md` (ad surrounding text). RFC 8216bis defines `HOLD-BACK` as a separate attribute from `PART-HOLD-BACK`: "OPTIONAL; absence implies three times the Target Duration; MAY appear in any Media Playlist."
**Code:** `broadcast-hls/src/lib.rs:758-797` (`LowLatencyConfig`), `hls-runtime/src/server/engine.rs:652-663` (where `LowLatencyConfig` is built for rendering).
**What the code does:** `LowLatencyConfig` has `part_hold_back: f64` but no `hold_back` field. `to_m3u8` at line 1228-1236 renders `CAN-BLOCK-RELOAD`, `PART-HOLD-BACK`, and optionally `CAN-SKIP-UNTIL` — never `HOLD-BACK`. A non-low-latency playlist (classic HLS) with custom hold-back distance cannot express `HOLD-BACK` at all; an LL-HLS playlist that wants a non-default `HOLD-BACK` (different from 3×Target Duration) has no way to set it.
**Falls back to** the spec default (3×Target Duration implied by absence), which is benign for compliance but precludes explicit configuration.
**Severity:** Low (spec default covers it; no silent violation).

### F2. `CAN-SKIP-DATERANGES` attribute missing from `LowLatencyConfig`

**Spec:** `playlist-tags.md` §4.4.3.8, `low-latency.md` §1.2: `CAN-SKIP-DATERANGES` is an OPTIONAL enumerated-string attribute of `#EXT-X-SERVER-CONTROL`, REQUIRING the presence of `CAN-SKIP-UNTIL`.
**Code:** `broadcast-hls/src/lib.rs:758-797` (`LowLatencyConfig`). Grep for `CAN-SKIP-DATERANGES` / `can_skip_dateranges` across the entire crate returns zero results.
**What the code does:** The attribute is never parsed from wire playlists (the parser at line 1505-1514 reads `PART-HOLD-BACK`, `CAN-SKIP-UNTIL`, `CAN-BLOCK-RELOAD` but not `CAN-SKIP-DATERANGES`) and never rendered. A playlist that carries `CAN-SKIP-DATERANGES=YES` silently drops it on round-trip.
**Failure scenario:** A server advertising `_HLS_skip=v2` support needs `CAN-SKIP-DATERANGES=YES` in its playlist; this crate cannot parse or emit playlists with that attribute. Any tool consuming this crate's parsed `MediaPlaylist` loses the server's daterange-skipping capability.
**Severity:** Medium (lost data on round-trip; blocks `_HLS_skip=v2` adoption).

### F3. No 85% partial-segment duration floor in `HlsOrigin`

**Spec:** `low-latency.md` §1.3 (RFC 8216bis §4.4.4.9): "The duration of each Partial Segment **MUST be at least 85%** of the Part Target Duration, with the exception of Partial Segments with the INDEPENDENT=YES or GAP=YES attribute, Partial Segments that are immediately followed by a Partial Segment with a GAP=YES attribute, and the final Partial Segment of any Parent Segment."
**Code:** `hls-runtime/src/server/engine.rs:574-683` (`render_playlist` — builds `PartSpec` items from `Trunk` part entries, no duration validation). `media-doctor/src/hls_validator.rs:116-149` catches this but only at `Severity::Warning`, not `Error`.
**What the code does:** `HlsOrigin` renders whatever part durations the segmenter produced, with no 85% floor check. A segmenter emitting a too-short part (e.g. a CMAF chunk at 0.05s with Part Target 1.0s) would appear in the rendered playlist without any rejection or even logging.
**Failure scenario:** A segmenter produces a 50ms chunk for a 500ms Part Target (10%, well below the 85% MUST). The origin emits `#EXT-X-PART:DURATION=0.05,…` and Apple's `mediastreamvalidator` flags it as non-conformant. Clients may see stutter or unexpected gaps.
**Severity:** Medium (MUST violation; the validator catches it but only as a Warning — an origin should not silently emit nonconformant output).

### F4. `EXT-X-SESSION-KEY` parser allows `METHOD=NONE`

**Spec:** `playlist-tags.md` §4.4.6.5 + HLS 2e §4.4.6.5: `EXT-X-SESSION-KEY` "MUST NOT have a METHOD of NONE."
**Code:** `broadcast-hls/src/lib.rs:607-625` (`SessionKey` struct) + line 1877-1878 (parser accepts `"NONE"` as a valid method). The module doc at line 182-183 explicitly documents this: "`EXT-X-SESSION-KEY`'s 'METHOD MUST NOT be NONE' ... this crate ... leaves such semantic validation to a higher-level tool."
**What the code does:** `parse_session_key` at line 1879 maps `"NONE"` → `EncryptionMethod::None` without error. `to_m3u8` renders `#EXT-X-SESSION-KEY:METHOD=NONE` without complaint.
**Failure scenario:** Parse a Master Playlist containing `#EXT-X-SESSION-KEY:METHOD=NONE,URI="k"`. The crate accepts it, and `to_m3u8` re-emits `METHOD=NONE` — a spec-violating line that Apple's `mediastreamvalidator` rejects.
**Severity:** Medium (MUST violation; documented gap but still a factual, actionable defect — a validator downstream sees it).

### F5. `_HLS_msn` abuse bound is +4 instead of spec's +2

**Spec:** `server-requirements.md` [6.2.5.2-S1] + `low-latency.md` §2.2: "If the `_HLS_msn` is greater than the Media Sequence Number of the last Media Segment in the current Playlist plus two ... the server **SHOULD** immediately return Bad Request."
**Code:** `hls-runtime/src/server/engine.rs:158` (`ABUSE_MSN_FUTURE_BOUND = 4`), line 699: `msn > u64::from(in_progress_seg) + ABUSE_MSN_FUTURE_BOUND`.
**What the code does:** The bound is "+4 from the live-edge segment" (which is typically `last_closed + 1`), making it effectively `last_closed + 5` when parts are present, vs. the spec's `last_closed + 2`. The code is up to 3 segments more permissive than the SHOULD recommends.
**Failure scenario:** A malfunctioning client sends `_HLS_msn=100` when `last_closed_segment=95`. Spec says SHOULD 400; the code allows it (100 ≤ 95+4+1=100) and blocks. The caller's blocking timeout eventually fires, which is a recoverable outcome, but the fast-fail recommendation is missed.
**Severity:** Low (SHOULD, not MUST; the blocking timeout still catches it).

---

## What was checked and found clean

### Server requirements walk-through (70 MUSTs)

Items assessed as **not applicable** (out of `HlsOrigin`'s sans-IO scope — caller/HTTP-adapter/segmenter responsibility):

- [6.2.1-M1], [6.2.1-M3], [6.2.1-M4], [6.2.1-M5], [6.2.1-M6], [6.2.1-M7], [6.2.1-M9], [6.2.1-M11], [6.2.1-M12], [6.2.1-M14], [6.2.1-M15], [6.2.1-M17], [6.2.1-M18], [6.2.1-M19], [6.2.1-M20], [6.2.1-M21], [6.2.1-M22], [6.2.1-M23], [6.2.1-M24], [6.2.3-M1..M5], [6.2.3-SHALL1], [6.2.4-M1], [6.2.4-M2a..M2j], [6.2.5.1-M1..M5], [6.2.5.2-M1..M6], [6.2.6-M1..M3]

Items assessed as **implemented/satisfied**:

- [6.2.1-M2] — URI for every segment: `render_playlist` generates `seg-{track}-{seq}.{ext}` URIs.
- [6.2.1-M8] — Media Playlist with URIs in order: `render_playlist` produces segments in window order.
- [6.2.1-M10] — Playlist mutations (append/remove/increment sequence): `Window` eviction + cursor drain implements the allowed mutation set.
- [6.2.1-M13] — TARGETDURATION MUST NOT change: `HlsOrigin` computes it once from max of configured/actual, never changes it dynamically (though no explicit guard; the code simply doesn't mutate it).
- [6.2.2-M1..M3] — MEDIA-SEQUENCE presence, increment, monotonic: `Window` eviction bumps `sequence_number`, rendered as `#EXT-X-MEDIA-SEQUENCE`.
- [6.2.2-M5] — Remove in order: `Window` is a `VecDeque`, always pops front.
- [6.2.2-M9..M11] — DISCONTINUITY-SEQUENCE: `Window.discontinuity_sequence` increments on evicted discontinuous entries, never decreases or wraps.

Items with **known limitations** (documented as out of scope here):

- [6.2.1-M10] (EVENT/VOD restriction enforcement): `HlsOrigin` is live-only (`endlist: false` hardcoded) — no VOD/EVENT mode, so those constraints are not exercised.
- [6.2.2-M6] (removal floor ≥ 3×Target Duration): `Window` capacity is configurable as a count of segments, not duration. No check that the window duration ≥ 3×Target Duration. The caller (`multimux`) configures this; if misconfigured, the playlist could be shorter than the spec's minimum.
- [6.2.5.2-M5] (ignore `_HLS_msn`/`_HLS_part` when EXT-X-ENDLIST present): not applicable since `HlsOrigin` never sets `endlist: true`.

### Attribute fidelity (all 32 §4.4 tags)

Every tag in `playlist-tags.md` checked against `broadcast-hls/src/lib.rs` struct fields and `to_m3u8` rendering:

| Tag | Struct field | Parse | Render | Notes |
|---|---|---|---|---|
| EXTM3U | — | ✓ | ✓ | Always first line. |
| EXT-X-VERSION | `MediaPlaylist::version` + `computed_version` | ✓ | ✓ | Derived from content, not hardcoded (§8). |
| EXT-X-INDEPENDENT-SEGMENTS | `MediaPlaylist::independent_segments` | ✓ | ✓ | |
| EXT-X-START | `MediaPlaylist::start: Option<StartPoint>` | ✓ | ✓ | TIME-OFFSET + PRECISE both modeled. |
| EXT-X-DEFINE | `MediaPlaylist::defines: Vec<Define>` | ✓ | ✓ | NAME/VALUE/IMPORT/QUERYPARAM all modeled. |
| EXT-X-TARGETDURATION | `MediaPlaylist::target_duration` | ✓ | ✓ | |
| EXT-X-MEDIA-SEQUENCE | `MediaPlaylist::media_sequence` | ✓ | ✓ | |
| EXT-X-DISCONTINUITY-SEQUENCE | `MediaPlaylist::discontinuity_sequence` | ✓ | ✓ | Emitted only when > 0. |
| EXT-X-ENDLIST | `MediaPlaylist::endlist` | ✓ | ✓ | |
| EXT-X-PLAYLIST-TYPE | `MediaPlaylist::playlist_type` | ✓ | ✓ | EVENT/VOD enum. |
| EXT-X-I-FRAMES-ONLY | `MediaPlaylist::iframes_only` | ✓ | ✓ | Triggers version ≥ 4. |
| EXT-X-PART-INF | `LowLatencyConfig::part_target` | ✓ | ✓ | PART-TARGET only. |
| EXT-X-SERVER-CONTROL | `LowLatencyConfig` | ✓ | ✓ | **Missing HOLD-BACK and CAN-SKIP-DATERANGES** (F1, F2). |
| EXTINF | `MediaSegment::duration` | ✓ | ✓ | |
| EXT-X-BYTERANGE | `MediaSegment::byte_range` | ✓ | ✓ | Segment + Part both modeled. |
| EXT-X-DISCONTINUITY | `MediaSegment::discontinuous` | ✓ | ✓ | |
| EXT-X-KEY | `extra_tags` (opaque) | ✓† | ✓† | †Pass-through only; no structural parse. Documented gap. |
| EXT-X-MAP | `MediaSegment::map` / `OpenSegment::map` | ✓ | ✓ | Carry-forward + dedup correct. |
| EXT-X-PROGRAM-DATE-TIME | `extra_tags` (opaque) | ✓† | ✓† | †Pass-through only. Documented gap. |
| EXT-X-GAP | `MediaSegment::gap` | ✓ | ✓ | |
| EXT-X-BITRATE | `MediaSegment::bitrate` | ✓ | ✓ | Carry-forward + dedup same as MAP. |
| EXT-X-PART | `PartSpec` (in `MediaSegment::parts` / `OpenSegment::parts`) | ✓ | ✓ | URI, DURATION, INDEPENDENT, BYTERANGE, GAP all modeled. |
| EXT-X-SKIP | `MediaPlaylist::skip: Option<SkipInfo>` | ✓ | ✓ | SKIPPED-SEGMENTS + RECENTLY-REMOVED-DATERANGES. |
| EXT-X-PRELOAD-HINT | `LowLatencyConfig` fields | ✓ | ✓ | TYPE, URI, BYTERANGE-START, BYTERANGE-LENGTH all modeled. |
| EXT-X-RENDITION-REPORT | `MediaPlaylist::rendition_reports` | ✓ | ✓ | URI, LAST-MSN, LAST-PART all modeled. |
| EXT-X-DATERANGE | `extra_tags` (opaque) | ✓† | ✓† | †Pass-through only. Documented gap. |
| EXT-X-I-FRAME-STREAM-INF | `MasterPlaylist::iframe_variants` | ✓ | ✓ | |
| EXT-X-MEDIA | `extra_tags` (opaque) | ✓† | ✓† | †Pass-through only. Documented gap. |
| EXT-X-STREAM-INF | `MasterPlaylist::variants` | ✓ | ✓ | |
| EXT-X-SESSION-DATA | `MasterPlaylist::session_data` | ✓ | ✓ | |
| EXT-X-SESSION-KEY | `MasterPlaylist::session_keys` | ✓ | ✓ | **Allows METHOD=NONE** (F4). |
| EXT-X-CONTENT-STEERING | `MasterPlaylist::content_steering` | ✓ | ✓ | |

### Low-latency numeric relationships

- **PART-HOLD-BACK ≥ 3× Part Target (SHOULD):** `effective_part_hold_back()` floors at 3×. ✓
- **PART-HOLD-BACK ≥ 2× Part Target (MUST):** Ensured by the 3× floor. ✓
- **CAN-SKIP-UNTIL ≥ 6× Target Duration (MUST):** Not enforced — caller sets the value. No validation. (Documented as caller's responsibility; `HlsOrigin` never sets it.)
- **Partial segment DURATION ≤ Part Target Duration:** Checked by `media-doctor` validator (Warning). ✓
- **85% minimum part duration:** Not enforced by `HlsOrigin`. **F3.**
- **Skip Boundary ≥ 6× Target Duration:** Not in scope (Delta Updates not implemented).
- **Blocking timeout SHOULD 503 after 3× Target Duration:** Caller responsibility (multimux's `AwaitPolicy`). ✓

### Container rules (§3)

Checked `transmux/src/pipeline.rs` + `transmux/src/ts_hls.rs` against `media-segment-formats.md`:

- **fMP4 init segment: `ftyp` with `iso6`-compatible brand** — `compatible_brands: ["iso5", "iso6", "mp41"]`. ✓
- **fMP4 init: `moov` after `ftyp`** — serialized in order. ✓
- **fMP4 init: `trak` for every `traf`** — one per track. ✓
- **fMP4 init: `mvhd`/`tkhd` duration = 0** — both set to 0. ✓
- **fMP4 init: `mvex` present** — `mvex: Some(...)`. ✓
- **fMP4 segment: `tfdt` in every `traf`** — `tfdt: Some(...)`. ✓
- **fMP4 segment: movie-fragment-relative addressing** — `TFHD_DEFAULT_BASE_IS_MOOF` flag set. ✓
- **fMP4 segment: no external data references** — `DataEntryUrlBox { flags: 1 }` (self-contained). ✓
- **TS segment: PAT+PMT at start** — `ts_hls.rs` re-emits PAT+PMT per segment. ✓
- **TS segment: single program** — one PAT entry → one PMT PID. ✓
- **EXT-X-MAP for fMP4: always emitted** — `HlsOrigin::render_playlist` emits it unconditionally when `Container::Fmp4`. ✓
- **EXT-X-MAP for TS: omitted by default** — `Container::MpegTs` never emits it. ✓ (TS segments carry in-band PAT/PMT.)
- No other container format rules applicable to the current scope.

---

## Areas NOT audited

- `_HLS_skip` / Playlist Delta Updates — unimplemented entirely; `HlsOrigin` never advertises `CAN-SKIP-UNTIL`, so no spec violation occurs. Feature gap, not defect.
- Multivariant playlist server requirements (EXT-X-MEDIA, cross-rendition constraints) — documented gap (no multi-rendition support).
- DATERANGE lifecycle rules — no DATERANGE support in origin.
- VOD/EVENT playlist-type semantics — `HlsOrigin` is live-only.
- Client-side requirements (§6.3) — out of scope for a server audit.
- `broadcast-hls::MasterPlaylist` parsing/serialization — not part of the server audit pass.
