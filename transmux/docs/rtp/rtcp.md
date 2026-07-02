# RTCP — RTP Control Protocol (RFC 3550 §6)

Companion to the `rtp` module. Big-endian; every RTCP packet starts with the same
2-byte prefix + PT + length; packets are sent as a **compound** packet (§6.1:
first packet must be SR or RR, then SDES).

Common header: `V(2)=2 | P(1) | RC/SC(5) | PT(8) | length(16, in 32-bit words − 1)`.
Packet types: **SR=200, RR=201, SDES=202, BYE=203, APP=204**.

## SR — Sender Report (§6.4.1, PT=200)
Header (RC = reception report count) then **sender info** (20 bytes):
`SSRC(32)`, `NTP timestamp MSW(32)`, `NTP timestamp LSW(32)`, `RTP timestamp(32)`,
`sender's packet count(32)`, `sender's octet count(32)`; then RC × **report block**.

## RR — Receiver Report (§6.4.2, PT=201)
Header (RC) + `SSRC of packet sender(32)` + RC × report block. (No sender info.)

## Report block (24 bytes, §6.4.1)
`SSRC_n(32)` | `fraction lost(8)` | `cumulative number of packets lost(24, signed)` |
`extended highest sequence number received(32)` | `interarrival jitter(32)` |
`last SR – LSR(32)` | `delay since last SR – DLSR(32)`.

## SDES — Source Description (§6.5, PT=202)
SC chunks; each chunk = `SSRC/CSRC(32)` + list of items `[type(8), length(8), text]`
terminated by a type-0 item, padded to 32-bit. Item types: CNAME=1, NAME=2,
EMAIL=3, PHONE=4, LOC=5, TOOL=6, NOTE=7, PRIV=8.

## BYE (§6.6, PT=203)
Header (SC) + SC × `SSRC/CSRC(32)` + optional `length(8)` + reason text (padded).

## APP (§6.7, PT=204)
Header (subtype in the RC field) + `SSRC(32)` + `name(4 ASCII)` +
application-dependent data (32-bit aligned).

## Mapping
`rtcp` module: typed Parse/Serialize per packet type + a `CompoundPacket`
(Vec of typed packets) with byte-exact round-trip. NTP↔unix via the existing
`broadcast_common::time` epoch helpers if convenient. No media; not a hub spoke.
