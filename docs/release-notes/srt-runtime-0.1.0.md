# srt-runtime 0.1.0 — 2026-07-04

First publish. Sans-IO SRT (Secure Reliable Transport) building blocks —
**packet codecs only** in this release. `no_std` + `alloc`.

## Packet codecs (#565)
Typed parse/serialize per **IETF draft-sharabayko-srt-01 §3**:
- Data packet + all control packets: HANDSHAKE, KEEPALIVE, ACK (Full/Small/
  Light), NAK (loss-list), CONGESTION_WARNING, SHUTDOWN, ACKACK, DROPREQ,
  PEERERROR, USER_DEFINED (incl. Key Material delivery).
- Handshake extensions: HSREQ, Key Material, StreamID, Group Membership.
- Symmetric byte-exact round-trip; `no_panic` fuzz over every parser.

## Not yet (tracked follow-ups)
Handshake state machine, ARQ / loss-retransmit, crypto (AES KM/cipher), and the
tokio socket adapter. See the crate root docs.

## Compatibility
`no_std` core; MSRV 1.86.
