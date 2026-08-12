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
    if region.len() < need {
        // Shorter than the `"RIFF"`..`"WAVE"` span the magic needs: a truncated
        // RIFF read a few bytes at a time ("RIF", "RIFF") is undecided
        // (`Insufficient`), not `Unknown`.
        return Outcome::Insufficient(need);
    }
    if region[..RIFF_SIGNATURE.len()] != RIFF_SIGNATURE {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// Finding 4: an 11-byte prefix of a real WAV fixture (1 byte short of the
    /// 12-byte `"RIFF"`..`"WAVE"` span) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/container-probe/pcm_s16le.wav");
        let need = WAVE_OFFSET + WAVE_FORM_TYPE.len();
        let region = &data[..need - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(n) => assert_eq!(n, need),
            other => panic!("11-byte RIFF prefix must be Insufficient(12), got {other:?}"),
        }
    }
}
