# rtsp-runtime 0.7.0 — 2026-09-26

Security release for `ServerSession`: `Session` ids are now unpredictable instead of a fixed
counter. **Upgrade if you run `ServerSession` or `io::AsyncRtspServer` against clients you do not
control.** This is a breaking release: `ServerSession::new` now takes the id source, and
`impl Default for ServerSession` is removed.

## Security

| Advisory | Before this release |
|---|---|
| GHSA-3rw9-cq7p-4v47 | Every `ServerSession` started its `Session` id counter at the same fixed value (`305419896`, i.e. `0x1234_5678`) and incremented from there, so a client of one session could predict — or simply reuse — another session's id. `Session` header validation on `PLAY`/`PAUSE`/`RECORD`/`TEARDOWN` was also missing, so a request naming any id (guessed or not) was accepted rather than rejected. |

## Breaking change

`ServerSession::new` now takes the `Session` id source:

```rust
// Before (0.6.x)
let session = ServerSession::new();

// After (0.7.0) — caller supplies a CSPRNG (RFC 2326 §3.4) as
// `impl FnMut() -> u64 + Send + 'static`
fn os_session_id() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("OS random source unavailable");
    u64::from_ne_bytes(bytes)
}
let session = ServerSession::new(os_session_id);

// io::AsyncRtspServer::accept / accept_tls supply the OS RNG for you this
// same way (new optional `getrandom` dependency under the `tokio` feature) —
// no source to pass at that call site.
```

`impl Default for ServerSession` is removed — there is no safe default id source. `Debug` is now
hand-written (it no longer derives, since the id-source closure isn't `Debug`).
`with_session_seed` keeps its existing signature but is now `#[doc(hidden)]`, documented for
deterministic tests only — do not use it to seed a real server's session ids.

## Behaviour changes

- **Random session ids.** `ServerSession` ids are 64 random bits rendered as 16 hex digits
  instead of the fixed counter, so separate connections no longer share ids and an id cannot be
  guessed from a previous one.
- **`Session` header now validated.** A request whose `Session` header names a different id, or
  a `PLAY`/`PAUSE`/`RECORD`/`TEARDOWN` naming none once a session exists, is answered
  `454 Session Not Found` (RFC 2326 §11.3.2, §12.37) instead of `200`.
- **SETUP reply carries one `Transport` spec.** The reply's `Transport` header now carries only
  the chosen (first) spec rather than every offered one (§12.39); `negotiated_transport()` and
  `ServerEvent::SessionSetup` hold that single spec.
- **Unparseable `Transport` is a protocol error, not a dropped connection.** SETUP with an
  unparseable `Transport` header is answered `461 Unsupported Transport` instead of returning
  `Err` (which previously dropped the connection).

## Migration

Pass a CSPRNG closure to `ServerSession::new` (e.g. `rand::random` or an OS RNG wrapper) if you
construct sessions directly. If you go through `io::AsyncRtspServer::accept`/`accept_tls`,
nothing changes — the OS RNG is supplied for you via the `tokio` feature's new `getrandom`
dependency. Remove any reliance on `ServerSession::default()` or on session ids being sequential
or predictable.

MSRV 1.95.0.
