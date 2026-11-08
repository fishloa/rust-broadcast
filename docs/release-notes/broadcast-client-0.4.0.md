# broadcast-client 0.4.0

_Released 2026-11-08._

Breaking-class release (0.3 -> 0.4) because `rtsp-runtime` moves to 0.10, which changes the caret epoch this crate builds against. The one new feature is an opt-in recording of the buffering controller's inputs for offline replay; nothing changes for a caller that does not turn it on.

Version: 0.3.0 -> 0.4.0.

## Added

An opt-in input log. With `Config::record_buffer_inputs` set (default off; nothing is recorded or allocated when off), the session records every call it makes into its buffering controller, so a capture from a device can be replayed offline through `buffering-runtime`.

- `Config::record_buffer_inputs` and `Config::max_recorded_buffer_inputs` (default 1 000 000; must be non-zero, otherwise `Session::open` returns `Error::InvalidConfig`).
- `Session::drain_buffer_inputs()` yields the recorded `buffering_runtime::input_log::Input`s; `Session::buffer_input_log_config()` and `Session::buffer_input_log_seed()` return the controller configuration and stall-memory seed to write beside them; `Config::controller_config()` is the exact `ControllerConfig` the session's controller is built from.
- When the queue is full, new inputs are dropped and counted by `Session::dropped_buffer_inputs()`, and one non-fatal `ErrorKind::BufferInputsDropped` is emitted per session. A log with drops no longer replays faithfully.

```rust
use buffering_runtime::input_log::InputLog;

let mut config = Config::default();
config.record_buffer_inputs = true;
let mut session = Session::open(url, config, now_us)?;
// ... drive the session ...
let log = InputLog::new(
    session.buffer_input_log_config(),
    session.buffer_input_log_seed(),
    session.drain_buffer_inputs().collect(),
);
let mut text = String::new();
log.write(&mut text)?;
std::fs::write("capture.csv", text)?;
// offline: InputLog::parse(&text)?.replay()?
```

Drain regularly on a long run; the queue holds at most `max_recorded_buffer_inputs` rows.

## Changed

- `BufferInput` was a `doc(hidden)` type behind the `__test-seams` feature. It is now a re-export of the public `buffering_runtime::input_log::Input`, and its `Register` and `Unregister` variants are struct variants carrying `now_us` and `track`. This is not a public break, because the type was never public.

## Dependencies

Workspace sibling requirements move to the new epochs of this release:

- `rtsp-runtime` 0.9 -> 0.10
- `buffering-runtime` requirement raised to 0.2.1 (the `input_log` module); same caret line, no epoch move

`rtsp-runtime::Error` now carries its source errors, prints only its own context and no longer derives `Clone`, `PartialEq` or `Eq` (#1726; see the rtsp-runtime 0.10.0 note). `broadcast-client`'s own `Error` does not expose it, so there is no migration here. A project that names `broadcast-client` and `rtsp-runtime` together must move both.

---

Published from tag `broadcast-client-v0.4.0`.
