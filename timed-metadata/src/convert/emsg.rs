//! SCTE-35 ↔ DASH `emsg` conversion (scheme `urn:scte:scte35:2013:bin`).
//!
//! Carriage is **ANSI/SCTE 214-3 §8.3.3** for the inband `emsg` form used
//! here. (The MPD `EventStream` form, `urn:scte:scte35:2014:xml+bin`, is
//! SCTE 214-1 §6.8.4 — a different part for a different carriage.)
//!
//! #951 recorded this crate and `mp4-emsg` citing contradictory parts. This
//! crate was right. Settled from DASH-IF IOP v4.3 §5.4.3 p.163, which is
//! freely published and states the split explicitly, so no paywalled SCTE
//! document was needed after all.
use crate::error::{Error, Result};
use alloc::{string::String, vec::Vec};
use broadcast_common::timescale::Timescale;
use mp4_emsg::{EmsgBox, PresentationTime};

/// The SCTE-35 binary carriage scheme for inband DASH `emsg`
/// (ANSI/SCTE 214-3 §8.3.3).
pub const SCTE35_SCHEME: &str = "urn:scte:scte35:2013:bin";

/// Parameters for emitting a SCTE-35-carrying `emsg`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EmsgConfig {
    /// `timescale` (ticks/second) for the emsg time fields.
    pub timescale: Timescale,
    /// `presentation_time_delta` (v0) or `presentation_time` (v1).
    pub presentation: PresentationTime,
    /// `event_duration` in `timescale` units (0 if unknown).
    pub event_duration: u32,
    /// `value` string (often the segmentation type id, as text).
    pub value: String,
    /// `id` — unique event identifier (u32).
    pub id: u32,
}

impl EmsgConfig {
    /// Build the emsg parameters from their required values.
    #[must_use]
    pub fn new(
        timescale: Timescale,
        presentation: PresentationTime,
        event_duration: u32,
        value: String,
        id: u32,
    ) -> Self {
        Self {
            timescale,
            presentation,
            event_duration,
            value,
            id,
        }
    }
}

/// Wrap a verbatim `splice_info_section` as a SCTE-35 `emsg` box (serialized bytes).
pub fn scte35_to_emsg(splice_raw: &[u8], cfg: &EmsgConfig) -> Result<Vec<u8>> {
    let boxx = EmsgBox {
        flags: mp4_emsg::EMSG_FLAGS,
        scheme_id_uri: SCTE35_SCHEME,
        value: &cfg.value,
        timescale: cfg.timescale,
        presentation_time: cfg.presentation,
        event_duration: cfg.event_duration,
        id: cfg.id,
        message_data: splice_raw,
    };
    Ok(boxx.to_vec()?)
}

/// Extract the verbatim `splice_info_section` from a SCTE-35 `emsg` box.
pub fn emsg_to_scte35(emsg_bytes: &[u8]) -> Result<Vec<u8>> {
    let boxx = EmsgBox::parse(emsg_bytes)?;
    if !boxx.is_scte35() {
        return Err(Error::UnsupportedScheme {
            scheme: String::from(boxx.scheme_id_uri),
        });
    }
    Ok(boxx.message_data.to_vec())
}

#[cfg(test)]
mod tests {
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    use super::*;
    use alloc::string::ToString;

    fn splice_2002() -> alloc::vec::Vec<u8> {
        test_fixtures::read("scte35/splice-insert-2002.bin")
    }

    #[test]
    fn scte35_to_emsg_embeds_splice_verbatim_then_round_trips() {
        let splice = splice_2002();
        let cfg = EmsgConfig {
            timescale: timescale(90_000),
            presentation: PresentationTime::Delta(0),
            event_duration: 2_160_000,
            value: "1".to_string(),
            id: 1,
        };
        let emsg = scte35_to_emsg(&splice, &cfg).unwrap();
        // message_data must equal the splice verbatim:
        let extracted = emsg_to_scte35(&emsg).unwrap();
        assert_eq!(extracted, splice);
    }
}
