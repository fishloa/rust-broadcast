# broadcast-client

Generic sans-IO client session for live media: auth, redirects, reconnect/retry and buffering
decisions over a TS-over-HTTP source (`tshttp-runtime`), an HLS source (`hls-runtime`), a DASH
source (`dash-runtime`) and an RTSP source (`rtsp-runtime`). The host performs only transport I/O (HTTP requests, or a TCP
connection for RTSP); time is a caller-supplied monotonic `now_us`.

## Sources

`Config::kind` selects the source; `SourceKind::Auto` picks RTSP for an `rtsp://` or `rtsps://` URL,
HLS for a `.m3u8`/`.m3u` URL path and DASH for a `.mpd` URL path (both ASCII case-insensitive) and
TS-over-HTTP otherwise. An explicit kind wins over the extension (`SourceKind::Dash` plays an MPD served
from any path; DASH is HTTP-only, so `rtsp://` with `SourceKind::Dash` is `Error::UnsupportedScheme`).

TS-over-HTTP announces its tracks once: the first media of a connection is held until every elementary stream
the PAT/PMT declares and that has sent a packet has resolved (plus a 0.5 s settle for a stream that has not sent
yet), or 4 MiB of input, 1 s of media or 1 s of wall clock (whichever is first) has passed, then released behind ONE `Event::Tracks` with the whole resolved set, so a host can build its
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
media the client cannot play (PCMU audio, H.265, application data); each skipped section is reported once
per DESCRIBE as a non-fatal `ErrorKind::TrackDropped` (`track_id` is the 1-based position of the `m=` line, not
the id of an announced track; the same media is not reported again on a reconnect to the same SDP, a different
SDP reports its own drops). An SDP
with nothing playable left fails: when a skipped section names an unsupported codec by `a=rtpmap` (H.265,
PCMU, ...) the failure is ONE fatal `ErrorKind::UnsupportedCodec { fourcc }` (the `rtpmap` encoding name) and
the session goes silent instead of reconnecting; a section that names no codec, or a played codec with an
unusable `a=fmtp`, or an SDP that does not parse, is an ordinary failed attempt (backoff and retry). At most
16 sections are played; a SETUP reply that repeats or overlaps an earlier track's
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
Failed requests retry with the `Config::backoff_ms` schedule (2, 4, 8 s, then
`backoff_cap_ms`, 10 s) and report `Status::Reconnecting`; a 404/410 playlist also reports
`Status::NotFound`; schedule entries below 1 ms are raised to 1 ms. A `429` or `503` with a `Retry-After` in seconds stretches the wait to at least that long (clamped to 60 s; the HTTP-date form is ignored, the session has no wall clock). A segment's retries are counted per segment: its failures walk the schedule and the count resets only when that segment is delivered as media. A `200` whose body is not media for its container (an HTML error page, random bytes, an empty body, a fragment without its `mdat`) is a failed fetch, never a consumed segment. After `Config::max_segment_failures` (8) consecutive failures of one segment (`0`: the first is final; `u32::MAX`: retry forever) a VOD playlist ends with one fatal `ErrorKind::SegmentFetchFailed { status }`, and a live one skips the segment: one non-fatal `SegmentFetchFailed`, then `Event::Discontinuity`, and playback continues with the next segment. A playlist reload is never given up on. Init fetches time out after `request_timeout_us`. At most
`Config::max_inflight_segments` (4) segment/part requests are outstanding at once, however long the
playlist; after `Status::Ended` of an HLS session `reconnect()`, `select_variant()` and
`select_audio_rendition()` do nothing. Bodies are bounded (`max_manifest_bytes`, `max_segment_bytes`) and requests
time out (`request_timeout_us`, `segment_timeout_us`). A byte-ranged segment must come back as
`206` with a matching `Content-Range`, or as a `200` the range is sliced out of; anything else fails. Video samples carry a `join` decision (`JoinPoint` at the first random-access point, `Drop(_)` before it, `Keep` after; audio is `None`), so a mid-GOP join starts at an IDR and the host skips `Drop` samples exactly as for TS-over-HTTP. A TS-segment elementary stream that cannot be carried (SCTE-35, ID3 or private data, a subtitle without a sample entry) is reported once per track as a non-fatal `Event::Error { kind: ErrorKind::TrackDropped { track_id, stream_type, fourcc }, fatal: false }`; a stream with no playable track reports those and ends. The same holds for an fMP4 init: a track the demuxer cannot model is reported once per track id (an encrypted `encv`/`enca` sample entry as `ErrorKind::UnsupportedEncryption { method: "encv" }` — CENC/CBCS is never decrypted, whatever the playlist tags say — any other as `TrackDropped { fourcc: Some(..) }`), and a stream whose every track is skipped ends with a FATAL error of that kind, not `Ended`. A progressive MP4 served as HLS media is fatal `ErrorKind::UnsupportedContainer { reason: "non_fragmented_mp4" }`; a fragment whose sample data lies outside its own segment (absolute `tfhd` base-data-offset; CMAF uses default-base-is-moof) is lost and reported once as non-fatal `UnsupportedContainer { reason: "damaged_fragment" }`; a playlist or segment over `max_manifest_bytes`/`max_segment_bytes` reports `ErrorKind::BodyTooLarge { field }` once per field. Every segment is decoded with the init of its own `EXT-X-MAP` run (RFC 8216 §4.4.4.5), so an init change mid-playlist announces a new `Event::Tracks` after the `Event::Discontinuity`. Separate audio renditions are played by a second adapter
(`Config::auto_select_audio`, `Config::preferred_audio_language`, `Session::select_audio_rendition`);
audio track ids are offset by `AUDIO_TRACK_ID_OFFSET` (1000; a main-playlist track at or above it is
dropped and reported once as `ErrorKind::TrackIdReserved`), `Event::Tracks` carries the merged set
and `Event::AudioRendition` reports the active rendition. Subtitle renditions are not implemented: a `SUBTITLES` rendition is listed in `Event::Variants` (`renditions`) but never fetched, and no event reports it; fetching and rendering it is the host's job.

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
`Config::max_key_failures` consecutive failures. A failed `EXT-X-MAP` initialization-section fetch (any
status, a transport error, a timeout) is retried the same way, VOD and live alike, and reported as
`ErrorKind::InitFetchFailed { status }`, and an init that arrives but holds no `moov` box as
`ErrorKind::InitDecodeFailed`: once while retrying, fatal after `max_key_failures` consecutive failures
with no decoded samples in between. A fatal error closes all connections and the session
goes silent. 401/403 on a key request stay `AuthRequired`/`AuthRejected`/`Forbidden`.

## Host contract

- Drive the session with `poll(now)` until it returns `None` and drain `next_event()` after EVERY callback
  (`on_response`, `on_data`, `on_closed`, `on_connected`, `on_playback_position`, `set_credentials`,
  `reconnect`): actions and events are produced inside the callbacks, not only by `poll`.
- `on_response(conn, status, headers, now_us)` takes the host clock at which the response head
  arrived, so a redirect or 401 replay after a long blocking reload gets a fresh deadline and cookie
  `Max-Age` counts from the response.
- Live TS decides `Event::Buffer` inside `on_playback_position(now, pos)`: a host that never calls it
  gets no `Start` at all, and calling it less often than every display tick only delays `Start` (a
  captured stream missing a declared audio started at 5.3 s with ticks only after the data, 1.6 s with
  a tick per chunk); call it every display tick (`pos` is `None` before `Start`). `Start.at_27mhz` / `Resume.at_27mhz` are the 27 MHz position the
  player must set (the sample-time domain of the samples); a demuxer-level discontinuity (PCR
  discontinuity flag, re-anchor) is invisible to the host except through these events, so `Resume.at`
  can jump.
- Live TS rendered set: until `set_rendered_tracks` is called the buffer controller judges the default
  set, the lowest `TrackId` of each kind among the announced tracks (`Session::default_rendered_tracks()`
  returns `(video, audio)`; it moves when a lower id is announced later, not by announce order). The session
  runs ahead of the player, so `set_rendered_tracks` may name a track that already emitted samples: the
  controller is seeded from the progress the session kept for it (media end from the join point, audio
  past the join PTS), so a late declaration neither pauses nor blinds the controller. A track that never
  emitted is waited for as before. `rejoin()` and a timeline discontinuity discard that progress. The
  HLS/finite playout gate uses the same default rule. The default treats every video codec as video
  (AVC, HEVC, VVC, AV1, VP9, VP8, MPEG-2) and gives each video sample a join decision: the NAL-aware
  `LiveJoin` for AVC/HEVC/VVC, and for the others the first sync sample is the join point (`JoinPoint`,
  earlier samples `Drop`, leading pictures before the join point in presentation order `Drop`).
- Live TS declaration semantics (`set_rendered_tracks`): an id no `Event::Tracks` has announced is ignored,
  remembered, applied when a later announcement names it, and reported once per declaration as a non-fatal
  `ErrorKind::UnknownRenderedTrack { track_id }` (nothing is reported before the first announcement; a
  declaration made earlier simply waits for it). A non-empty declaration none of whose ids is announced
  renders the default set meanwhile (a typo never freezes `Start`); an empty declaration renders nothing. A
  declared track that vanishes (PMT drop, reconnect to another stream) and comes back under a new id (the
  demuxer never reuses an id) is replaced by the lowest announced track of its kind (video or audio) that the
  host did not declare itself, so an explicit video+audio declaration keeps playing across a PMT flip; if the
  original id returns it wins again. HLS and RTSP sessions pass the declaration through unchanged (the HLS
  playout gate waits for a declared separate audio rendition until it is announced).
- Every new TS-over-HTTP connection announces its own track set: `Event::Tracks` is the full set, so a track the
  new connection does not carry is absent from the first `Tracks` after the reconnect (and from
  `default_rendered_tracks()`), never a union with the old connection's.
- Output is independent of how the host chunks the bytes: the `Sample` sequence (track, pts, dts, join, data) and
  the position of every `Discontinuity` among the samples are identical for every read size at a given
  arrival-clock policy. What depends on arrival time, by nature, is: `Buffer` events, `Status`/`Starved`, the
  grouping and timing of `Tracks`/`DisplayChanged` (the hold releases at the first read boundary at which its
  condition holds), and timers (a connection with no sample within `start_timeout_ms` reconnects at a
  read-quantised byte offset).
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
- Live TS-over-HTTP holds the first media of each connection behind ONE `Event::Tracks`. The hold ends when
  every declared stream has resolved, or when every declared stream that has carried a payload packet has
  resolved and 0.5 s (media time or session clock) have passed since the last one did: a declared stream that
  sends nothing is never waited for. Backstops: `Config::tracks_hold_max_wall_us` (1 s),
  `tracks_hold_max_media_us` (1 s), `tracks_hold_max_input_bytes` (4 MiB), `tracks_hold_quiet_input_bytes`
  (512 KiB); a stream that sends but does not resolve (scrambled, sparse data) costs up to the wall limit.
  A host with a tighter first-frame budget lowers `tracks_hold_max_wall_us`, or sets `0` (also for
  `tracks_hold_max_media_us`) to disable the hold and take tracks as each stream resolves.
  After the first media, `Start` also waits for a PMT-declared audio/video stream that has no track yet, at most
  `Config::start_pending_track_wait_us` (default 1.5 s; `0` starts on the tracks announced so far, which can
  flap `Start` then `Pause` when audio arrives late; `u64::MAX` waits until the first `Start`). With a start lead
  below the default 1.8 s (zenith-tv: 0.5 s) and a first-frame budget to protect, 0.5 s to 1 s covers the measured
  audio resolution gaps (0.16 s to 0.48 s of media) at a bounded cost; `0` gives the earliest `Start`.
  `Session::open` returns `Error::InvalidConfig` when the quiet limit is `0` or exceeds the input limit.
  It also rejects every other value that silently disables output: `max_queued_events`, `max_manifest_bytes`,
  `max_segment_bytes`, `max_key_bytes`, `request_timeout_us`, `segment_timeout_us`,
  `hls_key_timeout_us`, `timeline_jump.backward_us` and `timeline_jump.forward_slack_us` of `0`, an invalid
  `dash` field (see the clock section), and a
  `buffer` that `buffering_runtime::ControllerConfig::validate` rejects. `ts_http` values are clamped to the
  nearest valid ones (`tshttp_runtime::Config::clamped`), `max_key_failures: 0` behaves as `1`, `max_segment_failures: 0` makes the first segment failure final, and a
  playout lead that the stream cannot reach (`u64::MAX`) keeps the gate closed.
  The hold never changes the sample sequence (track, pts, dts, sync flag, join decision, bytes): it is
  identical to the disabled hold. Host contract: the held samples are delivered as one burst behind the
  `Tracks`, up to the hold long; `Buffer::Start` follows within a few hundred ms. A host must accept that
  burst without lateness-based dropping and without a decoder input queue smaller than the hold (1 s of
  media by default). Before the early release, a captured rai-gulp run with a 3 s hold released 401
  samples in one burst before `Start` (229 with no hold); those samples match the no-hold run byte for byte,
  so a decode failure after such a burst is host-side.
- `Action` implements `Debug` without header values or `Send` payloads.
- `select_audio_rendition` after `Status::Ended` is accepted and ignored; so are `select_variant` and
  `reconnect` for HLS and DASH (a finite TS source restarts on `reconnect`). A DASH `reconnect` starts over
  from a fresh MPD request and announces `Event::Discontinuity` first; cookies and credentials are kept.
- `Event::AuthRequired::resource_metadata` is chosen by the server that sent the challenge: validate it
  (https, expected host, no private/loopback address) before fetching it.
- After a fatal `Event::Error` the session emits nothing further.

## Clock, `Retry-After` dates and the DASH knobs

The session never reads a clock. Besides the monotonic `now_us` every call takes, a host can give it
the UTC time once:

- `Session::set_wall_clock(now_us, unix_us)`: "at the monotonic instant `now_us` my UTC clock read
  `unix_us` microseconds since 1970-01-01T00:00:00Z" (POSIX time, no leap seconds). Read both clocks back to
  back. Call it any time and as often as you like (after `open` and before the first `poll`, every ten
  minutes, on a system clock-change notification, after a device wakes): the latest call replaces the
  previous anchor, an earlier `now_us` than a previous call is accepted, no value is rejected.
- What it changes today: an HLS `429`/`503` whose `Retry-After` is an HTTP-date (RFC 9110 5.6.7: IMF-fixdate,
  RFC 850 and asctime forms) waits until that date, counted from the anchor, clamped to 60 s like the
  delta-seconds form, and never shorter than the backoff step. With no anchor the date is counted from the
  response's own `Date` header (the server's clock cancels out); with neither the header is ignored as before.
  A date in the past waits nothing extra; garbage is ignored.
- DASH uses the anchor for dynamic MPDs only (segment availability and the live edge are defined on UTC); a
  static MPD and every other source need no clock. The anchor is the second rung of the clock ladder: a
  measured `UTCTiming` comes first (it is the packager's own time authority, sampled by the session through
  its own requests), then `set_wall_clock`, then the MPD response's `Date` header; with `DashClockPolicy::PreferHost`
  the host clock comes first and no `UTCTiming` request is made. A dynamic MPD with no source at all is the
  fatal `ErrorKind::ClockUnavailable`. Call `set_wall_clock` between `Session::open` (which performs no I/O)
  and the first `poll`; calling it later or again is fine and a step is detected and followed.
- `Event::ClockSynced { source: ClockSource, uncertainty_us }` is emitted by the DASH source when it first has
  a UTC clock and when the clock's source changes (not on a re-sync from the same source), before the first
  sample that depends on it. `ClockSource` is `UtcTiming`, `Host` or `MpdDate`; `uncertainty_us` is the
  half-width of the error bar.
- `Action::HttpRequest::method` is `"GET"` or `"HEAD"` (the DASH `http-head` clock method issues `HEAD` for a
  `UTCTiming` element): pass `method` through verbatim and do not assume GET. A `HEAD` response has no body;
  the session ignores its `Content-Length`.
- `Config::dash: DashConfig` holds the DASH-only knobs (`clock_policy`, `utc_timeout_us` 2 s, `utc_wait_us`
  5 s, `utc_max_rtt_us` 2 s, `utc_resync_us` 600 s, `host_clock_uncertainty_us` 500 ms, `live_delay_us`,
  `availability_margin_us`, `early_retry_us` 250 ms, `early_grace_us` 1 s, `min_mpd_refresh_us` 1 s,
  `use_availability_time_offset`, `cross_origin_urls`). `Session::open` rejects, as
  `Error::InvalidConfig { field: "dash.<name>" }`, a zero `utc_timeout_us`, `utc_wait_us`, `utc_max_rtt_us`,
  `utc_resync_us`, `early_retry_us` or `min_mpd_refresh_us`, and a `utc_wait_us` shorter than `utc_timeout_us`;
  it does so for every source kind, because a config is a value, not a promise to play DASH. `u64::MAX` is
  accepted everywhere zero is not (read as "never"); zero is accepted for `host_clock_uncertainty_us`,
  `availability_margin_us`, `early_grace_us` and `live_delay_us`.

## DASH

`SourceKind::Dash` plays an MPD through `dash-runtime`: static (VOD) and dynamic (live) presentations,
`SegmentTemplate` (`$Number$`, `$Time$`), `SegmentTimeline`, `SegmentList` and `SegmentBase` with a `sidx`
(on-demand profile), multiple Periods, fMP4/CMAF segments. The host makes the requests the session asks for
and answers them exactly as for HLS; what differs is below.

**Host contract.**
- `set_wall_clock` between `open` and the first `poll` (see the clock section): without it a dynamic MPD with
  neither `UTCTiming` nor a `Date` header is fatal. Static MPDs need nothing.
- Pass `Action::HttpRequest::method` through (`HEAD` occurs) and honour `byte_range` (`(offset, length)`):
  init segments, `sidx` indexes and on-demand subsegments are byte-range requests. A server that ignores `Range`
  and answers `200` with the whole file is handled (the range is cut out); a `206` must carry an exact
  `Content-Range`.
- The MPD refresh is conditional: the session sets `If-None-Match` / `If-Modified-Since` from the last
  `ETag` / `Last-Modified`, and a `304` is accepted. Report response headers verbatim, `Date`, `ETag`,
  `Last-Modified` and `Set-Cookie` included.
- Cookies, `Config::extra_headers` and credentials ride on every request to the session origin (the MPD's) and on
  no request to another origin: a `UTCTiming` server or a CDN `BaseURL` on another origin gets none. Redirects
  follow `max_redirects` per request; the final URL of an MPD redirect becomes the base of every relative
  reference (ISO/IEC 23009-1 A.11), a `Location` element in the MPD wins over it.
- Samples carry the container media timeline (`tfdt`), not a Period-rebased one. `Event::Discontinuity` marks a
  Period that does not continue the previous one (no `urn:mpeg:dash:period-continuity:2015` signal or a changed
  init), a skipped segment and a live join that fell behind.

**Shared knobs.** `max_redirects`, `extra_headers`, cookies, `variant_policy` (`max_bandwidth`, `max_height`,
`supported_codecs`), `preferred_audio_language`, `auto_select_audio`, `backoff_ms`, `backoff_cap_ms`,
`max_segment_failures`, `max_inflight_segments`, `request_timeout_us` (MPD, init, index), `segment_timeout_us`,
`max_manifest_bytes` (MPD), `max_segment_bytes`, `max_key_failures` (consecutive init and MPD-parse failures),
`playout`, `timeline_jump` and `max_queued_events` apply as for HLS. A `UTCTiming` request is bounded by
`dash.utc_timeout_us` and its body by 4 KiB. `Config::hls_key_timeout_us` and `max_key_bytes` are HLS-only.

**What the host sees.** `Event::Variants` lists the Representations of the video AdaptationSets of the current
Period in MPD order (`VariantInfo::url` is the Representation's initialization URL) and, as `renditions`, the
audio AdaptationSets (`name` is the first `Label`, else `@lang`, else the first Representation's `@id`; `default`
is Role `main`; `url` is the init URL). It is emitted once per Period. `select_variant(i)` takes an index into
those variants, `select_audio_rendition(Some(i))` an index into those renditions (`None` stops audio; it starts
again with a later `Some`). A choice takes effect at the next segment boundary and is kept across Periods by
Representation properties (bandwidth, size, codecs) and by rendition name and language, else the Period's own
pick stands. Automatic audio selection follows `preferred_audio_language` (BCP 47 prefix, ASCII
case-insensitive, never a commentary or description role), then Role `main`, then the first set;
`auto_select_audio = false` plays video only until a rendition is chosen. Track ids are 1-based, video then
audio, session-global and stable across a Representation switch (`AUDIO_TRACK_ID_OFFSET` is HLS-only).
`Status`: `Establishing` at the first successful response, `Live` at the first sample, `NotFound` for a 404/410
of the first MPD, `Reconnecting` from every scheduled retry, `Ended` when every selected segment of a
presentation that ends was delivered. The playout gate runs in VOD mode for a static MPD and in live mode for a
dynamic one, with a start lead of `min(3 x maxSegmentDuration, playout.live_start_lead_us)`.

**Errors.** `MpdParse { reason }` (`xml`, `invalid_utf8`, `not_mpd`, `invalid`, `too_large`; non-fatal, once per
kind until samples arrive, fatal after `max_key_failures` consecutive bodies with no good MPD between),
`MpdInvalid { reason }` (no Period, no playable Representation, missing `availabilityStartTime`; fatal),
`UnsupportedAddressing { reason }` (`presentation_type_list` is fatal; a Representation that cannot be addressed
or leaves the MPD origin under `cross_origin_urls = false` is dropped, non-fatal, and fatal only when nothing is
left), `UnsupportedFeature { feature }` (`preselection`, `xlink`, `mpd_patch`, `event_stream`, `low_latency`,
`utc_timing_scheme`, `utc_cross_origin`; once per feature per session, never fatal), `LiveEdgeLost` (the session
fell outside the time-shift window and continued at the oldest segment still there, with a `Discontinuity`),
`ClockUnavailable` (fatal), plus the shared `UnsupportedEncryption`, `TrackDropped` (subtitles, TS and WebM
segments, trick modes, dependent Representations), `UnsupportedContainer` (`non_fragmented_mp4` is fatal),
`SegmentFetchFailed` (fatal for a static MPD at `max_segment_failures`, a skipped segment plus a `Discontinuity`
for a live one), `InitFetchFailed`, `InitDecodeFailed` and `BodyTooLarge`.

**Supported and not.**

| Feature | Status |
|---|---|
| Static MPD, VOD to `Ended` | supported |
| Dynamic MPD, live edge, time-shift window, refresh | supported; needs a clock (see above) |
| `SegmentTemplate` (`$Number$`, `$Time$`, `$Bandwidth$`, `$RepresentationID$`, widths), `SegmentTimeline`, `SegmentList`, `SegmentBase` + `sidx` | supported |
| Periods, `period-continuity` | supported (continuity honoured, `period-connectivity` treated as a discontinuity) |
| `UTCTiming` `direct`, `http-xsdate`, `http-iso`, `http-ntp`, `http-head` | supported; `ntp`/`sntp` skipped (`UnsupportedFeature`) |
| `BaseURL` alternates, DVB `@priority` | supported; `@weight` is not used |
| Video and audio Representation switching, audio set switching | supported (`select_variant`, `select_audio_rendition`); no automatic ABR |
| Low-latency DASH | plays at segment granularity (`UnsupportedFeature { feature: "low_latency" }`) |
| `EventStream`/`InbandEventStream`, `Preselection`, MPD patch, xlink | reported, not used (xlink `resolve-to-zero` is honoured) |
| Subtitles, WebM, MPEG-2 TS segments, trick modes | dropped (`TrackDropped`) |
| CENC/CBCS | never decrypted (`UnsupportedEncryption`) |
| Seeking, `MPD@type="list"`, segment sequences, hierarchical `sidx`, `data:` URLs, open-ended byte ranges | not supported (typed errors) |

**Verification.** `tests/dash_matrix.rs` injects 45 fault kinds (transport and status faults, `UTCTiming`,
MPD-refresh, Period-churn, `sidx`, `425`/`Retry-After`, container and host-clock faults) into eight presentation
shapes at each request class, once and permanently, and asserts the fault was applied; its exhaustiveness guard
fails when a fault or request class has no row. The `dash_session` fuzz target drives a whole `Session` over DASH.
`tests/e2e_livesim2.rs` plays 30 s of live DASH from DASH-IF livesim2 (pinned) over real HTTP in six shapes
and asserts samples flow, no segment is requested before it exists (strictly so under an exact host clock),
and no segment is missing or repeated. It skips loudly when livesim2 is not installed and fails under
`REQUIRE_LIVESIM2=1` (the `dash-e2e` CI job). `tests/no_clock_guard.rs` bans clock reads in this crate and in
`dash-runtime`.

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
