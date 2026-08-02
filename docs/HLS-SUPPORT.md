# HLS support

What this workspace implements of HTTP Live Streaming, and — equally important — what it
does not.

**Spec revision: `draft-pantos-hls-rfc8216bis-22`, 1 May 2026.** Confirmed current against the
IETF datatracker. It obsoletes RFC 8216, and both are vendored under `specs/`. The syntax
tables, version-compatibility matrix, server requirements, low-latency mechanism and segment
formats are transcribed into [`broadcast-hls/docs/`](../broadcast-hls/docs/) — those
transcriptions, not the raw draft, are what the implementation is written against.

## Crate layout

| crate | role |
|---|---|
| [`broadcast-hls`](../broadcast-hls/) | **M3U8 syntax** — parse + serialize, `EXT-X-VERSION` derivation. `no_std` + `alloc`, builds for `thumbv7em-none-eabi`. Depends only on `broadcast-common` |
| [`hls-runtime`](../hls-runtime/) | **protocol** — origin (over `media-plane`) and client. Depends on `broadcast-hls` |
| [`transmux`](../transmux/) | **segmenters** — produces the fMP4/CMAF and MPEG-TS segments a playlist describes |
| [`media-doctor`](../media-doctor/) | **validation** — structural checks plus the Apple `mediastreamvalidator` oracle |

Syntax is a separate crate from the runtime because four things parse M3U8 — `transmux`,
`media-doctor`, `hls-runtime` and the fuzz sub-workspace — and only one of them wants an origin
engine. Folding syntax into `hls-runtime` is not merely undesirable but *impossible*: the origin
is `Trunk`-backed, so `transmux → hls-runtime → media-plane → transmux` is a dependency cycle
the moment the runtime feature is enabled.

## Playlist tags — 32 of 32

Every tag defined in §4.4 parses **and** serializes, with typed values (enums and structured
attribute lists, never stringly-typed), `#[non_exhaustive]` public types, and a round-trip test.
`broadcast-hls/tests/hls_tag_completeness.rs` enumerates all 32 by name and fails if one becomes
unhandled, so a future spec revision surfaces as a red test rather than silence.

**Basic** (§4.4.1) — `EXTM3U`, `EXT-X-VERSION`

**Media or Multivariant** (§4.4.2) — `EXT-X-INDEPENDENT-SEGMENTS`, `EXT-X-START`, `EXT-X-DEFINE`

**Media Playlist** (§4.4.3) — `EXT-X-TARGETDURATION`, `EXT-X-MEDIA-SEQUENCE`,
`EXT-X-DISCONTINUITY-SEQUENCE`, `EXT-X-ENDLIST`, `EXT-X-PLAYLIST-TYPE`, `EXT-X-I-FRAMES-ONLY`,
`EXT-X-PART-INF`, `EXT-X-SERVER-CONTROL`

**Media Segment** (§4.4.4) — `EXTINF`, `EXT-X-BYTERANGE`, `EXT-X-DISCONTINUITY`, `EXT-X-KEY`,
`EXT-X-MAP`, `EXT-X-PROGRAM-DATE-TIME`, `EXT-X-GAP`, `EXT-X-BITRATE`, `EXT-X-PART`

**Media Metadata** (§4.4.5) — `EXT-X-DATERANGE` (including the §4.4.5.1.1 SCTE-35 mapping),
`EXT-X-SKIP`, `EXT-X-PRELOAD-HINT`, `EXT-X-RENDITION-REPORT`

**Multivariant** (§4.4.6) — `EXT-X-MEDIA`, `EXT-X-STREAM-INF`, `EXT-X-I-FRAME-STREAM-INF`,
`EXT-X-SESSION-DATA`, `EXT-X-SESSION-KEY`, `EXT-X-CONTENT-STEERING`

## `EXT-X-VERSION` — derived, not chosen

All 13 §8 rows are implemented. The rendered version is `max()` over the minimums actually
triggered by the playlist's contents; a playlist triggering nothing emits **no**
`EXT-X-VERSION` tag, per §8's opening rule.

This replaced a hard-coded `9` that over-declared every playlist. §7 says a client *MUST NOT*
attempt playback if it does not support the declared version, so over-declaring told every
client on 6, 7 or 8 to refuse a stream it could play.

**Cross-checked against the spec authors.** `broadcast-hls/tests/spec_fixture_version.rs` parses
the RFC's own §9 example playlists and compares the derived version with the one the authors
declared:

| fixture | declared | derived |
|---|---|---|
| 9.1 simple media playlist | 3 | 3 |
| 9.2 live media playlist | 3 | 3 |
| 9.3 encrypted media segments | 3 | 3 |
| 9.4 – 9.7, 9.12 multivariant | *(untagged)* | *(none)* |

That is the one check in the suite not measuring our code against our own reading of the spec.

## Origin (`hls-runtime::server`)

`HlsOrigin` serves a `media_plane::Trunk` with an **advertised == servable** guarantee: every
URI a rendered playlist names can be fetched from the same origin.

| | classic | low-latency |
|---|---|---|
| **fMP4 / CMAF** | `EXT-X-MAP`, `.m4s` | + `EXT-X-PART` `.m4s` |
| **MPEG-TS** | no map, `.ts` | + `EXT-X-PART` `.ts` |

Container and latency mode are orthogonal — all four combinations are supported and tested.
Low latency is opt-in via the builder; omitting it yields a classic playlist with no LL-HLS
tags at all.

Also implemented: blocking playlist reload (`_HLS_msn`/`_HLS_part`), part availability,
preload hints, and rendition reports.

### On `EXT-X-MAP` under MPEG-TS

§3.1.1 makes this a **disjunction**, not a container restriction:

> Each Transport Stream Segment MUST contain a PAT and a PMT, **or** have an EXT-X-MAP tag
> applied to it.

`transmux`'s TS segmenter re-emits PAT+PMT at the head of every segment, so our TS playlists
correctly omit the tag. `MpegTs` mode omits it **by default without forbidding it**, because a
consumer feeding pre-segmented TS that lacks in-band PSI genuinely needs it.

fMP4 is different and unconditional (§3.1.2): every fMP4 segment MUST have `EXT-X-MAP` applied.

## Client (`hls-runtime::client`)

Sans-IO, caller-driven: blocking-reload scheduling, part prefetch, discontinuity handling, and
demux to access units. Handles both fMP4 (via `Fmp4Demux`) and MPEG-TS (via `TsDemux`,
content-sniffed), including classic playlists with no `EXT-X-MAP` at all. An optional
`tokio`+`reqwest` adapter is provided.

## Validation

Three layers, deliberately independent:

1. **Round-trip** — every committed fixture parses, serializes and re-parses to an equal
   document.
2. **`media-doctor`** — structural rules over HLS and DASH manifests.
3. **Apple `mediastreamvalidator`** — the reference implementation's own conformance tool.

The third exists because the first two share an author with the renderer, so a *shared
misreading of the spec passes twice*. On its first run it found that CMAF-HLS output emitted
**zero `EXT-X-MAP` tags** — an unconditional §3.1.2 MUST that `media-doctor` had passed every
time. It is calibrated against the RFC's own §9 playlists first: if it rejects something the
spec authors wrote, the invocation is wrong, not the fixture.

macOS-only, so it is non-blocking in CI and documented for local runs.

## Fixtures

19 playlists under `fixtures/hls/`, each with provenance in `MANIFEST.md`:

- **10 spec vectors** — the RFC's own §9 examples, verbatim
- **6 real streams** — Apple BipBop, both MPEG-TS and fMP4/CMAF shapes
- **3 hand-built** — covering tags no §9 example exercises, explicitly labelled as derived

The real-stream tier earned itself immediately: `#EXTINF` was being rounded to 3 decimals, so
Apple's `9.9766` became `9.977` — corrupting timings on every repackage. **No hand-made fixture
could have caught it**, because they were all authored at exactly the precision the bug
preserved.

## Known gaps

Each is pinned by a test, so it announces itself rather than rotting.

| gap | detail |
|---|---|
| **§8 row 12** (#884) | A `REQ-` attribute on a *modeled* tag is dropped at parse time, so it cannot trigger version 12. It still fires on unmodeled tags via `extra_tags`. Closing it needs unknown-attribute retention across the typed structs |
| **Multivariant round-trip** | An unmodeled `EXT-X-MEDIA` is dropped rather than preserved; tag ordering is canonical, not input-preserving. Enumerated in `broadcast-hls/README.md` |
| **§9.10 / §9.11** | Not committed as parseable fixtures — the RFC abridges both with a literal `...` line, so they are not valid playlists |
| **MPTS** | §3.1.1 requires a TS segment to carry a single MPEG-2 Program. `multimux` ingests MPTS; single-program enforcement on the TS-HLS path is unverified |
| **multimux TS-HLS output** (#887) | The origin supports `Container::MpegTs`, but `multimux`'s pipeline is fMP4-only, so it cannot expose classic TS-HLS yet |

## Not implemented

- **Content Steering runtime** — the tags parse and serialize; pathway switching is not implemented
- **Encryption key management** — `EXT-X-KEY`/`EXT-X-SESSION-KEY` are parsed and rendered, and
  CENC/SAMPLE-AES encryption lives in `transmux`; key *delivery* and rotation policy are the
  caller's
- **I-frame playlist generation** — `EXT-X-I-FRAME-STREAM-INF` and `EXT-X-I-FRAMES-ONLY` parse
  and serialize, but trick-play playlist generation is `transmux`'s domain
