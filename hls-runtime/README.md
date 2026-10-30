# hls-runtime

[![crates.io](https://img.shields.io/crates/v/hls-runtime.svg)](https://crates.io/crates/hls-runtime)
[![docs.rs](https://img.shields.io/docsrs/hls-runtime)](https://docs.rs/hls-runtime)

Sans-IO Low-Latency HLS (RFC 8216bis) client and origin engines in one crate: a driveable playback client, and an origin that renders playlists and resolves blocking-reload requests straight from a `media_plane::Trunk`. An optional `tokio` + `reqwest` adapter provides real HTTP for the client.

Formerly published as `ll-hls-runtime` (last release 0.3.1 under that name); search for either name. (The earlier in-repo name `ll-hls-client` was never published.)

```toml
[dependencies]
hls-runtime = "0.9"
```

## What it does / what it does not

**Does (client):** `client::HlsClient` is a caller-driven state machine. It tells you what to fetch (`Action::FetchPlaylist`, `FetchResource`, `FetchKey`, `WaitMs`), you do the IO, you feed the bytes back, and decoded `Output`s (one `Init`, then ordered `Samples`, plus `Discontinuity` and `EndOfStream`) come out. It schedules Blocking Playlist Reloads (`_HLS_msn`/`_HLS_part`), requests and merges Playlist Delta Updates (`EXT-X-SKIP`), prefetches the `EXT-X-PRELOAD-HINT` part, honours `BYTERANGE` parts, avoids re-fetching a segment whose parts it already has, demuxes fMP4/CMAF with `transmux::Fmp4Demux`, and also plays classic MPEG-TS-segment HLS (HLS v3, no `EXT-X-MAP`) through `transmux::TsDemux`.

**Does (origin):** `server::HlsOrigin` implements `media_plane::egress::ServedEgress`. It owns one segment cursor, renders media and master playlists (LL-HLS parts, preload hint, `EXT-X-DATERANGE` from the trunk's SCTE-35 events, discontinuity sequence), names resources per origin instance so a name maps to one set of bytes for ever, and resolves blocking-reload and part-availability decisions as a synchronous poll returning `Ready`, `Await`, `BadRequest` or `NotFound`.

**Does not:** open sockets or read a clock in the core (an async adapter such as `multimux` turns an `Await` into a bounded wait), define its own playlist model (that is `broadcast-hls`), choose a rendition from a multivariant playlist (the client follows one media playlist URL), or have the origin emit `EXT-X-SKIP`, `CAN-SKIP-UNTIL` or `EXT-X-RENDITION-REPORT` (so the two halves cannot exercise delta updates against each other).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF HLS 2nd edition, `draft-pantos-hls-rfc8216bis` | Internet-Draft of 1 May 2026, "HTTP Live Streaming 2nd Edition" (vendored `specs/ietf_draft_pantos_hls_rfc8216bis.txt`) | §6.2.5.2 Blocking Playlist Reload, Partial Segments, preload hint, Playlist Delta Updates (client), `EXT-X-DATERANGE` (origin), discontinuities | Partial | Origin has no delta updates or rendition reports; client has no ABR |
| IETF RFC 8216 HLS | RFC 8216 (vendored `specs/ietf_rfc8216_hls.txt`) | Classic TS-segment HLS playback (HLS v3 form) | Partial | Client only |
| Apple HLS timed metadata | Vendored `specs/apple_hls_timed_metadata.html` | SCTE-35 cues rendered as `EXT-X-DATERANGE` via `timed-metadata` | Partial | |
| IETF RFC 3986 reference resolution, RFC 3339 dates | RFC 3986, RFC 3339 | Relative URL joins (`url`), program-date-time (`jiff`) | Full | Not hand-rolled |
| IETF RFC 7617 / 7616 / 6750 | not vendored | `TokioClient` authentication through `broadcast-auth`; credentials go only to the playlist's origin | Full, behind `tokio` | |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Enables `server` (needs `media_plane::Trunk`, which is `std`), plus `bytes`, `url/std`, `jiff/std`. Without it the client core is `#![no_std]` + `alloc`. |
| `serde` | no | `Serialize` on the public action, output and id types. |
| `tokio` | no | `client::tokio_client::TokioClient`: async shell over tokio and reqwest (rustls). Blocking-reload query parameters, typed `Range`, per-request and connect timeouts, `backon` retry with jitter (no retry of a 4xx other than 408 and 429), a `CancellationToken`, and Basic, Bearer or Digest through `broadcast-auth`. Implies `std`. |

## Quick start

Drive the client over real ffmpeg fMP4 HLS output, with the playlist text and the segment files handed over by hand (no HTTP):

```rust
use hls_runtime::client::{Action, HlsClient, Output};

let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n\
    #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MAP:URI=\"init.mp4\"\n\
    #EXTINF:1.000000,\nindex0.m4s\n#EXTINF:1.000000,\nindex1.m4s\n#EXTINF:1.000000,\nindex2.m4s\n\
    #EXT-X-ENDLIST\n";

let mut client = HlsClient::new("http://origin/live/index.m3u8");
// The first action is always a plain playlist GET.
assert!(matches!(client.poll(), Some(Action::FetchPlaylist { .. })));
client.on_playlist(playlist.as_bytes()).unwrap();

let (mut init, mut samples) = (0, 0);
loop {
    while let Some(action) = client.poll() {
        if let Action::FetchResource { id, url, .. } = action {
            let file = url.rsplit('/').next().unwrap();
            let bytes = test_fixtures::read(&format!("hls/cmaf-fmp4/{file}"));
            client.on_resource(id, &bytes).unwrap();
        }
    }
    let mut progressed = false;
    while let Some(out) = client.next_output() {
        progressed = true;
        match out {
            Output::Init(_) => init += 1,
            Output::Samples { samples: s, .. } => samples += s.len(),
            _ => {}
        }
    }
    if !progressed {
        break;
    }
}
assert_eq!(init, 1, "exactly one init precedes the samples");
assert!(samples > 0, "real H.264 and AAC access units came out");
```

For the origin, `examples/origin_playlist.rs` builds a `Trunk`, publishes parts and segments, and renders a playlist through `HlsOrigin::resolve`; `examples/client_stepping.rs` steps the client against that origin's output. Run them with `cargo run -p hls-runtime --example <name>`. `tests/origin_loop.rs` drives the client against a real `transmux::ll_hls::LlHlsSegmenter` in process.

## What is implemented

- **`client`:** `HlsClient` (`new`, `poll`, `on_playlist`, `on_resource`, `on_error`, `on_key`, `on_key_error`, `next_output`, `poll_timeout`), `Action`, `Output`, `ResourceId`, `KeyId`, error type, relative URL resolution.
- **AES-128 segments (RFC 8216 §4.3.2.4, §5):** `METHOD=AES-128` with the `identity` `KEYFORMAT` is decrypted before demuxing. The key is requested once per distinct URI (`Action::FetchKey`, answered with `on_key`, which requires exactly 16 octets), segments wait for it, and the IV is the `IV` attribute or the Media Sequence Number as a 16-octet big-endian number. A bad key, padding or length is `Error::DecryptFailed`. `SAMPLE-AES`, `AES-256-GCM`, an encrypted `EXT-X-MAP`, `EXT-X-I-FRAMES-ONLY` with encryption and encrypted partial segments are `Error::UnsupportedEncryption`, never passed through as plaintext.
- **`server`** (feature `std`): `HlsOrigin` and its builder (`target_duration_secs`, `window_segments`, `low_latency`), `HlsRequest`, `BlockingQuery`, `HlsBody`, `ClosedSegment`, `master_playlist` (measured peak `BANDWIDTH`, the init segment's `CODECS`, audio rendition groups via `audio_renditions`/`closed_captions_none`, typed `HlsMasterError`), `master_playlist_m3u8` (context-free renderer), `set_init`, `set_track_specs` (TS origins), `DEFAULT_TRACK_ID`.
- **Typed identifiers:** `server::SegmentSeq` and `server::PartIndex` re-export `media_plane::ids`, `server::Generation` types the init-segment generation, and tracks are `transmux::TrackId`, so a swapped segment, part or generation argument is a compile error. Wire names (`seg-1-5-7.m4s`, `init-1-5-2.mp4`) are unchanged.
- **Cache policy:** each resolved resource carries a `media_plane::egress::CachePolicy` (`Immutable` for instance-named parts, segments and generation-named inits; `NoCache` for the bare current init and playlists).
- **Poison-tolerant locks** in the origin, and bounded behaviour on abusive requests (`tests/origin_hardening.rs`).

## Design notes

Sans-IO (no clock read, no socket, in the core), `#![forbid(unsafe_code)]`, `no_std` + `alloc` client, one wire model (playlists are `broadcast_hls::MediaPlaylist`, not re-described), the `Trunk` is the single copy of the data (the old push-fed `MediaStore` was deleted), `#[non_exhaustive]` on `Action`, `Output` and errors, golden client transcripts and a no-hand-roll guard. Known gap: a discontinuity revealed only when a still-open segment closes is signalled late, because `OpenSegment` carries no discontinuity flag in the wire model.

## Related crates

Depends on `broadcast-hls` (playlist syntax), `transmux` (demux, `TrackId`), `media-plane` (`Trunk`, `ServedEgress`), `timed-metadata` (`EXT-X-DATERANGE`) and, for `tokio`, `broadcast-auth`. `multimux` is the reference adapter: it serves every LL-HLS route through one `HlsOrigin` per program over axum and tokio.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
