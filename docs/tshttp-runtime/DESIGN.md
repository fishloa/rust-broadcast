# tshttp-runtime — sans-IO TS-over-HTTP pull session (issue #1451)

Client-side bytes-in / actions-out session, same shape as `hls-runtime`'s `HlsClient`. The host
(Swift `URLSession`, tokio, anything) does only I/O. Requirements and defaults come from the
zenith-tv player (shipped and tested on Apple TV); the policy is spec-less, implementation-defined.

## Contract

Crate `tshttp-runtime`, `no_std`+`alloc`, depends on `transmux` (`StreamingTsDemux`) and
`broadcast-common`. Time is a caller-supplied monotonic `u64` millisecond count (`now_ms`); no
wall clock, no `std::time`.

```
Config { start_timeout_ms: 15_000, stall_notice_ms: 5_000, stall_reconnect_ms: 30_000,
         backoff_ms: [2_000, 4_000, 8_000], backoff_cap_ms: 10_000 }

TsHttpClient::new(url, Config, now_ms)
  poll() -> Option<Action>            Action::{ Open{url}, Reconnect{after_ms}, Abort }
  on_response(status: u16, now_ms)    response headers received
  feed(chunk: &[u8], now_ms)          body bytes
  on_eof(now_ms)                      body ended / connection closed cleanly
  on_error(now_ms)                    connect/read failure
  reconnect(now_ms)                   caller-initiated (decoder failures live in the player)
  poll_timeout(now_ms)                advance timers; may queue Open/Abort/Reconnect/events
  next_deadline() -> Option<u64>      earliest now_ms at which poll_timeout does something
  poll_event() -> Option<Event>
Event::{ Demux(DemuxEvent), StallNotice{secs}, Starved(StarveKind::{Data,NoData}),
         NotFound{status}, Connected, Reconnecting{attempt} }
```

`Reconnect{after_ms}` tells the host a wait is scheduled; the client queues the following
`Open` itself once `poll_timeout` passes the due time. `Abort` is queued when the client ends a
live connection (start/stall timeout, caller `reconnect`); the host must drop the connection and
ignore any later callbacks for it (callbacks while not Connected are ignored).

## States and transitions

| # | From | Input | Action / event | To |
|---|------|-------|----------------|----|
| T1 | Idle | new() | Open | Opening |
| T2 | Opening | on_response 2xx | Event Connected | Awaiting (waiting for first sample) |
| T3 | Opening | on_response 404 or 410 | Event NotFound{status}, then as T9 | Backoff |
| T4 | Opening | on_response other non-2xx / on_error | as T9 | Backoff |
| T5 | Awaiting | first Sample emitted | none | Playing (attempt counter reset) |
| T6 | Awaiting | now - open_at >= start_timeout | Abort, as T9 | Backoff |
| T7 | Awaiting | on_eof / on_error before any sample | as T9 | Backoff |
| T8 | Playing | on_eof / on_error | Reconnect{after_ms:0}, attempt = 0 | Backoff(0 ms) |
| T9 | any failure before playing | - | attempt += 1; Reconnect{after_ms: backoff[attempt-1] else cap}; Event Reconnecting | Backoff |
| T10 | Backoff | now >= due | Open | Opening |
| T11 | Playing | no bytes for stall_notice | Event StallNotice{secs} (repeats once per second after the first notice) | Playing |
| T12 | Playing | no bytes for stall_reconnect | Abort, Reconnect{after_ms:0} (counts as played-then-ended: attempt = 0) | Backoff(0 ms) |
| T13 | any connected | reconnect() | Abort, Reconnect{after_ms:0}, attempt = 0 | Backoff(0 ms) |
| T14 | Playing/Awaiting | bytes arrive, no sample for stall_notice | Event Starved(Data) once per episode | same |
| T15 | Playing | no bytes for stall_notice | Event Starved(NoData) once per episode | same |

Rules: the start timer restarts on every Open. A "stall" is measured from the last received byte
(or from Connected before any byte). Starved(Data) = bytes flowing but the demuxer emitted no
sample for stall_notice; Starved(NoData) = no bytes. Live never gives up: the only terminal input is
the host dropping the client. NotFound keeps retrying at the same backoff (cap 10 s) and the event is
re-emitted on each 404/410 so the UI can keep saying "channel not found". A server that closes the
stream every 60 s produces T8 each time with 0 ms reconnect and attempt reset, and never a
StallNotice.

## Demux

`feed` forwards bytes to `transmux::StreamingTsDemux`; every `DemuxEvent` is surfaced as
`Event::Demux` in order (TrackAdded/Updated/Removed, Sample, DisplayChanged, ...). A new
connection starts a fresh demuxer (continuity of timestamps across reconnects is the
player's concern; a `Event::Reconnecting` precedes it). Sample emission is what moves Awaiting to
Playing.

## Fixtures

Real: `private/fixtures/zenith-http/tf1-live.ts` (TF1 off-air, broadcaster copyright, private only;
PROVENANCE.md carries the capture text), `tf1.headers.txt`, `missing.headers.txt`,
`missing.body.txt` (zenith's own output, 404 `text/plain`). Public CC BY alternative:
zenith-player-demo TS files. Tests load them only through `test_fixtures`.
