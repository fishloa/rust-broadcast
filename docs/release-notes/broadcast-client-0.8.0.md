# broadcast-client 0.8.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking. The wall clock is `jiff::Timestamp` (jiff 1.0), the crate builds at the `broadcast-common` 14 epoch and every source it adapts moves with it. Behaviour fixes: an encrypted HLS init that fails is refetched and its queued segments are decrypted, a starved SRT wake counts as one retransmit, an RTSP 401 is a rejection, and `select_audio_rendition` refuses a non-HLS/DASH session.

## Breaking changes

- `jiff` 0.2 -> 1.0. `Session::set_wall_clock` takes a jiff 1.x `Timestamp`. Migration: read `jiff::Timestamp::now()` where a `SystemTime` was taken, and depend on `jiff` 1.x.
- Builds against new caret epochs of `broadcast-auth` 0.9, `broadcast-common` 14, `broadcast-dash` 0.3, `broadcast-hls` 0.9, `buffering-runtime` 0.5, `dash-runtime` 0.7, `dvb-si` 16, `hls-runtime` 0.16, `mpeg-ts` 0.10, `rtmp-runtime` 0.12, `rtp-packet` 0.9, `rtsp-runtime` 0.14, `smooth-runtime` 0.4, `srt-runtime` 0.12 and `transmux` 0.33. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single copy of each.

## Fixed

- An HLS VOD whose encrypted init fails to decrypt when its key arrives now plays and ends after the init refetch: the segments queued behind that key are decrypted instead of the session stalling (#3951).
- A starved runtime that wakes the SRT caller once, several retransmit periods late, counts that wake as one retransmit instead of synthesizing one per missed 250 ms period. A single late poll used to spend the whole 12-retry budget at once and time out a handshake whose budget was still open (#3847).
- An AES-128 encrypted HLS init that failed to decrypt (or decrypted to something that is not an init) is refetched on the init's own schedule, whether the key was already held or arrived after the init, instead of the VOD session stalling with neither `Status::Ended` nor a fatal error (audit 23 finding 38, #3289).
- An RTSP 401 with credentials configured is treated as a rejection: the unreachable replay arm is gone, so the source waits for host-supplied credentials and reconnects instead of replaying a request whose credentials the server already refused (#3634).
- `Session::select_audio_rendition` on a Smooth session now returns `Error::NotHls`, as its documentation states, instead of `Ok(())` and doing nothing (audit 23 finding 52, #3303).

## Changed

- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).

## Tests

The session-level suites pin event equality field by field, every label literal, the tracks-hold boundary, the playout gate's hysteresis, the DASH/HLS credential-wait and refusal paths, the DER client-certificate boundary, the TS reconnect wake, the A/V join bookkeeping, the RTP reorder state and the SRT caller's socket id and sequence numbers. The libsrt, mediastreamvalidator and mediamtx oracles fail the run under their `REQUIRE_*` switches instead of skipping.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4020 (`thiserror`), plus #4008/#3992's epoch truthfulness in the CHANGELOG.

---

Published from tag `broadcast-client-v0.8.0`.
