# rist-runtime 0.1.0

**Release date:** 2026-08-10

First release. Wire-level codecs for the RTCP messages the **RIST Simple
Profile** (VSF TR-06-1:2020) defines or profiles, plus a sans-IO ARQ
reliability engine, built on the generic
[`rtcp-packet`](https://crates.io/crates/rtcp-packet) crate (RFC 3550 §6
SR/RR/SDES/BYE/APP). `no_std` + `alloc`; the `std` feature is on by default.

## What it is

| Type | Spec |
|---|---|
| `GenericNack` | RFC 4585 §6.2.1 — RTCP Transport-Layer Feedback (PT 205, FMT 1), bitmask retransmission request |
| `RangeNack` | TR-06-1 §5.3.2.2 — RIST RTCP APP (PT 204, subtype 0, name `"RIST"`), range-based retransmission request |
| `RttEcho` / `RttEchoKind` | TR-06-1 §5.2.6 — RTCP APP (PT 204, name `"RIST"`, subtype 2/3), round-trip time measurement |
| `RistSenderCompound` / `RistReceiverCompound` | TR-06-1 §5.2.1 — compound RTCP packet structure |

`arq` is a **sans-IO** reliability engine — it owns no socket, no timer and no
runtime:

- `arq::Receiver` implements TR-06-1 §5.3.1's two-stage buffer (Reorder
  Section + Retransmission Reassembly Section), loss detection, and
  retry-capped retransmission-request scheduling.
- `arq::Sender` implements the §5.3.3 sender-side NACK response: locating a
  previously-sent packet by sequence number. The packet store itself is the
  caller's, deliberately — the engine decides *what* to retransmit, not where
  the bytes live.
- `arq::rtt::rtt_sample` turns a completed `RttEcho` round trip into an RTT
  sample for the scheduler.

## What it is not

This crate does **not** implement the RIST Main or Advanced profiles, and it
carries no transport: no sockets, no encryption, no tunnelling. It is the
Simple Profile's RTCP message layer and the reliability logic that sits on top
of it. RTP framing itself belongs to `rtp-packet`.

## Verified against real captures, not just spec vectors

Every wire type round-trips byte-exactly — parse → serialize → identical bytes
— and that claim is checked against a real librist capture
(`fixtures/rist/rist-simple-loss25pct-loopback.pcap`), not only against inline
hand-written vectors:

- All **68** RTCP compounds the capture yields (3 SR, 65 RR in every
  combination of bare / `RangeNack` / `RttEcho`) are re-serialized and compared
  against the original captured slice.
- The ARQ engine's independently computed NACK output for the capture's
  verified isolated-loss shape is byte-identical to librist's own frame-15
  `RangeNack` payload (`tests/arq_frame15_loss_reproduction.rs`).

Two corrections were made before this first release rather than shipped:

- `RistSenderCompound` and `RistReceiverCompound` implemented only
  `Serialize`, so the crate's "byte-exact round-trip fidelity for every wire
  type" claim actually covered three of five. Both now implement `Parse`
  (#938).
- Four tests in `tests/round_trip.rs` were named `*_round_trip` but only called
  `to_bytes()` and never parsed back — they could not have failed on a
  round-trip defect. They now parse the serialized bytes and assert byte
  equality, and a new test exercises every optional compound slot together
  (#938).

## Compatibility

Edition 2024, MSRV 1.95.0. Builds `--no-default-features` as `no_std` +
`alloc`, cross-compiled for `thumbv7em-none-eabi` by CI. Depends only on
`broadcast-common` and `rtcp-packet`.
