# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]


## [0.2.0] - 2026-10-05

### Security
Fixes two security issues: the DTLS peer certificate was never verified, and
the remote-ICE-candidate cap could be bypassed via STUN peer-reflexive
candidates. Upgrade if you use the `media` feature.

### Changed (breaking)
- `media::MediaTransportConfig` has a new required field `remote_fingerprint` (the remote SDP's
  `a=fingerprint`). The DTLS handshake now fails unless the peer's certificate matches it, the
  passive role requires a client certificate, DTLS is accepted only from the ICE-selected
  address, and an established session's SRTP keys cannot be replaced by another association.
- `media::MediaTransportConfig` has a new required field `max_remote_candidates`.
  `media::MediaTransport::add_remote_candidate` previously admitted an unbounded number of
  remote ICE candidates; a caller (e.g. a WHIP offer or a trickle ICE fragment) that supplied a
  very large number of candidates aimed at arbitrary IP/port pairs could make the transport
  originate an unbounded number of STUN connectivity checks. RFC 8445 §6.1.2.5 requires this
  cap to be configurable, so every constructor must now supply a value; pass the new
  `MAX_REMOTE_CANDIDATES` constant (100, the spec's own recommended default) unless a stricter
  cap is wanted. Candidates past the configured cap are rejected with `Error::Media`, never
  silently dropped.

- `MediaTransport::new(config, now)` and the rest of construction take a caller-supplied `std::time::Instant`: every internal timer (ICE agent, STUN gatherer) is scheduled from it and the transport never reads the wall clock.
- The crate is `std` (the `std` feature is kept as a name but no longer gates anything) and left CI's `thumbv7em-none-eabi` list: the WHIP/WHEP state machines now speak `http`/`headers` types. `HttpRequest`/`HttpResponse` (one shared definition, re-exported from `whip::{client,server}` and `whep::{player,server}`) carry `http::Method`, `http::StatusCode` and an `http::HeaderMap` (`HttpRequest::content_type()`/`if_match()`, `HttpResponse::new()`/`with_body()`/`with_content_type()`/`with_location()`/`with_etag()`); the old `Method` enums are `http::Method`. `WhipSession::on_patch(fragment, &HeaderMap)`, `WhepSession::on_patch(body, &HeaderMap)` (content type from the headers) and `WhepSession::no_publisher(Option<Duration>)` (typed `Retry-After`). New `Error::InvalidHeader { header }`.
- `If-Match` is read as an RFC 9110 entity-tag list with strong comparison. `If-Match: *` is an ICE restart and the current quoted tag (`"etag1"`) a trickle update, as before; an unquoted tag (`etag1`), a quoted star (`"*"`) and a weak tag (`W/"etag1"`) no longer match (they fail with `Error::ETagMismatch`, whose `got` is now the raw header text, `"\"old\""`). A duplicated `Content-Type` on a PATCH is rejected. A session URL (non-ASCII) or ETag that cannot be sent as a header makes `accept` answer `500` with no `Location` instead of emitting a bad header. Header names are lower-case in the `HeaderMap`.
- `media::parse_remote_fingerprint` reads the SDP with `sdp-types` (`Session::parse`): the text must be a well-formed session (`v=`/`o=`/`s=`/`t=`), the first media section carrying an `a=fingerprint` wins over the session level, and the result is the typed attribute's normalised text (`a=fingerprint:SHA-256 ab:cd:0f` gives `sha-256 AB:CD:0F`). A digest without colons is accepted when it is 32 bytes. The first level (media section, else session) that carries an `a=fingerprint` decides: an unparseable one now gives `None` instead of falling through to a later one. A weak server `ETag` is no longer reused as a strong `If-Match` tag by the WHIP/WHEP clients (it is treated as absent). `MediaTransportConfig::remote_fingerprint` is validated the same way.
- `MediaTransport::handle_timeout` now returns `Vec<MediaEvent>` (was `()`); a failed ICE or
  DTLS timer drive is now surfaced as `MediaEvent::TimerError` instead of silently discarded
  (#1090).
- `MediaTransport::handle_srtp_datagram`'s SRTP/SRTCP authentication failure (spoofed or
  garbage traffic in the RFC 5764 §5.1.2 band) is now `MediaEvent::AuthFailure`, not `Err` from
  `handle_datagram` — this is the expected outcome for unsolicited traffic on an open UDP port,
  not a transport error (#1090).
- `DecryptedRtp` gained an `extension: Option<DecryptedRtpExtension>` field (RFC 3550 §5.3.1,
  e.g. RFC 8285 CVO/AV1-dependency-descriptor/`mid`/`rid`), which used to be dropped between
  decrypt and the caller (#1090).
- `MediaTransport::rekey` now returns `Error::Media` for `SetupRole::Passive` instead of tearing
  down the association: `maybe_start_active_dtls` never dials out for Passive, so the old
  behavior left a Passive side waiting forever for a `ClientHello` a browser/OBS peer never
  sends without a new SDP offer, silently ending media for good. Drive an ICE restart or SDP
  renegotiation instead (#1090).
- `WhipClient::add_candidate` and its `buffered_candidates` field are removed: `flush_candidates`
  never read them (it takes its own aggregated fragment), so they were dead, unbounded-until-
  flush state with no consumer (#1090).
- `WhipClient::flush_candidates`/`ice_restart`/`terminate` and `WhepPlayer::trickle_ice`/
  `ice_restart`/`terminate` now record which request is in flight and dispatch the response on
  that, rather than guessing from status code and `ETag` alone. `WhepPlayer::trickle_ice`/
  `ice_restart` now take `&mut self` (previously `&self`) to record it. Before this, a trickle
  ack answered `200` with an `ETag` (RFC 9725 permits either `204` or `200`+`ETag`) was
  misread as an ICE-restart answer, and a `DELETE` answered `204` (as common as the `200` the
  code checked for) left the client `Established` forever instead of `Closed` (#1090).

### Added
- `parse_remote_fingerprint(sdp)` to read `a=fingerprint` from an SDP body.
- `media::MediaEvent::RtcpUnsupported(rtcp_packet::Error)`: an inbound SRTCP packet that
  decrypted and authenticated but is not an RFC 3550 §6 compound `rtcp-packet` decodes (e.g.
  RFC 4585 PLI/NACK/REMB feedback, which is most of what a browser receiver sends).

- `test-support` feature (non-default, `#[doc(hidden)]` hooks, not public API): `MediaTransport::with_certificate_for_test`, `force_next_timer_error`, `force_stuck_timer` and `media::certificate_fingerprint`, used by `multimux`'s loopback/fault-injection tests. None of it exists in a default build (guarded by `tests/test_support_is_gated.rs`). The `rtc-dtls` dependency `multimux` lists is a `[dev-dependencies]` entry of `multimux` only; `webrtc-runtime`'s own dependency set is unchanged.

- `MediaTransport::poll_timeout()` (earliest deadline over the ICE agent, every DTLS association, the STUN gatherer and the retired-key purge; schedule `handle_timeout` there, not on a fixed tick) and `MediaTransport::local_candidates()` (the host candidate as an `a=candidate:` body, from `rtc-ice`'s own marshaller).

### Fixed
- `MediaTransport::handle_datagram` no longer returns `Err` for such a packet. It passed SRTCP
  authentication, so it is a genuine packet from the peer, not a transport error. It also now
  counts toward the RFC 3711 key-lifetime read counter.
- `media::MediaTransport::add_remote_candidate`'s cap on remote ICE candidates
  (RFC 8445 §6.1.2.5) could be bypassed entirely: an authenticated STUN Binding
  Request from a source address the transport didn't already recognize made
  the ICE agent create its own peer-reflexive remote candidate, uncounted by
  the cap. Since the remote peer already knows the negotiated ICE
  ufrag/password, it could grow the remote-candidate (and pair) count without
  bound by sending from many source ports (RFC 8445 §19.5.1). New STUN
  source addresses are now checked against the same configured cap before
  being handed to the ICE agent; an address already admitted keeps working.

- The server-reflexive candidate's `stun:` URL brackets an IPv6 host (`stun:[2001:db8::1]:3478`).
- `StunGather`'s deadline is the first instant at which `handle_timeout` does work: `rtc-stun` collects an expired transaction only when `deadline < now`, so a driver sleeping until the raw deadline and calling `handle_timeout` there would spin.
- `ice::parse_ice_server_links`/`format_ice_server_links`: a `Link` header parameter value
  (`username`/`credential`) containing `;`, `,` or `"` — all legal in an RFC 8288
  `quoted-string`, e.g. a static TURN operator password — now round-trips through format ->
  parse instead of being split or silently corrupted; `rel` matching is now case-insensitive
  and accepts a space-separated list of relation types, per RFC 8288 (#1090).
- `whep::server::WhepSession::on_patch` now matches `Content-Type` by media type only (ignoring
  any `; parameter=value`), so `application/sdp; charset=utf-8` is accepted like the bare
  `application/sdp` it used to require exactly (#1090).
- `MediaEvent::RtcpUnsupported` was the outcome for nearly every real
  browser SRTCP datagram (RFC 4585 PSFB/RTPFB feedback or RFC 3611 XR, most
  of what a WebRTC peer sends), because `rtcp_packet::CompoundPacket::parse`
  rejected the whole datagram on the unrecognized `PT` — discarding a
  leading SR/RR's real stats along with the feedback, not just failing to
  decode the feedback itself. Fixed upstream by `rtcp-packet` 0.4 (issue
  #1071, `RtcpPacket::Unknown`); `decrypt_srtp` needed no logic change, but
  `MediaEvent::RtcpUnsupported`'s doc comment is corrected (it no longer
  names RTPFB/PSFB/XR as the typical cause) and WHEP liveness behavior is
  unchanged — `multimux`'s `is_liveness_event` already treated both
  `MediaEvent::Rtcp` and `MediaEvent::RtcpUnsupported` as proof of life.

### Changed
- Dependency bumps, non-breaking (no public API change): `rtc-dtls`/`rtc-ice`/`rtc-shared`/`rtc-srtp`/`rtc-stun` 0.21, and dev-only RustCrypto 0.13 (`aes` 0.9, `cipher` 0.5, `ctr` 0.10, `hmac` 0.13, `sha1` 0.11, `sha2` 0.11). rtc 0.21 takes its crypto provider explicitly; this crate uses the default provider (ring), so the provider choice is unchanged, but ring now implements the primitives that were previously RustCrypto underneath (a different backend, same provider).

## [0.1.0] - 2026-08-11

### Fixed

- The advertised `no_std` build now works. `std` was declared as an empty
  feature (`std = []`) that forwarded nothing, so `--no-default-features`
  still resolved `broadcast-common` and `thiserror` with their default
  features and the crate could never link without the std runtime — the
  crate-root `cfg_attr` and the README claim were both unreachable.
  `std` now forwards to both dependencies, and `broadcast-auth` and `log`
  are dropped: they were declared but referenced by no line of this crate,
  and being std-only they alone made a bare-metal build impossible. The
  crate is now in CI's `no_std` (`thumbv7em-none-eabi`) job.

### Added

- `MediaTransport::needs_rekey()` / `MediaTransport::rekey()` (issue #948
  item 3): RFC 3711 §8.2/§9.2's `2^31`-packet key-usage limit (RFC 5764
  §4.4's `maximum_lifetime`) is now tracked per direction/type and
  actionable — `rekey()` tears down the DTLS association and establishes a
  fresh one (§4.4's own "a new DTLS session SHOULD be used" mechanism, not
  §5.2's in-band-renegotiation one, which the `rtc-dtls` dependency has no
  API for — see `MediaTransport::rekey`'s doc). The old read key is
  retained for 2 minutes (this crate's chosen MSL, RFC 5764 §5.2) so a
  packet reordered across the rekey boundary still decrypts.

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Removed

- The `tokio` feature (and its `dep:tokio`/`dep:reqwest` optional
  dependencies) has been removed. It gated zero code — enabling it pulled in
  two heavy async dependencies for no behavioural effect, while `src/lib.rs`
  claimed present-tense that it "provides real HTTP client/server adapters"
  and `README.md` called the same row "(planned)". Since the crate is
  unpublished, removing it as a no-op breaks no downstream consumer; no
  adapter was implemented. `src/lib.rs`/`README.md`/`Cargo.toml` now agree:
  there is no IO adapter, by design (#939).
- Four public items that were never constructed anywhere in the crate, each
  implying a capability the engine does not have, were removed rather than
  left as misleading dead API (#939):
  - `whip::client::Method::Options` and `whep::player::Method::Options`
    (both `Method` enums are `#[non_exhaustive]`, so this is not breaking
    for downstream `match`es).
  - `whep::player::Method::Head`.
  - `Error::InvalidSdp` — implied SDP validation that categorically does not
    happen; SDP is carried as an opaque `Vec<u8>`.
  - `Error::CounterOfferExpired` — implied `valid-until` deadline
    enforcement that the player never performs (the server emits the header
    but nothing consumes it).

### Added

- Sans-IO WHIP client (`WhipClient`) and server (`WhipSession`) state
  machines — SDP offer/answer via HTTP POST, Trickle ICE via PATCH,
  ICE restart, and session teardown via DELETE (RFC 9725).
- Sans-IO WHEP player (`WhepPlayer`) and server (`WhepSession`) state
  machines — direct-accept and counter-offer (406) flows, no-publisher
  409 detection (draft-ietf-wish-whep-04).
- ICE server Link header parsing and formatting (`ice` module) for
  STUN/TURN server discovery (RFC 9725 §4.4).
- Bearer token authentication on all requests.
- `no_std` + `alloc` support (`std` feature on by default).
- Spec transcriptions: `docs/whip-rfc9725.md`, `docs/whep-draft-04.md`.
