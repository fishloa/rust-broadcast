# srt-runtime 0.9.0

_Released 2026-11-14._

Breaking-class release (0.8 -> 0.9): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.8.0 -> 0.9.0.

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

## Also in this release

### Removed
- BREAKING: `SrtSocket::{connect_with, connect_from_with, rendezvous_with}` and `SrtListener::bind_with`; pass `SrtConfig::new(handshake).with_io(io)` to `connect`, `connect_from`, `rendezvous` and `bind`. `connect_from_socket` and `rendezvous_from_socket` take the config in place of the separate handshake and io arguments (#2514).

### Changed
- BREAKING: `HandshakeConfig` is `#[non_exhaustive]`: build it from `HandshakeConfig::default()` and the `with_*` setters instead of a struct literal; mutating a field of a value you hold still works (#2514).
- BREAKING: `io::IoConfig`'s `connect`, `handshake`, `read_idle` and `write` fields and their `with_*` setters are one `timeouts: broadcast_common::timeouts::Timeouts` (set with `IoConfig::with_timeouts`), the type the RTSP and RTMP adapters share. The default is the new `io::SRT_TIMEOUTS` (10 s / 5 s / 5 s / 5 s), unchanged in value (#2514).
- Test-harness `tool_available` now uses `test-fixtures::process::tool_available_lenient`.

### Fixed
- `SrtUrl::parse` ignores a query key whose percent-decoding is not UTF-8 like any other unknown key, instead of rejecting the URL.

### Added
- `io::SrtConfig { handshake, io }` (`new`, `with_handshake`, `with_io`, `Default`, `From<HandshakeConfig>`): every `SrtSocket`/`SrtListener` constructor takes `impl Into<SrtConfig>`, so a bare `HandshakeConfig` is still accepted (#2514).
- `HandshakeConfig` has a `with_*` setter for every field (`with_latency_ms`, `with_mtu`, `with_max_flow_window_size`, `with_initial_seq_number`, `with_srt_version`, `with_flags`, `with_encryption_field`, `with_stream_id`, `with_group`, `with_local_ip`, `with_retransmit_after_ticks`, `with_max_retries`, and `with_crypto` under `crypto`) (#2514).
- `url::SrtUrl` (feature `std`): the typed `srt://` URL — `host`/`port` (default `DEFAULT_PORT`, 9000), `stream_id`, `latency_ms` (capped at `MAX_LATENCY_MS`, the draft's practical maximum), `mode` and `passphrase` — with `SrtUrl::parse`, and `dial_addr()` for the handshake. A rejection is a typed `url::UrlRejection` naming the part of the grammar that failed, so a caller keeps its own policy (which modes it accepts, whether it allows a passphrase) and its own wording. Accepts `srt://host:port` and a scheme-less `host:port`, bracketed IPv6, and a raw `streamid` containing `#`; rejects userinfo and a bare (ambiguous) IPv6 literal. The parser moved here from `multimux`'s SRT push (issue #2369).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `srt-runtime-v0.9.0`.
