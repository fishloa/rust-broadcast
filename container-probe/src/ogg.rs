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

    if region.len() < OGG_CAPTURE_PATTERN.len() {
        // Shorter than the 4-byte `"OggS"` capture pattern: a truncated Ogg
        // read a few bytes at a time ("Ogg") is undecided (`Insufficient`),
        // not `Unknown`.
        return Outcome::Insufficient(OGG_CAPTURE_PATTERN.len());
    }
    if region[..OGG_CAPTURE_PATTERN.len()] != OGG_CAPTURE_PATTERN {
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

    /// Finding 4: a 3-byte prefix of a real Ogg fixture (1 byte short of the
    /// 4-byte `"OggS"` pattern) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/container-probe/opus.ogg");
        let region = &data[..OGG_CAPTURE_PATTERN.len() - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(n) => assert_eq!(n, OGG_CAPTURE_PATTERN.len()),
            other => panic!("3-byte Ogg prefix must be Insufficient(4), got {other:?}"),
        }
    }
}
