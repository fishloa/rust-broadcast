//! multimux configuration: routes + segmentation/window/bind parameters.
//!
//! CLI-first with an optional JSON config file. A route maps one input
//! ([`InputSpec`] — RTSP pull, raw RTP/UDP, MPEG-TS/UDP, MPEG-TS/HTTP,
//! HLS-pull, or RTMP push) to a served stream name.

use crate::dvr::DvrConfig;
use crate::error::{MultimuxError, Result};
use crate::output::OutputKind;
use broadcast_auth::Credentials;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

/// Default [`Route::outputs`] when a route's config omits the field: LL-HLS
/// only, preserving pre-#663-P4 behaviour for every existing config.
fn default_outputs() -> Vec<OutputKind> {
    vec![OutputKind::LlHls]
}

/// Default [`InputSpec::File::loop_file`] when a config omits the field:
/// restart from the beginning at EOF (`true`), matching `default_outputs`.
fn default_file_loop() -> bool {
    true
}

/// One route's ingest transport (issue #663 P3a/P3c): tagged so a JSON
/// config can name which transport a route uses (`"type": "rtsp" | "rtp" |
/// "ts_udp" | "ts_http" | "hls_pull"`).
///
/// - [`InputSpec::Rtsp`] pulls a live RTSP source (DESCRIBE/SETUP/PLAY,
///   interleaved TCP) — see [`crate::source::rtsp`].
/// - [`InputSpec::Rtp`] receives raw RTP over UDP (uni/multicast), depayloaded
///   using an out-of-band SDP (inline text, or `@path` to a file) that
///   supplies the codec/fmtp a DESCRIBE would otherwise provide — see
///   [`crate::source::rtp_udp`].
/// - [`InputSpec::TsUdp`] receives an MPEG-2 Transport Stream over UDP
///   (uni/multicast); the track set comes from the stream's own in-band PMT,
///   so no SDP is needed — see [`crate::source::ts_udp`].
/// - [`InputSpec::TsHttp`] receives an MPEG-2 Transport Stream over a
///   streaming HTTP GET (chunked/progressive) — see
///   [`crate::source::ts_http`].
/// - [`InputSpec::HlsPull`] pulls a remote (LL-)HLS Media Playlist — see
///   [`crate::source::hls_pull`].
/// - [`InputSpec::DashPull`] pulls a remote MPEG-DASH presentation (issue
///   #758) — see [`crate::source::dash_pull`].
/// - [`InputSpec::SmoothPull`] pulls a remote Microsoft Smooth Streaming
///   (MS-SSTR) presentation (issue #759) — see
///   [`crate::source::smooth_pull`]. PlayReady/PIFF sample-encrypted sources
///   are rejected with [`crate::MultimuxError::Encrypted`] rather than
///   silently emitting garbage samples — see that module's doc for the
///   detection heuristic.
/// - [`InputSpec::Rtmp`] accepts an inbound RTMP push publisher (a *push*
///   input — see [`crate::source::rtmp`].
/// - [`InputSpec::Srt`] receives an SRT-carried MPEG-2 Transport Stream, in
///   either listener mode (a *push* input, exactly like [`InputSpec::Rtmp`])
///   or caller mode (dials out, like every other variant) — see
///   [`crate::source::srt`]. Encrypted SRT is out of scope: no passphrase
///   field is exposed.
///
/// [`InputSpec::TsHttp`]/[`InputSpec::HlsPull`]/[`InputSpec::DashPull`] all
/// may carry `user:pass@` URL userinfo (Basic/Digest — see
/// [`crate::source::http_auth`]), redacted the same way [`InputSpec::Rtsp`]'s
/// URL is.
///
/// [`InputSpec::Rtsp`]/[`InputSpec::TsHttp`]/[`InputSpec::HlsPull`]/
/// [`InputSpec::DashPull`] each also take an optional config-supplied `auth`
/// ([`AuthSpec`]) — the only way to supply a Bearer token (RFC 6750 has no
/// URL-userinfo form) and, when present, taking precedence over any URL
/// userinfo (see `crate::source::http_auth::resolve_credentials`).
/// [`InputSpec::Rtp`]/[`InputSpec::TsUdp`] are raw UDP transports with no
/// HTTP/RTSP request line to attach credentials to, so they carry no `auth`
/// field.
///
/// - [`InputSpec::Custom`] (issue #663 external scheme plugin registry) names
///   an external input scheme by an opaque `type_tag`, resolved at
///   `crate::origin::serve_with_registry` time via
///   [`crate::registry::SchemeRegistry::input`] — the escape hatch that lets
///   a third-party crate add a new ingest transport without editing this
///   crate. `params` is passed through unexamined to the registered factory.
#[non_exhaustive]
#[derive(Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputSpec {
    /// Pull a live RTSP source.
    Rtsp {
        /// RTSP source URL to pull. May carry `user:pass@` userinfo — see
        /// [`InputSpec`]'s `Debug` impl, which redacts it.
        url: String,
        /// Config-supplied credentials, overriding any URL userinfo. See
        /// [`AuthSpec`].
        #[serde(default)]
        auth: Option<AuthSpec>,
    },
    /// Receive raw RTP over UDP (uni/multicast), depayloaded per an
    /// out-of-band SDP.
    Rtp {
        /// `host:port` to bind the UDP socket to.
        addr: String,
        /// The SDP describing the stream's codec/fmtp: either inline SDP
        /// text, or `@path` to a file containing one (read fresh on every
        /// connect/reconnect).
        sdp: String,
        /// Multicast group to join, if the stream is multicast rather than
        /// unicast (must be a multicast IP address of `addr`'s family).
        #[serde(default)]
        multicast_group: Option<String>,
    },
    /// Receive an MPEG-2 Transport Stream over UDP (uni/multicast).
    TsUdp {
        /// `host:port` to bind the UDP socket to.
        addr: String,
        /// Multicast group to join, if the stream is multicast rather than
        /// unicast (must be a multicast IP address of `addr`'s family).
        #[serde(default)]
        multicast_group: Option<String>,
    },
    /// Receive an MPEG-2 Transport Stream over a streaming HTTP GET
    /// (chunked/progressive).
    TsHttp {
        /// `http://` or `https://` URL to GET. May carry `user:pass@`
        /// userinfo — see [`InputSpec`]'s `Debug` impl, which redacts it.
        url: String,
        /// Config-supplied credentials, overriding any URL userinfo. See
        /// [`AuthSpec`].
        #[serde(default)]
        auth: Option<AuthSpec>,
    },
    /// Pull a remote (LL-)HLS Media Playlist.
    HlsPull {
        /// `http://` or `https://` Media Playlist URL to pull. May carry
        /// `user:pass@` userinfo — see [`InputSpec`]'s `Debug` impl, which
        /// redacts it.
        url: String,
        /// Config-supplied credentials, overriding any URL userinfo. See
        /// [`AuthSpec`].
        #[serde(default)]
        auth: Option<AuthSpec>,
    },
    /// Pull a remote MPEG-DASH presentation (issue #758) — see
    /// [`crate::source::dash_pull`].
    DashPull {
        /// `http://` or `https://` MPD URL to pull. May carry `user:pass@`
        /// userinfo — see [`InputSpec`]'s `Debug` impl, which redacts it.
        url: String,
        /// Config-supplied credentials, overriding any URL userinfo. See
        /// [`AuthSpec`].
        #[serde(default)]
        auth: Option<AuthSpec>,
    },
    /// Pull a remote Microsoft Smooth Streaming (MS-SSTR) client Manifest
    /// (issue #759) — see [`crate::source::smooth_pull`].
    SmoothPull {
        /// `http://` or `https://` client Manifest URL to pull. May carry
        /// `user:pass@` userinfo — see [`InputSpec`]'s `Debug` impl, which
        /// redacts it.
        url: String,
        /// Config-supplied credentials, overriding any URL userinfo. See
        /// [`AuthSpec`].
        #[serde(default)]
        auth: Option<AuthSpec>,
    },
    /// Accept an inbound RTMP push publisher (issue #738).
    ///
    /// `stream_key` is this route's only ingest authentication: leaving it
    /// `None` means **anyone who can reach `listen` may publish** (and, since
    /// a route only ever serves its first live publisher — see
    /// `crate::route::RouteHandle::publish_program` — the first connection to
    /// publish wins the route). A startup log line warns when a route is
    /// wired up this way; set `stream_key` for any listener reachable outside
    /// a trusted network.
    Rtmp {
        /// `host:port` to bind the RTMP listen socket to (e.g.
        /// `"0.0.0.0:1935"`, the IANA-assigned RTMP port).
        listen: String,
        /// If set, the publisher's `connect` `app` name must match exactly
        /// or the route's `connect()` fails.
        #[serde(default)]
        app: Option<String>,
        /// If set, the publisher's `publish` stream key must match exactly
        /// (enforced by the RTMP session itself — a mismatch never reaches
        /// this crate as a `Publish`/`Media` event, and the session is closed
        /// after a few mismatched attempts — see `rtmp_runtime::server`).
        /// `None` means this ingest listener requires no authentication at
        /// all — see this variant's own doc.
        #[serde(default)]
        stream_key: Option<String>,
    },
    /// Accept an inbound WHIP (RFC 9725) publisher (issue #740): an HTTP
    /// `POST`ed SDP offer answered over ICE + DTLS-SRTP — see
    /// [`crate::source::whip`]. **Video (H.264) only** in this cut: a WHIP
    /// publisher's audio is essentially always Opus, and this workspace has
    /// no RTP/Opus depacketiser yet (see that module's doc). Only compiled
    /// in behind this crate's own `whip` Cargo feature, which (unlike this
    /// crate's default build) needs rustc >= 1.88 — see `Cargo.toml`'s
    /// `whip` feature doc.
    ///
    /// This cut exposes **no ingest authentication at all**: any `POST` to
    /// `listen` may publish (only the first, per-route — see
    /// `crate::route::RouteHandle::publish_program`). A startup log line
    /// warns every time this input kind is wired up, unconditionally; only
    /// run it behind a trusted network or a fronting proxy that adds auth
    /// until this crate grows a WHIP Bearer-token config knob.
    #[cfg(feature = "whip")]
    Whip {
        /// `host:port` to bind the WHIP publish HTTP endpoint to (e.g.
        /// `"0.0.0.0:8080"`). Any request path is accepted — this is a
        /// single-route listener, not a multi-tenant path router.
        listen: String,
    },
    /// Receive an SRT-carried MPEG-2 Transport Stream (issue #739), in
    /// either listener mode (`listen` set — binds and accepts inbound
    /// Callers, a push input) or caller mode (`remote` set — dials out);
    /// exactly one of `listen`/`remote` must be set, enforced at config
    /// validation time. The track set comes from the stream's own in-band
    /// PMT, exactly like [`InputSpec::TsUdp`].
    ///
    /// Encrypted SRT (`draft-sharabayko-srt-01` §6) is **out of scope**:
    /// [`srt_runtime::io`] does not yet apply the SEK to decrypt DATA
    /// payloads, so no passphrase field is exposed here — see
    /// [`crate::source::srt`]'s module doc. In listener mode this means
    /// **any Caller may publish** (only the first, per-route); a startup log
    /// line warns every time a listener-mode SRT route is wired up.
    Srt {
        /// Listener bind address (e.g. `"0.0.0.0:9000"`) — mutually
        /// exclusive with `remote`.
        #[serde(default)]
        listen: Option<String>,
        /// Caller dial-out address (e.g. `"remote-host:9000"`) — mutually
        /// exclusive with `listen`.
        #[serde(default)]
        remote: Option<String>,
        /// Stream ID to advertise (caller mode only — `draft-sharabayko-srt-01`
        /// §3.2.1.3).
        #[serde(default)]
        stream_id: Option<String>,
        /// Overrides the negotiated TSBPD latency (milliseconds); `None`
        /// keeps the handshake's default.
        #[serde(default)]
        latency_ms: Option<u16>,
    },
    /// External input scheme resolved at runtime via
    /// [`crate::registry::SchemeRegistry`]. `type_tag` selects the registered
    /// factory; `params` is passed opaquely to it. JSON:
    /// `{ "type": "custom", "type_tag": "webrtc", "params": { ... } }`.
    Custom {
        /// Selects the registered factory in
        /// [`crate::registry::SchemeRegistry`] that builds this input.
        type_tag: String,
        /// Opaque config passed to the registered factory verbatim — may
        /// carry external-scheme credentials, so it is always redacted (as
        /// `"<params>"`) in `Debug`, never rendered.
        #[serde(default)]
        params: serde_json::Value,
    },
    /// Play a media file from local disk as a source.
    File {
        /// Path to the media file.
        path: String,
        /// Restart from the beginning at EOF. Defaults to `true` — a slate or
        /// filler asset is normally expected to run until the schedule moves
        /// on.
        #[serde(default = "default_file_loop", rename = "loop")]
        loop_file: bool,
    },
}

/// Config-supplied credentials for an [`InputSpec::Rtsp`]/
/// [`InputSpec::TsHttp`]/[`InputSpec::HlsPull`] route (client-side
/// multi-scheme auth, issue #663): either a username/password — answered as
/// Basic or Digest, whichever the server's own `WWW-Authenticate` challenge
/// asks for (RFC 7617/RFC 7616) — or a bearer token (RFC 6750). A bearer
/// token has no URL-userinfo form, so config is its only source; a
/// username/password pair may instead come from the route's own URL
/// userinfo, but an explicit `auth` here always wins over that (see
/// `crate::source::http_auth::resolve_credentials`).
///
/// JSON shape is untagged — either
/// `{ "username": "...", "password": "..." }` or
/// `{ "bearer_token": "..." }`.
#[non_exhaustive]
#[derive(Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AuthSpec {
    /// Username/password, answered as Basic or Digest per the server's
    /// challenge.
    Password {
        /// Account username.
        username: String,
        /// Account password.
        password: String,
    },
    /// A bearer token (RFC 6750), sent verbatim as `Authorization: Bearer
    /// <token>` with no challenge round-trip.
    Bearer {
        /// The opaque bearer token.
        bearer_token: String,
    },
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): both variants carry a
/// secret (`password`/`bearer_token`) that must never render verbatim.
impl std::fmt::Debug for AuthSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthSpec::Password { username, .. } => f
                .debug_struct("Password")
                .field("username", username)
                .field("password", &"***")
                .finish(),
            AuthSpec::Bearer { .. } => f
                .debug_struct("Bearer")
                .field("bearer_token", &"***")
                .finish(),
        }
    }
}

impl AuthSpec {
    /// Converts to the scheme-agnostic [`Credentials`] the RTSP source /
    /// HTTP sources actually authenticate with.
    ///
    /// Called from the per-route ingest wiring in
    /// `crate::origin::serve_with_registry` for every input kind that carries
    /// an `AuthSpec` (RTSP, TS-HTTP, HLS/DASH/Smooth pull) — see the
    /// `with_auth(auth.as_ref().map(AuthSpec::to_credentials))` call sites in
    /// `src/origin/mod.rs`.
    pub(crate) fn to_credentials(&self) -> Credentials {
        match self {
            AuthSpec::Password { username, password } => {
                Credentials::new(username.clone(), password.clone())
            }
            AuthSpec::Bearer { bearer_token } => Credentials::bearer(bearer_token.clone()),
        }
    }
}

/// Container format for a push output.
/// Only [`PushFormat::Ts`] is implemented — `crate::push::drive_push` builds
/// a `transmux::TsMux` unconditionally. `Mp4`/`Mkv` are reserved config
/// surface for a later phase; selecting either is rejected at config-validate
/// time (`Route::validate_standalone`, issue #744/M1c) rather than silently
/// downgraded to TS.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushFormat {
    /// MPEG-2 Transport Stream. The only variant currently implemented.
    Ts,
    /// Fragmented MP4 / CMAF. Not yet implemented — rejected at config
    /// validation time.
    Mp4,
    /// Matroska / WebM. Not yet implemented — rejected at config validation
    /// time.
    Mkv,
}

impl PushFormat {
    pub fn name(&self) -> &'static str {
        match self {
            PushFormat::Ts => "ts",
            PushFormat::Mp4 => "mp4",
            PushFormat::Mkv => "mkv",
        }
    }
}

broadcast_common::impl_spec_display!(PushFormat);

/// Reconnect policy for push outputs.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ReconnectPolicy {
    #[serde(default = "ReconnectPolicy::default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "ReconnectPolicy::default_max_backoff_ms")]
    pub max_backoff_ms: u64,
    #[serde(default)]
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_ms: 1_000,
            max_backoff_ms: 30_000,
            max_attempts: None,
        }
    }
}

/// A push output's reconnect backoff doubles on every attempt (1 s, 2 s, 4 s,
/// … up to [`ReconnectPolicy::max_backoff_ms`]).
const RECONNECT_BACKOFF_FACTOR: f64 = 2.0;

impl ReconnectPolicy {
    fn default_initial_backoff_ms() -> u64 {
        1_000
    }
    fn default_max_backoff_ms() -> u64 {
        30_000
    }

    pub fn backoff_for(&self, attempt: u32) -> std::time::Duration {
        crate::origin::supervisor::Backoff::new(
            std::time::Duration::from_millis(self.initial_backoff_ms),
            std::time::Duration::from_millis(self.max_backoff_ms),
            RECONNECT_BACKOFF_FACTOR,
        )
        .delay_for_attempt(attempt)
    }

    /// Validate the policy (audit run 7, W19): a zero backoff (either bound)
    /// means a reconnect with no delay — a storm against the destination.
    pub fn validate(&self) -> Result<()> {
        if self.initial_backoff_ms == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs[].reconnect.initial_backoff_ms",
                reason: "must be greater than 0 (a zero initial backoff reconnects \
                         with no delay)"
                    .into(),
            });
        }
        if self.max_backoff_ms == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs[].reconnect.max_backoff_ms",
                reason: "must be greater than 0 (a zero max backoff reconnects with \
                         no delay)"
                    .into(),
            });
        }
        if self.initial_backoff_ms > self.max_backoff_ms {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs[].reconnect.initial_backoff_ms",
                reason: "must not exceed max_backoff_ms".into(),
            });
        }
        const MAX_BACKOFF_MS: u64 = 86_400_000; // 24 h
        if self.initial_backoff_ms > MAX_BACKOFF_MS || self.max_backoff_ms > MAX_BACKOFF_MS {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs[].reconnect.max_backoff_ms",
                reason: "backoff must not exceed 86_400_000 ms (24 h)".into(),
            });
        }
        Ok(())
    }
}

/// Server-side output auth (issue #663 "shared output auth"): configures one
/// [`broadcast_auth::Verifier`] gating **every** media output route
/// (`/{stream}/…` — manifests and init/segment/part bytes alike, across
/// every configured route) — independent of, and unrelated to, any given
/// route's own ingest [`AuthSpec`]/URL-userinfo credentials. `None` (the
/// default) leaves every output route open, unchanged from pre-#663
/// behaviour. Ops endpoints (`/healthz`/`/readyz`/`/metrics`) are never
/// gated by this — see `crate::origin::router`'s docs.
///
/// Unlike [`AuthSpec`] (which lets the *server*'s own challenge pick Basic vs
/// Digest), this is the *server* issuing the challenge, so the scheme itself
/// must be explicit — the `scheme` tag selects which
/// [`broadcast_auth::Credentials`] variant `to_credentials` builds.
///
/// JSON shape is tagged on `scheme`: `{ "scheme": "basic", "username": "...",
/// "password": "..." }`, `{ "scheme": "digest", "username": "...", "password":
/// "..." }`, `{ "scheme": "bearer", "token": "..." }`, or
/// `{ "scheme": "forwarded", "user_header": "...", "forwarded_for_header":
/// "..." }` (see [`OutputAuthSpec::Forwarded`]).
#[non_exhaustive]
#[derive(Clone, Deserialize)]
#[serde(tag = "scheme", rename_all = "snake_case")]
pub enum OutputAuthSpec {
    /// HTTP Basic (RFC 7617) — credentials compared in constant time.
    Basic {
        /// Account username.
        username: String,
        /// Account password.
        password: String,
    },
    /// HTTP Digest (RFC 7616) — a fresh, time-limited nonce is issued on
    /// every challenge and an expired one is answered `stale=true` so a
    /// compliant client retries without re-prompting (see
    /// `broadcast_auth::Verifier`'s "Nonce handling" module doc, and
    /// `crate::origin::output_auth_gate`/`crate::origin::admin::admin_auth_gate`,
    /// which challenge via `Verifier::challenge_for`, not `Verifier::challenge`,
    /// specifically so an expired nonce gets `stale=true`).
    Digest {
        /// Account username.
        username: String,
        /// Account password.
        password: String,
    },
    /// Bearer (RFC 6750) — token compared in constant time.
    Bearer {
        /// The opaque bearer token.
        token: String,
    },
    /// Reverse-proxy forwarded-auth (issue #663 extensibility wave part 1,
    /// `broadcast_auth::Verifier::forwarded`): trusts that a fronting
    /// reverse proxy has already authenticated the caller and forwards the
    /// authenticated username in `user_header`. Authenticated iff that
    /// header is present and non-empty; unlike Basic/Digest/Bearer there is
    /// no credential configured here at all and no `WWW-Authenticate`
    /// challenge/response round-trip a direct client could answer.
    ///
    /// # Trust assumption
    ///
    /// **Safe ONLY behind a trusted reverse proxy that strips any
    /// client-supplied copies of `user_header` (and `forwarded_for_header`,
    /// if set) before forwarding.** multimux performs no such stripping and
    /// trusts every inbound header completely — if the origin is reachable
    /// directly (not exclusively through the proxy), any client can set
    /// these headers itself and bypass authentication entirely.
    Forwarded {
        /// Header naming the proxy-authenticated username. Defaults to
        /// `"X-Forwarded-User"` when omitted.
        #[serde(default = "default_forwarded_user_header")]
        user_header: String,
        /// Header the proxy uses to forward the original client's address,
        /// read back for observability (tracing) only — never used for any
        /// trust decision. Defaults to `Some("X-Forwarded-For")`; set to
        /// `null` to disable reading it at all.
        #[serde(default = "default_forwarded_for_header")]
        forwarded_for_header: Option<String>,
    },
    /// HMAC signed-URL (issue #747, `broadcast_auth::Verifier::signed_url`):
    /// a CDN-style, short-lived, tamper-proof token carried in the request's
    /// own query string (`?exp=...&kid=...&sig=...[&ip=...]`) — no
    /// `Authorization` header at all, so a player can fetch segments/parts
    /// without carrying a credential. `keys` is the full set of currently
    /// valid `(kid, secret)` pairs; listing more than one lets keys rotate
    /// without invalidating URLs already handed out under an older
    /// (still-listed) key. See `broadcast_auth::signed_url` for the wire
    /// form and canonical string a token is minted against.
    ///
    /// JSON: `{ "scheme": "signed_url", "keys": [{ "kid": "...", "secret":
    /// "..." }, ...] }`.
    SignedUrl {
        /// The currently valid signing keys. Each `secret` must be at least
        /// `broadcast_auth::SignedUrlKeySet::MIN_SECRET_LEN` (32) bytes —
        /// checked at config `validate()` time, not per-request.
        keys: Vec<SignedUrlKeySpec>,
    },
    /// External output-auth scheme resolved at runtime via
    /// [`crate::registry::SchemeRegistry`] (issue #663 external scheme
    /// plugin registry) — the escape hatch that lets a third-party crate add
    /// a new server-side output-auth scheme without editing this crate.
    /// `type_tag` selects the registered factory; `params` is passed
    /// opaquely to it. JSON: `{ "scheme": "custom", "type_tag": "hmac",
    /// "params": { ... } }`.
    Custom {
        /// Selects the registered factory in
        /// [`crate::registry::SchemeRegistry`] that builds this
        /// `broadcast_auth::Verifier`.
        type_tag: String,
        /// Opaque config passed to the registered factory verbatim — may
        /// carry external-scheme credentials, so it is always redacted (as
        /// `"<params>"`) in `Debug`, never rendered.
        #[serde(default)]
        params: serde_json::Value,
    },
}

/// One HMAC signed-URL key (issue #747): a `kid` (key id, selects this entry
/// out of [`OutputAuthSpec::SignedUrl`]'s `keys`) and its `secret`, taken as
/// this string's raw UTF-8 bytes (the same convention `AuthSpec`/
/// `OutputAuthSpec`'s other secret fields use — a plain config string, not a
/// separately-encoded byte blob).
#[derive(Clone, Deserialize)]
pub struct SignedUrlKeySpec {
    /// The key id a token's `kid` query parameter selects.
    pub kid: String,
    /// The HMAC secret, as raw UTF-8 bytes — must be at least
    /// `broadcast_auth::SignedUrlKeySet::MIN_SECRET_LEN` (32) bytes.
    pub secret: String,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `secret` must never
/// render verbatim; `kid` is not secret and renders as-is.
impl std::fmt::Debug for SignedUrlKeySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedUrlKeySpec")
            .field("kid", &self.kid)
            .field("secret", &"***")
            .finish()
    }
}

/// [`OutputAuthSpec::Forwarded`]'s default `user_header` when the config
/// omits the field.
fn default_forwarded_user_header() -> String {
    "X-Forwarded-User".to_string()
}

/// [`OutputAuthSpec::Forwarded`]'s default `forwarded_for_header` when the
/// config omits the field (`null` explicitly disables it instead).
fn default_forwarded_for_header() -> Option<String> {
    Some("X-Forwarded-For".to_string())
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): the Basic/Digest/Bearer
/// variants each carry a secret (`password`/`token`) that must never render
/// verbatim; `Forwarded`'s header names aren't secret and render as-is.
impl std::fmt::Debug for OutputAuthSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputAuthSpec::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"***")
                .finish(),
            OutputAuthSpec::Digest { username, .. } => f
                .debug_struct("Digest")
                .field("username", username)
                .field("password", &"***")
                .finish(),
            OutputAuthSpec::Bearer { .. } => {
                f.debug_struct("Bearer").field("token", &"***").finish()
            }
            OutputAuthSpec::Forwarded {
                user_header,
                forwarded_for_header,
            } => f
                .debug_struct("Forwarded")
                .field("user_header", user_header)
                .field("forwarded_for_header", forwarded_for_header)
                .finish(),
            OutputAuthSpec::SignedUrl { keys } => f
                .debug_struct("SignedUrl")
                .field(
                    "kids",
                    &keys.iter().map(|k| k.kid.as_str()).collect::<Vec<_>>(),
                )
                .finish(),
            OutputAuthSpec::Custom { type_tag, .. } => f
                .debug_struct("Custom")
                .field("type_tag", type_tag)
                .field("params", &"<params>")
                .finish(),
        }
    }
}

impl OutputAuthSpec {
    /// Builds the [`broadcast_auth::Verifier`] this spec configures —
    /// Basic/Digest/Bearer via a [`Credentials`] + `realm` (the scheme is
    /// preserved exactly: unlike [`AuthSpec::to_credentials`], this is the
    /// server side, so it must issue the challenge for the scheme it was
    /// actually configured with, not whichever the client's challenge
    /// implies); `Forwarded` via [`broadcast_auth::Verifier::forwarded`]
    /// (no credential/challenge round-trip at all — see that variant's
    /// trust-assumption docs); `SignedUrl` via
    /// [`broadcast_auth::Verifier::signed_url`].
    ///
    /// `SignedUrl`'s `broadcast_auth::SignedUrlKeySet::new` call is
    /// infallible *here*: [`Self::validate`] already enforces every key's
    /// minimum secret length before this is ever called (see
    /// [`Config::validate`]'s call site, always before any `build_verifier`
    /// call) — the `.expect` below documents that invariant rather than
    /// re-deriving a `Result` this method's signature (matching every other
    /// builtin scheme) doesn't carry.
    pub(crate) fn build_verifier(&self, realm: &str) -> broadcast_auth::Verifier {
        match self {
            OutputAuthSpec::Basic { username, password } => broadcast_auth::Verifier::new(
                Credentials::Basic {
                    username: username.clone(),
                    password: password.clone(),
                },
                realm,
            ),
            OutputAuthSpec::Digest { username, password } => broadcast_auth::Verifier::new(
                Credentials::Digest {
                    username: username.clone(),
                    password: password.clone(),
                },
                realm,
            ),
            OutputAuthSpec::Bearer { token } => {
                broadcast_auth::Verifier::new(Credentials::bearer(token.clone()), realm)
            }
            OutputAuthSpec::Forwarded {
                user_header,
                forwarded_for_header,
            } => broadcast_auth::Verifier::forwarded(
                user_header.clone(),
                forwarded_for_header.clone(),
            ),
            OutputAuthSpec::SignedUrl { keys } => {
                let key_pairs = keys
                    .iter()
                    .map(|k| (k.kid.clone(), k.secret.clone().into_bytes()));
                let keyset = broadcast_auth::SignedUrlKeySet::new(key_pairs).expect(
                    "OutputAuthSpec::validate already enforces the minimum signed-url secret \
                     length before build_verifier is ever called",
                );
                broadcast_auth::Verifier::signed_url(keyset)
            }
            OutputAuthSpec::Custom { .. } => unreachable!(
                "OutputAuthSpec::Custom cannot build a Verifier without a SchemeRegistry — \
                 crate::origin::serve_with_registry resolves it via `registry.auth(type_tag)` \
                 before this method is ever called on a Custom variant"
            ),
        }
    }

    /// Rejects an empty `username`/`token`/`user_header` (an empty `password`
    /// is left unvalidated, mirroring [`validate_auth`]); an explicitly-set
    /// but empty `forwarded_for_header` is also rejected (use `null` to
    /// disable it instead of an empty string). `SignedUrl` additionally
    /// rejects an empty `keys` list, an empty `kid`, and any `secret` shorter
    /// than `broadcast_auth::SignedUrlKeySet::MIN_SECRET_LEN` — checked here,
    /// at config-load time, so [`Self::build_verifier`] never has to.
    fn validate(&self) -> Result<()> {
        match self {
            OutputAuthSpec::Basic { username, .. } | OutputAuthSpec::Digest { username, .. }
                if username.is_empty() =>
            {
                Err(MultimuxError::ConfigInvalid {
                    field: "output_auth.username",
                    reason: "must not be empty".into(),
                })
            }
            OutputAuthSpec::Bearer { token } if token.is_empty() => {
                Err(MultimuxError::ConfigInvalid {
                    field: "output_auth.token",
                    reason: "must not be empty".into(),
                })
            }
            OutputAuthSpec::Forwarded { user_header, .. } if user_header.is_empty() => {
                Err(MultimuxError::ConfigInvalid {
                    field: "output_auth.user_header",
                    reason: "must not be empty".into(),
                })
            }
            OutputAuthSpec::Forwarded {
                forwarded_for_header: Some(header),
                ..
            } if header.is_empty() => Err(MultimuxError::ConfigInvalid {
                field: "output_auth.forwarded_for_header",
                reason: "must not be empty (use null to disable)".into(),
            }),
            OutputAuthSpec::SignedUrl { keys } if keys.is_empty() => {
                Err(MultimuxError::ConfigInvalid {
                    field: "output_auth.keys",
                    reason: "must not be empty".into(),
                })
            }
            OutputAuthSpec::SignedUrl { keys } => {
                for key in keys {
                    if key.kid.is_empty() {
                        return Err(MultimuxError::ConfigInvalid {
                            field: "output_auth.keys[].kid",
                            reason: "must not be empty".into(),
                        });
                    }
                    if key.secret.len() < broadcast_auth::SignedUrlKeySet::MIN_SECRET_LEN {
                        return Err(MultimuxError::ConfigInvalid {
                            field: "output_auth.keys[].secret",
                            reason: format!(
                                "must be at least {} bytes, got {}",
                                broadcast_auth::SignedUrlKeySet::MIN_SECRET_LEN,
                                key.secret.len()
                            ),
                        });
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Runtime admin API configuration (issue #749) — opt-in: omit this field
/// entirely (the default, `None`) and no admin listener is ever bound, no
/// admin route ever exists. See [`crate::origin::admin`]'s module doc for
/// the full design (add/remove/list routes + reload without restarting the
/// origin) and this crate's README for the security posture.
///
/// # Two hard rules, both enforced structurally
///
/// - **Separate listener.** [`Self::bind`] must differ from
///   [`Config::bind`] (the media listener) — [`Config::validate`] rejects a
///   config where they're equal. The admin API must never be reachable on
///   the public media port.
/// - **Mandatory auth.** [`Self::auth`] is a plain [`OutputAuthSpec`], not
///   `Option<OutputAuthSpec>` — a config that enables the admin API without
///   naming a scheme fails to *deserialize* (a missing required field),
///   before the process ever binds a socket. There is no way to start the
///   admin listener unauthenticated.
#[derive(Debug, Clone, Deserialize)]
pub struct AdminSpec {
    /// `host:port` the admin HTTP API binds. Must differ from
    /// [`Config::bind`] — see this struct's own docs.
    pub bind: String,
    /// Mandatory auth gating every admin request. Reuses [`OutputAuthSpec`]
    /// (the same scheme set that gates media output routes) rather than a
    /// parallel type, since the shape (Basic/Digest/Bearer/Forwarded/Custom)
    /// is identical; this is a *separate* [`broadcast_auth::Verifier`]
    /// instance from [`Config::output_auth`], so the admin credential can
    /// (and should) differ from whatever gates media playback.
    pub auth: OutputAuthSpec,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): [`InputSpec::Rtsp`]'s
/// `url` may carry a live camera's `user:pass@` userinfo, so it must never
/// render verbatim; the UDP variants carry no secret but get a tidy summary
/// (the SDP body's length rather than its full text, which can be sizeable).
impl std::fmt::Debug for InputSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputSpec::Rtsp { url, auth } => f
                .debug_struct("Rtsp")
                .field("url", &crate::redact::redact_url(url))
                .field("auth", auth)
                .finish(),
            InputSpec::Rtp {
                addr,
                sdp,
                multicast_group,
            } => f
                .debug_struct("Rtp")
                .field("addr", addr)
                .field("sdp_len", &sdp.len())
                .field("multicast_group", multicast_group)
                .finish(),
            InputSpec::TsUdp {
                addr,
                multicast_group,
            } => f
                .debug_struct("TsUdp")
                .field("addr", addr)
                .field("multicast_group", multicast_group)
                .finish(),
            InputSpec::TsHttp { url, auth } => f
                .debug_struct("TsHttp")
                .field("url", &crate::redact::redact_url(url))
                .field("auth", auth)
                .finish(),
            InputSpec::HlsPull { url, auth } => f
                .debug_struct("HlsPull")
                .field("url", &crate::redact::redact_url(url))
                .field("auth", auth)
                .finish(),
            InputSpec::DashPull { url, auth } => f
                .debug_struct("DashPull")
                .field("url", &crate::redact::redact_url(url))
                .field("auth", auth)
                .finish(),
            InputSpec::SmoothPull { url, auth } => f
                .debug_struct("SmoothPull")
                .field("url", &crate::redact::redact_url(url))
                .field("auth", auth)
                .finish(),
            InputSpec::Rtmp {
                listen,
                app,
                stream_key,
            } => f
                .debug_struct("Rtmp")
                .field("listen", listen)
                .field("app", app)
                .field("stream_key", &stream_key.as_ref().map(|_| "***"))
                .finish(),
            #[cfg(feature = "whip")]
            InputSpec::Whip { listen } => f.debug_struct("Whip").field("listen", listen).finish(),
            InputSpec::Srt {
                listen,
                remote,
                stream_id,
                latency_ms,
            } => f
                .debug_struct("Srt")
                .field("listen", listen)
                .field("remote", remote)
                .field("stream_id", stream_id)
                .field("latency_ms", latency_ms)
                .finish(),
            InputSpec::Custom { type_tag, .. } => f
                .debug_struct("Custom")
                .field("type_tag", type_tag)
                .field("params", &"<params>")
                .finish(),
            InputSpec::File { path, loop_file } => f
                .debug_struct("File")
                .field("path", path)
                .field("loop_file", loop_file)
                .finish(),
        }
    }
}

impl InputSpec {
    /// Validates this input's fields in isolation (no I/O — reachability is
    /// checked at connect time, not here): an RTSP URL must parse with an
    /// `rtsp`/`rtsps` scheme; a UDP `addr` must parse as a socket address; a
    /// `multicast_group`, if present, must be a multicast IP; an [`Rtp`]
    /// input's `sdp` must be non-empty, and — unless it's an `@path`
    /// reference (existence checked at connect time) — parseable SDP.
    ///
    /// [`Rtp`]: InputSpec::Rtp
    fn validate(&self) -> Result<()> {
        match self {
            InputSpec::Rtsp { url, auth } => {
                validate_rtsp_url(url)?;
                validate_auth(auth)
            }
            InputSpec::Rtp {
                addr,
                sdp,
                multicast_group,
            } => {
                validate_udp_addr(addr)?;
                validate_sdp(sdp)?;
                if let Some(group) = multicast_group {
                    validate_multicast_group(group)?;
                }
                Ok(())
            }
            InputSpec::TsUdp {
                addr,
                multicast_group,
            } => {
                validate_udp_addr(addr)?;
                if let Some(group) = multicast_group {
                    validate_multicast_group(group)?;
                }
                Ok(())
            }
            InputSpec::TsHttp { url, auth } => {
                validate_http_url(url)?;
                validate_auth(auth)
            }
            InputSpec::HlsPull { url, auth } => {
                validate_http_url(url)?;
                validate_auth(auth)
            }
            InputSpec::DashPull { url, auth } => {
                validate_http_url(url)?;
                validate_auth(auth)
            }
            InputSpec::SmoothPull { url, auth } => {
                validate_http_url(url)?;
                validate_auth(auth)
            }
            InputSpec::Rtmp { listen, .. } => validate_listen_addr(listen),
            #[cfg(feature = "whip")]
            InputSpec::Whip { listen } => validate_listen_addr(listen),
            InputSpec::Srt { listen, remote, .. } => match (listen, remote) {
                (Some(_), Some(_)) => Err(MultimuxError::ConfigInvalid {
                    field: "routes.input.listen",
                    reason: "exactly one of listen/remote must be set, got both".into(),
                }),
                (None, None) => Err(MultimuxError::ConfigInvalid {
                    field: "routes.input.listen",
                    reason: "exactly one of listen/remote must be set, got neither".into(),
                }),
                (Some(listen), None) => validate_listen_addr(listen),
                (None, Some(remote)) => validate_host_port(remote),
            },
            // Always structurally valid: the registered factory (resolved at
            // `crate::origin::serve_with_registry` time, not here) validates
            // `params` itself.
            InputSpec::Custom { .. } => Ok(()),
            // Path existence/reachability is checked at connect time, not
            // here; only the empty path is rejected at validate time (an
            // empty path is always invalid and would otherwise spin the
            // supervisor's retry forever).
            InputSpec::File { path, .. } => {
                if path.trim().is_empty() {
                    Err(MultimuxError::ConfigInvalid {
                        field: "routes.input.path",
                        reason: "file input path must not be empty".into(),
                    })
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// An RTSP URL must parse and use the `rtsp`/`rtsps` scheme (RFC 2326 §1 /
/// IANA).
fn validate_rtsp_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| MultimuxError::ConfigInvalid {
        field: "routes.input.url",
        reason: format!("bad rtsp(s) URL {url:?}: {e}"),
    })?;
    match parsed.scheme() {
        "rtsp" | "rtsps" => Ok(()),
        other => Err(MultimuxError::ConfigInvalid {
            field: "routes.input.url",
            reason: format!("scheme must be rtsp or rtsps, got {other:?}"),
        }),
    }
}

/// A TS-over-HTTP/HLS-pull URL must parse and use the `http`/`https` scheme.
fn validate_http_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| MultimuxError::ConfigInvalid {
        field: "routes.input.url",
        reason: format!("bad http(s) URL {url:?}: {e}"),
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(MultimuxError::ConfigInvalid {
            field: "routes.input.url",
            reason: format!("scheme must be http or https, got {other:?}"),
        }),
    }
}

/// An `rtsp_push` output URL: must parse, use the `rtsp`/`rtsps` scheme, and
/// name a host (a cannot-be-a-base URL such as `rtsp:cam` parses but has no
/// authority, and the push transport cannot build a control URL or a dial
/// address from it). Returns a plain reason string so the caller can attach
/// the `routes.outputs[].url` field.
pub(crate) fn validate_rtsp_push_url(url: &str) -> std::result::Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("bad rtsp(s) push URL {url:?}: {e}"))?;
    match parsed.scheme() {
        "rtsp" | "rtsps" => {}
        other => return Err(format!("scheme must be rtsp or rtsps, got {other:?}")),
    }
    if parsed.host_str().is_none() {
        return Err(format!(
            "rtsp push URL {url:?} has no host (expected `rtsp://host[:port]/path`)"
        ));
    }
    Ok(())
}

/// A config-supplied [`AuthSpec`], if present, must not carry an empty
/// `username`/`bearer_token` (an empty `password` is left unvalidated — some
/// devices genuinely use a blank password). `None` (no config auth — the
/// route falls back to URL userinfo, if any) always passes.
fn validate_auth(auth: &Option<AuthSpec>) -> Result<()> {
    match auth {
        None => Ok(()),
        Some(AuthSpec::Password { username, .. }) if username.is_empty() => {
            Err(MultimuxError::ConfigInvalid {
                field: "routes.input.auth.username",
                reason: "must not be empty".into(),
            })
        }
        Some(AuthSpec::Bearer { bearer_token }) if bearer_token.is_empty() => {
            Err(MultimuxError::ConfigInvalid {
                field: "routes.input.auth.bearer_token",
                reason: "must not be empty".into(),
            })
        }
        Some(_) => Ok(()),
    }
}

/// [`Config::playlist_name`] must be non-empty, end in `.m3u8`, and contain
/// no path separator (it names a single path segment under `/{stream}/`, not
/// a sub-path) — issue #663 "configurable `playlist_name`".
fn validate_playlist_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(MultimuxError::ConfigInvalid {
            field: "playlist_name",
            reason: "must not be empty".into(),
        });
    }
    if !name.ends_with(".m3u8") {
        return Err(MultimuxError::ConfigInvalid {
            field: "playlist_name",
            reason: format!("must end in .m3u8, got {name:?}"),
        });
    }
    if name.contains('/') {
        return Err(MultimuxError::ConfigInvalid {
            field: "playlist_name",
            reason: format!("must not contain a slash, got {name:?}"),
        });
    }
    // `LlHlsOutput::manifest_routes` mounts `master.m3u8` and `playlist_name`
    // as two separate axum routes on the same per-stream router; the same
    // name for both would panic axum at router-build time (a route
    // conflict) rather than fail with a clean config error, so reject it
    // here instead.
    if name == "master.m3u8" {
        return Err(MultimuxError::ConfigInvalid {
            field: "playlist_name",
            reason: "must not be \"master.m3u8\" (that name is the master playlist route)".into(),
        });
    }
    Ok(())
}

/// A UDP bind address must parse as `host:port`.
fn validate_udp_addr(addr: &str) -> Result<()> {
    addr.parse::<SocketAddr>()
        .map(|_| ())
        .map_err(|e| MultimuxError::ConfigInvalid {
            field: "routes.input.addr",
            reason: format!("bad UDP address {addr:?}: {e}"),
        })
}

/// An RTMP `listen` address must parse as a socket address (same shape as
/// [`validate_udp_addr`], distinct field name for a clearer error message).
fn validate_listen_addr(addr: &str) -> Result<()> {
    addr.parse::<SocketAddr>()
        .map(|_| ())
        .map_err(|e| MultimuxError::ConfigInvalid {
            field: "routes.input.listen",
            reason: format!("bad listen address {addr:?}: {e}"),
        })
}

/// A caller-mode SRT `remote` may be a hostname (`SrtSocket::connect` resolves
/// it via `ToSocketAddrs`, doing its own DNS lookup — unlike a bind address,
/// which must always be a literal socket address), so it gets a looser
/// `host:port` shape check here rather than [`validate_listen_addr`]'s strict
/// [`SocketAddr`] parse: a non-empty host part and a numeric port after the
/// last `:`. See [`InputSpec::Srt`]'s `remote` doc (`"remote-host:9000"`) and
/// `crate::source::srt`'s module doc.
fn validate_host_port(addr: &str) -> Result<()> {
    let field = "routes.input.remote";
    let invalid = |reason: String| MultimuxError::ConfigInvalid { field, reason };
    // Validate the `host:port` shape through the `url` crate (a throwaway
    // `srt://` prefix makes `addr` the authority), so a hostname, a bracketed
    // IPv6 literal, and the 0..=65535 port range are all checked by the
    // parser rather than by a hand `rsplit_once(':')`. `SrtSocket::connect`
    // resolves a hostname via `ToSocketAddrs`, so a non-IP host is allowed —
    // this is a shape check, not a bind-address parse.
    //
    // `url` is LOOSER than the old `rsplit_once(':')` + `u16` parse: it accepts
    // a path/query/fragment after the port (`host:9000/path`), a userinfo
    // prefix (`user@host:9000`), and a trailing `/`. Reject all of those
    // explicitly so the accepted set matches the old strict check (a bare
    // `host:port`), and no silently-dropped tail reaches `SrtSocket::connect`.
    let Ok(url) = url::Url::parse(&format!("srt://{addr}")) else {
        return Err(invalid(format!(
            "bad host:port {addr:?}: not a valid host[:port]"
        )));
    };
    if url.host().is_none() {
        return Err(invalid(format!("bad host:port {addr:?}: empty host")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(format!(
            "bad host:port {addr:?}: must be a bare host:port (no userinfo)"
        )));
    }
    // `Url::path()` is "" (or "/" for a special scheme) for an authority-only
    // URL; anything longer is a path the old `rsplit_once(':')`+`u16` parse
    // would have rejected. `addr` is also checked literally, since `url`
    // normalises some tails away (a trailing `/`).
    if !matches!(url.path(), "" | "/") || addr.contains(['/', '?', '#']) {
        return Err(invalid(format!(
            "bad host:port {addr:?}: must be a bare host:port (no path/query/fragment)"
        )));
    }
    match url.port() {
        Some(_) => Ok(()),
        None => Err(invalid(format!(
            "bad host:port {addr:?}: missing \":port\""
        ))),
    }
}

/// A multicast group must parse as an IP address and actually be multicast
/// (RFC 1112 §4 IPv4 224.0.0.0/4; RFC 4291 §2.7 IPv6 `ff00::/8`) — a unicast
/// address here would silently fail (or worse, do nothing useful) at the
/// OS-level `IP_ADD_MEMBERSHIP`/`IPV6_JOIN_GROUP` join, so it's rejected at
/// config time instead.
fn validate_multicast_group(group: &str) -> Result<()> {
    let ip: IpAddr = group.parse().map_err(|e| MultimuxError::ConfigInvalid {
        field: "routes.input.multicast_group",
        reason: format!("bad multicast group {group:?}: {e}"),
    })?;
    if !ip.is_multicast() {
        return Err(MultimuxError::ConfigInvalid {
            field: "routes.input.multicast_group",
            reason: format!("{group} is not a multicast address"),
        });
    }
    Ok(())
}

/// An [`InputSpec::Rtp`] SDP must be non-empty; an inline body (not an
/// `@path` file reference) must also parse as SDP (RFC 4566) — this is the
/// full codec/fmtp source for that route, so a config with unparseable SDP
/// would never usefully connect. A `@path` reference is only checked for
/// non-emptiness of the path itself: the file may not exist yet at
/// config-validation time (mirrors how an RTSP URL's reachability is never
/// checked here either), and is read + parsed fresh at connect time by
/// [`crate::source::sdp::load_sdp`]/`parse_sdp_tracks`.
fn validate_sdp(sdp: &str) -> Result<()> {
    if sdp.is_empty() {
        return Err(MultimuxError::ConfigInvalid {
            field: "routes.input.sdp",
            reason: "must not be empty".into(),
        });
    }
    let Some(path) = sdp.strip_prefix('@') else {
        return sdp_types::Session::parse(sdp.as_bytes())
            .map(|_| ())
            .map_err(|e| MultimuxError::ConfigInvalid {
                field: "routes.input.sdp",
                reason: format!("unparsable inline SDP: {e}"),
            });
    };
    if path.is_empty() {
        return Err(MultimuxError::ConfigInvalid {
            field: "routes.input.sdp",
            reason: "@ file reference must name a path".into(),
        });
    }
    Ok(())
}

/// One input→output route: an [`InputSpec`] served under `name`, packaged to
/// every [`OutputKind`] in [`Self::outputs`] (issue #663 P4 — "ingest-once,
/// many-outputs": ` outputs` is **per-route** rather than one global
/// default, since different routes plausibly want different output sets,
/// e.g. a DASH-only route feeding an existing DASH-only player fleet
/// alongside an LL-HLS+DASH route for a browser audience — a single
/// process-wide default couldn't express that).
#[derive(Clone, Deserialize, PartialEq)]
pub struct Route {
    /// Served stream name (URL path segment).
    pub name: String,
    /// The ingest transport this route pulls from.
    pub input: InputSpec,
    /// Which delivery protocol(s) to package this route's ingested media as.
    /// Defaults to LL-HLS only (`default_outputs`), preserving every
    /// existing config's behaviour unchanged. Validated non-empty by
    /// [`Config::validate`].
    #[serde(default = "default_outputs")]
    pub outputs: Vec<OutputKind>,
    /// DVR durable segment archive (issue #746). Off by default — set
    /// `"enabled": true` and configure an `archive_root` and at least one
    /// retention limit to enable recording.
    #[serde(default)]
    pub dvr: DvrConfig,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): [`InputSpec`] already
/// redacts what needs redacting; this just forwards to it so `Route` values
/// embedded in `Config`'s (derived) `Debug` and ad-hoc `{:?}` logging never
/// leak a credential either.
impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("name", &self.name)
            .field("input", &self.input)
            .field("outputs", &self.outputs)
            .finish()
    }
}

/// Maximum accepted [`Route`] name length, in bytes. Long enough for any real
/// camera/route identifier, short enough that a name can never approach a
/// filesystem's `NAME_MAX` or a URL segment's practical bound.
pub const MAX_ROUTE_NAME_LEN: usize = 255;

/// Windows reserved device names (case-insensitive), matched with or without
/// an extension — `CON`, `con.txt`, `COM1`, etc. (item 7).
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Whether `name` is (or starts with, before the first `.`) a Windows
/// reserved device name — case-insensitive.
fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    WINDOWS_RESERVED_NAMES.contains(&stem.as_str())
}

impl Route {
    /// Semantic validation for one route in isolation — no other route's
    /// name is visible here, so the duplicate-name check stays in
    /// [`Config::validate`]'s own loop. Reused by that loop (so every startup
    /// route is checked exactly the same way) and by the runtime admin API
    /// (`crate::origin::admin`, issue #749) for a `POST /admin/routes` body
    /// and every route a `POST /admin/reload` would add or restart —
    /// validated before any of them touch the live registry.
    pub(crate) fn validate_standalone(&self, playlist_name: &str) -> Result<()> {
        if self.name.is_empty() {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.name",
                reason: "must not be empty".into(),
            });
        }
        // The name becomes a URL path segment (`/{name}/…`, `origin::router`'s
        // `nest`) AND a filesystem path component (`archive_root/{name}`, the
        // DVR/catch-up archive dir) — so it must be a single, safe path
        // segment, not merely "any string without a slash". Without this, a
        // name of `".."` (or `"a/../.."`, `.`-routed) walks out of
        // `archive_root` and the DVR writes `pN.*` wherever the traversal
        // lands (audit run 7, W5). `*` in a name also panics axum's
        // nest-wildcard handling at build time. A NUL byte (which a JSON
        // string can carry as ` `) truncates an OS path, so it is
        // rejected by the same charset check.
        if self.name.len() > MAX_ROUTE_NAME_LEN {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.name",
                reason: format!(
                    "must be at most {MAX_ROUTE_NAME_LEN} bytes, got {}",
                    self.name.len()
                ),
            });
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.name",
                reason: format!(
                    "must contain only ASCII letters, digits, '.', '_' or '-' (it is both a \
                     URL path segment and an on-disk directory name), got {:?}",
                    self.name
                ),
            });
        }
        if self.name == "." || self.name == ".." {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.name",
                reason: format!(
                    "{:?} is a path-traversal component, not a route name (the DVR/catch-up \
                     archive dir would escape archive_root)",
                    self.name
                ),
            });
        }
        // A trailing '.' or ' ' is dropped by Windows and confuses some
        // shells/tools; reject it portably rather than only on Windows
        // (item 7).
        if self.name.ends_with('.') || self.name.ends_with(' ') {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.name",
                reason: format!("must not end with '.' or a space, got {:?}", self.name),
            });
        }
        // Windows reserved device names (case-insensitive, with or without an
        // extension) are not usable as a directory component on Windows and
        // must not become an archive directory (item 7).
        if is_windows_reserved_name(&self.name) {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.name",
                reason: format!(
                    "{:?} is a reserved device name on Windows (CON, PRN, AUX, NUL, COM1-9, LPT1-9) and cannot be a route/archive directory name",
                    self.name
                ),
            });
        }
        if self.outputs.is_empty() {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs",
                reason: format!("route {:?} has no outputs configured", self.name),
            });
        }
        // Issue #887: `ts_hls` is mutually exclusive with `llhls`/`dash`/
        // `ll_dash` on the same route. Container (fMP4 vs. classic TS) is a
        // per-*route* property (`crate::route::RouteHandle::with_container`),
        // not per-output, because a `media_plane::Trunk` has exactly ONE
        // segment ring per program — a program's samples are segmented into
        // fMP4 *or* TS, never both, without a second ring. Serving both
        // containers from one ingest is a legitimate future want, but it
        // needs a `Trunk` change (a second, container-keyed ring group) and
        // belongs in its own issue; today the workaround is to run two routes
        // against the same source, one per container. See
        // `crate::output`/`crate::route`'s own module docs for the same
        // constraint from the serving side.
        if self.outputs.iter().any(|k| matches!(k, OutputKind::TsHls))
            && self.outputs.iter().any(|k| {
                matches!(
                    k,
                    OutputKind::LlHls | OutputKind::Dash | OutputKind::LlDash | OutputKind::Smooth
                )
            })
        {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs",
                reason: format!(
                    "route {:?} configures both \"ts_hls\" and an fMP4-based output \
                     (\"llhls\"/\"dash\"/\"ll_dash\"/\"smooth\") — a route's container (fMP4 vs. classic \
                     TS) is one property shared by every output on it, since a Trunk has only \
                     one segment ring per program; run two routes against the same source \
                     instead, one per container",
                    self.name
                ),
            });
        }
        // Issue #900: `"catchup"` serves the route's DVR archive — without
        // DVR enabled there is no archive to read, so reject the
        // combination here rather than mounting an output that would 404
        // every request.
        if self
            .outputs
            .iter()
            .any(|k| matches!(k, OutputKind::Catchup))
            && !self.dvr.enabled
        {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.outputs",
                reason: format!(
                    "route {:?} configures the \"catchup\" output but has no DVR archive \
                     enabled (\"routes.dvr.enabled\": true) — there would be nothing to serve",
                    self.name
                ),
            });
        }
        // Issue #744/M1c: `PushFormat::Mp4`/`PushFormat::Mkv` are declared
        // config surface with no muxer behind them — `crate::push::drive_push`
        // only ever builds a `transmux::TsMux` and silently ignores the
        // configured format (`let _ = _format;`). Selecting either downgrades
        // an operator's "mp4"/"mkv" request to TS with no signal that
        // happened, so reject them here instead of at push-task spawn time,
        // where the mismatch would otherwise be invisible until packets hit
        // the wire in the wrong container.
        for kind in &self.outputs {
            let format = match kind {
                OutputKind::SrtPush { format, .. }
                | OutputKind::RtmpPush { format, .. }
                | OutputKind::RtspPush { format, .. } => *format,
                _ => None,
            };
            if let Some(bad @ (PushFormat::Mp4 | PushFormat::Mkv)) = format {
                return Err(MultimuxError::ConfigInvalid {
                    field: "routes.outputs[].format",
                    reason: format!(
                        "push output format {:?} is not implemented — only \"ts\" is \
                         currently supported for push outputs (srt_push/rtmp_push/rtsp_push)",
                        bad.name()
                    ),
                });
            }
        }
        // Audit run 7, W19: a zero backoff (either bound) reconnects with no
        // delay — a reconnect storm against the destination. Reject it here,
        // at config-validation time, rather than letting a push task spin.
        for kind in &self.outputs {
            let reconnect = match kind {
                OutputKind::SrtPush { reconnect, .. }
                | OutputKind::RtmpPush { reconnect, .. }
                | OutputKind::RtspPush { reconnect, .. } => reconnect.as_ref(),
                _ => None,
            };
            if let Some(policy) = reconnect {
                policy.validate()?;
            }
        }
        // Audit W17: an `srt_push` URL is parsed/validated at config time so a
        // bad host/port/query surfaces as a config error (also on admin
        // add/reload), not when the push task first dials.
        for kind in &self.outputs {
            if let OutputKind::SrtPush { url, .. } = kind {
                crate::push::validate_srt_url(url).map_err(|reason| {
                    MultimuxError::ConfigInvalid {
                        field: "routes.outputs[].url",
                        reason,
                    }
                })?;
            }
        }
        // An `rtsp_push` URL is parsed/validated at config time too, so a URL
        // with no authority (`rtsp:cam`, which `url` parses but which names no
        // host) surfaces as a config error, not a panic when the push task
        // first builds its control URL.
        for kind in &self.outputs {
            if let OutputKind::RtspPush { url, .. } = kind {
                validate_rtsp_push_url(url).map_err(|reason| {
                    MultimuxError::ConfigInvalid {
                        field: "routes.outputs[].url",
                        reason,
                    }
                })?;
            }
        }
        // Issue #743: `OutputKind::Whep`'s `listen` field needs the same
        // `host:port` validation `InputSpec::Whip`'s own `listen` gets below.
        #[cfg(feature = "whep")]
        for kind in &self.outputs {
            if let OutputKind::Whep { listen } = kind {
                validate_listen_addr(listen)?;
            }
        }
        // Two outputs that mount the SAME axum path panic `Router::merge`
        // with "Overlapping method route" at BUILD time (audit run 7, W5),
        // which for a startup route is a process panic and for
        // `POST /admin/routes` a poisoned registry (see `admin`). The check
        // is on the *paths* each output mounts, not on the output kind:
        // `"outputs": ["llhls","llhls"]` collides on `/media.m3u8`, but two
        // `srt_push` (or `rtmp_push`/`rtsp_push`/`custom`/`whep`) outputs
        // mount no path at all and are legitimate multi-destination
        // configurations — a `mem::discriminant`-based check wrongly
        // rejected those. `playlist_name` colliding with an output's own
        // path (`"catchup.m3u8"` on a route with a `catchup` output) is the
        // same collision and is caught here too (`manifest_paths` includes
        // it).
        let mut seen_paths = std::collections::HashSet::new();
        for path in self.manifest_paths(playlist_name) {
            if !seen_paths.insert(path.clone()) {
                return Err(MultimuxError::ConfigInvalid {
                    field: "routes.outputs",
                    reason: format!(
                        "route {:?} mounts {path:?} from more than one output (or from \
                         playlist_name); each path may be served by exactly one output",
                        self.name
                    ),
                });
            }
        }
        self.input.validate()
    }

    /// Every media path this route's outputs mount under `/{name}/`, plus the
    /// configurable `playlist_name` itself — the set `Config::validate` /
    /// `Route::validate_standalone` check for collisions so two outputs (or an
    /// output and the playlist name) can never mount the same axum path and
    /// panic the router build (audit run 7, W5).
    pub(crate) fn manifest_paths(&self, playlist_name: &str) -> Vec<String> {
        let mut paths = Vec::new();
        for kind in &self.outputs {
            match kind {
                OutputKind::LlHls | OutputKind::TsHls => {
                    paths.push("/master.m3u8".to_string());
                    paths.push(format!("/{playlist_name}"));
                }
                OutputKind::Dash => paths.push("/manifest.mpd".to_string()),
                OutputKind::LlDash => {
                    paths.push(format!(
                        "/{}",
                        crate::output::ll_dash::LL_DASH_MANIFEST_NAME
                    ));
                }
                OutputKind::Smooth => paths.push("/Manifest".to_string()),
                OutputKind::Catchup => paths.push("/catchup.m3u8".to_string()),
                // Push outputs and `Custom`/`Whep` mount no HTTP manifest
                // under the stream router (WHEP is its own listener; push is
                // outbound; a `Custom` output's own routes are its business,
                // not statically knowable here).
                _ => {}
            }
        }
        paths
    }

    /// Validate any DVR config on this route — separate so the admin API can
    /// call it independently.
    pub(crate) fn validate_dvr(&self) -> Result<()> {
        if let Err(reason) = self.dvr.validate() {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes.dvr",
                reason,
            });
        }
        Ok(())
    }
}

/// multimux runtime configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `host:port` the HTTP origin binds.
    pub bind: String,
    /// Target full-segment duration (seconds).
    pub target_duration_secs: f64,
    /// LL-HLS part target (milliseconds).
    pub part_target_ms: u32,
    /// Rolling window depth (full segments retained in RAM).
    pub window_segments: usize,
    /// Input→output routes.
    pub routes: Vec<Route>,
    /// Per-request HTTP timeout, in seconds (issue #663 P5, audit-concurrency
    /// #3) — see [`crate::origin::HttpLimits::request_timeout`]. Must exceed
    /// 5.0 (the LL-HLS blocking-reload cap,
    /// `output::llhls`/`origin::resource`'s `BLOCKING_RELOAD_TIMEOUT`) or a
    /// legitimate long-poll blocking request would be cut off by this layer
    /// before it ever gets the chance to resolve or fall back on its own —
    /// enforced by [`Config::validate`].
    pub request_timeout_secs: f64,
    /// Maximum number of requests serviced concurrently, across every route
    /// — see [`crate::origin::HttpLimits::max_concurrent_requests`].
    pub max_concurrent_requests: usize,
    /// Maximum accepted request body size, in bytes — see
    /// [`crate::origin::HttpLimits::max_request_body_bytes`].
    pub max_request_body_bytes: usize,
    /// How long a request may wait for a concurrency permit before the origin
    /// sheds it with `503 Service Unavailable` (and `Retry-After`) — see
    /// [`crate::origin::HttpLimits::queue_timeout`] and
    /// [`crate::origin::limit`]. Defaults to 5 s. Must be positive; a
    /// non-positive value is rejected by [`Config::validate`].
    #[serde(default = "default_concurrency_queue_timeout_secs")]
    pub concurrency_queue_timeout_secs: f64,
    /// Ingest connect-handshake timeout, in seconds, applied to every route's
    /// source (issue #663 P5, audit-ingest #3) — see
    /// [`crate::source::IngestTimeouts::connect`].
    pub ingest_connect_timeout_secs: f64,
    /// Ingest per-read timeout, in seconds, applied to every route's source
    /// — see [`crate::source::IngestTimeouts::read`].
    pub ingest_read_timeout_secs: f64,
    /// The media-playlist filename served at `/{stream}/{playlist_name}`
    /// (issue #663 "configurable `playlist_name`") — `master.m3u8`'s
    /// `#EXT-X-STREAM-INF` reference follows suit
    /// (`crate::output::llhls::LlHlsOutput::new`). Applies to whichever of
    /// [`OutputKind::LlHls`]/[`OutputKind::TsHls`] a route configures (issue
    /// #887: the two are mutually exclusive on one route, so this is never
    /// ambiguous). Defaults to
    /// [`crate::output::llhls::DEFAULT_PLAYLIST_NAME`] (`"media.m3u8"`),
    /// preserving every existing config's behaviour unchanged. `master.m3u8`
    /// itself is not configurable, and DASH's `manifest.mpd` is unaffected.
    /// Validated non-empty, `.m3u8`-suffixed, and slash-free by
    /// [`Config::validate`].
    #[serde(default = "default_playlist_name")]
    pub playlist_name: String,
    /// Server-side output auth (issue #663 "shared output auth") gating
    /// every media output route (`/{stream}/…`) across every configured
    /// route — see [`OutputAuthSpec`]. `None` (the default) leaves every
    /// output route open, unchanged from pre-#663 behaviour.
    #[serde(default)]
    pub output_auth: Option<OutputAuthSpec>,
    /// Runtime admin API (issue #749): add/remove/list routes and reload the
    /// config file without restarting — see [`AdminSpec`] and
    /// [`crate::origin::admin`]. `None` (the default): no admin listener, no
    /// admin routes, at all.
    #[serde(default)]
    pub admin: Option<AdminSpec>,
}

/// Default [`Config::playlist_name`] when a config omits the field:
/// [`crate::output::llhls::DEFAULT_PLAYLIST_NAME`], preserving every
/// pre-#663 config's `/media.m3u8` behaviour unchanged.
/// Default [`Config::concurrency_queue_timeout_secs`] — the limit's own
/// default, so the two cannot drift.
fn default_concurrency_queue_timeout_secs() -> f64 {
    crate::origin::DEFAULT_QUEUE_TIMEOUT.as_secs_f64()
}

fn default_playlist_name() -> String {
    crate::output::llhls::DEFAULT_PLAYLIST_NAME.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "0.0.0.0:8080".to_string(),
            target_duration_secs: 4.0,
            part_target_ms: 500,
            window_segments: 8,
            routes: Vec::new(),
            request_timeout_secs: crate::origin::DEFAULT_REQUEST_TIMEOUT.as_secs_f64(),
            max_concurrent_requests: crate::origin::DEFAULT_MAX_CONCURRENT_REQUESTS,
            max_request_body_bytes: crate::origin::DEFAULT_MAX_REQUEST_BODY_BYTES,
            concurrency_queue_timeout_secs: crate::origin::DEFAULT_QUEUE_TIMEOUT.as_secs_f64(),
            ingest_connect_timeout_secs: crate::source::DEFAULT_CONNECT_TIMEOUT.as_secs_f64(),
            ingest_read_timeout_secs: crate::source::DEFAULT_READ_TIMEOUT.as_secs_f64(),
            playlist_name: default_playlist_name(),
            output_auth: None,
            admin: None,
        }
    }
}

/// Lower bound [`Config::validate`] enforces on `request_timeout_secs`: the
/// LL-HLS engine's own blocking-reload cap (5 s —
/// `output::llhls`/`origin::resource`'s `BLOCKING_RELOAD_TIMEOUT`). The
/// global HTTP timeout must stay strictly above it, or the global layer
/// would cut off a legitimate long-poll blocking request before the LL-HLS
/// engine's own cap ever gets a chance to resolve it or fall back.
const MIN_REQUEST_TIMEOUT_SECS: f64 = 5.0;

/// Documented maximum for every timeout/backoff duration in seconds (24 h) —
/// anything larger is rejected at validation, and `Duration::try_from_secs_f64`
/// (which fails above the `Duration` range) is the backstop for a value that
/// slipped through unvalidated.
pub(crate) const MAX_TIMEOUT_SECS: f64 = 86_400.0;

/// Validate a seconds-valued timeout: finite, `> 0`, and `<= MAX_TIMEOUT_SECS`.
/// Rejects NaN/infinity/negative and an overflowing-but-finite value such as
/// `1e20` (which `Duration::from_secs_f64` would panic on) — audit W19.
fn validate_timeout_secs(field: &'static str, secs: f64) -> Result<()> {
    if !secs.is_finite() || secs <= 0.0 || secs > MAX_TIMEOUT_SECS {
        return Err(MultimuxError::ConfigInvalid {
            field,
            reason: format!(
                "must be a finite number of seconds in (0, {MAX_TIMEOUT_SECS}], got {secs}"
            ),
        });
    }
    Ok(())
}

impl Config {
    /// Load a JSON config file.
    pub fn from_json_file(path: &Path) -> Result<Config> {
        let bytes = std::fs::read(path).map_err(|source| MultimuxError::ConfigRead {
            path: path.to_path_buf(),
            source,
        })?;
        let cfg: Config =
            serde_json::from_slice(&bytes).map_err(|e| MultimuxError::ConfigParse {
                path: path.to_path_buf(),
                reason: e.to_string(),
            })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject empty route sets, duplicate stream names, nonsensical timing,
    /// and any route whose [`InputSpec`] fails its own field validation.
    pub fn validate(&self) -> Result<()> {
        if self.routes.is_empty() {
            return Err(MultimuxError::ConfigInvalid {
                field: "routes",
                reason: "no routes configured".into(),
            });
        }
        if !self.target_duration_secs.is_finite() || self.target_duration_secs <= 0.0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "target_duration_secs",
                reason: "must be a finite positive number".into(),
            });
        }
        if self.part_target_ms == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "part_target_ms",
                reason: "must be positive".into(),
            });
        }
        if self.window_segments == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "window_segments",
                reason: "must be positive".into(),
            });
        }
        if self.request_timeout_secs <= MIN_REQUEST_TIMEOUT_SECS {
            return Err(MultimuxError::ConfigInvalid {
                field: "request_timeout_secs",
                reason: format!(
                    "must exceed {MIN_REQUEST_TIMEOUT_SECS} (the LL-HLS blocking-reload cap), \
                     got {}",
                    self.request_timeout_secs
                ),
            });
        }
        if self.max_concurrent_requests == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "max_concurrent_requests",
                reason: "must be positive".into(),
            });
        }
        if self.max_request_body_bytes == 0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "max_request_body_bytes",
                reason: "must be positive".into(),
            });
        }
        validate_timeout_secs(
            "ingest_connect_timeout_secs",
            self.ingest_connect_timeout_secs,
        )?;
        validate_timeout_secs("ingest_read_timeout_secs", self.ingest_read_timeout_secs)?;
        if !self.concurrency_queue_timeout_secs.is_finite()
            || self.concurrency_queue_timeout_secs <= 0.0
        {
            return Err(MultimuxError::ConfigInvalid {
                field: "concurrency_queue_timeout_secs",
                reason: "must be a finite, positive number of seconds".into(),
            });
        }
        validate_playlist_name(&self.playlist_name)?;
        if let Some(output_auth) = &self.output_auth {
            output_auth.validate()?;
        }
        // Case-insensitive duplicate detection: `Cam1` and `cam1` share
        // `archive_root/<name>` on a case-insensitive filesystem (APFS's
        // default, NTFS), so their DVR archives would silently collide even
        // though the two names are distinct URL segments. Fold to lowercase
        // (Unicode-aware) for the collision key.
        let mut seen = std::collections::HashSet::new();
        for r in &self.routes {
            let key = r.name.to_lowercase();
            if !seen.insert(key) {
                return Err(MultimuxError::ConfigInvalid {
                    field: "routes",
                    reason: format!(
                        "duplicate stream name {:?} (names are compared case-insensitively \
                         — two names differing only in case would share one DVR archive \
                         directory)",
                        r.name
                    ),
                });
            }
            r.validate_standalone(&self.playlist_name)?;
            r.validate_dvr()?;
        }
        if let Some(admin) = &self.admin {
            if admin.bind.is_empty() {
                return Err(MultimuxError::ConfigInvalid {
                    field: "admin.bind",
                    reason: "must not be empty".into(),
                });
            }
            if admin.bind == self.bind {
                return Err(MultimuxError::ConfigInvalid {
                    field: "admin.bind",
                    reason: format!(
                        "must differ from bind (both {:?}) — the runtime admin API must never \
                         be reachable on the media listener",
                        admin.bind
                    ),
                });
            }
            admin.auth.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- W5 (audit run 7): route-name safety and router-build panics ---

    /// A route named `..` (or `.`) makes `<archive_root>/<name>` escape the
    /// archive root, so the DVR writes `pN.*` outside it. A name with a
    /// character outside `[A-Za-z0-9._-]` (notably `*`) panics axum's
    /// nest-wildcard handling at router build.
    ///
    /// Biting test: remove the charset/dot-segment check in
    /// `validate_standalone` and every case here validates `Ok`.
    #[test]
    fn route_name_traversal_and_exotic_chars_are_rejected() {
        for bad in ["..", ".", "a/b", "a*b", "a b", "a\u{00e9}b", ""] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{
                          "name": {bad_json},
                          "input": {{ "type": "rtsp", "url": "rtsp://host/s1" }},
                          "outputs": [ "llhls" ]
                        }}
                    ]
                }}"#,
                bad_json = serde_json::to_string(bad).unwrap()
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            let err = cfg
                .validate()
                .expect_err(&format!("route name {bad:?} must be rejected"));
            assert!(
                matches!(err, MultimuxError::ConfigInvalid { field, .. } if field == "routes.name"),
                "route name {bad:?} gave the wrong error: {err:?}"
            );
        }
    }

    /// Case-insensitive duplicate names are rejected: `Cam1` and `cam1`
    /// share one DVR archive directory on a case-insensitive filesystem.
    ///
    /// Biting test: switch `Config::validate`'s seen-set back to
    /// `r.name.as_str()` and this config validates `Ok`.
    #[test]
    fn case_insensitive_duplicate_route_names_are_rejected() {
        let json = r#"{
            "routes": [
                { "name": "Cam1",
                  "input": { "type": "rtsp", "url": "rtsp://host/a" },
                  "outputs": [ "llhls" ] },
                { "name": "cam1",
                  "input": { "type": "rtsp", "url": "rtsp://host/b" },
                  "outputs": [ "llhls" ] }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        let err = cfg
            .validate()
            .expect_err("names differing only in case must collide");
        assert!(
            matches!(err, MultimuxError::ConfigInvalid { field, .. } if field == "routes"),
            "got {err:?}"
        );
    }

    /// An over-long name, and names carrying a NUL or an encoded traversal
    /// component, are rejected as config errors (audit run 7, A4).
    ///
    /// Biting test: drop the length cap / charset check and these validate.
    #[test]
    fn over_long_and_nul_and_encoded_names_are_rejected() {
        let long = "a".repeat(MAX_ROUTE_NAME_LEN + 1);
        let nul = "cam x";
        for bad in [long.as_str(), nul, "cam%2e%2e", "cam "] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{ "name": {name},
                           "input": {{ "type": "rtsp", "url": "rtsp://host/a" }},
                           "outputs": [ "llhls" ] }}
                    ]
                }}"#,
                name = serde_json::to_string(bad).unwrap()
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            assert!(
                cfg.validate().is_err(),
                "route name {bad:?} must be rejected"
            );
        }
    }

    /// Item 7: Windows reserved device names (case-insensitive, with or
    /// without an extension) and a trailing '.'/' ' are rejected as route
    /// names.
    ///
    /// Biting test: remove the reserved-name / trailing checks and every case
    /// validates.
    #[test]
    fn windows_reserved_and_trailing_dot_names_are_rejected() {
        for bad in [
            "CON", "con", "Con", "con.txt", "PRN", "AUX", "NUL", "COM1", "com9.log", "LPT1",
            "lpt9", "cam.", "cam ", "CAM1.",
        ] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{ "name": {name},
                           "input": {{ "type": "rtsp", "url": "rtsp://host/a" }},
                           "outputs": [ "llhls" ] }}
                    ]
                }}"#,
                name = serde_json::to_string(bad).unwrap()
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            assert!(
                cfg.validate().is_err(),
                "route name {bad:?} must be rejected"
            );
        }
        // A name that merely CONTAINS a reserved token is fine.
        for good in ["contact", "console1", "cam.com1", "com"] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{ "name": "{good}",
                           "input": {{ "type": "rtsp", "url": "rtsp://host/a" }},
                           "outputs": [ "llhls" ] }}
                    ]
                }}"#
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            cfg.validate()
                .unwrap_or_else(|e| panic!("{good:?} must validate, got {e:?}"));
        }
    }

    /// Safe names still validate — the guard must not reject legitimate
    /// hostnames/IDs.
    #[test]
    fn ordinary_route_names_still_validate() {
        for good in ["cam1", "cam-1", "cam_1", "a.b.c", "CAM1"] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{
                          "name": "{good}",
                          "input": {{ "type": "rtsp", "url": "rtsp://host/s1" }},
                          "outputs": [ "llhls" ]
                        }}
                    ]
                }}"#
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            cfg.validate()
                .unwrap_or_else(|e| panic!("route name {good:?} must validate, got {e:?}"));
        }
    }

    /// `"outputs": ["llhls","llhls"]` mounts the same axum route twice, which
    /// panics the router build ("Overlapping method route").
    ///
    /// Biting test: remove the manifest-path collision check in
    /// `validate_standalone` and this config validates `Ok` (and would panic
    /// at router build).
    #[test]
    fn duplicate_output_kinds_are_rejected() {
        for outputs in [r#"["llhls","llhls"]"#, r#"["dash","dash"]"#] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{
                          "name": "cam1",
                          "input": {{ "type": "rtsp", "url": "rtsp://host/s1" }},
                          "outputs": {outputs}
                        }}
                    ]
                }}"#
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            let err = cfg
                .validate()
                .expect_err("a duplicated mounting output must be rejected");
            match err {
                MultimuxError::ConfigInvalid { field, reason } => {
                    assert_eq!(field, "routes.outputs");
                    assert!(
                        reason.contains("more than one output"),
                        "reason must explain the collision, got {reason:?}"
                    );
                }
                other => panic!("expected ConfigInvalid, got {other:?}"),
            }
        }
    }

    /// Multiple push (or `custom`/`whep`) outputs on ONE route are a
    /// legitimate multi-destination configuration — they mount no HTTP path
    /// at all, so there is nothing to collide. A `mem::discriminant`-based
    /// duplicate-kind check wrongly rejected them.
    ///
    /// Biting test: restore the discriminant check and every case here
    /// fails to validate.
    #[test]
    fn multiple_push_outputs_on_one_route_validate() {
        let json = r#"{
            "routes": [
                {
                  "name": "cam1",
                  "input": { "type": "rtsp", "url": "rtsp://host/s1" },
                  "outputs": [
                    { "srt_push": { "url": "srt://a:9000" } },
                    { "srt_push": { "url": "srt://b:9000" } },
                    { "rtmp_push": { "url": "rtmp://c/app/key" } },
                    { "rtsp_push": { "url": "rtsp://d/app/key" } }
                  ]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        cfg.validate()
            .expect("multiple push outputs on one route must be accepted");
        assert_eq!(cfg.routes[0].outputs.len(), 4);
    }

    /// The configurable `playlist_name` can collide with another output's own
    /// manifest path (`catchup.m3u8`, `manifest.mpd`, `Manifest`, …) — the
    /// pre-fix code only guarded against `master.m3u8`.
    ///
    /// Biting test: remove the manifest-path collision check in
    /// `validate_standalone` and this config validates `Ok` (and would panic
    /// at router build with two routes on `/cam1/catchup.m3u8`).
    #[test]
    fn playlist_name_colliding_with_another_output_is_rejected() {
        let json = r#"{
            "playlist_name": "catchup.m3u8",
            "routes": [
                {
                  "name": "cam1",
                  "input": { "type": "rtsp", "url": "rtsp://host/s1" },
                  "outputs": [ "llhls", "catchup" ],
                  "dvr": { "enabled": true, "archive_root": "/tmp/arch",
                           "retention_periods": 2 }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        let err = cfg
            .validate()
            .expect_err("playlist_name colliding with a catchup output must be rejected");
        match err {
            MultimuxError::ConfigInvalid { field, reason } => {
                assert_eq!(field, "routes.outputs");
                assert!(
                    reason.contains("catchup.m3u8"),
                    "reason must name the colliding path, got {reason:?}"
                );
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn parses_json_config_with_rtsp_routes() {
        let json = r#"{
            "bind": "127.0.0.1:9000",
            "target_duration_secs": 2.0,
            "part_target_ms": 250,
            "window_segments": 6,
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } },
                { "name": "cam2", "input": { "type": "rtsp", "url": "rtsp://host/stream2" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.bind, "127.0.0.1:9000");
        assert_eq!(cfg.part_target_ms, 250);
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[1].name, "cam2");
        match &cfg.routes[1].input {
            InputSpec::Rtsp { url, .. } => assert_eq!(url, "rtsp://host/stream2"),
            other => panic!("expected InputSpec::Rtsp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// A NaN / infinite / non-positive `target_duration_secs` must be rejected
    /// by `validate()`: `<= 0.0` alone lets NaN and +inf through, and the
    /// HLS origin builder then refused it at publish time (release audit).
    #[test]
    fn validate_rejects_non_finite_or_non_positive_target_duration() {
        let json = r#"{ "routes": [
            { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
        ] }"#;
        let base: Config = serde_json::from_str(json).unwrap();
        base.validate().unwrap();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            let mut cfg = base.clone();
            cfg.target_duration_secs = bad;
            match cfg.validate() {
                Err(MultimuxError::ConfigInvalid { field, .. }) => {
                    assert_eq!(field, "target_duration_secs", "{bad}");
                }
                other => panic!("{bad}: expected ConfigInvalid, got {other:?}"),
            }
        }
    }

    // --- push-output format validation (issue #744) ---

    /// `PushFormat::Mp4`/`Mkv` are config surface with no muxer behind them —
    /// `push::drive_push` only ever builds a `TsMux`. Selecting either used to
    /// silently downgrade to TS, so validation must reject them outright.
    ///
    /// Biting test: remove the check in `validate_standalone` and this fails,
    /// because the config is otherwise entirely valid.
    #[test]
    fn push_output_rejects_unimplemented_formats() {
        for bad in ["mp4", "mkv"] {
            for push in ["srt_push", "rtmp_push", "rtsp_push"] {
                let json = format!(
                    r#"{{
                        "routes": [
                            {{
                              "name": "cam1",
                              "input": {{ "type": "rtsp", "url": "rtsp://host/s1" }},
                              "outputs": [
                                {{ "{push}": {{ "url": "srt://host:9000", "format": "{bad}" }} }}
                              ]
                            }}
                        ]
                    }}"#
                );
                let cfg: Config = serde_json::from_str(&json).unwrap();
                let err = cfg
                    .validate()
                    .expect_err("mp4/mkv push format must be rejected");
                match err {
                    MultimuxError::ConfigInvalid { field, reason } => {
                        assert_eq!(field, "routes.outputs[].format");
                        assert!(
                            reason.to_lowercase().contains(bad),
                            "reason should name the rejected format {bad}, got: {reason}"
                        );
                    }
                    other => panic!("expected ConfigInvalid for {push}/{bad}, got {other:?}"),
                }
            }
        }
    }

    /// The supported format, and an omitted format, both still validate — the
    /// rejection above must not have broken the working path.
    #[test]
    fn push_output_accepts_ts_and_default_format() {
        for fmt in [r#", "format": "ts""#, ""] {
            let json = format!(
                r#"{{
                    "routes": [
                        {{
                          "name": "cam1",
                          "input": {{ "type": "rtsp", "url": "rtsp://host/s1" }},
                          "outputs": [
                            {{ "srt_push": {{ "url": "srt://host:9000"{fmt} }} }}
                          ]
                        }}
                    ]
                }}"#
            );
            let cfg: Config = serde_json::from_str(&json).unwrap();
            cfg.validate()
                .unwrap_or_else(|e| panic!("ts/default push format must validate, got: {e:?}"));
        }
    }

    // --- issue #663 P5: HTTP-layer resource limits (audit-concurrency #3) ---

    /// A config omitting the new limit fields gets the same defaults
    /// [`crate::origin::HttpLimits::default`] applies — every pre-P5 config
    /// keeps working unchanged.
    #[test]
    fn http_limits_default_when_omitted() {
        let json = r#"{
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(
            cfg.request_timeout_secs,
            crate::origin::DEFAULT_REQUEST_TIMEOUT.as_secs_f64()
        );
        assert_eq!(
            cfg.max_concurrent_requests,
            crate::origin::DEFAULT_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            cfg.max_request_body_bytes,
            crate::origin::DEFAULT_MAX_REQUEST_BODY_BYTES
        );
        cfg.validate().unwrap();
    }

    /// A `request_timeout_secs` at or below the LL-HLS blocking-reload cap
    /// (5 s) must be rejected — it would cut off a legitimate long-poll
    /// blocking request before that engine ever gets a chance to resolve or
    /// fall back.
    #[test]
    fn validate_rejects_request_timeout_at_or_below_blocking_cap() {
        for bad in [1.0, 5.0] {
            let cfg = Config {
                routes: vec![Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://a".into(),
                        auth: None,
                    },
                    outputs: default_outputs(),
                    dvr: DvrConfig::default(),
                }],
                request_timeout_secs: bad,
                ..Config::default()
            };
            assert!(cfg.validate().is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn validate_rejects_zero_max_concurrent_requests() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtsp {
                    url: "rtsp://a".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            max_concurrent_requests: 0,
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_max_request_body_bytes() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtsp {
                    url: "rtsp://a".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            max_request_body_bytes: 0,
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    /// The limit fields parse from JSON when given explicitly.
    #[test]
    fn parses_json_config_with_http_limits() {
        let json = r#"{
            "request_timeout_secs": 15.0,
            "max_concurrent_requests": 100,
            "max_request_body_bytes": 2048,
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.request_timeout_secs, 15.0);
        assert_eq!(cfg.max_concurrent_requests, 100);
        assert_eq!(cfg.max_request_body_bytes, 2048);
        cfg.validate().unwrap();
    }

    /// Audit W17: an `srt_push` URL is validated at config time — a bad host
    /// or unsupported query is a config error, not a silent dial failure.
    #[test]
    fn validate_rejects_bad_srt_push_url() {
        let mk = |url: &str| Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtsp {
                    url: "rtsp://a".into(),
                    auth: None,
                },
                outputs: vec![OutputKind::SrtPush {
                    url: url.to_string(),
                    format: None,
                    reconnect: None,
                }],
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(mk("srt://").validate().is_err(), "empty host");
        assert!(mk("srt://h:notaport").validate().is_err(), "bad port");
        assert!(
            mk("srt://h:9000?passphrase=x").validate().is_err(),
            "passphrase"
        );
        assert!(mk("srt://h:9000").validate().is_ok(), "a valid URL passes");
    }

    /// An `rtsp_push` URL is validated at config time: a cannot-be-a-base URL
    /// (`rtsp:cam`, no host) is a config error, not a panic when the push task
    /// builds its control URL.
    #[test]
    fn validate_rejects_rtsp_push_url_without_a_host() {
        let mk = |url: &str| Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtsp {
                    url: "rtsp://a".into(),
                    auth: None,
                },
                outputs: vec![OutputKind::RtspPush {
                    url: url.to_string(),
                    format: None,
                    reconnect: None,
                }],
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        let err = mk("rtsp:cam").validate().unwrap_err();
        assert!(
            format!("{err}").contains("no host"),
            "a hostless rtsp push URL must be a config error: {err}"
        );
        assert!(
            mk("rtsp://cam/live").validate().is_ok(),
            "a valid rtsp push URL passes"
        );
    }

    /// The exact config string the reviewer cited (`rtsp:cam`) must be
    /// rejected by the URL validator itself.
    #[test]
    fn validate_rtsp_push_url_rejects_rtsp_cam() {
        assert!(validate_rtsp_push_url("rtsp:cam").is_err());
        assert!(validate_rtsp_push_url("rtsp://cam").is_ok());
    }

    /// Audit run 7, W19: a zero reconnect backoff is rejected (it would
    /// reconnect with no delay), and an extreme (non-finite) ingest timeout
    /// is rejected at validation rather than panicking later in
    /// `Duration::from_secs_f64`.
    #[test]
    fn validate_rejects_zero_backoff_and_non_finite_timeouts() {
        fn route_with(reconnect: ReconnectPolicy) -> Config {
            Config {
                routes: vec![Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://a".into(),
                        auth: None,
                    },
                    outputs: vec![OutputKind::RtmpPush {
                        url: "rtmp://a/live".into(),
                        format: None,
                        reconnect: Some(reconnect),
                    }],
                    dvr: DvrConfig::default(),
                }],
                ..Config::default()
            }
        }

        // Zero initial or max backoff is rejected.
        assert!(
            route_with(ReconnectPolicy {
                initial_backoff_ms: 0,
                ..ReconnectPolicy::default()
            })
            .validate()
            .is_err()
        );
        assert!(
            route_with(ReconnectPolicy {
                max_backoff_ms: 0,
                ..ReconnectPolicy::default()
            })
            .validate()
            .is_err()
        );
        // initial > max is rejected.
        assert!(
            route_with(ReconnectPolicy {
                initial_backoff_ms: 5_000,
                max_backoff_ms: 1_000,
                ..ReconnectPolicy::default()
            })
            .validate()
            .is_err()
        );
        // An absurdly large backoff is rejected.
        assert!(
            route_with(ReconnectPolicy {
                initial_backoff_ms: 1_000,
                max_backoff_ms: u64::MAX,
                ..ReconnectPolicy::default()
            })
            .validate()
            .is_err()
        );
        // The same policy is validated on SrtPush and RtspPush outputs too.
        for kind in [
            OutputKind::SrtPush {
                url: "srt://h:9000".into(),
                format: None,
                reconnect: Some(ReconnectPolicy {
                    initial_backoff_ms: 0,
                    ..ReconnectPolicy::default()
                }),
            },
            OutputKind::RtspPush {
                url: "rtsp://h/live".into(),
                format: None,
                reconnect: Some(ReconnectPolicy {
                    initial_backoff_ms: 0,
                    ..ReconnectPolicy::default()
                }),
            },
        ] {
            let cfg = Config {
                routes: vec![Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://a".into(),
                        auth: None,
                    },
                    outputs: vec![kind],
                    dvr: DvrConfig::default(),
                }],
                ..Config::default()
            };
            assert!(
                cfg.validate().is_err(),
                "a zero backoff on any push output must be rejected"
            );
        }
        // A sane policy passes.
        route_with(ReconnectPolicy::default()).validate().unwrap();

        // NaN and an overflowing (but finite) timeout are both rejected.
        for bad in [f64::NAN, f64::INFINITY, 1e20] {
            let cfg = Config {
                ingest_read_timeout_secs: bad,
                ..route_with(ReconnectPolicy::default())
            };
            assert!(
                cfg.validate().is_err(),
                "ingest_read_timeout_secs {bad} must be rejected"
            );
        }
    }

    /// An unvalidated config with an extreme timeout must not panic
    /// `IngestTimeouts::from` — it falls back to the default.
    #[test]
    fn ingest_timeouts_from_extreme_config_does_not_panic() {
        let cfg = Config {
            ingest_connect_timeout_secs: f64::NAN,
            ingest_read_timeout_secs: 1e20,
            ..Config::default()
        };
        let timeouts = crate::source::IngestTimeouts::from(&cfg);
        assert_eq!(timeouts.connect, crate::source::DEFAULT_CONNECT_TIMEOUT);
        assert_eq!(timeouts.read, crate::source::DEFAULT_READ_TIMEOUT);
    }

    // --- issue #663 P4: per-route `outputs` ---

    /// `OutputKind` no longer derives `PartialEq` (its `Custom` variant
    /// carries a `serde_json::Value` — see the type's doc comment), so tests
    /// compare a parsed `outputs` list by each kind's `name()` label instead
    /// of `==`.
    fn output_kind_names(kinds: &[OutputKind]) -> Vec<&str> {
        kinds.iter().map(OutputKind::name).collect()
    }

    /// A route with no `outputs` key defaults to LL-HLS only — every
    /// pre-#663-P4 config keeps working unchanged.
    #[test]
    fn route_outputs_defaults_to_llhls_only_when_omitted() {
        let json = r#"{
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(output_kind_names(&cfg.routes[0].outputs), vec!["llhls"]);
        cfg.validate().unwrap();
    }

    /// A route may name both outputs explicitly (issue #663 P4's headline
    /// config shape: one ingest, LL-HLS + DASH).
    #[test]
    fn route_outputs_parses_llhls_and_dash() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["llhls", "dash"]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(
            output_kind_names(&cfg.routes[0].outputs),
            vec!["llhls", "dash"]
        );
        cfg.validate().unwrap();
    }

    /// A DASH-only route is valid too — `outputs` genuinely selects the set,
    /// it isn't just an LL-HLS toggle.
    #[test]
    fn route_outputs_dash_only_is_valid() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["dash"]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(output_kind_names(&cfg.routes[0].outputs), vec!["dash"]);
        cfg.validate().unwrap();
    }

    /// Issue #663 P4.2: a route may name `ll_dash` alongside `llhls`/`dash` —
    /// the headline config shape for low-latency DASH.
    #[test]
    fn route_outputs_parses_ll_dash() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["llhls", "dash", "ll_dash"]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(
            output_kind_names(&cfg.routes[0].outputs),
            vec!["llhls", "dash", "ll_dash"]
        );
        cfg.validate().unwrap();
    }

    /// An explicitly empty `outputs` list must be rejected at `validate()`
    /// time (a route with nothing to serve is a config mistake, not a
    /// silently-do-nothing route).
    #[test]
    fn validate_rejects_empty_outputs_list() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": []
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(cfg.validate().is_err());
    }

    /// An unknown `outputs` token (e.g. a typo'd `"lldash"`, not yet
    /// implemented) is rejected at parse time, not silently dropped.
    #[test]
    fn rejects_unknown_output_kind() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["lldash"]
                }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown output kind must be rejected");
    }

    /// Issue #887: `"ts_hls"` is mutually exclusive with `"llhls"` (and with
    /// `"dash"`/`"ll_dash"`) on the same route — a `Trunk` has one segment
    /// ring per program, so a program's samples are segmented into fMP4 or
    /// classic TS, never both. Rejected at `Config::validate()` time with a
    /// message naming both offending kinds and the route.
    #[test]
    fn validate_rejects_ts_hls_combined_with_llhls_on_one_route() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["ts_hls", "llhls"]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        let err = cfg
            .validate()
            .expect_err("ts_hls + llhls on one route must be rejected");
        let message = err.to_string();
        assert!(
            message.contains("ts_hls") && message.contains("llhls"),
            "error must name both offending output kinds: {message}"
        );
    }

    /// `"ts_hls"` alone (or alongside another `"ts_hls"`-only route) is fine
    /// — only combined with an fMP4-based output on the SAME route is
    /// rejected.
    #[test]
    fn validate_accepts_ts_hls_alone() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["ts_hls"]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        cfg.validate()
            .expect("a ts_hls-only route must be accepted");
    }

    // --- issue #900: "catchup" output requires routes.dvr.enabled ---

    /// Biting test: remove the check this test targets from
    /// `validate_standalone` and this fails, because the config is
    /// otherwise entirely valid (a plain rtsp route, one output kind).
    #[test]
    fn validate_rejects_catchup_output_without_dvr_enabled() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["catchup"]
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        let err = cfg
            .validate()
            .expect_err("catchup without routes.dvr.enabled must be rejected");
        let message = err.to_string();
        assert!(
            message.contains("catchup") && message.contains("DVR"),
            "error must name the offending output and explain why: {message}"
        );
    }

    #[test]
    fn validate_accepts_catchup_output_with_dvr_enabled() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
                    "outputs": ["catchup", "llhls"],
                    "dvr": {
                        "enabled": true,
                        "archive_root": "/tmp/multimux-dvr",
                        "retention_periods": 8
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        cfg.validate()
            .expect("catchup with routes.dvr.enabled must be accepted");
    }

    #[test]
    fn parses_json_config_with_rtp_input() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
                   m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
                   a=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\n";
        let json = serde_json::json!({
            "bind": "127.0.0.1:9000",
            "target_duration_secs": 2.0,
            "part_target_ms": 250,
            "window_segments": 6,
            "routes": [
                {
                    "name": "cam-rtp",
                    "input": {
                        "type": "rtp",
                        "addr": "0.0.0.0:5004",
                        "sdp": sdp,
                        "multicast_group": "239.1.1.1"
                    }
                }
            ]
        });
        let cfg: Config = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::Rtp {
                addr,
                sdp: parsed_sdp,
                multicast_group,
            } => {
                assert_eq!(addr, "0.0.0.0:5004");
                assert_eq!(parsed_sdp, sdp);
                assert_eq!(multicast_group.as_deref(), Some("239.1.1.1"));
            }
            other => panic!("expected InputSpec::Rtp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_ts_udp_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts",
                    "input": { "type": "ts_udp", "addr": "0.0.0.0:5005" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::TsUdp {
                addr,
                multicast_group,
            } => {
                assert_eq!(addr, "0.0.0.0:5005");
                assert_eq!(*multicast_group, None);
            }
            other => panic!("expected InputSpec::TsUdp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_ts_udp_multicast_group() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts-mc",
                    "input": {
                        "type": "ts_udp",
                        "addr": "0.0.0.0:5006",
                        "multicast_group": "239.2.2.2"
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::TsUdp {
                multicast_group, ..
            } => assert_eq!(multicast_group.as_deref(), Some("239.2.2.2")),
            other => panic!("expected InputSpec::TsUdp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_ts_http_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts-http",
                    "input": { "type": "ts_http", "url": "http://host/stream.ts" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::TsHttp { url, .. } => assert_eq!(url, "http://host/stream.ts"),
            other => panic!("expected InputSpec::TsHttp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_hls_pull_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-hls-pull",
                    "input": { "type": "hls_pull", "url": "https://origin/live/media.m3u8" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::HlsPull { url, .. } => assert_eq!(url, "https://origin/live/media.m3u8"),
            other => panic!("expected InputSpec::HlsPull, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_smooth_pull_input() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-smooth-pull",
                    "input": { "type": "smooth_pull", "url": "https://origin/live.ism/Manifest" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        match &cfg.routes[0].input {
            InputSpec::SmoothPull { url, .. } => {
                assert_eq!(url, "https://origin/live.ism/Manifest")
            }
            other => panic!("expected InputSpec::SmoothPull, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    // --- issue #663 "Finish client-side multi-scheme auth": config-supplied
    // `auth` (`AuthSpec`) on `Rtsp`/`TsHttp`/`HlsPull` ---

    #[test]
    fn parses_json_config_with_password_auth() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts-http",
                    "input": {
                        "type": "ts_http",
                        "url": "http://host/stream.ts",
                        "auth": { "username": "admin", "password": "hunter2" }
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::TsHttp { auth, .. } => match auth {
                Some(AuthSpec::Password { username, password }) => {
                    assert_eq!(username, "admin");
                    assert_eq!(password, "hunter2");
                }
                other => panic!("expected Some(AuthSpec::Password), got {other:?}"),
            },
            other => panic!("expected InputSpec::TsHttp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_json_config_with_bearer_auth() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-hls-pull",
                    "input": {
                        "type": "hls_pull",
                        "url": "https://origin/live/media.m3u8",
                        "auth": { "bearer_token": "tok123" }
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::HlsPull { auth, .. } => match auth {
                Some(AuthSpec::Bearer { bearer_token }) => assert_eq!(bearer_token, "tok123"),
                other => panic!("expected Some(AuthSpec::Bearer), got {other:?}"),
            },
            other => panic!("expected InputSpec::HlsPull, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// `Rtsp` also takes config-supplied `auth` — the same field, same
    /// precedence-over-URL-userinfo rule, just for the RTSP transport.
    #[test]
    fn parses_json_config_with_rtsp_password_auth() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam1",
                    "input": {
                        "type": "rtsp",
                        "url": "rtsp://host/stream",
                        "auth": { "username": "admin", "password": "hunter2" }
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::Rtsp { auth, .. } => {
                assert!(matches!(auth, Some(AuthSpec::Password { .. })));
            }
            other => panic!("expected InputSpec::Rtsp, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// A route with no `auth` key at all still parses (backward
    /// compatibility with every pre-existing config) and defaults to `None`.
    #[test]
    fn auth_defaults_to_none_when_omitted() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-ts-http",
                    "input": { "type": "ts_http", "url": "http://host/stream.ts" }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::TsHttp { auth, .. } => assert!(auth.is_none()),
            other => panic!("expected InputSpec::TsHttp, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_empty_auth_username() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsHttp {
                    url: "http://host/stream.ts".into(),
                    auth: Some(AuthSpec::Password {
                        username: String::new(),
                        password: "p".into(),
                    }),
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_bearer_token() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::HlsPull {
                    url: "https://host/media.m3u8".into(),
                    auth: Some(AuthSpec::Bearer {
                        bearer_token: String::new(),
                    }),
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    /// A `password` may legitimately be empty (some devices use a blank
    /// password) — only `username`/`bearer_token` are rejected when empty.
    #[test]
    fn validate_accepts_empty_password() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsHttp {
                    url: "http://host/stream.ts".into(),
                    auth: Some(AuthSpec::Password {
                        username: "admin".into(),
                        password: String::new(),
                    }),
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        cfg.validate().unwrap();
    }

    /// Biting test: config-supplied `auth` must never appear in `Debug`
    /// output — neither the password nor the bearer token.
    #[test]
    fn input_spec_debug_redacts_config_supplied_auth() {
        let password_auth = InputSpec::TsHttp {
            url: "http://host/stream.ts".into(),
            auth: Some(AuthSpec::Password {
                username: "admin".into(),
                password: "hunter2secret".into(),
            }),
        };
        let debug = format!("{password_auth:?}");
        assert!(debug.contains("admin"), "username may render: {debug}");
        assert!(
            !debug.contains("hunter2secret"),
            "debug leaked password: {debug}"
        );

        let bearer_auth = InputSpec::HlsPull {
            url: "https://host/media.m3u8".into(),
            auth: Some(AuthSpec::Bearer {
                bearer_token: "supersecrettoken".into(),
            }),
        };
        let debug = format!("{bearer_auth:?}");
        assert!(
            !debug.contains("supersecrettoken"),
            "debug leaked bearer token: {debug}"
        );
    }

    /// `AuthSpec::to_credentials` converts to the scheme-agnostic
    /// `broadcast_auth::Credentials` the sources actually authenticate with.
    #[test]
    fn auth_spec_to_credentials_converts_both_variants() {
        let password = AuthSpec::Password {
            username: "admin".into(),
            password: "hunter2".into(),
        };
        assert_eq!(
            password.to_credentials(),
            Credentials::new("admin", "hunter2")
        );

        let bearer = AuthSpec::Bearer {
            bearer_token: "tok".into(),
        };
        assert_eq!(bearer.to_credentials(), Credentials::bearer("tok"));
    }

    #[test]
    fn validate_rejects_bad_ts_http_scheme() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsHttp {
                    url: "rtsp://host/stream.ts".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_hls_pull_scheme() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::HlsPull {
                    url: "ftp://host/media.m3u8".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparsable_ts_http_url() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsHttp {
                    url: "not a url".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    /// Biting test: an `InputSpec::TsHttp`/`HlsPull`'s credential must never
    /// appear in `Debug` output, mirroring `route_debug_redacts_rtsp_credentials`.
    #[test]
    fn route_debug_redacts_ts_http_and_hls_pull_credentials() {
        let ts_http = Route {
            name: "cam-ts-http".into(),
            input: InputSpec::TsHttp {
                url: "http://user:secretpass@host/stream.ts".into(),
                auth: None,
            },
            outputs: default_outputs(),
            dvr: DvrConfig::default(),
        };
        let debug = format!("{ts_http:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");

        let hls_pull = Route {
            name: "cam-hls-pull".into(),
            input: InputSpec::HlsPull {
                url: "https://user:secretpass@origin/media.m3u8".into(),
                auth: None,
            },
            outputs: default_outputs(),
            dvr: DvrConfig::default(),
        };
        let debug = format!("{hls_pull:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@origin"), "debug: {debug}");
    }

    #[test]
    fn validate_rejects_bad_rtsp_scheme() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtsp {
                    url: "http://host/stream".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparsable_udp_addr() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsUdp {
                    addr: "not-an-addr".into(),
                    multicast_group: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_multicast_group() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::TsUdp {
                    addr: "0.0.0.0:5005".into(),
                    // A unicast address, not a valid multicast group.
                    multicast_group: Some("10.0.0.1".into()),
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_rtp_sdp() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtp {
                    addr: "0.0.0.0:5004".into(),
                    sdp: String::new(),
                    multicast_group: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unparsable_inline_rtp_sdp() {
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtp {
                    addr: "0.0.0.0:5004".into(),
                    sdp: "not an sdp body".into(),
                    multicast_group: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_at_path_rtp_sdp_reference_without_reading_it() {
        // The referenced file need not exist yet at validate() time — only
        // connect() reads/parses it (via `crate::source::sdp::load_sdp`).
        let cfg = Config {
            routes: vec![Route {
                name: "x".into(),
                input: InputSpec::Rtp {
                    addr: "0.0.0.0:5004".into(),
                    sdp: "@/no/such/file/does-not-exist.sdp".into(),
                    multicast_group: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_duplicate_stream_names() {
        let cfg = Config {
            routes: vec![
                Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://a".into(),
                        auth: None,
                    },
                    outputs: default_outputs(),
                    dvr: DvrConfig::default(),
                },
                Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://b".into(),
                        auth: None,
                    },
                    outputs: default_outputs(),
                    dvr: DvrConfig::default(),
                },
            ],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_no_routes() {
        assert!(Config::default().validate().is_err());
    }

    #[test]
    fn rejects_unknown_config_key() {
        // A typo'd key (e.g. "window_segment" instead of "window_segments")
        // must error rather than silently fall back to the default —
        // `#[serde(deny_unknown_fields)]` on `Config` enforces this.
        let json = r#"{
            "bind": "127.0.0.1:9000",
            "window_segment": 6,
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "unknown key must be rejected, not silently ignored"
        );
    }

    #[test]
    fn rejects_unknown_input_type() {
        // A typo'd/unsupported `type` discriminator (e.g. "rtmp") must be
        // rejected by serde's internally-tagged enum, not silently coerced
        // into one of the known variants.
        let json = r#"{
            "routes": [
                { "name": "cam1", "input": { "type": "rtmp", "url": "rtmp://host/stream1" } }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown input type must be rejected");
    }

    /// Biting test: an `InputSpec::Rtsp`'s credential must never appear in
    /// its (and therefore `Route`'s) `Debug` output. Fails immediately if
    /// the manual `Debug` impl is reverted to `#[derive(Debug)]` (which
    /// would render `url` verbatim, userinfo included).
    #[test]
    fn route_debug_redacts_rtsp_credentials() {
        let route = Route {
            name: "cam1".into(),
            input: InputSpec::Rtsp {
                url: "rtsp://user:secretpass@host/s".into(),
                auth: None,
            },
            outputs: default_outputs(),
            dvr: DvrConfig::default(),
        };
        let debug = format!("{route:?}");
        assert!(!debug.contains("user"), "debug leaked username: {debug}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");
    }

    /// Same biting property, but through `Config`'s *derived* `Debug` — this
    /// proves the redaction is wired end-to-end (a route embedded in a
    /// config, as it always is at runtime) and not just on a bare `Route`.
    #[test]
    fn config_debug_redacts_route_credentials() {
        let cfg = Config {
            routes: vec![Route {
                name: "cam1".into(),
                input: InputSpec::Rtsp {
                    url: "rtsp://user:secretpass@host/s".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        };
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("user"), "config debug leaked username");
        assert!(
            !debug.contains("secretpass"),
            "config debug leaked password"
        );
        assert!(debug.contains("***@host"));
    }

    /// A raw-RTP route's `Debug` must not dump the full SDP body verbatim
    /// (just its length) — keeps a route's `Debug`/log line short even when
    /// the SDP is large, mirroring the RTSP variant's "no giant blobs in
    /// logs" spirit even though the SDP itself carries no secret.
    #[test]
    fn route_debug_shows_sdp_length_not_full_body() {
        let long_sdp = "v=0\r\n".repeat(50);
        let route = Route {
            name: "cam-rtp".into(),
            input: InputSpec::Rtp {
                addr: "0.0.0.0:5004".into(),
                sdp: long_sdp.clone(),
                multicast_group: None,
            },
            outputs: default_outputs(),
            dvr: DvrConfig::default(),
        };
        let debug = format!("{route:?}");
        assert!(!debug.contains(&long_sdp), "debug: {debug}");
        assert!(
            debug.contains(&long_sdp.len().to_string()),
            "debug: {debug}"
        );
    }

    // --- issue #663 "configurable `playlist_name`" ---

    fn cfg_with_one_route() -> Config {
        Config {
            routes: vec![Route {
                name: "cam1".into(),
                input: InputSpec::Rtsp {
                    url: "rtsp://host/stream".into(),
                    auth: None,
                },
                outputs: default_outputs(),
                dvr: DvrConfig::default(),
            }],
            ..Config::default()
        }
    }

    #[test]
    fn playlist_name_defaults_to_media_m3u8_when_omitted() {
        let json = r#"{
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.playlist_name, "media.m3u8");
        cfg.validate().unwrap();
    }

    #[test]
    fn playlist_name_parses_from_json() {
        let json = r#"{
            "playlist_name": "index.m3u8",
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.playlist_name, "index.m3u8");
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_empty_playlist_name() {
        let cfg = Config {
            playlist_name: String::new(),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_playlist_name_without_m3u8_suffix() {
        let cfg = Config {
            playlist_name: "media.mpd".into(),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_playlist_name_with_slash() {
        let cfg = Config {
            playlist_name: "sub/media.m3u8".into(),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_playlist_name_master_m3u8_collision() {
        let cfg = Config {
            playlist_name: "master.m3u8".into(),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_a_valid_non_default_playlist_name() {
        let cfg = Config {
            playlist_name: "index.m3u8".into(),
            ..cfg_with_one_route()
        };
        cfg.validate().unwrap();
    }

    // --- issue #663 "shared output auth" ---

    #[test]
    fn output_auth_defaults_to_none_when_omitted() {
        let json = r#"{
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(cfg.output_auth.is_none());
        cfg.validate().unwrap();
    }

    #[test]
    fn output_auth_parses_basic() {
        let json = r#"{
            "output_auth": { "scheme": "basic", "username": "admin", "password": "hunter2" },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::Basic { username, password }) => {
                assert_eq!(username, "admin");
                assert_eq!(password, "hunter2");
            }
            other => panic!("expected Some(OutputAuthSpec::Basic), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn output_auth_parses_digest() {
        let json = r#"{
            "output_auth": { "scheme": "digest", "username": "admin", "password": "hunter2" },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(matches!(
            &cfg.output_auth,
            Some(OutputAuthSpec::Digest { .. })
        ));
        cfg.validate().unwrap();
    }

    #[test]
    fn output_auth_parses_bearer() {
        let json = r#"{
            "output_auth": { "scheme": "bearer", "token": "tok123" },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::Bearer { token }) => assert_eq!(token, "tok123"),
            other => panic!("expected Some(OutputAuthSpec::Bearer), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// Issue #663 extensibility wave part 1: the `forwarded` scheme parses
    /// with explicit header names.
    #[test]
    fn output_auth_parses_forwarded() {
        let json = r#"{
            "output_auth": {
                "scheme": "forwarded",
                "user_header": "X-Auth-User",
                "forwarded_for_header": "X-Real-IP"
            },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::Forwarded {
                user_header,
                forwarded_for_header,
            }) => {
                assert_eq!(user_header, "X-Auth-User");
                assert_eq!(forwarded_for_header.as_deref(), Some("X-Real-IP"));
            }
            other => panic!("expected Some(OutputAuthSpec::Forwarded), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// Omitting `user_header`/`forwarded_for_header` defaults to
    /// `X-Forwarded-User`/`Some("X-Forwarded-For")`.
    #[test]
    fn output_auth_forwarded_defaults_headers_when_omitted() {
        let json = r#"{
            "output_auth": { "scheme": "forwarded" },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::Forwarded {
                user_header,
                forwarded_for_header,
            }) => {
                assert_eq!(user_header, "X-Forwarded-User");
                assert_eq!(forwarded_for_header.as_deref(), Some("X-Forwarded-For"));
            }
            other => panic!("expected Some(OutputAuthSpec::Forwarded), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// `forwarded_for_header: null` explicitly disables reading it at all.
    #[test]
    fn output_auth_forwarded_for_header_can_be_disabled() {
        let json = r#"{
            "output_auth": {
                "scheme": "forwarded",
                "forwarded_for_header": null
            },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::Forwarded {
                forwarded_for_header,
                ..
            }) => assert_eq!(*forwarded_for_header, None),
            other => panic!("expected Some(OutputAuthSpec::Forwarded), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_output_auth_forwarded_empty_user_header() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::Forwarded {
                user_header: String::new(),
                forwarded_for_header: None,
            }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_output_auth_forwarded_empty_forwarded_for_header() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::Forwarded {
                user_header: "X-Forwarded-User".into(),
                forwarded_for_header: Some(String::new()),
            }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn output_auth_rejects_unknown_scheme() {
        let json = r#"{
            "output_auth": { "scheme": "hmac", "username": "admin", "password": "p" },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let result: std::result::Result<Config, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "unknown output_auth scheme must be rejected"
        );
    }

    #[test]
    fn validate_rejects_output_auth_empty_username() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::Basic {
                username: String::new(),
                password: "p".into(),
            }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_output_auth_empty_bearer_token() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::Bearer {
                token: String::new(),
            }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_output_auth_empty_password() {
        // Mirrors AuthSpec's own "empty password is allowed" rule.
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::Basic {
                username: "admin".into(),
                password: String::new(),
            }),
            ..cfg_with_one_route()
        };
        cfg.validate().unwrap();
    }

    #[test]
    fn output_auth_spec_build_verifier_preserves_scheme_exactly() {
        // Unlike `AuthSpec::to_credentials` (which always builds a `Digest`
        // value regardless of what the caller intends, since the *client*
        // answers whichever scheme the server's challenge asks for),
        // `OutputAuthSpec` is the *server* side: it must issue the challenge
        // for the scheme actually configured, so `Basic` must produce a
        // `Verifier` whose challenge is `Basic`, not `Digest` — checked via
        // `Verifier::challenge`'s scheme-distinguishing prefix (the same
        // thing `crate::origin`'s output-auth gate sends on a `401`).
        let basic = OutputAuthSpec::Basic {
            username: "admin".into(),
            password: "p".into(),
        };
        assert!(
            basic
                .build_verifier("realm")
                .challenge()
                .starts_with("Basic ")
        );

        let digest = OutputAuthSpec::Digest {
            username: "admin".into(),
            password: "p".into(),
        };
        assert!(
            digest
                .build_verifier("realm")
                .challenge()
                .starts_with("Digest ")
        );

        let bearer = OutputAuthSpec::Bearer {
            token: "tok".into(),
        };
        assert_eq!(bearer.build_verifier("realm").challenge(), "Bearer");

        let forwarded = OutputAuthSpec::Forwarded {
            user_header: "X-Forwarded-User".into(),
            forwarded_for_header: Some("X-Forwarded-For".into()),
        };
        assert_eq!(forwarded.build_verifier("realm").challenge(), "Forwarded");
    }

    /// Biting test: `OutputAuthSpec`'s `Debug` must never render the
    /// password/token verbatim.
    #[test]
    fn output_auth_spec_debug_redacts_secret() {
        let basic = OutputAuthSpec::Basic {
            username: "admin".into(),
            password: "supersecretpass".into(),
        };
        let debug = format!("{basic:?}");
        assert!(debug.contains("admin"), "username may render: {debug}");
        assert!(!debug.contains("supersecretpass"), "debug: {debug}");

        let bearer = OutputAuthSpec::Bearer {
            token: "supersecrettoken".into(),
        };
        let debug = format!("{bearer:?}");
        assert!(!debug.contains("supersecrettoken"), "debug: {debug}");
    }

    /// `Forwarded` carries no secret, so its header names render plainly —
    /// still worth a biting test that `Debug` doesn't panic and does name
    /// both fields.
    #[test]
    fn output_auth_spec_forwarded_debug_shows_header_names() {
        let forwarded = OutputAuthSpec::Forwarded {
            user_header: "X-Forwarded-User".into(),
            forwarded_for_header: Some("X-Forwarded-For".into()),
        };
        let debug = format!("{forwarded:?}");
        assert!(debug.contains("X-Forwarded-User"), "debug: {debug}");
        assert!(debug.contains("X-Forwarded-For"), "debug: {debug}");
    }

    // --- issue #747 "signed-URL output auth" ---

    fn valid_signed_url_key() -> String {
        "01234567890123456789012345678901".to_string() // 32 bytes
    }

    #[test]
    fn output_auth_parses_signed_url() {
        let json = format!(
            r#"{{
                "output_auth": {{
                    "scheme": "signed_url",
                    "keys": [{{ "kid": "key-1", "secret": "{}" }}]
                }},
                "routes": [
                    {{ "name": "cam1", "input": {{ "type": "rtsp", "url": "rtsp://host/stream1" }} }}
                ]
            }}"#,
            valid_signed_url_key()
        );
        let cfg: Config = serde_json::from_str(&json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::SignedUrl { keys }) => {
                assert_eq!(keys.len(), 1);
                assert_eq!(keys[0].kid, "key-1");
                assert_eq!(keys[0].secret, valid_signed_url_key());
            }
            other => panic!("expected Some(OutputAuthSpec::SignedUrl), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn output_auth_parses_signed_url_with_multiple_keys_for_rotation() {
        let json = format!(
            r#"{{
                "output_auth": {{
                    "scheme": "signed_url",
                    "keys": [
                        {{ "kid": "old", "secret": "{}" }},
                        {{ "kid": "new", "secret": "{}" }}
                    ]
                }},
                "routes": [
                    {{ "name": "cam1", "input": {{ "type": "rtsp", "url": "rtsp://host/stream1" }} }}
                ]
            }}"#,
            valid_signed_url_key(),
            "abcdefghijabcdefghijabcdefghij01"
        );
        let cfg: Config = serde_json::from_str(&json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::SignedUrl { keys }) => assert_eq!(keys.len(), 2),
            other => panic!("expected Some(OutputAuthSpec::SignedUrl), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_output_auth_signed_url_empty_keys() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::SignedUrl { keys: vec![] }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_output_auth_signed_url_empty_kid() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::SignedUrl {
                keys: vec![SignedUrlKeySpec {
                    kid: String::new(),
                    secret: valid_signed_url_key(),
                }],
            }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    /// Biting test: the 32-byte minimum secret length is enforced at
    /// config-validate time, not silently accepted or deferred to a
    /// per-request failure.
    #[test]
    fn validate_rejects_output_auth_signed_url_secret_too_short() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::SignedUrl {
                keys: vec![SignedUrlKeySpec {
                    kid: "key-1".into(),
                    secret: "too-short".into(),
                }],
            }),
            ..cfg_with_one_route()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_output_auth_signed_url_secret_exactly_min_len() {
        let cfg = Config {
            output_auth: Some(OutputAuthSpec::SignedUrl {
                keys: vec![SignedUrlKeySpec {
                    kid: "key-1".into(),
                    secret: valid_signed_url_key(),
                }],
            }),
            ..cfg_with_one_route()
        };
        cfg.validate().unwrap();
    }

    /// `build_verifier` produces a `SignedUrl`-scheme `Verifier` (bare
    /// scheme-name challenge, same as `Forwarded`) once `validate()` has
    /// already guaranteed every secret meets the minimum length.
    #[test]
    fn output_auth_spec_signed_url_build_verifier_produces_signed_url_scheme() {
        let spec = OutputAuthSpec::SignedUrl {
            keys: vec![SignedUrlKeySpec {
                kid: "key-1".into(),
                secret: valid_signed_url_key(),
            }],
        };
        spec.validate().unwrap();
        assert_eq!(spec.build_verifier("realm").challenge(), "SignedUrl");
    }

    /// Biting test: `OutputAuthSpec::SignedUrl`'s `Debug` must show `kid`s
    /// but never render a `secret`.
    #[test]
    fn output_auth_spec_signed_url_debug_redacts_secret() {
        let spec = OutputAuthSpec::SignedUrl {
            keys: vec![SignedUrlKeySpec {
                kid: "key-1".into(),
                secret: valid_signed_url_key(),
            }],
        };
        let debug = format!("{spec:?}");
        assert!(debug.contains("key-1"), "kid should render: {debug}");
        assert!(
            !debug.contains(&valid_signed_url_key()),
            "secret leaked: {debug}"
        );
    }

    // --- issue #663 external scheme plugin registry: `Custom` variants ---

    /// `InputSpec::Custom` deserializes with the right `type_tag`/`params`,
    /// and always validates (the registry checks `params` at build time, not
    /// `Config::validate`).
    #[test]
    fn input_spec_custom_deserializes_with_type_tag_and_params() {
        let json = r#"{
            "routes": [
                {
                    "name": "cam-custom",
                    "input": {
                        "type": "custom",
                        "type_tag": "webrtc",
                        "params": { "offer_url": "https://example/offer" }
                    }
                }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::Custom { type_tag, params } => {
                assert_eq!(type_tag, "webrtc");
                assert_eq!(
                    params.get("offer_url").and_then(|v| v.as_str()),
                    Some("https://example/offer")
                );
            }
            other => panic!("expected InputSpec::Custom, got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// `InputSpec::Custom`'s `params` defaults to `null` when omitted.
    #[test]
    fn input_spec_custom_params_defaults_to_null_when_omitted() {
        let json = r#"{
            "routes": [
                { "name": "cam-custom", "input": { "type": "custom", "type_tag": "webrtc" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.routes[0].input {
            InputSpec::Custom { params, .. } => assert!(params.is_null()),
            other => panic!("expected InputSpec::Custom, got {other:?}"),
        }
    }

    /// Biting test: `InputSpec::Custom`'s `Debug` must show `type_tag` but
    /// never render `params` (which may hold an external scheme's
    /// credentials) — checked with a secret planted in `params`.
    #[test]
    fn input_spec_custom_debug_redacts_params() {
        let spec = InputSpec::Custom {
            type_tag: "webrtc".into(),
            params: serde_json::json!({ "password": "s3cret" }),
        };
        let debug = format!("{spec:?}");
        assert!(debug.contains("webrtc"), "type_tag may render: {debug}");
        assert!(!debug.contains("s3cret"), "debug leaked params: {debug}");
    }

    /// `OutputAuthSpec::Custom` deserializes with the right `type_tag`/
    /// `params`, and always validates.
    #[test]
    fn output_auth_spec_custom_deserializes_with_type_tag_and_params() {
        let json = r#"{
            "output_auth": {
                "scheme": "custom",
                "type_tag": "hmac",
                "params": { "key_id": "abc" }
            },
            "routes": [
                { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        match &cfg.output_auth {
            Some(OutputAuthSpec::Custom { type_tag, params }) => {
                assert_eq!(type_tag, "hmac");
                assert_eq!(params.get("key_id").and_then(|v| v.as_str()), Some("abc"));
            }
            other => panic!("expected Some(OutputAuthSpec::Custom), got {other:?}"),
        }
        cfg.validate().unwrap();
    }

    /// Biting test: `OutputAuthSpec::Custom`'s `Debug` must show `type_tag`
    /// but never render `params`.
    #[test]
    fn output_auth_spec_custom_debug_redacts_params() {
        let spec = OutputAuthSpec::Custom {
            type_tag: "hmac".into(),
            params: serde_json::json!({ "shared_secret": "topsecret" }),
        };
        let debug = format!("{spec:?}");
        assert!(debug.contains("hmac"), "type_tag may render: {debug}");
        assert!(!debug.contains("topsecret"), "debug leaked params: {debug}");
    }

    /// `OutputAuthSpec::Custom`'s `build_verifier` is never called by
    /// production code (`crate::origin::serve_with_registry` resolves it via
    /// the registry first) — documented via `#[should_panic]` so a future
    /// refactor that accidentally routes a `Custom` value into
    /// `build_verifier` fails loudly instead of silently misbehaving.
    #[test]
    #[should_panic(expected = "SchemeRegistry")]
    fn output_auth_spec_custom_build_verifier_is_unreachable() {
        let spec = OutputAuthSpec::Custom {
            type_tag: "hmac".into(),
            params: serde_json::Value::Null,
        };
        let _ = spec.build_verifier("realm");
    }

    // --- issue #739 review: caller `remote` accepts a hostname ---

    fn srt_input(listen: Option<&str>, remote: Option<&str>) -> InputSpec {
        InputSpec::Srt {
            listen: listen.map(str::to_string),
            remote: remote.map(str::to_string),
            stream_id: None,
            latency_ms: None,
        }
    }

    /// A caller-mode `remote` naming a hostname (not a literal `SocketAddr`)
    /// must pass `validate()` — `SrtSocket::connect` resolves it via
    /// `ToSocketAddrs`/DNS, exactly like the `InputSpec::Srt` doc's own
    /// `"remote-host:9000"` example, so rejecting it here would make that
    /// documented example itself invalid.
    #[test]
    fn srt_caller_remote_accepts_a_hostname() {
        let input = srt_input(None, Some("example.com:9000"));
        input.validate().expect("hostname remote must validate");
    }

    /// A literal `SocketAddr` `remote` (the strict shape) must also still
    /// validate — the looser host:port check must not regress the existing
    /// IP:port case.
    #[test]
    fn srt_caller_remote_accepts_a_literal_socket_addr() {
        let input = srt_input(None, Some("127.0.0.1:9000"));
        input
            .validate()
            .expect("literal socket addr remote must validate");
    }

    /// The `url`-crate host:port check must reject the shapes its OWN parser is
    /// looser about than the old `rsplit_once(':')` + `u16` parse: a path, a
    /// query, a fragment, and a userinfo prefix after/before the authority.
    /// (I10: adopting `url` silently accepted `host:9000/path` etc.)
    #[test]
    fn srt_caller_remote_rejects_a_non_bare_host_port() {
        for bad in [
            "host:9000/path",
            "host:9000/path/", // trailing slash normalised to a path
            "user@host:9000",
            "user:pass@host:9000",
            "host:9000?x=1",
            "host:9000#frag",
        ] {
            let input = srt_input(None, Some(bad));
            assert!(
                input.validate().is_err(),
                "{bad:?} must be rejected as not a bare host:port"
            );
        }
        // The bare forms still pass.
        for ok in ["example.com:9000", "127.0.0.1:9000", "[::1]:9000"] {
            srt_input(None, Some(ok))
                .validate()
                .unwrap_or_else(|e| panic!("{ok:?} must validate: {e:?}"));
        }
    }

    /// A `remote` with no `:port` at all must fail `validate()`, and the
    /// error's `field` must name `remote` (issue #739 review: the field was
    /// previously hardcoded to `"...listen"` even when `remote` is the
    /// branch that actually failed).
    #[test]
    fn srt_caller_remote_without_port_fails_with_remote_field() {
        let input = srt_input(None, Some("nonsense"));
        let err = input.validate().expect_err("remote with no port must fail");
        match err {
            MultimuxError::ConfigInvalid { field, .. } => {
                assert_eq!(
                    field, "routes.input.remote",
                    "field must name remote, got {err:?}"
                );
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    /// A `remote` with an empty host (`":9000"`) must also fail, still
    /// against the `remote` field.
    #[test]
    fn srt_caller_remote_with_empty_host_fails_with_remote_field() {
        let input = srt_input(None, Some(":9000"));
        let err = input
            .validate()
            .expect_err("remote with empty host must fail");
        match err {
            MultimuxError::ConfigInvalid { field, .. } => {
                assert_eq!(field, "routes.input.remote");
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    /// A `remote` with a non-numeric port must fail against the `remote`
    /// field too.
    #[test]
    fn srt_caller_remote_with_non_numeric_port_fails_with_remote_field() {
        let input = srt_input(None, Some("example.com:notaport"));
        let err = input
            .validate()
            .expect_err("remote with a non-numeric port must fail");
        match err {
            MultimuxError::ConfigInvalid { field, .. } => {
                assert_eq!(field, "routes.input.remote");
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    /// Listener-mode `listen` keeps the strict `SocketAddr` validator — a
    /// bind address is never resolved via DNS, so a hostname there must
    /// still be rejected (unlike `remote`), against the `listen` field.
    #[test]
    fn srt_listener_listen_rejects_a_hostname() {
        let input = srt_input(Some("example.com:9000"), None);
        let err = input
            .validate()
            .expect_err("hostname listen address must fail");
        match err {
            MultimuxError::ConfigInvalid { field, .. } => {
                assert_eq!(field, "routes.input.listen");
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    // --- issue #748 WP5: the file input is the whole feature now ---

    /// A `File` route input parses with its path, and `loop` defaults to
    /// `true` when omitted.
    #[test]
    fn file_input_parses_with_loop_defaulting_to_true() {
        let json = r#"{
            "routes": [
                { "name": "file-route", "input": { "type": "file", "path": "/media/slate.ts" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        cfg.validate().unwrap();
        match &cfg.routes[0].input {
            InputSpec::File { path, loop_file } => {
                assert_eq!(path, "/media/slate.ts");
                assert!(*loop_file, "loop must default to true when omitted");
            }
            other => panic!("expected InputSpec::File, got {other:?}"),
        }
    }

    /// `loop: false` on a `File` input is honored (not silently left at the
    /// default), and the `Debug` arm renders the path without redacting it.
    #[test]
    fn file_input_loop_false_is_honored_and_debug_shows_path() {
        let json = r#"{
            "routes": [
                { "name": "file-route", "input": { "type": "file", "path": "/media/slate.ts", "loop": false } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        cfg.validate().unwrap();
        match &cfg.routes[0].input {
            InputSpec::File { path, loop_file } => {
                assert_eq!(path, "/media/slate.ts");
                assert!(!*loop_file, "loop: false must be honored");
            }
            other => panic!("expected InputSpec::File, got {other:?}"),
        }
        let debug = format!("{:?}", cfg.routes[0].input);
        assert!(
            debug.contains("/media/slate.ts"),
            "the path is not a credential and must render: {debug}"
        );
    }

    /// An empty `path` on a `File` input is rejected at validate time — it is
    /// always invalid and would otherwise spin the route's supervisor retrying
    /// the same empty path forever.
    #[test]
    fn file_input_rejects_empty_path() {
        let json = r#"{
            "routes": [
                { "name": "file-route", "input": { "type": "file", "path": "" } }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        let err = cfg
            .validate()
            .expect_err("empty file path must fail validation");
        match err {
            MultimuxError::ConfigInvalid { field, .. } => {
                assert_eq!(field, "routes.input.path");
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }
}
