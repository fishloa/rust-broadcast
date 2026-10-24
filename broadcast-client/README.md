# broadcast-client

Generic sans-IO client session for live media: auth, redirects, reconnect/retry and buffering
decisions over a TS-over-HTTP source (`tshttp-runtime`), an HLS source (`hls-runtime`) and an
RTSP source (`rtsp-runtime`). The host performs only transport I/O (HTTP requests, or a TCP
connection for RTSP); time is a caller-supplied monotonic `now_us`.

## Sources

`Config::kind` selects the source; `SourceKind::Auto` picks RTSP for an `rtsp://` or `rtsps://` URL,
HLS for a `.m3u8`/`.m3u` URL path (ASCII case-insensitive) and TS-over-HTTP otherwise.

TS-over-HTTP announces its tracks once: the first media of a connection is held until every elementary stream
the PAT/PMT declares has resolved, or 4 MiB of input, 3 s of media or 3 s of wall clock (whichever is first)
has passed, then released behind ONE `Event::Tracks` with the whole resolved set, so a host can build its
pipeline from the first `Tracks`. A stream that resolves later is announced by a further `Event::Tracks` with the
merged set. Samples keep their order; `Status::Live` is delivered with the release.

## RTSP

Sources for SAT>IP tuners and IP cameras. RTP/RTCP travel interleaved over the RTSP TCP connection
(`RTP/AVP/TCP;interleaved`); UDP RTP is not supported. The host executes `Action::TcpConnect { conn,
host, port, tls }` (ports default to 554 for `rtsp`, 322 for `rtsps`; for `rtsps` the host performs
TLS and certificate validation, the session only sees plaintext) and reports the outcome with
`Session::on_connected(conn)` or `Session::on_closed(conn, Some(error), now_us)`. It then writes the
bytes of every `Action::Send { conn, bytes }` and feeds what it reads to `Session::on_data`.

The handshake is DESCRIBE, one SETUP per SDP media (H.264 video and AAC audio; interleaved
channels 0-1, 2-3, ...), then PLAY. Track ids are the 1-based order of the played media. A relative
`a=control` resolves against the DESCRIBE `Content-Base` (else `Content-Location`, else the request URL
with a trailing `/`); an absolute control for another scheme, host or port skips that section, as does any
media the client cannot play (PCMU audio, H.265, application data) - only an SDP with nothing playable
left fails. At most 16 sections are played; a SETUP reply that repeats or overlaps an earlier track's
interleaved channels, or carries no `Transport`, fails the connection. `Event::Tracks` is
emitted before the first sample of every connection, `Event::Sample { join: None }` per access
unit, `Status::Establishing` on connect and `Status::Live` at the first sample. An `OPTIONS`
keepalive is sent at half the server's `Session` `timeout` (clamped to 5 s..3600 s, 30 s when none
is declared); a non-success reply to it is ignored. `Event::Discontinuity` is emitted only
before the first sample of a connection that follows one that produced media (never for a connection
that failed or was redirected before playing); the end of a connection that had played reports
`Status::Reconnecting { attempt: 0 }` first. A session-level `a=control` is the URL of PLAY, the
keepalive and TEARDOWN. `close()` sends `TEARDOWN` before `Close`.
RTCP sender reports are forwarded to the depayloader; no receiver reports are sent. Sequence gaps
drop the damaged access unit and the stream continues (`Session::rtp_loss_count`); a malformed RTP
frame costs only itself (`Session::rtp_malformed_count`).

A `301`/`302`/`303`/`307`/`308` answer to a request is followed to its `rtsp://` or `rtsps://`
`Location` (never from `rtsps` to `rtsp`, at most `Config::max_redirects`; anything else, or a
missing `Location`, fails the attempt): the session asks for a `TcpConnect` to the new host and
port. Credentials follow only a redirect to the same scheme, host and port; to another authority
they are dropped, and a 401/407 there fails the attempt. A later reconnect starts from the
original URL. Digest `uri=` is the RTSP request URI in absolute form; for HTTP(S) requests it is
the origin-form request target (path and query), as RFC 7616 section 3.4 requires.

Credentials work as for HTTP (`Session::set_credentials`, Basic/Digest/Bearer through
`rtsp-runtime`): a 401/407 without credentials reports `Event::AuthRequired` and
`Status::WaitingForCredentials` and the session goes quiet on the open connection until
credentials arrive, which re-send the challenged request. Rejected credentials report
`Event::AuthRejected`. A 403 reports `Event::Forbidden`; a 404 on DESCRIBE reports
`Status::NotFound`. Reconnect and stall behaviour is exactly the TS-over-HTTP policy and uses the
same `Config::ts_http` values (`start_timeout_ms`, `stall_notice_ms`, `stall_reconnect_ms`,
`backoff_ms`, `backoff_cap_ms`). No `Event::Buffer` is produced for RTSP; `Config::extra_headers`
and cookies do not apply. Credentials in the URL's userinfo (`rtsp://user:pw@host/...`, also for
`http(s)://`) are percent-decoded into the initial credentials and stripped from request URIs; a
later `set_credentials` replaces them.

HLS (BC-3a): a single media playlist, or a master playlist with the variant chosen by
`Config::variant_policy` (`Event::Variants` reports the choice, `Session::select_variant` switches
and emits `Event::Discontinuity`). TS and fMP4 segments, VOD (`Status::Ended` once) and live
(blocking reload). Every request gets the origin-scoped auth layer and its own redirect budget.
Failed requests retry with the `Config::hls_backoff_ms` schedule (2, 4, 8 s, then
`hls_backoff_cap_ms`, 10 s) and report `Status::Reconnecting`; a 404/410 playlist also reports
`Status::NotFound`; schedule entries below 1 ms are raised to 1 ms. At most
`Config::hls_max_inflight_segments` (4) segment/part requests are outstanding at once, however long the
playlist; after `Status::Ended` of an HLS session `reconnect()`, `select_variant()` and
`select_audio_rendition()` do nothing. Bodies are bounded (`max_playlist_bytes`, `max_segment_bytes`) and requests
time out (`hls_request_timeout_us`, `hls_segment_timeout_us`). A byte-ranged segment must come back as
`206` with a matching `Content-Range`, or as a `200` the range is sliced out of; anything else fails. Video samples carry a `join` decision (`JoinPoint` at the first random-access point, `Drop(_)` before it, `Keep` after; audio is `None`), so a mid-GOP join starts at an IDR and the host skips `Drop` samples exactly as for TS-over-HTTP. A TS-segment elementary stream that cannot be carried (SCTE-35, ID3 or private data, a subtitle without a sample entry) is reported once per track as a non-fatal `Event::Error { kind: ErrorKind::TrackDropped { track_id, stream_type }, fatal: false }`; a stream with no playable track reports those and ends. Separate audio renditions are played by a second adapter
(`Config::auto_select_audio`, `Config::preferred_audio_language`, `Session::select_audio_rendition`);
audio track ids are offset by `AUDIO_TRACK_ID_OFFSET` (1000; a main-playlist track at or above it is
dropped and reported once as `ErrorKind::TrackIdReserved`), `Event::Tracks` carries the merged set
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
the resume lead, or at once when the stream ends while paused. A position the host reported before
`Start` is discarded at `Start`: report the position again only after applying `Start.at`. A `Discontinuity`, `rejoin()`,
variant switch or `reconnect()` resets the gate and `Start` fires again from the new timeline. Leads: `vod_start_lead_us` 1 s, live HLS
`min(3 x EXT-X-TARGETDURATION, live_start_lead_us)` with `live_start_lead_us` 6 s,
`resume_lead_us` defaults to the start lead and is never less than `pause_lead_us` plus 1 us (a
smaller value would pause and resume on every update). A `set_rendered_tracks` made before the gate
engages (finite TS) is applied when it does. Separate audio renditions join the minimum.

## Timeline jumps

A new timeline that no TS discontinuity flag announces (origin restart, a recorder looping a file, a
channel swap behind one URL) is detected by the session itself (`Config::timeline_jump`,
`TimelineJumpConfig`, on by default; `None` disables it). Per video track, between consecutive kept
samples (`join` not `Drop`), a step below `-backward_us` (1 s) or above the session-clock time
between the two deliveries plus `forward_slack_us` (5 s) starts a new timeline; inside one burst the
elapsed time is zero. Audio is watched the same way only for the first rendered audio track of a
stream without video. Before the first sample of the new timeline the session emits
`Event::Discontinuity`, re-arms the join, hints `PcrDiscontinuity` (live TS), and resets the playout
gate, so `Start`/`Resume` follows from the new timeline's media. B-frame reordering stays far inside
the backward threshold. Applies to TS-over-HTTP, HLS (main and audio rendition) and RTSP.

After a detected jump every other track delivers nothing until its first sample lies within
`forward_slack_us` of the new timeline's first sample (trailing samples of the old timeline queued
behind the late video sample are dropped, counted by `Session::stale_samples_dropped()`). A PCR
discontinuity that coincides with a jump hints and re-arms once. A reconnect, a `Discontinuity`
from a new connection or a jump re-arms the live join and the buffering controller (the reconnected
stream waits for an IDR and `Start`/`Resume` follow from the new timeline). Buffer positions
are the time stamps converted to 27 MHz; a timeline whose first kept sample has a negative time stamp
(a jump whose unrolled timestamps go below zero) is measured from that sample, so `0` is the first
kept sample and positions never clamp to a flat zero.

## Encrypted HLS

`METHOD=AES-128` playlists are decrypted inside the session: the key request is a normal request
(origin-scoped credentials, cookies and `extra_headers`, own redirect budget, `hls_key_timeout_us`,
`max_key_bytes`, shared retry/backoff) and the key is cached for the session. `Event::Error { kind,
fatal }` reports why playback cannot proceed: `ErrorKind::UnsupportedEncryption` (SAMPLE-AES,
FairPlay/Widevine key formats, ...) is fatal immediately; `KeyFetchFailed`, `BadKey` and
`DecryptFailed` are reported once while the session retries and become fatal after
`Config::max_key_failures` consecutive failures. A fatal error closes all connections and the session
goes silent. 401/403 on a key request stay `AuthRequired`/`AuthRejected`/`Forbidden`.

## Host contract

- Drive the session with `poll(now)` until it returns `None` and drain `next_event()` after EVERY callback
  (`on_response`, `on_data`, `on_closed`, `on_connected`, `on_playback_position`, `set_credentials`,
  `reconnect`): actions and events are produced inside the callbacks, not only by `poll`.
- `on_response(conn, status, headers, now_us)` takes the host clock at which the response head
  arrived, so a redirect or 401 replay after a long blocking reload gets a fresh deadline and cookie
  `Max-Age` counts from the response.
- Live TS gets `Event::Buffer` only if the host calls `on_playback_position(now, pos)` every display
  tick (`pos` is `None` before `Start`). `Start.at_27mhz` / `Resume.at_27mhz` are the 27 MHz position the
  player must set (the sample-time domain of the samples); a demuxer-level discontinuity (PCR
  discontinuity flag, re-anchor) is invisible to the host except through these events, so `Resume.at`
  can jump.
- `Action::HttpRequest::byte_range` is `(offset, length)`, not `(start, end)`; the host builds
  the `Range: bytes=offset-(offset+length-1)` header itself.
- `Action::WakeAt(t)` means "poll at or after `t`"; the session drops one that is not after the `now` of
  the poll that would return it.
- HLS: the final (post-redirect) URL of a playlist response is the base of its relative segment, key
  and map URIs (RFC 3986 section 5.1.3); credentials stay scoped to the session origin.

- `Session::close()` is terminal: queued actions other than `Close` are dropped, every live connection gets
  exactly one `Close` (RTSP first queues a `Send` carrying `TEARDOWN` when a session was set up), then `poll` and `next_event` return
  nothing. A response, data or close callback for a connection the session already closed is ignored.
- `Action::WakeAt(t)`: poll again at or after `t`. A host that polls only at `WakeAt` times sees every timer
  (start timeout, stall notices, stall reconnect, backoff, keepalive, HLS retries) of all three sources.
- `Session::set_credentials` returns `Err(Error::InvalidCredentials)` for a token, user name or password with
  control characters and keeps the previous credentials. A `stale=true` Digest re-challenge is replayed once
  per request; a second one is `Event::AuthRejected`.
- `Config::max_queued_events` (default 100 000) bounds the whole event queue of a host that does not drain
  `Session::next_event`: transient `Status`/`Starved` events coalesce to the latest, then the oldest `Sample`s
  are dropped (counted by `Session::dropped_events()`), each burst leaving one `Event::Discontinuity`;
  tracks, errors, buffer events and terminal statuses are never dropped.
- Redirects are followed only to `http`/`https` targets and never from `https` to `http`. A 401/407 from an
  origin other than the session origin is a connect failure (backoff), never a credential prompt.
- `Action` implements `Debug` without header values or `Send` payloads.
- `select_audio_rendition` after `Status::Ended` is accepted and ignored; so are `select_variant` and
  `reconnect` for HLS (a finite TS source restarts on `reconnect`).
- `Event::AuthRequired::resource_metadata` is chosen by the server that sent the challenge: validate it
  (https, expected host, no private/loopback address) before fetching it.
- After a fatal `Event::Error` the session emits nothing further.

## Cookies and custom headers

Responses of the session origin may set cookies (`Set-Cookie`, any status including 401 and 3xx);
the jar is host-only and scoped to the origin, honours `Path` (RFC 6265 path-match), `Secure` and
`Max-Age` (relative to the session clock) and ignores `Domain` and `Expires`. A `Secure` cookie from an `http` origin, a `__Secure-` cookie without
`Secure`, a `__Host-` cookie without `Secure` and `Path=/` (or with `Domain`) and a `Path` over 1024 bytes
are rejected. Cookies ride on every
request to the origin (TS-HTTP and HLS playlists, segments, reconnects) and never to another origin.
Seeded cookies obey the same rules (over 4096 bytes dropped; `__Secure-`/`__Host-` names only on an
`https` origin) and the `Cookie` header is capped at 16 KiB. Seed with `Config::initial_cookies`; persist with `Session::cookies()` (values are secrets, never in
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
