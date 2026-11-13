# rtmp-runtime 0.10.0

_Released 2026-11-14._

Breaking-class release (0.9 -> 0.10): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.9.0 -> 0.10.0.

## Fixed

- Data-AMF0 `@setDataFrame` forwarded as FLV script tag `onMetaData`.
- `ServerSession` refuses second `connect` while publishing with `RtmpError::UnexpectedState`.
- `BasicHeader`/`MessageHeader` `serialize_into` returns `RtmpError::OutputBufferTooSmall` for too-small buffer.

## Also in this release

### Removed
- BREAKING: `io::RtmpTimeouts`; the adapters take `broadcast_common::timeouts::Timeouts` (the same four fields and defaults), shared with the RTSP and SRT adapters (#2514).

### Changed
- BREAKING: `RtmpTarget::stream_key` and `app` are percent-decoded (they were raw), and `AsyncRtmpClient::connect` refuses an `rtmps` target (plain TCP to a TLS port would leak the key; use `from_stream` over TLS) (#2588).
- **BREAKING**: `RtmpConnection`'s default stream type is now the erased `BoxStream` (a TCP socket, a TLS stream over one, or any other byte stream), so a caller naming one `RtmpConnection` no longer names its transport. `AsyncRtmpServer::accept` returns the new `AcceptedRtmpConnection` (the publisher's address, which an erased stream cannot report, plus the connection) instead of a `RtmpConnection`; call `into_connection()` for the connection itself. `RtmpConnection::peer_addr` is removed; use `AcceptedRtmpConnection::peer_addr`.

### Added
- `target::RtmpTarget` accepts `rtmps://` (`RtmpTarget::tls`, default port `RTMPS_DEFAULT_PORT` 443, `SCHEME_RTMP`/`SCHEME_RTMPS`), percent-decodes the app and stream-key path (RFC 3986 §2.1; the query stays as written), and refuses a key that is only a query (`RtmpUrlError::PathNotUtf8` for undecodable text). A new `target` feature (implied by `tokio`) exposes the URL grammar without sockets (#2588).
- `io::RtmpStream`, `io::BoxStream` and `RtmpConnection::from_stream_erased`: wrap any connected stream, so a session can be driven over something other than a TCP socket (an in-memory pipe in a test, a TLS stream in a host) without a second connection type.
- `flv::FlvTagHeader` (`parse`, `total_len`, `Serialize`): the 11-byte FLV tag header on its own, so an incremental reader can size a tag before its body arrives. `FlvTag::parse` and the tolerant walk decode the header through it, and `transmux`'s streaming FLV demuxer uses it instead of hand-rolled `u32::from_be_bytes` reads.
- `amf0::amf0_values(&[Amf0Value]) -> Result<Vec<u8>>`: encode several bare AMF0 values as one message body (the shape an FLV `onMetaData` script tag has). `transmux`'s FLV muxer writes its script tag through it.
- `rtmp_runtime::flv`: the Adobe FLV v10.1 Annex E header and tag codec (`FlvHeader`, `TagType`, `FlvTag<'a>`, the strict `tags()` walk, `Parse`/`Serialize` with byte-exact round trips on the committed FLV fixtures), `no_std` + `alloc`. It replaces the crate-private FLV writer the server and client used, and transmux frames FLV through it, so the workspace has one FLV tag codec (#2342). A final tag whose trailing `PreviousTagSize` is cut off is accepted (the field is redundant, §E.4.1); a cut tag body is an error. New fuzz target `rtmp_flv_tags`.
- `rtmp_runtime::flv::tags_resync(input, header) -> ResyncWalk`: the tolerant walk beside the strict `tags()`, tested in `tests/it/flv_tags.rs` over the committed FLV captures (a damaged tag is skipped and its offset reported, a cut-off final tag is flagged, a header with no tags yields none). It keeps the tags after a damaged span (skipping a span whose tag chain does not hold to the next consistent one, reported in `damaged`), reports a final tag whose body overruns the input as `truncated_tail` instead of dropping the good prefix, and accepts a final tag missing only its redundant trailing `PreviousTagSize`. It is the workspace's one FLV tag walker: `transmux`'s `FlvDemux` adapts its result, and transmux's own resync logic is gone.

### Fixed
- `flv::FlvHeader` serialize now writes the `DataOffset - 9` extra header bytes the parser skipped, so a non-standard `DataOffset` round-trips byte-exactly; `FlvTag::parse` rejects a trailing `PreviousTagSize` that disagrees with `11 + DataSize`.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `rtmp-runtime-v0.10.0`.
