//! ASF prober — Advanced Systems Format, the 16-byte Header Object GUID at the
//! start of the file (Microsoft ASF Specification §3.1.2.1, "Header Object
//! GUID", `30 26 B2 75 8E 66 CF 11 A6 D9 00 AA 00 62 CE 6C`).
//!
//! The fixture `fixtures/container-probe/video.asf` begins with exactly these
//! 16 bytes. Magic-only `STRONG`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The 16-byte ASF Header Object GUID (Microsoft ASF §3.1.2.1), big-endian
/// field order as stored in the file. First 8 bytes `30 26 B2 75 8E 66 CF 11`.
const ASF_HEADER_GUID: [u8; 16] = [
    0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C,
];

/// The registered ASF prober: 16-byte Header Object GUID at offset 0.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if region.len() < ASF_HEADER_GUID.len() {
        // Shorter than the 16-byte Header Object GUID: a truncated ASF read a
        // few bytes at a time is undecided (`Insufficient`), not `Unknown`.
        return Outcome::Insufficient(ASF_HEADER_GUID.len());
    }
    if region[..ASF_HEADER_GUID.len()] != ASF_HEADER_GUID {
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

    /// Finding 4: a 15-byte prefix of a real ASF fixture (1 byte short of the
    /// 16-byte Header Object GUID) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/container-probe/video.asf");
        let region = &data[..ASF_HEADER_GUID.len() - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(n) => assert_eq!(n, ASF_HEADER_GUID.len()),
            other => panic!("15-byte ASF prefix must be Insufficient(16), got {other:?}"),
        }
    }
}
