//! `identify` — probe a file and print a human-readable verdict.
//!
//! Reads a file path from `std::env::args()` (defaulting to a committed
//! fixture), runs `container_probe::probe`, and prints the detected format, its
//! confidence tier, and the prober detail (TS stride/phase, ISOBMFF major brand,
//! EBML DocType). It also shows what `Ambiguous`, `Insufficient` and `Unknown`
//! mean so a caller can see how each outcome should be handled.

use container_probe::{Detail, Probe};

/// Render the prober detail for `Identified` output.
fn detail_text(detail: &Detail) -> String {
    match detail {
        Detail::Ts { stride, phase, .. } => {
            format!("MPEG-2 TS: {stride}-byte stride, first sync at offset {phase}")
        }
        Detail::Isobmff {
            major_brand,
            boxes_walked,
            layout,
            ..
        } => {
            let brand = major_brand
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_else(|| "<none>".into());
            // `layout` is what a consumer needs to pick a demuxer: a
            // fragmented file wants transmux's `Fmp4Demux`, a progressive one
            // `ProgressiveDemux`. The major brand cannot tell them apart —
            // real fragmented and progressive files share the `isom` brand.
            format!("ISOBMFF: major brand '{brand}', {boxes_walked} boxes chained, {layout} layout")
        }
        Detail::Ebml { doc_type, .. } => format!("EBML: DocType {doc_type}"),
        Detail::Flv {
            has_audio,
            has_video,
            data_offset,
            ..
        } => format!("FLV: audio={has_audio}, video={has_video}, data offset={data_offset}"),
        Detail::Mxf { partition_kind, .. } => {
            format!("MXF: partition kind 0x{partition_kind:02X}")
        }
        Detail::MpegPs {
            structurally_valid, ..
        } => format!("MPEG-PS: structural marker bits {structurally_valid}"),
        Detail::None => "no format-specific detail".into(),
        _ => "unknown detail".into(),
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        // Absolute, via CARGO_MANIFEST_DIR: a bare relative path only resolves
        // when the example happens to be run from the workspace root, so it
        // turns "wrong working directory" into "fixture missing".
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/container-probe/m2ts_192.m2ts"
        )
        .to_string()
    });
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            std::process::exit(2);
        }
    };

    match container_probe::probe(&data) {
        Probe::Identified {
            format,
            confidence,
            detail,
            ..
        } => {
            println!(
                "{}: {} ({}, {})",
                path,
                format.name(),
                confidence.name(),
                detail_text(&detail)
            );
        }
        Probe::Ambiguous { candidates, .. } => {
            println!("{path}: ambiguous; candidates by score:");
            for c in &candidates {
                println!(
                    "    {} ({} = {})",
                    c.format.name(),
                    c.confidence.name(),
                    c.confidence.as_u8()
                );
            }
        }
        Probe::Insufficient { need_at_least, .. } => {
            println!(
                "{path}: not enough bytes to decide — supply at least {need_at_least} bytes (read more), then probe again"
            );
        }
        Probe::Unknown => {
            println!(
                "{path}: no format matched, and reading more bytes will not change that (stop)"
            );
        }
        _ => {} // `#[non_exhaustive]` requires a wildcard arm.
    }
}
