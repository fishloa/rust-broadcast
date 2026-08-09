# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

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
