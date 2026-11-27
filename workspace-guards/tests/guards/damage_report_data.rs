//! Data for the `damage_report` guard: every crate that reads damaged input,
//! whether it reports through `broadcast_core::damage::Damage` yet, and the
//! report enums it declares.
//!
//! A crate is `Converted` once its reader publishes a [`Damage`] carrying
//! variant and/or converts its own report enum into one; it is `Pending` with
//! the tracking issue number otherwise. The pending list may only shrink:
//! converting a crate moves it to `Converted`, it never leaves the table.

use crate::damage_report::Status;

/// One reading crate.
pub struct Reader {
    /// Workspace member name.
    pub name: &'static str,
    /// Whether it reports through `Damage` yet.
    pub status: Status,
    /// Public enums this crate declares that report skipped content. Each must
    /// carry a `Damage` (a variant holding one) or convert to one.
    pub reports: &'static [&'static str],
}

pub const READERS: &[Reader] = &[
    // Lane dm0 converted the MPEG-2 TS-over-HTTP slice: `StreamingTsDemux`
    // emits `DemuxEvent::Damage`, `InputDegradation` converts to a `Damage`,
    // and the media-plane `SessionEvent` and broadcast-client `Event` channels
    // carry it to the player.
    Reader {
        name: "transmux",
        status: Status::Converted,
        reports: &["DemuxEvent", "InputDegradation"],
    },
    Reader {
        name: "media-plane",
        status: Status::Converted,
        reports: &["SessionEvent"],
    },
    Reader {
        name: "broadcast-client",
        status: Status::Converted,
        reports: &["Event"],
    },
    // Lane rz2 converted the ISO BMFF readers: `Warning`, `ImageFinding` and
    // `BrandFinding` convert to a `Damage`, `Reader::step` publishes
    // `Step::Damage` and `ItemReader::step` publishes `ItemStep::Damage`.
    Reader {
        name: "isobmff",
        status: Status::Converted,
        reports: &["Warning", "ImageFinding", "BrandFinding"],
    },
    Reader {
        name: "broadcast-matroska",
        status: Status::Pending(4191),
        reports: &["Warning"],
    },
];

/// Substrings that mark a public enum's *name* as a report of skipped content.
/// A crate in the table that declares one must list it in `Reader::reports`.
pub const REPORT_NAME_NEEDLES: &[&str] = &[
    "Warning",
    "Finding",
    "Damage",
    "Degradation",
    "Defect",
    "Anomaly",
];
