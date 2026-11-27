//! Synthetic AC-3 / E-AC-3 syncframes built from the syntax the parsers read
//! (ETSI TS 102 366 §E.1.2.2): one copy, shared by the bitstream crate's own
//! tests and the demuxer suites that need a well-formed frame chain.
//!
//! These are generators, not parsers: they exist so a suite can hand a demuxer
//! an access unit whose `crc2` really checks out without re-implementing the
//! bit layout or the CRC next to the parser.

use broadcast_bitstream::ac3_syncframe::{AC3_SYNCWORD, ac3_crc16};
use test_fixtures::bits::BitWriter;

/// Bytes per 16-bit word — ETSI TS 102 366 §E.1.3.1.3 makes `frmsiz` a count of
/// 16-bit words, minus one.
const BYTES_PER_WORD: usize = 2;

/// Bytes of the `0x0B77` syncword, which the CRC excludes (§6.10.1).
const SYNCWORD_BYTES: usize = 2;

/// Build one E-AC-3 syncframe's `bsi()` with a zero-padded body of the declared
/// `frmsiz` and a valid `crc2`.
///
/// `chanmape` is only written for a dependent substream (`strmtyp` 1), and
/// `bsmod` only when `infomdate` is set, matching §E.1.2.2.
pub fn eac3_bsi(
    strmtyp: u8,
    substreamid: u8,
    frmsiz: u16,
    acmod: u8,
    lfeon: bool,
    fscod: u8,
    numblkscod: u8,
    chanmape: Option<u16>,
    bsmod: Option<u8>,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(16, u64::from(AC3_SYNCWORD));
    w.u(2, u64::from(strmtyp));
    w.u(3, u64::from(substreamid));
    w.u(11, u64::from(frmsiz));
    w.u(2, u64::from(fscod));
    w.u(2, u64::from(numblkscod));
    w.u(3, u64::from(acmod));
    w.u(1, u64::from(lfeon));
    w.u(5, 16); // bsid = 16 (E-AC-3)
    w.u(5, 31); // dialnorm
    w.u(1, 0); // compre
    if acmod == 0 {
        w.u(5, 31); // dialnorm2
        w.u(1, 0); // compr2e
    }
    if strmtyp == 1 {
        w.u(1, u64::from(chanmape.is_some())); // chanmape
        if let Some(map) = chanmape {
            w.u(16, u64::from(map));
        }
    }
    w.u(1, 0); // mixmdate
    w.u(1, u64::from(bsmod.is_some())); // infomdate
    if let Some(m) = bsmod {
        w.u(3, u64::from(m));
    }
    w.align_zero();
    let mut bytes = w.finish();
    let frame_len = (usize::from(frmsiz) + 1) * BYTES_PER_WORD;
    assert!(
        bytes.len() <= frame_len,
        "synthetic bsi ({} bytes) exceeds its declared frame size ({frame_len})",
        bytes.len()
    );
    bytes.resize(frame_len, 0);
    stamp_crc2(&mut bytes);
    bytes
}

/// Write a valid `crc2` into the last word of a whole E-AC-3 syncframe
/// (ETSI TS 102 366 §E.2.2 / §6.10.1): the CRC of everything between the
/// syncword and `crc2`, so the check over the whole frame leaves zero.
pub fn stamp_crc2(frame: &mut [u8]) {
    let n = frame.len();
    let crc = ac3_crc16(&frame[SYNCWORD_BYTES..n - 2]);
    frame[n - 2..].copy_from_slice(&crc.to_be_bytes());
}

/// Flip one bit in the middle of a whole frame so its `crc2` no longer checks
/// out, leaving the header's shape intact.
pub fn damaged_for_tests(mut frame: Vec<u8>) -> Vec<u8> {
    let mid = frame.len() / 2;
    frame[mid] ^= 0x10;
    frame
}

/// A `chanmap` value written the way §E.1.3.1.8 describes it: `bits` are
/// Table E.1.4 bit numbers, MSB-first, so Table E.1.4 bit 0 is the value's
/// most significant bit.
pub fn wire_chanmap(bits: &[u8]) -> u16 {
    let mut v: u16 = 0;
    for &b in bits {
        assert!(b < 16, "Table E.1.4 only defines bits 0..=15");
        v |= 1u16 << (15 - b);
    }
    v
}

/// A synthetic E-AC-3 elementary stream for a 7.1 programme: an independent
/// 5.1 frame followed by a dependent frame whose custom map carries Lrs/Rrs +
/// Cs (§E.2.8.2).
pub fn seven_one_eac3_es() -> Vec<u8> {
    let mut es = Vec::new();
    es.extend_from_slice(&eac3_bsi(0, 0, 64, 7, true, 0, 3, None, Some(0)));
    es.extend_from_slice(&eac3_bsi(
        1,
        0,
        64,
        2,
        false,
        0,
        3,
        Some(wire_chanmap(&[6, 7])),
        None,
    ));
    es
}
