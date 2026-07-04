# Changelog

All notable changes to `srt-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Initial scaffold — SRT ([`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01))
packet codecs (issue #565).

### Added

- **Packet dispatch** (`SrtPacket`) — parses the 16-byte SRT header's `F` bit
  to route to a data or control packet (§3, Figure 2).
- **Data packet** (`DataPacket`, §3.1) — sequence number, `PacketPosition`
  (First/Middle/Last/Solo), order flag, `EncryptionKeyField`, retransmitted
  flag, message number, and the opaque payload.
- **Control packets** (`ControlPacket`, §3.2), one struct per Table 1 type:
  - `HandshakePacket` (§3.2.1) — `EncryptionField`, `HandshakeType`, and a
    lazily-walked `HandshakeExtensions` loop (mirroring `dvb-si`'s descriptor
    loop convention) with typed decoders for the Handshake Extension Message
    (`HsExtMessage`, §3.2.1.1), Key Material (§3.2.1.2), Stream ID
    (`as_stream_id`, §3.2.1.3 — including the 32-bit-little-endian-word
    storage quirk), and Group Membership (`GroupMembershipExtension`,
    §3.2.1.4).
  - `KeyMaterial` (§3.2.2) — the full KEKI/Cipher/Auth/SE/Salt/ICV/xSEK/oSEK
    layout, with the `S`/`V`/`PT`/`Sign`/reserved fixed-value fields
    validated (not stored). Carries wrapped-key bytes opaquely — no AES
    key-wrap/unwrap.
  - `KeepAlivePacket`, `CongestionWarningPacket`, `ShutdownPacket` (§3.2.3,
    §3.2.6, §3.2.7).
  - `AckPacket` with `AckCif::{Full,Small,Light}` (§3.2.4), selected by CIF
    length.
  - `NakPacket` (§3.2.5) with lazy `LossListEntry` (Single/Range) decoding
    per Appendix A's sequence-number coding.
  - `AckAckPacket`, `DropReqPacket`, `PeerErrorPacket` (§3.2.8-§3.2.10).
  - `UserDefinedPacket` for Control Type `0x7FFF` / undefined types, with
    `as_key_material()` for the Key Material-over-control-packet delivery
    form.
- Every public spec/field enum (`PacketPosition`, `EncryptionKeyField`,
  `ControlType`, `EncryptionField`, `HandshakeType`, `ExtensionType`,
  `GroupType`, `KmKeyFlag`, `Cipher`, `KmAuth`, `StreamEncapsulation`) has a
  `name()` + `Display` (issue #204 convention), enforced by
  `tests/label_coverage.rs`.
- Reserved/fixed-value fields (`Subtype`, the header `Type-specific
  Information` word where unused, the Key Material fixed fields) are
  validated on parse and not stored — see the crate root's reserved-bit
  policy.
- `tests/no_panic.rs` — a deterministic-PRNG fuzz-smoke test feeding
  truncated/random bytes to every parser and lazy-loop iterator.
- `no_std` + `alloc` core (default `std` feature togglable); no `unsafe`
  (`#![forbid(unsafe_code)]`).

### Explicit non-goals for this release

- Handshake state machine (caller/listener/rendezvous, §4.3).
- ARQ/loss handling, TSBPD, congestion control (§4-§5).
- AES key-wrap/unwrap crypto (§6).
- `tokio` socket adapter.

[Unreleased]: https://github.com/fishloa/rust-broadcast/compare/main...HEAD
