# webrtc-runtime 0.9.0

_Released 2026-11-17._

Minor-class release (0.8 -> 0.9) closing the STUN identity gap from the audit 20 security-surface review. `SocketDemux` requires the session's `ice-pwd` before it maps a source address, `MediaTransport` admits a remote candidate only after the STUN credentials verify, and the ICE-server Link and WHEP `valid_until` inputs are checked before they are written into headers.

## Upgrading

- BREAKING: `media::SocketDemux::register` takes the session's local `ice-pwd` as well as its `ice-ufrag`, and `classify` maps a source only when `MESSAGE-INTEGRITY` verifies under that password; add the password at each `register` call.
- BREAKING: `ice::format_ice_server_links` returns `Result<String, Error>`; add `?`. A `url`, `username` or `credential` that could end the `<...>` value or forge the next link is `Error::InvalidHeader { header: "Link" }`.

## Changed

- BREAKING: `media::SocketDemux::register` takes the session's local `ice-pwd` as well as its local `ice-ufrag`, and `classify` maps a source address only when the check's `MESSAGE-INTEGRITY` verifies under that password (RFC 8445 §6.1.3). A check naming a known ufrag without the password maps nothing, so a sender who knows only the ufrag (not secret) can neither fill a session's tuple budget nor steal a mapped source address; the type's `Debug` names the registered ufrags but never a password (audit 20 finding 40). Migration: `register(ufrag, session)` -> `register(ufrag, pwd, session)`.
- BREAKING: `ice::format_ice_server_links` returns `Result<String, Error>`, rejecting a `url` that would end the `<...>` value or forge the next link (a control character, `<`, `>` or `"`) and a `username`/`credential` holding a control character, as `Error::InvalidHeader { header: "Link" }`. Migration: `format_ice_server_links(&servers)` -> `format_ice_server_links(&servers)?` (#2859).

## Fixed

- `media::MediaTransport` admits a new STUN source address to its remote-candidate budget only after the check's `USERNAME` and `MESSAGE-INTEGRITY` verify (the same RFC 8445 §6.1.3 checks the ICE agent applies), so junk or spoofed STUN datagrams can no longer exhaust `max_remote_candidates` before the peer's peer-reflexive address arrives (audit 20 finding 11).
- `WhepSession::counter_offer` rejects a `valid_until` holding a control character, `"` or `>` with the documented `500` and unchanged state, instead of interpolating it between the `Content-Type` parameter quotes (#2859).

---

Published from tag `webrtc-runtime-v0.9.0`.
