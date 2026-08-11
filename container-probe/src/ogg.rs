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

    if region.len() < OGG_CAPTURE_PATTERN.len()
        || region[..OGG_CAPTURE_PATTERN.len()] != OGG_CAPTURE_PATTERN
    {
        return Outcome::None;
    }

    Outcome::Match(Evidence {
        confidence: Confidence::STRONG,
        detail: Detail::None,
    })
}
