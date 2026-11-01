//! The AC-3 `crc2` check on a real capture (ETSI TS 102 366 §4.4.5.2 / §6.10.1;
//! #1255).
//!
//! PID 0x82 of the `m6-*` DVB captures (`stream_type 0x06` + Enhanced AC-3
//! descriptor) carries E-AC-3 (`bsid` 16) whose payload is full of `0B 77`
//! pairs. At four of them the next five bytes read as a valid AC-3 `bsi()`
//! header (`bsid` 4 / 7 / 8 / 0), which E-AC-3 splitting accepts as an AC-3
//! syncframe standing in for independent substream 0 (Annex J.1.5.2.2). Only
//! the AC-3 CRC tells those false headers from real frames; without it the
//! garbage "frame" swallows the real E-AC-3 frame that follows (track 1 had 7
//! samples instead of 8).
//!
//! Revert-verified: with the `crc2` check in `Ec3SyncframeInfo::
//! from_ac3_independent` disabled, both tests fail (the false headers parse
//! and the demuxed track loses its eighth frame).

use broadcast_common::Unpackage;
use transmux::ac3::{Ac3SyncframeInfo, Ec3SyncframeInfo};
use transmux::pipeline::CodecConfig;
use transmux::{Error, TsDemux};

const CAPTURES: [&str; 3] = ["m6-single.ts", "m6-discontinuity.ts", "m6-duplicate.ts"];
const PID: u16 = 0x82;
/// Offsets (in the PID's reassembled payload) of the AC-3-header-shaped false
/// syncs, identical in all three captures (they share the track's payload).
const FALSE_HEADER_OFFSETS: [usize; 4] = [9165, 16764, 30064, 69270];

fn capture(name: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/ts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The PID's PES payload bytes, concatenated.
fn payload_of(ts: &[u8], pid: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut in_pes = false;
    for p in ts.chunks_exact(188) {
        if p[0] != 0x47 || (u16::from(p[1] & 0x1F) << 8 | u16::from(p[2])) != pid {
            continue;
        }
        let afc = (p[3] >> 4) & 3;
        let mut off = 4;
        if afc & 2 != 0 {
            off += 1 + usize::from(p[4]);
        }
        if afc & 1 == 0 || off >= 188 {
            continue;
        }
        let mut pl = &p[off..];
        if p[1] & 0x40 != 0 {
            in_pes = true;
            pl = &pl[9 + usize::from(pl[8])..];
        }
        if in_pes {
            out.extend_from_slice(pl);
        }
    }
    out
}

#[test]
fn false_ac3_headers_in_the_real_payload_are_rejected_by_crc2() {
    for name in CAPTURES {
        let es = payload_of(&capture(name), PID);
        let mut crc_rejected = Vec::new();
        for off in 0..es.len() - 1 {
            if es[off..off + 2] != [0x0B, 0x77] {
                continue;
            }
            let ac3 = Ac3SyncframeInfo::from_es(&es[off..]);
            let ec3 = Ec3SyncframeInfo::from_es(&es[off..]);
            // Header-shaped as AC-3 (bsid <= 8) yet not an E-AC-3 frame: the
            // reason must be the CRC, not a reserved code.
            if ac3.is_ok_and(|a| a.bsid <= 8) {
                match ec3 {
                    Err(Error::InvalidValue {
                        field: "AC-3 crc2", ..
                    }) => crc_rejected.push(off),
                    Err(_) => {}
                    Ok(e) => panic!("{name}: false AC-3 header @{off} accepted: {e:?}"),
                }
            }
        }
        assert_eq!(
            crc_rejected, FALSE_HEADER_OFFSETS,
            "{name}: AC-3-shaped false syncs rejected by crc2"
        );
    }
}

#[test]
fn the_real_eac3_frame_after_a_false_header_is_recovered() {
    for name in CAPTURES {
        let media = TsDemux::new().unpackage(&capture(name)).expect("demux");
        let track = media
            .tracks
            .iter()
            .find(|t| {
                t.spec.track_id.value() == 1 && matches!(t.spec.config, CodecConfig::Eac3 { .. })
            })
            .unwrap_or_else(|| panic!("{name}: E-AC-3 track 1"));
        assert_eq!(
            track.samples.len(),
            8,
            "{name}: 7 before the crc2 check, the eighth frame was swallowed by a false header"
        );
        for (i, s) in track.samples.iter().enumerate() {
            let info = Ec3SyncframeInfo::from_es(&s.data)
                .unwrap_or_else(|e| panic!("{name}: sample {i} not an E-AC-3 frame: {e}"));
            assert_eq!(info.bsid, 16, "{name}: sample {i} is E-AC-3 syntax");
        }
    }
}
