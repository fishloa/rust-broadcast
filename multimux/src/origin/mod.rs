//! HTTP origin server for stream delivery.
//!
//! Wires a per-stream [`crate::store::MediaStore`] to the axum sub-routers of
//! each stream's configured [`crate::output::Output`]s (LL-HLS today; DASH in
//! future), mounting each output's routes under `/{stream}/`. See
//! [`crate::output::llhls`] for the LL-HLS output itself (playlists, byte
//! ranges, bounded blocking playlist reload — RFC 8216bis §6.2.5.2).

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;

use crate::output::Output;
use crate::output::llhls::LlHlsOutput;
use crate::store::MediaStore;

/// One stream's shared store plus the `Output`s configured to serve it.
pub type StreamRoute = (Arc<MediaStore>, Vec<Arc<dyn Output>>);

/// Shared HTTP origin state: one [`MediaStore`] plus its configured
/// [`Output`]s per served stream name, keyed by the stream name used in the
/// URL path (`/:stream/...`).
pub struct AppState {
    /// Served stream name -> its rolling in-RAM store and the outputs
    /// serving it.
    pub streams: HashMap<String, StreamRoute>,
}

/// Build the axum router serving `state`'s streams: for each stream, every
/// configured `Output`'s router is merged (`nest`ed) under `/{stream}/`, all
/// sharing that stream's one [`MediaStore`]. A request for a stream name not
/// present in `state.streams` matches no nest and 404s, same as an unknown
/// filename within a known stream 404s inside the output's own router.
pub fn router(state: Arc<AppState>) -> Router {
    let mut router = Router::new();
    for (name, (store, outputs)) in &state.streams {
        for output in outputs {
            router = router.nest(&format!("/{name}"), output.router(store.clone()));
        }
    }
    router
}

/// Run the multimux origin: one [`MediaStore`] + one spawned pipeline task
/// per `config.routes` entry, then bind `config.bind` and serve them all
/// under [`router`]. Each route is served by a single [`LlHlsOutput`] (the
/// default — and today the only — output wiring).
///
/// Each route's pipeline task independently connects its [`crate::source::rtsp::RtspSource`],
/// runs it through [`crate::pipeline::run_pipeline`], and — on either a connect
/// failure or a pipeline error — logs to stderr and lets only that route's
/// task end; a single bad source never brings the server (or any other route)
/// down.
///
/// Returns only on a bind failure or if the HTTP server itself stops (e.g. a
/// fatal accept-loop I/O error); the per-route ingest tasks run detached.
pub async fn serve(config: crate::config::Config) -> crate::Result<()> {
    config.validate()?;

    let mut streams: HashMap<String, StreamRoute> = HashMap::new();
    let target_duration_secs = config.target_duration_secs;
    let part_target_ms = config.part_target_ms;

    for route in &config.routes {
        let store = Arc::new(MediaStore::new(
            target_duration_secs,
            part_target_ms,
            config.window_segments,
        ));
        let outputs: Vec<Arc<dyn Output>> = vec![Arc::new(LlHlsOutput)];
        streams.insert(route.name.clone(), (store.clone(), outputs));

        let name = route.name.clone();
        let rtsp_url = route.rtsp_url.clone();
        tokio::spawn(async move {
            let source = crate::source::rtsp::RtspSource::new(name.clone(), rtsp_url);
            match source.connect().await {
                Ok(session) => {
                    if let Err(e) = crate::pipeline::run_pipeline(
                        store,
                        target_duration_secs,
                        part_target_ms,
                        session,
                    )
                    .await
                    {
                        eprintln!("multimux: route {name:?} pipeline stopped: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("multimux: route {name:?} failed to connect: {e}");
                }
            }
        });
    }

    let state = Arc::new(AppState { streams });
    let listener = tokio::net::TcpListener::bind(config.bind.as_str()).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MediaStore;
    use tower::ServiceExt;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_init(vec![0xAA; 4]);
        let mut streams = HashMap::new();
        streams.insert(
            "cam1".to_string(),
            (store, vec![Arc::new(LlHlsOutput) as Arc<dyn Output>]),
        );
        Arc::new(AppState { streams })
    }

    fn get(uri: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    /// Sanity-checks real axum route dispatch (not just the handler
    /// functions in isolation): the static `master.m3u8`/`media.m3u8` routes
    /// must win over the `:file` catch-all registered for the same
    /// `/:stream/*` prefix, and the catch-all must still serve dynamic
    /// filenames.
    #[tokio::test]
    async fn router_dispatches_static_routes_over_catch_all() {
        let app = router(make_state());

        let resp = app.clone().oneshot(get("/cam1/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .contains("#EXT-X-STREAM-INF")
        );

        let resp = app.clone().oneshot(get("/cam1/init-1.mp4")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(bytes.to_vec(), vec![0xAA; 4]);

        let resp = app.oneshot(get("/cam1/no-such-file.bin")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    // --- "unknown stream" 404 tests, moved from `output::llhls`'s handlers:
    // a stream name absent from `state.streams` matches no nested output
    // router at all, so every route under it 404s — same externally-visible
    // behaviour as the pre-refactor per-handler `contains_key` check, now
    // proven at the router-dispatch level since the handlers themselves no
    // longer know about stream names. ---

    #[tokio::test]
    async fn master_playlist_unknown_stream_404() {
        let app = router(make_state());
        let resp = app.oneshot(get("/nope/master.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_playlist_unknown_stream_404() {
        let app = router(make_state());
        let resp = app.oneshot(get("/nope/media.m3u8")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dynamic_file_unknown_stream_404() {
        let app = router(make_state());
        let resp = app.oneshot(get("/nope/init-1.mp4")).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
