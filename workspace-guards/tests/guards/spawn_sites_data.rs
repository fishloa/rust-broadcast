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
                "bind",
                1,
                "listener routing pump on the listener's TaskTracker; cancelled with the listener's CancellationToken when the handle and every accepted socket drop",
            ),
            (
                "src/io.rs",
                "bind",
                2,
                "listener core on the listener's TaskTracker; cancelled with the listener's CancellationToken when the handle and every accepted socket drop",
            ),
        ],
    },
    Crate {
        name: "webrtc-runtime",
        sites: &[],
    },
];
