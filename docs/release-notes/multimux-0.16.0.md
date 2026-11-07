---
status: DRAFT
---

# multimux 0.16.0

_Released 2026-11-07._

Breaking-class release (0.15 -> 0.16): DASH and LL-DASH dynamic MPDs now carry `MPD@publishTime`, the five pull routes are one generic `PullRoute`, the origin depends on `broadcast-client::tshttp` instead of the absorbed `tshttp-runtime`, and about a dozen sibling epochs move. Public constructors and builders are unchanged.

Version: 0.15.0 -> 0.16.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- Unused `origin::limit::RETRY_AFTER_SECS`.

### Fix: `MPD@publishTime` on dynamic MPDs (#1704)

DASH and LL-DASH `type="dynamic"` MPDs now carry `MPD@publishTime` (ISO/IEC 23009-1 section 5.3.1.2 Table 3: shall be present for dynamic MPDs, and a later value marks an update). It is the instant the newest listed segment became available, so it is unchanged while the window is unchanged and later after every new segment. Players that poll the MPD can now tell an update from a re-fetch.

## Added

- `MultimuxError::Flv(transmux::FlvError)`: an RTMP ingest FLV demux failure keeps its source instead of a flattened `Depay` string; WHIP RTP depayload failures surface as `MultimuxError::Transmux`.

## Fixed

- The DASH and LL-DASH `type="dynamic"` MPDs now carry `MPD@publishTime` (ISO/IEC 23009-1 section 5.3.1.2 Table 3: shall be present for dynamic MPDs, and a later value marks an update). It is the instant the newest listed segment became available, so it is unchanged while the window is unchanged and later after every new segment.
- `rescale_to_90k` (segment ingest) and `ticks_to_us` (DASH period stitcher) divided by a zero timescale; both now share `broadcast_common::timescale` and treat 0 as 1. The call sites already skipped a zero timescale, so no panic was reachable, but the helpers no longer rely on that.

## Other changes

- Depends on `broadcast-client` (its `tshttp` module) instead of the absorbed `tshttp-runtime` crate; the policy types are unchanged.
- The DASH period stitcher's `ticks_to_us` uses `broadcast_common::timescale::to_micros`.
- The five pull routes are one generic `PullRoute<K: PullKind>`; `RtspRoute`, `HlsPullRoute`, `SmoothPullRoute`, `TsHttpRoute` and `DashPullRoute` are type aliases over it with unchanged constructors and builders. The marker types `RtspKind`, `HlsPullKind`, `SmoothPullKind`, `TsHttpKind` and `DashPullKind` are public.
- `split_audio` compares ticks with `broadcast_common::timescale::cmp_ticks`.
- URL userinfo decoding and stripping (RTSP source, HTTP pulls, RTSP push) use `broadcast_auth::{credentials_from_url, strip_userinfo}` instead of four private copies.
- WHIP source and WHEP output share `webrtc_session::{ensure_connect_info, ice_attr}`.
- WHIP and WHEP offer parsing share `webrtc_session::single_video_media`.
- The five pull routes (`RtspRoute`, `HlsPullRoute`, `SmoothPullRoute`, `TsHttpRoute`, `DashPullRoute`) share a crate-private `PullTarget` (name, URL, timeouts, credentials, redacting `Debug`); public constructors unchanged.
- DASH-pull fetch retries use the shared `ReconnectSchedule` (500 ms to 4 s, equal-jittered) instead of a private jitter-free doubling.
- TS packet size and sync byte come from `mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE}` instead of a local copy.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## XML (#1716)

- The Smooth output writer uses the shared `broadcast_common::xml_chars` writer helpers; output is byte-identical (golden test unchanged).
- Smooth-pull protection detection follows `transmux`: only a `<Protection>` element in no namespace counts.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-client` 0.2 -> 0.3
- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6
- `caption-convert` 0.4 -> 0.5
- `container-probe` 0.2 -> 0.3
- `dash-runtime` 0.2 -> 0.3
- `dvb-si` 12 -> 13
- `hls-runtime` 0.11 -> 0.12
- `media-plane` 0.9 -> 0.10
- `mpeg-ts` 0.6 -> 0.7
- `rtmp-runtime` 0.8 -> 0.9
- `rtp-packet` 0.5 -> 0.6
- `rtsp-runtime` 0.8 -> 0.9
- `srt-runtime` 0.7 -> 0.8
- `timed-metadata` 0.8 -> 0.9
- `transmux` 0.29 -> 0.30
- `webrtc-runtime` 0.4 -> 0.5

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `multimux-v0.16.0`.
