# broadcast-auth 0.3.1 — 2026-09-26

Security fix for the Digest `Verifier`: nonces are now single-issue and expiring instead of
fixed. **Upgrade if you use `Verifier`'s Digest scheme.** No breaking API change; this is
additive (`Verifier::challenge_for`, `Verifier::with_clock`, `Verifier::with_digest_nc_capacity`,
and three new crate-root constants).

## Security

| Advisory | Before this release |
|---|---|
| GHSA-j7jp-f64w-73hv | `Verifier::challenge` handed out the same Digest nonce to every rejected request, with no expiry and no per-`nc` replay tracking. A captured `(nonce, cnonce, response)` from one exchange verified again indefinitely, and a client that answered an expired nonce was silently re-prompted rather than told the nonce was stale (RFC 7616 §3.3). |

## Behaviour changes

- **Nonces are per-challenge and expiring.** Each nonce is now `issue-time ‖ issue-sequence ‖
  HMAC-SHA256` under a per-`Verifier` random secret, and expires `DIGEST_NONCE_LIFETIME` (3600 s)
  after issue. `Verifier::challenge` returns a different nonce on every call.
- **Per-`nc` replay protection.** Each `(nonce, cnonce)` pair keeps its highest verified `nc`
  plus a 64-value anti-replay window (`NC_WINDOW`, RFC 4303 §3.4.3 style), so pipelined requests
  may arrive out of order but no `nc` verifies twice (RFC 7616 §3.3/§3.4). Up to
  `DIGEST_NC_TRACK_CAP` (65 536) pairs are tracked with least-recently-used eviction; a pair
  evicted under load is answered as stale rather than becoming replayable again. A clock that
  steps backward is clamped to the latest issue time seen, so it cannot resurrect an expired
  nonce.

## Added

- `Verifier::challenge_for(&RequestContext)`: the challenge to send for a rejected request. It
  carries `stale=true` when the request's Digest nonce had only expired, or its tracked pair was
  evicted — not when credentials were simply wrong — so a compliant client retries with the new
  nonce instead of re-prompting the user.
- `Verifier::with_clock`: replace the clock nonce ages are measured against (for deterministic
  tests, or a custom time source).
- `Verifier::with_digest_nc_capacity`: change how many `(nonce, cnonce)` pairs are tracked before
  least-recently-used eviction kicks in.
- `DIGEST_NONCE_LIFETIME`, `DIGEST_NC_TRACK_CAP`, `NC_WINDOW` at the crate root.

## Migration

No breaking change. A caller that builds its own 401/407 response should switch from
`Verifier::challenge` to `Verifier::challenge_for(&request)` so a stale-nonce rejection carries
`stale=true` and a compliant client can retry silently instead of re-prompting for credentials;
`challenge` itself is unchanged and still usable for an unconditional challenge.

`hls-runtime` and `multimux` pick up this fix in their own next releases; `multimux` also needs
this crate's floor raised to `0.3.1` because it now calls `Verifier::challenge_for` directly.

MSRV 1.95.0.
