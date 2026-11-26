//! Data for the `dep_policy` guard: the owner's rule (2026-11-22) "one
//! external crate per purpose across the suite; never chrono next to jiff".
//!
//! Adding an external dependency means adding it to exactly one `PURPOSES`
//! entry, with reviewer sign-off. A purpose marked `single` admits one crate,
//! so a second crate for it is a policy change, not a table edit.
//!
//! `hex` and `base64` are purposes, not bans: each is the one crate for its
//! encoding. Routing the remaining direct call sites through
//! `broadcast_common::{hex, base64}` is code locality, tracked by #3957, and is
//! not a dependency choice this guard can express.

use crate::dep_policy::{Banned, Exempt, Purpose};

pub const WORKSPACES: &[&str] = &[
    ".",
    "fuzz",
    "demo",
    "tools/differential-sweep",
    "bindings/python",
    "bindings/transmux-py",
];

pub const WORKSPACES_EXEMPT: &[(&str, &str)] = &[(
    "spikes/trunk-bench",
    "throwaway benchmark spike outside every workspace; never built by CI or published",
)];

const fn one(name: &'static str, crates: &'static [&'static str]) -> Purpose {
    Purpose {
        name,
        single: true,
        crates,
    }
}

const fn family(name: &'static str, crates: &'static [&'static str]) -> Purpose {
    Purpose {
        name,
        single: false,
        crates,
    }
}

pub const PURPOSES: &[Purpose] = &[
    one("date/time", &["jiff"]),
    one("XML", &["quick-xml"]),
    one("MISB ST 0601 value decoding", &["motion-imagery-metadata"]),
    one("error derive", &["thiserror"]),
    one("logging/tracing", &["tracing"]),
    one("tracing subscriber", &["tracing-subscriber"]),
    one("metrics facade", &["metrics"]),
    one("metrics exporter", &["metrics-exporter-prometheus"]),
    one("OS randomness", &["getrandom"]),
    one("HTTP client", &["reqwest"]),
    one("HTTP server framework", &["axum"]),
    one("CLI parsing", &["clap"]),
    one("benchmarks", &["criterion"]),
    one("cache", &["lru"]),
    one("TLS roots", &["rustls-platform-verifier"]),
    one("async runtime", &["tokio"]),
    one("locks", &["parking_lot"]),
    one("JSON", &["serde_json"]),
    one("serialization framework", &["serde"]),
    one("type-erased serde", &["erased-serde"]),
    one("hex codec", &["hex"]),
    one("base64 codec", &["base64"]),
    one("URL", &["url"]),
    one("percent-encoding", &["percent-encoding"]),
    one("HTTP types", &["http"]),
    one("typed HTTP headers", &["headers"]),
    one("HTTP auth challenges", &["http-auth"]),
    one("byte buffers", &["bytes"]),
    one("tokio codecs", &["tokio-util"]),
    one("socket options", &["socket2"]),
    one("OS syscalls", &["rustix"]),
    one("compression", &["flate2"]),
    one("legacy charsets", &["encoding_rs"]),
    one("language codes", &["iso-639"]),
    one("self-referential borrows", &["yoke"]),
    one("no_std float math", &["libm"]),
    one("WAV test input", &["hound"]),
    one("fuzzing", &["libfuzzer-sys"]),
    one("Python FFI", &["pyo3"]),
    one("serde to Python", &["pythonize"]),
    one("WASM bindings", &["wasm-bindgen"]),
    one("constant-time compare", &["subtle"]),
    one("secret zeroization", &["zeroize"]),
    one("byte search", &["memchr"]),
    one("subprocess timeout", &["wait-timeout"]),
    one("event notification", &["event-listener"]),
    one("atomic swap", &["arc-swap"]),
    one("async trait", &["async-trait"]),
    one("RTSP message codec", &["rtsp-types"]),
    one("SDP codec", &["sdp-types"]),
    one("test certificates", &["rcgen"]),
    family(
        "futures utilities (one family)",
        &["futures-core", "futures-util"],
    ),
    family(
        "hyper/tower HTTP stack (one family)",
        &[
            "hyper",
            "hyper-util",
            "http-body",
            "http-body-util",
            "tower",
            "tower-http",
        ],
    ),
    family(
        "rustls TLS (one family)",
        &["rustls", "tokio-rustls", "rustls-pki-types"],
    ),
    family("HTTP cookies (one family)", &["cookie", "cookie_store"]),
    family(
        "WebRTC transport (one family)",
        &[
            "rtc-dtls",
            "rtc-ice",
            "rtc-shared",
            "rtc-srtp",
            "rtc-stun",
            "sansio",
        ],
    ),
    family(
        "RustCrypto primitives (one per algorithm)",
        &[
            "aes", "aes-kw", "cbc", "cipher", "ctr", "hmac", "md-5", "pbkdf2", "sha1", "sha2",
        ],
    ),
];

const fn ban(krate: &'static str, instead: &'static str) -> Banned {
    Banned { krate, instead }
}

pub const BANNED: &[Banned] = &[
    ban("chrono", "jiff"),
    ban("time", "jiff"),
    ban("humantime", "jiff"),
    ban("httpdate", "jiff (or typed `headers`)"),
    ban("anyhow", "typed thiserror errors"),
    ban("eyre", "typed thiserror errors"),
    ban("once_cell", "std::sync::{LazyLock, OnceLock}"),
    ban("lazy_static", "std::sync::LazyLock"),
    ban("log", "tracing"),
    ban("env_logger", "tracing-subscriber"),
    ban("prometheus", "metrics + metrics-exporter-prometheus"),
    ban("simd-json", "serde_json"),
    ban("roxmltree", "quick-xml via broadcast_common::xml"),
    ban("xml-rs", "quick-xml via broadcast_common::xml"),
    ban("xmltree", "quick-xml via broadcast_common::xml"),
    ban("data-encoding", "broadcast_common::{hex, base64}"),
    ban("faster-hex", "broadcast_common::hex"),
    ban("urlencoding", "percent-encoding"),
    ban("form_urlencoded", "url::form_urlencoded"),
    ban("crc*", "broadcast_common::crc32_mpeg2"),
    ban("rand", "getrandom"),
    ban("fastrand", "getrandom"),
    ban("ureq", "reqwest"),
    ban("actix-web", "axum"),
    ban("warp", "axum"),
    ban("async-std", "tokio"),
    ban("smol", "tokio"),
    ban("futures", "futures-core / futures-util"),
    ban("spin", "parking_lot"),
    ban("native-tls", "rustls"),
    ban("openssl", "rustls"),
    ban("aws-lc-rs", "rustls with the ring provider"),
    ban(
        "ring",
        "rustls with the ring provider (never a direct dependency)",
    ),
    ban("webpki-roots", "rustls-platform-verifier"),
    ban("libc", "rustix"),
    ban("nix", "rustix"),
    ban("mio", "tokio"),
    ban("md5", "md-5"),
    ban("sha1_smol", "sha1"),
    ban("num_enum", "a hand-written TryFrom"),
    ban("strum", "a hand-written name() + impl_spec_display!"),
    ban("derive_more", "thiserror or hand-written impls"),
    ban("structopt", "clap"),
    ban("argh", "clap"),
    ban("divan", "criterion"),
    ban("moka", "lru"),
    ban("backon", "the runtime crates' sans-IO retry/backoff"),
    ban("backoff", "the runtime crates' sans-IO retry/backoff"),
    ban(
        "ffmpeg-sys-next",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "ffmpeg-next",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "ffmpeg-sys",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "ffmpeg",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "rsmpeg",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "rusty_ffmpeg",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "ac-ffmpeg",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "ffmpeg-the-third",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "ffmpeg-sys-the-third",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
    ban(
        "broadcast-codec-libav",
        "FFmpeg binding: links LGPL/GPL code; allowed only in a side workspace adapter, never in the main graph",
    ),
];

const fn exempt(member: &'static str, krate: &'static str, reason: &'static str) -> Exempt {
    Exempt {
        member,
        krate,
        reason,
    }
}

pub const EXEMPT: &[Exempt] = &[exempt(
    "atsc3",
    "roxmltree",
    "atsc3 is abandoned and permanently unpublished (owner decision)",
)];
