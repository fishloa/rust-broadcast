# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

### Fixed
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

## [0.2.0] - 2026-09-25

### Security
Fixes GHSA-48qq-7p78-2jvj (the DTLS peer certificate was never verified) and
GHSA-89f2-5m24-r6m7 (the remote-ICE-candidate cap could be bypassed via STUN peer-reflexive
candidates). Upgrade if you use the `media` feature.

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

### Added
- `parse_remote_fingerprint(sdp)` to read `a=fingerprint` from an SDP body.
- `media::MediaEvent::RtcpUnsupported(rtcp_packet::Error)`: an inbound SRTCP packet that
  decrypted and authenticated but is not an RFC 3550 §6 compound `rtcp-packet` decodes (e.g.
  RFC 4585 PLI/NACK/REMB feedback, which is most of what a browser receiver sends).

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
