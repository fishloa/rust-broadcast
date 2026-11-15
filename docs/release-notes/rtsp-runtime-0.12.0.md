# rtsp-runtime 0.12.0

_Released 2026-11-15._

### Changed
- BREAKING: `Error::MessageParse { what, source }` carries `source: MessageParseError`, a new typed enum (`Codec(rtsp_types::ParseError)`, `TooLarge { at_least, limit }`, `Buffered { buffered, limit }`, `HeaderUnterminated { at_least, limit }`, `NotARequest`) instead of a boxed any-error, so a caller matches the failure instead of reading its text. `Error::Tls { source }` is a `rustls::pki_types::InvalidDnsNameError` and `Error::TlsConfig` wraps `rustls::Error`, both named types the `tls` feature already depends on (#2695).

### Changed
- BREAKING: `Error::MessageParse { what, source }` carries `source: MessageParseError`, a new typed enum (`Codec(rtsp_types::ParseError)`, `TooLarge { at_least, limit }`, `Buffered { buffered, limit }`, `HeaderUnterminated { at_least, limit }`, `NotARequest`) instead of a boxed any-error, so a caller matches the failure instead of reading its text. `Error::Tls { source }` is a `rustls::pki_types::InvalidDnsNameError` and `Error::TlsConfig` wraps `rustls::Error`, both named types the `tls` feature already depends on (#2695).


#### Changed
- BREAKING: one config per role, beside the adapter's IO config. `ClientSession::new(RtspClientConfig)` and `ServerSession::new(RtspServerConfig) -> Result<Self>` replace `ClientSession::new()` and `ServerSession::new(closure)`/`try_new(closure)`: the client config carries `user_agent` and `initial_cseq` (`with_user_agent`, `with_initial_cseq`; `ClientSession::default()` is a fresh session), the server config carries `session_ids`, `server_header`, `describe_sdp`, `server_ports` and `session_timeout` (`Duration`) (`with_session_ids`, `with_server_header`, `with_describe_sdp`, `with_server_ports`, `with_session_timeout`, `with_timeouts`). The `tokio` adapter's config is now `RtspClientIoConfig` (session + timeouts), so `RtspClientConfig` names the protocol config alone. Migration: `ClientSession::new()` -> `ClientSession::new(RtspClientConfig::default())?`, `ServerSession::new(|| id)` -> `ServerSession::new(RtspServerConfig::default().with_session_ids(SessionIdSource::counter(id)))?`, `session.with_describe_sdp(..).with_server_timeout(secs)` -> the same setters on the server config passed to `new` (#2698).
- BREAKING: session ids are a `SessionId` newtype, not a raw `u64` (`SessionId::new`, `value`, `parse`, `Display` as 16 lower-case hex digits) and their source is `SessionIdSource` (`OsRandom`, the default, or `Counter(seed)` for deterministic tests) rather than an untyped closure; `ServerSession::session_id`, `AsyncRtspServer::session_id` and `ClientSession::session_id`/`AsyncRtspClient::session_id` return it. A client reads a peer's id leniently (upper case, or fewer hex digits, is the same session), so a non-conforming server still gets a `TEARDOWN`. The `try_new` seam that staged a failing random source is gone with the closure form; a source failure still surfaces as `Error::RandomSource` on the `SETUP` (#2698).
- BREAKING: the keepalive clock is a `broadcast_common::stage::Timestamp`, not a microsecond `u64`: `ClientSession::mark_activity` and `ClientSession::handle_timeout` take one, and `ClientSession::poll_timeout` returns one. Migration: `session.mark_activity(now_us)` -> `session.mark_activity(Timestamp::from_micros(now_us))`, likewise `handle_timeout`, and a deadline compared against the host's clock compares as a `Timestamp`.

#### Removed
- BREAKING: the `state` module's FSM transition helpers, `is_state_neutral`, `client_next_state` and `server_next_state`, are crate-private: they are the state tables the crate's own client and server use, with no use outside it. `SessionState` stays public. Migration: nothing to call; the state transitions happen inside `ClientSession`/`ServerSession` (#2665).
- BREAKING: `RtspClientConfig::new(session)`, `RtspServerConfig::new(session)` and the `From<ClientSession>`/`From<ServerSession>` impls, and the `impl Into<RtspClientConfig>`/`impl Into<RtspServerConfig>` parameters that let a bare session stand in for a config. Build a config with `RtspClientConfig::default().with_session(session).with_timeouts(t)` (or the server's), so `with_*` is only ever a builder setter. Migration: `RtspClientConfig::new(s)` -> `RtspClientConfig::default().with_session(s)`, `connect(addr, session)` -> `connect(addr, RtspClientConfig::default().with_session(session))`.
- BREAKING: `AsyncRtspServer::accept(stream)` and `accept_with(stream, session)`. Migration: `AsyncRtspServer::from_stream(stream, RtspServerConfig::default())` (the default config's session takes its ids from the OS random source, exactly as `accept` did) and `from_stream(stream, RtspServerConfig::default().with_session(session))` for `accept_with`.

#### Added
- `AsyncRtspServer::from_stream(stream, RtspServerConfig)`: the one plain (or TLS-stream) server constructor, named as `AsyncRtmpClient::from_stream`/`AsyncRtmpServer::bind` are, beside `accept_tls` for an accepted `rtsps://` connection.

#### Changed
- BREAKING: `AsyncRtspClient::with_stream` and `AsyncRtspServer::with_stream` are now `from_stream` (a constructor named `with_*` read as a builder setter; `with_*` stays reserved for setters). Every constructor takes its `RtspClientConfig`/`RtspServerConfig` by value, no `impl Into` (#2660).
- BREAKING: `Error::TransportParse`, `HeaderSerialize`, `SessionParse`, `InterleavedFrame` and `InvalidSdp` now wrap reason enums (`TransportParseError`, `HeaderSerializeError`, `SessionParseError`, `FrameError`, `SdpParseError`) instead of `String`; `InterleavedFrame::payload` is now `bytes::Bytes` (zero-copy) (#2659).

---

Published from tag `rtsp-runtime-v0.12.0`.
