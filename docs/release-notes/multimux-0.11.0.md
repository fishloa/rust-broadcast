# multimux 0.11.0 — 2026-09-26

Security release, plus DASH SCTE-35 inband events. **Upgrade if you run WHIP ingest, WHEP output,
push outputs, RTP/UDP inputs, or any authenticated route.** This is a minor (0.x-breaking) release
because the dependencies it builds against changed epoch: `hls-runtime` 0.6 → 0.7,
`broadcast-auth` 0.3 → 0.3.1, `rtsp-runtime` 0.6 → 0.7, and, for `whip`/`whep`, `webrtc-runtime`
0.1 → 0.2. Config and HTTP routes are unchanged.

## Security

| Advisory | Area | Before this release |
|---|---|---|
| GHSA-jwfh-m4vx-fhwx | WHEP / WHIP HTTP | WHEP didn't apply the configured `output_auth`. Sessions never ended, so the 64 slots filled up permanently. The request reader had no header or body limit and no timeout. WHIP allocated a session before checking capacity. |
| GHSA-48qq-7p78-2jvj | WHIP / WHEP media | The DTLS peer wasn't authenticated against the SDP fingerprint (fixed in webrtc-runtime 0.2.0; multimux now passes the offer's `a=fingerprint`). |
| GHSA-6cpc-jqv3-qcj3 | push outputs | `drive_push` parked a runtime worker thread, and busy-spun when every listener slot was taken. |
| GHSA-c5v7-p4jv-2fhc | RTP/UDP input | One malformed datagram froze the route permanently while it still reported Live. |
| GHSA-2w4r-qf2x-pqm6 | route ingest / output auth | A route accepted a second, concurrent RTMP/WHIP publisher and silently handed program ownership to it (or froze once the first disconnected); output-auth and admin-auth Digest challenges re-prompted for credentials on an expired nonce instead of returning a `stale=true` challenge; nothing told an operator when an ingest route ran with no authentication at all. |

## Behaviour changes

**WHEP**
- Every request passes the same output-auth check as the HTTP routes: 401 plus a challenge, and
  OPTIONS is exempt.
- A session ends after 30 s without RTP, RTCP or a completed DTLS handshake, which frees its
  slot.
- An offer without `a=fingerprint` is rejected.

**WHIP and WHEP**
- **Shared request reader.**
  - Headers are capped at 16 KiB (431) and bodies at 64 KiB (413). An oversized
    `Content-Length` is rejected before any body is read.
  - A malformed `Content-Length` is a 400, and chunked transfer-encoding is refused with 411.
  - The whole read times out after 10 s.
- **Connections.** Both accept loops cap concurrent connections at 256.
- **Capacity.**
  - Capacity is checked before a UDP socket is bound (503 when full).
  - The reserved slot is released on every failure path, for example when the client
    disconnects before the answer is written.

**Push outputs**
- They await the trunk listener with a 250 ms timeout, and back off for 50 ms when no listener
  slot is free.

**RTP/UDP input**
- A packet that fails depacketisation is dropped and counted.
- A session that is no longer running ends the route, so the supervisor reconnects it.

**Ingest and output auth**
- A route rejects a second, concurrent RTMP/WHIP publisher instead of letting it silently take
  over (or freeze, once the first disconnects) the program the route is already receiving from.
  The rejected connection's own session is reaped normally by its listener's usual timeout/error
  handling; once the active publisher's session ends, the route accepts a new one for that
  program again (a legitimate reconnect, or a backup encoder taking over).
- Output-auth and admin-auth Digest challenges now use `broadcast_auth::Verifier::challenge_for`
  instead of `challenge`, so a request that correctly answers an expired nonce gets a fresh
  challenge carrying `stale=true` (RFC 7616 §3.3) rather than being silently re-prompted for
  credentials.
- Config docs for `InputSpec::Rtmp`/`Whip`/`Srt` now say plainly when an ingest listener runs
  with no authentication at all, and a startup log line warns for each such route.
- A WHEP viewer's session no longer ends after 30 seconds: the silence timer counted only RTCP
  that `rtcp-packet` could decode, but a browser viewer sends RFC 4585 feedback (PLI, NACK,
  REMB), which it cannot. Any SRTCP packet that authenticates now counts (needs
  `webrtc-runtime`'s new `MediaEvent::RtcpUnsupported`); a datagram that fails authentication
  still does not.

## Added

- **DASH SCTE-35 inband event signalling (#969).**
  - The MPD declares `<InbandEventStream schemeIdUri="urn:scte:scte35:2013:bin">`.
  - Served fMP4 segments carry `emsg` boxes (after `styp`, before `moof`) for segments with
    resolved SCTE-35 events.

## Upgrading

Take this release together with the dependency releases it needs:
- `hls-runtime` 0.7.0
- `media-plane` 0.4.1
- `broadcast-auth` 0.3.1 (needs `Verifier::challenge_for`)
- `rtsp-runtime` 0.7.0
- `webrtc-runtime` 0.2.0 (if you use `whip`/`whep`)

Code outside multimux that builds `webrtc_runtime::media::MediaTransportConfig` itself needs the
new `remote_fingerprint` field; see the webrtc-runtime 0.2.0 note.

MSRV 1.95.0.
