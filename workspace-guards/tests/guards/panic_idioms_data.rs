//! Per-crate data for the `panic_idioms` guard. The scope is exactly these
//! crates (parse/serialize/convert libraries that must return a typed error
//! for hostile input); add a line for a new crate of that kind. A justified
//! exception goes in `allow` with its reason.

use crate::panic_idioms::Crate;

const fn plain(name: &'static str) -> Crate {
    Crate { name, allow: &[] }
}

/// `broadcast-client`: a constant-width, compile-time timescale; the match arm a
/// non-zero constant can never take.
const BROADCAST_CLIENT: Crate = Crate {
    name: "broadcast-client",
    allow: &[
        (
            "src/buffer.rs",
            "None => core::unreachable!(),",
            "Timescale::new of a non-zero constant, checked at compile time",
        ),
        (
            "src/buffer.rs",
            "Err(_) => core::unreachable!(),",
            "Clock27::from_27mhz of a count already reduced modulo its wrap, so the range check cannot fail and there is no case for a caller to handle",
        ),
    ],
};

pub const TABLE: &[Crate] = &[
    BROADCAST_CLIENT,
    plain("broadcast-smooth"),
    plain("smooth-runtime"),
    plain("caption-convert"),
    plain("cc-data"),
    plain("dvb-subtitle"),
    plain("dvb-vbi"),
    plain("playout-runtime"),
    plain("scte104"),
    plain("scte35-splice"),
    plain("ssai-runtime"),
    plain("timed-metadata"),
];
