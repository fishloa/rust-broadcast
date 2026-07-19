//! The `Output` abstraction: one implementation per delivery protocol
//! (LL-HLS, DASH) layered over the protocol-neutral
//! [`crate::store::MediaStore`].
//!
//! Each `Output` renders only its own **manifest** (m3u8 / MPD) — the
//! init/segment/part byte serving both protocols reference is identical
//! (LL-HLS and DASH are both fMP4/CMAF over the same produced bytes) and is
//! mounted **once per stream** by the origin itself
//! (`crate::origin::resource`), not per-output; see that module's docs for
//! why (the "multi-output nest collision" this split fixes — issue #663 P4).

pub mod dash;
pub mod llhls;

use std::sync::Arc;

use axum::Router;

use crate::store::MediaStore;

/// Which delivery protocol an [`Output`] implements — used for config
/// (`crate::config::Route::outputs`) and diagnostics; never for dispatch
/// (the manifest routes an `Output` mounts are the actual behaviour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub enum OutputKind {
    /// Low-Latency HLS (`master.m3u8` + `media.m3u8`).
    #[serde(rename = "llhls")]
    LlHls,
    /// MPEG-DASH (`manifest.mpd`).
    #[serde(rename = "dash")]
    Dash,
}

impl OutputKind {
    /// The spec/field-enum label (workspace #204 convention): a stable,
    /// lowercase token per kind, suitable for logs/config.
    pub fn name(&self) -> &'static str {
        match self {
            OutputKind::LlHls => "llhls",
            OutputKind::Dash => "dash",
        }
    }

    /// Build the [`Output`] this kind names.
    pub fn build(&self) -> Arc<dyn Output> {
        match self {
            OutputKind::LlHls => Arc::new(llhls::LlHlsOutput),
            OutputKind::Dash => Arc::new(dash::DashOutput),
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
    /// sharing the one `store`. The origin merges the returned router with
    /// every other configured output's manifest routes and the shared
    /// resource route, then mounts the whole thing under `/{stream}/`, so
    /// routes here are relative (e.g. `/media.m3u8`, not `/:stream/media.m3u8`)
    /// and must never collide with another enabled output's manifest
    /// filename or with the shared `/:file` catch-all (a bare numeric/opaque
    /// filename would; `master.m3u8`/`media.m3u8`/`manifest.mpd` don't).
    fn manifest_routes(&self, store: Arc<MediaStore>) -> Router;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_kind_name_and_display_agree() {
        for (kind, label) in [(OutputKind::LlHls, "llhls"), (OutputKind::Dash, "dash")] {
            assert_eq!(kind.name(), label);
            assert_eq!(kind.to_string(), label);
        }
    }

    #[test]
    fn output_kind_serde_round_trips() {
        for kind in [OutputKind::LlHls, OutputKind::Dash] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: OutputKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
        assert_eq!(
            serde_json::to_string(&OutputKind::LlHls).unwrap(),
            "\"llhls\""
        );
    }

    #[test]
    fn output_kind_build_matches_kind() {
        assert_eq!(OutputKind::LlHls.build().kind(), OutputKind::LlHls);
        assert_eq!(OutputKind::Dash.build().kind(), OutputKind::Dash);
    }
}
