# HLS I-frame playlists from the hls-runtime origin (issue #1413)

Status: design rev 2, 2026-10-22 (rev 1 reviewed against the code by an independent reader; 18 findings folded in).
Spec basis (page-verified transcriptions, committed first): [`broadcast-hls/docs/iframe-playlists.md`](../../broadcast-hls/docs/README.md#private-spec-transcriptions)
cited as **T§n**, and [`http-range-requests.md`](http-range-requests.md) (RFC 9110 range requests) cited as **H§n**.
Apple authoring 6.1: *I-frame playlists MUST be provided to support scrubbing and scanning UI.*

## 1. Decisions (the owner may overturn D1, D2, D6)

| # | Decision | Basis |
|---|---|---|
| D1 | I-frames come from the **normal content**; no dense dedicated rendition. | Apple 6.3 (T§5); dense renditions (6.2, SHOULD) need a second encode a repackager does not have. |
| D2 | Phase 1 = **fMP4/CMAF output only**. `Container::MpegTs` returns `NotFound` for the I-frame request (TS I-frames need IDR PES offsets: phase 2). | scope |
| D3 | One I-frame per source segment: the **first sample of the video track's fragment**. The video track is the first video track by `track_id` — the same rule `set_track_specs` already uses for `RESOLUTION`/`VIDEO-RANGE` (NOT the segmenter's positional anchor; they can differ). | consistency |
| D3′ | **No mid-stream skipping.** If, within the window, a segment *after the first included one* does not start with a sync sample of that track, or cannot be parsed, the I-frame playlist is **unavailable** (`NotFound`; master omits the variant) while that segment is in the window. Leading segments without a sync start (e.g. the stream's first) are simply not entries. | a skip would stretch the previous `EXTINF` to ~2× the segment duration and break `EXTINF ≤ TARGETDURATION` (RFC 8216bis §4.4.3.1, MUST) while the target duration is pinned to the media playlist's (Apple 6.11, live) |
| D4 | Entry = `EXT-X-BYTERANGE:<len>@0` into the **existing segment resource** (the URI the media playlist uses) + `EXT-X-MAP` to the init. No new media resources, no re-mux. | T§1, T§3, Apple 6.3 |
| D5 | `len` = end offset (from the start of the segment resource, which begins with `styp`) of that first video sample. **Emitted only if no other track's bytes precede it**: i.e. the sample's `start` equals the `mdat` payload start. The origin's segments hold ONE `moof` with one `traf` per track and an `mdat` that is each track's whole run in `traf` order (not interleaved), so when audio precedes video the range would contain the segment's whole audio run — such a segment makes the I-frame playlist unavailable (D3′ rule) rather than inflating the range and `BANDWIDTH`. | T§1 (range MUST NOT include init), Apple 6.10 (`moof` included), T§4 (`mdat` tail MAY be omitted) |
| D6 | Separate route/name `iframe-{track}.m3u8` (`{track}` is cosmetic: the origin serves ONE program's muxed segment stream; `DEFAULT_TRACK_ID`). New `HlsRequest::IFramePlaylist { track_id, query }` (additive, the enum is `#[non_exhaustive]`). Existing `master_playlist(media)` unchanged; new `master_playlist_with_iframes(media, iframe)`. | additive API |
| D7 | Audio-only origin: no I-frame variant; the I-frame request is `NotFound`. | Apple 6.1 concerns video |
| D8 | `EXTINF` of every entry = that segment's `duration_secs` (the origin's own segment duration, an anchor-track duration), **not** a presentation-time difference between I-frames. Consequences: across `EXT-X-DISCONTINUITY`, init changes and rolled-off gaps the definition stays well-formed; the approximation vs T§1's PTS wording holds when consecutive IDRs have equal composition offsets (usual; hardening option: use `composition_offset` from `FragmentSampleRange`). | simplicity + D3′ |

## 2. What exists (verified against `origin/main`)

* `broadcast_hls::MediaPlaylist.iframes_only` (renders `#EXT-X-I-FRAMES-ONLY`, version floor ≥4; with `EXT-X-MAP` ≥5, T§1/T§4);
  `MediaSegment { byte_range: Option<ByteRange{length, offset: Option<u64>}>, map: Option<MapTag>, .. }`;
  `IFrameVariant { bandwidth, codecs: Option<String>, resolution: Option<(u32,u32)>, uri, extra_attrs }` — **no** `average_bandwidth`
  (added by this work; breaking, rides the broadcast-hls 0.5 cascade; `VIDEO-RANGE` goes through `extra_attrs` as the origin already does for `Variant`).
* Origin: `Window` holds each in-window closed segment's whole `bytes` (`WindowSegment { sequence_number, init_gen, bytes, duration_secs, discontinuous, start_ns }`);
  `render_playlist` sets `iframes_only: false`; `HlsRequest` is `#[non_exhaustive]`; `set_track_specs` joins ALL tracks' codec strings into one `codecs` snapshot
  (so a **video-only codec string must be stored separately** for `CODECS` on the I-frame variant, Apple 9.3) and picks the first video track by `track_id`.
* Segment layout (`transmux` `build_chunk`, used by `build_media_segment` and `LlHlsSegmenter::finish_segment`): `styp` + ONE `moof` (a `traf` per track, `tracks` order)
  + ONE `mdat` (each track's whole-segment run, in `traf` order). Segments k≥2 start on the segmenter anchor's sync sample; the first segment is not guaranteed.
* `transmux::frag_offsets::sample_ranges(file, moof_off, &MovieFragmentBox, track_id) -> Vec<FragmentSampleRange{start,end,duration,flags,composition_offset}>`
  is `pub`: needs the whole segment bytes (not the init), the byte offset of the `moof` (after `styp`) and the parsed `MovieFragmentBox`; sync = flag bit `[16]` clear
  (absent flags default to 0 = sync — correct for origin output, which writes flags; externally supplied segments could be misread).
  `mdat_ranges`/`sample_ranges_in` avoid re-scanning.
* **No `Range` support anywhere in the HTTP origin** (`Range` appears only in the CORS allow-list, `multimux/src/origin/mod.rs`; no `206`). → prerequisite P0.

## 3. Behaviour

**Per segment, computed at drain, OUTSIDE the cursor/window locks** (parse moof, `sample_ranges` for the video track): sample 0 sync **and** `start == mdat payload start` → record `iframe_len = end` (offset 0); otherwise record "no I-frame" (D3′ decides leading-vs-mid). Never panic, never invent.
Bandwidth accounting (cumulative, survives eviction, like `peak_bandwidth_bps`/`total_bytes`): per entry bits = `iframe_len*8`, divided by its `EXTINF` → peak `BANDWIDTH`
(Apple 6.9: peak segment bit rate per the HLS spec) and `AVERAGE-BANDWIDTH`; same `u32` overflow check (`HlsMasterError::BandwidthOverflow`).

**I-frame media playlist.** `MediaPlaylist { iframes_only: true, target_duration: SAME as the media playlist (Apple 6.11), media_sequence: public MSN of the first entry,
discontinuity_sequence: window.discontinuity_sequence (the media playlist's value — or `EXT-X-DISCONTINUITY` counts drift on eviction), segments: [{ uri: segment uri,
byte_range: {iframe_len, offset 0}, duration: segment duration (D8), map: init of that segment's generation, discontinuous }], no parts, no open segment }` — version computed by `to_m3u8`.
Same window length/eviction as the media playlist. It may lag the media playlist's live edge (closed segments only; in LL mode a segment closes after its last part).
No `EXT-X-RENDITION-REPORT` (the media playlist emits none either; the RFC's SHOULD for I-frame reports is moot).
**Blocking reload:** if and only if the media playlist advertises `CAN-BLOCK-RELOAD`, the I-frame playlist carries a server-control-only `low_latency` config (no `PART-INF`,
no `PART-HOLD-BACK`; confirm `to_m3u8` accepts it) and `_HLS_msn` is honoured against closed segments (T§4 SHOULD); `_HLS_part` on the I-frame playlist → `BadRequest` (it has no parts).
Partial segments: none (T§4 SHOULD NOT).

**Master.** `EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=<peak>,AVERAGE-BANDWIDTH=<avg>,CODECS="<video codec only>",RESOLUTION=<presentation size, same as the variant>[,VIDEO-RANGE=..],URI="iframe-{track}.m3u8"`
— `CODECS`, `RESOLUTION`: MUST (Apple 9.3, 9.4); `BANDWIDTH`: MUST (T§2); **no `FRAME-RATE`** (not defined for this tag, T§2; no MUST NOT in the text, we simply do not emit it);
no `AUDIO`/`SUBTITLES`/`CLOSED-CAPTIONS`. The variant is omitted until an entry exists (never a placeholder number), so the master changes shape over time (same as the existing measured `BANDWIDTH`);
an init change resets the video codec/resolution snapshots exactly as `set_init` already resets `codecs`/`resolution`.

**P0 — HTTP range requests (prerequisite, in `multimux`'s origin).** A client fetches each entry with `Range: bytes=0-<len-1>`; the origin must answer `206 Partial Content`
+ `Content-Range: bytes 0-<len-1>/<total>` + `Accept-Ranges: bytes` on segment resources, `416` + `Content-Range: bytes */<total>` when unsatisfiable, and may answer `200` to multi-range
requests (H§ Range / 14.4 / 15.3.7 / 15.5.17). Use the typed `headers::Range` (already a dependency; no hand-parsing) and `Range`-aware responses, with the CORS allow-list already
exposing `Range`. Applies to every segment-like resource of the origin (init too is harmless).

**Non-goals.** Dense 1-fps renditions (D1); TS-HLS (D2); AES-128 whole-resource encryption (the origin emits no `EXT-X-KEY`); more than one video track per origin; DVR/catch-up
playlists (archived segments are not in the `Window`, so `iframe_len` is unavailable there: the catch-up/DVR playlists get no I-frame variant in phase 1).

## 4. Verification plan (the gate, not the claim)

1. **Geometry oracle.** For every entry on real fixtures: `sample_ranges` over the FULL segment → sample 0 of the video track is sync, `start == mdat payload start`, `len == end`; the served range bytes
   equal `segment[0..len)`; and the I-frame's bytes `[start..end)` equal the demuxed first video sample (`Fmp4Demux` over the FULL segment). (A truncated range is NOT fed to `Fmp4Demux`:
   its `moof` describes more samples than the truncated `mdat` holds, so the demuxer correctly errors — the rev-1 "demux the slice" oracle was wrong.)
2. **Apple `mediastreamvalidator`** over master + media + I-frame playlist + segments: zero errors (this is the check that the truncated-`mdat` ranges are accepted; no player is available here).
   Harness bites: a wrong `len`, a wrong `BANDWIDTH`, a dropped `EXT-X-I-FRAMES-ONLY` are each rejected — find which the validator rejects and assert exactly those.
   **Contingency if the validator (or AVPlayer, reported by the owner) rejects truncated-`mdat` ranges:** serve a synthesized per-I-frame fragment (`moof`+`mdat` holding only that sample) as a new resource
   instead of ranges into the segment — changes D4; decided only on evidence.
3. HTTP-level test (P0): a real `GET` with `Range` on a segment returns the exact bytes with correct `Content-Range`/`206`; 416 and multi-range paths; header-only (`HEAD`) unaffected.
4. Unit: leading-skip vs mid-stream D3′ → unavailable; audio-first segment → unavailable; audio-only → none; eviction parity incl. `discontinuity_sequence`; `BANDWIDTH` equals the independently computed
   peak of `iframe_len*8/EXTINF`; `_HLS_part` → BadRequest; blocking reload parity; TS origin → NotFound.
5. Round-trip the rendered playlists through `broadcast_hls` (`iframes_only`, `byte_range`, `IFrameVariant`).
6. Mutations that must each fail a named test: off-by-one `len`, drop `iframes_only`, wrong `EXTINF`, include `FRAME-RATE`, wrong target duration, ignore the video-first check, drop `discontinuity_sequence`.

## 5. Phases (each: tests/oracle first, gates run by the orchestrator, mutation checks, independent review)

P0 `multimux` origin: `Range`/`206`/`416` on segment resources (+ the RFC 9110 transcription, already committed).
P1 `broadcast-hls`: `IFrameVariant.average_bandwidth` (+ round-trip tests).
P2 `hls-runtime`: per-segment I-frame record, I-frame playlist, `HlsRequest::IFramePlaylist`, master variant, video-only codec store.
P3 `multimux`: route + master wiring for the CMAF-HLS/LL-HLS outputs. P4 oracle/e2e with `mediastreamvalidator`.

## 6. Open owner questions (the defaults above apply until answered)

* D1: normal-content I-frames only (proposed) vs also a caller-supplied dense I-frame track.
* D2: TS-HLS I-frames wanted (phase 2)?
* D6: route/name scheme.
* D5/D3′: segments with audio before video, or non-sync starts, yield *no* I-frame playlist rather than a degraded one — acceptable, or should the origin reorder `traf`/`mdat` so video leads (changes segment bytes/goldens)?
