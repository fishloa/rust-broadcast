# dvb-stream 0.9.0

_Released 2026-11-14._

Major-class release (0.8 -> 0.9) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.8.0 -> 0.9.0.

## Added
- `resync::resync_confirmed` and `resync::unconfirmed_candidate`: the confirmed-only alignment scan the byte-stream decoder uses.

## Fixed
- `TsDecoder` (byte-stream mode): one corrupt sync byte now costs only that packet instead of up to 7 (1316 bytes); the decoder skips a byte and re-locks on the next sync byte confirmed one packet later.
- `TsDecoder` (byte-stream mode): a `0x47` in a short first read that cannot yet be confirmed by a second sync byte is no longer locked onto; the decoder waits for more input and accepts an unconfirmed sync byte only at end of input. Datagram mode is unchanged.

## Also in this release

### Changed
- `resync` module now re-exports the implementation from `mpeg_ts::resync`; the local algorithm was removed (#2479).

### Added
- `udp::bind_standard` with `udp::SocketSpec` and `udp::MulticastMembership`: the workspace's single datagram bind (UDP socket, `SO_REUSEADDR`, `SO_REUSEPORT` where the platform has it, `SO_RCVBUF`, an optional IPv4/IPv6 multicast join, non-blocking). `MulticastConfig::bind` is now a thin wrapper over it, with the same options and the same "a refused `SO_RCVBUF` is logged and the bind retried without it" behaviour.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `dvb-si` 13 -> 14, `dvb-t2mi` 13 -> 14, `mpeg-ts` 0.7 -> 0.8, `rtp-packet` 0.6 -> 0.7.

---

Published from tag `dvb-stream-v0.9.0`.
