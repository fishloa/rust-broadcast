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

    if region.len() < EBML_MAGIC.len() {
        // Shorter than the 4-byte magic we must rule out: an EBML file read a
        // few bytes at a time is genuinely undecided, so this is `Insufficient`
        // (`need` = the magic length), not `Unknown` — more bytes could make it
        // the exact bytes EBML opens with.
        return Outcome::Insufficient(EBML_MAGIC.len());
    }
    if region[..EBML_MAGIC.len()] != EBML_MAGIC {
        return Outcome::None;
    }

    match find_doc_type(region) {
        DocTypeResult::Found(doc_type) => Outcome::Match(Evidence {
            confidence: Confidence::CERTAIN,
            detail: Ebml { doc_type },
        }),
        // The header walk ran off the end of the supplied region before it
        // could read the DocType. `DocType` is what distinguishes `WebM` from
        // `Matroska`, so concluding here would send the caller to the wrong
        // demuxer — this must be `Insufficient` ("read more"), not a confident
        // guess at the more general format (the shared `Insufficient` vs
        // `Unknown` decision).
        DocTypeResult::Truncated => Outcome::Insufficient(region.len() + 1),
        // Magic at offset 0 but the header was fully walked and holds no
        // DocType. That is still unambiguous EBML (STRONG), just with no
        // DocType to name.
        DocTypeResult::Absent => Outcome::Match(Evidence {
            confidence: Confidence::STRONG,
            detail: Ebml {
                doc_type: DocType::Other,
            },
        }),
    }
}

/// The outcome of walking the EBML header for its `DocType`.
enum DocTypeResult {
    /// A `DocType` element was read.
    Found(DocType),
    /// The walk ran off the end of the supplied region mid-structure, so the
    /// `DocType` could not yet be read.
    Truncated,
    /// The region was fully walked and held no (or an unreadable) `DocType`.
    Absent,
}

/// Walk the EBML header element to read its `DocType`. See [`DocTypeResult`].
///
/// `Truncated` is reported whenever a VINT or element body extends past the
/// end of `region` (the prober ran out of data before the DocType was
/// readable); `Absent` is reported only when the header was examined and the
/// DocType element is missing or holds a non-UTF-8 string.
fn find_doc_type(region: &[u8]) -> DocTypeResult {
    // The 4-byte EBML element ID is followed by a size VINT bounding its data.
    let cursor = EBML_MAGIC.len(); // 4
    // Read the EBML header element's size VINT. An empty or too-short tail is a
    // truncation, not an absence.
    let (len, size) = match read_size_vint(&region[cursor..]) {
        Some(x) => x,
        None => return DocTypeResult::Truncated,
    };
    // The header data runs for `size` bytes (or to region end when unknown).
    // A known `size` that extends past the region is a truncation: the header
    // body the DocType lives in is not fully present.
    let data = if let Some(sz) = size {
        // `cursor + len + sz` cannot be trusted to fit a `usize` in one
        // addition on every target: it is the attacker-controlled `size`, so
        // add it checked (as line 92 does) and clamp to the region.
        let body_start = cursor + len; // <= region.len(), guaranteed by read_size_vint
        match body_start.checked_add(sz) {
            Some(e) if e <= region.len() => &region[body_start..e],
            _ => return DocTypeResult::Truncated,
        }
    } else {
        &region[cursor + len..]
    };

    // Walk the header's child elements looking for DocType.
    let mut off = 0usize;
    while off < data.len() {
        let (id_len, id) = match read_id_vint(&data[off..]) {
            Some(x) => x,
            None => return DocTypeResult::Truncated,
        };
        let after_id = off + id_len;
        let (esz_len, esz) = match read_size_vint(&data[after_id..]) {
            Some(x) => x,
            None => return DocTypeResult::Truncated,
        };
        let value_start = after_id + esz_len;
        let value_end = match esz {
            Some(sz) => {
                // The value must lie within the header region; a size that
                // extends past it (or overflows) is malformed.
                match value_start.checked_add(sz) {
                    Some(end) if end <= data.len() => end,
                    _ => return DocTypeResult::Truncated,
                }
            }
            None => data.len(),
        };
        if id == ID_DOC_TYPE {
            let value = &data[value_start..value_end];
            let text = match core::str::from_utf8(value) {
                Ok(t) => t,
                Err(_) => return DocTypeResult::Absent,
            };
            return DocTypeResult::Found(match text {
                DOC_TYPE_WEBM => DocType::Webm,
                DOC_TYPE_MATROSKA => DocType::Matroska,
                _ => DocType::Other,
            });
        }
        off = value_end;
    }
    DocTypeResult::Absent
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
    // Zero out the length-marker bit. For a width-`w` VINT the first byte
    // holds `w-1` leading zeros then the marker `1` (RFC 8794 §4.2), so the
    // marker is the `(8-w)`-th bit from the right. `8 - w` is in `1..=7` for
    // every valid width `1..=8`, so `1u8 << (8 - w)` never shifts off a bit
    // and never underflows — unlike the earlier `7 - width + 1`, which
    // underflowed to `0..6` for `width == 8` and panicked (attacker bytes can
    // set any width).
    let marker: u8 = 1u8 << (8 - width);
    // A size VINT's value field is `7 * w` bits in total (RFC 8794 §4.5): the
    // marker bit is data-free in the first byte and the remaining `w-1` bytes
    // each carry a full 8; `7 * 8 == 56`, the width-8 maximum. Decode into a
    // `u64` so the `(1 << bits) - 1` "all-ones unknown-size" test never
    // overflows on a 32-bit `usize` target (where `1usize << 35` for width 5
    // would otherwise panic or mask).
    let bits = width * 7;
    let mut v: u64 = u64::from(first & !marker);
    for &byte in &b[1..width] {
        v = (v << 8) | u64::from(byte);
    }
    // "Unknown size": every payload bit set = this width's all-ones value.
    let max: u64 = (1u64 << bits) - 1;
    if v == max {
        Some((width, None))
    } else {
        // A known size that does not fit a `usize` (possible only in the last
        // two widths on a 32-bit target) cannot be addressed into a slice, so
        // abort the element walk — the header is unreadable on this target.
        Some((width, Some(usize::try_from(v).ok()?)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// Finding 1: a 12-byte input whose size VINT width is 8 must never panic.
    ///
    /// `1A 45 DF A3 | 01 00 00 00 00 00 00 00` is the EBML magic followed by a
    /// size VINT whose first byte `0x01` implies width 8 (seven leading zeros).
    /// The old marker arithmetic `1 << (7 - width + 1)` underflowed for
    /// `width == 8` (`7usize - 8usize`) and panicked with "attempt to subtract
    /// with overflow"; release builds only survived because the double wrap
    /// happened to land on the answer. The width-8 (7 leading zeros) case is
    /// attacker-reachable, so it must be exercised, not assumed away.
    #[test]
    fn width_8_size_vint_does_not_panic() {
        let input: [u8; 12] = [
            0x1A, 0x45, 0xDF, 0xA3, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let p = crate::probe(&input);
        // No panic is the core assertion; the specific verdict just confirms the
        // walk still concluded something sane (magic + no DocType -> STRONG).
        assert!(matches!(
            p,
            crate::Probe::Identified {
                confidence: crate::Confidence::STRONG,
                ..
            }
        ));
    }

    /// Finding 1: every VINT width 1..=8 must decode a size without panic or
    /// underflow. A first byte with `w-1` leading zeros (so `vint_width == w`)
    /// followed by zeros yields a valid, decodable size VINT.
    #[test]
    fn every_vint_width_decodes_without_panic() {
        for width in 1..=8u32 {
            // `vint_width(first) = leading_zeros + 1 = width` needs the top
            // `width` bits of `first` clear except a `1` at position
            // `8 - width` — a single `1 << (8 - width)`.
            let first = 1u8 << (8 - width);
            // The remaining (width-1) bytes are zero, so the decoded value is
            // just first's low bits after the marker is cleared (0).
            let mut buf = [0u8; 8];
            buf[0] = first;
            let (got_width, size) = read_size_vint(&buf).expect("width {width} must decode");
            assert_eq!(got_width, width as usize);
            // The marker bit is cleared, so the value is 0 — a *known* size.
            assert_eq!(size, Some(0));
        }
    }

    /// Finding 4: a 3-byte prefix of a real Matroska file (1 byte short of the
    /// 4-byte EBML magic) is `Insufficient`, not `Unknown` — a truncated .mkv
    /// must be told to read more, never to stop.
    #[test]
    fn short_magic_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/mkv/h264_aac.mkv");
        let region = &data[..EBML_MAGIC.len() - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, EBML_MAGIC.len()),
            other => panic!("3-byte EBML prefix must be Insufficient(4), got {other:?}"),
        }
    }
}
