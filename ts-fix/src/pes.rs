//! PES access-unit reconstruction from TS payloads — framing only, no codec
//! bitstream parsing (ISO/IEC 13818-1 §2.4.3.6).
//!
//! The public entry point is [`reconstruct_access_units`]. Each PUSI-delimited
//! PES packet on the requested PIDs becomes one [`AccessUnit`] carrying the
//! reassembled PES bytes and any PTS/DTS from the PES header.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) — §2.4.3.6 (PES packet), §2.4.3.7 (PES
//! header / PTS/DTS).

use alloc::vec::Vec;

/// A reassembled PES access unit — the complete PES packet bytes with optional
/// timing from the PES header.
///
/// The `data` field holds the **full** PES packet (from `0x00 0x00 0x01`
/// `packet_start_code_prefix` through the last `PES_packet_data_byte`). No codec
/// parsing is performed; the bytes are opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    /// PID this access unit was carried on.
    pub pid: u16,
    /// Presentation time stamp from the PES header, if present (33-bit, 90 kHz).
    pub pts: Option<u64>,
    /// Decoding time stamp from the PES header, if present (33-bit, 90 kHz).
    pub dts: Option<u64>,
    /// The reassembled PES packet bytes (`00 00 01 stream_id ...`).
    pub data: Vec<u8>,
}

/// Reconstruct PES access units on the given PIDs from a contiguous TS buffer.
///
/// Iterates 188-byte TS packets, reassembles PES payloads per PID via
/// [`mpeg_pes::PesAssembler`], and parses each completed PES packet to extract
/// PTS/DTS. Access units are returned in arrival (completion) order; AUs from
/// different PIDs may interleave.
///
/// # Panics
///
/// Panics if `ts` is not a multiple of 188 bytes (caller must pre-chop to
/// packet boundaries).
pub fn reconstruct_access_units(ts: &[u8], pids: &[u16]) -> Vec<AccessUnit> {
    const TS_PACKET_SIZE: usize = 188;

    assert_eq!(
        ts.len() % TS_PACKET_SIZE,
        0,
        "ts buffer length {} is not a multiple of 188",
        ts.len()
    );

    // Build a set for O(1) PID membership checks.
    let pid_set = {
        let mut set = alloc::collections::BTreeSet::new();
        for &pid in pids {
            set.insert(pid);
        }
        set
    };

    // Per-PID assemblers.
    let mut assemblers: alloc::collections::BTreeMap<u16, mpeg_pes::PesAssembler> =
        alloc::collections::BTreeMap::new();

    // Completed AUs, in order.
    let mut result: Vec<AccessUnit> = Vec::new();

    for chunk in ts.chunks(TS_PACKET_SIZE) {
        let raw: [u8; TS_PACKET_SIZE] = match chunk.try_into() {
            Ok(a) => a,
            Err(_) => continue, // not 188 bytes; should not happen due to the assert above
        };

        let pkt = match mpeg_ts::OwnedTsPacket::parse(raw) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if !pid_set.contains(&pkt.pid) {
            continue;
        }

        let payload = match pkt.payload() {
            Some(p) => p,
            None => continue,
        };

        let asm = assemblers.entry(pkt.pid).or_default();

        if let Some(completed) = asm.feed(pkt.pusi, payload) {
            let au = parse_au(pkt.pid, completed);
            result.push(au);
        }
    }

    // Flush any remaining PES on each PID.
    for (&pid, asm) in assemblers.iter_mut() {
        if let Some(completed) = asm.flush() {
            let au = parse_au(pid, completed);
            result.push(au);
        }
    }

    result
}

/// Parse a completed PES packet `Vec<u8>` into an `AccessUnit`.
fn parse_au(pid: u16, data: Vec<u8>) -> AccessUnit {
    let (pts, dts) = match mpeg_pes::PesPacket::parse(&data) {
        Ok(pkt) => {
            let pts = pkt.header.as_ref().and_then(|h| h.pts).map(|p| p.ticks());
            let dts = pkt.header.as_ref().and_then(|h| h.dts).map(|d| d.ticks());
            (pts, dts)
        }
        Err(_) => (None, None),
    };

    AccessUnit {
        pid,
        pts,
        dts,
        data,
    }
}
