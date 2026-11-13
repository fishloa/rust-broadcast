# webrtc-runtime 0.7.0

_Released 2026-11-14._

Breaking-class release (0.6 -> 0.7): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

- The non-default test-hook feature is renamed `test-support` -> `test-seams`. Only a test suite enabling it by name is affected.

```toml
# before
webrtc-runtime = { version = "0.6", features = ["test-support"] }
# after
webrtc-runtime = { version = "0.7", features = ["test-seams"] }
```

## Fixed

- Trickle PATCH `412` adopts response `ETag`.
- In-flight request tracking in send order.
- Non-2xx offer POST returns client to `Idle`.
- `Session::accept`, `ack_restart`, `ack_answer` no longer reopen closed session.
- `WhepSession::ack_answer` validates entity-tag and sends it as `ETag`.

## Also in this release

### Changed
- BREAKING: `Client::new(endpoint_url, bearer_token)` (`WhipClient`, `WhepPlayer`) takes a `session::client::ClientConfig` (`ClientConfig::new(endpoint_url).with_bearer_token(token)`), the same `Config` + `with_*` shape as the other engines (#2514).

### Added
- `media::SocketDemux<K>` (feature `media`): a sans-IO demultiplexer that lets many `MediaTransport`s share one UDP socket. A session is registered by its local `ice-ufrag`; `classify(datagram, from)` routes a STUN connectivity check by the receiver ufrag in its `USERNAME` (RFC 8445 §7.2.2) and maps the sender's address to that session, then routes DTLS, SRTP/SRTCP and STUN responses by that address. A check naming an unknown ufrag maps nothing, an address from an unmapped source is dropped, and each session maps at most `max_tuples_per_session` addresses (a further one is dropped, never evicting a working mapping).
- `sdp`, the one place a WHIP/WHEP SDP document is built: `session_answer` renders the whole answer (origin, session name, timing and the single media section with its ICE/DTLS attributes) from typed inputs — the local `IpAddr`, `LocalMediaIdentity` (ICE ufrag/pwd and the `sha-256` certificate fingerprint), the `AnswerTransport` narrowed to one payload type, `CodecAttribute`s and the gathered candidates — and `restart_answer` renders the ICE-restart answer fragment. `offer_codec_attributes` picks the offer's `a=rtpmap`/`a=fmtp`/`a=rtcp-fb` entries naming the chosen payload type for an answer to echo, `offer_mid` the `a=mid` to echo back, and `fragment_prelude` the session header lines a fragment has to be parsed against. Rendering reports `Error::SdpWrite` instead of a lossy conversion, and `LocalMediaIdentity`'s `Debug` redacts the ICE password. `sdp-types` is now a plain dependency (the builder is in the default build), so the `media` feature no longer enables it.
- `sdp-types` is now a required dependency (it was optional behind `media`): the `sdp` builder is part of the default build.

### Fixed
- The bearer-token origin check resolves the target through `broadcast_common::url_ref::resolve`, so a target with a control character is a different origin instead of being repaired into the endpoint's own (RFC 3986 §2) (#2585).
- `sdp::offer_codec_attributes` now echoes `a=rtcp-fb` attributes whose payload-type token is `*` (RFC 4585 wildcard), in addition to the chosen payload type.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `rtp-packet` 0.6 -> 0.7.

---

Published from tag `webrtc-runtime-v0.7.0`.
