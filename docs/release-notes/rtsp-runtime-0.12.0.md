# rtsp-runtime 0.12.0

_Released 2026-11-15._

Minor-class release (0.11 -> 0.12). RTSP has one config per role: `RtspClientConfig` and `RtspServerConfig` replace the constructor closures and `impl Into` parameters, so every `with_*` is a builder setter and every constructor takes its config by value. Session ids are a typed `SessionId`, the keepalive clock is a `stage::Timestamp`, and the error variants that carried a reason string now carry a typed enum.

Version: 0.11.0 -> 0.12.0.

## Breaking changes and migration

### One config per role

`ClientSession::new(RtspClientConfig)` and `ServerSession::new(RtspServerConfig) -> Result<Self>` replace `ClientSession::new()` and `ServerSession::new(closure)`/`try_new(closure)`. The client config carries `user_agent` and `initial_cseq` (`with_user_agent`, `with_initial_cseq`; `ClientSession::default()` is a fresh session); the server config carries `session_ids`, `server_header`, `describe_sdp`, `server_ports` and `session_timeout`. The tokio adapter's config is `RtspClientIoConfig` (session + timeouts), so `RtspClientConfig` names the protocol config alone.

```rust
// before
let client = ClientSession::new();
let server = ServerSession::new(|| next_id());
// after
let client = ClientSession::new(RtspClientConfig::default())?;
let server = ServerSession::new(RtspServerConfig::default().with_session_ids(SessionIdSource::counter(seed)))?;
```

### Session ids are `SessionId`

Ids are a `SessionId` newtype rather than a raw `u64` (`SessionId::new`, `value`, `parse`, `Display` as 16 lower-case hex digits), and their source is `SessionIdSource` (`OsRandom`, the default, or `Counter(seed)` for deterministic tests) rather than an untyped closure. `ServerSession::session_id`, `AsyncRtspServer::session_id` and `ClientSession::session_id`/`AsyncRtspClient::session_id` return it. A client reads a peer's id leniently (upper case, or fewer hex digits, is the same session), so a non-conforming server still gets a `TEARDOWN`.

```rust
// before
let id: u64 = server.session_id();
// after
let id: SessionId = server.session_id();
println!("{id}");                      // 16 hex digits
let _: u64 = id.value();
```

### The keepalive clock is a `stage::Timestamp`

`ClientSession::mark_activity` and `ClientSession::handle_timeout` take a `broadcast_common::stage::Timestamp`, and `ClientSession::poll_timeout` returns one, instead of a microsecond `u64`.

```rust
// before
session.mark_activity(now_us);
let deadline: Option<u64> = session.poll_timeout();
// after
session.mark_activity(Timestamp::from_micros(now_us));
let deadline: Option<Timestamp> = session.poll_timeout();
```

### Config constructors are gone

`RtspClientConfig::new(session)`, `RtspServerConfig::new(session)`, the `From<ClientSession>`/`From<ServerSession>` impls and the `impl Into<RtspClientConfig>`/`impl Into<RtspServerConfig>` parameters are removed. Build a config with `RtspClientConfig::default().with_session(session).with_timeouts(t)` (or the server's).

```rust
// before
let config = RtspClientConfig::new(session).with_timeouts(t);
let client = AsyncRtspClient::connect(addr, session).await?;
// after
let config = RtspClientConfig::default().with_session(session).with_timeouts(t);
let client = AsyncRtspClient::connect(addr, RtspClientIoConfig::default().with_session(session)).await?;
```

### Adapter constructors are `from_stream`

`AsyncRtspClient::with_stream` and `AsyncRtspServer::with_stream` are `from_stream`, and `AsyncRtspServer::accept`/`accept_with` become `AsyncRtspServer::from_stream(stream, RtspServerConfig)`. Every constructor takes its config by value, with no `impl Into`.

```rust
// before
let server = AsyncRtspServer::accept_with(stream, session).await?;
// after
let server = AsyncRtspServer::from_stream(stream, RtspServerConfig::default().with_session(session)).await?;
```

### Error variants carry a typed cause

`Error::TransportParse`, `HeaderSerialize`, `SessionParse`, `InterleavedFrame` and `InvalidSdp` wrap reason enums (`TransportParseError`, `HeaderSerializeError`, `SessionParseError`, `FrameError`, `SdpParseError`) instead of `String`; `Error::MessageParse` carries `MessageParseError` (`Codec`, `TooLarge`, `Buffered`, `HeaderUnterminated`, `NotARequest`); `Error::Tls` is a `rustls::pki_types::InvalidDnsNameError` and `Error::TlsConfig` wraps `rustls::Error`. `InterleavedFrame::payload` is `bytes::Bytes` (zero-copy). A caller matches the failure instead of reading its text.

```rust
// before
if let Error::MessageParse { what, source } = err { eprintln!("{what}: {source}"); }
// after
if let Error::MessageParse { source: MessageParseError::TooLarge { at_least, limit }, .. } = err {
    eprintln!("a message of at least {at_least} bytes exceeds the {limit}-byte limit");
}
```

## Removed

- The `state` module's FSM transition helpers `is_state_neutral`, `client_next_state` and `server_next_state` are crate-private; `SessionState` stays public, and the transitions happen inside `ClientSession`/`ServerSession`.
- `AsyncRtspServer::accept` and `AsyncRtspServer::accept_with` (see `from_stream` above).

## Added

- `AsyncRtspServer::from_stream(stream, RtspServerConfig)`: the one plain (or TLS-stream) server constructor, beside `accept_tls` for an accepted `rtsps://` connection.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-auth` 0.7 -> 0.8.

---

Published from tag `rtsp-runtime-v0.12.0`.
