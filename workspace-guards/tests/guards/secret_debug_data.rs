//! Per-crate data for the `secret_debug` guard. Add a line for a crate that
//! holds secrets (passwords, tokens, stream keys, passphrases, ICE credentials).

use crate::secret_debug::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-client",
        allow: &[(
            "source/hls.rs",
            "BodyLimits",
            "`key` is the byte cap on an HLS key fetch, not key material",
        )],
        ct_compare: false,
    },
    Crate {
        name: "broadcast-auth",
        allow: &[],
        ct_compare: true,
    },
    Crate {
        name: "dvb-ci-runtime",
        allow: &[],
        ct_compare: false,
    },
    Crate {
        name: "dvb-csa",
        allow: &[],
        ct_compare: false,
    },
    Crate {
        name: "hls-runtime",
        allow: &[],
        ct_compare: true,
    },
    Crate {
        name: "media-plane",
        allow: &[],
        ct_compare: false,
    },
    Crate {
        name: "multimux",
        allow: &[],
        ct_compare: false,
    },
    Crate {
        name: "rist-runtime",
        allow: &[],
        ct_compare: false,
    },
    Crate {
        name: "rtmp-runtime",
        allow: &[],
        ct_compare: true,
    },
    Crate {
        name: "rtsp-runtime",
        allow: &[(
            "src/client.rs",
            "ClientSession",
            "its `credentials` is a broadcast_auth::Credentials, whose Debug redacts the secret",
        )],
        ct_compare: true,
    },
    Crate {
        name: "srt-runtime",
        allow: &[(
            "src/handshake_sm.rs",
            "NegotiatedParams",
            "its `sek` is a SecretBytes, whose Debug prints only the length",
        )],
        ct_compare: true,
    },
    Crate {
        name: "transmux",
        allow: &[(
            "src/klv.rs",
            "KlvItem",
            "`key` is the SMPTE KLV universal label, a public identifier",
        )],
        ct_compare: true,
    },
    Crate {
        name: "webrtc-runtime",
        allow: &[],
        ct_compare: true,
    },
];
