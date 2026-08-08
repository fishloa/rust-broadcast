# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

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
