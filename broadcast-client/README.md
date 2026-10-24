# broadcast-client

Generic sans-IO client session for live media: auth, redirects, reconnect/retry and buffering
decisions over a TS-over-HTTP source (`tshttp-runtime`) and an HLS source (`hls-runtime`).
The host performs only the HTTP transport; time is a caller-supplied monotonic `now_us`.

## Sources

`Config::kind` selects the source; `SourceKind::Auto` picks HLS for a `.m3u8`/`.m3u` URL path
(ASCII case-insensitive) and TS-over-HTTP otherwise.

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
and `Event::AudioRendition` reports the active rendition. Subtitle renditions are not implemented.

## Buffer events

`Event::Buffer(Start|Pause|Resume)` has two engines. Live TS-over-HTTP is driven by
`buffering-runtime` (`Config::buffer`). HLS (live and VOD) and finite TS (a response with
`Content-Length`, `Event::Finite`) use a deterministic playout gate (BC-5, `Config::playout`,
`PlayoutConfig`): `Start { at }` once every rendered track has buffered `start lead` past the
origin (the first sample of the first rendered video track, else audio; the minimum of the
tracks' media ends decides), at the end of the stream if it ends first; `Pause` when the playback
position reported with `Session::on_playback_position` reaches `pause_lead_us` (default 0) of the
buffered media and the stream has not ended; `Resume` once the lead past the held position reaches
the resume lead, or at once when the stream ends while paused. A `Discontinuity`, `rejoin()` or
variant switch resets the gate and `Start` fires again. Leads: `vod_start_lead_us` 1 s, live HLS
`min(3 x EXT-X-TARGETDURATION, live_start_lead_us)` with `live_start_lead_us` 6 s,
`resume_lead_us` defaults to the start lead. Separate audio renditions join the minimum.

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
