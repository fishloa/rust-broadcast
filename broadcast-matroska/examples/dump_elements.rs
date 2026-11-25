//! Print the EBML element tree of a Matroska or WebM file.
//!
//! ```text
//! cargo run -p broadcast-matroska --example dump_elements -- fixtures/mkv/vp9-opus.webm
//! ```
//!
//! The file is walked element by element: each header is read from the file,
//! a leaf's payload is skipped, and a master's children are walked in place, so
//! a capture is never held whole.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::process::ExitCode;

use broadcast_common::Parse;

use broadcast_matroska::Limits;
use broadcast_matroska::ebml::ElementHeader;
use broadcast_matroska::ebml::document::EbmlHeader;
use broadcast_matroska::matroska::ids;
use broadcast_matroska::reader::MAX_ELEMENT_HEADER_BYTES;

/// Room for one EBML element header (RFC 8794 §5, §6.1).
const HEADER_BYTES: usize = MAX_ELEMENT_HEADER_BYTES as usize;

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

/// An element tree walker over a seekable file.
struct Walker {
    file: BufReader<File>,
    len: u64,
    limits: Limits,
}

impl Walker {
    /// Read one element header at `pos`, with its encoded header length.
    fn header_at(
        &mut self,
        pos: u64,
    ) -> Result<(ElementHeader, usize), Box<dyn std::error::Error>> {
        self.file.seek(SeekFrom::Start(pos))?;
        let mut buf = [0u8; HEADER_BYTES];
        let read = read_up_to(&mut self.file, &mut buf)?;
        let (header, used) = ElementHeader::parse(&buf[..read])?;
        Ok((header, used))
    }

    /// Print the element at `pos` and, when it is a master, its children.
    /// Returns the offset one past the element.
    fn walk(&mut self, pos: u64, depth: usize) -> Result<u64, Box<dyn std::error::Error>> {
        if depth > usize::try_from(self.limits.max_depth()).unwrap_or(usize::MAX) {
            return Err("element nesting deeper than the limits allow".into());
        }
        let (header, used) = self.header_at(pos)?;
        print_header(header, depth);
        let id = header.id().value();
        let payload_start = pos.saturating_add(used as u64);
        let Some(payload_len) = header.payload_len() else {
            // An unknown-size master runs to its sibling (a Cluster) or to the
            // end of the file (a Segment).
            let mut child = payload_start;
            while child < self.len {
                let (next, _) = self.header_at(child)?;
                if id != ids::SEGMENT && ids::SEGMENT_CHILD_IDS.contains(&next.id().value()) {
                    break;
                }
                child = self.walk(child, depth + 1)?;
            }
            return Ok(child);
        };
        let end = payload_start.saturating_add(payload_len);
        if !ids::is_master(id) {
            return Ok(end);
        }
        let mut child = payload_start;
        while child < end {
            let next = self.walk(child, depth + 1)?;
            if next <= child {
                return Err("element does not advance the walk".into());
            }
            child = next;
        }
        Ok(end)
    }
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    let limits = Limits::new();
    let mut walker = Walker {
        file: BufReader::new(file),
        len,
        limits,
    };

    let (ebml, used) = walker.header_at(0)?;
    let ebml_len = ebml
        .payload_len()
        .ok_or("the EBML header has an unknown size")?;
    let mut header_bytes = vec![0u8; usize::try_from(header_len(used, ebml_len)).unwrap_or(0)];
    walker.file.seek(SeekFrom::Start(0))?;
    walker.file.read_exact(&mut header_bytes)?;
    let document_header = EbmlHeader::parse(&header_bytes)?;
    println!(
        "EBML doc_type={:?} version={} read_version={}",
        document_header.doc_type(),
        document_header.version(),
        document_header.read_version()
    );

    let segment_start = (used as u64).saturating_add(ebml_len);
    walker.walk(segment_start, 0)?;
    Ok(())
}

/// The whole byte length of an element whose header is `header_len` bytes and
/// whose payload is `payload_len` bytes.
fn header_len(header_len: usize, payload_len: u64) -> u64 {
    (header_len as u64).saturating_add(payload_len)
}

/// Fill `buf` as far as the stream allows; returns how many bytes arrived.
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
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
