# broadcast-auth 0.4.0

_Released 2026-10-05._

Breaking (0.x minor), behaviour-tightening release of the shared HTTP/RTSP auth crate. There is no source-level API removal: the one new public item is an additive `Error` variant. The breaks are in what the `Verifier` accepts and in the bytes `SignedUrlKeySet::sign` produces. Requests that an older `Verifier` accepted can now get a 401, and URLs minted by this version differ from older ones for some inputs. Anyone running `Verifier` as an origin (multimux, hls-runtime, your own server) or minting signed URLs must read "Behaviour changes" below before upgrading clients and servers together. **This crate ships as 0.4.0, not 0.3.1.** The 0.3.1 release (Digest nonce expiry and replay protection) was prepared but never tagged; its content is included here, see [broadcast-auth-0.3.1.md](broadcast-auth-0.3.1.md). The latest published version before this one is 0.3.0.

## Highlights

- **Digest `username*` (RFC 7616 §3.4.4).** A non-ASCII user now authenticates through `username*=UTF-8''<pct-encoded>` (RFC 8187 ext-value). A raw non-ASCII `username` is rejected.
- **Digest nonces expire and cannot be replayed** (from the unpublished 0.3.1): `Verifier::challenge_for`, `Verifier::with_clock`, `Verifier::with_digest_nc_capacity`, and the crate-root constants `DIGEST_NONCE_LIFETIME`, `DIGEST_NC_TRACK_CAP`, `NC_WINDOW`. Servers that build their own 401 should call `Verifier::challenge_for(&ctx)` so a stale nonce carries `stale=true`.
- **Header injection closed.** The `WWW-Authenticate` realm is rendered as an escaped quoted-string (`"`, `\`, CR and LF can no longer break or split the header), and a Bearer token that cannot be a header value is refused by the client side instead of emitted.
- **Stricter, spec-conformant `Authorization` parsing** (RFC 7235 `auth-param` lists, RFC 7617 Basic).

## Behaviour changes (these cause 401s or different output)

Server side (`Verifier::verify`):

- **Digest `uri` must be in normalised spelling.** The client's absolute-form `uri` must round-trip through `url::Url` unchanged and match the request's path and query exactly. A client that hashed `http://H:80/x` or `rtsp://[2001:DB8::1]/x` (upper-case host or IPv6 literal, default port, no root `/`) was accepted before and now gets a 401. Fix on the client: send and hash the normalised spelling (`http://h/x`, `rtsp://[2001:db8::1]/x`). This is the substitution guard: `http://h/a/../b`, `%2e%2e` variants and `HTTP://` stay rejected.
- **Digest parameter parsing** uses `http-auth`'s `ChallengeParser`: quoted-pairs are honoured, parameter names are case-insensitive, a repeated parameter or a second challenge is rejected, and `userhash=true` is rejected. A raw non-ASCII `username` is rejected; use `username*`. `username*` with any charset other than UTF-8, malformed percent-encoding, invalid UTF-8, or both `username` and `username*` present are rejected.
- **Digest `qop` / `algorithm` are checked** (#1087): a request whose `qop` is not `auth`, or whose `algorithm` (when present) is not `MD5`, is rejected. Previously a mislabeled value was accepted if the hash still matched.
- **Auth-scheme token matches case-insensitively** for `Basic`/`Digest`/`Bearer` (RFC 7235 §2.1), and a literal comma inside a quoted Digest field value no longer splits the field list (#1087). These accept more than before.
- **Basic** is decoded with `headers::Authorization`: a configured user-id containing `:` can no longer match (RFC 7617 §2), and a non-UTF-8 payload yields `Unauthorized`.
- **Bearer** tokens are compared trimmed on both sides (a configured token with surrounding spaces still verifies, as before).
- **Realm** is stored as it is rendered (control characters other than HTAB dropped). A realm containing CR/LF now authenticates a Digest client. A Digest `username` containing `"` or `\` now verifies.
- **Signed URL, bare `ip` key.** `?ip` with no `=` used to be ignored, which silently dropped the IP binding. It is now rejected.

Minting side (`SignedUrlKeySet::sign`):

- The query is `application/x-www-form-urlencoded`. `kid` and `ip` are percent-encoded: `ip=2001:db8::1` is now emitted as `ip=2001%3Adb8%3A%3A1`. Verification percent-decodes, so URLs minted by older versions still verify, with one exception: a `+` in a `kid` of an older URL now reads as a space. Parameter order stays `exp`, `kid`, `sig`, `ip`. Decoding is lossy by design (`%FF` and `%EF%BF%BD` both read as U+FFFD, first occurrence of a key wins). Only the signed fields (path, `exp`, `ip`) matter to the signature, so this opens no bypass.

## API additions

- `Error::InvalidBearerToken` (`Error` is `#[non_exhaustive]`): returned by `Authenticator::authorization` when the Bearer token contains a byte outside visible ASCII.

A caller that previously forwarded the token unconditionally should handle this error:

```rust
match authenticator.authorization(&ctx) {
    Ok(header) => send(header),
    Err(broadcast_auth::Error::InvalidBearerToken) => reject_config(),
    Err(e) => return Err(e.into()),
}
```

## Dependencies

Verified from the diff of `broadcast-auth/Cargo.toml` against `broadcast-auth-v0.3.0`:

```toml
# bumped (RustCrypto 0.13 generation; Digest MD5 responses and HMAC-SHA256 signatures are byte-identical)
base64 = "0.22" -> "0.23"
md-5   = "0.10" -> "0.11"
hmac   = "0.12" -> "0.13"   # default-features = false
sha2   = "0.10" -> "0.11"   # default-features = false
# new
headers          = "0.4"
percent-encoding = "2"
url              = "2"
lru              = "0.18"
form_urlencoded  = "1"
hex              = { version = "0.4", default-features = false, features = ["alloc"] }
```

`rand = "0.8"` and `subtle = "2"` are unchanged. Known-answer tests now pin the RFC 2617 §3.5 and RFC 7617 §2 examples.

## Fixes

- Digest `Verifier` accepted a captured `(nonce, cnonce, response)` indefinitely (fixed nonce, no expiry, no `nc` tracking): nonces are now per-challenge and expire after `DIGEST_NONCE_LIFETIME` (3600 s), with a 64-value anti-replay window per `(nonce, cnonce)` pair and up to `DIGEST_NC_TRACK_CAP` (65 536) pairs tracked with LRU eviction. See the 0.3.1 note for detail.
- `qop`/`algorithm` not validated, auth-scheme case sensitivity, and comma-in-quoted-value splitting (#1087, above).
- Realm and Bearer header injection (above).

## Read together with

- [broadcast-auth-0.3.1.md](broadcast-auth-0.3.1.md) (content shipped in this release)
- Consumers in this wave that build on this crate: [rtsp-runtime-0.7.0.md](rtsp-runtime-0.7.0.md).

---

Published from tag `broadcast-auth-v0.4.0`.
