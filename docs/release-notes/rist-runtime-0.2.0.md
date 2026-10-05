# rist-runtime 0.2.0

_Released 2026-10-05._

Breaking (0.x minor) release that combines a security fix for the ARQ engine with the de-hand-roll wave's compound-packet rework. **Upgrade if you run RIST senders or receivers exposed to peers you do not control.** Three source-level breaks: the sender's NACK handlers take the current time (and `&mut self`); the two compound types carry their leading report as a `ReportPart` instead of separate `sr`/`rr` fields; and serializers now return an error instead of truncating. The previous published version is 0.1.1. Read together with [rtcp-packet-0.4.0.md](rtcp-packet-0.4.0.md), whose padding handling this crate now builds on.

## Security

- **GHSA-w4f3-953h-6jfm**: the NACK lookup fix in 0.1.1 bounded lookup cost but not response size. One NACK could still trigger an unbounded set of retransmissions: duplicates weren't removed and nothing was rate limited. A single 76-byte NACK could produce about 210 MB of retransmission traffic.
- **GHSA-q5vp-jg67-2xp9**: the 0.1.1 forward-gap limit was a fixed 512 packets. Legitimate burst losses the receive buffer could recover were never requested again, and a single forged far-ahead packet reset the whole stream, discarding tracked loss and buffered out-of-order packets.

## Breaking changes

### 1. `arq::Sender` NACK handlers take the clock and `&mut self`

```rust
// 0.1.x
let retransmits = sender.on_range_nack(&nack);
let retransmits = sender.on_generic_nack(&nack);
// 0.2.0
let retransmits = sender.on_range_nack(&nack, now);
let retransmits = sender.on_generic_nack(&nack, now);
```

`now` is a `core::time::Duration` on the same clock you already pass to the rest of the engine. The sender keeps a per-sequence-number last-retransmission time, which is why `&self` became `&mut self`.

### 2. Compound packets carry `ReportPart`, not `sr` / `rr` (#1086, RIST-W3)

`RistSenderCompound.sr` and `RistReceiverCompound.rr` are replaced by `report: ReportPart`, a new public `#[non_exhaustive]` enum with variants `Sr(SenderReport)` and `Rr(ReceiverReport)`, plus `ssrc()` and `name()`. A sender compound may carry either variant (SR, or an empty RR per TR-06-1 §5.2.3); a receiver compound always carries `Rr`. `ReportPart` and `UnknownPacket` are re-exported at the crate root.

```rust
// 0.1.x
let ssrc = compound.sr.ssrc;
// 0.2.0
let ssrc = compound.report.ssrc();            // either variant
if let ReportPart::Sr(sr) = &compound.report { /* sender-report fields */ }
```

Constructing either compound by struct literal now also needs the new fields below (the structs are not `#[non_exhaustive]`).

### 3. Padding and unmodelled sub-packets are preserved (#1086, RIST-W5 and RIST-W6)

Both compound types gain fields, so struct-literal construction must set them:

- `report_padding: Vec<u8>`: the RFC 3550 §6.4.1 P-bit padding of the leading report (excluding the trailing count byte, which is re-derived on serialize). Empty when the P bit was clear.
- `unknown: Vec<UnknownPacket>`: sub-packets with payload types the crate does not model, duplicate SDES, and modelled sub-packets that carry padding. Each `UnknownPacket` holds `packet_type`, `count`, `payload`, `padding` and `position` (its wire position, `None` for a hand-built one, which is appended after parsed ones). A re-serialized compound is byte-identical to the parse input. Previously such sub-packets were dropped or misclassified.

Use `unknown: Vec::new()` and `report_padding: Vec::new()` when building a compound by hand.

### 4. Serializers return errors instead of truncating (#1129)

`GenericNack`, `RangeNack` and `RttEcho` serialization write the RTCP 16-bit `length` field through a checked helper. A packet whose word count does not fit now returns the new `Error::FieldOverflow` (wrapping `broadcast_common::len::FieldOverflow`) instead of emitting a misframed packet with `Ok`. A `GenericNack` with 65 534 FCI entries is the smallest case that overflows; 65 533 still round-trips. New `Error::InvalidPaddingCount { count, body }` is returned when a P-bit padding count is zero or larger than the sub-packet body. `Error` is `#[non_exhaustive]`, so existing matches already carry a wildcard arm; add explicit arms if you want to handle these two.

## Behaviour changes

- **Sender**: the sequence numbers one NACK requests are de-duplicated, capped at the buffered-packet count, and a sequence number is not retransmitted again within `MIN_RETRANSMIT_INTERVAL` (20 ms, an internal constant).
- **Receiver gap limit** comes from the configured receive buffer instead of a fixed 512 packets. A jump beyond it resynchronises only after `RESYNC_CONFIRM_COUNT` (3) consecutive packets continue from the new position; an unconfirmed one-off jump is dropped as an outlier.
- **`Sender::new(max_buffered)`** clamps `max_buffered` to at most 32 767 (strictly below half the 16-bit sequence space) and at least 1. A larger value used to be accepted.
- `arq::seq` arithmetic now delegates to `broadcast_common::seq::SeqSpace`; public functions and results are unchanged (#1141).

## Fixes

- **#1108 (RIST-W1)**: with `max_buffered >= 32768`, after the sequence number wrapped, a reused sequence number overwrote the older generation's buffer entry in place and bypassed eviction, so the eviction queue grew by one duplicate entry on every send, forever. Fixed by the clamp above.
- **#1108 (RIST-W2)**: `Receiver::tick` ordered due retransmission requests numerically, not circularly. Across a 16-bit wrap the newest losses sorted before older, more urgent ones, so the per-tick `MAX_RANGE_ENTRIES` cap dropped the oldest losses and they aged out unrequested. Now ordered by circular distance from the next expected sequence number.
- **#1129**: silent truncation of the RTCP `length` field, see breaking change 4. `RangeNack`'s own cast was provably bounded by its `MAX_RANGE_ENTRIES` check, so it was not a live bug and was only switched to the checked helper for consistency (#1108, RIST-W4).

## Dependencies

Verified from the `Cargo.toml` diff against `rist-runtime-v0.1.1`:

```toml
broadcast-common = { version = "9.3" -> "9.4", default-features = false }
rtcp-packet      = { version = "0.3" -> "0.4", default-features = false }
```

Moving to `rtcp-packet` 0.4 is a different caret epoch for 0.x, so a project that also depends on `rtcp-packet` directly must move to 0.4 as well to share types such as `SenderReport`.

MSRV 1.95.0.

---

Published from tag `rist-runtime-v0.2.0`.
