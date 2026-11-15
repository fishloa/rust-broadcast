# broadcast-common 13.0.0

_Released 2026-11-15._

Major release (12 -> 13). This is the wave where the workspace's shared time, clock and backoff vocabulary landed: instants are `stage::Timestamp`, wall clocks are `jiff::Timestamp`, media positions are validated `Clock27`/`Clock33` values, and every fallible construction reports a typed error instead of asserting. Because every crate in the workspace depends on `broadcast-common`, all of them move to a new caret epoch in this wave.

Version: 12.0.0 -> 13.0.0.

## Breaking changes and migration

### `stage::Timestamp` is microsecond-valued with a private field

`Timestamp(raw_micros)` is gone: the field is private, `as_micros()` reads it, and the type is built with `from_micros`/`from_millis`/`from_nanos` (which round toward the later instant). Its arithmetic splits by purpose: a *reading* clamps (`saturating_add`/`saturating_sub`), a *deadline* is `checked_add`, where `None` means "never fires".

```rust
// before
let now = Timestamp(1_700_000_000_000_000);
let deadline = Timestamp(u64::MAX);
// after
let now = Timestamp::from_micros(1_700_000_000_000_000);
let micros = now.as_micros();
let deadline: Option<Timestamp> = now.checked_add(Duration::from_secs(5)); // None: never fires
```

### `Timescale` constructors

`Timescale::new` (an `Option<Self>` beside `try_new`'s `Result` for the same job) is gone, and `from_nonzero` takes a `NonZeroU32` rather than a bare `u32`. A literal or a named constant goes through the exported `broadcast_common::timescale!` macro, whose check runs inside an inline `const { }` block, so a zero is a compile error with no runtime panic path.

```rust
use core::num::NonZeroU32;
// before
let ts = Timescale::from_nonzero(90_000);           // panicked on a runtime zero
let ts = Timescale::new(value)?;                    // Option
// after
let ts = broadcast_common::timescale!(90_000);      // literal
let ts = Timescale::from_nonzero(NonZeroU32::new(90_000).unwrap());
let ts = Timescale::try_new(value)?;                // runtime value
```

### `SeqSpace` constructors and `len::fit_bits`

`SeqSpace::new` asserted a runtime width, so a config value of `0` or above 31 panicked; it is gone. A literal width goes through `broadcast_common::seq_space!` (a compile-time check), a runtime width through `SeqSpace::try_new`, and a width already proven through `SeqSpace::from_valid_bits`. `len::fit_bits` no longer panics on a width of `0` or above 64: it returns the same `FieldOverflow` it returns for an over-wide value, so no public constructor in this crate panics.

```rust
// before
let space = SeqSpace::new(16);
// after
let space = broadcast_common::seq_space!(16);
let space = SeqSpace::try_new(bits)?;   // InvalidSeqSpace { bits } on a bad width
```

### `xml_chars` reports a typed error

`xml_chars::char_data`, `namespace_of` and `read_event` fail with `XmlCharError` (`BadChar`, `UndefinedEntity { name }`, `UnknownPrefix { prefix }`, `Reader { message }`) instead of a `String`, so a caller matches the failure kind; each variant's `Display` is the description the old `String` carried. `broadcast-dash` and `broadcast-smooth` gain a `From<XmlCharError>` for their own error enums.

```rust
// before
fn text(raw: &str) -> Result<String, String> { xml_chars::char_data(raw) }
// after
fn text(raw: &str) -> Result<String, broadcast_common::xml_chars::XmlCharError> {
    broadcast_common::xml_chars::char_data(raw)
}
```

### `net::UdpOptions::recv_buffer_bytes` is `Option<NonZeroU32>`

It was `Option<usize>`, so a zero-byte request could not be told apart from `None` (the kernel keeps its default either way); a zero is now unrepresentable.

```rust
// before
let options = UdpOptions::new().with_recv_buffer_bytes(4 * 1024 * 1024);
// after
let options = UdpOptions::new().with_recv_buffer_bytes(NonZeroU32::new(4 * 1024 * 1024).unwrap());
let bytes: Option<NonZeroU32> = options.recv_buffer_bytes;
```

### `chrono` feature replaced by `jiff`

`time::decode_mjd_bcd_utc`, `encode_mjd_bcd_utc`, `decode_seconds_since_2000_utc` and `encode_seconds_since_2000_utc` take and return `jiff::Timestamp` instead of `chrono::DateTime<Utc>`, with the same MJD + BCD semantics; `chrono` is no longer a dependency. `decode_seconds_since_2000_utc` returns `None` for `subsec_nanos` of a whole second or more (chrono accepted one or two seconds as a leap second). `time::mjd_to_ymd`/`ymd_to_mjd` no longer need a feature.

```toml
# before
broadcast-common = { version = "12", features = ["chrono"] }
# after
broadcast-common = { version = "13", features = ["jiff"] }
```

### `http_date` and `time::MonoUtc` carry `jiff` instants

`http_date::parse_http_date` and `date_to_epoch_us` return a `jiff::Timestamp` instead of Unix microseconds as a raw `i64`; `date_to_epoch_us` still refuses a `Date` before 1970 and reports a value outside the `jiff` range as `HttpDateError::HttpDate`. `time::MonoUtc` (behind the `jiff` feature, with its `WallClock`/`Anchor` aliases) holds typed instants instead of `mono_us`/`utc_us` fields, its mapping methods take and return those types, and the raw-count aliases `unix_at`/`mono_for_unix` are gone.

```rust
// before
let unix_us = parse_http_date(&value)?;
let utc = mono.utc_at(now_us);
// after
let unix_us = parse_http_date(&value)?.as_microsecond();
let utc = mono.utc_at(Timestamp::from_micros(now_us)); // -> jiff::Timestamp
let mono = MonoUtc::new(Timestamp::from_micros(m), jiff::Timestamp::from_microsecond(u)?);
```

### `backoff::RetrySchedule` is fallible

`RetrySchedule::new`, `seeded`, `from_steps` and `BackoffSteps::schedule` return `Result<_, InvalidBackoff>` instead of silently repairing a bad schedule: a cap under the floor, a floor above `MAX_FLOOR` and a growth factor under one or NaN are each their own variant (a factor of one, a flat series, is still accepted). A caller that relied on `max < min` being raised to `min` now gets `InvalidBackoff::CapBelowFloor` and must choose the cap it wants. `RetrySchedule::no_delay(seed)` is the schedule no bound can refuse.

```rust
// before
let schedule = RetrySchedule::new(Duration::from_millis(50), Duration::from_secs(30), 2.0, seed);
// after
let schedule = RetrySchedule::new(Duration::from_millis(50), Duration::from_secs(30), 2.0, seed)?;
let never = RetrySchedule::no_delay(seed);
```

### `backoff::ReconnectPolicy` replaces the hand-rolled reconnect loops

`ReconnectPolicy` is the workspace's one reconnect policy: a `BackoffSteps` schedule plus an `Option<NonZeroU32>` attempt bound, with `ReconnectPolicy::doubling` for an exponential schedule from two bounds and `ReconnectPolicy::exhausted` to read the bound. The runtimes' own reconnect timers and `Backoff` structs are gone in favour of it (see their notes).

```rust
// before: a per-crate timer struct
let policy = broadcast_client::ReconnectTimer::new(500, 30_000);
// after
use core::num::NonZeroU32;
let policy = ReconnectPolicy::doubling(Duration::from_millis(500), Duration::from_secs(30))
    .with_max_attempts(NonZeroU32::new(10).unwrap());
```

### `InvalidBackoff` is typed and no longer `Eq`

`InvalidBackoff::GrowthBelowOne { factor: f64 }` is `InvalidBackoff::GrowthFactor { factor: GrowthFactor }` (`GrowthFactor::Nan` or `GrowthFactor::Below(f64)`), because a NaN factor made the error unequal to its own copy. `InvalidBackoff::reason()` is gone: `Display` is the message, `core::error::Error` is still implemented, and `GrowthFactor::name()` carries the description. Match the `GrowthFactor` variants rather than the raw `f64`, and render the message with `to_string()`.

```rust
// before
if let InvalidBackoff::GrowthBelowOne { .. } = err { .. }
let text = err.reason();
// after
if let InvalidBackoff::GrowthFactor { factor: GrowthFactor::Below(_) } = err { .. }
let text = err.to_string();
```

## Removed

- `timescale::Timescale::new`, `seq::SeqSpace::new` and `stage::Timestamp`'s public field (see above).
- `backoff::InvalidBackoff::reason` and `backoff::InvalidBackoff::GrowthBelowOne`.
- `time::MonoUtc::unix_at` and `time::MonoUtc::mono_for_unix`.
- The `chrono` feature and the `chrono` dependency.

## Fixes

- `ber::encode_ber_length`/`encode_ber_length_as` write their length byte through `len::fit_u8` instead of clamping an over-range value into the wire, so an unrepresentable length is reported as a `KlvError` rather than silently changed.
- `xml::read_tree` rejects CDATA sections and non-XML-S whitespace outside the root element; `Elem::text` documents that it holds only direct text.

## Added

- `clock33::Clock33` and `clock33::Clock27` (with `ClockOutOfRange`): validated 33-bit 90 kHz and 27 MHz clock values, with strict construction (`from_parts`, `from_27mhz`, `from_27mhz_wrapping`) and lossless parsing (`from_wire_parts`, `to_clock27`, `is_valid`). `clock33::PTS_TIMESCALE` and `clock33::PCR_TIMESCALE` name the two clocks as `Timescale`s.
- `reserved::{Reserved, ReservedOnes, ReservedZero}`: the workspace's one typed run of reserved bits in a fixed-width field, moved here from `dvb-ci` (`dvb_ci::reserved` re-exports these names).
- `net::{UdpOptions, MulticastJoin, MulticastInterface}` (plus `UdpOptions::validate` and `JoinMismatch`): the one UDP bind/join option set, plain `core::net` data, `no_std`.
- `range::ByteSpan`: the one byte-range type (offset plus non-zero length), rendering the `Range: bytes=<first>-<last>` field value.
- `backoff::MAX_FLOOR`, `RetrySchedule::peek_delay_for_attempt`, `Timescale::NANOS`, `Timescale`'s `Display`/transparent `Debug`/`serde`, and `http_date::date_to_epoch_us` taking the typed `headers::Date`.

## Dependencies

- None. `broadcast-common` is the root of the graph and is published first, from tag `broadcast-common-v13.0.0`.

---

Published from tag `broadcast-common-v13.0.0`.
