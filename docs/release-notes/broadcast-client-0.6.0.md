# broadcast-client 0.6.0

_Released 2026-11-15._

Minor-class release (0.5 -> 0.6). The session's whole boundary is typed: `Session::open` takes a parsed `url::Url`, every `now` is a `broadcast_common::stage::Timestamp`, every config span is a `Duration`, an HTTP request or response carries `http`/`url` types instead of strings, a connection is a `ConnId` and its failure a `TransportError`. TS over HTTP has one public entry point (`Session`), the retry schedule is the workspace's `RetrySchedule`, and the UDP and SRT sources take the shared option and timeout types.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

### `Session::open` takes a `url::Url`

`Session::open` and `Session::open_with_memory` take `url: url::Url` (was `&str`): the session no longer parses the URL, so `Error::InvalidUrl` only ever carries a validation failure from a part the caller was trusted to parse. `Error::UnsupportedScheme(String)` is `Error::UnsupportedScheme { scheme }`.

```rust
// before
let session = Session::open("https://example.com/live.m3u8", config, now_us)?;
// after
let session = Session::open(Url::parse("https://example.com/live.m3u8")?, config, Timestamp::from_micros(now_us))?;
```

### Every `now` is a `stage::Timestamp`

`Session::open`, `open_with_memory`, `poll`, `on_response`, `on_data`, `on_closed`, `reconnect`, `on_playback_position`, `set_rendered_tracks`, `set_wall_clock`, `set_credentials`, `select_variant`, `select_audio_rendition`, `hint`, `rejoin` and `close` take `broadcast_common::stage::Timestamp` (monotonic microseconds from the host's epoch, the type `media-plane` and `smooth-runtime` already take) instead of `now_us: u64`, and `Action::WakeAt` carries a `Timestamp`. One clock now drives HLS, DASH, Smooth, RTSP and TS-over-HTTP, and the requests and timers those calls start are stamped with the instant of the call rather than the last instant the session happened to see.

```rust
// before
session.poll(now_us);
session.set_credentials(creds);
session.close();
// after
session.poll(Timestamp::from_micros(now_us));
session.set_credentials(Timestamp::from_micros(now_us), creds);
session.close(Timestamp::from_micros(now_us));
```

### `Action::HttpRequest`/`on_response` carry HTTP types

`Action::HttpRequest` carries `method: http::Method`, `url: url::Url` and `headers: http::HeaderMap` (was `String`, `String` and `Vec<(String, String)>`), and `byte_range` is `Option<broadcast_common::range::ByteSpan>` (was `Option<(u64, u64)>`). `Session::on_response` takes `status: http::StatusCode` and `headers: &http::HeaderMap`. `Event::AuthRejected`, `Event::Forbidden` and `Event::ServerRefused` carry `http::StatusCode`, and `ErrorKind::{KeyFetchFailed, InitFetchFailed, SegmentFetchFailed}` carry `status: Option<http::StatusCode>`.

```rust
// before
match action {
    Action::HttpRequest { method, url, headers, byte_range, .. } => {
        let ranged = byte_range.map(|(offset, len)| format!("bytes={offset}-{}", offset + len - 1));
        // method/url/headers were a String, a String and pairs
    }
    _ => {}
}
// after
match action {
    Action::HttpRequest { method, url, headers, byte_range, .. } => {
        let ranged = byte_range.map(|span| span.range_header());
        let (text_method, path) = (method.as_str(), url.path());
        for (name, value) in headers.iter() { let _ = (name, value.to_str()); }
    }
    _ => {}
}
```

### `ConnId` is a newtype and a failure is a `TransportError`

`ConnId` is a `#[repr(transparent)]` newtype over `u32` (was a type alias) with `new`/`get`/`Display`, so a connection id cannot be passed where a track id or another `u32` belongs; every action `conn` and `Session::on_response`/`on_data`/`on_closed`/`on_connected` take it. `TransportError` is an enum (`Refused`, `Reset`, `TimedOut`, `Dns { host }`, `Tls { reason }`, `Eof`, `Other { reason }`, `#[non_exhaustive]`) instead of `String`, so a close reason tells a TLS failure from a reset.

```rust
// before
let id: u32 = 7;
session.on_closed(id, Some("connection reset".into()), now_us);
// after
let id = ConnId::new(7);
session.on_closed(id, Some(TransportError::Reset), Timestamp::from_micros(now_us));
let raw: u32 = id.get();
```

### `Action::UdpBind` splits into `UdpBind` and `UdpConnect`

`Action::UdpBind { conn, addr: SocketAddr, options }` is a literal local/multicast receive bind, and the new `Action::UdpConnect { conn, host: url::Host, port, options }` is an SRT peer the host resolves first, so the session never resolves a name and a receive bind always carries a `SocketAddr`; the `mode` field is gone (the variant names the mode). A UDP/RTP URL whose host is a name rather than a literal IP is the new `Error::DatagramHostNotIp { scheme, host }` at open.

```rust
// before
Action::UdpBind { conn, host, port, mode: BindMode::Receive, .. }
// after
Action::UdpBind { conn, addr, options }
Action::UdpConnect { conn, host, port, options }
```

### `UdpConfig` takes one `UdpOptions`

`UdpConfig`'s `interface`, `interface_index`, `source` and `recv_buffer_bytes` fields are one `options: broadcast_common::net::UdpOptions`, and `MulticastJoin` is the shared `broadcast_common::net::MulticastJoin`. The session fills the join's group from the URL, so "interface and index never set together" is unrepresentable, and the multicast-join errors are typed at `Session::open`. `broadcast_client::MulticastJoin` is no longer re-exported at the crate root.

```rust
// before
cfg.udp.interface = Some(ip);
cfg.udp.source = Some(src);
cfg.udp.recv_buffer_bytes = Some(n);
// after
cfg.udp.options = cfg.udp
    .options
    .with_join(MulticastJoin::new(group).with_interface(MulticastInterface::V4(ip)).with_source(src))
    .with_recv_buffer_bytes(NonZeroU32::new(n).unwrap());
// an IPv6 interface index: .with_interface(MulticastInterface::V6Index(i))
```

### Every span is a `Duration`

`Config`, `PlayoutConfig`, `UdpConfig` and `BufferStats` carry `Duration` (or `Option<Duration>` where `None` means "off") instead of microsecond integers, so `0` no longer stands for both "disabled" and "invalid" and `u64::MAX` is no longer "forever". `Config::tracks_hold_max_media`/`tracks_hold_max_wall` are `Option<Duration>` (`None` announces each stream as it resolves), `start_pending_track_wait`/`start_pending_track_hard_cap` are `Duration` (`Duration::ZERO` does not wait, `Duration::MAX` waits until the first `Start`), `PlayoutConfig::vod_start_lead`/`live_start_lead`/`resume_lead`/`pause_lead` are `Duration`/`Option<Duration>`, `request_timeout`/`segment_timeout`/`segment_inactivity_timeout`/`hls_key_timeout` replace the `_us` fields, `UdpConfig::reorder_wait` is a `Duration` that `validate` rejects when zero, and `BufferStats::longest_freeze` is a `Duration`. `Config::backoff_ms: Vec<u64>` and `backoff_cap_ms: u64` are one `backoff: BackoffSteps`. The `DEFAULT_*` constants and the `with_*` setters follow.

```rust
// before
let config = Config { tracks_hold_max_wall_us: 0, start_pending_track_hard_cap_us: 0, request_timeout_us: 10_000_000, backoff_ms: vec![500], .. };
// after
let config = Config { tracks_hold_max_wall: None, start_pending_track_hard_cap: Duration::ZERO, request_timeout: Duration::from_secs(10), backoff: BackoffSteps { steps: vec![Duration::from_millis(500)], .. }, .. };
```

### The retry schedule is `broadcast_common::backoff::RetrySchedule`

`Config::backoff` (`BackoffSteps`) is handed to the one `RetrySchedule` as configured, so a sub-millisecond step keeps its microseconds instead of being truncated to whole milliseconds and re-raised to 1 ms, and a configured zero is taken as a zero. A schedule no retry schedule can run with (a cap above `MAX_FLOOR`) is `Error::InvalidConfig(ConfigField::Backoff)` from `Session::open` instead of being silently repaired. The `fetch` module (`retry_after_us`, `HEALTHY_CONNECTION_US`, `ReconnectTimer`) is no longer public surface; the workspace's reconnect schedule is `broadcast_common::backoff::RetrySchedule`/`ReconnectPolicy`.

```rust
// before
let mut timer = broadcast_client::fetch::ReconnectTimer::new(500, 30_000);
// after
let policy = broadcast_common::backoff::ReconnectPolicy::doubling(Duration::from_millis(500), Duration::from_secs(30));
```

### TS over HTTP has one entry point: `Session`

The `tshttp` module (with `TsHttpClient`, `TsHttpPolicy`, `Action`, `Event` and `ConfigError`) and `TsHttpSource` are no longer public: drive TS over HTTP through `Session` with `Config { kind: SourceKind::TsHttp, .. }`, and type the stall policy as the re-exported `broadcast_client::TsHttpConfig` (`StarveKind` stays at the crate root). The internal engine's integration tests moved into the crate's own test modules, the two `tshttp` examples are deleted, and the `tshttp_runtime` fuzz target now drives a `Session`.

```rust
// before
let client = TsHttpClient::new(policy);
// after
let session = Session::open(url, Config { kind: SourceKind::TsHttp, ..Config::default() }, now)?;
```

### `SrtConfig` is `SrtCallerConfig`, with `Timeouts`

`broadcast_client::SrtConfig` is `broadcast_client::SrtCallerConfig` (it no longer shares a name with `srt_runtime::io::SrtSocketConfig`), and its `handshake_timeout_ms`/`peer_idle_timeout_ms` fields and `timeouts()`/`with_timeouts` converters are one `timeouts: Timeouts` field (`handshake` the handshake, `read_idle` the peer idle). A zero bound was raised to one millisecond; it is now `Error::InvalidConfig(ConfigField::SrtHandshakeTimeout)` (or `SrtReadIdleTimeout`), as for every other zero timeout in `Config`.

```rust
// before
c.srt.handshake_timeout_ms = 1_000;
// after
c.srt.timeouts = c.srt.timeouts.with_handshake(Duration::from_secs(1));
```

### The config error names the field

`Error::InvalidConfig { field: &'static str }` is `Error::InvalidConfig(ConfigField)` — a typed `ConfigField` enum (`Backoff`, `ClientIdentity`, `SrtEntropySeed`, `SrtHandshakeTimeout`, `SrtReadIdleTimeout`, `SrtPassphrase`, `TracksHold*`, `Udp*`, `Max*`, `RequestTimeout`, `SegmentTimeout`, `SegmentInactivityTimeout`, `HlsKeyTimeout`, `Buffer`, `TimelineJump*`, `Dash*`) with `name()` returning the dotted field path, re-exported at the crate root.

```rust
// before
if let Error::InvalidConfig { field } = err { if field == "backoff" { .. } }
// after
if let Error::InvalidConfig(ConfigField::Backoff) = err { .. }
```

### Track ids and attempt counts are typed

`ErrorKind::TrackIdReserved`, `ErrorKind::TrackDropped` and `ErrorKind::UnknownRenderedTrack` (each's `track_id`) carry their id as `transmux::TrackId` (was `u32`); `Status::Reconnecting::attempt` is a `u32` (was `u64`), the width every other attempt counter already uses, and it saturates at `u32::MAX` instead of wrapping to `0` (which would announce a reconnect after a played connection).

```rust
// before
if let ErrorKind::TrackDropped { track_id, .. } = kind { if track_id == 0 { .. } }
let attempt: u64 = status.attempt;
// after
if let ErrorKind::TrackDropped { track_id, .. } = kind { if track_id == TrackId::new(0) { .. } }
let attempt: u32 = status.attempt;
```

### `Hint` is `BufferHint`; more types are `#[non_exhaustive]`

The `Hint` alias for `buffering_runtime::Hint` is `BufferHint`, matching `BufferInput`/`BufferEvent`/`BufferStats`. `playout::PlayoutConfig` and `TsHttpConfig` (the former `tshttp::Config`) are `#[non_exhaustive]`; build them with `Default::default()` + `with_*`.

## Fixes

- A deadline that does not fit the clock is no deadline, not the last representable instant: RTSP, SRT, UDP/RTP and TS-over-HTTP source timers and the session's TS wake are computed with `Timestamp::checked_add`, so a session started near the end of its clock goes idle instead of waking on every poll forever.
- A header value the server put in its own `Set-Cookie` lines (a DEL byte parses as a cookie but is not an HTTP field value) no longer panics the session when the next request is built: the header `http` cannot represent is dropped and the request goes out without it.
- A DASH Representation's `@bandwidth` is reported in `Event::Variants` as parsed, not clamped to `u32::MAX`, and a Smooth quality's variant reports `uri: None` instead of an empty string. A DASH Representation whose Initialization Segment URL cannot be parsed is left out of the list rather than listed with no URI.
- `Config::with_timeouts` maps SRT exactly as `SrtCallerConfig::with_timeouts` does (`handshake` to the handshake bound, `read_idle` to the peer-idle bound; it used `connect` for the handshake), and under the default `SourceKind::Auto` applies every protocol's mapping so the one the URL resolves to takes effect.

## Removed

- `fetch` (`retry_after_us`, `HEALTHY_CONNECTION_US`, `ReconnectTimer`) — use `broadcast_common::backoff`.
- `tshttp` and `TsHttpSource` (one entry point: `Session`).
- The unused `cookie`, `cookie_store` and `percent-encoding` dependencies; the cookie jar lives in `broadcast-auth`.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`); `broadcast-auth` 0.7 -> 0.8, `broadcast-dash` 0.1 -> 0.2, `broadcast-hls` 0.7 -> 0.8, `buffering-runtime` 0.3 -> 0.4, `dash-runtime` 0.4 -> 0.5, `dvb-si` 14 -> 15, `hls-runtime` 0.13 -> 0.14, `mpeg-ts` 0.8 -> 0.9, `rtmp-runtime` 0.10 -> 0.11, `rtp-packet` 0.7 -> 0.8, `rtsp-runtime` 0.11 -> 0.12, `smooth-runtime` 0.1 -> 0.2, `srt-runtime` 0.9 -> 0.10, `transmux` 0.31 -> 0.32.
- Dev-dependency `rtsp-types` 0.1 -> 0.2, following `rtsp-runtime`.

---

Published from tag `broadcast-client-v0.6.0`.
