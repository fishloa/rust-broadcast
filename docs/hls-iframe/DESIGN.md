# HLS I-frame playlists from the hls-runtime origin (issue #1413)

Status: design, 2026-10-22. Spec basis (page-verified transcription, committed first):
[`broadcast-hls/docs/iframe-playlists.md`](../../broadcast-hls/docs/iframe-playlists.md) — cited below as **T§n**.
Apple authoring item 6.1: *I-frame playlists MUST be provided to support scrubbing and scanning UI.*

## 1. Decisions (and the two the owner may overturn)

| # | Decision | Basis |
|---|---|---|
| D1 | I-frames come from the **normal content** (no dense dedicated rendition). | Apple 6.3 allows it (T§5); dense renditions (6.2, SHOULD) need a second encode, which a repackager does not have. **Owner may overturn.** |
| D2 | Phase 1 = **fMP4/CMAF output only** (`Container::Fmp4`). TS-HLS (`Container::MpegTs`) I-frames need IDR PES offsets inside `.ts` and are phase 2. | scope |
| D3 | One I-frame per source segment: the **first sample** of the video track's fragment (segments start at a sync sample). A segment whose first video sample is not a sync sample contributes no entry; the previous entry's `EXTINF` then spans it. | T§1 (`EXTINF` = distance to the next I-frame **in the playlist**) |
| D4 | Entry = `EXT-X-BYTERANGE:<len>@0` into the **existing segment resource** (same URI the media playlist uses) + `EXT-X-MAP` to the init. No new media resources, no re-mux. | T§1, T§3, Apple 6.3 |
| D5 | `len` = end offset of that first sample counted from the start of the segment resource, i.e. it covers `moof` + `mdat` header + the I-frame sample (+ any bytes of other tracks' samples that precede it in the `mdat`). It never includes the init segment. | T§1 ("MUST NOT include its Media Initialization Section"), Apple 6.10 (`moof` MUST be included), T§4 (fMP4 segment MAY omit the `mdat` tail) |
| D6 | The I-frame playlist is a **separate route/name**, `iframe-{track}.m3u8`, served by the same origin; the existing `master_playlist(media)` is unchanged; new `master_playlist_with_iframes(media, iframe)` adds the `EXT-X-I-FRAME-STREAM-INF`. | additive API; **URL scheme is the owner's call** |
| D7 | Audio-only origin: no I-frame playlist, no I-frame tag, I-frame request = `NotFound`. | Apple 6.1 concerns video |

## 2. What exists (verified against the code, 2026-10-22)

* `broadcast_hls::MediaPlaylist` already has `iframes_only: bool` (renders `#EXT-X-I-FRAMES-ONLY`, forces the version floor);
  `MediaSegment` has `byte_range: Option<ByteRange>`, `map: Option<MapTag>`; `IFrameVariant` renders/parses
  `EXT-X-I-FRAME-STREAM-INF`. The origin sets `iframes_only: false` today (`hls-runtime/src/server/engine.rs`, `render_playlist`).
* The origin's `Window` holds every in-window closed segment's bytes (`WindowSegment { bytes, duration_secs, init_gen, discontinuous, .. }`),
  so the I-frame range is computable from data it already has.
* `transmux::frag_offsets::sample_ranges` resolves `moof`/`trun` sample byte ranges + flags (the single addressing function `Fmp4Demux`
  and the CENC decryptor share). A sample is sync when `trun`/default flag bit `[16]` is clear.
* `HlsRequest` is `#[non_exhaustive]`: a new `IFramePlaylist { track_id, query }` variant is additive.

## 3. Behaviour

**Per segment (at drain, when it enters the Window).** Parse the segment with `sample_ranges` for the video track's fragment; take sample 0:
sync → `iframe_len = sample0.end` (offset 0); not sync / unparsable → no I-frame record (never panic, never invent). Keep `iframe_len`
beside `bytes`. Bandwidth accounting (cumulative, survives eviction, like `peak_bandwidth_bps`/`total_bytes`): per I-frame entry
`bits = iframe_len*8`, `EXTINF` = distance to the next I-frame entry → peak `BANDWIDTH` (Apple 6.9: peak segment bit rate per the HLS
spec) and `AVERAGE-BANDWIDTH` over all entries.

**I-frame media playlist.** `MediaPlaylist { iframes_only: true, target_duration: SAME value as the media playlist (Apple 6.11, live),
media_sequence: public MSN of the first included segment, segments: [{ uri: <segment uri>, byte_range: {len, offset 0}, duration: EXTINF
per D3, map: init of that segment's generation, discontinuous }], no parts, no open segment, no LL-HLS directives }` — version computed by
`to_m3u8` (≥5 with `EXT-X-MAP`, T§4). Same window length and eviction as the media playlist (an I-frame entry leaves when its segment does).
Blocking reload (`_HLS_msn`) is honoured through the same machinery as the media playlist (T§4: SHOULD when other renditions can block).
Partial segments: none (T§4 SHOULD NOT). `EXT-X-DATERANGE` and `EXT-X-DISCONTINUITY` follow the segments they annotate, as in the media playlist.

**Master.** `EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=<peak>,AVERAGE-BANDWIDTH=<avg>,CODECS="<video codec only>",RESOLUTION=<presentation size, as the
variant>[,VIDEO-RANGE=..],URI="iframe-{track}.m3u8"` — `CODECS` and `RESOLUTION` are MUST (Apple 9.3, 9.4); `BANDWIDTH` MUST (T§2); **no `FRAME-RATE`** —
the attribute is not defined for this tag (T§2); no `AUDIO`/`SUBTITLES`/`CLOSED-CAPTIONS` (T§2). Before the first I-frame is measured the variant is
omitted (never a placeholder number). `IFrameVariant` gains `average_bandwidth` (broadcast-hls breaking, already the 0.5 cascade).

**Out of scope / explicit non-goals.** Dense 1-fps renditions (D1); TS-HLS (D2); AES-128 whole-resource encryption (T§4: the whole resource must be
encrypted with block restarts — this origin does not emit METHOD=AES-128); multiple video tracks per origin (first video track, as `RESOLUTION`).

## 4. Verification plan (the gate, not the claim)

1. **Byte-range read-back (ungameable oracle).** For every I-frame entry, slice the served segment bytes `[offset, offset+len)`, prepend the init,
   demux with `Fmp4Demux`: exactly **one** sample, `is_sync`, and its bytes equal the segment's first video sample. Run on real fixtures
   (`hls-runtime/tests/fixtures/{init.mp4,index*.m4s}` and a real HEVC/H.264 CMAF with several GOPs).
2. **Apple `mediastreamvalidator`** over master + media + I-frame playlist + segments: zero errors; harness bites (a wrong `len`, a wrong `BANDWIDTH`,
   missing `EXT-X-I-FRAMES-ONLY` each rejected — find which the validator rejects and assert exactly those).
3. Parser round-trip of the rendered playlists through `broadcast_hls` (`iframes_only`, `byte_range`, `IFrameVariant`).
4. Unit: `EXTINF` accumulation across a segment without a leading sync sample (D3); audio-only → none; eviction parity; blocking reload parity;
   `BANDWIDTH` equals the independently computed peak of `iframe_len*8/EXTINF`.
5. multimux: route + master wiring, an end-to-end HTTP test serving all three, validated by the oracle.
6. Mutations that must each fail a named test: off-by-one in `len`, drop `iframes_only`, wrong `EXTINF` accumulation, include `FRAME-RATE`, wrong target duration.

## 5. Phases

P1 `broadcast-hls`: `IFrameVariant.average_bandwidth` (+ round-trip tests). P2 `hls-runtime`: per-segment I-frame record + playlist + master + `HlsRequest`.
P3 `multimux`: route + master. P4 oracle/e2e. Each phase: tests first against the oracle above, gates run by the orchestrator, mutation checks.

## 6. Open owner questions (defaults above apply until answered)

* D1: normal-content I-frames only (proposed) vs also supporting a caller-supplied dense I-frame track (needs a second ingest track per program).
* D6: route/name `iframe-{track}.m3u8` vs another scheme.
* D2: is TS-HLS I-frame support wanted (phase 2), given it needs per-segment IDR PES offsets?
