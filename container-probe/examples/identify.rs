//! `identify` — probe a file and print a human-readable verdict.
//!
//! Reads a file path from `std::env::args()` (defaulting to a committed
//! fixture), runs `container_probe::probe`, and prints the detected format, its
//! confidence tier, and the prober detail (TS stride/phase, ISOBMFF major brand,
//! EBML DocType). It also shows what `Ambiguous`, `Insufficient` and `Unknown`
//! mean so a caller can see how each outcome should be handled.

use container_probe::{Detail, Probe};

/// Map a `Confidence` score to its tier name. The named tier constants
/// (`Confidence::CERTAIN` etc.) are crate-internal; the example keeps the
/// display mapping to the documented values.
fn tier_label(score: u8) -> &'static str {
    match score {
        240 => "CERTAIN",
        192 => "STRONG",
        160 => "STRUCTURAL",
        144 => "LATTICE_STRONG",
        96 => "LATTICE_WEAK",
        64 => "HEURISTIC",
        _ => "?",
    }
}

/// Render the prober detail for `Identified` output.
fn detail_text(detail: &Detail) -> String {
    match detail {
        Detail::Ts { stride, phase } => {
            format!("MPEG-2 TS: {stride}-byte stride, first sync at offset {phase}")
        }
        Detail::Isobmff {
            major_brand,
            boxes_walked,
        } => {
            let brand = major_brand
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_else(|| "<none>".into());
            format!("ISOBMFF: major brand '{brand}', {boxes_walked} boxes chained")
        }
        Detail::Ebml { doc_type } => format!("EBML: DocType {doc_type}"),
        Detail::None => "no format-specific detail".into(),
        _ => "unknown detail".into(),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/container-probe/m2ts_192.m2ts".to_string());
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
        } => {
            println!(
                "{}: {} ({}, {})",
                path,
                format.name(),
                tier_label(confidence.as_u8()),
                detail_text(&detail)
            );
        }
        Probe::Ambiguous { candidates } => {
            println!("{path}: ambiguous; candidates by score:");
            for c in &candidates {
                println!(
                    "    {} ({} = {})",
                    c.format.name(),
                    tier_label(c.confidence.as_u8()),
                    c.confidence.as_u8()
                );
            }
        }
        Probe::Insufficient { need_at_least } => {
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
