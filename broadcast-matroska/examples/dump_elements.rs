//! Print the EBML element tree of a Matroska or WebM file.
//!
//! ```text
//! cargo run -p broadcast-matroska --example dump_elements -- fixtures/mkv/vp9-opus.webm
//! ```

use std::process::ExitCode;

use broadcast_common::Parse;

use broadcast_matroska::ebml::ElementHeader;
use broadcast_matroska::matroska::{Document, Element, ids};

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: dump_elements <file.mkv|file.webm>");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dump_elements: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let document = Document::parse(&bytes)?;
    println!(
        "EBML doc_type={:?} version={} read_version={}",
        document.header().doc_type(),
        document.header().version(),
        document.header().read_version()
    );
    print_header(document.segment().header(), 0);
    for child in document.segment().children() {
        dump(child, 1);
    }
    Ok(())
}

/// Print one element and, when it is a master, its children.
fn dump(element: &Element<'_>, depth: usize) {
    print_header(element.header(), depth);
    let Some(children) = element.children() else {
        return;
    };
    for child in children {
        dump(child, depth + 1);
    }
}

/// Print a header line at `depth`.
fn print_header(header: ElementHeader, depth: usize) {
    let id = header.id().value();
    let (name, kind) = match id {
        broadcast_matroska::ebml::global::VOID_ID => ("Void", "global"),
        broadcast_matroska::ebml::global::CRC32_ID => ("CRC-32", "global"),
        _ => (
            ids::lookup(id).map_or("unknown", |def| def.name),
            ids::kind(id).map_or("unknown", |kind| kind.name()),
        ),
    };
    let size = match header.payload_len() {
        Some(len) => format!("{len}"),
        None => "unknown".to_string(),
    };
    println!(
        "{:indent$}{} (0x{id:X}) {kind} {size}",
        "",
        name,
        indent = depth * 2
    );
}
