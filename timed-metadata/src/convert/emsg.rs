//! SCTE-35 ↔ DASH `emsg` conversion (SCTE 214-3; scheme `urn:scte:scte35:2013:bin`).
use crate::error::{Error, Result};
use alloc::{string::String, vec::Vec};
use mp4_emsg::{EmsgBox, PresentationTime};

/// The SCTE-35 binary carriage scheme for DASH `emsg` (SCTE 214-3).
pub const SCTE35_SCHEME: &str = "urn:scte:scte35:2013:bin";

/// Parameters for emitting a SCTE-35-carrying `emsg`.
#[derive(Debug, Clone)]
pub struct EmsgConfig {
    /// `timescale` (ticks/second) for the emsg time fields.
    pub timescale: u32,
    /// `presentation_time_delta` (v0) or `presentation_time` (v1).
    pub presentation: PresentationTime,
    /// `event_duration` in `timescale` units (0 if unknown).
    pub event_duration: u32,
    /// `value` string (often the segmentation type id, as text).
    pub value: String,
    /// `id` — unique event identifier (u32).
    pub id: u32,
}

/// Wrap a verbatim `splice_info_section` as a SCTE-35 `emsg` box (serialized bytes).
pub fn scte35_to_emsg(splice_raw: &[u8], cfg: &EmsgConfig) -> Result<Vec<u8>> {
    let boxx = EmsgBox {
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
    use super::*;
    use alloc::string::ToString;

    fn splice_2002() -> alloc::vec::Vec<u8> {
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn scte35_to_emsg_embeds_splice_verbatim_then_round_trips() {
        let splice = splice_2002();
        let cfg = EmsgConfig {
            timescale: 90_000,
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
