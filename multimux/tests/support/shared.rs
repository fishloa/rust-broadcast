//! Process-wide build-once cache for read-only test rigs: a fixture demux, a
//! segmenter pump or a validator baseline is computed by the first test that
//! asks and handed to every later one. Tests that mutate rig state must not
//! use it.

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
        *map.entry(key.to_string())
            .or_insert_with(|| Box::leak(Box::new(OnceLock::new())))
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
