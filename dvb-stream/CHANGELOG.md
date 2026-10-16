# Changelog

## [Unreleased]

### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `dvb-t2mi` 11 -> 12, `mpeg-ts` 0.5 -> 0.6, `rtp-packet` 0.4 -> 0.5). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Fixed
- `TsDecoder::datagram` (and so `UdpSectionStream` / `UdpT2miStream`) now
  unwraps RTP-encapsulated MPEG-2 TS datagrams (RFC 3550 §5.1 header, CSRCs,
  header extension and padding skipped; payload must be whole 188-byte
  packets starting on `0x47`). Previously the TS lattice was hunted for
  through the RTP header bytes, so a `0x47` in the header (sequence number,
  timestamp or SSRC) 188 bytes before a `0x47` in the first packet latched
  the decoder onto misaligned bytes (#1099, r11-W-DS-2). Adds `rtp-packet`
  and `broadcast-common` dependencies.

### Changed (BREAKING)

- **BREAKING (#1159):** `T2miStream::{new,from_socket,bind}` take `mpeg_ts::pid::Pid` instead of `u16`.

## [0.6.0] - 2026-10-05
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

## [0.5.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- A crate-root note recording this crate's exemption from both #806
  drift guards (it defines no `pub enum`). No functional change.

## [0.4.1] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [0.4.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `dvb-si` 9 and `dvb-t2mi` 9** (issue #819). No functional change.

  The published 0.3.1 still required `^8` of both as *normal* dependencies, so
  a consumer combining it with `dvb-si` 9 got two majors of the same crate in
  one graph and the `Parse`/`Serialize` impls belonged to the wrong one. This
  was missed by the #819 sweep, which only checked `broadcast-common`
  requirements -- `dvb-stream`'s broadcast-common dependency is dev-only, so it
  did not show up. Found by the new published-dependency consistency check
  (#821) on its first run, which is the argument for having it.

## [0.3.1] - 2026-07-21
### Changed
- Widen the internal `mpeg-ts` dependency to `0.3` (was `0.2`; issue #663;
  private dependency — no public API change to `dvb-stream`).

## [0.3.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.2.2] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.2.1] — 2026-06-19

### Added
- `examples/`: `count_sections` (drive `SectionStream` over an in-memory TS) and
  `stream_stats` (tally table types + report demux/resync stats).

## [0.2.0] — 2026-06-16

### Added
- `ResyncStats { resyncs, bytes_discarded, desyncs }` + a `resync_stats()`
  accessor on `SectionStream` and `T2miStream`. `feed_buf` now counts re-aligns
  and discarded bytes, and **detects mid-stream desync** (a fed packet not
  starting with the `0x47` sync byte): it increments `desyncs`, discards the rest
  of the chunk, and forces a re-resync on the next read — instead of silently
  slicing garbage on corrupted mid-stream data (#220). Byte-identical for
  well-formed streams (counters stay zero).

### Changed
- Dependency requirements on the core crates bumped to `7.2`.

## [0.1.0]

Initial release — `SectionStream` / `T2miStream` async adapters over
`dvb_si::SiDemux` / `dvb_t2mi::T2miPump` with 188-byte TS resync.
