# dvb-stream 0.8.1

_Released 2026-11-10._

Patch release: byte-stream `TsDecoder` resync fixes.

Version: 0.8.0 -> 0.8.1.

## Added
- `resync::resync_confirmed` and `resync::unconfirmed_candidate`: the confirmed-only alignment scan the byte-stream decoder uses.

## Fixed
- `TsDecoder` (byte-stream mode): one corrupt sync byte now costs only that packet instead of up to 7 (1316 bytes); the decoder skips a byte and re-locks on the next sync byte confirmed one packet later.
- `TsDecoder` (byte-stream mode): a `0x47` in a short first read that cannot yet be confirmed by a second sync byte is no longer locked onto; the decoder waits for more input and accepts an unconfirmed sync byte only at end of input. Datagram mode is unchanged.

## Dependencies
- `mpeg-ts` 0.7.0 -> 0.7.1.

---

Published from tag `dvb-stream-v0.8.1`.
