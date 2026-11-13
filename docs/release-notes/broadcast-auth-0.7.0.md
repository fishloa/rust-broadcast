# broadcast-auth 0.7.0

_Released 2026-11-14._

Breaking-class release (0.6 -> 0.7): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.6.0 -> 0.7.0.

## Changed
- BREAKING: `Verifier::new` returns `Result<Verifier>`; building a Digest verifier when the OS random source is unavailable is the new `Error::RandomSource` instead of a panic. There is no panicking variant (#2591).

## Added
- `AuthState`, `AuthDecision`, `AuthEvent` and `carries_credentials`: the sans-IO 401/407 challenge-answer flow with its cookie jar, moved here from `broadcast-client` so a client primitive is not reached for through the session crate. `AuthState` scopes credentials to one origin (RFC 6454 §4), tries every line of a multi-line `WWW-Authenticate`, replays once after a Digest `stale=true` re-challenge, and stores `Set-Cookie` (RFC 6265) for the origin's later requests; it is `Clone` only as a way to start a fresh exchange (a clone carries no negotiated authenticator and no cookies). `broadcast-client`, `hls-runtime` and `multimux` all answer challenges through it. `broadcast_client::auth` re-exports the same types.

## Fixed
- `credentials_from_url` returns credentials for an empty username with a password (`scheme://:pw@host/`) instead of `None`: RFC 3986 §3.2.1 allows an empty user, and RFC 7617 §2 and RFC 7616 §3.4 carry it (#2587).

## Dependencies
- New sibling dependencies: `broadcast-common` 12.

---

Published from tag `broadcast-auth-v0.7.0`.
