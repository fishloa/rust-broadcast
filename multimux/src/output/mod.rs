//! The `Output` abstraction: one implementation per delivery protocol
//! (LL-HLS, DASH, LL-DASH, classic TS-HLS) layered over the protocol-neutral
//! [`crate::route::RouteHandle`] (step 5b's replacement for the deleted
//! `hls_runtime::server::MediaStore` — see that module's own docs).
//!
//! Each `Output` renders only its own **manifest** (m3u8 / MPD) — the
//! init/segment/part byte serving is mounted **once per stream** by the
//! origin itself (`crate::origin::resource`), not per-output; see that
//! module's docs for why (the "multi-output nest collision" this split fixes
//! — issue #663 P4). LL-HLS and DASH share that one shared resource route
//! because they are both fMP4/CMAF over the same produced bytes;
//! [`ts_hls::TsHlsOutput`] (issue #887) shares the exact same route, but the
//! bytes it references are classic whole-segment `.ts` instead — the
//! resource route itself is container-agnostic (resolved through the route's
//! `hls_runtime::server::HlsOrigin`, which already dispatches on its own
//! configured `Container`), so no separate resource route is needed for it.
//! [`OutputKind::TsHls`] is mutually exclusive with
//! [`OutputKind::LlHls`]/[`OutputKind::Dash`]/[`OutputKind::LlDash`] on one
//! route (`crate::config::Route::validate_standalone`) — see that check's own
//! doc for why.

pub mod dash;
pub mod ll_dash;
pub mod llhls;
pub mod smooth;
pub mod ts_hls;

use std::sync::Arc;

use axum::Router;

use crate::route::RouteHandle;

/// Which delivery protocol an [`Output`] implements — used for config
/// (`crate::config::Route::outputs`) and diagnostics; never for dispatch
/// (the manifest routes an `Output` mounts are the actual behaviour).
///
/// [`OutputKind::Custom`] (issue #663 external scheme plugin registry) names
/// an external delivery protocol by an opaque `type_tag`, resolved at
/// `crate::origin::serve_with_registry` time via
/// [`crate::registry::SchemeRegistry::output`] — the escape hatch that lets a
/// third-party crate add a new output without editing this crate. Its
/// `params` is a `serde_json::Value`, which is `Clone` but not `Copy`, so this
/// enum can no longer derive `Copy`/`Hash` (a breaking change from the
/// pre-registry `OutputKind`); `PartialEq` (via `serde_json::Value`'s own) is
/// still derived — the runtime admin API's reload diffing
/// (`crate::origin::admin`, issue #749) compares a whole `crate::config::Route`
/// (which embeds `Vec<OutputKind>`) for equality to decide whether a route's
/// config actually changed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum OutputKind {
    /// Low-Latency HLS (`master.m3u8` + `media.m3u8`).
    #[serde(rename = "llhls")]
    LlHls,
    /// MPEG-DASH (`manifest.mpd`).
    #[serde(rename = "dash")]
    Dash,
    /// Low-latency DASH (`manifest-ll.mpd`) — issue #663 P4.2 / #721. True
    /// chunked-transfer LL-DASH: see [`ll_dash`]'s module docs for how a
    /// whole-segment `SegmentTemplate` is served over HTTP chunked
    /// transfer-encoding while the segment is still being produced.
    #[serde(rename = "ll_dash")]
    LlDash,
    /// Microsoft Smooth Streaming (MS-SSTR) — `Manifest` plus fragment
    /// responses from the shared `Trunk`'s segment ring (the same fMP4
    /// bytes every other output shares). Issue #742.
    #[serde(rename = "smooth")]
    Smooth,
    /// Classic MPEG-TS HLS (`master.m3u8` + `media.m3u8` referencing `.ts`
    /// media segments instead of fMP4) — issue #887. Container is a
    /// per-*route* property (`crate::route::RouteHandle::with_container`),
    /// not per-output, so this kind is mutually exclusive with
    /// [`OutputKind::LlHls`]/[`OutputKind::Dash`]/[`OutputKind::LlDash`] on
    /// the same route — see `crate::config::Route::validate_standalone` for
    /// why (one `Trunk` segment ring per program, fMP4 *or* TS, never both).
    #[serde(rename = "ts_hls")]
    TsHls,
    /// External output scheme resolved at runtime via
    /// [`crate::registry::SchemeRegistry`]. `type_tag` selects the registered
    /// factory; `params` is passed opaquely to it. JSON (this variant is not
    /// internally tagged like [`crate::config::InputSpec`], since the other
    /// three variants are plain strings): `{ "custom": { "type_tag": "webrtc",
    /// "params": { ... } } }`.
    #[serde(rename = "custom")]
    Custom {
        /// Selects the registered factory in
        /// [`crate::registry::SchemeRegistry`] that builds this output.
        type_tag: String,
        /// Opaque config passed to the registered factory verbatim.
        #[serde(default)]
        params: serde_json::Value,
    },
}

impl OutputKind {
    /// The spec/field-enum label (workspace #204 convention): a stable,
    /// lowercase token per kind, suitable for logs/config.
    /// [`OutputKind::Custom`] labels itself by its own `type_tag` rather than
    /// a fixed token, which is why this borrows from `self` (`&str`) instead
    /// of returning `&'static str` like most `name()` methods in this
    /// workspace.
    pub fn name(&self) -> &str {
        match self {
            OutputKind::LlHls => "llhls",
            OutputKind::Dash => "dash",
            OutputKind::LlDash => "ll_dash",
            OutputKind::Smooth => "smooth",
            OutputKind::TsHls => "ts_hls",
            OutputKind::Custom { type_tag, .. } => type_tag,
        }
    }

    /// Build the [`Output`] this kind names, using
    /// [`llhls::DEFAULT_PLAYLIST_NAME`] for LL-HLS's media playlist filename.
    /// Use [`Self::build_with_playlist_name`] to serve it under a
    /// configured name instead (`crate::config::Config::playlist_name`).
    ///
    /// # Panics
    ///
    /// Panics if `self` is [`OutputKind::Custom`] — a custom output cannot be
    /// built without a [`crate::registry::SchemeRegistry`]; use
    /// `crate::origin::serve_with_registry`, which resolves it via
    /// `registry.output(type_tag)` instead of calling this method.
    pub fn build(&self) -> Arc<dyn Output> {
        self.build_with_playlist_name(llhls::DEFAULT_PLAYLIST_NAME)
    }

    /// Build the [`Output`] this kind names, serving LL-HLS's media playlist
    /// at `playlist_name` (ignored for [`OutputKind::Dash`]/[`OutputKind::LlDash`],
    /// neither of which has an equivalent configurable filename —
    /// `manifest.mpd`/`manifest-ll.mpd` are fixed).
    ///
    /// # Panics
    ///
    /// Panics if `self` is [`OutputKind::Custom`] — see [`Self::build`].
    pub fn build_with_playlist_name(&self, playlist_name: &str) -> Arc<dyn Output> {
        match self {
            OutputKind::LlHls => Arc::new(llhls::LlHlsOutput::new(playlist_name)),
            OutputKind::Dash => Arc::new(dash::DashOutput),
            OutputKind::LlDash => Arc::new(ll_dash::LlDashOutput),
            OutputKind::Smooth => Arc::new(smooth::SmoothOutput),
            OutputKind::TsHls => Arc::new(ts_hls::TsHlsOutput::new(playlist_name)),
            OutputKind::Custom { .. } => unreachable!(
                "OutputKind::Custom cannot be built without a SchemeRegistry — \
                 crate::origin::serve_with_registry resolves it via \
                 `registry.output(type_tag)` instead of this method"
            ),
        }
    }
}

broadcast_common::impl_spec_display!(OutputKind);

/// One delivery protocol's axum **manifest** routes for a single stream,
/// mounted by the origin under `/{stream}/` alongside every other configured
/// output's manifest routes and the one shared resource route (see this
/// module's docs).
pub trait Output: Send + Sync + 'static {
    /// This output's kind — for diagnostics/config round-tripping only.
    fn kind(&self) -> OutputKind;

    /// Build the axum routes this output serves for one stream's manifest,
    /// sharing the one `route`. The origin merges the returned router with
    /// every other configured output's manifest routes and the shared
    /// resource route, then mounts the whole thing under `/{stream}/`, so
    /// routes here are relative (e.g. `/media.m3u8`, not `/:stream/media.m3u8`)
    /// and must never collide with another enabled output's manifest
    /// filename or with the shared `/:file` catch-all (a bare numeric/opaque
    /// filename would; `master.m3u8`/`media.m3u8`/`manifest.mpd`/
    /// `manifest-ll.mpd` don't).
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_kind_name_and_display_agree() {
        for (kind, label) in [
            (OutputKind::LlHls, "llhls"),
            (OutputKind::Dash, "dash"),
            (OutputKind::LlDash, "ll_dash"),
            (OutputKind::Smooth, "smooth"),
            (OutputKind::TsHls, "ts_hls"),
        ] {
            assert_eq!(kind.name(), label);
            assert_eq!(kind.to_string(), label);
        }
    }

    #[test]
    fn output_kind_serde_round_trips() {
        // `OutputKind` no longer derives `PartialEq` (its `Custom` variant
        // carries a `serde_json::Value`, so instances are compared by
        // `name()` instead of `==` — see the type's doc comment).
        for kind in [
            OutputKind::LlHls,
            OutputKind::Dash,
            OutputKind::LlDash,
            OutputKind::Smooth,
            OutputKind::TsHls,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: OutputKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back.name(), kind.name());
        }
        assert_eq!(
            serde_json::to_string(&OutputKind::LlHls).unwrap(),
            "\"llhls\""
        );
        assert_eq!(
            serde_json::to_string(&OutputKind::TsHls).unwrap(),
            "\"ts_hls\""
        );
    }

    #[test]
    fn output_kind_build_matches_kind() {
        assert!(matches!(
            OutputKind::LlHls.build().kind(),
            OutputKind::LlHls
        ));
        assert!(matches!(OutputKind::Dash.build().kind(), OutputKind::Dash));
        assert!(matches!(
            OutputKind::LlDash.build().kind(),
            OutputKind::LlDash
        ));
        assert!(matches!(
            OutputKind::TsHls.build().kind(),
            OutputKind::TsHls
        ));
    }

    // --- issue #663 external scheme plugin registry: `OutputKind::Custom` ---

    /// `OutputKind::Custom` deserializes with the right `type_tag`/`params`.
    #[test]
    fn output_kind_custom_deserializes_with_type_tag_and_params() {
        let json = r#"{ "custom": { "type_tag": "webrtc", "params": { "k": "v" } } }"#;
        let kind: OutputKind = serde_json::from_str(json).unwrap();
        match &kind {
            OutputKind::Custom { type_tag, params } => {
                assert_eq!(type_tag, "webrtc");
                assert_eq!(params.get("k").and_then(|v| v.as_str()), Some("v"));
            }
            other => panic!("expected OutputKind::Custom, got {other:?}"),
        }
        assert_eq!(kind.name(), "webrtc");
    }

    /// `#[should_panic]`: [`OutputKind::build_with_playlist_name`] cannot
    /// build a [`OutputKind::Custom`] without a
    /// `crate::registry::SchemeRegistry` — documented via this test so a
    /// future refactor that silently returns a bogus `Output` instead of
    /// panicking is caught.
    #[test]
    #[should_panic(expected = "SchemeRegistry")]
    fn output_kind_custom_build_panics() {
        let kind = OutputKind::Custom {
            type_tag: "webrtc".into(),
            params: serde_json::Value::Null,
        };
        let _ = kind.build();
    }
}
