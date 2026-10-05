# dvb-stream 0.6.0

_Released 2026-10-05._

### Added
- `udp::MulticastConfig` (`socket2` bind + join: `SO_RCVBUF`, opt-in `SO_REUSEADDR` / `SO_REUSEPORT` (default off, as with the plain bind it replaces), multicast interface), `UdpSectionStream::bind` / `UdpT2miStream::bind`, and `from_socket` constructors for an already-bound socket (tests bind port 0).
- `SectionStream::take_io_error` / `T2miEventStream::take_io_error`: when
  `poll_next` ends the stream because the reader errored (rather than a
  clean EOF), the error is now retrievable instead of silently discarded, so
  a supervisor can distinguish "source finished" from "source failed" and
  decide whether to reconnect (#1099, W-DS-1).

### Fixed
- `bind_multicast` (both streams) now reads each UDP datagram into a
  65,535-byte buffer instead of the 1,316-byte (7×188) buffer used for
  stream-oriented sources, and never stitches a trailing partial packet from
  one datagram onto the next, unrelated one. The old buffer silently
  truncated any datagram larger than 7 TS packets (routine for
  RTP-encapsulated delivery), and the carry-over corrupted alignment across
  datagram boundaries (#1099, W-DS-2).
- `T2miEventStream::feed_buf` discarded any trailing partial TS packet at
  the end of every read (unconditionally setting `filled = 0`), unlike its
  sibling `SectionStream` which already carries the partial over. Any read
  boundary that didn't land on a 188-byte packet boundary silently lost
  data. Partial bytes are now carried over to the next read, matching
  `SectionStream` (#1036).

### Changed
- **BREAKING** `TsFramer` (crate-private) is replaced by the public `TsDecoder`, a `tokio_util::codec::Decoder` over 188-byte TS packets (`Item = bytes::Bytes`; `TsDecoder::datagram()` for UDP). `SectionStream<R>` / `T2miEventStream<R>` keep their reader-generic API and wrap `FramedRead<R, TsDecoder>`; framing behaviour is unchanged (pinned by a golden over a real capture), including the bound on loss at a corrupt sync byte: at most one old 7 x 188-byte read is dropped per desync, then the decoder resyncs in-buffer.
- **BREAKING** `section_stream::UdpReader` is removed; `SectionStream<UdpReader>::bind_multicast` / `T2miEventStream<UdpReader>::bind_multicast` move to `UdpSectionStream::bind_multicast` / `UdpT2miStream::bind_multicast` with unchanged signatures.
- New dependencies: `tokio-util` (`codec`; `net` only with feature `udp`), `bytes`, and `socket2` (feature `udp`).
- `SectionStream` and `T2miEventStream` now share one internal framer (read, resync, 188-byte alignment, partial-packet carry-over, datagram framing; now `TsDecoder`, see Changed) instead of two diverging copies of `feed_buf` and the read loop; behaviour unchanged (#1141).

---

Published from tag `dvb-stream-v0.6.0`.
