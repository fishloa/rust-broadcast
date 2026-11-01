//! Process-wide build-once cache for read-only test rigs: a fixture demux, a
//! segmenter pump or a validator baseline is computed by the first test that
//! asks and handed to every later one. Tests that mutate rig state must not
//! use it.

#![allow(dead_code)]

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

type Cell = OnceLock<Box<dyn Any + Send + Sync>>;

/// Builds `make()` once per `key` and returns the shared value. The map lock is
/// released before `make` runs, so a builder may itself call `shared`.
pub fn shared<T: Send + Sync + 'static>(key: &str, make: impl FnOnce() -> T) -> &'static T {
    static CELLS: Mutex<Option<HashMap<String, &'static Cell>>> = Mutex::new(None);
    let cell: &'static Cell = {
        let mut guard = CELLS.lock().unwrap_or_else(|p| p.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        match map.get(key) {
            Some(cell) => cell,
            None => {
                let cell: &'static Cell = Box::leak(Box::new(OnceLock::new()));
                map.insert(key.to_string(), cell);
                cell
            }
        }
    };
    cell.get_or_init(|| Box::new(make()))
        .downcast_ref::<T>()
        .expect("shared cell holds the type it was keyed with")
}

/// Runs `fut` to completion on its own thread and current-thread runtime, so a
/// builder inside a `#[tokio::test]` can issue requests without nesting
/// runtimes.
pub fn run_async<T: Send>(fut: impl std::future::Future<Output = T> + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("run_async runtime")
                    .block_on(fut)
            })
            .join()
            .expect("run_async thread")
    })
}

/// Issues the first request to every playlist a rig may serve, so the route's
/// lazily populated playlist state exists before the rig is shared: two tests
/// making that first request concurrently see an empty playlist on one of them.
pub fn warm_up(app: &axum::Router, stream: &str) {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    const PLAYLISTS: &[&str] = &[
        "master.m3u8",
        "media.m3u8",
        "iframe.m3u8",
        "subs-0.m3u8",
        "subs-1.m3u8",
        "audio/a0.m3u8",
        "audio/a1.m3u8",
        "audio/a2.m3u8",
    ];
    run_async(async {
        for path in PLAYLISTS {
            let request = Request::builder()
                .uri(format!("/{stream}/{path}"))
                .body(Body::empty())
                .expect("warm-up request");
            let _ = app.clone().oneshot(request).await;
        }
    });
}
