Sans-IO TS-over-HTTP pull session client (module `broadcast_client::tshttp`, the former `tshttp-runtime` crate).

The host performs all I/O; this module owns the session state machine, demuxing
via `transmux::StreamingTsDemux`, backoff and stall detection.

## What it does / what it does not

Does: run the session state machine of a TS-over-HTTP pull (open, connect, play, stall detection, reconnect with backoff, 404/410 handling, finite-resource completion), and demux the body with `transmux::StreamingTsDemux`. The host supplies the clock (a monotonic millisecond count) and performs every HTTP request.

Does not: open sockets, parse HTTP, read a clock, decrypt, or play HLS/DASH (see `hls-runtime` and `dash-runtime`). `Session` drives it for the TS-over-HTTP source; `multimux` and the RTMP source use the demux-free `TsHttpPolicy` core directly.

## Standards supported

| Standard | Scope | Status |
|---|---|---|
| ISO/IEC 13818-1 (MPEG-2 Systems) | Transport stream demuxing, via `transmux` | Delegated |
| RFC 9110 `Retry-After` | Server-requested reconnect delay (`set_retry_after`) | Implemented |

## Quick start

```no_run
use broadcast_client::tshttp::{Config, TsHttpClient};
use broadcast_common::stage::Timestamp;

// The policy's own clock is milliseconds; `Timestamp` is microseconds.
let at = |ms: u64| Timestamp::from_millis(ms);
let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), at(0));
// Drive I/O from the host:
let _ = client.poll();                 // -> Open
client.on_response(200, &[], at(0));   // -> Connected
client.feed(&[0u8; 188], at(1));       // body bytes
client.poll_timeout(at(5_000));        // timers
while let Some(ev) = client.poll_event() {
    let _ = ev;
}
if let Some(a) = client.poll() {
    let _ = a;
}
client.on_eof(at(2));
```

## What is implemented

The `TsHttpClient` session and the demux-free `TsHttpPolicy` core, `Config` with its defaults, the 20-row transition table and the `Action`/`Event` stream.

## Policy core

`TsHttpClient` is `TsHttpPolicy` plus a `StreamingTsDemux`. `TsHttpPolicy` is the
demux-free core: it sees response headers, chunk lengths, a "sample produced"
flag and timestamps (never TS bytes) and implements the start/stall timers,
finite/VOD detection and backoff. A host that demuxes elsewhere (multimux uses
its own `ts_program`) drives the policy alone with `on_response`, `on_bytes`,
`on_sample`, `on_eof`, `on_error`, `poll_timeout` and reads the same
`Action`/`Event` stream (it never emits `Event::Demux`).

## Defaults

`Config::default()`: start_timeout_ms 15000, stall_notice_ms 5000,
stall_reconnect_ms 30000, backoff_ms `[2000,4000,8000]`, backoff_cap_ms 10000, min_reconnect_interval_ms 1000.

## no_std

The module uses only `core` and `alloc` and time is a caller-supplied monotonic millisecond count, but it is part of the `std` crate `broadcast-client`: the former standalone `no_std` build of `tshttp-runtime` (and its `thumbv7em-none-eabi` CI check) no longer exists. Nothing in this workspace built it without `std`.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `serde` | no | `broadcast-client`'s `serde` feature adds `Serialize`/`Deserialize` on this module's public event and config types. |

## Transitions

The full table is [`docs/tshttp-runtime/DESIGN.md`](https://github.com/fishloa/rust-broadcast/blob/main/docs/tshttp-runtime/DESIGN.md):
T1 Idle->Open, T2 2xx->Connected/Awaiting, T3 404/410->NotFound+backoff,
T4 other/error->backoff, T5 first Sample->Playing, T6 start timeout->Abort+backoff,
T7 eof/error before sample->backoff, T8 playing eof/error->Reconnect 0, T9 failure backoff,
T10 backoff due->Open, T11 stall notice, T12 stall reconnect, T13 reconnect(), T14 Starved(Data),
T15 Starved(NoData), T16-T19 finite resources (`Finite`, completion, `Ended`, truncated transfer),
T20 start timeout while Opening (no response headers).

## Server-requested delay

`set_retry_after(ms)` raises the next reconnect wait after a failed response to at least the
server's `Retry-After`; it is consumed by that failure and cleared by a successful response.

## Design notes

The full state table and the rules behind each transition are in [`docs/tshttp-runtime/DESIGN.md`](https://github.com/fishloa/rust-broadcast/blob/main/docs/tshttp-runtime/DESIGN.md).

## Related crates

`transmux` (the streaming TS demuxer this module wraps), `hls-runtime` and `dash-runtime` (the other pull sources). Pre-merge history: `docs/history/tshttp-runtime/CHANGELOG.md`.
