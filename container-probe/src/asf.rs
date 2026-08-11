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

    if region.len() < ASF_HEADER_GUID.len() || region[..ASF_HEADER_GUID.len()] != ASF_HEADER_GUID {
        return Outcome::None;
    }

    Outcome::Match(Evidence {
        confidence: Confidence::STRONG,
        detail: Detail::None,
    })
}
