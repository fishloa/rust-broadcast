//! Server-side challenge + verify — the origin half of RFC 7235
//! (`WWW-Authenticate`/`Authorization`), RFC 7617 (Basic), RFC 7616 (Digest),
//! RFC 2326 §14 (RTSP's reuse of the same two schemes), and RFC 6750
//! (Bearer).
//!
//! [`crate::Authenticator`]/[`crate::respond`] are the *client* half: answer
//! a challenge. [`Verifier`] is the other side of the same handshake: an
//! origin (multimux's shared output auth gating every `/{stream}/…` route,
//! issue #663; or any other credentialed origin in this workspace) builds
//! one from a configured [`Credentials`] + realm, calls [`Verifier::challenge`]
//! for the `WWW-Authenticate` value to send on a `401`, and
//! [`Verifier::verify`] to check an incoming `Authorization` header.
//!
//! Promoted from `multimux`'s test-only mock auth server
//! (`multimux::testutil`, issue #663 "Finish client-side multi-scheme
//! auth"): that module's Digest verification was already a real,
//! independent RFC 7616 §3.4.1 computation (not a literal-string match)
//! purely to drive multimux's own client-side tests against something that
//! genuinely rejects wrong credentials. This module is that same
//! computation, promoted into the shared crate so it is the *production*
//! server-side verifier (multimux's output-auth middleware) rather than a
//! test-only fixture, and so no crate hand-rolls a second copy.
//!
//! # Verification per scheme
//!
//! - **Basic** (RFC 7617 §2): the header's base64 payload is decoded and
//!   compared, in constant time, against `"{username}:{password}"`.
//! - **Bearer** (RFC 6750 §2.1): the token is compared, in constant time,
//!   against the configured token.
//! - **Digest** (RFC 7616 §3.4.1): `HA1 = MD5(username:realm:password)`,
//!   `HA2 = MD5(method:digest-uri-value)`, `response =
//!   MD5(HA1:nonce:nc:cnonce:qop:HA2)` — `qop=auth`/`algorithm=MD5` only (the
//!   one shape every client in this workspace answers) — recomputed and
//!   compared, in constant time, against the client's `response` field.
//!   `digest-uri-value` is the client's own claimed `uri` field (RFC 7616
//!   §3.4.1: HA2 is always computed over what the client actually hashed),
//!   not the server's request URI — the two need not be textually identical,
//!   only to refer to the same request-target (see below). The client's
//!   claimed `uri` field must also match the actual request URI (RFC 7616
//!   §3.4.1: the server "SHOULD check" this), not merely be internally
//!   consistent with its own `response` — but RFC 7230 §5.3 permits a
//!   request-target in either origin-form (`/path`) or absolute-form
//!   (`scheme://authority/path`), and a legitimate client may hash either;
//!   [`digest_uri_matches`] accepts both representations of the same target
//!   while still rejecting a genuinely different one.
//! - **Forwarded** ([`Self::forwarded`], issue #663 extensibility wave part
//!   1): not an RFC 7235 challenge scheme at all — trusts that a fronting
//!   reverse proxy has already authenticated the caller and forwards the
//!   authenticated username in a configured header (conventionally
//!   `X-Forwarded-User`). Authenticated iff that header is present and
//!   non-empty. **Safe ONLY behind a trusted reverse proxy that strips any
//!   client-supplied copies of that header (and of the forwarded-for header,
//!   if configured) before forwarding** — this crate performs no such
//!   stripping and trusts [`crate::RequestContext::headers`] completely; a
//!   direct or spoofed client could otherwise set the header itself and
//!   bypass authentication entirely. [`Self::challenge`] returns just the
//!   bare scheme name for diagnostics (there is no challenge/response
//!   round-trip a direct client could answer).
//! - **SignedUrl** ([`Self::signed_url`], issue #747): a CDN-style,
//!   short-lived, tamper-proof token in the URL's own query string — no
//!   `Authorization` header at all, so a player can fetch segments without
//!   carrying a credential. See [`crate::signed_url`] for the full wire
//!   form, canonical string, and rejection semantics. Like `Forwarded`,
//!   [`Self::challenge`] returns just the bare scheme name — there is no
//!   `WWW-Authenticate` round-trip a client could answer for a query-string
//!   token.
//!
//! # Nonce handling (RFC 7616 §3.3 / §5.4)
//!
//! A `Digest` [`Verifier`] issues a fresh nonce on every
//! [`Verifier::challenge`] call: `issue-time ‖ salt ‖ HMAC-SHA256(secret,
//! issue-time ‖ salt)`, hex-encoded, where `secret` is drawn from the OS RNG
//! once per verifier. A nonce is accepted only if its HMAC checks out and it
//! is younger than [`DIGEST_NONCE_LIFETIME`]; a correctly answered but expired
//! nonce is reported via [`Verifier::challenge_for`], which then carries
//! `stale=true` so the client retries with the new nonce without re-prompting.
//! The highest `nc` accepted for each `(nonce, cnonce)` pair is remembered
//! (at most [`DIGEST_NC_TRACK_CAP`] pairs, expired ones evicted first) and a
//! request whose `nc` does not exceed it is rejected, so a captured
//! `Authorization` header cannot be replayed. The verifier's clock defaults
//! to [`SystemTime::now`] and can be replaced with [`Verifier::with_clock`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use md5::{Digest as _, Md5};
use sha2::Sha256;

use crate::credentials::Credentials;
use crate::request::RequestContext;
use crate::signed_url::{self, SignedUrlKeySet};

/// The outcome of [`Verifier::verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthResult {
    /// The `Authorization` header (or absence of one) satisfies the
    /// verifier's configured credential.
    Ok,
    /// Missing, malformed, or wrong-credential `Authorization` — the caller
    /// should respond `401` with [`Verifier::challenge`].
    Unauthorized,
}

/// Per-scheme state a [`Verifier`] holds — mirrors [`Credentials`] but adds
/// the realm (Basic/Digest) and the nonce secret + `nc` tracker (Digest; see
/// the module docs' nonce handling).
enum VerifierScheme {
    Basic {
        username: String,
        password: String,
        realm: String,
    },
    Digest {
        username: String,
        password: String,
        realm: String,
        nonces: DigestNonces,
    },
    Bearer {
        token: String,
    },
    /// Reverse-proxy forwarded-auth (see the module docs) — no
    /// `Credentials`/realm/nonce at all, since there is no client-answered
    /// challenge for this scheme.
    Forwarded {
        user_header: String,
        forwarded_for_header: Option<String>,
    },
    /// HMAC signed-URL (see the module docs / [`crate::signed_url`]) — no
    /// `Credentials`/realm/nonce either: the token lives in the request's own
    /// query string, verified against `keys`.
    SignedUrl {
        keys: SignedUrlKeySet,
    },
}

/// Challenges + verifies incoming requests against one configured
/// [`Credentials`] (RFC 7235 origin-side auth) — see the module docs.
pub struct Verifier {
    scheme: VerifierScheme,
    clock: Clock,
}

/// The time source a [`Verifier`] reads nonce ages from.
type Clock = Box<dyn Fn() -> SystemTime + Send + Sync>;

/// How long an issued Digest nonce is accepted (RFC 7616 §5.4 leaves the
/// lifetime to the server). An older nonce is answered with `stale=true`.
pub const DIGEST_NONCE_LIFETIME: Duration = Duration::from_secs(300);

/// Most `(nonce, cnonce)` pairs whose highest `nc` a Digest [`Verifier`]
/// remembers at once; past this, expired pairs are evicted first, then the
/// oldest-issued ones.
pub const DIGEST_NC_TRACK_CAP: usize = 4096;

/// Bytes of per-verifier HMAC key drawn from the OS RNG.
const NONCE_SECRET_LEN: usize = 32;
/// Bytes of the big-endian issue time (seconds since the Unix epoch).
const NONCE_TIME_LEN: usize = 8;
/// Bytes of per-challenge random salt, so two challenges in the same second
/// still get distinct nonces.
const NONCE_SALT_LEN: usize = 8;
/// Bytes of HMAC-SHA256 output.
const NONCE_MAC_LEN: usize = 32;
/// Bytes of the decoded nonce.
const NONCE_LEN: usize = NONCE_TIME_LEN + NONCE_SALT_LEN + NONCE_MAC_LEN;
/// RFC 7616 §3.4: `nc-value = 8LHEX`.
const NC_HEX_LEN: usize = 8;

/// Digest nonce secret plus the per-`(nonce, cnonce)` highest-`nc` table.
struct DigestNonces {
    secret: [u8; NONCE_SECRET_LEN],
    seen: Mutex<NcTable>,
}

#[derive(Default)]
struct NcTable {
    /// `(nonce, cnonce)` → (nonce issue time in seconds, highest accepted `nc`).
    entries: HashMap<(String, String), (u64, u32)>,
    /// Nonces issued before this time (seconds) are accepted only for pairs
    /// still in `entries` — set when a live pair had to be evicted.
    floor: u64,
}

/// What a Digest `Authorization` header amounts to, before any `nc` is
/// recorded.
enum DigestCheck {
    /// Correct response on a live nonce with a fresh `nc`.
    Accept {
        key: (String, String),
        issued: u64,
        nc: u32,
    },
    /// Correct response, but the nonce is past its lifetime.
    Stale,
    /// Anything else.
    Reject,
}

impl DigestNonces {
    fn new() -> Self {
        DigestNonces {
            secret: rand::random(),
            seen: Mutex::new(NcTable::default()),
        }
    }

    fn mac(&self, time_and_salt: &[u8]) -> Hmac<Sha256> {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.secret)
            .expect("HMAC accepts a key of any length");
        mac.update(time_and_salt);
        mac
    }

    /// A fresh nonce issued at `now_secs`.
    fn issue(&self, now_secs: u64) -> String {
        let mut raw = [0u8; NONCE_LEN];
        raw[..NONCE_TIME_LEN].copy_from_slice(&now_secs.to_be_bytes());
        let salt: [u8; NONCE_SALT_LEN] = rand::random();
        raw[NONCE_TIME_LEN..NONCE_TIME_LEN + NONCE_SALT_LEN].copy_from_slice(&salt);
        let tag = self
            .mac(&raw[..NONCE_TIME_LEN + NONCE_SALT_LEN])
            .finalize()
            .into_bytes();
        raw[NONCE_TIME_LEN + NONCE_SALT_LEN..].copy_from_slice(&tag);
        hex(&raw)
    }

    /// The issue time of `nonce` if this verifier minted it, else `None`.
    fn issued_at(&self, nonce: &str) -> Option<u64> {
        let raw = unhex::<NONCE_LEN>(nonce)?;
        let (signed, tag) = raw.split_at(NONCE_TIME_LEN + NONCE_SALT_LEN);
        self.mac(signed).verify_slice(tag).ok()?;
        let mut time = [0u8; NONCE_TIME_LEN];
        time.copy_from_slice(&raw[..NONCE_TIME_LEN]);
        Some(u64::from_be_bytes(time))
    }

    fn is_expired(issued: u64, now_secs: u64) -> bool {
        now_secs.saturating_sub(issued) >= DIGEST_NONCE_LIFETIME.as_secs()
    }

    /// Records `nc` for `key` if it exceeds the highest seen so far; `false`
    /// when it does not (a repeated or reordered request).
    fn record(&self, key: (String, String), issued: u64, nc: u32, now_secs: u64) -> bool {
        let mut table = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, highest)) = table.entries.get_mut(&key) {
            if nc <= *highest {
                return false;
            }
            *highest = nc;
            return true;
        }
        if issued < table.floor {
            return false;
        }
        if table.entries.len() >= DIGEST_NC_TRACK_CAP {
            table
                .entries
                .retain(|_, (t, _)| !Self::is_expired(*t, now_secs));
        }
        if table.entries.len() >= DIGEST_NC_TRACK_CAP
            && let Some((oldest_key, oldest)) = table
                .entries
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, (t, _))| (k.clone(), *t))
        {
            table.entries.remove(&oldest_key);
            table.floor = table.floor.max(oldest.saturating_add(1));
            if issued < table.floor {
                return false;
            }
        }
        table.entries.insert(key, (issued, nc));
        true
    }

    /// Whether an untracked pair under a nonce issued at `issued` would be
    /// refused because live pairs of that age were evicted.
    fn below_floor(&self, key: &(String, String), issued: u64) -> bool {
        let table = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        issued < table.floor && !table.entries.contains_key(key)
    }
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn default_clock() -> Clock {
    Box::new(SystemTime::now)
}

impl Verifier {
    /// Builds a verifier for `credentials`, using `realm` for the
    /// `WWW-Authenticate` challenge (Basic/Digest only — RFC 6750 Bearer has
    /// no realm parameter in this crate's minimal challenge, see
    /// [`Self::challenge`]).
    ///
    /// For `Credentials::Digest`, a random nonce-signing secret is generated
    /// now; every [`Self::challenge`] then issues a fresh, time-limited nonce
    /// (see the module docs' nonce handling).
    pub fn new(credentials: Credentials, realm: impl Into<String>) -> Self {
        let realm = realm.into();
        let scheme = match credentials {
            Credentials::Basic { username, password } => VerifierScheme::Basic {
                username,
                password,
                realm,
            },
            Credentials::Digest { username, password } => VerifierScheme::Digest {
                username,
                password,
                realm,
                nonces: DigestNonces::new(),
            },
            Credentials::Bearer { token } => VerifierScheme::Bearer { token },
        };
        Verifier {
            scheme,
            clock: default_clock(),
        }
    }

    /// Replaces the clock Digest nonce ages are measured against (default
    /// [`SystemTime::now`]) — for sans-IO drivers and tests.
    pub fn with_clock(mut self, clock: impl Fn() -> SystemTime + Send + Sync + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    fn now_secs(&self) -> u64 {
        unix_secs((self.clock)())
    }

    /// Builds a verifier for the reverse-proxy forwarded-auth scheme (see the
    /// module docs' trust assumption — read it before using this).
    ///
    /// `user_header` (conventionally `X-Forwarded-User`) is the header whose
    /// presence (non-empty) [`Self::verify`] treats as "the proxy already
    /// authenticated this caller". `forwarded_for_header` (conventionally
    /// `Some("X-Forwarded-For".to_string())`), if configured, is read back by
    /// [`Self::forwarded_for`] for observability only — this crate makes no
    /// trust decision based on it.
    pub fn forwarded(user_header: impl Into<String>, forwarded_for_header: Option<String>) -> Self {
        Verifier {
            scheme: VerifierScheme::Forwarded {
                user_header: user_header.into(),
                forwarded_for_header,
            },
            clock: default_clock(),
        }
    }

    /// Builds a verifier for the HMAC signed-URL scheme (issue #747) —
    /// see [`crate::signed_url`] for the wire form, canonical string, and
    /// rejection semantics.
    pub fn signed_url(keys: SignedUrlKeySet) -> Self {
        Verifier {
            scheme: VerifierScheme::SignedUrl { keys },
            clock: default_clock(),
        }
    }

    /// The `WWW-Authenticate` header value to send on a `401` in response to
    /// a missing/failed [`Self::verify`] call.
    ///
    /// `Forwarded` (built via [`Self::forwarded`]) has no real RFC 7235
    /// challenge (a direct client cannot answer it — see the module docs);
    /// this just names the scheme for diagnostics.
    ///
    /// For Digest every call issues a fresh nonce; prefer
    /// [`Self::challenge_for`] when the rejected request is at hand, so an
    /// expired nonce is flagged `stale=true`.
    pub fn challenge(&self) -> String {
        self.render_challenge(false)
    }

    /// Like [`Self::challenge`], but for the request that [`Self::verify`]
    /// just rejected: when that request answered a Digest challenge
    /// correctly and only its nonce had expired, the new challenge carries
    /// `stale=true` (RFC 7616 §3.3) so the client retries without
    /// re-prompting for credentials. Identical to [`Self::challenge`] for
    /// every other scheme and outcome.
    pub fn challenge_for(&self, ctx: &RequestContext<'_>) -> String {
        let stale = match &self.scheme {
            VerifierScheme::Digest {
                username,
                password,
                realm,
                nonces,
            } => ctx.header("authorization").is_some_and(|header| {
                matches!(
                    check_digest(
                        header,
                        username,
                        password,
                        realm,
                        nonces,
                        ctx.method,
                        ctx.uri,
                        self.now_secs(),
                    ),
                    DigestCheck::Stale
                )
            }),
            _ => false,
        };
        self.render_challenge(stale)
    }

    fn render_challenge(&self, stale: bool) -> String {
        match &self.scheme {
            VerifierScheme::Basic { realm, .. } => format!("Basic realm=\"{realm}\""),
            VerifierScheme::Digest { realm, nonces, .. } => {
                let nonce = nonces.issue(self.now_secs());
                let stale = if stale { ", stale=true" } else { "" };
                format!(
                    "Digest realm=\"{realm}\", nonce=\"{nonce}\", qop=\"auth\", algorithm=MD5{stale}"
                )
            }
            VerifierScheme::Bearer { .. } => "Bearer".to_string(),
            VerifierScheme::Forwarded { .. } => "Forwarded".to_string(),
            VerifierScheme::SignedUrl { .. } => "SignedUrl".to_string(),
        }
    }

    /// Verifies an incoming request against this verifier's configured
    /// scheme.
    ///
    /// Basic/Digest/Bearer read `ctx`'s `Authorization` header
    /// ([`RequestContext::header`], case-insensitive) — missing entirely is
    /// `Unauthorized`, same as before this took a full [`RequestContext`].
    /// `ctx.method` feeds Digest's `HA2` directly; `ctx.uri` is the request
    /// URI the client's claimed `uri` field is matched against (RFC 7616
    /// §3.4.1's SHOULD, accepting either origin-form or absolute-form —
    /// unused for Basic/Bearer.
    /// Forwarded reads `ctx`'s configured user header instead — see the
    /// module docs. SignedUrl reads `ctx.uri`'s own query string (`exp`/
    /// `kid`/`sig`\[/`ip`\]) instead of any header at all, and `ctx.peer_addr`
    /// when the token is IP-scoped — see [`crate::signed_url`].
    ///
    /// A pathologically large `Digest` `Authorization` header is rejected
    /// outright rather than parsed (see `MAX_DIGEST_FIELDS`) — this bounds
    /// the per-request allocation cost, but is not a substitute for a
    /// transport-level cap on header size, which callers should also enforce.
    pub fn verify(&self, ctx: &RequestContext<'_>) -> AuthResult {
        let ok = match &self.scheme {
            VerifierScheme::Basic {
                username, password, ..
            } => ctx
                .header("authorization")
                .is_some_and(|header| verify_basic(header, username, password)),
            VerifierScheme::Bearer { token } => ctx
                .header("authorization")
                .is_some_and(|header| verify_bearer(header, token)),
            VerifierScheme::Digest {
                username,
                password,
                realm,
                nonces,
            } => ctx.header("authorization").is_some_and(|header| {
                let now = self.now_secs();
                match check_digest(
                    header, username, password, realm, nonces, ctx.method, ctx.uri, now,
                ) {
                    DigestCheck::Accept { key, issued, nc } => nonces.record(key, issued, nc, now),
                    DigestCheck::Stale | DigestCheck::Reject => false,
                }
            }),
            VerifierScheme::Forwarded { user_header, .. } => verify_forwarded(ctx, user_header),
            VerifierScheme::SignedUrl { keys } => signed_url::verify(ctx, keys),
        };
        if ok {
            AuthResult::Ok
        } else {
            AuthResult::Unauthorized
        }
    }

    /// For a [`Self::forwarded`] verifier with a configured
    /// `forwarded_for_header`, returns that header's value from `ctx` — for
    /// tracing/observability only; this crate makes no trust decision with
    /// it (the module docs' trust assumption is what actually matters).
    /// `None` for any other verifier, or when no such header is
    /// configured/present in `ctx`.
    pub fn forwarded_for<'a>(&self, ctx: &RequestContext<'a>) -> Option<&'a str> {
        match &self.scheme {
            VerifierScheme::Forwarded {
                forwarded_for_header: Some(header_name),
                ..
            } => ctx.header(header_name),
            _ => None,
        }
    }
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): every scheme carries a
/// secret (`password`/`token`) that must never render verbatim.
impl core::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let scheme = match &self.scheme {
            VerifierScheme::Basic { .. } => "Basic",
            VerifierScheme::Digest { .. } => "Digest",
            VerifierScheme::Bearer { .. } => "Bearer",
            VerifierScheme::Forwarded { .. } => "Forwarded",
            VerifierScheme::SignedUrl { .. } => "SignedUrl",
        };
        f.debug_struct("Verifier")
            .field("scheme", &scheme)
            .finish_non_exhaustive()
    }
}

/// RFC 7617 §2: decode the base64 payload and compare, in constant time,
/// against `"{username}:{password}"`.
fn verify_basic(header: &str, username: &str, password: &str) -> bool {
    let Some(encoded) = header.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let expected = format!("{username}:{password}");
    constant_time_eq(&decoded, expected.as_bytes())
}

/// RFC 6750 §2.1: compare the bearer token, in constant time.
fn verify_bearer(header: &str, token: &str) -> bool {
    let Some(sent) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(sent.trim().as_bytes(), token.as_bytes())
}

/// A real Digest `Authorization` response (RFC 7616 §3.4.1) carries under 15
/// `key=value` fields (`username`, `realm`, `nonce`, `uri`, `response`,
/// `algorithm`, `cnonce`, `opaque`, `qop`, `nc`, plus a couple of optional
/// extensions). Capping well above that bounds [`verify_digest`]'s
/// `HashMap` allocation against a request carrying a pathologically large
/// `Authorization` header (a huge field count forcing a huge per-request
/// map) without rejecting any legitimate client.
const MAX_DIGEST_FIELDS: usize = 64;

/// RFC 7616 §3.4.1: parse the `Digest` `Authorization` header's
/// `key=value`/`key="value"` fields, check the nonce was minted by `nonces`,
/// independently recompute the expected `response`, and compare in constant
/// time — `qop=auth`/`algorithm=MD5` only (the one shape every client in this
/// workspace answers). Records nothing; the caller records an accepted `nc`.
///
/// `HA2` is computed over the client's own claimed `uri` field (the
/// `digest-uri-value` RFC 7616 §3.4.1 defines HA2 over) — not `request_uri` —
/// since that is what the client actually hashed into its `response`. The
/// client's claimed `uri` is separately checked against `request_uri` (RFC
/// 7616 §3.4.1's SHOULD) via [`digest_uri_matches`], which accepts either
/// legal RFC 7230 request-target representation of the same target
/// (origin-form or absolute-form) while still rejecting a genuinely
/// different `uri`.
///
/// Rejects outright (without building the field map) a header carrying more
/// than [`MAX_DIGEST_FIELDS`] comma-separated fields — see that constant's
/// docs.
#[allow(clippy::too_many_arguments)]
fn check_digest(
    header: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonces: &DigestNonces,
    method: &str,
    request_uri: &str,
    now_secs: u64,
) -> DigestCheck {
    let Some(rest) = header.strip_prefix("Digest ") else {
        return DigestCheck::Reject;
    };
    if rest.split(',').count() > MAX_DIGEST_FIELDS {
        return DigestCheck::Reject;
    }
    let mut fields = HashMap::new();
    for part in rest.split(',') {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        fields.insert(key.trim(), value.trim().trim_matches('"'));
    }
    let get = |k: &str| fields.get(k).copied().unwrap_or_default();

    if get("username") != username || get("realm") != realm {
        return DigestCheck::Reject;
    }
    let nonce = get("nonce");
    let Some(issued) = nonces.issued_at(nonce) else {
        return DigestCheck::Reject;
    };
    let client_uri = get("uri");
    if !digest_uri_matches(client_uri, request_uri) {
        return DigestCheck::Reject;
    }
    let nc_text = get("nc");
    let cnonce = get("cnonce");
    let qop = get("qop");
    let client_response = get("response");
    if cnonce.is_empty() || client_response.is_empty() {
        return DigestCheck::Reject;
    }
    let Some(nc) = parse_nc(nc_text) else {
        return DigestCheck::Reject;
    };

    let ha1 = md5_hex(format!("{username}:{realm}:{password}"));
    let ha2 = md5_hex(format!("{method}:{client_uri}"));
    let expected_response = md5_hex(format!("{ha1}:{nonce}:{nc_text}:{cnonce}:{qop}:{ha2}"));
    if !constant_time_eq(expected_response.as_bytes(), client_response.as_bytes()) {
        return DigestCheck::Reject;
    }
    let key = (nonce.to_owned(), cnonce.to_owned());
    if DigestNonces::is_expired(issued, now_secs) || nonces.below_floor(&key, issued) {
        return DigestCheck::Stale;
    }
    DigestCheck::Accept { key, issued, nc }
}

/// RFC 7616 §3.4: `nc` is exactly eight hex digits.
fn parse_nc(nc: &str) -> Option<u32> {
    if nc.len() != NC_HEX_LEN || !nc.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(nc, 16).ok()
}

/// RFC 7616 §3.4.1's SHOULD-check: does the client's claimed Digest `uri`
/// field refer to the same request-target as `request_uri` (the actual
/// request the server is verifying against)?
///
/// RFC 7230 §5.3 permits a request-target in either **origin-form**
/// (`/path[?query]`) or **absolute-form** (`scheme://authority/path[?query]`)
/// — a legitimate client may hash either, and `request_uri` here is always
/// whatever form the caller's own request line/context uses (in this
/// workspace, always origin-form for HTTP). This accepts:
/// - `client_uri == request_uri` verbatim (the origin-form case), or
/// - `client_uri` in absolute-form whose path(+query) — everything from the
///   first `/` after the `"://"` authority — is byte-identical to
///   `request_uri`.
///
/// Anything else is rejected. This is a real substitution guard, not a
/// prefix/suffix check: a `client_uri` that merely contains or is suffixed by
/// `request_uri` (or vice versa) does NOT match.
fn digest_uri_matches(client_uri: &str, request_uri: &str) -> bool {
    if client_uri == request_uri {
        return true;
    }
    if let Some((_scheme, after_scheme)) = client_uri.split_once("://")
        && let Some(slash) = after_scheme.find('/')
    {
        return &after_scheme[slash..] == request_uri;
    }
    false
}

/// Reverse-proxy forwarded-auth (see the module docs): authenticated iff
/// `user_header` is present in `ctx` and non-empty (after trimming) — the
/// proxy having already verified the caller's identity. No credential/secret
/// is compared here, so no constant-time comparison is needed.
fn verify_forwarded(ctx: &RequestContext<'_>, user_header: &str) -> bool {
    ctx.header(user_header)
        .is_some_and(|v| !v.trim().is_empty())
}

/// Lowercase-hex MD5 digest of `input`.
fn md5_hex(input: String) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Byte-equality that does not short-circuit on the first differing byte —
/// only the *length* check short-circuits (an equal-length requirement is
/// not itself the secret being protected). Guards against a timing
/// side-channel on the password/token/digest-response comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes exactly `N` bytes of hex, or `None`.
fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (byte, pair) in out.iter_mut().zip(text.chunks_exact(2)) {
        if !pair.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let pair = core::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Credentials, RequestContext, respond};

    const REALM: &str = "cameras";

    /// Test helper: builds a [`RequestContext`] carrying `authorization` (if
    /// any) as the `Authorization` header, then verifies it — stands in for
    /// the pre-#663-extensibility-wave-1 `Verifier::verify(Option<&str>,
    /// &str, &str)` signature so the tests below read the same as before.
    fn verify_auth(
        v: &Verifier,
        authorization: Option<&str>,
        method: &str,
        uri: &str,
    ) -> AuthResult {
        let auth_header = authorization.map(|h| [("authorization", h)]);
        let headers: &[(&str, &str)] = match &auth_header {
            Some(arr) => arr,
            None => &[],
        };
        let ctx = RequestContext::new(method, uri).with_headers(headers);
        v.verify(&ctx)
    }

    // --- challenge() shape ---

    #[test]
    fn basic_challenge_names_the_realm() {
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        assert_eq!(v.challenge(), "Basic realm=\"cameras\"");
    }

    #[test]
    fn digest_challenge_carries_realm_nonce_qop_algorithm() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let challenge = v.challenge();
        assert!(challenge.starts_with("Digest "), "got: {challenge}");
        for needle in [
            "realm=\"cameras\"",
            "nonce=",
            "qop=\"auth\"",
            "algorithm=MD5",
        ] {
            assert!(
                challenge.contains(needle),
                "missing {needle} in {challenge}"
            );
        }
    }

    #[test]
    fn bearer_challenge_is_bare_scheme_name() {
        let v = Verifier::new(Credentials::bearer("tok"), REALM);
        assert_eq!(v.challenge(), "Bearer");
    }

    #[test]
    fn digest_challenges_issue_distinct_nonces() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        assert_ne!(v.challenge(), v.challenge());
    }

    // --- round trip: a client's respond() to challenge() must verify() Ok ---

    #[test]
    fn basic_respond_to_challenge_verifies_ok() {
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream"),
            AuthResult::Ok
        );
    }

    #[test]
    fn digest_respond_to_challenge_verifies_ok() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "12345")).unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "DESCRIBE", "rtsp://cam/live"),
            AuthResult::Ok
        );
    }

    #[test]
    fn bearer_respond_to_challenge_verifies_ok() {
        let v = Verifier::new(Credentials::bearer("mytoken123"), REALM);
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::bearer("mytoken123"),
        )
        .unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream"),
            AuthResult::Ok
        );
    }

    // --- wrong credentials -> Unauthorized (must BITE) ---

    #[test]
    fn basic_wrong_password_is_unauthorized() {
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::new("admin", "WRONG"),
        )
        .unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn digest_wrong_password_is_unauthorized() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "WRONG")).unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "DESCRIBE", "rtsp://cam/live"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn digest_mismatched_request_uri_is_unauthorized() {
        // A digest response computed for one URI must not verify against a
        // different URI the caller passes to `verify` (RFC 7616 SHOULD-check
        // that the header's `uri` matches the actual request).
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "12345")).unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "DESCRIBE", "rtsp://cam/OTHER"),
            AuthResult::Unauthorized
        );
    }

    /// RFC 7230 §5.3.2: a client may legally answer a Digest challenge using
    /// the absolute-form request-target instead of origin-form — e.g.
    /// multimux's outbound HTTP client (`source::http_auth::authenticated_get`,
    /// issue #724) sends the absolute URL as `uri`. The server here only ever
    /// sees the request's path (origin-form) as its own request `uri`; RFC
    /// 7616 §3.4.1 permits this because HA2 is computed over the CLIENT's
    /// claimed `uri`, and the SHOULD uri-match ([`digest_uri_matches`])
    /// accepts either representation of the same target. Built via the real
    /// `respond()` round-trip (not a rigged expected string) so this exercises
    /// the true client computation.
    #[test]
    fn digest_accepts_absolute_form_client_uri_matching_request_path() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let client_ctx = RequestContext::new("GET", "http://cam.local/stream/media.m3u8");
        let header = respond(
            &v.challenge(),
            &client_ctx,
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        assert!(
            header.contains("uri=\"http://cam.local/stream/media.m3u8\""),
            "expected the client to hash the absolute-form uri, got: {header}"
        );
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream/media.m3u8"),
            AuthResult::Ok
        );
    }

    /// Regression/mutation guard: an absolute-form `uri` whose PATH is
    /// genuinely different from the request must still be rejected — the
    /// SHOULD uri-match is a real substitution guard, not a rubber stamp for
    /// any absolute-form uri. Note this also exercises the response-mismatch
    /// path independently of the match check: because HA2 is computed over
    /// the client's own claimed uri, the client here computes a
    /// self-consistent (but wrong-target) response, so a neutered
    /// `digest_uri_matches` (hardcoded `true`) would let this wrongly verify
    /// — this test must fail if that guard is ever dropped.
    #[test]
    fn digest_rejects_absolute_form_uri_with_wrong_path() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let client_ctx = RequestContext::new("GET", "http://cam.local/other/path");
        let header = respond(
            &v.challenge(),
            &client_ctx,
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream/media.m3u8"),
            AuthResult::Unauthorized
        );
    }

    /// Same substitution guard, origin-form vs. origin-form (no scheme at
    /// all): a client claiming a different path outright must be rejected.
    #[test]
    fn digest_rejects_origin_form_uri_with_wrong_path() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let client_ctx = RequestContext::new("GET", "/other/path");
        let header = respond(
            &v.challenge(),
            &client_ctx,
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream/media.m3u8"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn digest_uri_matches_unit_cases() {
        // Origin-form, identical.
        assert!(digest_uri_matches("/a/b", "/a/b"));
        // Absolute-form whose path matches.
        assert!(digest_uri_matches("http://host/a/b", "/a/b"));
        assert!(digest_uri_matches("https://host:8080/a/b?q=1", "/a/b?q=1"));
        // Wrong path in either form.
        assert!(!digest_uri_matches("/a/c", "/a/b"));
        assert!(!digest_uri_matches("http://host/a/c", "/a/b"));
        // Not a suffix/prefix rubber stamp.
        assert!(!digest_uri_matches("http://host/x/a/b", "/a/b"));
        assert!(!digest_uri_matches("/a/b/extra", "/a/b"));
        // Absolute-form with no path at all never matches a non-empty path.
        assert!(!digest_uri_matches("http://host", "/a/b"));
    }

    #[test]
    fn bearer_wrong_token_is_unauthorized() {
        let v = Verifier::new(Credentials::bearer("right-token"), REALM);
        let header = respond(
            &v.challenge(),
            &RequestContext::new("GET", "/stream"),
            Credentials::bearer("wrong-token"),
        )
        .unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn missing_authorization_header_is_unauthorized() {
        let v = Verifier::new(Credentials::bearer("tok"), REALM);
        assert_eq!(
            verify_auth(&v, None, "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn wrong_scheme_header_is_unauthorized() {
        // A Basic-configured verifier must reject a Bearer-shaped header
        // (and vice versa) rather than mis-parsing it as a match.
        let v = Verifier::new(
            Credentials::Basic {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        assert_eq!(
            verify_auth(&v, Some("Bearer sometoken"), "GET", "/stream"),
            AuthResult::Unauthorized
        );
    }

    // --- Forwarded (reverse-proxy forwarded-auth, issue #663 extensibility
    // wave part 1) ---

    #[test]
    fn forwarded_challenge_is_bare_scheme_name() {
        let v = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        assert_eq!(v.challenge(), "Forwarded");
    }

    /// Biting test: a request carrying the configured user header (non-empty)
    /// must verify `Ok` — this is the whole trust mechanism, no secret is
    /// ever compared.
    #[test]
    fn forwarded_with_user_header_present_is_ok() {
        let v = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        let headers: &[(&str, &str)] = &[("X-Forwarded-User", "alice")];
        let ctx = RequestContext::new("GET", "/stream").with_headers(headers);
        assert_eq!(v.verify(&ctx), AuthResult::Ok);
    }

    /// Biting test: a request with no user header at all must `Unauthorized`
    /// — the whole point of the scheme is that only a trusted proxy having
    /// authenticated the caller sets it.
    #[test]
    fn forwarded_without_user_header_is_unauthorized() {
        let v = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        let ctx = RequestContext::new("GET", "/stream");
        assert_eq!(v.verify(&ctx), AuthResult::Unauthorized);
    }

    /// An empty (but present) user header must not count as authenticated —
    /// otherwise a proxy bug forwarding an empty header would silently grant
    /// access.
    #[test]
    fn forwarded_with_empty_user_header_is_unauthorized() {
        let v = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        let headers: &[(&str, &str)] = &[("X-Forwarded-User", "")];
        let ctx = RequestContext::new("GET", "/stream").with_headers(headers);
        assert_eq!(v.verify(&ctx), AuthResult::Unauthorized);
    }

    /// The user-header lookup is case-insensitive, matching real HTTP header
    /// semantics (RFC 7230 §3.2) rather than a literal-string match.
    #[test]
    fn forwarded_user_header_lookup_is_case_insensitive() {
        let v = Verifier::forwarded("X-Forwarded-User", None);
        let headers: &[(&str, &str)] = &[("x-forwarded-user", "alice")];
        let ctx = RequestContext::new("GET", "/stream").with_headers(headers);
        assert_eq!(v.verify(&ctx), AuthResult::Ok);
    }

    /// Biting test: `forwarded_for` reads the configured header's value back
    /// out of the request context — the mechanism the origin middleware uses
    /// to surface the proxy-forwarded client IP to tracing.
    #[test]
    fn forwarded_for_reads_configured_header() {
        let v = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        let headers: &[(&str, &str)] = &[
            ("X-Forwarded-User", "alice"),
            ("X-Forwarded-For", "203.0.113.7"),
        ];
        let ctx = RequestContext::new("GET", "/stream").with_headers(headers);
        assert_eq!(v.forwarded_for(&ctx), Some("203.0.113.7"));
    }

    /// With no `forwarded_for_header` configured, `forwarded_for` is always
    /// `None`, even if an `X-Forwarded-For` header happens to be present.
    #[test]
    fn forwarded_for_is_none_when_not_configured() {
        let v = Verifier::forwarded("X-Forwarded-User", None);
        let headers: &[(&str, &str)] = &[("X-Forwarded-For", "203.0.113.7")];
        let ctx = RequestContext::new("GET", "/stream").with_headers(headers);
        assert_eq!(v.forwarded_for(&ctx), None);
    }

    /// `forwarded_for` is always `None` for a non-`Forwarded` verifier, even
    /// if the request happens to carry an `X-Forwarded-For` header.
    #[test]
    fn forwarded_for_is_none_for_non_forwarded_verifier() {
        let v = Verifier::new(Credentials::bearer("tok"), REALM);
        let headers: &[(&str, &str)] = &[("X-Forwarded-For", "203.0.113.7")];
        let ctx = RequestContext::new("GET", "/stream").with_headers(headers);
        assert_eq!(v.forwarded_for(&ctx), None);
    }

    /// Debug must never need to redact anything for `Forwarded` (no secret is
    /// involved), but must still not panic and must name the scheme.
    #[test]
    fn forwarded_debug_names_scheme() {
        let v = Verifier::forwarded("X-Forwarded-User", Some("X-Forwarded-For".to_string()));
        let debug = format!("{v:?}");
        assert!(debug.contains("Forwarded"), "debug: {debug}");
    }

    // --- SignedUrl (issue #747) — the scheme's own biting tests (sign/
    // verify round trip, cross-route replay, expiry, key rotation, IP
    // scoping, malformed input) live in `crate::signed_url`'s test module;
    // these just cover `Verifier`'s own surface (`challenge`/`Debug`).

    #[test]
    fn signed_url_challenge_is_bare_scheme_name() {
        let keys = SignedUrlKeySet::new([("k".to_string(), vec![0u8; 32])]).unwrap();
        let v = Verifier::signed_url(keys);
        assert_eq!(v.challenge(), "SignedUrl");
    }

    #[test]
    fn signed_url_debug_names_scheme_and_never_leaks_secret() {
        let secret = b"super-secret-32-byte-hmac-key!!!".to_vec();
        let keys = SignedUrlKeySet::new([("k".to_string(), secret.clone())]).unwrap();
        let v = Verifier::signed_url(keys);
        let debug = format!("{v:?}");
        assert!(debug.contains("SignedUrl"), "debug: {debug}");
        assert!(
            !debug.contains(std::str::from_utf8(&secret).unwrap()),
            "debug: {debug}"
        );
    }

    #[test]
    fn constant_time_eq_matches_naive_equality() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer-string"));
        assert!(constant_time_eq(b"", b""));
    }

    // Regression: an oversized Digest `Authorization` header (way more
    // `key=value` fields than any real client sends) must be rejected
    // outright rather than parsed into an unbounded `HashMap` — and must
    // never panic. Must FAIL if the `MAX_DIGEST_FIELDS` cap in
    // `verify_digest` is ever removed.
    #[test]
    fn oversized_digest_header_is_rejected_not_parsed() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let mut huge = String::from("Digest ");
        for i in 0..(MAX_DIGEST_FIELDS + 1) {
            if i > 0 {
                huge.push(',');
            }
            huge.push_str(&format!("k{i}=\"v{i}\""));
        }
        assert_eq!(
            verify_auth(&v, Some(&huge), "DESCRIBE", "rtsp://cam/live"),
            AuthResult::Unauthorized,
            "oversized Digest header must not be accepted"
        );
    }

    fn digest_verifier_at(now: std::sync::Arc<std::sync::atomic::AtomicU64>) -> Verifier {
        Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        )
        .with_clock(move || {
            UNIX_EPOCH + Duration::from_secs(now.load(std::sync::atomic::Ordering::SeqCst))
        })
    }

    fn digest_header(challenge: &str, uri: &str) -> String {
        respond(
            challenge,
            &RequestContext::new("GET", uri),
            Credentials::new("admin", "12345"),
        )
        .unwrap()
    }

    /// Rewrites the `nc` and `response` fields of a Digest header as a client
    /// would for its `nc`-th request under the same nonce + cnonce.
    fn with_nc(header: &str, nc: u32, uri: &str) -> String {
        let field = |name: &str| {
            let start = header.find(&format!("{name}=")).unwrap() + name.len() + 1;
            let rest = &header[start..];
            let rest = rest.trim_start_matches('"');
            let end = rest.find(['"', ',']).unwrap_or(rest.len());
            rest[..end].to_string()
        };
        let (nonce, cnonce, old_nc, old_resp) = (
            field("nonce"),
            field("cnonce"),
            field("nc"),
            field("response"),
        );
        let nc = format!("{nc:08x}");
        let ha1 = md5_hex(format!("admin:{REALM}:12345"));
        let ha2 = md5_hex(format!("GET:{uri}"));
        let resp = md5_hex(format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
        header
            .replace(&format!("nc={old_nc}"), &format!("nc={nc}"))
            .replace(&old_resp, &resp)
    }

    #[test]
    fn digest_increasing_nc_is_accepted_and_repeated_nc_rejected() {
        let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000_000));
        let v = digest_verifier_at(now);
        let uri = "/stream/index.m3u8";
        let first = digest_header(&v.challenge(), uri);
        assert_eq!(verify_auth(&v, Some(&first), "GET", uri), AuthResult::Ok);
        let second = with_nc(&first, 2, uri);
        assert_eq!(verify_auth(&v, Some(&second), "GET", uri), AuthResult::Ok);
        let third = with_nc(&first, 3, uri);
        assert_eq!(verify_auth(&v, Some(&third), "GET", uri), AuthResult::Ok);
        assert_eq!(
            verify_auth(&v, Some(&second), "GET", uri),
            AuthResult::Unauthorized,
            "nc lower than the highest seen must be rejected"
        );
    }

    #[test]
    fn digest_expired_nonce_is_rejected_with_stale_challenge() {
        let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000_000));
        let v = digest_verifier_at(now.clone());
        let uri = "/stream/index.m3u8";
        let header = digest_header(&v.challenge(), uri);
        now.fetch_add(
            DIGEST_NONCE_LIFETIME.as_secs(),
            std::sync::atomic::Ordering::SeqCst,
        );
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", uri),
            AuthResult::Unauthorized
        );
        let headers = [("authorization", header.as_str())];
        let ctx = RequestContext::new("GET", uri).with_headers(&headers);
        let challenge = v.challenge_for(&ctx);
        assert!(challenge.ends_with(", stale=true"), "got: {challenge}");
        // The fresh nonce from that challenge verifies.
        let retry = digest_header(&challenge, uri);
        assert_eq!(verify_auth(&v, Some(&retry), "GET", uri), AuthResult::Ok);
        // A wrong password on an expired nonce is not stale.
        let wrong = respond(
            &v.challenge(),
            &RequestContext::new("GET", uri),
            Credentials::new("admin", "WRONG"),
        )
        .unwrap();
        now.fetch_add(
            DIGEST_NONCE_LIFETIME.as_secs(),
            std::sync::atomic::Ordering::SeqCst,
        );
        let headers = [("authorization", wrong.as_str())];
        let ctx = RequestContext::new("GET", uri).with_headers(&headers);
        assert!(!v.challenge_for(&ctx).contains("stale"));
    }

    #[test]
    fn digest_nonce_with_wrong_mac_is_rejected() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let uri = "/stream/index.m3u8";
        let challenge = v.challenge();
        let start = challenge.find("nonce=\"").unwrap() + "nonce=\"".len();
        let nonce = &challenge[start..start + NONCE_LEN * 2];
        // Flip the last hex digit of the MAC, keeping the timestamp + salt.
        let last = nonce.as_bytes()[nonce.len() - 1];
        let flipped = if last == b'0' { '1' } else { '0' };
        let forged_nonce = format!("{}{flipped}", &nonce[..nonce.len() - 1]);
        let forged = challenge.replace(nonce, &forged_nonce);
        let header = digest_header(&forged, uri);
        assert!(header.contains(&forged_nonce));
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", uri),
            AuthResult::Unauthorized
        );
        // A nonce from a different verifier (different secret) is rejected too.
        let other = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let header = digest_header(&other.challenge(), uri);
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", uri),
            AuthResult::Unauthorized
        );
    }

    #[test]
    fn digest_nc_table_stays_bounded_and_evicted_pairs_are_stale() {
        let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1_000_000));
        let v = digest_verifier_at(now.clone());
        let uri = "/s";
        let first = digest_header(&v.challenge(), uri);
        assert_eq!(verify_auth(&v, Some(&first), "GET", uri), AuthResult::Ok);
        now.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        for _ in 0..DIGEST_NC_TRACK_CAP {
            let h = digest_header(&v.challenge(), uri);
            assert_eq!(verify_auth(&v, Some(&h), "GET", uri), AuthResult::Ok);
        }
        let VerifierScheme::Digest { nonces, .. } = &v.scheme else {
            unreachable!()
        };
        assert_eq!(
            nonces.seen.lock().unwrap().entries.len(),
            DIGEST_NC_TRACK_CAP
        );
        // The oldest pair was evicted: its replay must not verify, and the
        // next challenge tells the client to retry with a fresh nonce.
        let replay = with_nc(&first, 2, uri);
        assert_eq!(
            verify_auth(&v, Some(&replay), "GET", uri),
            AuthResult::Unauthorized
        );
        let headers = [("authorization", replay.as_str())];
        let ctx = RequestContext::new("GET", uri).with_headers(&headers);
        assert!(v.challenge_for(&ctx).ends_with(", stale=true"));
    }

    #[test]
    fn digest_replayed_request_is_rejected() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "12345".into(),
            },
            REALM,
        );
        let ctx = RequestContext::new("GET", "/stream/index.m3u8");
        let header = respond(&v.challenge(), &ctx, Credentials::new("admin", "12345")).unwrap();
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream/index.m3u8"),
            AuthResult::Ok
        );
        assert_eq!(
            verify_auth(&v, Some(&header), "GET", "/stream/index.m3u8"),
            AuthResult::Unauthorized,
            "same nonce + nc + cnonce must not verify twice"
        );
    }

    #[test]
    fn debug_never_leaks_password_or_token() {
        let v = Verifier::new(
            Credentials::Digest {
                username: "admin".into(),
                password: "supersecret".into(),
            },
            REALM,
        );
        let debug = format!("{v:?}");
        assert!(!debug.contains("supersecret"), "debug: {debug}");

        let v = Verifier::new(Credentials::bearer("topsecrettoken"), REALM);
        let debug = format!("{v:?}");
        assert!(!debug.contains("topsecrettoken"), "debug: {debug}");
    }
}
