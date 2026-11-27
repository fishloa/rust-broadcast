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
    // Lane dm3: the segment-pull clients forward every report their MP4/TS
    // reader publishes and convert each degradation the reader only describes
    // into the same report, coalesced by kind per resource.
    Reader {
        name: "hls-runtime",
        status: Status::Converted,
        reports: &["Output"],
    },
    Reader {
        name: "dash-runtime",
        status: Status::Converted,
        reports: &["Output"],
    },
    Reader {
        name: "smooth-runtime",
        status: Status::Converted,
        reports: &["Output"],
    },
    // Lane dm3: the RTSP client resynchronises past junk between interleaved
    // frames and reports it, plus the responses and headers it had to skip,
    // as `ClientEvent::Damage`; a dropped media section converts to a report.
    Reader {
        name: "rtsp-runtime",
        status: Status::Converted,
        reports: &["ClientEvent"],
    },
    // Lane dm3: the SRT delivery engine reports every packet run TSBPD gave
    // up on (too late, gap skipped, peer drop request) and every datum it
    // dropped before the scheduler saw it, as `SrtEvent::Damage`.
    Reader {
        name: "srt-runtime",
        status: Status::Converted,
        reports: &["SrtEvent"],
    },
    // Lane dm3 did NOT reach this one: its chunk-stream, AMF0 and FLV skips
    // still report nothing, so `broadcast-client`'s RTMP source publishes no
    // damage report yet. Listed so the gap is visible and the guard enforces
    // it the moment it is converted.
    Reader {
        name: "rtmp-runtime",
        status: Status::Pending(4191),
        reports: &[],
    },
    // Pending: #4191 converts the remaining readers to the one report.
    Reader {
        name: "isobmff",
        status: Status::Pending(4191),
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
