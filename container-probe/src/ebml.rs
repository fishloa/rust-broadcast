//! EBML / Matroska / WebM prober — RFC 8794 (EBML framing) + RFC 9559
//! (Matroska element IDs); transcription in `transmux/docs/webm/ebml-matroska.md`.
//!
//! The EBML magic `1A 45 DF A3` must be at offset 0 (nowhere else). The header's
//! element chain is then walked to locate the `DocType` element (ID `0x4282`)
//! and read its ASCII string value:
//!
//! - `DocType == "webm"` -> `Format::WebM`.
//! - `DocType == "matroska"` -> `Format::Matroska`.
//! - anything else -> `Format::Matroska` with `DocType::Other`.
//!
//! Magic **plus** a successfully read DocType scores `CERTAIN`; magic alone with
//! an unparseable header scores `STRONG` with `DocType::Other`.
//!
//! EBML elements are `(ID, size, data)` triples where ID and size are
//! variable-length integers (VINTs): the first byte's leading-zero count gives
//! the width (1-8 bytes); the first `1` bit is the length marker. For element
//! *IDs* the marker bits are kept; for *sizes* they are cleared. An all-ones
//! size is "unknown" (runs to end of stream). Every varint read is bounded by
//! the region so a malformed stream cannot read past it or loop.

use crate::{Confidence, Detail::Ebml, DocType, Evidence, Outcome};

/// The EBML magic, first-element ID `EBML` (RFC 8794 §9.6 / RFC 9559), 4 bytes.
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// Maximum VINT width, 8 bytes (RFC 8794 §4.2).
const VINT_MAX_WIDTH: usize = 8;
/// The `EBMLDocType` element ID (RFC 9559 EBML header), whose string value is
/// "webm", "matroska", or another name.
const ID_DOC_TYPE: [u8; 2] = [0x42, 0x82];
/// Lower-case ASCII "webm".
const DOC_TYPE_WEBM: &str = "webm";
/// Lower-case ASCII "matroska".
const DOC_TYPE_MATROSKA: &str = "matroska";

/// The registered EBML prober: magic + DocType read over `limit` bytes.
///
/// The harness resolves the candidate `Format` from `Detail::Edml`'s `DocType`:
/// `Webm` -> `Format::WebM`, otherwise `Format::Matroska`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if region.len() < EBML_MAGIC.len() || region[..EBML_MAGIC.len()] != EBML_MAGIC {
        return Outcome::None;
    }

    match find_doc_type(region) {
        Some(doc_type) => Outcome::Match(Evidence {
            confidence: Confidence::CERTAIN,
            detail: Ebml { doc_type },
        }),
        None => {
            // Magic at offset 0 but the header would not yield a DocType. That
            // is still unambiguous EBML (STRONG), just with no DocType to name.
            Outcome::Match(Evidence {
                confidence: Confidence::STRONG,
                detail: Ebml {
                    doc_type: DocType::Other,
                },
            })
        }
    }
}

/// Walk the EBML header element to read its `DocType`. Returns `None` if the
/// DocType element is absent or its string cannot be read.
fn find_doc_type(region: &[u8]) -> Option<DocType> {
    // The 4-byte EBML element ID is followed by a size VINT bounding its data.
    let cursor = EBML_MAGIC.len(); // 4
    // Read the EBML header element's size VINT.
    let (len, size) = read_size_vint(&region[cursor..])?;
    // The header data runs for `size` bytes (or to region end when unknown).
    let data = if let Some(sz) = size {
        let end = (cursor + len + sz).min(region.len());
        &region[cursor + len..end]
    } else {
        &region[cursor + len..]
    };

    // Walk the header's child elements looking for DocType.
    let mut off = 0usize;
    while off < data.len() {
        let (id_len, id) = read_id_vint(&data[off..])?;
        let after_id = off + id_len;
        let (esz_len, esz) = read_size_vint(&data[after_id..])?;
        let value_start = after_id + esz_len;
        let value_end = match esz {
            Some(sz) => {
                // The value must lie within the header region; a size that
                // extends past it (or overflows) is malformed.
                match value_start.checked_add(sz) {
                    Some(end) if end <= data.len() => end,
                    _ => return None,
                }
            }
            None => data.len(),
        };
        if id == ID_DOC_TYPE {
            let value = &data[value_start..value_end];
            let text = core::str::from_utf8(value).ok()?;
            return Some(match text {
                DOC_TYPE_WEBM => DocType::Webm,
                DOC_TYPE_MATROSKA => DocType::Matroska,
                _ => DocType::Other,
            });
        }
        off = value_end;
    }
    None
}

/// Read an EBML **element-ID** VINT: the whole encoded value including the
/// length-marker bit (RFC 8794 §4.2). Returns the encoded ID bytes and width.
fn read_id_vint(b: &[u8]) -> Option<(usize, &[u8])> {
    let width = vint_width(*b.first()?)?;
    if b.len() < width {
        return None;
    }
    Some((width, &b[..width]))
}

/// Read an EBML **size** VINT, clearing the length-marker bit. Returns the
/// width and the decoded value, or `None` for an all-ones "unknown" size.
fn read_size_vint(b: &[u8]) -> Option<(usize, Option<usize>)> {
    let first = *b.first()?;
    let width = vint_width(first)?;
    if b.len() < width {
        return None;
    }
    // Zero out the marker bit (bit 8-width).
    let bits = width * 7;
    let marker: u8 = 1 << (7 - width + 1);
    let mut v: usize = (first & !marker) as usize;
    for &byte in &b[1..width] {
        v = (v << 8) | byte as usize;
    }
    // "Unknown size": all payload bits set + width's implied max.
    let max = if width > 0 { (1usize << bits) - 1 } else { 0 };
    if v == max {
        Some((width, None))
    } else {
        Some((width, Some(v)))
    }
}

/// The VINT width (1..=8) from a first byte: the count of leading zeros.
fn vint_width(first: u8) -> Option<usize> {
    let width = first.leading_zeros() as usize + 1;
    if width <= VINT_MAX_WIDTH {
        Some(width)
    } else {
        None
    }
}
