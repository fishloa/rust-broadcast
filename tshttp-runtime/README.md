# tshttp-runtime

Sans-IO TS-over-HTTP pull session client.

The host performs all I/O; this crate owns the session state machine, demuxing
via `transmux::StreamingTsDemux`, backoff and stall detection.

## Quick start

```no_run
use tshttp_runtime::{Config, TsHttpClient};

let mut client = TsHttpClient::new("http://example/live.ts", Config::default(), 0);
// Drive I/O from the host:
let _ = client.poll();                 // -> Open
client.on_response(200, 0);            // -> Connected
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

## Defaults

`Config::default()`: start_timeout_ms 15000, stall_notice_ms 5000,
stall_reconnect_ms 30000, backoff_ms `[2000,4000,8000]`, backoff_cap_ms 10000.

## no_std

`#![no_std]` + `alloc`; time is a caller-supplied monotonic millisecond count.

## Transitions

See DESIGN.md: T1 Idle->Open, T2 2xx->Connected/Awaiting, T3 404/410->NotFound+backoff,
T4 other/error->backoff, T5 first Sample->Playing, T6 start timeout->Abort+backoff,
T7 eof/error before sample->backoff, T8 playing eof/error->Reconnect 0, T9 failure backoff,
T10 backoff due->Open, T11 stall notice, T12 stall reconnect, T13 reconnect(), T14 Starved(Data),
T15 Starved(NoData).
