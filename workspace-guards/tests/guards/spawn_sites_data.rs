//! Per-crate data for the `spawn_sites` guard: the runtime crates whose tokio
//! adapters spawn tasks (or, with an empty list, are asserted to spawn none).
//! Every entry is line-pinned with its owner and cancel path. Add a line for a
//! new runtime crate.

use crate::spawn_sites::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "dash-runtime",
        sites: &[],
    },
    Crate {
        name: "hls-runtime",
        sites: &[],
    },
    Crate {
        name: "rtmp-runtime",
        sites: &[],
    },
    Crate {
        name: "rtsp-runtime",
        sites: &[],
    },
    Crate {
        name: "smooth-runtime",
        sites: &[],
    },
    Crate {
        name: "srt-runtime",
        sites: &[
            (
                "src/io.rs",
                983,
                "connection driver task; its JoinHandle is stored in SrtSocket and aborted in Drop",
            ),
            (
                "src/io.rs",
                2116,
                "listener-side forwarder; owned by a TaskGuard that aborts it on drop",
            ),
            (
                "src/io.rs",
                2424,
                "listener routing pump on the listener's TaskTracker; cancelled with the listener's CancellationToken when the handle and every accepted socket drop",
            ),
            (
                "src/io.rs",
                2432,
                "listener core on the listener's TaskTracker; cancelled with the listener's CancellationToken when the handle and every accepted socket drop",
            ),
            (
                "src/io.rs",
                3528,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                3801,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                4541,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                4746,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                5034,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
        ],
    },
    Crate {
        name: "webrtc-runtime",
        sites: &[],
    },
];
