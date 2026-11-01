//! Per-crate data for the three de-hand-roll tripwire shapes. Each crate keeps
//! the shape (scanner variant) it already used; patterns/needles and every
//! allowlist entry with its reason are carried over verbatim. Add a line for a
//! new protocol/runtime crate to the shape that fits it.
//!
//! Not in any table, by design (their per-crate guards carry extra checks or a
//! different scanner and stay in the crate): `broadcast-client`, `dash-runtime`,
//! `dvb-stream`, `multimux`, `rtmp-runtime`, `rtsp-runtime`.

use crate::handroll_dehandroll::Crate as Dehandroll;
use crate::handroll_lines::Crate as Lines;
use crate::handroll_needles::Crate as Needles;

/// Shape 1 (was `tests/dehandroll_guard.rs`).
pub const DEHANDROLL: &[Dehandroll] = &[
    Dehandroll {
        name: "broadcast-auth",
        allow: &[],
    },
    Dehandroll {
        name: "broadcast-common",
        allow: &[],
    },
    Dehandroll {
        name: "scte35-splice",
        allow: &[],
    },
    Dehandroll {
        name: "timed-metadata",
        allow: &[],
    },
    Dehandroll {
        name: "transmux",
        allow: &[],
    },
];

/// Shape 2 (was `tests/no_handroll_guard.rs` with `PATTERNS`).
pub const LINE_PATTERNS: &[Lines] = &[
    // Tripwire (de-hand-roll W1-P, spec §5): fails if `src/` (outside
    // `#[cfg(test)] mod` bodies) reintroduces a hand-rolled pattern the
    // migration removed — poison-recovering std locks, sleep-based waits, HTTP framing literals, calendar maths, hand-built SDP.
    // **This is a lexical tripwire, not a proof**; a renamed helper or a split
    // literal evades it. Code review is the real control. Every allowlist entry
    // carries a reason.
    Lines {
        name: "media-plane",
        patterns: &[
            ("\"HTTP/1.", "hand-built HTTP status line"),
            ("\"\\r\\n\\r\\n\"", "HTTP head terminator literal"),
            ("find(\"://\")", "hand-rolled URL scheme split"),
            ("://\")", "hand-rolled URL scheme prefix strip"),
            ("\"a=", "hand-built SDP attribute line"),
            ("\"m=", "hand-built SDP media line"),
            ("\"v=0", "hand-built SDP version line"),
            ("civil_from_days", "hand-rolled calendar maths (use jiff)"),
            ("days_from_civil", "hand-rolled calendar maths (use jiff)"),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
                "base64 alphabet (use base64)",
            ),
            ("thread::sleep", "sleep-based wait"),
            ("sleep(", "sleep-based wait"),
            ("std::sync::Mutex", "std Mutex poisons; use parking_lot"),
            (
                "std::sync::Condvar",
                "pair the Condvar with the parking_lot Mutex",
            ),
            (
                "PoisonError",
                "poison-recovery wrapper (parking_lot does not poison)",
            ),
            (".lock().unwrap()", "poisoning lock unwrap"),
            (".lock().expect(", "poisoning lock expect"),
        ],
        allow: &[],
    },
    // Tripwire (de-hand-roll W1-P, spec §5): fails if `src/` (outside
    // `#[cfg(test)] mod` bodies) reintroduces a hand-rolled pattern the
    // migration removed — `libc::poll`, sleep-based waits, hand-built text framing.
    // **This is a lexical tripwire, not a proof**; a renamed helper or a split
    // literal evades it. Code review is the real control. Every allowlist entry
    // carries a reason.
    Lines {
        name: "dvb-ci-runtime",
        patterns: &[
            ("\"HTTP/1.", "hand-built HTTP status line"),
            ("\"\\r\\n\\r\\n\"", "HTTP head terminator literal"),
            ("find(\"://\")", "hand-rolled URL scheme split"),
            ("://\")", "hand-rolled URL scheme prefix strip"),
            ("\"a=", "hand-built SDP attribute line"),
            ("\"m=", "hand-built SDP media line"),
            ("\"v=0", "hand-built SDP version line"),
            ("civil_from_days", "hand-rolled calendar maths (use jiff)"),
            ("days_from_civil", "hand-rolled calendar maths (use jiff)"),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
                "base64 alphabet (use base64)",
            ),
            ("thread::sleep", "sleep-based wait"),
            ("sleep(", "sleep-based wait"),
            (
                "libc::poll",
                "readiness polling goes through `rustix::event::poll`",
            ),
            (
                "libc::pollfd",
                "readiness polling goes through `rustix::event::poll`",
            ),
            (
                "POLLIN",
                "readiness polling goes through `rustix::event::poll`",
            ),
        ],
        allow: &[
            (
                "src/linux.rs",
                "thread::sleep",
                "RESET_SETTLE",
                "CAM CA_RESET settle delay (RESET_SETTLE): a fixed hardware reset time in synchronous device code, not a wait on a condition",
            ),
            (
                "src/linux.rs",
                "sleep(",
                "RESET_SETTLE",
                "CAM CA_RESET settle delay (RESET_SETTLE): a fixed hardware reset time in synchronous device code, not a wait on a condition",
            ),
        ],
    },
    // Tripwire (de-hand-roll W1-P, spec §5): fails if `src/` (outside
    // `#[cfg(test)] mod` bodies) reintroduces a hand-rolled pattern the
    // migration removed — hand-built HTTP/exposition text, sleep-based waits, raw `UdpSocket::bind`, the deleted `render_prometheus`.
    // **This is a lexical tripwire, not a proof**; a renamed helper or a split
    // literal evades it. Code review is the real control. Every allowlist entry
    // carries a reason.
    Lines {
        name: "media-doctor",
        patterns: &[
            ("\"HTTP/1.", "hand-built HTTP status line"),
            ("\"\\r\\n\\r\\n\"", "HTTP head terminator literal"),
            ("find(\"://\")", "hand-rolled URL scheme split"),
            ("://\")", "hand-rolled URL scheme prefix strip"),
            ("\"a=", "hand-built SDP attribute line"),
            ("\"m=", "hand-built SDP media line"),
            ("\"v=0", "hand-built SDP version line"),
            ("civil_from_days", "hand-rolled calendar maths (use jiff)"),
            ("days_from_civil", "hand-rolled calendar maths (use jiff)"),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
                "base64 alphabet (use base64)",
            ),
            ("thread::sleep", "sleep-based wait"),
            ("sleep(", "sleep-based wait"),
            (
                "UdpSocket::bind(",
                "UDP binds go through `udp::bind_udp` (socket2)",
            ),
            (
                "render_prometheus",
                "the hand-written Prometheus renderer was deleted (use `render_metrics`)",
            ),
        ],
        allow: &[(
            "src/metrics_server.rs",
            "sleep(",
            "ACCEPT_ERROR_BACKOFF",
            "accept-error backoff: EMFILE/ECONNABORTED must not spin the accept loop; not a wait for a condition",
        )],
    },
];

/// Shape 3 (was `tests/no_handroll_guard.rs` with `NEEDLES`).
pub const NEEDLE_CRATES: &[Needles] = &[
    Needles {
        name: "hls-runtime",
        allow: &[(
            "client/tokio_client.rs",
            "tokio::time::sleep(",
            "backon-scheduled retry delay and WaitMs hint, both raced against the CancellationToken in sleep_or_cancelled",
        )],
    },
    Needles {
        name: "srt-runtime",
        allow: &[],
    },
    Needles {
        name: "tshttp-runtime",
        allow: &[(
            "client/tokio_client.rs",
            "tokio::time::sleep(",
            "backon-scheduled retry delay and WaitMs hint, both raced against the CancellationToken in sleep_or_cancelled",
        )],
    },
    Needles {
        name: "webrtc-runtime",
        allow: &[],
    },
];
