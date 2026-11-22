# broadcast-common 14.0.0

_Released 2026-11-22._

Major release (13.1 -> 14.0). The breaking change is `jiff` 0.2 -> 1.0 in the `jiff` and `http-date` public faces; the rest is additive (a shared bit buffer, a resume offset on the bit reader/writer, an exact-length hex decoder, URL-safe base64, and the `wire_enum!` macro that replaced three crate-local copies). Every publishable crate in the workspace depends on this one, so this release is the root of the 2026-11-22 wave.

## Breaking changes

- `jiff` 0.2 -> 1.0. The `jiff::Timestamp` faces of the `jiff` and `http-date` features (`time::decode_mjd_bcd_utc`, `encode_mjd_bcd_utc`, `encode_seconds_since_2000_utc`, `MonoUtc`, `http_date::parse_http_date`, `date_to_epoch_us`) are jiff 1.x types. Migration: depend on `jiff` 1.x; the constructors and accessors are spelled the same (`jiff::Timestamp::from_second`, `as_microsecond`, `jiff::SignedDuration::from_secs`). A `jiff` 0.2 item no longer unifies, so a consumer must move every crate that hands it one of these values in the same step.
- `base64::base64_decode_strict`, `base64_decode_lenient` and `base64_url_decode_nopad` return this crate's own `base64::Base64DecodeError` (a `thiserror` type with `name()`) instead of the `base64` crate's `DecodeError`, so a `base64` major bump can no longer break this crate's public API. Migration: name `Base64DecodeError` in the error position; the variants keep the information the previous type carried (invalid byte and offset, invalid length, invalid last symbol, padding).

## Added

- `hex::hex_decode_into`: exact-length hex decode into a caller buffer, with `hex::HexDecodeError` naming the length mismatch or the first non-digit's offset. The text is validated in full first, so the output is never left partly written.
- `base64::{base64_url_encode_nopad, base64_url_decode_nopad}`: RFC 4648 §5 URL-safe alphabet without padding.
- `bits::BitReader::starting_at` / `bits::BitWriter::starting_at` (resume at a bit offset) and `bits::BitBuf`, a growable owning MSB-first writer, so a caller with its own bit cursor uses the one implementation; `dvb-si`'s descriptor writer and `transmux`'s FLV/test writers were moved onto it.
- `wire_enum!`, an exported macro declaring a spec enum together with its checked `TryFrom` and `From<Self> for repr` from one variant table, taking the consumer's own error type. `dvb-si`, `dvb-t2mi` and `dvb-bbframe` dropped their three copies.

## Fixed

- `backoff::RetrySchedule` applies the drawn jitter term itself, not that term raised to the attempt index. Raising it collapsed every retry from the fifth attempt on to exactly 5/6 of the raw delay, so a fleet of clients retried on one shared instant during the long outage the cap exists for. The delay now stays inside `[raw * 5/6, raw]` with the sequence's own spread at every attempt (#2920).

## Changed

- `thiserror` is now an unconditional `no_std` dependency, and the shared error kinds plus `BitError`, `KlvError`, `InvalidBackoff`, `InterfaceError`, `ClockOutOfRange`, `InvalidSeqSpace` and `XmlCharError` derive `thiserror::Error`. Messages are unchanged; `InterfaceError` implements `core::error::Error` without `std` (#3962).
- README: `thiserror` is named as an unconditional dependency, and the `bits`, `hex` and `base64` rows list `BitBuf`/`starting_at`, `hex_decode_into` with `HexDecodeError`, and the URL-safe helpers with `Base64DecodeError` (#4013).

## Tests

The audit-21 boundary suites pin the BCD/calendar gates, `SeqSpace` wrap arithmetic, the XML reader's limits and namespace matching, the PUSI cap, the `Content-Range`, BER and BCD boundaries, the shared value types' formatting and conversions, and every error `Display` (#2921-#2926).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3993 (foreign error type re-exported), #3994 (`hex_decode_into` returns a typed error), #4013 (README rows), plus #3992's epoch truthfulness in the CHANGELOG.

---

Published from tag `broadcast-common-v14.0.0`.
