//! multimux configuration: routes + segmentation/window/bind parameters.
//!
//! CLI-first with an optional JSON config file. A route maps one input
//! ([`InputSpec`] — RTSP pull, raw RTP/UDP, MPEG-TS/UDP, MPEG-TS/HTTP, or
//! HLS-pull) to a served stream name.

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
///
/// [`InputSpec::TsHttp`]/[`InputSpec::HlsPull`] both may carry `user:pass@`
/// URL userinfo (Basic/Digest — see [`crate::source::http_auth`]), redacted
/// the same way [`InputSpec::Rtsp`]'s URL is.
///
/// [`InputSpec::Rtsp`]/[`InputSpec::TsHttp`]/[`InputSpec::HlsPull`] each also
/// take an optional config-supplied `auth` ([`AuthSpec`]) — the only way to
/// supply a Bearer token (RFC 6750 has no URL-userinfo form) and, when
/// present, taking precedence over any URL userinfo (see
/// `crate::source::http_auth::resolve_credentials`). [`InputSpec::Rtp`]/
/// [`InputSpec::TsUdp`] are raw UDP transports with no HTTP/RTSP request line
/// to attach credentials to, so they carry no `auth` field.
///
/// - [`InputSpec::Custom`] (issue #663 external scheme plugin registry) names
///   an external input scheme by an opaque `type_tag`, resolved at
///   `crate::origin::serve_with_registry` time via
///   [`crate::registry::SchemeRegistry::input`] — the escape hatch that lets
///   a third-party crate add a new ingest transport without editing this
///   crate. `params` is passed through unexamined to the registered factory.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
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
#[derive(Clone, Deserialize)]
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
    pub(crate) fn to_credentials(&self) -> Credentials {
        match self {
            AuthSpec::Password { username, password } => {
                Credentials::new(username.clone(), password.clone())
            }
            AuthSpec::Bearer { bearer_token } => Credentials::bearer(bearer_token.clone()),
        }
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
    /// HTTP Digest (RFC 7616) — a fresh server nonce is generated once per
    /// process (see `broadcast_auth::Verifier`'s nonce-handling caveat).
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
    /// trust-assumption docs).
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
    /// disable it instead of an empty string).
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
            _ => Ok(()),
        }
    }
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
            InputSpec::Custom { type_tag, .. } => f
                .debug_struct("Custom")
                .field("type_tag", type_tag)
                .field("params", &"<params>")
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
            // Always structurally valid: the registered factory (resolved at
            // `crate::origin::serve_with_registry` time, not here) validates
            // `params` itself.
            InputSpec::Custom { .. } => Ok(()),
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
#[derive(Clone, Deserialize)]
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
    /// Ingest connect-handshake timeout, in seconds, applied to every route's
    /// source (issue #663 P5, audit-ingest #3) — see
    /// [`crate::source::IngestTimeouts::connect`].
    pub ingest_connect_timeout_secs: f64,
    /// Ingest per-read timeout, in seconds, applied to every route's source
    /// — see [`crate::source::IngestTimeouts::read`].
    pub ingest_read_timeout_secs: f64,
    /// The LL-HLS media-playlist filename served at `/{stream}/{playlist_name}`
    /// (issue #663 "configurable `playlist_name`") — `master.m3u8`'s
    /// `#EXT-X-STREAM-INF` reference follows suit
    /// (`crate::output::llhls::LlHlsOutput::new`). Defaults to
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
}

/// Default [`Config::playlist_name`] when a config omits the field:
/// [`crate::output::llhls::DEFAULT_PLAYLIST_NAME`], preserving every
/// pre-#663 config's `/media.m3u8` behaviour unchanged.
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
            ingest_connect_timeout_secs: crate::source::DEFAULT_CONNECT_TIMEOUT.as_secs_f64(),
            ingest_read_timeout_secs: crate::source::DEFAULT_READ_TIMEOUT.as_secs_f64(),
            playlist_name: default_playlist_name(),
            output_auth: None,
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
        if self.target_duration_secs <= 0.0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "target_duration_secs",
                reason: "must be positive".into(),
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
        if self.ingest_connect_timeout_secs <= 0.0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "ingest_connect_timeout_secs",
                reason: "must be positive".into(),
            });
        }
        if self.ingest_read_timeout_secs <= 0.0 {
            return Err(MultimuxError::ConfigInvalid {
                field: "ingest_read_timeout_secs",
                reason: "must be positive".into(),
            });
        }
        validate_playlist_name(&self.playlist_name)?;
        if let Some(output_auth) = &self.output_auth {
            output_auth.validate()?;
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.routes {
            if !seen.insert(r.name.as_str()) {
                return Err(MultimuxError::ConfigInvalid {
                    field: "routes",
                    reason: format!("duplicate stream name {:?}", r.name),
                });
            }
            if r.outputs.is_empty() {
                return Err(MultimuxError::ConfigInvalid {
                    field: "routes.outputs",
                    reason: format!("route {:?} has no outputs configured", r.name),
                });
            }
            r.input.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                },
                Route {
                    name: "x".into(),
                    input: InputSpec::Rtsp {
                        url: "rtsp://b".into(),
                        auth: None,
                    },
                    outputs: default_outputs(),
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
}
