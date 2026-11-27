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
    // Lane dm3: the chunk reassembler reports every message an interrupting
    // header, an `Abort` or a length ceiling discarded; each session reports
    // the messages, commands, user control events and AMF0 values it passed
    // over and the media that belonged to no published stream, and the FLV
    // reader's tolerant walk reports the spans it jumped and the tail it
    // lost. `ServerEvent` and `ClientEvent` carry the report.
    Reader {
        name: "rtmp-runtime",
        status: Status::Converted,
        reports: &["ServerEvent", "ClientEvent"],
    },
    // Converted with the TS/PS demuxer forwarding: every tolerant bitstream
    // parser is handed a `Findings` collector and records one `ParseFinding`
    // per skip, and `ParseFinding` converts into the one `Damage` report.
    Reader {
        name: "broadcast-bitstream",
        status: Status::Converted,
        reports: &["ParseFinding"],
    },
    // Pending: #4191 converts the remaining readers to the one report.
    Reader {
        name: "isobmff",
        status: Status::Pending(4191),
        reports: &["Warning", "ImageFinding", "BrandFinding"],
    },
    // Lane rz1 converted the Matroska/WebM reader: `Warning` converts to a
    // `Damage` (`impl From<Warning>`), and `MatroskaDemux` republishes every
    // one as `DemuxEvent::Damage` beside its own finding channel.
    Reader {
        name: "broadcast-matroska",
        status: Status::Converted,
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
