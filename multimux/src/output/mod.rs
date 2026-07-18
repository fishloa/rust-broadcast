//! The `Output` abstraction: one implementation per delivery protocol
//! (LL-HLS, and eventually DASH) layered over the protocol-neutral
//! [`crate::store::MediaStore`].
//!
//! Each `Output` renders its own manifest (m3u8 / MPD) and resolves its own
//! media-segment/part URIs to bytes from the *same* store — the segmenter
//! feeds the store once; every configured output for a stream reuses those
//! bytes rather than re-muxing per output.

pub mod llhls;

use std::sync::Arc;

use axum::Router;

use crate::store::MediaStore;

/// One delivery protocol's axum routes for a single stream, mounted by the
/// origin under `/{stream}/`.
pub trait Output: Send + Sync + 'static {
    /// Build the axum routes this output serves for one stream, sharing the
    /// one `store`. The origin mounts the returned router under `/{stream}/`,
    /// so routes here are relative (e.g. `/media.m3u8`, not `/:stream/media.m3u8`).
    fn router(&self, store: Arc<MediaStore>) -> Router;
}
