# broadcast-client 0.7.0

_Released 2026-11-17._

Minor-class release (0.6 -> 0.7) in the audit 20 resource-bounds bundle. It bounds the UDP reorder buffer in bytes as well as packets, ends a session after a configured number of failed reconnects, passes playlist limits into the HLS engine, and closes a response whose declared length is already past its cap at the head. `ConfigField` also gained a variant in the middle of the enum.

## Upgrading

- `ConfigField` gained `UdpReorderBytes` between `UdpReorderPackets` and `UdpReorderWait`, so `UdpReorderWait` and every later variant moved to a new discriminant. This matters only to code that casts a variant to an integer; a `match` on the enum is unchanged.
- `dash-runtime` 0.5 -> 0.6 (the requirement is now `0.6.0`).
- `hls-runtime` 0.14 -> 0.15 (the requirement is now `0.15.0`).
- `rtsp-runtime` 0.12 -> 0.13 (the requirement is now `0.13.0`).
- `smooth-runtime` 0.2 -> 0.3 (the requirement is now `0.3.0`).
- `srt-runtime` 0.10 -> 0.11 (the requirement is now `0.11.0`).

## Added

- `Config::hls_limits` (`broadcast_hls::limits::Limits`) is passed to the HLS engine, and a playlist the parser refuses for exceeding one is a permanent failure: it ends the session at once as `ErrorKind::PlaylistParse { reason: "too_large" }` instead of being fetched and re-parsed forever (audit 20 finding 41, #2852).
- `UdpConfig::reorder_bytes` (default `DEFAULT_REORDER_BYTES`, 4 MiB) bounds the RTP reorder buffer in bytes next to `reorder_packets`. The sender chooses a payload's size, so the packet window alone let it hold `reorder_packets` payloads of 64 KiB; reaching the byte budget gives up on every held packet, counts the gaps lost and delivers the packet that would have overrun it. `UdpConfig::validate` rejects a zero budget as `ConfigField::UdpReorderBytes` (#2846).
- `TsHttpConfig::max_reconnect_attempts` (`Option<u32>`, default `None`) ends the session with one fatal `Event::Error` (`ErrorKind::ReconnectAttemptsExhausted`) once that many reconnects in a row failed without media in between, instead of retrying a dead URL forever; it applies to TS-over-HTTP, RTMP and RTSP. `TsHttpConfig::with_max_reconnect_attempts` sets it (#2864).

## Fixed

- A response whose declared `Content-Length` (or the length its `Content-Range` names) is already past `max_segment_bytes`/`max_manifest_bytes` is closed at the head, with one non-fatal `ErrorKind::BodyTooLarge`, instead of being downloaded until the cap trips. The parked-prefix stash also holds at most two segment bodies together, evicting the oldest, so a host that stalls every request in flight keeps `max_segment_bytes` x 2 parked rather than x 8 (#2847).
- A host that can send UDP to the bound port can no longer reset playback with one datagram: a new SSRC is adopted only after `SSRC_PROBATION` consecutive in-order packets (until then its packets are dropped and counted `DatagramStats::late`), a `Event::Discontinuity` queued right behind another coalesces into it, and the kept events a player can resync past (`Event::Discontinuity`, `Event::AvSync`) are capped at 1024, so a stalled host's queue no longer grows without bound and a draining host's playout is no longer reset per spoofed packet (#2854).
- `UdpConfig::reorder_packets` is exercised just past its limit: `MAX_REORDER_PACKETS + 1` is rejected as `ConfigField::UdpReorderPackets` rather than only the zero case, so removing the upper-bound check no longer passes the suite (#2865).
- A jar of 50 near-cap cookies now has a test that drives the emitted `Cookie` header past its 16 KiB cap and asserts both the cap and that the longest-path cookies are the ones kept, so removing the cap no longer passes the suite. The 1 KiB `Path` cap already had one (`audit_1484::cookie_path_longer_than_cap_is_rejected`) (#2869).

---

Published from tag `broadcast-client-v0.7.0`.
