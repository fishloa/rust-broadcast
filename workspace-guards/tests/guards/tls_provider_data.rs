//! Per-crate data for the `tls_provider` guard: every workspace member whose
//! manifest declares a rustls-stack dependency (`rustls`, `tokio-rustls`,
//! `hyper-rustls`, `reqwest`). The `table_covers_every_rustls_dependent` test
//! fails if a manifest gains one of these and the table is not updated.

use crate::tls_provider::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-client",
    },
    Crate { name: "multimux" },
    Crate {
        name: "rtsp-runtime",
    },
];
