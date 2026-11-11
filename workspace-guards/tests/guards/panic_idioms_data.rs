//! Per-crate data for the `panic_idioms` guard. The scope is exactly these
//! crates (parse/serialize/convert libraries that must return a typed error
//! for hostile input); add a line for a new crate of that kind. A justified
//! exception goes in `allow` with its reason.

use crate::panic_idioms::Crate;

const fn plain(name: &'static str) -> Crate {
    Crate { name, allow: &[] }
}

pub const TABLE: &[Crate] = &[
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
