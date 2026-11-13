# broadcast-common 12.0.0

_Released 2026-11-14._

Major release (11 -> 12): the `ts_dup` module moved to `mpeg-ts` (`mpeg_ts::ts_dup`); everything else is additive or a fix. Because nearly every crate in the workspace depends on `broadcast-common`, every one of them moves to a new caret epoch in this wave.

Version: 11.0.1 -> 12.0.0.

## Removed
- `ts_dup` module moved to `mpeg-ts` (legal duplicate TS packet detection, H.222.0 §2.4.3.3); import from `mpeg_ts::ts_dup` instead (#2466).

## Added
- `units`: the decimal time-unit ratios (`MILLIS_PER_SEC`, `MICROS_PER_MILLI`, `MICROS_PER_SEC`, `NANOS_PER_MICRO`, `NANOS_PER_MILLI`, `NANOS_PER_SEC`, all `u64`) declared once; about 40 per-file copies in broadcast-client, broadcast-auth, broadcast-dash, dash-runtime, hls-runtime, smooth-runtime, dvb-conformance, transmux and multimux are gone (#2591).
- `content_range::ContentRange::parse`: the one RFC 9110 §14.4 `Content-Range` parser (`bytes first-last/length-or-*`; `*` valid, last-pos at or past a known length invalid, first > last invalid, one space after the unit and none around `-`/`/`), `no_std` (#2586).
- `timeouts::Timeouts`: the `{connect, handshake, read_idle, write}` bounds of an adapter's awaited I/O, one type for the RTSP, RTMP and SRT adapters instead of three copies (#2514).
- `impl From<BitError> for BufferTooShort`: the one conversion from a bit-level failure to the workspace's byte-counted buffer error, so a bit reader's short input is reported in the same unit everywhere. `need`/`have` are whole bytes (a partial byte rounds up in `need`, down in `have`), read the same way whether an adapter reports the bits needed or the absolute position. It also fixes `BitReader::skip_bits` leaving the cursor past the end on a failed skip, so `bits_remaining` stays a real count (issue #2461).
- `backoff` module: `RetrySchedule`, the one capped retry schedule the workspace's clients and servers pace a reconnect from (issue #2471). Pure policy with no clock and no allocation: `raw_delay_for_attempt` is the deterministic series and `delay_for_attempt` the retry actually waited. A schedule is either exponential (`min * factor^attempt`, capped) or a literal step table (`from_steps`); jitter is equal-jitter from a fixed term sequence (`with_offset` starts a caller further into it), so the first retry is exactly the configured step, no delay ever exceeds it, the capped band stays jittered, and a fleet spread over the retry interval stays spread. `mix_seed` folds a caller's own entropy into a seed without reading a clock.
- `PusiAccumulator::len`/`is_empty`/`is_started` accessors; `PesAssembler` mirrors them.
- `http_date` module (`std` feature): `parse_http_date` (RFC 9110 §5.6.7), moved from `dash-runtime`.
- `url_ref` module (`url` feature): guarded RFC 3986 reference resolution wrapping `url::Url::join`, rejecting ASCII control characters before joining (issue #2458).
- `time::MonoUtc`: the pure monotonic-microseconds to UTC-microseconds mapping (`utc_at`, `mono_for_utc`), built from one anchor pair a caller supplies. It never reads a clock, so a sans-IO engine can map instants without touching the host's clocks. `time::WallClock` and `time::Anchor` name the same type for the two callers that use those names.
- `lang` module (`lang` feature): `bcp47_language`, ISO 639-2 -> BCP 47 (RFC 5646) language canonicalisation shared by HLS and DASH, backed by the `iso-639` crate.
- `xml` module (`xml` feature): `read_tree`, the bounded namespace-aware quick-xml reader that builds a small `xml::Elem` tree under a caller-supplied `xml::Namespace`, moved from `dash-runtime` so the Smooth Streaming manifest reader (#2356) can share it. It refuses DOCTYPE declarations and every entity they would declare, validates text against the XML 1.0 `Char` production, and enforces `XmlLimits` through `XmlBudget`; `xml::XmlError` reports the four ways it stops, and every bound stays the one `xml_chars::XmlLimit` names (#2355).
- `klv` and `ber` modules: the SMPTE ST 336 KLV triplet (`KlvItem<'a>` with a 16-byte `UniversalLabel` key), `walk_klv_items`/`collect_klv_items`, and the BER length codec (`decode_ber_length`, `encode_ber_length`, `ber_length_size`, `BerLength` which keeps the on-wire length width so a parsed item re-serializes byte-identically), with `KlvError`. Moved from st377-1 so st377-1 (MXF) and transmux (MISB KLV) share one implementation (#2341).

## Fixed
- `ber::encode_ber_length` / `encode_ber_length_as` write their length byte through `len::fit_u8` instead of `u8::try_from(..).unwrap_or(u8::MAX)`, so an over-range value is reported as a `KlvError` rather than silently clamped into the wire (unreachable today: the short-form range is checked first). The `truncation` workspace guard now covers `broadcast-common`, including that clamp idiom.
- `xml::read_tree` now rejects CDATA sections and non-XML-S whitespace outside the root element; `Elem::text` docs clarify it holds only direct text, not descendants.

---

Published from tag `broadcast-common-v12.0.0`.
