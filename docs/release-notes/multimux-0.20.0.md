# multimux 0.20.0

_Released 2026-11-17._

Minor-class release (0.19 -> 0.20) from the audit 20 resource-bounds and security-surface bundle. It bounds per-peer connections, the admin reload budget, the catch-up index cache and the DASH-IF segment handle; confines `File`/SDP/DVR archive paths to configured roots; authenticates `/metrics` when `output_auth` is set; and fixes a WHEP socket mapping that accepted a ufrag without the password. TLS moves to ring, and `load_sdp` becomes async.

## Upgrading

- BREAKING: TLS uses the ring crypto provider, not aws-lc-rs, and `rustls` is no longer optional. Configs are unchanged; a downstream crate that wants aws-lc-rs must install that provider itself.
- BREAKING: `source::sdp::load_sdp` is async; `.await` the call. An SDP file must be a regular file no larger than `source::sdp::MAX_SDP_FILE_BYTES` (64 KiB).
- `broadcast-client` 0.6 -> 0.7 (the requirement is now `0.7.0`).
- `dash-runtime` 0.5 -> 0.6 (the requirement is now `0.6.0`).
- `hls-runtime` 0.14 -> 0.15 (the requirement is now `0.15.0`).
- `media-doctor` 0.16 -> 0.17 (the requirement is now `0.17.0`).
- `media-plane` 0.12 -> 0.13 (the requirement is now `0.13.0`).
- `rtsp-runtime` 0.12 -> 0.13 (the requirement is now `0.13.0`).
- `smooth-runtime` 0.2 -> 0.3 (the requirement is now `0.3.0`).
- `srt-runtime` 0.10 -> 0.11 (the requirement is now `0.11.0`).
- `webrtc-runtime` 0.8 -> 0.9 (the requirement is now `0.9.0`).

## Changed

- BREAKING: the TLS crypto provider is ring, not aws-lc-rs (#2877). `rustls` is no longer optional because the HTTP pull client always uses it; `tokio-rustls`/`rustls` take `default-features = false`; `reqwest` uses `rustls-no-provider` with a ring-backed `ClientConfig` built by `source::client_host::http_client` (platform roots, as reqwest's default). The `tls` feature no longer names `dep:rustls`. Migration: none for configs; a downstream crate that wants aws-lc-rs must install its own provider.
- BREAKING: `source::sdp::load_sdp` is async (it reads an on-disk `@path` with `tokio::fs`), and an SDP file is capped at `source::sdp::MAX_SDP_FILE_BYTES` (64 KiB) and must be a regular file, so `@/dev/zero` or a FIFO can no longer stall a runtime worker or grow the process. Migration: `.await` the call (#2836).

## Fixed

- The WHEP shared media socket registers each viewer's local `ice-pwd` with `webrtc_runtime::media::SocketDemux` (including after an ICE restart), so a STUN check that names a viewer's ufrag but does not carry the password no longer maps its source address to that viewer: unauthenticated traffic can neither fill a viewer's tuple budget nor steal an already-mapped address (audit 20 finding 40).
- The RTMP accept pump backs off 100 ms through a transient listen-socket error (a connection flood's `EMFILE`, a peer's pre-accept `ECONNABORTED`) instead of ending the route, and a pump that does end is marked dead so the next attempt rebinds rather than reusing a receiver nothing feeds (#2812).
- The media listener caps concurrent connections per peer IP at `MAX_CONNECTIONS_PER_IP` (64) alongside the global cap, and fails a response body that produces no frame for `limit::IDLE_WRITE_TIMEOUT` (30 s), so one host cannot hold every connection slot and pool permit by never reading what it requested (#2818).
- The Prometheus recorder runs `run_upkeep` every `prometheus::UPKEEP_INTERVAL` (5 s) on the caller's Tokio runtime, so histogram samples no longer accumulate until `/metrics` is scraped (#2833).
- A WHEP media socket's receive loop backs off 10 ms on a failed `recv_from` and, after `RECV_ERRORS_BEFORE_DEAD` consecutive errors, ends and marks the socket dead; the next `socket_for` rebinds it, so a persistent receive error no longer busy-loops the task and starves every viewer on that socket (#2834).
- A removed or replaced route's catch-up index cache is dropped (`catchup::drop_route_cache`, called from the admin drain) instead of staying resident for the process lifetime, and the cache is bounded by total cached segments (`INDEX_CACHE_MAX_SEGMENTS`), not just period-file count (#2835).
- The admin registry keeps one `limit::BudgetLimitLayer` and passes it to every router rebuild, so the ordinary concurrency pool and its in-flight permits survive a route add/remove; only the per-route blocking-reload pools are rebuilt, instead of the effective bound doubling after each rebuild (#2874).
- `GET /metrics` is behind `output_auth` when it is configured (the liveness/readiness probes stay open) and draws from the ordinary concurrency pool instead of being exempt, so an unauthenticated scrape can no longer read every route name and counter or bypass the limit (#2875).
- `AdminSpec` gains `allowed_roots` (`with_allowed_roots`), confining a route's `File` path, `@path` SDP reference and DVR `archive_root` to those directories (canonicalised, existing roots); empty keeps the documented unrestricted default. Enforced for startup configs, `POST /admin/routes` and `POST /admin/reload` (#2875).
- DASH-IF ingest: a publisher disconnect mid-PUT now releases the open segment handle through an RAII guard, and every pump reply is bounded by the route body deadline, so a later segment is no longer refused with `SegmentInProgress`, and a stalled pump no longer parks the handler (#2825).
- Removing or reloading a route lets each push task close its transport gracefully under the shutdown grace period, aborting only when it overruns, so downstream RTMP/RTSP/SRT servers see a normal close instead of an abrupt drop (#2826).
- A failed DVR archive scan now fails the route's start or reload through `DvrError::ArchiveScan` (`MultimuxError::Dvr`) instead of reading as sequence floor `0`, so a restart cannot renumber over archived segments and collide with them (#2827).
- A `Range: bytes=-N` suffix longer than the representation answers `206` with `Content-Range: bytes 0-(n-1)/n` instead of a full `200` (RFC 9110 §14.1.2); `bytes=-0` remains `416` (#2828).
- The file source opens the path once and checks that handle's metadata, reading through a `max_file_bytes + 1` window, so a path swapped for a FIFO or a file grown after the check cannot block the read or exceed the cap (#2837).
- `redact::redact_url` renders a query string as `?<redacted>` and drops the fragment, and its unparseable-URL fallback finds the credential `@` past a `/` inside a password, so a `?token=…` or a `rtsp://u:pa/ss@host/s` URL can no longer reach a log line (#2858).

## Tests

- The DASH-IF handler-drop abort and the over-long `Range` suffix each have a regression test (#2876).

---

Published from tag `multimux-v0.20.0`.
