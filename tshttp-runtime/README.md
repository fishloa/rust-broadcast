# tshttp-runtime

Sans-IO TS-over-HTTP pull session client.

The host performs all I/O; this crate owns the session state machine, demuxing
via `transmux::StreamingTsDemux`, backoff and stall detection.

## What it does / what it does not

Does: run the session state machine of a TS-over-HTTP pull (open, connect, play, stall detection, reconnect with backoff, 404/410 handling, finite-resource completion), and demux the body with `transmux::StreamingTsDemux`. The host supplies the clock (a monotonic millisecond count) and performs every HTTP request.

Does not: open sockets, parse HTTP, read a clock, decrypt, or play HLS/DASH (see `hls-runtime` and `dash-runtime`). `broadcast-client` drives this crate for its TS-over-HTTP source.

## Standards supported

| Standard | Scope | Status |
|---|---|---|
| ISO/IEC 13818-1 (MPEG-2 Systems) | Transport stream demuxing, via `transmux` | Delegated |
| RFC 9110 `Retry-After` | Server-requested reconnect delay (`set_retry_after`) | Implemented |

## Quick start

```no_run
use tshttp_runtime::{Config, TsHttpClient};

let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
// Drive I/O from the host:
let _ = client.poll();                 // -> Open
client.on_response(200, &[], 0);            // -> Connected
client.feed(&[0u8; 188], 1);           // body bytes
client.poll_timeout(5_000);            // timers
while let Some(ev) = client.poll_event() {
    let _ = ev;
}
if let Some(a) = client.poll() {
    let _ = a;
}
client.on_eof(2);
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

`#![no_std]` + `alloc`; time is a caller-supplied monotonic millisecond count.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links the standard library (`broadcast-common/std`, `transmux/std`). |
| `serde` | no | `Serialize`/`Deserialize` on the public event and config types. |

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

`transmux` (the streaming TS demuxer this crate wraps), `broadcast-client` (the session that drives it), `hls-runtime` and `dash-runtime` (the other pull sources).

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
