# rtsp-runtime 0.11.0

_Released 2026-11-14._

Breaking-class release (0.10 -> 0.11): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.10.0 -> 0.11.0.

## Breaking changes and migration

- `Error::MissingCSeq` is removed. `ServerSession::handle_request` answers a request with no parseable `CSeq` with `400 Bad Request` and carries on, instead of returning an error that ended the connection.

```rust
// before: match err { Error::MissingCSeq => ... }
// after: handle the 400 response path; no variant to match
```

## Added

- `ServerSession::with_describe_sdp` and `with_server_ports`.
- `OPTIONS` reply carries `Public` listing implemented methods.

## Fixed

- Reversed `port`/`client_port`/`server_port`/`interleaved` range is a `Transport` parse error.
- Server picks first servable `Transport` spec; unmodelled spec no longer fails whole header.
- Unimplemented method answered `501` instead of `455`.
- Client forgets session state on `3xx`/`454`; non-2xx `SETUP` no longer records session; `401` stale replay capped at 3.
- `handle_data` applies 1 MiB inbound cap after decoding.
- `Session` `timeout=` parsed as `u64` delta-seconds.
- `AsyncRtspClient` send-side drain keeps decode error.

## Other changes

- A `3xx` or `454 Session Not Found` response resets the session.

## Also in this release

### Changed
- BREAKING: `io::default_tls_client_config` returns `Result<ClientConfig>` (new `Error::TlsConfig`) instead of panicking when the crypto provider refuses the safe default protocol versions (#2591).
- BREAKING: `ServerSession::new` takes a session-id source that is `Send + Sync` (it was `Send`), so a `ServerSession` is `Sync` and can be shared behind an `Arc`; a test pins it (issue #2533).
- The RTSP session keepalive schedule is the only one: `poll_timeout`/`handle_timeout` derive the deadline from the SETUP-declared `Session;timeout=` (RFC 2326 §12.37, 60 s default) and now drive every adapter, which no longer keep timers of their own.

### Added
- `ServerSession::try_new` (a fallible `Session` id source) and `Error::RandomSource`: when the OS random source is unavailable the `SETUP` that needs an id returns that error and creates no session, instead of panicking. `AsyncRtspServer::accept` and `RtspServerConfig::default` use it; their `# Panics` section is gone (#2591).
- `RtspClientConfig { session, timeouts }` and `RtspServerConfig { session, timeouts }` (`new`, `with_session`, `with_timeouts`, `Default`, `From<ClientSession>`/`From<ServerSession>`): every client and server constructor takes `impl Into<..Config>` (#2514).
- `MAX_TRACKS` (16): the track cap owned by rtsp-runtime, used by `PlayPlan` consumers.
- `AsyncRtspClient::start_record(url, sdp, interleaved_channel)` runs the whole publish flow: `ANNOUNCE` (RFC 2326 §10.3), `SETUP` on the track's control URL in `mode=record` (§10.4, §12.39) and `RECORD` (§10.11), returning `Error::UnexpectedResponse { method, status }` for the first request a server rejects and `Error::UnexpectedEvent` when a request draws anything but a response, so a pushing caller supplies only its SDP and its payload.
- `sdp::control_url(base)` and `TRACK_CONTROL`: the track's control URL, `base` with `trackID=0` appended as a final path segment (RFC 2326 Appendix C.1.1) via `Url::path_segments_mut`, and the attribute value that names it; a URL with no authority (`rtsp:cam`) is `Error::InvalidUrl`. `url` is now a plain dependency.
- `sdp::session_description(name, origin, connection, medias)` (re-exported as `session_description`), the workspace's one RFC 4566 §5 session builder: it renders the `v=`/`o=`/`s=`/`c=`/`t=` lines and each media description handed to it, so a crate owning a container's payload formats (`transmux`) describes only its media sections and the RTSP push announces its one section through it. It returns `Error::SdpWrite { what, source }` instead of a lossy conversion when the writer's bytes are not UTF-8, and `sdp-types` 0.3 is now a plain dependency (it is `std`-only, as this crate).
- `AsyncRtspClient::start_play(url, first_channel)` runs the whole pull flow: `DESCRIBE` (RFC 2326 §10.2), `SETUP` per media section in interleaved TCP with consecutive channel pairs from `first_channel` (§12.39), then `PLAY` (§10.5). Relative `a=control` values resolve against the `Content-Base` (else `Content-Location`, else the request URL) of the DESCRIBE response (Appendix C.1.1); `PLAY` goes to the session-level control (absent or `*` is the aggregate); a session control on another authority leaves `PLAY` on the request URL. It returns a `PlaySession { sdp, tracks, skipped }` of `PlayTrack { control, channel }`: a media control on another authority is not set up and is listed in `skipped`, and the channel the server answered with is the one reported.
- `PlayPlan` (module `play`, re-exported with `PlannedSetup`, `SkippedMedia`, `PlayTrack`, `PlaySession`): the sans-IO core of the pull flow. `PlayPlan::from_describe` turns a `DESCRIBE` answer into the `SETUP` list (absolute control URIs, interleaved channel pairs), the skipped foreign-authority media and the `PLAY` URI; `from_describe_selecting` also takes a per-media filter and falls back to the request URL for a foreign session control; `accept_setup` records each `SETUP` answer and rejects a reused channel (RTP or RTCP) or a non-interleaved-TCP answer. `AsyncRtspClient::start_play` is a thin driver of it, and a sans-IO host drives the same plan. `PlayTrack`/`PlaySession` moved here (still re-exported).
- `Error::UnusableTransport`.
- `Error::InterleavedChannelCollision { channel }` (the server assigned one interleaved channel to two tracks), `Error::InterleavedChannelExhausted` (the channel pairs no longer fit in a `u8` from the requested first channel) and `Error::InvalidSdp` (a `DESCRIBE` body that is not UTF-8 or not an SDP).
- `ClientSession::keepalive_method`/`public_methods`: the keepalive uses `GET_PARAMETER` only when the server listed it in the `Public` header of its `OPTIONS` response (RFC 2326 §12.28), and `OPTIONS` otherwise — a server that never advertised it need not implement it, and an unsupported keepalive can draw a `405`/`501` or drop a camera's session.

### Removed
- BREAKING: the `connect_with`, `connect_with_timeouts`, `connect_tls_with`, `connect_tls_with_timeouts`, `accept_tls_with_timeouts` and `with_stream_timeouts` ladder. `AsyncRtspClient::connect(addr, config)`, `connect_tls(addr, server_name, tls, config)`, `with_stream(stream, config)` and `AsyncRtspServer::accept_tls(stream, tls, config)` / `with_stream(stream, config)` take the config; `connect(addr)` and `connect_tls(addr, name, tls)` without one now pass `RtspClientConfig::default()` (#2514).
- BREAKING: `RtspTimeouts`; the adapters take `broadcast_common::timeouts::Timeouts` (the same four fields and defaults), shared with the RTMP and SRT adapters (#2514).

### Fixed
- A `Content-Base`/`Content-Location` carrying a control character is ignored instead of being repaired into another URL by the raw `Url::join`; it resolves through `broadcast_common::url_ref::resolve` (RFC 3986 §2) (#2585).
- `a=control:*` names the aggregate URL itself (`Content-Base`/`Content-Location`, else the request URL) without a trailing slash added, so `PLAY` for request URL `rtsp://h/live/key` goes to `rtsp://h/live/key`; the slash is kept only when joining relative media controls onto a request-URL base (an interop choice, documented).
- A 2xx `SETUP` reply with no `Transport` header no longer leaves the previous `SETUP`'s negotiated transport in `ClientSession::negotiated_transport()`; the transport is cleared on every `SETUP` response, so a second track cannot inherit the first track's channels.
- BREAKING: `PlayPlan::accept_setup(index, None)` is the new `Error::MissingTransport` (RFC 2326 §12.39 requires `Transport` in a 2xx `SETUP` reply) instead of silently keeping the offered channel pair; `start_play` surfaces it for a server that omits the header.
- `AsyncRtspClient::start_play` no longer shows the `send_interleaved` paragraph at the head of its rustdoc.
- `sdp::resolve_control` compiled only with the `tokio` feature because it read the default-port constants from `io`; `RTSP_DEFAULT_PORT`/`RTSPS_DEFAULT_PORT` now live at the crate root (the `rtsp_runtime::` paths are unchanged; BREAKING: the `rtsp_runtime::io::RTSP_DEFAULT_PORT`/`RTSPS_DEFAULT_PORT` paths are gone), so the sans-IO build has them.
- `AsyncRtspClient::start_record` now sends `ANNOUNCE`, `SETUP` and `RECORD` one at a time, checking each response before sending the next, so a rejected `ANNOUNCE` no longer leaks `SETUP`/`RECORD`.
- `AsyncRtspClient::start_record` uses checked arithmetic for the RTCP interleaved channel and returns `Error::InterleavedChannelExhausted` on overflow (matching the play flow) instead of wrapping.
- `AsyncRtspClient::start_record` and `start_play` (and `PlayPlan`) strip userinfo from the request URI, so credentials travel only in the `Authorization` header.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-auth` 0.6 -> 0.7.
- New sibling dependencies: `broadcast-common` 12.

---

Published from tag `rtsp-runtime-v0.11.0`.
