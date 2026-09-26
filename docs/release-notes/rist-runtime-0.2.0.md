# rist-runtime 0.2.0

Security release for the ARQ engine. **Upgrade if you run RIST senders or receivers exposed to
peers you do not control.** Breaking: the sender's NACK handlers take the current time.

## Security

- **GHSA-w4f3-953h-6jfm** — the NACK lookup fix in 0.1.1 was incomplete. One NACK could still
  trigger an unbounded set of retransmissions: duplicates weren't removed and nothing was rate
  limited. A single 76-byte NACK could produce about 210 MB of retransmission traffic.
- **GHSA-q5vp-jg67-2xp9** — the 0.1.1 gap limit (512 packets) was fixed and applied on one
  packet. Legitimate burst losses the receive buffer could recover were never requested again,
  and a single out-of-range packet could reset the stream.

## Breaking change

`arq::Sender::on_range_nack` and `on_generic_nack` now take `&mut self` and the current time:

```rust
// 0.1.x
let retransmits = sender.on_range_nack(&nack);
// 0.2.0
let retransmits = sender.on_range_nack(&nack, now);
```

`now` is a `Duration` on the same clock you already pass to the rest of the engine.

## Changes

- **Sender:**
  - the sequence numbers one NACK requests are de-duplicated;
  - they are capped at the send-buffer size;
  - a sequence number isn't retransmitted again within `MIN_RETRANSMIT_INTERVAL`.
- **Receiver:**
  - the gap limit comes from the configured receive buffer instead of a fixed 512 packets;
  - a jump beyond it resyncs only after the stream continues from the new position;
  - one outlier packet is dropped instead of resetting the stream;
  - 16-bit sequence wraparound is handled.

MSRV 1.95.0.
