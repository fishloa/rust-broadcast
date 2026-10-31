# rtsp-runtime 0.8.1

_Released 2026-10-31._

### Added
- `ClientSession::take_rtp_info` and `RtpInfoEntry`: the `RTP-Info` entries (RFC 2326 12.33) of a successful PLAY response.
### Changed
- Dependency `sdp-types` 0.2 -> 0.3 (used only by tests; not in the public API).
### Changed
- Dependency `getrandom` 0.2 -> 0.4 (internal OS-RNG call site: `getrandom::getrandom` -> `getrandom::fill`). No public API exposure.
### Changed
- Dependency `webpki-roots` 0.26 -> 1.0.

### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).
### Added
- `ClientSession::redirect_location()`: the `Location` header of the most recent redirection (3xx) response, cleared by any other response (`ClientEvent::Response` carries no headers). Used by `broadcast-client` to follow an RTSP redirect.
- `ClientSession::content_base()`: the `Content-Base` (else `Content-Location`) of the most recent successful DESCRIBE response, the base for relative `a=control` values (RFC 2326 Appendix C.1.1). `ClientEvent::Response` carries no headers.
- `ClientSession::take_negotiated_transport()`: takes the transport of the latest SETUP reply and clears it, so a later reply without a `Transport` header is detectable.
- `ClientSession::last_challenge()`: the most recent `WWW-Authenticate` (else `Proxy-Authenticate`) value seen on any response, recorded even when no credentials are configured, so a caller can learn the scheme and realm to ask for (`ClientEvent::Response` carries no headers).
- `ClientSession::authenticate_with(credentials, challenge)`: attaches credentials to a live session and builds the authenticator from the given (else the last recorded) challenge, so the next request already carries `Authorization`. Used by `broadcast-client`'s RTSP source to resume after a `401`.

---

Published from tag `rtsp-runtime-v0.8.1`.
