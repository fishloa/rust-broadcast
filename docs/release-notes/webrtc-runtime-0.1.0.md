# webrtc-runtime 0.1.0

**Release date:** 2026-08-10

First release. A sans-IO WHIP (RFC 9725) + WHEP (draft-ietf-wish-whep-04)
HTTP signalling engine: SDP offer/answer, Trickle ICE, ICE restart, and
session lifecycle state machines for both client and server roles. Two
things a consumer will hit immediately: **there is no IO adapter** — this
crate has no HTTP client or server built in, the caller drives the actual
HTTP requests/responses against the state machines — and the optional
`media` feature (ICE + DTLS-SRTP transport) pulls a large dependency tree,
so it is off by default and worth avoiding unless you need it.

## What's in 0.1.0

- **WHIP** (`whip` module): `WhipClient` and `WhipSession` state machines —
  SDP offer/answer via HTTP POST, Trickle ICE via PATCH, ICE restart, and
  session teardown via DELETE.
- **WHEP** (`whep` module): `WhepPlayer` and `WhepSession` state machines —
  direct-accept and counter-offer (406) flows, and no-publisher (409)
  detection.
- ICE server discovery: `Link` header parsing/formatting (RFC 9725 §4.4)
  for STUN/TURN server lists.
- Bearer token authentication (RFC 6750) on all requests.
- `no_std` + `alloc` support, with `std` on by default (see below).
- Optional `media` feature: `MediaTransport` — ICE + DTLS-SRTP over
  `rtc-ice`/`rtc-dtls`/`rtc-srtp`, including `needs_rekey()`/`rekey()`
  tracking RFC 3711 §8.2/§9.2's 2^31-packet SRTP key-usage limit and
  performing the RFC 5764 §4.4 fresh-DTLS-session rekey (not §5.2's
  in-band renegotiation, which the underlying `rtc-dtls` has no API for).
  The old read key is retained for 2 minutes after a rekey so a packet
  reordered across the boundary still decrypts.
- Spec transcriptions: `docs/whip-rfc9725.md`, `docs/whep-draft-04.md`.
- Sans-IO throughout: no bundled tokio/reqwest HTTP client, no async
  runtime dependency in the default build.

## No IO adapter

This crate never implements HTTP transport. `WhipClient`/`WhipSession`/
`WhepPlayer`/`WhepSession` produce and consume SDP/HTTP-header-shaped data;
the caller is responsible for making the actual HTTP requests (POST/PATCH/
DELETE for WHIP, GET/POST for WHEP) and feeding the responses back in. A
`tokio` feature existed in earlier development but gated no code — it added
`tokio`/`reqwest` as dependencies for zero behavioural effect — and has been
removed before this first release rather than shipped as a non-functional
stub.

## `media` feature: heavy, opt-in

`media = [...]` pulls in `rtc-ice`, `rtc-dtls`, `rtc-srtp`, `rtc-stun`,
`rtc-shared`, `sansio`, `bytes`, `sha2`, `rtp-packet`, and `rtcp-packet` on
top of the default build. It exists purely for the ICE/DTLS-SRTP media
transport (`MediaTransport`); a consumer that only needs WHIP/WHEP
signalling should leave it off. Enabling it also lifts the effective rustc
requirement — `rcgen` (a transitive dependency) previously needed 1.88 where
the rest of the workspace's MSRV was 1.86, which is why `media` had its own
CI job, six `--exclude` lanes, and a guard script; the workspace MSRV bump to
1.95.0 (below) removes that split.

## `no_std` support is real as of this release

`std` was previously declared as an empty feature (`std = []`) that forwarded
nothing, so `--no-default-features` still resolved `broadcast-common` and
`thiserror` with their own default (std-requiring) features — the crate
could never actually link without the std runtime, despite the crate-root
`cfg_attr` and README both claiming `no_std` support. `std` now forwards to
`broadcast-common/std` and `thiserror/std`. `broadcast-auth` and `log` have
also been dropped from the dependency list: neither was referenced by any
line of this crate, and both are std-only, so on their own they made a
bare-metal build impossible. The crate is now built for `thumbv7em-none-eabi`
in CI's `no_std` job, so this claim is verified rather than aspirational.

## Also removed before this release

Four public items that were never constructed anywhere in the crate, each
implying a capability the engine does not have, were removed before this
first release rather than shipped as misleading dead API: `Method::Options`
on both the WHIP and WHEP `Method` enums (both `#[non_exhaustive]`, so no
downstream `match` breaks), `whep::player::Method::Head`,
`Error::InvalidSdp` (SDP is carried as an opaque `Vec<u8>`, never validated),
and `Error::CounterOfferExpired` (the WHEP server emits the `valid-until`
header but nothing in this crate enforces it).

## Migration

New crate — no prior public API. MSRV is 1.95.0 (workspace-wide, issue #949).
