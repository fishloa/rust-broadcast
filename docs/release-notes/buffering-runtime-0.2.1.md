# buffering-runtime 0.2.1

_Released 2026-11-08._

Patch release (0.2.0 -> 0.2.1): one additive module, `input_log`, that records what a host fed a `BufferController` and replays it offline. No existing item changes, so this stays in the 0.2 caret line and no dependent changes epoch.

Version: 0.2.0 -> 0.2.1.

## Added

The `input_log` module has one format and one implementation for writing, reading and replaying a controller's inputs, so a log captured on a device gives the same decisions on any host.

- `Input` is a typed, `#[non_exhaustive]` enum with one variant per controller call: `Chunk`, `TrackMedia`, `Tick`, `Register`, `Unregister` and `Hint`. `Input::apply` is the single place a recorded input becomes a controller call. `write_row` and `parse_row` are symmetric, and a bad row is reported as an `InputLogError` that carries the row and a `Reason`.
- `InputLog` holds the `ControllerConfig`, an optional stall-memory seed and the inputs. `write` and `parse` round-trip it; `parse_with_config` reads logs recorded before `config` rows existed (the Apple TV goldens), taking the configuration from the caller; `rebase` shifts times so the first input is at zero; `controller` builds a controller from the recorded configuration and seed; `replay` feeds every input through a fresh controller and returns each `Event` with the time of the input that produced it.
- The log is plain text with the header `kind,now_us,a,b`. The first rows after the header are one `config` row per `ControllerConfig` field (named by `ConfigKey`) and a `seed` row when a stall-memory seed was used; the rest are the inputs in call order: `chunk` (bytes, last PCR), `media` (track key, media end), `tick` (position flag, position), `register` and `unregister` (track key), and `hint` (the hint's `name()` token). Times are the host's monotonic microseconds; positions and media ends are 27 MHz ticks.
- New example `replay_input_log`: with no arguments it writes a synthetic two-track log, parses it back and replays it; with a path it replays that file, with `--initial-ms` replacing the cold-start target for logs that have no `config` rows.

```text
cargo run -p buffering-runtime --example replay_input_log
cargo run -p buffering-runtime --example replay_input_log -- capture.csv --initial-ms 1800
```

```rust
use buffering_runtime::input_log::InputLog;

let log = InputLog::parse(&std::fs::read_to_string("capture.csv")?)?;
for (at_us, event) in log.replay()? {
    println!("{at_us} {event:?}");
}
```

`broadcast-client` 0.4.0 produces these logs from a live session (see its note).

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `buffering-runtime-v0.2.1`.
