//! Ogg prober — Xiph Ogg container, the `"OggS"` capture-pattern at the start
//! of every page (= "OggS page" header, Xiph `Theora`/`OggS` spec §"Page
//! Structure").
//!
//! `"OggS"` at offset 0. Magic-only `STRONG`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The Ogg capture pattern `"OggS"` at the start of every page.
const OGG_CAPTURE_PATTERN: [u8; 4] = *b"OggS";

/// The registered Ogg prober: `"OggS"` at offset 0 over `limit` bytes.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if let Some(outcome) =
        crate::signature_gate(region, &OGG_CAPTURE_PATTERN, OGG_CAPTURE_PATTERN.len())
    {
        return outcome;
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
        test_fixtures::read(rel.trim_start_matches("fixtures/"))
    }

    /// Finding 4: a 3-byte prefix of a real Ogg fixture (1 byte short of the
    /// 4-byte `"OggS"` pattern) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/audio/opus.ogg");
        let region = &data[..OGG_CAPTURE_PATTERN.len() - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(n) => assert_eq!(n, OGG_CAPTURE_PATTERN.len()),
            other => panic!("3-byte Ogg prefix must be Insufficient(4), got {other:?}"),
        }
    }
}
