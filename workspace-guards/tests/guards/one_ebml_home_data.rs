//! Per-crate data for the `one_ebml_home` guard: the crates that may spell an
//! EBML element ID outside `broadcast-matroska`, one reviewed line at a time.

use crate::one_ebml_home::Crate;

/// The one home of the EBML/Matroska element IDs (RFC 8794, RFC 9559).
pub const EXEMPT: &[&str] = &["broadcast-matroska"];

pub const ALLOW: &[Crate] = &[Crate {
    name: "container-probe",
    allow: &[(
        "src/ebml.rs",
        "const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];",
        "container-probe depends only on broadcast-common at runtime; its dev-only drift tests pin this magic to broadcast-matroska",
    )],
}];
