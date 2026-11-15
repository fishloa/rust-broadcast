# srt-runtime 0.10.0

_Released 2026-11-15._

Minor-class release (0.9 -> 0.10). The SRT wire numbers are typed (`SocketId`, `SeqNo`, `MessageNumber`, `AckNumber`, `SrtTimestamp`), which removes the class of bug that started this wave: an ACK or ACKACK whose Acknowledgement Number has bit 31 set was accepted by `parse` but refused by `serialize_into`. The handshake is decoded once at the parse edge (a malformed loss list or extension block now fails the control packet), every latency and retransmit setting is a `Duration`, and the sans-IO engines take the caller's clock as a `stage::Timestamp`.

Version: 0.9.0 -> 0.10.0.

## Fixes

- An ACK or ACKACK whose Acknowledgement Number has bit 31 set no longer fails to serialize: the field is a whole 32-bit `Type-specific Information` word (`draft-sharabayko-srt-01` §3.2.4, §3.2.8), so every value a peer can put on the wire was accepted by `parse` but refused by `serialize_into` with `Error::FieldOverflow` (found by the `srt_packet` fuzz target). The bound added in #2766 is removed, and `AckPacket::ack_number`/`AckAckPacket::ack_number` are a `packet::AckNumber`.
- The remaining bounds sit at the parse edge instead of the serializer, so `parse` can never yield a packet `serialize` refuses: a DROPREQ whose Message Number word is wider than the 26-bit Message Number it names is rejected with `Error::FieldOverflow`, and the 31-bit sequence numbers are masked at parse.
- A truncated handshake extension block header reports the bytes actually available in `BufferTooShort::have` instead of always `0`.
- `KmRefreshDriver::on_packet_sent` rotates through every cycle a batch crosses, returning each cycle's PreAnnounce/Switchover/Decommission in order, instead of at most one switchover per call.

## Breaking changes and migration

### Typed packet numbers

`packet::number` defines `SocketId` (32-bit Destination/Source Socket ID), `SeqNo` (31-bit Packet Sequence Number with `broadcast_common::seq` wrap arithmetic), `MessageNumber` (26-bit, wrapping to `1`), `AckNumber` (32-bit ACK/ACKACK Acknowledgement Number) and `SrtTimestamp` (32-bit header Timestamp). Every packet's header `timestamp` is a `SrtTimestamp` and its `dest_socket_id` a `SocketId`; `ControlPacket::{timestamp, dest_socket_id}` and `NakPacket::parse_cif` take and return the typed values; `NakPacket`'s entries are `SeqNo` values, so an over-wide endpoint cannot be constructed.

```rust
// before
let packet = ControlPacket { timestamp: 0, dest_socket_id: 1234, .. };
let first: u32 = nak.first_seq;
// after
let packet = ControlPacket { timestamp: SrtTimestamp::ZERO, dest_socket_id: SocketId::from_wire(1234), .. };
let first: SeqNo = match nak.entries[0] { LossListEntry::Single(seq) => seq, LossListEntry::Range(first, _) => first };
```

### `HandshakeExtensions` and `NakPacket` are decoded values

`HandshakeExtensions` is the decoded `Vec<HandshakeExtensionBlock>`, not a borrowed `&[u8]` walked lazily: `parse` decodes the extension section once, so a list whose declared block length overruns the bytes present fails the control-packet parse (`Error::ExtensionOverrun`). `iter()` yields blocks directly. `NakPacket` carries a decoded `entries: Vec<LossListEntry>` instead of a borrowed `raw_loss_list: &[u8]` and is no longer lifetime-generic; a malformed loss list is rejected by `ControlPacket::parse` with `Error::InvalidLossList`. `LossListIter` and `build_loss_list` are removed.

```rust
// before
let nak = NakPacket { raw_loss_list: &raw, .. };
for entry in nak.entries()? { .. }
// after
let nak = NakPacket { entries, .. };          // or NakPacket::parse_cif(&cif)
for entry in &nak.entries { .. }
```

### The engines take a `stage::Timestamp`

`arq::Receiver::{feed_data, tick, on_ackack}`, `arq::Sender::{on_data, on_data_keyed, tick, on_ack}`, `tsbpd::TsbpdScheduler::{feed_data, tick, next_release_after}`, `filecc::FileCc::on_ack` take a `broadcast_common::stage::Timestamp` instead of a `Duration`, and `arq::Receiver::next_timeout` returns `Option<Timestamp>` (a `checked_add` deadline, where `None` never fires) instead of a saturated instant equal to "now". `filecc::FileCc::tick` (an empty, dead method) is deleted.

```rust
// before
receiver.tick(Duration::from_millis(n));
let timeout = receiver.next_timeout();
// after
receiver.tick(Timestamp::from_millis(n));
let timeout = receiver.next_timeout();   // Option<Timestamp>
```

### Latency and retransmit are `Duration`

`HandshakeConfig::latency` (was `latency_ms: u16`), `HandshakeConfig::retransmit_after` (was `retransmit_after_ticks: u32`, now the caller-clock silence before a retransmit), their setters `with_latency`/`with_retransmit_after`, `NegotiatedParams::latency`, `url::SrtUrl::latency` and `TsbpdScheduler::new`'s TSBPD delay are `Duration`; the handshake `tick` methods take the time advanced since the last call (`tick(elapsed: Duration)`) instead of counting fixed ticks. A latency still goes on the wire as whole milliseconds (rounded down), and `DEFAULT_RETRANSMIT_AFTER` replaces the old three-tick default.

```rust
// before
let cfg = HandshakeConfig::default().with_latency_ms(180).with_retransmit_after_ticks(3);
handshake.tick();
// after
let cfg = HandshakeConfig::default()
    .with_latency(Duration::from_millis(180))
    .with_retransmit_after(Duration::from_millis(60));
handshake.tick(period);
```

### `io::SrtConfig` is `io::SrtSocketConfig`, with no `impl Into`

`io::SrtConfig` is `io::SrtSocketConfig`, named for its role because one config serves both the caller and the listener. The `impl Into<SrtSocketConfig>` on the constructors and the `From<HandshakeConfig>` impl are removed: `connect`, `connect_from`, `connect_from_socket`, `rendezvous`, `rendezvous_from_socket` and `bind` take an `io::SrtSocketConfig` by value.

```rust
// before
let listener = SrtListener::bind(addr, handshake).await?;
let socket = SrtSocket::connect(addr, handshake.clone()).await?;
// after
let listener = SrtListener::bind(addr, SrtSocketConfig::new(handshake)).await?;
let socket = SrtSocket::connect(addr, SrtSocketConfig::default()).await?;
```

### `KmRefreshThresholds` and `CryptoConfig` are validated

`KmRefreshThresholds` is `#[non_exhaustive]` with private fields: build it with `KmRefreshThresholds::new(refresh_period, pre_announcement_period)`, which rejects a zero pre-announcement period and any pre-announcement at or past the refresh period (a zero refresh period used to rotate the active key on every packet), and read it with `refresh_period()`/`pre_announcement_period()`. `CryptoConfig` is `#[non_exhaustive]`, built with `CryptoConfig::new(passphrase, salt, sek)`.

```rust
// before
let thresholds = KmRefreshThresholds { refresh_period: r, pre_announcement_period: p };
// after
let thresholds = KmRefreshThresholds::new(r, p)?;
```

### The wire constants are the typed enums

The `packet::control::CONTROL_TYPE_*` (`u16`) and `packet::handshake::ENCRYPTION_FIELD_*` (`u16`) / `HANDSHAKE_TYPE_*` (`u32`) constants are removed: `ControlType`, `EncryptionField` and `HandshakeType` carry the same values through `from_bits`/`to_bits` (and `name()`/`Display`).

```rust
// before
if handshake_type == HANDSHAKE_TYPE_INDUCTION { .. }
// after
if handshake_type == HandshakeType::Induction { .. }
```

## Added

- `packet::number` (`SocketId`, `SeqNo`, `MessageNumber`, `AckNumber`, `SrtTimestamp`).
- `packet::GroupFlags::message_number_sync()`: a public accessor for the `M` bit (§3.2.1.4), in place of a test-only one.
- `url::SrtUrl::parse` now parses through `url::Url`, so `SRT://host` names the same endpoint as `srt://host` and a malformed authority stays an error; a `#` inside a `streamid` value is still kept whole.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `srt-runtime-v0.10.0`.
