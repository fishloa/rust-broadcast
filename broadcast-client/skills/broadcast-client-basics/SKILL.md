---
name: broadcast-client-basics
description: Use when writing Rust code against the broadcast-client crate to pull a live media stream (TS over HTTP, HLS, DASH, Smooth, RTSP, RTMP, UDP/RTP or SRT) through its sans-IO `Session`, driving the host loop that performs each `Action` and feeds the result back.
---

# broadcast-client basics

`broadcast-client` is one sans-IO client **`Session`** over every source kind:
TS over HTTP, HLS, DASH, Smooth Streaming, RTMP, RTSP, UDP, RTP and SRT. Bytes
go in, `Action`s come out, and time is a caller-supplied monotonic `now_us`.

## Scope

**It does:** source selection and every protocol state machine behind it,
auth (Basic/Digest/Bearer, mutual TLS), redirects, cookies, retry/backoff,
buffering and join decisions, demuxing to `transmux` `Sample`s.

**It does not:** open sockets, run TLS, spawn threads or read a clock. The host
performs each `Action` and reports the outcome back. A `Session` is `std`-only.

## The host loop

The three rules of the contract: drain `next_event()` after **every** callback
(not only after `poll`), honour `Action::WakeAt`, and report `conn` ids
verbatim.

```rust
use broadcast_client::{Config, Session};

let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
while let Some(action) = session.poll(0) {
    // host performs I/O based on `action`
    let _ = action;
}
```
(`broadcast-client/README.md`, "Quick start")

A host that answers one request with a canned body, then drains what the
session learned:

```rust
let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
let mut opens = 0;
let mut samples = 0;
let mut events = 0;
while let Some(action) = session.poll(0) {
    match action {
        broadcast_client::Action::HttpRequest { conn, .. } => {
            opens += 1;
            let bytes = test_fixtures::read("ts/h264-aac.ts");
            let bytes = &bytes[..];
            session.on_response(conn, 200, &[], 0);
            session.on_data(conn, bytes, 0);
        }
        broadcast_client::Action::Close { .. } => {}
        broadcast_client::Action::WakeAt(_) => {}
        _ => {}
    }
}
while let Some(e) = session.next_event() {
    events += 1;
    if matches!(e, broadcast_client::Event::Sample { .. }) {
        samples += 1;
    }
}
```
(`broadcast-client/examples/session_loop.rs`; the `test_fixtures::read` line is
the example's fixture loader, replace it with your own body read)

The callback set:

| Callback | When |
|---|---|
| `Session::on_response(conn, status, &headers, now_us)` | response head arrived (host clock) |
| `Session::on_data(conn, bytes, now_us)` | body bytes (or one datagram) |
| `Session::on_closed(conn, Option<TransportError>, now_us)` | connection ended or failed |
| `Session::on_connected(conn)` | a `TcpConnect`/`UdpBind` succeeded |
| `Session::on_playback_position(now_us, Some(pts_27mhz))` | every display tick (live TS `Start` depends on it) |
| `Session::set_credentials(Credentials)` | when the host decides to |
| `Session::reconnect(now_us)` | when the host policy decides to restart the source |

## Config

`Config` is `#[non_exhaustive]` and all public fields: build it with
`Config::default()` (or `Config::relay()` for a relay that forwards samples
instead of playing them) and then **mutate fields**. There is no `with_*`
builder for most of it, and a struct literal does not compile.

```rust
fn config_with(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config::default();
    f(&mut c);
    c
}

let config = config_with(|cfg| {
    cfg.udp.interface = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
    cfg.udp.source = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)));
    cfg.udp.recv_buffer_bytes = Some(4 << 20);
});
let mut s = Session::open(GROUP, config, 0).unwrap();
```
(`broadcast-client/tests/it/udp.rs`, `config_with` + `a_multicast_url_asks_the_host_to_bind_and_join_the_group`)

`Session::open` validates the config and returns `Error::InvalidConfig { field }`
for a value that would silently disable output (a zero `max_queued_events`,
`max_queued_bytes`, `request_timeout_us`, `segment_timeout_us`, a bad `dash`
field, ...). `ts_http` values are clamped rather than rejected.

The one builder pair is `with_timeouts`: set `kind` first, then map one
`broadcast_common::timeouts::Timeouts` onto the protocol's own bounds.

```rust
fn with(kind: SourceKind, timeouts: Timeouts) -> Config {
    let mut config = Config::relay();
    config.kind = kind;
    config.with_timeouts(timeouts)
}

fn timeouts(connect_ms: u64, read_ms: u64) -> Timeouts {
    Timeouts::default()
        .with_connect(Duration::from_millis(connect_ms))
        .with_read_idle(Duration::from_millis(read_ms))
}
```
(`broadcast-client/tests/it/config_timeouts.rs`)

Per protocol: HLS takes `read_idle` for playlist/segment/key requests; DASH and
Smooth for manifest and segment requests; SRT takes `connect` as the handshake
bound; every other kind maps `connect`/`read_idle` onto the TS stall policy.
Every bound is floored at 1 ms.

## Sources

`Config::kind` (`SourceKind`) selects the source. `SourceKind::Auto` (the
default) reads the URL: `udp://` → `Udp`, `rtp://` → `Rtp`, `rtsp://`/`rtsps://`
→ `Rtsp`, `rtmp://`/`rtmps://` → `Rtmp`, `srt://` → `Srt`, a `.m3u8`/`.m3u`
path → `Hls`, a `.mpd` path → `Dash`, a last path segment `Manifest` → `Smooth`,
anything else → TS over HTTP. An explicit kind wins over the extension (a DASH
MPD served from `/live.ts` needs `SourceKind::Dash`).

Each kind asks for a different action: HTTP kinds get `Action::HttpRequest`
(pass `method` through, `byte_range` is `(offset, length)`), RTSP gets
`Action::TcpConnect` + `Action::Send`, UDP/RTP/SRT get `Action::UdpBind` and
`Action::Send`/`on_data` per datagram.

An SRT session **needs** an entropy seed from the host's CSPRNG - the session
reads no randomness, and opening without one fails with
`Error::InvalidConfig { field: "srt.entropy_seed" }`.

```rust
cfg(|c| c.srt.entropy_seed = Some(SrtEntropySeed::new(seed)))
```
(`broadcast-client/tests/it/srt.rs`, `the_entropy_seed_is_required_and_every_attempt_derives_fresh_identifiers`)

## Auth, redirects, cookies, retry

Auth is credentials-in, events-out: answer `Event::AuthRequired` with
`Session::set_credentials`. A rejected set is `Error::InvalidCredentials`
(control characters) and keeps the previous credentials.

```rust
use broadcast_client::{Config, Credentials, Session};

let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
session
    .set_credentials(Credentials::bearer("secret-token"))
    .unwrap();
```
(`broadcast-client/examples/auth_wait.rs`)

- `Event::AuthRequired { resource_metadata, .. }` names a URL chosen by the
  server: validate it (https, expected host, no private/loopback address)
  before fetching it. The session never fetches it.
- Redirects are followed only to `http`/`https` and never `https` → `http`,
  at most `Config::max_redirects` times; a `401` from another origin is a
  connect failure, not a prompt.
- Cookies are host-only, scoped to the session origin, seeded with
  `Config::initial_cookies`, read back with `Session::cookies()`.
  `Config::extra_headers` go only to the origin; `Authorization`, `Cookie`,
  `Host`, `Range`, ... are `Error::ReservedHeader`.
- Retry/backoff is internal: HLS, DASH and Smooth segment failures walk
  `Config::backoff_ms`/`backoff_cap_ms` and give up after
  `Config::max_segment_failures`; TS, RTSP and SRT ride `Config::ts_http`'s
  start/stall policy. A `429`/`503` `Retry-After` stretches the wait (clamped
  to 60 s) and reports one `Event::ServerRefused { status, retry_after_us,
  reason }`. When retrying for another reason the host only sees
  `Status::Reconnecting`, not retry timing: poll at `Action::WakeAt` times and
  the session does the rest.

## Events and the queue

Events: `Tracks(Vec<TrackSpec>)` (always before that connection's samples),
`Sample { track, sample, join }`, `Discontinuity`, `Status`, `Starved`,
`Buffer(BufferEvent)`, `Finite`, `Error { kind, fatal }`, plus the auth,
variant and clock events.

`join` is `Some(JoinDecision::Drop(_))` before a mid-GOP join point, so skip
those samples; `JoinDecision::is_keep()` is the test.

```rust
if let Event::Sample { track, sample, join } = event {
    if join.is_none_or(|j| j.is_keep()) {
        deliver(track, &sample);
    }
}
```
(`Event::Sample` per `broadcast-client/src/events.rs`; `JoinDecision::is_keep`
in `transmux/src/live_join.rs`; `deliver` is your decoder/relay hand-off)

The queue is bounded by `Config::max_queued_events` (100 000) and
`Config::max_queued_bytes` (256 MiB). A host that does not drain loses the
oldest samples (counted by `Session::dropped_events()`, each burst replaced by
one `Event::Discontinuity`); tracks, errors and buffer events are never
dropped. Drain every callback, or the numbers lie.

A DASH live session needs a wall clock: call `Session::set_wall_clock(now_us,
unix_us)` between `open` and the first `poll` (static MPDs need nothing).

## Features

| Feature | Default | Effect |
|---|---|---|
| `serde` | no | `Serialize` on events and config, forwarded to `broadcast-common`, `transmux` and `buffering-runtime` |

There is no `std` feature: the crate is `std`-only. Do not look for a
`no_std` build of `Session`. (`broadcast-common` and most sources are
`no_std`; this crate is the adapter above them.)

## Pitfalls

- **Drain `next_event()` after every callback.** Actions and events are
  produced inside `on_response`/`on_data`/`on_closed`/`set_credentials`/
  `reconnect`, not only by `poll`. Polling without draining hits the queue cap
  and drops samples.
- **No playback ticks, no `Buffer::Start` on live TS.** `Start`/`Resume` are
  decided in `Session::on_playback_position(now_us, pos)`: call it each display
  tick, `pos` `None` before `Start`. The 27 MHz position it names
  (`at_27mhz`) is the sample-time domain.
- **`Action::byte_range` is `(offset, length)`**, not `(start, end)`; the host
  builds `Range: bytes=offset-(offset+length-1)`.
- **Pass `method` through verbatim.** It is `"GET"` or `"HEAD"` (a DASH
  `UTCTiming` issues `HEAD`; a `HEAD` response has no body to feed back). A
  resumed request carries its own `Range`/`If-Range` headers and a `None`
  `byte_range`.
- **One `on_data` per datagram** for UDP/RTP/SRT; a partial 188-byte packet or
  a non-sync byte is dropped and counted, not buffered.
- **`Action::Debug` hides header values and payloads**; do not rely on it to
  log a request. Credentials, SRT passphrases and seeds are redacted.
- **`Session::close()` is terminal** (one `Close` per connection, a `TEARDOWN`
  first for RTSP); callbacks for a closed connection are ignored. After a
  fatal `Event::Error` the session emits nothing further.
- **`reconnect()` does nothing after `Status::Ended` of HLS/DASH**; a finite
  TS source restarts.
- **`Session::open` performs no I/O.** Nothing happens until the first `poll`;
  call `set_wall_clock` (DASH live) and `set_credentials` between the two.
- **A live TS track set is held behind the first `Event::Tracks`,
  not per stream.** Build the pipeline from that event; a sample burst may
  follow it (bounded by `Config::tracks_hold_max_wall_us`, 1 s). Lower the hold
  for a tight first-frame budget, `0` to take tracks as they resolve.
- **`Config::with_timeouts` reads `kind`.** Set `kind` first, or the bounds land
  on the wrong protocol.
- **Wrong source kind = wrong protocol.** `SourceKind::Dash` on an `rtsp://` URL
  is `Error::UnsupportedScheme`; an MPD behind a `.ts` path is TS bytes until
  `kind` says otherwise.
- **`Config::max_segment_failures: 0` makes the first segment failure final**
  (`u32::MAX` retries forever), and `max_key_failures: 0` behaves as `1`. The
  defaults are 8.
- **Do not fetch a challenge's `resource_metadata` blindly** (SSRF): validate
  scheme, host and address class first.
