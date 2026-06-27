//! Program Stream walker — ISO/IEC 13818-1 §2.5.3.1–2.5.3.3 (Tables 2-37, 2-38).
//!
//! Iterates through a Program Stream, yielding each [`Pack`], which itself
//! carries a [`PackHeader`], an optional
//! [`SystemHeader`], and parsed PES packets (via `mpeg-pes`).
//!
//! The stream terminates with the `MPEG_program_end_code` `0x000001B9`.

use alloc::vec::Vec;

use dvb_common::{Parse, Serialize};

use crate::pack_header::{PackHeader, PACK_START_CODE};
use crate::system_header::{SystemHeader, SYSTEM_HEADER_START_CODE};
use crate::Result;

/// `MPEG_program_end_code` — `0x000001B9`.
const PROGRAM_END_CODE: u32 = 0x0000_01B9;

/// A single pack within a Program Stream: a `pack_header()`, optionally a
/// `system_header()`, followed by zero or more PES packets.
#[derive(Debug, Clone)]
pub struct Pack<'a> {
    /// The pack header (SCR, program_mux_rate, stuffing).
    pub pack_header: PackHeader<'a>,
    /// The optional system header (only in the first pack of a compliant stream).
    pub system_header: Option<SystemHeader>,
    /// Parsed PES packets within this pack.
    pub pes_packets: Vec<mpeg_pes::PesPacket<'a>>,
}

/// Scans forward for the next pack_start_code or program_end_code boundary.
fn find_next_boundary(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 4 <= b.len() {
        let word = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        if word == PACK_START_CODE || word == PROGRAM_END_CODE {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parses a single pack from the start of `b`.
///
/// Returns `Ok((Some(pack), consumed_bytes))` on success,
/// or `Ok((None, 4))` when `MPEG_program_end_code` `0x000001B9` is reached.
pub fn parse_pack(b: &[u8]) -> Result<(Option<Pack<'_>>, usize)> {
    use crate::error::Error;

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

    // Find the next pack boundary or end_code to limit PES parsing
    let boundary = find_next_boundary(rest, 0);

    // Check for optional system header (before any PES)
    let (system_header, pes_start, _sh_len) = if rest.len() >= 4 {
        let maybe_sh = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        if maybe_sh == SYSTEM_HEADER_START_CODE {
            let sh = SystemHeader::parse(rest)?;
            let slen = sh.serialized_len();
            (Some(sh), slen, slen)
        } else {
            (None, 0, 0)
        }
    } else {
        (None, 0, 0)
    };

    let pes_data = &rest[pes_start..];
    // `boundary` is an offset within `rest`; convert to an offset within `pes_data`.
    // If the boundary falls at or before `pes_start` (e.g. a new pack_start_code that
    // appears inside what we parsed as a system-header), clamp to 0 so we yield no PES
    // data rather than underflowing.
    let pes_end = boundary.map_or(pes_data.len(), |b| {
        b.saturating_sub(pes_start).min(pes_data.len())
    });

    // Parse PES packets up to the boundary
    let (pes_packets, _pes_consumed) = parse_pes_loop(&pes_data[..pes_end])?;

    let consumed = hdr_len + pes_start + _pes_consumed;
    Ok((
        Some(Pack {
            pack_header,
            system_header,
            pes_packets,
        }),
        consumed,
    ))
}

fn parse_pes_loop<'a>(data: &'a [u8]) -> Result<(Vec<mpeg_pes::PesPacket<'a>>, usize)> {
    use crate::error::Error;

    let mut packets = Vec::new();
    let mut pos = 0;

    while pos + 6 <= data.len()
        && data[pos] == 0x00
        && data[pos + 1] == 0x00
        && data[pos + 2] == 0x01
    {
        match mpeg_pes::PesPacket::parse(&data[pos..]) {
            Ok(pkt) => {
                let pkt_len = pkt.serialized_len();
                packets.push(pkt);
                pos += pkt_len;
            }
            Err(e) => return Err(Error::Pes(e)),
        }
    }

    Ok((packets, pos))
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

#[cfg(test)]
mod tests {
    use super::parse_pack;

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
