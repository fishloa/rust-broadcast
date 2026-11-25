//! Per-crate data for the `panic_idioms` guard. The scope is exactly these
//! crates (parse/serialize/convert libraries that must return a typed error
//! for hostile input); add a line for a new crate of that kind. A justified
//! exception goes in `allow` with its reason.

use crate::panic_idioms::Crate;

const fn plain(name: &'static str) -> Crate {
    Crate { name, allow: &[] }
}

pub const TABLE: &[Crate] = &[
    plain("broadcast-matroska"),
    plain("broadcast-smooth"),
    plain("smooth-runtime"),
    plain("caption-convert"),
    plain("cc-data"),
    plain("dvb-subtitle"),
    plain("dvb-vbi"),
    plain("mpeg-ps"),
    plain("mpeg-pes"),
    plain("mpeg-ts"),
    plain("playout-runtime"),
    plain("rmt-flute"),
    plain("rtmp-runtime"),
    plain("rtp-packet"),
    plain("scte104"),
    plain("scte35-splice"),
    plain("srt-runtime"),
    plain("ssai-runtime"),
    plain("st2022"),
    plain("st291"),
    plain("st377-1"),
    plain("timed-metadata"),
    plain("broadcast-client"),
    plain("dvr-runtime"),
];
