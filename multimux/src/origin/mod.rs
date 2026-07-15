//! HTTP origin server for LL-HLS delivery.
//!
//! Wires a per-stream [`crate::store::StreamStore`] to an [`axum`] router
//! serving the master/media playlists and the init/segment/part byte ranges
//! LL-HLS clients fetch, including bounded blocking playlist reload
//! (RFC 8216bis §6.2.5.2). See [`handlers`] for the request handlers
//! themselves.

pub mod handlers;

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::store::StreamStore;

/// Shared HTTP origin state: one [`StreamStore`] per served stream name,
/// keyed by the stream name used in the URL path (`/:stream/...`).
pub struct AppState {
    /// Served stream name -> its rolling in-RAM LL-HLS store.
    pub streams: HashMap<String, Arc<StreamStore>>,
}

/// Build the axum router serving `state`'s streams.
///
/// Routes:
/// - `GET /:stream/master.m3u8` — minimal single-variant master playlist.
/// - `GET /:stream/media.m3u8` — LL-HLS media playlist, blocking-reload aware.
/// - `GET /:stream/:file` — catch-all serving `init-*.mp4`/`seg-*.m4s`/
///   `part-*.m4s` byte ranges (see [`handlers::dynamic_file`] for why a
///   single catch-all is used instead of per-filename routes).
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/:stream/master.m3u8", get(handlers::master_playlist))
        .route("/:stream/media.m3u8", get(handlers::media_playlist))
        .route("/:stream/:file", get(handlers::dynamic_file))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StreamStore;
    use tower::ServiceExt;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(StreamStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert("cam1".to_string(), store);
        Arc::new(AppState { streams })
    }

    /// Sanity-checks real axum route dispatch (not just the handler
    /// functions in isolation): the static `master.m3u8`/`media.m3u8` routes
    /// must win over the `:file` catch-all registered for the same
    /// `/:stream/*` prefix, and the catch-all must still serve dynamic
    /// filenames.
    #[tokio::test]
    async fn router_dispatches_static_routes_over_catch_all() {
        let app = router(make_state());

        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/cam1/master.m3u8")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .contains("#EXT-X-STREAM-INF")
        );

        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/cam1/init-1.mp4")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(bytes.to_vec(), vec![0xAA; 4]);

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/cam1/no-such-file.bin")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
