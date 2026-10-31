# tshttp-runtime 0.1.0

_Released 2026-10-31._

Initial release. `tshttp-runtime` is a **sans-IO TS-over-HTTP pull session client**: the policy a player needs to keep a live MPEG-TS HTTP stream running, with none of the I/O. The host (URLSession, tokio, anything) opens the connection and feeds back what happened; the crate decides when to open, wait, abort, reconnect and tell the user, and demuxes the bytes through `transmux::StreamingTsDemux`. It reads no clock (time is a caller-supplied monotonic millisecond count), spawns nothing and is `no_std` + `alloc`. The requirements and defaults come from the zenith-tv player (shipped on Apple TV); the policy is implementation-defined, not specified by any standard, and the full transition table is in `docs/tshttp-runtime/DESIGN.md`. It is the TS-over-HTTP source underneath `broadcast-client`.

## Quick start

```rust
use tshttp_runtime::{Config, TsHttpClient};

let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
let _ = client.poll();                 // -> Open
client.on_response(200, &[], 0);       // -> Connected
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

This is the README snippet (a `no_run` doctest). `examples/live_loop.rs` replays a committed TS capture through a client, including a caller-initiated reconnect, and `examples/stall_demo.rs` shows the stall notice and reconnect timers; both run with no arguments.

## Surface

| Piece | Provided |
|---|---|
| `TsHttpClient` | `poll`, `on_response`, `feed`, `on_eof`, `on_error`, `reconnect`, `set_retry_after`, `poll_timeout`, `next_deadline`, `poll_event`: the policy core plus a `StreamingTsDemux` |
| `TsHttpPolicy` | The demux-free core for a host that demuxes elsewhere: `on_response`, `on_bytes(len, now, sampled)`, `on_sample`, `on_eof`, `on_error`, `reconnect`, `set_retry_after`, `poll_timeout`; the same `Action`/`Event` stream, never `Event::Demux`. `tests/policy_core.rs` drives both over a real fixture and requires identical decisions |
| `Action` | `Open { url }`, `Reconnect { after_ms }`, `Abort` |
| `Event` | `Demux(DemuxEvent)`, `StallNotice { secs }`, `Starved(StarveKind::{Data, NoData})`, `NotFound { status }`, `Connected`, `Finite { expected_bytes }`, `Ended`, `Reconnecting { attempt }` |
| `Config` | `start_timeout_ms` 15 000, `stall_notice_ms` 5 000, `stall_reconnect_ms` 30 000, `backoff_ms` `[2000, 4000, 8000]`, `backoff_cap_ms` 10 000, `min_reconnect_interval_ms` 1 000; `validate()` / `ConfigError`, `clamped()` |
| Features | `std` (default), `serde` (`Event` serializes only) |

## Behaviour guarantees

- **Live streams keep playing.** A connection that played and then ended or failed reconnects immediately (attempt counter reset); a connection that fails before its first sample walks the backoff schedule, then the cap. `reconnect()` is host-initiated and takes effect with 0 ms wait.
- **No reconnect spin.** A server that sends one sample and closes cannot make the client loop: `min_reconnect_interval_ms` spaces opens after a short played-then-ended connection, while a long-lived stream still reconnects with no delay. A zero backoff entry is rejected by `Config::validate` and raised to 1 ms by `clamped()`, which `TsHttpClient::new` applies.
- **Dead connections are detected.** The start timer runs from `Open`, so a server that accepts but never sends headers is aborted after `start_timeout_ms`; no first sample within it aborts too. A stall notice follows `stall_notice_ms` without bytes, repeating once per second, and `stall_reconnect_ms` aborts and reconnects.
- **Starvation is typed.** `Starved(Data)` (bytes arriving, no sample) and `Starved(NoData)` (no bytes) are distinct, once per episode.
- **A server can slow the client down.** `set_retry_after(ms)` raises the next reconnect wait after a failed response to at least that long (a 429/503 `Retry-After`; `broadcast-client` converts the header, caps it at 60 s and reports it as `Event::ServerRefused`). It is consumed by that failure and cleared by a successful response.
- **404/410 are reported** as `NotFound { status }` and then retried on the backoff schedule.
- **Finite resources end.** A `2xx` with `Content-Length` (or a `206` whose `Content-Range` is consistent) emits `Finite { expected_bytes }` and, once all bytes arrive and the body ends, `Ended`; a truncated transfer is a failure, not an end. A `206` whose last byte is not below the total, or any `Content-Encoding` other than identity, is treated as live.
- **Deadlines are honest.** `next_deadline` never returns a time in the past, so a host that sleeps until it does not busy-loop.

## Known limits

- Playback-side decisions (decoder failures, buffering depth) are not in this crate; the host calls `reconnect()` for a decoder failure, and `buffering-runtime` and `broadcast-client` own buffering.
- A TS stream that never produces a sample is retried forever on the backoff schedule; the host decides when to give up.
- The policy is tuned to the zenith-tv requirements; it is not derived from a specification.

## Validation

Named HTTP statuses and header names are constants in the policy core (the `http` crate needs `std`, this crate is `no_std`). Public state-machine tests for every transition of the design table (`tests/transitions.rs`, `timers.rs`, `vod.rs`, `policy.rs`), audit regressions (`tests/audit_*`), a real-capture run over the committed `ts/h264_aac.ts` fixture (`tests/vod_real.rs`), a run over a live TV capture (`tests/real_stream.rs`) held in the private fixtures repository (CI checks it out with a read-only deploy key and fails on a missing fixture; a source checkout without it skips loudly), the policy-core equivalence test, and the label-coverage, `#[non_exhaustive]` and no-hand-roll guards.

## Dependencies

`broadcast-common` 10.0 (no default features), `transmux` 0.28 (no default features; the version of this release wave), optional `serde`.

## Migration

New crate; nothing to migrate. `tshttp-runtime = "0.1"`.

---

Published from tag `tshttp-runtime-v0.1.0`.
