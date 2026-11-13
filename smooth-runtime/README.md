# smooth-runtime

Sans-IO Microsoft Smooth Streaming (MS-SSTR) engine, both ends: a pull client and a live origin.

`SmoothClient` pulls; `server::SmoothOrigin` serves. Both are sans-IO: the host performs the GETs the client asks for, and maps the origin's answers onto HTTP.

## The live origin (`server`, feature `std`)

`SmoothOrigin` is the server-side counterpart of `SmoothClient`: given a `media_plane::Trunk`, the program's published segments and track set, and its init segment, it

- renders the MS-SSTR client Manifest through `broadcast_smooth::SmoothManifest::live(..)`/`to_xml` (no hand-written XML),
- resolves a fragment request (`QualityLevels(BITRATE)/Fragments(KIND=START)`) to that stream's re-muxed fragment bytes, with the `tfxd` the manifest advertises,
- reports each track's `FourCC`, `CodecPrivateData` and parameters through `transmux::resolve_codec`, the resolver the packager uses, so the manifest never disagrees with the fragments.

A second stream of a kind is named `TYPE_N` (`audio`, `audio_2`), subtitle tracks are a `text` `StreamIndex` with `Subtype="SUBT"`, and a track whose codec cannot be described (an AAC track with no `AudioSpecificConfig`) makes the Manifest a refusal rather than an empty `CodecPrivateData`.

`resolve` answers `Ready`, `Await` (nothing to describe yet — wait on `Trunk::listen`), `BadRequest` (a track cannot be described) or `NotFound`; it never blocks, sleeps or reads a clock.

## The pull client

Sans-IO Microsoft Smooth Streaming (MS-SSTR) pull client: `SmoothClient` plays a static or live client manifest to ordered, demuxed samples. The manifest syntax is `broadcast-smooth`'s and the codec glue (init segment synthesis, fMP4 demux) is `transmux`'s; this crate decides what to fetch next and what the bytes mean.

Use it to build a Smooth player or ingest without taking a dependency on an HTTP stack or an async runtime: the host performs the GETs the engine asks for, tells it the time, and reads back tracks, samples and typed outcomes. `broadcast-client` and `multimux` are both thin adapters over it.

## What it does / what it does not

**Does.**
- Fetches and refreshes the manifest, retrying a failed fetch with backoff and reporting a 404/410 on it.
- Refuses a manifest with a `Protection` element and a fragment carrying PIFF/CENC sample-encryption boxes, as typed fatal errors.
- Matches each stream across a refresh by its full identity (`Type`, `Name`, `Url`), abandons one that stays missing, and reports a window that moved past the last fragment as a discontinuity.
- Selects every video and audio stream, with the video `QualityLevel` chosen by a caller-supplied `QualitySelector`.
- Enumerates the `c` timeline, resolves fragment URLs and joins a live presentation a configurable number of fragments behind the edge, never before the DVR window.
- Discovers each stream's wire track id from its first fragment, synthesizes the init segment from `CodecPrivateData`, announces one track per stream (ids `1..`), and rebases every fragment's samples onto its `tfxd` start.
- Retries a failed fragment with backoff; skips it with a discontinuity on a live presentation, ends a static one.

**Does not.**
- Perform I/O, read a clock or sleep: every instant is an argument in microseconds and `next_deadline` says when to call back.
- Decrypt, or play `text` streams.

## Example

```bash
cargo run -p smooth-runtime --example play_static
cargo run -p smooth-runtime --example play_live -- --look-ahead 0
```

## Verification

Behaviour goldens of both former clients (`fixtures/golden/smooth/pull-*.txt`), engine tests over the committed `fixtures/smooth` fixtures, a no-clock source guard and a fuzz target (`smooth_client`).

License: MIT OR Apache-2.0
