# srt-runtime 0.5.0

_Released 2026-10-05._

Breaking (0.x minor), large release. It is the first publication since 0.4.0 (0.4.1, a security patch for the tokio adapter, was prepared but never tagged, so its fixes ship here; see [srt-runtime-0.4.1.md](srt-runtime-0.4.1.md)). It combines that security work, a hardening and libsrt-interoperability pass over the adapter and the sans-IO engines (audit r08, issues #1029, #1060 to #1064, #1084, #1129, #1134, #1141, #1142), and the de-hand-roll wave's bounded, deadline-driven tokio adapter. **Upgrade if you use `io::SrtSocket` or `io::SrtListener`.** Who must act: adapter users (`recv` now returns `bytes::Bytes`; handshake errors are new typed variants; constants became `IoConfig`), and users of the sans-IO engines (several signatures changed: `arq::Receiver::new`, `arq::Sender::on_data`, `tsbpd::TsbpdScheduler::new`, `handshake_sm::derive_cookie`; five control-packet structs gained a field; secret-bearing types changed). The `no_std` core gains no new mandatory dependency.

## Security

From the unpublished 0.4.1 (the full per-advisory table is in its note):

- a `HandshakeConfig` with `crypto` set negotiated keys but the adapter's data path never used them. The adapter now refuses encryption instead of faking it: a crypto-enabled config is rejected before any I/O with `Error::EncryptionUnsupported`, an incoming handshake that requests encryption is rejected, and encrypted data packets are never delivered. The sans-IO engines still negotiate keys.
- too-late packet drop was off and DROPREQ ignored, so one packet the sender would never retransmit stalled delivery while staging grew at stream bitrate. The adapter handles DROPREQ and caps staging at the flow window; `tsbpd::TsbpdScheduler` with too-late drop enabled skips the gap once the next packet's play time arrives (§4.6) and `TickOutcome::dropped` reports skipped sequence numbers.
- `SrtListener` kept unbounded pending-handshake state. It now bounds in-progress handshakes and expires them on a timer even under continuous traffic.
- fixed or sequential ISN and Socket ID. The adapter now draws a fresh random ISN and Socket ID per connection from the OS (`getrandom`), with Socket IDs folded into `1..=0x3FFF_FFFF`, the range libsrt allocates from.

New in this release (#1142, secret hygiene, `crypto` feature): `CryptoConfig` and `SecretBytes` implement `zeroize::Zeroize` and `ZeroizeOnDrop`; `CryptoConfig`, `NegotiatedParams` and `KeyMaterial` no longer print passphrases, keys, wrapped keys or ICVs in `Debug` (lengths only); secrets compare in constant time; the `serde` feature never serializes the SEK, or a `KeyMaterial`'s salt, ICV or wrapped keys.

## Breaking changes: tokio adapter (`io`, feature `tokio`)

**`SrtSocket::recv` returns `Option<bytes::Bytes>`** (was `Option<Vec<u8>>`). Each datagram is one exactly sized `Bytes` and the payload is a slice of it, so a packet is not copied again on the way up. Add `bytes` to your dependencies only if you need to name the type; `.to_vec()` recovers the old type. `SrtSocket::send_bytes(Bytes)` hands a payload to the driver without a copy.

```rust
// 0.4.x
while let Some(payload) = sock.recv().await? { sink(payload /* Vec<u8> */); }
// 0.5.0
while let Some(payload) = sock.recv().await? { sink(payload.to_vec()); }
```

**Typed handshake errors (#1084, r08-SRT-W12).** Connect, bind and accept now return `Error::Rejected(RejectionReason)` (the peer's own reason, so a wrong secret is distinguishable from a refused backlog), `Error::HandshakeTimedOut { stage }`, `Error::EncryptionUnsupported`, or `Error::Handshake { stage, source }` (keeps the underlying error as `source`). Previously these were flattened into `Error::InvalidField`. Code that matched `InvalidField` to detect a failed connect must match these.

**`IoConfig` replaces the timeout constants.** `HANDSHAKE_TIMEOUT` (5 s) and `PEER_IDLE_TIMEOUT` (5 s) are now `IoConfig::handshake` and `IoConfig::read_idle`, same defaults. `IoConfig` is `#[non_exhaustive]` with builders `with_max_datagram` (default 1500, clamped to `io::MIN_MAX_DATAGRAM` 64 ..= `io::MAX_MAX_DATAGRAM` 65535), `with_connect` (10 s, resolve plus bind), `with_handshake`, `with_read_idle`, `with_write` (5 s, every `send_to`). New entry points `SrtSocket::connect_with`, `connect_from_with` and `SrtListener::bind_with`; the existing `connect`, `connect_from` and `bind` use `IoConfig::default()`.

```rust
let cfg = IoConfig::default().with_read_idle(Duration::from_secs(15));
let sock = SrtSocket::connect_with(addr, handshake_cfg, cfg).await?;
```

**`SrtSocket::send` now applies backpressure.** It waits while the sender already holds a full flow window of unacknowledged packets or the hand-off queue is full, and fails only once the connection has ended (r08-SRT-W6). It used to queue without limit.

**`SrtListener` handshakes progress in a tracked background task.** `accept()` only waits for a finished handshake, so a caller connects (and a lost CONCLUSION response is answered again) whether or not anyone is polling `accept()`. At most 64 finished handshakes wait for `accept()`. Send errors met while answering a peer are counted (`SrtListener::send_errors()`), never returned from `accept()`. Dropping the listener frees its UDP port immediately (defect 3); connections it already accepted keep being routed until they are gone.

**Internal channels are bounded** (ingress, candidate-new-connection queue, delivered-payload channel): a full channel drops and counts instead of growing. Drops are visible through the new `SrtSocket::stats()` (returning `SocketStats`: `rx_dropped`, `deliver_dropped`, `late_dropped`, `rx_oversize`), `SrtListener::unrouted_dropped()` and `accept_overflow_dropped()`.

## Breaking changes: sans-IO engines (`no_std` core)

```rust
// 0.4.x                                              // 0.5.0
arq::Receiver::new(dest_socket_id, initial_seq)       arq::Receiver::new(dest_socket_id, initial_seq, max_flow_window)
tsbpd::TsbpdScheduler::new(seq, base: u64, .., drift: u64, ..)
                                                      tsbpd::TsbpdScheduler::new(seq, base: i64, .., drift: i64, ..)
handshake_sm::derive_cookie(peer_key, bucket, secret: u64)
                                                      handshake_sm::derive_cookie(peer_key, bucket, secret: u128)
```

- `arq::Receiver::new` takes `max_flow_window: u32`, the Available Buffer Size its Full ACK advertises (§3.2.1). `Receiver::with_mtu` sizes the periodic NAK against the negotiated MTU.
- `tsbpd::TsbpdScheduler::new` takes the time base and initial drift as signed `i64` microseconds: `TsbpdTimeBase = T_NOW - HSREQ_TIMESTAMP` is negative whenever the peer's clock is ahead (r08-SRT-W9). New accessors `time_base_us()`, `drift_us()`, `tlpktdrop_enabled()`, `next_release_after()`.
- `handshake_sm::derive_cookie` is SipHash-2-4 keyed by a 128-bit `secret: u128`, so **every cookie value changes** (r08-SRT-O5). The tokio listener draws the key from the OS and rotates it every minute; a handshake begun under the previous key still completes.
- `arq::Sender::on_data` returns `Result<Vec<u8>>`: `Error::FieldTooWide` for a `seq` wider than 31 bits or a `message_number` wider than 26 (nothing is buffered). It expects `seq` to increase (checked by a `debug_assert!`). `on_nak` resolves ranges against the send buffer instead of expanding them, ignores a range whose end precedes its start, and examines at most 65 536 sequence numbers per NAK datagram.
- `arq::Receiver::feed_data` ignores (`FeedOutcome::out_of_window`) a sequence number wider than 31 bits or further ahead of the ack point than the flow window (clamped to 262 144). A first packet beyond the peer's ISN now reveals the gap before it.
- `KeepAlivePacket`, `CongestionWarningPacket`, `ShutdownPacket`, `AckAckPacket` and `PeerErrorPacket` gain `libsrt_pad: bool` (these structs are not `#[non_exhaustive]`, so struct literals must set it). `true` is libsrt's 20-byte wire shape (4-byte zero-pad CIF), `false` the 16-byte shape of the draft; both now round-trip byte-identically. Real libsrt peers always send the pad and used to be rejected as `UnexpectedTrailingBytes` (#1060); we now emit it ourselves.
- `NegotiatedParams` gains `mtu` and `max_flow_window_size` (the smaller of the two sides' advertised values, §3.2.1, computed by Caller, Listener and Rendezvous engines).
- `crypto` feature: `NegotiatedParams::sek` is `Option<SecretBytes>` (was `Option<Vec<u8>>`; derefs to `&[u8]`); `crypto::derive_kek` and `crypto::unwrap_sek` return `Zeroizing<Vec<u8>>`; `CryptoConfig` wipes itself on drop, so its fields can no longer be moved out of it.
- `KeyMaterial::parse` and `serialize_into` both reject `KK = 00b` ("no SEK provided", invalid per §3.2.2); the old parse accepted a message `serialize_into` could not reproduce (r08-SRT-W13). `serialize_into` returns `Error::FieldTooWide` instead of truncating, for example a `Salt` over 1020 bytes (#1129).
- `RejectionReason::from_handshake_type` recognises only `1000..2^31` (`REJECTION_CODE_LIMIT`); libsrt's negative `URQ_*` range (for example `0xFFFF_FFFC`) is no longer reported as `Rejected(Reserved(..))` (r08-SRT-W14).
- `HandshakeConfig::default().max_retries` is 12 (`handshake_sm::DEFAULT_MAX_RETRIES`, was 5): twelve 250 ms retransmits are libsrt's 3 s connect timeout.
- The engines refuse a peer ISN wider than 31 bits (`Rejected(Rogue)`). The Rendezvous engine returns `HandshakeOutOfSequence` instead of panicking on a broken internal invariant.

## Wire and behaviour changes (observable by a peer)

- The Caller's Key Material request carries `SE` = 2 (MPEG-TS/SRT), as libsrt requires; a KMREQ with `SE` 0 is refused. Verified against a real `srt-live-transmit`, which now accepts our key exchange and decrypts our data (r08-SRT-W7).
- The Caller's CONCLUSION sends Destination Socket ID `0` (as a libsrt Caller does) and takes the peer Socket ID from the CONCLUSION response; sending the captured ID made a real libsrt Listener silently drop it and stall the handshake.
- A received Keep-Alive is no longer echoed back; each side sends its own (§3.2.3). An idle connection sends a Keep-Alive after one second of silence; dropping an `SrtSocket` sends the peer a SHUTDOWN; a connection with no traffic from its peer for `IoConfig::read_idle` (5 s, libsrt's `SRTO_PEERIDLETIMEO` default) is torn down.
- An unanswered handshake request is re-sent every 250 ms (was one 5 s receive timeout, three times: 15 s for one lost INDUCTION). A repeated CONCLUSION from an already accepted Caller is answered with the same response (r08-SRT-W5, r08-SRT-W10).
- Packet timestamps wrap modulo 2^32 microseconds (~71.6 min) instead of saturating at `u32::MAX`, and the receiver unwraps them (#1063): after that point every packet used to look impossibly late and TLPKTDROP discarded it forever. Packet Sequence Number (31 bit) and Message Number (26 bit) counters wrap at their own width (#1062); Message Number wraps to `1`, not `0`, which libsrt reserves for control messages.
- The adapter honours the negotiated TLPKTDROP flag, MTU, Maximum Flow Window Size and the greater of the two latency requests (§4.3.1.2), instead of its own configured values or a hard-coded skip. Without TLPKTDROP a missing packet is waited for.
- The receiver's Full ACK advertises the negotiated flow window instead of `avail_buf_size: 0`, which a real libsrt Caller read as "no receive space" and silently withheld every DATA packet.
- The Rendezvous cookie contest matches libsrt's `backwardCompatibleCookieContest` (signed 32-bit difference). The plain unsigned compare disagreed with libsrt for about a quarter of cookie pairs (#1064).
- The connection driver sleeps until its next deadline instead of ticking every 2 ms: an idle connection wakes 100 times a second (the 10 ms Full ACK is protocol) instead of 500. Control traffic (ACK, NAK, ACKACK, Keep-Alive) has its own outbound queue and is no longer delayed behind a LiveCC-paced DATA packet, and pacing no longer sleeps per packet (which capped throughput near 1000 packets/s regardless of `MAX_BW`, #1061); a late wake sends up to 16 paced packets.

## Other fixes

- `SrtListener` demultiplexes by Destination Socket ID and recorded source address; a route-table entry is no longer leaked when an `SrtSocket` is dropped. One routing pump replaces a shared `recv_from` that could hand a datagram to the wrong connection (#1029). A poisoned `Mutex` can no longer panic the route-table `Drop` or routing pump (#1134).
- A caller-side forwarder task is aborted on drop, so the local UDP port is freed (a later bind used to fail with `AddrInUse`). `SrtSocket::connect` no longer fails on a stray or unparseable packet during the handshake. `retransmit_after_ticks = u32::MAX` no longer panics `connect` (1 ms floor).
- A datagram larger than `IoConfig::max_datagram` is dropped and counted in `SocketStats::rx_oversize` instead of being truncated into a shorter, valid-looking packet. A packet behind the delivery cursor is dropped on arrival (`SocketStats::late_dropped`) instead of accumulating forever.
- DoS bounds: a NAK no longer costs O(buffer) per sequence number (r08-SRT-W1/O3); a crafted sequence jump no longer floods the loss list (r08-SRT-W2); unanswered Full ACKs are forgotten after `max(4 x RTT, 1 s)`, capped at 1 024 (r08-SRT-W3); a periodic NAK longer than one datagram is split to the MTU (r08-SRT-W4).
- The TSBPD time base is seeded from the peer's handshake timestamp and drift is estimated in 1 000-packet windows (`DRIFT_SAMPLE_COUNT`, `DRIFT_MAX_US` = 5 ms). Two wrap tests that could not fail were rewritten (r08-SRT-W9).
- Zero-copy ingress (r08-SRT-O4, O2). New helper `arq::seq::seq_in_closed_range` and `arq::Receiver::next_timeout`.

## Dependencies

Verified from the `Cargo.toml` diff against `srt-runtime-v0.4.0`:

```toml
broadcast-common = { version = "9.3" -> "9.4", default-features = false }
# crypto feature (RustCrypto 0.13 generation; key-wrap, PBKDF2 and AES-CTR output unchanged, KAT and libsrt interop tests pass)
aes 0.8 -> 0.9, ctr 0.9 -> 0.10, aes-kw 0.2 -> 0.3, pbkdf2 0.12 -> 0.13, hmac 0.12 -> 0.13, sha1 0.10 -> 0.11
zeroize = { version = "1", default-features = false, features = ["alloc"], optional = true }   # new, crypto
subtle  = { version = "2", default-features = false, optional = true }                          # new, crypto
# tokio feature
getrandom  = { version = "0.2", optional = true, default-features = false }                     # new
bytes      = { version = "1", optional = true, default-features = false }                       # new
tokio-util = { version = "0.7", optional = true, default-features = false, features = ["rt"] } # new
crypto = [..., "dep:zeroize", "dep:subtle"]
tokio  = ["dep:tokio", "dep:getrandom", "dep:bytes", "dep:tokio-util", "std"]
```

`arq::seq` arithmetic now delegates to `broadcast_common::seq::SeqSpace` with unchanged results (#1141). MSRV 1.95.0.

---

Published from tag `srt-runtime-v0.5.0`.
