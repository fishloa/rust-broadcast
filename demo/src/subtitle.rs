//! DVB (bitmap) subtitle extraction + a still-frame RGBA preview — GitHub
//! issue #662, item #5 ("caption preview") scoped to *container-level* DVB
//! subtitling (ETSI EN 300 743), not CEA-608/708 (which lives inside the
//! H.264 SEI bitstream and is out of scope for this project).
//!
//! Pipeline: find the PID(s) whose PMT ES_info carries a `subtitling_descriptor`
//! (ETSI EN 300 468 §6.2.42, tag 0x59) → reassemble that PID's PES stream
//! (`mpeg-pes`) → parse each PES payload's `PES_data_field` with `dvb-subtitle`
//! (ETSI EN 300 743 §7.2) → for the first page/region/object/CLUT combination
//! that fully resolves, decode the object's pixel-data sub-blocks (2/4/8-bit
//! run-length code strings, §7.2.5.2 Tables 22-26) through the page's CLUT
//! (§7.2.4 Table 16) into an RGBA8 still image.
//!
//! `dvb-subtitle` parses and structurally delimits pixel-data sub-blocks (it
//! knows exactly where each RLE code string starts/ends) but does not decode
//! their run-length payload into pixel indices, nor map indices through a
//! CLUT to colour — this module implements that decode directly off the
//! cited spec tables, since no crate in the workspace does it yet.
//!
//! Scope (deliberately not over-built per the issue): only the FIRST subtitle
//! PID found is reassembled/decoded; only the object's top field is decoded
//! (real captures are overwhelmingly progressive-as-single-field, i.e.
//! `bottom_field_data_block_length == 0`, so this covers the common case);
//! character-string and progressive (zlib) object coding are not rendered
//! (no pixels to decode — `dvb-subtitle` parses them, this module just skips
//! them for the preview).

use std::collections::BTreeMap;

use broadcast_common::Parse;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::pmt::PmtStream;
use dvb_subtitle::{
    AnySegment, ClutDefinitionSegment, ClutEntry, DataIdentifier, DataType, ObjectDataPayload,
    ObjectDataSegment, PageCompositionSegment, PesDataField, RegionCompositionSegment,
};
use mpeg_pes::{PesAssembler, PesPacket, StreamId};
use serde::Serialize;

/// `private_stream_1` — the PES `stream_id` DVB subtitle streams are carried
/// on (ETSI EN 300 743 §4.2, via ISO/IEC 13818-1 Table 2-22). The same PID
/// commonly multiplexes `padding_stream` (0xBE) PES too, which are not
/// subtitle data and must be filtered out by `stream_id` + `data_identifier`.
const PRIVATE_STREAM_1: StreamId = StreamId(0xBD);

/// Defensive cap on a single decoded pixel row. Real subtitle regions are
/// bounded by the spec to at most 720x576 (or the DDS display size); this
/// only guards against a malformed/adversarial run-length field blowing up
/// memory, mirroring the `MAX_*` caps in `lib.rs`.
const MAX_ROW_PIXELS: usize = 2048;
/// Defensive cap on the number of decoded rows (lines) per object, same
/// rationale as [`MAX_ROW_PIXELS`].
const MAX_SUBTITLE_ROWS: usize = 1152;
/// Defensive cap on RLE decode iterations per row, in case a malformed code
/// string never signals its own end marker. Never panics; just stops early.
const MAX_DECODE_ITERATIONS: u32 = 100_000;

/// True if `stream`'s ES_info descriptor loop carries a `subtitling_descriptor`
/// (tag 0x59) — i.e. this elementary stream is a DVB subtitle component.
#[must_use]
pub fn stream_is_subtitle(stream: &PmtStream<'_>) -> bool {
    stream
        .es_info
        .iter()
        .any(|d| matches!(d, Ok(AnyDescriptor::Subtitling(_))))
}

/// Pixel depth a decoded row's indices are expressed in — determines which
/// `ClutEntry::flag_*bit` must be set for a CLUT entry to apply to that row.
#[derive(Clone, Copy)]
enum PixelDepth {
    Two,
    Four,
    Eight,
}

/// One decoded pixel-data_sub-block: a row of CLUT-index pixel values plus
/// the bit depth it was decoded at.
struct RowData {
    depth: PixelDepth,
    indices: Vec<u8>,
}

/// A fully pixel-decoded `object_data_segment` (bitmap coding only).
struct DecodedObject {
    rows: Vec<RowData>,
}

/// The RGBA8 still-frame preview of one decoded subtitle object, ready to
/// paint onto a `<canvas>` via `ImageData`.
#[derive(Serialize)]
pub struct SubtitlePreview {
    /// The PID this preview was decoded from.
    pub pid: u16,
    /// `page_id` of the page composition segment the region belongs to.
    pub page_id: u16,
    /// `region_id` of the resolved region.
    pub region_id: u8,
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, top-to-bottom, standard (non-premultiplied) alpha —
    /// base64-encoded so it crosses the wasm_bindgen boundary as a plain
    /// string (decode client-side with `atob` + `Uint8ClampedArray`).
    pub rgba_base64: String,
}

/// The subtitle panel's report: what was found, how much was decoded, and
/// (if resolvable) a rendered preview.
#[derive(Serialize)]
pub struct SubtitleReport {
    /// Every PID whose PMT ES_info declared a `subtitling_descriptor`.
    pub pids: Vec<u16>,
    /// `PES_data_field`s successfully reassembled and parsed on the tracked
    /// (first) subtitle PID.
    pub pes_fields: u64,
    /// Subtitling segments seen across those PES data fields.
    pub segments_seen: u64,
    /// PES or `PES_data_field` parse failures on the tracked PID.
    pub parse_errors: u64,
    pub preview: Option<SubtitlePreview>,
}

/// Accumulates every subtitle PID discovered from the PMT and reassembles
/// each one's PES stream (a capture's PMT can advertise more subtitle
/// components than actually carry data — e.g. a second, unused program's
/// PMT — so every discovered PID is tracked, mirroring how `lib.rs` already
/// tracks one `PesAssembler` per video/audio/SCTE-35 PID). Regions/CLUTs/
/// objects are keyed by `(pid, id)` since `page_id`/`region_id`/`object_id`
/// are only unique *within* one subtitle component, not across PIDs.
#[derive(Default)]
pub struct SubtitleState {
    pids: Vec<u16>,
    assemblers: BTreeMap<u16, PesAssembler>,
    pages: Vec<(u16, PageCompositionSegment)>,
    regions: BTreeMap<(u16, u8), RegionCompositionSegment>,
    cluts: BTreeMap<(u16, u8), ClutDefinitionSegment>,
    objects: BTreeMap<(u16, u16), DecodedObject>,
    pes_fields: u64,
    segments_seen: u64,
    parse_errors: u64,
}

impl SubtitleState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `pid` declared a `subtitling_descriptor` and start
    /// reassembling its PES stream.
    pub fn note_pid(&mut self, pid: u16) {
        if self.pids.contains(&pid) {
            return;
        }
        self.pids.push(pid);
        self.assemblers.insert(pid, PesAssembler::new());
    }

    /// Feed one TS packet's payload. A no-op for any PID that wasn't
    /// discovered via [`Self::note_pid`].
    pub fn feed(&mut self, pid: u16, pusi: bool, payload: &[u8]) {
        let Some(asm) = self.assemblers.get_mut(&pid) else {
            return;
        };
        if let Some(completed) = asm.feed(pusi, payload) {
            self.process_pes(pid, &completed);
        }
    }

    /// Flush any PES packet still buffered at end of stream, on every
    /// tracked PID.
    pub fn flush(&mut self) {
        let pids: Vec<u16> = self.assemblers.keys().copied().collect();
        for pid in pids {
            let completed = self.assemblers.get_mut(&pid).and_then(PesAssembler::flush);
            if let Some(completed) = completed {
                self.process_pes(pid, &completed);
            }
        }
    }

    fn process_pes(&mut self, pid: u16, bytes: &[u8]) {
        let pkt = match PesPacket::parse(bytes) {
            Ok(p) => p,
            Err(_) => {
                self.parse_errors += 1;
                return;
            }
        };
        // The same PID commonly multiplexes padding_stream PES too — only
        // private_stream_1 carrying data_identifier 0x20 is DVB subtitle.
        if pkt.stream_id != PRIVATE_STREAM_1 || pkt.payload.first() != Some(&DataIdentifier) {
            return;
        }
        let field = match PesDataField::parse(pkt.payload) {
            Ok(f) => f,
            Err(_) => {
                self.parse_errors += 1;
                return;
            }
        };
        self.pes_fields += 1;
        for seg in &field.segments {
            self.segments_seen += 1;
            self.record_segment(pid, seg);
        }
    }

    fn record_segment(&mut self, pid: u16, seg: &AnySegment<'_>) {
        match seg {
            AnySegment::PageComposition(pcs) => self.pages.push((pid, pcs.clone())),
            AnySegment::RegionComposition(rcs) => {
                self.regions.insert((pid, rcs.region_id), rcs.clone());
            }
            AnySegment::ClutDefinition(cds) => {
                self.cluts.insert((pid, cds.clut_id), cds.clone());
            }
            AnySegment::ObjectData(ods) => {
                if let Some(decoded) = decode_object(ods) {
                    self.objects.insert((pid, ods.object_id), decoded);
                }
            }
            _ => {}
        }
    }

    /// Build the report, including a preview if a page/region/CLUT/object
    /// combination fully resolves.
    pub fn into_report(self) -> SubtitleReport {
        let preview = self.build_preview();
        SubtitleReport {
            pids: self.pids,
            pes_fields: self.pes_fields,
            segments_seen: self.segments_seen,
            parse_errors: self.parse_errors,
            preview,
        }
    }

    /// Walk pages → regions → objects in wire order (one subtitle component
    /// at a time), returning the first combination whose region has a
    /// resolvable CLUT and whose first object has decoded pixel rows.
    fn build_preview(&self) -> Option<SubtitlePreview> {
        for (pid, page) in &self.pages {
            for region_entry in &page.regions {
                let Some(region) = self.regions.get(&(*pid, region_entry.region_id)) else {
                    continue;
                };
                let Some(clut) = self.cluts.get(&(*pid, region.clut_id)) else {
                    continue;
                };
                for obj_entry in &region.objects {
                    let Some(decoded) = self.objects.get(&(*pid, obj_entry.object_id)) else {
                        continue;
                    };
                    if let Some(preview) = render_preview(*pid, page.page_id, region, clut, decoded)
                    {
                        return Some(preview);
                    }
                }
            }
        }
        None
    }
}

/// Decode an `object_data_segment`'s pixel data (bitmap coding only) into
/// rows of CLUT-index pixel values. Returns `None` for character-string or
/// progressive (zlib) coding — not rendered in this v1 preview — or when no
/// pixel row could be decoded.
fn decode_object(seg: &ObjectDataSegment<'_>) -> Option<DecodedObject> {
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        return None;
    };
    let mut rows = Vec::new();
    for sub in &ip.top_sub_blocks {
        let row = match sub.data_type {
            DataType::CodeString2Bit => RowData {
                depth: PixelDepth::Two,
                indices: decode_2bit_row(sub.data),
            },
            DataType::CodeString4Bit => RowData {
                depth: PixelDepth::Four,
                indices: decode_4bit_row(sub.data),
            },
            DataType::CodeString8Bit => RowData {
                depth: PixelDepth::Eight,
                indices: decode_8bit_row(sub.data),
            },
            // end_of_line marker (no data) and map-table sub-blocks (used to
            // remap CLUT indices between mixed-depth code strings within one
            // object) are not pixel rows themselves — skipped for v1.
            _ => continue,
        };
        rows.push(row);
        if rows.len() >= MAX_SUBTITLE_ROWS {
            break;
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(DecodedObject { rows })
    }
}

/// Read `n` (<=8) bits starting at bit offset `bp` in `data`; out-of-range
/// reads return 0. Mirrors the bit reader `dvb_subtitle` uses internally
/// (private to that crate) to *delimit* these same code strings — needed
/// again here because decoding the run values is a separate step the crate
/// does not expose.
fn read_bits(data: &[u8], bp: usize, n: usize) -> u16 {
    let byte_idx = bp / 8;
    let bit_idx = bp % 8;
    if byte_idx >= data.len() {
        return 0;
    }
    if bit_idx + n <= 8 {
        (u16::from(data[byte_idx]) >> (8 - bit_idx - n)) & ((1u16 << n) - 1)
    } else {
        let first_bits = 8 - bit_idx;
        let v1 = (u16::from(data[byte_idx]) & ((1u16 << first_bits) - 1)) << (n - first_bits);
        let v2 = if byte_idx + 1 < data.len() {
            u16::from(data[byte_idx + 1]) >> (8 - (n - first_bits))
        } else {
            0
        };
        v1 | v2
    }
}

/// Append `run` copies of `code`, capped at [`MAX_ROW_PIXELS`].
fn push_run(out: &mut Vec<u8>, code: u8, run: usize) {
    let room = MAX_ROW_PIXELS.saturating_sub(out.len());
    out.resize(out.len() + run.min(room), code);
}

/// Decode a 2-bit/pixel_code_string (ETSI EN 300 743 Table 22/23/42) into
/// pixel-index values. Never panics; a malformed/truncated string decodes
/// as much as it can and stops (out-of-range bit reads return 0, which is
/// itself a valid end-of-string signal, so decoding always terminates).
fn decode_2bit_row(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bp = 0usize;
    for _ in 0..MAX_DECODE_ITERATIONS {
        if out.len() >= MAX_ROW_PIXELS {
            break;
        }
        let b2 = read_bits(data, bp, 2) as u8;
        bp += 2;
        if b2 != 0 {
            out.push(b2);
            continue;
        }
        let s1 = read_bits(data, bp, 1);
        bp += 1;
        if s1 == 1 {
            // run_length_3-10: field value plus 3.
            let run = read_bits(data, bp, 3) as usize + 3;
            bp += 3;
            let code = read_bits(data, bp, 2) as u8;
            bp += 2;
            push_run(&mut out, code, run);
            continue;
        }
        let s2 = read_bits(data, bp, 1);
        bp += 1;
        if s2 == 1 {
            out.push(0);
            continue;
        }
        let s3 = read_bits(data, bp, 2);
        bp += 2;
        match s3 {
            0b00 => break, // end of 2-bit/pixel_code_string
            0b01 => {
                out.push(0);
                out.push(0);
            }
            0b10 => {
                // run_length_12-27: field value plus 12.
                let run = read_bits(data, bp, 4) as usize + 12;
                bp += 4;
                let code = read_bits(data, bp, 2) as u8;
                bp += 2;
                push_run(&mut out, code, run);
            }
            _ => {
                // run_length_29-284: field value plus 29.
                let run = read_bits(data, bp, 8) as usize + 29;
                bp += 8;
                let code = read_bits(data, bp, 2) as u8;
                bp += 2;
                push_run(&mut out, code, run);
            }
        }
    }
    out
}

/// Decode a 4-bit/pixel_code_string (ETSI EN 300 743 Table 24/25/43).
fn decode_4bit_row(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bp = 0usize;
    for _ in 0..MAX_DECODE_ITERATIONS {
        if out.len() >= MAX_ROW_PIXELS {
            break;
        }
        let b4 = read_bits(data, bp, 4) as u8;
        bp += 4;
        if b4 != 0 {
            out.push(b4);
            continue;
        }
        let s1 = read_bits(data, bp, 1);
        bp += 1;
        if s1 == 0 {
            // 3-bit field: 0 signals end_of_string; else run_length_3-9 = field + 2.
            let n3 = read_bits(data, bp, 3);
            bp += 3;
            if n3 == 0 {
                break;
            }
            push_run(&mut out, 0, n3 as usize + 2);
            continue;
        }
        let s2 = read_bits(data, bp, 1);
        bp += 1;
        if s2 == 0 {
            // run_length_4-7: field value plus 4.
            let run = read_bits(data, bp, 2) as usize + 4;
            bp += 2;
            let code = read_bits(data, bp, 4) as u8;
            bp += 4;
            push_run(&mut out, code, run);
            continue;
        }
        let s3 = read_bits(data, bp, 2);
        bp += 2;
        match s3 {
            0b00 => out.push(0),
            0b01 => {
                out.push(0);
                out.push(0);
            }
            0b10 => {
                // run_length_9-24: field value plus 9.
                let run = read_bits(data, bp, 4) as usize + 9;
                bp += 4;
                let code = read_bits(data, bp, 4) as u8;
                bp += 4;
                push_run(&mut out, code, run);
            }
            _ => {
                // run_length_25-280: field value plus 25.
                let run = read_bits(data, bp, 8) as usize + 25;
                bp += 8;
                let code = read_bits(data, bp, 4) as u8;
                bp += 4;
                push_run(&mut out, code, run);
            }
        }
    }
    out
}

/// Decode an 8-bit/pixel_code_string (ETSI EN 300 743 Table 26/44) — this one
/// is byte-, not bit-, aligned.
fn decode_8bit_row(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for _ in 0..MAX_DECODE_ITERATIONS {
        if out.len() >= MAX_ROW_PIXELS {
            break;
        }
        if pos >= data.len() {
            break;
        }
        if data[pos] != 0 {
            out.push(data[pos]);
            pos += 1;
            continue;
        }
        // 8-bit_zero.
        let Some(&b) = data.get(pos + 1) else {
            break;
        };
        let s1 = (b >> 7) & 1;
        if s1 == 0 {
            // run_length_1-127 in colour 0; a field value of 0 is the
            // end_of_string_signal.
            let run = (b & 0x7F) as usize;
            if run == 0 {
                break;
            }
            push_run(&mut out, 0, run);
            pos += 2;
        } else {
            // run_length_3-127 (literal pixel count) + an 8-bit pixel-code.
            let run = (b & 0x7F) as usize;
            let code = data.get(pos + 2).copied().unwrap_or(0);
            push_run(&mut out, code, run);
            pos += 3;
        }
    }
    out
}

/// Scale a `bits`-wide reduced-range CLUT component to full 8-bit range.
fn scale_to_8bit(value: u8, bits: u32) -> u8 {
    let max_in = (1u32 << bits) - 1;
    ((u32::from(value) * 255 + max_in / 2) / max_in) as u8
}

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Convert one CLUT entry's Y/Cr/Cb/T (ITU-R BT.601) to non-premultiplied
/// RGBA8, per ETSI EN 300 743 §7.2.4 (Table 16) semantics: `Y == 0` signals
/// full transparency regardless of the other fields; `T` is linearly
/// interpolated from 0 (opaque) to its max+1 (fully transparent).
fn ycbcr_to_rgba(entry: &ClutEntry) -> [u8; 4] {
    if entry.y_value == 0 {
        return [0, 0, 0, 0];
    }
    let (y, cr, cb, t) = if entry.full_range_flag {
        (entry.y_value, entry.cr_value, entry.cb_value, entry.t_value)
    } else {
        (
            scale_to_8bit(entry.y_value, 6),
            scale_to_8bit(entry.cr_value, 4),
            scale_to_8bit(entry.cb_value, 4),
            scale_to_8bit(entry.t_value, 2),
        )
    };
    let yf = f32::from(y);
    let crf = f32::from(cr) - 128.0;
    let cbf = f32::from(cb) - 128.0;
    let r = yf + 1.402 * crf;
    let g = yf - 0.344_136 * cbf - 0.714_136 * crf;
    let b = yf + 1.772 * cbf;
    let alpha = 255u8.saturating_sub(t);
    [clamp_u8(r), clamp_u8(g), clamp_u8(b), alpha]
}

/// Map one pixel-index at `depth` through `clut`; an index with no matching
/// (id, depth-flag) entry renders fully transparent rather than guessing a
/// colour.
fn clut_color(clut: &ClutDefinitionSegment, index: u8, depth: PixelDepth) -> [u8; 4] {
    let entry = clut.entries.iter().find(|e| {
        e.clut_entry_id == index
            && match depth {
                PixelDepth::Two => e.flag_2bit,
                PixelDepth::Four => e.flag_4bit,
                PixelDepth::Eight => e.flag_8bit,
            }
    });
    match entry {
        Some(e) => ycbcr_to_rgba(e),
        None => [0, 0, 0, 0],
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard (RFC 4648) base64 encode with `=` padding — used to cross the
/// wasm_bindgen boundary as a plain JSON string; decoded client-side with
/// the browser's built-in `atob`.
fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Render one decoded object, through `clut`, into an RGBA8 still preview.
/// `None` only if the object decoded to zero rows/columns (shouldn't happen
/// given [`decode_object`] already filters that).
fn render_preview(
    pid: u16,
    page_id: u16,
    region: &RegionCompositionSegment,
    clut: &ClutDefinitionSegment,
    decoded: &DecodedObject,
) -> Option<SubtitlePreview> {
    let width = decoded.rows.iter().map(|r| r.indices.len()).max()?;
    let height = decoded.rows.len();
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgba = vec![0u8; width * height * 4];
    for (y, row) in decoded.rows.iter().enumerate() {
        for x in 0..width {
            let color = match row.indices.get(x) {
                Some(&idx) => clut_color(clut, idx, row.depth),
                None => [0, 0, 0, 0], // short row: pad fully transparent
            };
            let o = (y * width + x) * 4;
            rgba[o..o + 4].copy_from_slice(&color);
        }
    }
    Some(SubtitlePreview {
        pid,
        page_id,
        region_id: region.region_id,
        width: width as u32,
        height: height as u32,
        rgba_base64: base64_encode(&rgba),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_is_subtitle_detects_tag_0x59() {
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::pmt::StreamType;

        // subtitling_descriptor: tag 0x59, len 8, "eng", type 0x10 (DVB
        // subtitles normal), composition_page_id=1, ancillary_page_id=2.
        let raw = [0x59, 8, b'e', b'n', b'g', 0x10, 0x00, 0x01, 0x00, 0x02];
        let stream = PmtStream {
            stream_type: StreamType::from_u8(0x06),
            elementary_pid: 0x100,
            es_info: DescriptorLoop::new(&raw),
        };
        assert!(stream_is_subtitle(&stream));

        let no_sub = PmtStream {
            stream_type: StreamType::from_u8(0x06),
            elementary_pid: 0x101,
            es_info: DescriptorLoop::new(&[]),
        };
        assert!(!stream_is_subtitle(&no_sub));
    }

    /// Hand-built 2-bit code string covering every branch in Table 22/23:
    /// direct pixel, run_length_3-10, 1-pixel-colour-0, 2-pixel-colour-0,
    /// run_length_12-27, then end-of-string.
    #[test]
    fn decode_2bit_row_covers_every_branch() {
        // Bits (MSB-first), grouped for readability:
        // 01                      -> pixel colour 1
        // 00 1 011 10              -> 2-bit_zero, switch_1=1, run=0b011+3=6, code=0b10
        // 00 0 1                   -> 2-bit_zero, switch_1=0, switch_2=1 -> 1 pixel colour 0
        // 00 0 0 01                -> ..switch_2=0, switch_3=01 -> 2 pixels colour 0
        // 00 0 0 10 0000 01        -> switch_3=10, run=0000+12=12, code=01(=1)
        // 00 0 0 00                -> switch_3=00 -> end of string
        let bits = "01\
                     001011 10\
                     0001\
                     000001\
                     000010000001\
                     000000";
        let bytes = bits_to_bytes(bits);
        let out = decode_2bit_row(&bytes);
        let mut expected = vec![1u8];
        expected.extend(std::iter::repeat_n(0b10u8, 6));
        expected.push(0);
        expected.push(0);
        expected.push(0);
        expected.extend(std::iter::repeat_n(1u8, 12));
        assert_eq!(out, expected);
    }

    #[test]
    fn decode_4bit_row_covers_every_branch() {
        // 0001                    -> pixel colour 1
        // 0000 1 0 00 1000        -> 4-bit_zero, switch_1=1, switch_2=0, run=00+4=4, code=1000(=8)
        // 0000 0 011               -> 4-bit_zero, switch_1=0, n3=011(=3) -> run=3+2=5 colour 0
        // 0000 0 000               -> end of string
        let bits = "0001\
                     000010001000\
                     00000011\
                     00000000";
        let bytes = bits_to_bytes(bits);
        let out = decode_4bit_row(&bytes);
        let mut expected = vec![1u8];
        expected.extend(std::iter::repeat_n(8u8, 4));
        expected.extend(std::iter::repeat_n(0u8, 5));
        assert_eq!(out, expected);
    }

    #[test]
    fn decode_8bit_row_covers_every_branch() {
        let bytes = [
            0x05, // pixel colour 5
            0x00, 0x03, // run_length_1-127 = 3, colour 0
            0x00, 0x83, 0x07, // run: s1=1, run=3, colour 7
            0x00, 0x00, // end of string
        ];
        let out = decode_8bit_row(&bytes);
        let mut expected = vec![5u8];
        expected.extend(std::iter::repeat_n(0u8, 3));
        expected.extend(std::iter::repeat_n(7u8, 3));
        assert_eq!(out, expected);
    }

    #[test]
    fn base64_round_trip_matches_known_vector() {
        // RFC 4648 test vector.
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn ycbcr_zero_y_is_fully_transparent() {
        let entry = ClutEntry {
            clut_entry_id: 0,
            flag_2bit: true,
            flag_4bit: false,
            flag_8bit: false,
            reserved_flags: 0,
            full_range_flag: true,
            y_value: 0,
            cr_value: 128,
            cb_value: 128,
            t_value: 0,
        };
        assert_eq!(ycbcr_to_rgba(&entry), [0, 0, 0, 0]);
    }

    #[test]
    fn ycbcr_full_range_white_opaque() {
        let entry = ClutEntry {
            clut_entry_id: 1,
            flag_2bit: true,
            flag_4bit: false,
            flag_8bit: false,
            reserved_flags: 0,
            full_range_flag: true,
            y_value: 235,
            cr_value: 128,
            cb_value: 128,
            t_value: 0,
        };
        let [r, g, b, a] = ycbcr_to_rgba(&entry);
        assert!(
            r > 200 && g > 200 && b > 200,
            "expected near-white, got {r},{g},{b}"
        );
        assert_eq!(a, 255);
    }

    /// Turn a whitespace-separated string of '0'/'1' bits into bytes,
    /// zero-padding the final byte — a compact way to write spec-table bit
    /// patterns for the decode tests above.
    fn bits_to_bytes(bits: &str) -> Vec<u8> {
        let clean: String = bits.chars().filter(|c| *c == '0' || *c == '1').collect();
        let mut out = Vec::new();
        let mut chars = clean.chars().peekable();
        while chars.peek().is_some() {
            let byte_bits: String = chars.by_ref().take(8).collect();
            let padded = format!("{byte_bits:0<8}");
            out.push(u8::from_str_radix(&padded, 2).unwrap());
        }
        out
    }
}
