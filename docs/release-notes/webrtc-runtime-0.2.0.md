# webrtc-runtime 0.2.0

_Released 2026-10-05._

Breaking (0.x minor), large release. It combines a security fix for the `media` feature (the ICE + DTLS-SRTP transport) with the de-hand-roll wave's move of the WHIP/WHEP signalling engine onto `http`/`headers` types. **Upgrade if you use `media::MediaTransport`**: before this release the DTLS peer certificate was never verified. Who must act: anyone constructing `MediaTransportConfig` (two new required fields), anyone driving `MediaTransport` (a new `now: Instant` argument, `handle_timeout` return type, new events), and anyone using the WHIP/WHEP state machines (`HttpRequest`/`HttpResponse` and several signatures changed). The previous published version is 0.1.0. Read together with [rtcp-packet-0.4.0.md](rtcp-packet-0.4.0.md), which fixes the RTCP decode problem described under Fixes.

## Security (`media` feature)

- **GHSA-48qq-7p78-2jvj**: the DTLS peer was never authenticated. `MediaTransport` built its DTLS configuration with certificate verification off, accepted DTLS from any source address, and replaced the session's SRTP keys on every completed handshake. In the passive (WHIP ingest) role an off-path host could complete its own handshake against the media port and substitute the session's keys. Now the peer's leaf certificate must hash to `remote_fingerprint` (RFC 8122), checked in the DTLS verify callback and again before any SRTP key is derived (digests compared without an early exit); the passive role requires a client certificate; DTLS is accepted only from the remote address of the ICE-selected pair (dropped before a pair is selected); and once a session's SRTP keys are installed, a handshake completed from a different address returns an error and leaves the keys untouched.
- **GHSA-89f2-5m24-r6m7**: the cap on remote ICE candidates (RFC 8445 §6.1.2.5) could be bypassed: an authenticated STUN Binding Request from an unrecognised source address made the ICE agent create its own peer-reflexive candidate, uncounted by the cap, so a remote peer could grow the candidate and pair count without bound by sending from many source ports. New STUN source addresses are now checked against the same cap first; an address already admitted keeps working. `add_remote_candidate` rejects candidates past the cap with `Error::Media`, never silently dropped.

## Breaking changes

### 1. `MediaTransportConfig` has two new required fields

```rust
pub remote_fingerprint: String,       // the remote SDP's a=fingerprint, e.g. "sha-256 AB:CD:..."
pub max_remote_candidates: usize,     // cap on admitted remote ICE candidates
```

```rust
let remote_fingerprint = webrtc_runtime::media::parse_remote_fingerprint(&remote_sdp)
    .ok_or("offer has no a=fingerprint")?;
// max_remote_candidates: webrtc_runtime::media::MAX_REMOTE_CANDIDATES (100, the spec's recommended default)
```

`MediaTransport::new` rejects a fingerprint that is not `sha-256` followed by 32 colon-separated hex bytes (SHA-256 is the hash WebRTC endpoints must support, RFC 8827 §6.5). `MediaTransportConfig::remote_fingerprint` is validated the same way as `parse_remote_fingerprint` reads it (see Behaviour changes).

### 2. `MediaTransport` takes the clock from the caller

`MediaTransport::new(config, now)` takes a caller-supplied `std::time::Instant`; every internal timer (ICE agent, STUN gatherer) is scheduled from it and the transport never reads the wall clock. New `poll_timeout()` returns the earliest deadline over the ICE agent, every DTLS association, the STUN gatherer and the retired-key purge: schedule `handle_timeout` there instead of on a fixed tick. New `local_candidates()` returns the host candidate as an `a=candidate:` body.

### 3. `MediaTransport` event and return-type changes (#1090)

- `handle_timeout(now)` returns `Vec<MediaEvent>` (was `()`). A failed ICE or DTLS timer drive is surfaced as `MediaEvent::TimerError(String)` instead of being silently discarded.
- An SRTP/SRTCP authentication failure (spoofed or garbage traffic in the RFC 5764 §5.1.2 band; the expected outcome for unsolicited traffic on an open UDP port) is now `MediaEvent::AuthFailure`, not an `Err` from `handle_datagram`.
- An inbound SRTCP packet that authenticates but does not decode as an RFC 3550 §6 compound is `MediaEvent::RtcpUnsupported(rtcp_packet::Error)` instead of an `Err` for a genuine peer packet; it still counts toward the RFC 3711 key-lifetime read counter.
- `DecryptedRtp` gained `extension: Option<DecryptedRtpExtension>` (RFC 3550 §5.3.1, for example RFC 8285 CVO, AV1 dependency descriptor, `mid`, `rid`), which used to be dropped between decrypt and the caller.
- `MediaTransport::rekey` returns `Error::Media` for `SetupRole::Passive` instead of tearing down the association. A Passive side never dials out, so the old behaviour left it waiting forever for a `ClientHello` a browser or OBS peer never sends without a new SDP offer. Drive an ICE restart or SDP renegotiation instead.

### 4. WHIP/WHEP state machines speak `http` and `headers` types

The crate is now `std` (the `std` feature is kept as a name but no longer gates anything) and left CI's `thumbv7em-none-eabi` target list. `HttpRequest` and `HttpResponse` are one shared definition (re-exported from `whip::{client,server}` and `whep::{player,server}`):

```rust
// 0.1.0
HttpResponse { status: u16, content_type: Option<&'static str>, headers: Vec<(String, String)>, body }
HttpRequest  { method: Method /* crate enum */, url, content_type: Option<&'static str>, headers: Vec<(String, String)>, body }
session.on_patch(fragment, if_match: Option<&str>)         // WHIP
session.on_patch(content_type: &str, body, ..)             // WHEP
WhepSession::no_publisher(retry_after_secs: Option<u32>)
// 0.2.0
HttpResponse { status: http::StatusCode, headers: http::HeaderMap, body }
HttpRequest  { method: http::Method, url, headers: http::HeaderMap, body }
WhipSession::on_patch(fragment: Vec<u8>, headers: &HeaderMap)
WhepSession::on_patch(body: Vec<u8>, headers: &HeaderMap)   // content type read from the headers
WhepSession::no_publisher(retry_after: Option<std::time::Duration>)
```

Builders: `HttpResponse::new(status)`, `with_body`, `with_content_type`, `with_location`, `with_etag`; accessors `HttpRequest::content_type()` and `if_match()` return the typed `headers` values. The old crate-level `Method` enums are `http::Method`. New `Error::InvalidHeader { header }`. Header names are lower-case in the `HeaderMap`.

- `WhipClient::add_candidate` and its `buffered_candidates` field are removed: `flush_candidates` never read them (it takes its own aggregated fragment), so they were dead, unbounded-until-flush state.
- `WhipClient::flush_candidates`, `ice_restart`, `terminate` and `WhepPlayer::trickle_ice`, `ice_restart`, `terminate` now record which request is in flight and dispatch the response on that instead of guessing from status and `ETag`. `WhepPlayer::trickle_ice` and `ice_restart` take `&mut self` (were `&self`). Fixed defects: a trickle ack answered `200` with an `ETag` (RFC 9725 permits `204` or `200` plus `ETag`) was misread as an ICE-restart answer, and a `DELETE` answered `204` left the client `Established` forever instead of `Closed` (#1090).

## Behaviour changes

- **`If-Match` is an RFC 9110 entity-tag list with strong comparison.** `If-Match: *` is an ICE restart and the current quoted tag (`"etag1"`) a trickle update, as before. An unquoted tag (`etag1`), a quoted star (`"*"`) and a weak tag (`W/"etag1"`) no longer match: they fail with `Error::ETagMismatch`, whose `got` is now the raw header text (`"\"old\""`). A duplicated `Content-Type` on a PATCH is rejected. A weak server `ETag` is no longer reused as a strong `If-Match` tag by the WHIP/WHEP clients (treated as absent). A session URL (non-ASCII) or ETag that cannot be sent as a header makes `accept` answer `500` with no `Location` instead of emitting a bad header.
- **`media::parse_remote_fingerprint` reads the SDP with `sdp-types`.** The text must be a well-formed session (`v=`, `o=`, `s=`, `t=`); the first media section carrying an `a=fingerprint` wins over the session level; the result is the typed attribute's normalised text (`a=fingerprint:SHA-256 ab:cd:0f` gives `sha-256 AB:CD:0F`). A digest without colons is accepted when it is 32 bytes. The first level that carries an `a=fingerprint` decides: an unparseable one gives `None` instead of falling through to a later one.
- `WhepSession::on_patch` matches `Content-Type` by media type only, so `application/sdp; charset=utf-8` is accepted like bare `application/sdp` (#1090).

## Fixes

- **Browser RTCP discarded (#1071).** `MediaEvent::RtcpUnsupported` was the outcome for nearly every real browser SRTCP datagram (RFC 4585 PSFB/RTPFB feedback or RFC 3611 XR, most of what a WebRTC peer sends), because `rtcp_packet::CompoundPacket::parse` rejected the whole datagram on the unrecognised `PT`, discarding a leading SR/RR's real statistics too. Fixed by `rtcp-packet` 0.4 (`RtcpPacket::Unknown`); the doc comment of `RtcpUnsupported` is corrected accordingly.
- `ice::parse_ice_server_links` / `format_ice_server_links`: a `Link` header parameter value (`username`, `credential`) containing `;`, `,` or `"` (legal in an RFC 8288 `quoted-string`, for example a static TURN password) now round-trips instead of being split or corrupted; `rel` matching is case-insensitive and accepts a space-separated list (#1090).
- The server-reflexive candidate's `stun:` URL brackets an IPv6 host (`stun:[2001:db8::1]:3478`).
- `StunGather`'s deadline is the first instant at which `handle_timeout` does work (`rtc-stun` collects an expired transaction only when `deadline < now`), so a driver sleeping until the raw deadline no longer spins.

## New feature: `test-support`

Non-default, `#[doc(hidden)]`, not public API: `MediaTransport::with_certificate_for_test`, `force_next_timer_error`, `force_stuck_timer` and `media::certificate_fingerprint`, used by `multimux`'s loopback and fault-injection tests. None of it exists in a default build (guarded by `tests/test_support_is_gated.rs`).

## Dependencies

Verified from the `Cargo.toml` diff against `webrtc-runtime-v0.1.0`:

```toml
broadcast-common = { version = "9.3", default-features = false } -> { version = "9.4" }   # std in every configuration
thiserror        = { version = "2", default-features = false }   -> { version = "2" }
http = "1"        # new
headers = "0.4"   # new
# media feature
rtcp-packet 0.3 -> 0.4;  rtc-ice / rtc-dtls / rtc-srtp / rtc-stun / rtc-shared 0.20 -> 0.21;  sha2 0.10 -> 0.11
sdp-types = { version = "0.2", optional = true }   # new
url       = { version = "2",   optional = true }   # new
test-support = []                                   # new feature
```

rtc 0.21 takes its crypto provider explicitly; this crate uses the default provider (ring), so the provider choice is unchanged, but ring now implements primitives that RustCrypto implemented underneath before: a different backend under the same provider. `multimux` lists `rtc-dtls` only as one of its own dev-dependencies; this crate's dependency set is otherwise as shown. MSRV 1.95.0.

---

Published from tag `webrtc-runtime-v0.2.0`.
