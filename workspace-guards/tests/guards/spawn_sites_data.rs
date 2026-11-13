//! Per-crate data for the `spawn_sites` guard: the runtime crates whose tokio
//! adapters spawn tasks (or, with an empty list, are asserted to spawn none).
//! Every entry is keyed by file, enclosing fn and spawn ordinal within the fn, with its owner and cancel path. Add a line for a
//! new runtime crate.

use crate::spawn_sites::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-client",
        sites: &[],
    },
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
                "spawn",
                1,
                "connection driver task; its JoinHandle is stored in SrtSocket and aborted in Drop",
            ),
            (
                "src/io.rs",
                "spawn_dedicated_socket_forwarder",
                1,
                "listener-side forwarder; owned by a TaskGuard that aborts it on drop",
            ),
            (
                "src/io.rs",
                "bind_with",
                1,
                "listener routing pump on the listener's TaskTracker; cancelled with the listener's CancellationToken when the handle and every accepted socket drop",
            ),
            (
                "src/io.rs",
                "bind_with",
                2,
                "listener core on the listener's TaskTracker; cancelled with the listener's CancellationToken when the handle and every accepted socket drop",
            ),
            (
                "src/io.rs",
                "the_run_loop_wakes_at_deadlines_not_on_a_fixed_two_ms_tick",
                1,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                "a_pending_handshake_is_retransmitted_by_the_listener_tick_alone",
                1,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                "dropping_an_accepted_socket_removes_its_route_entry",
                1,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                "run_stops_taking_app_payloads_once_the_window_is_full",
                1,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
            (
                "src/io.rs",
                "a_driver_ending_because_its_handle_is_gone_sends_shutdown",
                1,
                "test-only: inside `#[cfg(all(test, feature = \"tokio\"))] mod adapter_tests`, which the shared lexer cannot yet blank (a feature term); the test owns and awaits or aborts the task",
            ),
        ],
    },
    Crate {
        name: "webrtc-runtime",
        sites: &[],
    },
];
