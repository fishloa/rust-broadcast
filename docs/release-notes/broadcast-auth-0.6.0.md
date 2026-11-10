---
status: DRAFT
---
# broadcast-auth 0.6.0

_Released 2026-11-10._

Breaking-class release (0.5 -> 0.6): `Verifier::forwarded` is removed and `AuthResult::BadRequest` is added; sibling epochs move.

Version: 0.5.2 -> 0.6.0.

## Breaking changes and migration

- `Verifier::forwarded` is removed. Use `Verifier::forwarded_from(user_header, forwarded_for_header, trusted_proxies)`. An empty list trusts nobody.

```rust
// before
Verifier::forwarded(user_header)
// after
Verifier::forwarded_from(user_header, forwarded_for_header, TrustedProxy::loopback())
```

- `AuthResult::BadRequest` (additive on `#[non_exhaustive]` enum): returned when a request carries more than one `Authorization` header. Callers that treat every non-`Ok` as 401 must now map it to 400.

## Added

- `Verifier::forwarded_from` and `TrustedProxy` (IP/CIDR, IPv4-mapped peers canonicalised, `TrustedProxy::loopback()`).
- `Error::InvalidTrustedProxy`.
- `RequestContext::header_count`.

## Fixed

- `RequestContext` Debug redacts `sig`, Cookie, Set-Cookie, X-Api-Key.
- IP-bound signed URL accepts IPv4-mapped IPv6 peer.
- Digest absolute-form `uri` must name request's own Host.
- Blank Bearer token never verifies blank credential.

## Other changes

- Digest: absolute-form `uri` with no Host refused.

---

Published from tag `broadcast-auth-v0.6.0`.
