---
status: DRAFT
---
# srt-runtime 0.8.1

_Released 2026-11-10._

Fix release (0.8.0 -> 0.8.1): handshake timestamping, key refresh, and ARQ robustness fixes; no breaking change.

Version: 0.8.0 -> 0.8.1.

## Added

- `CallerHandshake`, `ListenerHandshake`, `RendezvousHandshake` gain `set_elapsed(Duration)`.

## Fixed

- Handshake packets carry microseconds since connection start.
- Refreshed SEK survives lost Key Material datagram; transmit/receive keys separated.
- Plaintext connections no longer run KM Refresh.
- DROPREQ remembered until ack point; out-of-window ignored.
- Stray first handshake packet no longer holds pending slot.
- Key Material refresh with `KK = Both` installs both SEKs.
- Rendezvous rejects IPSN wider than 31 bits; MTU/flow window 0 rejected.
- Sender ignores ACK for unsent sequence.
- LiveCC `MAX_BW` saturates at `u64::MAX`; FileCC treats 0 as unbounded.
- TSBPD unwraps reordered packet near timestamp wrap.

## Other changes

- Avoid per-packet `payload.to_vec()` copy on non-encrypted send path.

---

Published from tag `srt-runtime-v0.8.1`.
