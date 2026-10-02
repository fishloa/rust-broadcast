//! Program Stream walker — ISO/IEC 13818-1 §2.5.3.1–2.5.3.3 (Tables 2-37, 2-38).
//!
//! Iterates through a Program Stream, yielding each [`Pack`], which itself
//! carries a [`PackHeader`], an optional
//! [`SystemHeader`], and parsed PES packets (via `mpeg-pes`).
//!
//! The stream terminates with the `MPEG_program_end_code` `0x000001B9`.

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::Result;
use crate::error::Error;
use crate::pack_header::{PACK_START_CODE, PackHeader};
use crate::program_stream_map::{MAP_STREAM_ID, ProgramStreamMap};
use crate::system_header::{
    PREFIX_LEN as SYSTEM_HEADER_PREFIX_LEN, SYSTEM_HEADER_START_CODE, SystemHeader,
};

/// `MPEG_program_end_code` — `0x000001B9`.
const PROGRAM_END_CODE: u32 = 0x0000_01B9;

/// A single pack within a Program Stream: a `pack_header()`, optionally a
/// `system_header()`, an optional Program Stream Map, and zero or more PES
/// packets.
#[derive(Debug, Clone)]
pub struct Pack<'a> {
    /// The pack header (SCR, program_mux_rate, stuffing).
    pub pack_header: PackHeader<'a>,
    /// The optional system header (only in the first pack of a compliant stream).
    pub system_header: Option<SystemHeader>,
    /// The Program Stream Map (`stream_id 0xBC`), if one appears in this
    /// pack — `stream_type → elementary_stream_id` mapping (Table 2-41). If
    /// more than one PSM packet appears (a version-change re-announcement),
    /// this holds the last one.
    ///
    /// W5 (#1119): PSM packets used to be handed to `mpeg_pes::PesPacket::parse`
    /// like any other PES and returned as an opaque, un-mapped PES packet —
    /// so a consumer had no way to learn `stream_type` per elementary
    /// stream and had to guess the codec from `stream_id` alone.
    pub psm: Option<ProgramStreamMap<'a>>,
    /// Parsed PES packets within this pack (PSM packets are not included
    /// here — see [`psm`](Self::psm)).
    pub pes_packets: Vec<mpeg_pes::PesPacket<'a>>,
}

/// Parses a single pack from the start of `b`.
///
/// Returns `Ok((Some(pack), consumed_bytes))` on success,
/// or `Ok((None, 4))` when `MPEG_program_end_code` `0x000001B9` is reached.
pub fn parse_pack(b: &[u8]) -> Result<(Option<Pack<'_>>, usize)> {
    if b.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: b.len(),
            what: "pack start_code or end_code",
        });
    }

    let start = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    if start == PROGRAM_END_CODE {
        return Ok((None, 4));
    }

    // Parse pack header
    let pack_header = PackHeader::parse(b)?;
    let hdr_len = pack_header.header_len();
    let rest = &b[hdr_len..];

    // Check for optional system header (before any PES)
    let (system_header, pes_start) = if rest.len() >= 4 {
        let maybe_sh = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        if maybe_sh == SYSTEM_HEADER_START_CODE {
            let sh = SystemHeader::parse(rest)?;
            // W2 (#1119): PES data starts after the *wire* `header_length`
            // (Table 2-40), not after however many bytes the re-serialized
            // stream-bound loop happens to occupy. A conformant
            // `header_length` may declare trailing bytes past the loop
            // (the loop ends at the first byte whose MSB is 0, which the
            // encoder is free to pad with reserved bytes); starting PES
            // parsing early there feeds PES data into the system-header
            // parser's leftover bytes instead.
            let header_length = u16::from_be_bytes([rest[4], rest[5]]) as usize;
            (Some(sh), SYSTEM_HEADER_PREFIX_LEN + header_length)
        } else {
            (None, 0)
        }
    } else {
        (None, 0)
    };

    let pes_data = &rest[pes_start..];
    let (pes_packets, psm, pes_consumed) = parse_pes_loop(pes_data)?;

    let consumed = hdr_len + pes_start + pes_consumed;
    Ok((
        Some(Pack {
            pack_header,
            system_header,
            psm,
            pes_packets,
        }),
        consumed,
    ))
}

/// Parse PES packets from `data` until a `pack_start_code`/`program_end_code`
/// or non-PES-start bytes are found — always checked exactly at the boundary
/// right after a previously-consumed *whole* PES packet, never scanned for
/// mid-payload.
///
/// W3 (#1119): the caller used to pre-scan the whole buffer byte-by-byte for
/// the next `000001BA`/`000001B9`, including inside PES payloads. Several
/// private/audio stream types (AC-3, LPCM, DVD subpictures carried as
/// `private_stream_1`) are not start-code-emulation-free, so a chance
/// `00 00 01 BA` inside real payload bytes truncated the PES loop early,
/// leaving the last real PES packet's tail unparsed and the whole pack
/// (and, via `parse_all_packs`, the whole stream) rejected with a spurious
/// `BufferTooShort`/`Err`. Checking for the boundary only right after a
/// fully-parsed PES packet (by its own declared `PES_packet_length`) cannot
/// see mid-payload emulation at all.
fn parse_pes_loop(
    data: &[u8],
) -> Result<(
    Vec<mpeg_pes::PesPacket<'_>>,
    Option<ProgramStreamMap<'_>>,
    usize,
)> {
    let mut packets = Vec::new();
    let mut psm = None;
    let mut pos = 0;

    loop {
        if pos + 4 <= data.len() {
            let word = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            if word == PACK_START_CODE || word == PROGRAM_END_CODE {
                break;
            }
        }
        if !(pos + 6 <= data.len()
            && data[pos] == 0x00
            && data[pos + 1] == 0x00
            && data[pos + 2] == 0x01)
        {
            break;
        }
        // W5 (#1119): a Program Stream Map (`stream_id 0xBC`) is a distinct
        // self-delimiting structure (Table 2-41, with its own CRC-32
        // trailer, not a `PES_packet_length`) — route it to
        // `ProgramStreamMap::parse` instead of `mpeg_pes::PesPacket::parse`,
        // which had no way to make sense of its layout and returned it as
        // an opaque, un-mapped PES packet.
        if data[pos + 3] == MAP_STREAM_ID {
            let map = ProgramStreamMap::parse(&data[pos..])?;
            pos += map.serialized_len();
            psm = Some(map);
            continue;
        }
        match mpeg_pes::PesPacket::parse(&data[pos..]) {
            Ok(pkt) => {
                let pkt_len = pkt.serialized_len();
                packets.push(pkt);
                pos += pkt_len;
            }
            Err(e) => return Err(Error::Pes(e)),
        }
    }

    Ok((packets, psm, pos))
}

/// Iterate over all packs in a Program Stream buffer.
///
/// Returns all packs and the remaining trailing bytes (if any).
pub fn parse_all_packs(b: &[u8]) -> Result<(Vec<Pack<'_>>, &[u8])> {
    let mut packs = Vec::new();
    let mut remaining = b;
    while remaining.len() >= 4 {
        let (pack_opt, consumed) = parse_pack(remaining)?;
        match pack_opt {
            Some(pack) => {
                remaining = &remaining[consumed..];
                packs.push(pack);
            }
            None => {
                // End code consumed 4 bytes; finish
                remaining = &remaining[4..];
                break;
            }
        }
    }
    Ok((packs, remaining))
}

/// A span of the input [`scan_packs`] skipped because the pack starting
/// there did not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRegion {
    /// Byte offset (from the start of the scanned buffer) where the failed
    /// pack began.
    pub offset: usize,
    /// Bytes skipped: up to the next `pack_start_code` found after
    /// `offset`, or to the end of the buffer if none follows.
    pub len: usize,
    /// Why the pack at `offset` failed to parse.
    pub error: Error,
}

/// The outcome of [`scan_packs`].
#[derive(Debug, Clone)]
pub struct PackScan<'a> {
    /// Every pack that parsed, in stream order.
    pub packs: Vec<Pack<'a>>,
    /// Every region skipped after a parse failure, in stream order.
    pub skipped: Vec<SkippedRegion>,
    /// Bytes after the last pack / `MPEG_program_end_code` (fewer than a
    /// start code, or whatever followed the end code).
    pub remaining: &'a [u8],
}

/// Walk a Program Stream like [`parse_all_packs`], but **resynchronise** after
/// a pack that fails to parse instead of abandoning the whole stream.
///
/// On a parse failure the failed span is recorded in
/// [`PackScan::skipped`] (with the error, never swallowed) and scanning
/// resumes at the next `pack_start_code` (`0x000001BA`) after the failed
/// pack's first byte. That byte search is deliberately confined to this
/// recovery path: a well-formed stream is still walked purely by
/// `PES_packet_length` (see [`parse_pack`]), so start-code emulation inside a
/// payload cannot truncate a good pack — only a stream that is *already*
/// corrupt can resync onto an emulated code (audit r14-MPS-W3, #1119).
pub fn scan_packs(b: &[u8]) -> PackScan<'_> {
    let mut packs = Vec::new();
    let mut skipped = Vec::new();
    let mut pos = 0usize;
    while b.len() - pos >= 4 {
        match parse_pack(&b[pos..]) {
            Ok((Some(pack), consumed)) => {
                packs.push(pack);
                pos += consumed;
            }
            Ok((None, consumed)) => {
                pos += consumed;
                break;
            }
            Err(error) => {
                let next = find_pack_start(&b[pos + 1..]).map(|i| pos + 1 + i);
                let end = next.unwrap_or(b.len());
                skipped.push(SkippedRegion {
                    offset: pos,
                    len: end - pos,
                    error,
                });
                pos = end;
            }
        }
    }
    PackScan {
        packs,
        skipped,
        remaining: &b[pos..],
    }
}

/// Offset of the first `pack_start_code` in `data`, if any.
fn find_pack_start(data: &[u8]) -> Option<usize> {
    let code = PACK_START_CODE.to_be_bytes();
    data.windows(code.len()).position(|w| w == code)
}

#[cfg(test)]
mod tests {
    use super::parse_pack;
    use crate::program_stream_map::{EsMapEntry, ProgramStreamMap};
    use alloc::{vec, vec::Vec};
    use broadcast_common::Serialize;

    /// W5 (#1119): a PSM (`stream_id 0xBC`) within a pack must be surfaced
    /// via `Pack::psm`, typed (`stream_type` per elementary stream), not
    /// folded into `pes_packets` as an opaque, un-mapped PES packet.
    #[test]
    fn psm_is_surfaced_not_folded_into_pes_packets() {
        let psm = ProgramStreamMap {
            current_next_indicator: true,
            single_extension_stream_flag: false,
            version: 1,
            program_stream_info: &[],
            elementary_stream_map: vec![EsMapEntry {
                stream_type: 0x02, // MPEG-2 video
                elementary_stream_id: 0xE0,
                stream_id_extension: None,
                descriptors: &[],
            }],
            crc: 0,
        };
        let mut psm_bytes = vec![0u8; psm.serialized_len()];
        psm.serialize_into(&mut psm_bytes).unwrap();

        let mut b: Vec<u8> = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
        ];
        b.extend_from_slice(&psm_bytes);
        // One PES packet after the PSM.
        b.extend_from_slice(&[
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0xAA, 0xBB,
        ]);

        let (pack_opt, consumed) = parse_pack(&b).unwrap();
        let pack = pack_opt.expect("not an end code");
        let parsed_psm = pack.psm.expect("PSM must be surfaced on Pack::psm");
        assert_eq!(parsed_psm.elementary_stream_map.len(), 1);
        assert_eq!(parsed_psm.elementary_stream_map[0].stream_type, 0x02);
        assert_eq!(
            pack.pes_packets.len(),
            1,
            "the PSM must not appear (again, mis-typed) in pes_packets"
        );
        assert_eq!(consumed, b.len());
    }

    /// W2 (#1119): PES data must start after the system header's *wire*
    /// `header_length` (Table 2-40), not after its re-serialized length.
    /// A `header_length` of 8 (6-byte fixed body + 2 reserved padding bytes,
    /// both MSB=0 so the `while (nextbits()=='1')` stream-bound loop parses
    /// 0 entries) is 2 bytes longer than the 6-byte fixed body the loop
    /// itself accounts for — the re-serialized length was used instead,
    /// starting PES parsing 2 bytes early, inside the declared header.
    #[test]
    fn system_header_wire_length_used_for_pes_start_not_reserialized_length() {
        // Pack header, byte-identical to `pack_header::pack_header_round_trip_fixture_pattern`
        // (14 bytes, stuffing_length=0).
        let mut b: Vec<u8> = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
        ];
        // System header: header_length = 8 (2 bytes more than the 6-byte
        // fixed body its own loop consumes).
        b.extend_from_slice(&[
            0x00, 0x00, 0x01, 0xBB, // system_header_start_code
            0x00, 0x08, // header_length = 8
            0x80, 0x00, 0x01, 0x00, 0x20, 0x00, // 6-byte fixed body, 0 stream bounds
            0x00, 0x00, // 2 reserved padding bytes (MSB=0)
        ]);
        // A minimal PES packet (stream_id 0xE0, PES_packet_length=0x0A, PTS-only, 2 payload bytes).
        b.extend_from_slice(&[
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0xAA, 0xBB,
        ]);

        let (pack_opt, consumed) = parse_pack(&b).unwrap();
        let pack = pack_opt.expect("not an end code");
        assert!(pack.system_header.is_some());
        assert_eq!(
            pack.pes_packets.len(),
            1,
            "PES must be found starting after the full declared header_length"
        );
        assert_eq!(consumed, b.len());
    }

    /// W3 (#1119): a stray `000001BA` inside a PES packet's own (declared,
    /// bounded) payload must not truncate the pack — `pack_start_code`/
    /// `program_end_code` are only ever checked right after a fully-parsed
    /// PES packet, never scanned for mid-payload.
    #[test]
    fn stray_pack_start_code_inside_pes_payload_does_not_truncate() {
        let mut b: Vec<u8> = vec![
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
        ];
        // One PES packet, PES_packet_length=0x0E (14): flags(2)+hdl(1)+PTS(5)+
        // payload(6), where the payload embeds a stray `00 00 01 BA` at its
        // midpoint — plausible in start-code-emulation-bearing streams (AC-3,
        // LPCM, DVD subpicture `private_stream_1`).
        b.extend_from_slice(&[
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0E, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0xAA, 0x00, 0x00, 0x01, 0xBA, 0xBB,
        ]);

        let (pack_opt, consumed) = parse_pack(&b).unwrap();
        let pack = pack_opt.expect("not an end code");
        assert_eq!(
            pack.pes_packets.len(),
            1,
            "the one PES packet must parse in full, not truncate at the embedded 000001BA"
        );
        assert_eq!(consumed, b.len(), "the whole PES packet must be consumed");
    }

    /// Regression: a system_header whose serialized length (`pes_start`) is
    /// greater than the offset of the next boundary found in `rest` caused
    /// `b - pes_start` to subtract with overflow (panic) on the unsigned
    /// `pes_end` computation.  Fixed by using `saturating_sub`.
    ///
    /// Verbatim cargo-fuzz minimized artifact. A `system_header` declares
    /// `header_length` = 255, so its re-serialized length (`pes_start` = 261)
    /// covers a large parsed stream loop; meanwhile `find_next_boundary`
    /// returns a `pack_start_code` (0x000001BA) embedded at rest-offset 142,
    /// i.e. *before* `pes_start`. The old `b - pes_start` underflowed (panic on
    /// unsigned subtraction). Fixed with `saturating_sub`. A truncated input
    /// does NOT reproduce — `SystemHeader::parse` rejects it with
    /// `HeaderLengthOverflow` before the buggy line, so the full body is needed.
    #[test]
    fn fuzz_regression_mpeg_ps_boundary_underflow() {
        let crashing: &[u8] = &[
            0x00, 0x00, 0x01, 0xba, 0x5c, 0xf5, 0xf5, 0xc0, 0xff, 0xff, 0x21, 0xf3, 0xf3, 0xf3,
            0x90, 0xf3, 0xbb, 0x00, 0x00, 0x01, 0xbb, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x01, 0xba, 0x5c,
            0xf5, 0xf5, 0xc0, 0xff, 0xff, 0xf3, 0xf3, 0xf3, 0xf3, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        // Must not panic — result is either Ok or Err.
        let _ = parse_pack(crashing);
    }
}
