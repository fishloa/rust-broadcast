# Release wave — 2026-10 — protocol and runtime de-hand-roll

One coordinated release of 53 crates. Generic protocols and formats that the
workspace used to parse, build, frame, schedule or serve with its own code now
go through established crates, and the IO and event-loop machinery around them
(accept loops, reconnect loops, pull loops, thread-per-connection servers) now
runs on shared runtime facilities. Real defects found during the survey were
fixed in the same work. All outdated dependencies were taken.

The per-crate notes are the authoritative detail; this page is the map. Most
crates in the wave are **breaking** (a new 0.x minor or a new major).

## What changed, by concern

- **HTTP.** multimux serves on hyper-util behind axum and tower layers (a
  shared concurrency pool, a header-read timeout, typed headers) instead of a
  hand-written request reader. media-doctor's metrics server, the WHIP/WHEP
  signalling in webrtc-runtime and the HLS client's range and query handling
  moved to the same typed HTTP stack. See `multimux-0.11.0`,
  `webrtc-runtime-0.2.0`, `media-doctor-0.9.0`, `hls-runtime-0.7.0`.
- **Authentication.** `broadcast-auth` 0.4.0 parses Digest and Basic/Bearer
  with `http-auth` and `headers`. This changes what is accepted: a client that
  hashes a non-normalised absolute `uri`, or sends a raw non-ASCII `username`,
  is now refused, and RFC 7616 `username*` is supported. Signed-URL queries are
  form-urlencoded. Read `broadcast-auth-0.4.0` before upgrading a server.
- **RTSP.** rtsp-runtime frames messages with a bounded finder, has client and
  server timeouts, and owns one spec-grounded parser for the RFC 2326
  `Transport` and `Session` headers, the one documented exception because the
  `rtsp-types` crate cannot represent them faithfully. See `rtsp-runtime-0.7.0`.
- **URLs, SDP, XML.** URL resolution and construction use the `url` crate
  (transmux's hand-written URI module is gone), SDP uses `sdp-types`, and every
  XML path uses `quick-xml`. Where a crate now needs `std` for XML, the notes
  say so.
- **Dates and durations.** `jiff` replaces the hand-written civil-calendar and
  duration formatting. Output is byte-identical to before except that
  `xs:duration` values of a minute or more now print in the normal ISO 8601
  form (`PT60S` becomes `PT1M`, `PT3600S` becomes `PT1H`). See
  `transmux-0.25.0` and `multimux-0.11.0`.
- **Runtime structure.** multimux has one reconnect schedule (jittered, with
  `backon`, never longer than the cap), one pull scheduler for the HLS, DASH and
  Smooth pull inputs, one generic ingest scaffold, and RTSP and RTMP push
  adapters built on the protocol crates' own async clients. `socket2` sets the
  receive buffer and reuse options on UDP inputs; `parking_lot` replaces
  poisoning locks.

## Defects fixed along the way

Each is described in the owning crate's note and has a regression test that was
shown to fail against the old code. Examples: a quadratic RTSP re-parse, relative
URLs that mishandled `..`, slow-loris HTTP readers, WHIP timers that never fired,
accept starvation, tasks that leaked a bound port, and an `ESDescriptor` parse
that read its own framing as fields (`transmux`, #1148).

## Compatibility

- Upgrade by crate, not by wave: each note lists its breaking changes with a
  migration. Crates that only moved a dependency epoch say exactly that.
- Several versions staged by earlier waves were never published (for example
  dvb-si 10.1.0, broadcast-auth 0.3.1, transmux 0.24.2). This release absorbs
  them; the crates.io history goes straight from the last published version to
  the one in this wave.
- Parser crates that were `no_std` stay `no_std`. `no_std` was dropped only in
  the protocol and runtime crates this work touched.
- `atsc3` stays unpublished (the ATSC 3.0 work is archived). `atsc3-route` gets
  a maintenance release for the dependency epoch.
- `st2022` is new in this wave (0.1.0).

## Guards

Every touched protocol or runtime crate carries a lexical tripwire test that
fails if a banned hand-rolled pattern comes back (HTTP status lines, header
terminators, `://` scanning, SDP line building, civil-calendar helpers, base64
alphabets, sleeps in non-test async code). The tripwire is a reminder; review
remains the real control. The few allowed exceptions are listed with reasons in
the design spec.
