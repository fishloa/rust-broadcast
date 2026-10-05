# multimux-cli 0.9.0

_Released 2026-10-05._

Minor release of the `multimux` binary, rebuilt on the library `multimux` 0.11.0. The command line and the JSON config schema are unchanged, but **the binary's behaviour is not**: the library's breaking changes alter which configs are accepted and what the origin serves. **Upgrade if you run WHIP, WHEP, push outputs, RTP/UDP inputs or any authenticated route** (security fixes GHSA-jwfh-m4vx-fhwx, GHSA-48qq-7p78-2jvj, GHSA-6cpc-jqv3-qcj3, GHSA-c5v7-p4jv-2fhc, GHSA-2w4r-qf2x-pqm6), and **read the checklist below before restarting a production instance on an existing config.** The full detail is in [multimux-0.11.0.md](multimux-0.11.0.md). An earlier draft of this release described it as "No CLI changes"; that understated it.

## Check before upgrading

A config that worked on 0.8.x can now be rejected at startup (or on admin add/reload), or can serve different bytes:

- **Route names** must be one safe path segment: `[A-Za-z0-9._-]`, not `.`/`..`, at most 255 bytes; names differing only in case collide.
- **SRT push URLs** are validated: a missing host, a `mode` other than `caller`, a `passphrase`, an out-of-range `latency` or an unbracketed IPv6 authority is an error. `streamid` and `latency` query parameters are now applied rather than treated as part of the address.
- **Reconnect and timeout validation is stricter:** a zero backoff, `initial_backoff_ms` above `max_backoff_ms`, a backoff over 24 h, an `ingest_connect_timeout_secs`/`ingest_read_timeout_secs` outside `(0, 86400]` (or non-finite), or a non-finite or non-positive `target_duration_secs` is a config error.
- **Two outputs mounting the same manifest path** are rejected (several push, `custom` or `whep` outputs on one route remain valid).
- **Smooth Streaming manifest shape changed** (per-`StreamIndex` timelines, real `QualityLevel@Bitrate`, no HEVC); clients keyed on the old values must be updated.
- **DASH durations of 60 s or more are spelled differently** (`PT60S` becomes `PT1M`, `PT3600S` becomes `PT1H`); valid `xs:duration`, but a script matching the old text breaks.
- **Resources are instance-named:** init, segment and part URIs carry a per-origin instance token and only those are cached `immutable`; a route restart renames them.
- **Output-visible additions:** CORS preflight allows `Authorization`; the global concurrency bound answers `503` with `Retry-After`; a route rejects a second concurrent RTMP/WHIP publisher; a startup warning is logged for every ingest route with no authentication; HTTP listeners are HTTP/1 only (no h2c prior-knowledge).
- **UDP inputs** accept new optional keys `recv_buffer_bytes`, `reuse_address`, `multicast_interface`, `reuse_port`.

## Build

`Cargo.toml` delta against 0.8.0: `multimux = "0.11"` (was `0.10`). The binary's manifest also sets `doc = false` for the bin target, because it shares the name `multimux` with the library and rustdoc would write both into `target/doc/multimux` (cargo #6313); the CLI is documented by `--help`. MSRV 1.95.0.

Install or build with `--locked` so the sibling crates resolve to the epochs the library was tested with; see the sibling list in the multimux note.

---

Published from tag `multimux-cli-v0.9.0`.
