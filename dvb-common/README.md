# dvb_common

Shared primitives for the DVB crate family:

- `Parse<'a>` and `Serialize` traits with an associated `Error` type.
- `crc32_mpeg2` — CRC-32 per ETSI EN 300 468 Annex C / ETSI TS 102 773 Annex A.

Consumed by `dvb_si`, `dvb_t2mi`, `dvb_bbframe`. Zero runtime dependencies.
Publishable to crates.io under MIT OR Apache-2.0.

## Non-goals

- A shared error enum. Each DVB crate owns a domain-specific `Error`; the shared traits keep it that way via `type Error`.
- CRC-8 (used only by `dvb_bbframe`). Lives in the consumer.
- Anything with a dependency. If a helper needs `bytes` / `chrono` / `serde`, it belongs in a consumer with the matching feature flag.
