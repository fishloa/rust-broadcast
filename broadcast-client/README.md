# broadcast-client

Generic sans-IO client session for live media: auth, redirects, reconnect/retry and buffering
decisions over a TS-over-HTTP source (`tshttp-runtime`), an HLS source (`hls-runtime`) and an
RTSP source (`rtsp-runtime`). The host performs only transport I/O (HTTP requests, or a TCP
connection for RTSP); time is a caller-supplied monotonic `now_us`.

## Sources

`Config::kind` selects the source; `SourceKind::Auto` picks RTSP for an `rtsp://` or `rtsps://` URL,
HLS for a `.m3u8`/`.m3u` URL path (ASCII case-insensitive) and TS-over-HTTP otherwise.

## RTSP

Sources for SAT>IP tuners and IP cameras. RTP/RTCP travel interleaved over the RTSP TCP connection
(`RTP/AVP/TCP;interleaved`); UDP RTP is not supported. The host executes `Action::TcpConnect { conn,
host, port, tls }` (ports default to 554 for `rtsp`, 322 for `rtsps`; for `rtsps` the host performs
TLS and certificate validation, the session only sees plaintext) and reports the outcome with
`Session::on_connected(conn)` or `Session::on_closed(conn, Some(error), now_us)`. It then writes the
bytes of every `Action::Send { conn, bytes }` and feeds what it reads to `Session::on_data`.

The handshake is DESCRIBE, one SETUP per SDP media (H.264 video and AAC audio; interleaved
channels 0-1, 2-3, ...), then PLAY. Track ids are the 1-based SDP media order. `Event::Tracks` is
emitted before the first sample of every connection, `Event::Sample { join: None }` per access
unit, `Status::Establishing` on connect and `Status::Live` at the first sample. An `OPTIONS`
keepalive is sent at half the server's `Session` `timeout` (clamped to 5 s..3600 s, 30 s when none
is declared); a non-success reply to it is ignored. `close()` sends `TEARDOWN` before `Close`.
RTCP sender reports are forwarded to the depayloader; no receiver reports are sent. Sequence gaps
drop the damaged access unit and the stream continues (`Session::rtp_loss_count`); a malformed RTP
frame costs only itself (`Session::rtp_malformed_count`).

Credentials work as for HTTP (`Session::set_credentials`, Basic/Digest/Bearer through
`rtsp-runtime`): a 401/407 without credentials reports `Event::AuthRequired` and
`Status::WaitingForCredentials` and the session goes quiet on the open connection until
credentials arrive, which re-send the challenged request. Rejected credentials report
`Event::AuthRejected`. A 403 reports `Event::Forbidden`; a 404 on DESCRIBE reports
`Status::NotFound`. Reconnect and stall behaviour is exactly the TS-over-HTTP policy and uses the
same `Config::ts_http` values (`start_timeout_ms`, `stall_notice_ms`, `stall_reconnect_ms`,
`backoff_ms`, `backoff_cap_ms`). No `Event::Buffer` is produced for RTSP; `Config::extra_headers`
and cookies do not apply; credentials in the URL's userinfo are not used (they are stripped from
request URIs) - pass them with `set_credentials`.

HLS (BC-3a): a single media playlist, or a master playlist with the variant chosen by
`Config::variant_policy` (`Event::Variants` reports the choice, `Session::select_variant` switches
and emits `Event::Discontinuity`). TS and fMP4 segments, VOD (`Status::Ended` once) and live
(blocking reload). Every request gets the origin-scoped auth layer and its own redirect budget.
Failed requests retry with the `Config::hls_backoff_ms` schedule (2, 4, 8 s, then
`hls_backoff_cap_ms`, 10 s) and report `Status::Reconnecting`; a 404/410 playlist also reports
`Status::NotFound`. Bodies are bounded (`max_playlist_bytes`, `max_segment_bytes`) and requests
time out (`hls_request_timeout_us`, `hls_segment_timeout_us`). Separate audio renditions are played by a second adapter
(`Config::auto_select_audio`, `Config::preferred_audio_language`, `Session::select_audio_rendition`);
audio track ids are offset by `AUDIO_TRACK_ID_OFFSET` (1000), `Event::Tracks` carries the merged set
and `Event::AudioRendition` reports the active rendition. Subtitle renditions and `Event::Buffer`
for HLS are not implemented.

## Encrypted HLS

`METHOD=AES-128` playlists are decrypted inside the session: the key request is a normal request
(origin-scoped credentials, cookies and `extra_headers`, own redirect budget, `hls_key_timeout_us`,
`max_key_bytes`, shared retry/backoff) and the key is cached for the session. `Event::Error { kind,
fatal }` reports why playback cannot proceed: `ErrorKind::UnsupportedEncryption` (SAMPLE-AES,
FairPlay/Widevine key formats, ...) is fatal immediately; `KeyFetchFailed`, `BadKey` and
`DecryptFailed` are reported once while the session retries and become fatal after
`Config::max_key_failures` consecutive failures. A fatal error closes all connections and the session
goes silent. 401/403 on a key request stay `AuthRequired`/`AuthRejected`/`Forbidden`.

## Cookies and custom headers

Responses of the session origin may set cookies (`Set-Cookie`, any status including 401 and 3xx);
the jar is host-only and scoped to the origin, honours `Path` (RFC 6265 path-match), `Secure` and
`Max-Age` (relative to the session clock) and ignores `Domain` and `Expires`. Cookies ride on every
request to the origin (TS-HTTP and HLS playlists, segments, reconnects) and never to another origin.
Seed with `Config::initial_cookies`; persist with `Session::cookies()` (values are secrets, never in
`Debug`). `Config::extra_headers` are sent only to the origin and validated at `Session::open`
(`Error::InvalidHeader`; `Authorization`, `Cookie`, `Host`, `Content-Length`, `Transfer-Encoding`,
`Connection`, `Range` are `Error::ReservedHeader`).

## Quick start

```rust
use broadcast_client::{Config, Session};

let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
while let Some(action) = session.poll(0) {
    // host performs I/O based on `action`
    let _ = action;
}
```
