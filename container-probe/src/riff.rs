//! RIFF/WAVE prober — Microsoft Multimedia File Format (WAVE), the `"RIFF"` /
//! `"WAVE"` four-CC layout (Microsoft, "Multimedia Programming Interface and
//! Data Specifications" — RIFF section).
//!
//! `"RIFF"` at offset 0 and `"WAVE"` at offset 8. Magic-only `STRONG`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The RIFF container signature at offset 0.
const RIFF_SIGNATURE: [u8; 4] = *b"RIFF";
/// The WAVE form-type at offset 8 (a RIFF file's 4-byte form type).
const WAVE_FORM_TYPE: [u8; 4] = *b"WAVE";
/// Byte offset of the `"WAVE"` form type (after "RIFF" + the 4-byte size).
const WAVE_OFFSET: usize = 8;

/// The registered RIFF prober: `"RIFF"`..`"WAVE"` over `limit` bytes.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    let need = WAVE_OFFSET + WAVE_FORM_TYPE.len();
    if region.len() < need || region[..RIFF_SIGNATURE.len()] != RIFF_SIGNATURE {
        return Outcome::None;
    }
    if region[WAVE_OFFSET..WAVE_OFFSET + WAVE_FORM_TYPE.len()] != WAVE_FORM_TYPE {
        return Outcome::None;
    }

    Outcome::Match(Evidence {
        confidence: Confidence::STRONG,
        detail: Detail::None,
    })
}
