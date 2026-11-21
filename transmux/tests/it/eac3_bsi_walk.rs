//! The E-AC-3 `bsi()` walk past `bsid` — ETSI TS 102 366 §E.1.2.2.
//!
//! `bsmod` sits after `mixmdate`, which conditionally carries a dozen
//! sub-blocks, so the walk is exact bit accounting: a wrong condition reads
//! every following field from the wrong bits. Real captures only take the
//! common path (acmod 2/7, no 1+1 mode, no mixing metadata), so these frames
//! are built from the syntax itself, and every case ends with `infomdate` 1
//! and `bsmod` 0b101, which makes any desynchronisation visible.

use crate::bit_writer::BitWriter;
use transmux::Ec3SyncframeInfo;

const SYNCWORD: u64 = 0x0B77;
const BSID: u64 = 16;
const BSMS: u8 = 0b101;

/// An E-AC-3 syncframe whose `tail` writes everything between `dialnorm` and
/// `infomdate` (starting with `compre`). `frmsiz` and the padding past `bsmod`
/// are arbitrary: this test is about the walk, not the frame length.
fn frame(
    strmtyp: u8,
    acmod: u8,
    lfeon: bool,
    numblkscod: u8,
    tail: impl FnOnce(&mut BitWriter),
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(16, SYNCWORD);
    w.u(2, u64::from(strmtyp));
    w.u(3, 0); // substreamid
    w.u(11, 64); // frmsiz
    w.u(2, 0); // fscod 48 kHz
    w.u(2, u64::from(numblkscod));
    w.u(3, u64::from(acmod));
    w.u(1, u64::from(lfeon));
    w.u(5, BSID);
    w.u(5, 31); // dialnorm
    tail(&mut w);
    w.u(1, 1); // infomdate
    w.u(3, u64::from(BSMS));
    w.u(16, 0); // padding: a desynchronised walk still has bits to misread
    w.finish()
}

/// Parse the frame and assert the walk landed on `bsmod` and `acmod`.
fn assert_walk(
    label: &str,
    strmtyp: u8,
    acmod: u8,
    lfeon: bool,
    numblkscod: u8,
    tail: impl FnOnce(&mut BitWriter),
) {
    let es = frame(strmtyp, acmod, lfeon, numblkscod, tail);
    let info = Ec3SyncframeInfo::from_es(&es).unwrap_or_else(|e| panic!("{label}: {e}"));
    assert_eq!(info.acmod, acmod, "{label}");
    assert_eq!(info.bsmod, BSMS, "{label}: the walk desynchronised");
}

/// 1+1 mode carries `dialnorm2` and an optional `compr2`; skipping the latter
/// moves `mixmdate` and everything after it.
#[test]
fn compr2_in_one_plus_one_mode() {
    assert_walk("acmod 0 compr2", 0, 0, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(5, 20); // dialnorm2
        w.u(1, 1); // compr2e
        w.u(8, 0xA5); // compr2
        w.u(1, 0); // mixmdate
    });
}

/// `ltrtcmixlev`/`lorocmixlev` are carried when `(acmod & 0x1) && (acmod > 2)`
/// and `ltrtsurmixlev`/`lorosurmixlev` when `(acmod & 0x4)`, so each layout
/// skips a different width.
#[test]
fn mixmdate_mix_levels_per_acmod() {
    for acmod in [3u8, 4, 5] {
        assert_walk("mix levels", 0, acmod, false, 3, |w| {
            w.u(1, 0); // compre
            w.u(1, 1); // mixmdate
            w.u(2, 2); // dmixmod
            if acmod & 0x1 != 0 && acmod > 0x2 {
                w.u(3, 1); // ltrtcmixlev
                w.u(3, 2); // lorocmixlev
            }
            if acmod & 0x4 != 0 {
                w.u(3, 3); // ltrtsurmixlev
                w.u(3, 3); // lorosurmixlev
            }
            w.u(1, 0); // pgmscle
            w.u(1, 0); // extpgmscle
            w.u(2, 0); // mixdef
            w.u(1, 0); // frmmixcfginfoe
        });
    }
}

/// Every `mixdef` variant: 0x1 is a fixed 5 bits, 0x2 a fixed 12, 0x3 the
/// option-4 block (`mixdeflen`, `mixdata2e`/`mixdata3e`, a byte-aligned
/// `mixdata` payload and its fill). `pgmscle`/`extpgmscle` are set on the way
/// in, so their optional values are skipped too.
#[test]
fn mixdef_variants_walk_to_bsmod() {
    for mixdef in 0u64..=3 {
        assert_walk("mixdef", 0, 7, true, 3, |w| {
            w.u(1, 0); // compre
            w.u(1, 1); // mixmdate
            w.u(2, 0); // dmixmod
            w.u(3, 0); // ltrtcmixlev
            w.u(3, 0); // lorocmixlev
            w.u(3, 0); // ltrtsurmixlev
            w.u(3, 0); // lorosurmixlev
            w.u(1, 1); // lfemixlevcode
            w.u(5, 3); // lfemixlevcod
            w.u(1, 1); // pgmscle
            w.u(6, 0x2A); // pgmscl
            w.u(1, 1); // extpgmscle
            w.u(6, 0x15); // extpgmscl
            w.u(2, mixdef);
            match mixdef {
                1 => {
                    w.u(1, 1); // premixcmpsel
                    w.u(1, 0); // drcsrc
                    w.u(3, 5); // premixcmpscl
                }
                2 => {
                    w.u(12, 0xABC); // mixdata
                }
                3 => {
                    w.u(5, 1); // mixdeflen -> 8 * (1 + 2) bits of mixdata
                    w.u(1, 1); // mixdata2e
                    w.u(1, 0); // premixcmpsel
                    w.u(1, 0); // drcsrc
                    w.u(3, 0); // premixcmpscl
                    for present in [false, true, false, false, true, false, true] {
                        w.u(1, u64::from(present));
                        if present {
                            w.u(4, 0b1010);
                        }
                    }
                    w.u(1, 1); // addche
                    w.u(1, 1); // extpgmaux1scle
                    w.u(4, 0b1100); // extpgmaux1scl
                    w.u(1, 0); // extpgmaux2scle
                    w.u(1, 1); // mixdata3e
                    w.u(5, 0b01011); // spchdat
                    w.u(1, 0); // addspchdate
                    w.u(24, 0); // mixdata
                    w.align_zero(); // mixdatafill
                }
                _ => {}
            }
            w.u(1, 0); // frmmixcfginfoe
        });
    }
}

/// 1+1 mode carries `pgmscl2` behind `pgmscl2e`.
#[test]
fn pgmscl2_in_one_plus_one_mode() {
    assert_walk("acmod 0 pgmscl2", 0, 0, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(5, 31); // dialnorm2
        w.u(1, 0); // compr2e
        w.u(1, 1); // mixmdate
        w.u(1, 0); // pgmscle
        w.u(1, 1); // pgmscl2e
        w.u(6, 0x2A); // pgmscl2
        w.u(1, 0); // extpgmscle
        w.u(2, 0); // mixdef
        w.u(1, 0); // paninfoe
        w.u(1, 0); // paninfo2e
        w.u(1, 0); // frmmixcfginfoe
    });
}

/// The `paninfo` block is carried only when `acmod < 2`, and `paninfo2` only
/// in 1+1 mode: a layout that wrongly takes the block reads `frmmixcfginfoe`
/// and `infomdate` out of the following bits.
#[test]
fn paninfo_block_per_acmod() {
    assert_walk("acmod 0 paninfo", 0, 0, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(5, 31); // dialnorm2
        w.u(1, 0); // compr2e
        w.u(1, 1); // mixmdate
        w.u(1, 0); // pgmscle
        w.u(1, 0); // pgmscl2e
        w.u(1, 0); // extpgmscle
        w.u(2, 0); // mixdef
        w.u(1, 1); // paninfoe
        w.u(8, 0xA5); // panmean
        w.u(6, 0x2A); // paninfo
        w.u(1, 1); // paninfo2e
        w.u(8, 0x5A); // panmean2
        w.u(6, 0x15); // paninfo2
        w.u(1, 0); // frmmixcfginfoe
    });
    assert_walk("acmod 1 paninfo", 0, 1, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(1, 1); // mixmdate
        w.u(1, 0); // pgmscle
        w.u(1, 0); // extpgmscle (acmod != 0: no pgmscl2e)
        w.u(2, 0); // mixdef
        w.u(1, 1); // paninfoe
        w.u(8, 0x3C); // panmean
        w.u(6, 0x11); // paninfo
        w.u(1, 0); // frmmixcfginfoe (acmod != 0: no paninfo2)
    });
    assert_walk("acmod 2 paninfo", 0, 2, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(1, 1); // mixmdate
        w.u(1, 0); // pgmscle
        w.u(1, 0); // extpgmscle
        w.u(2, 0); // mixdef
        w.u(1, 0); // frmmixcfginfoe
    });
}

/// `frmmixcfginfo`: one 5-bit `blkmixcfginfo` when the frame has a single
/// block (`numblkscod` 0), otherwise one presence flag (and its 5-bit value)
/// per block.
#[test]
fn frame_mix_config_per_block() {
    assert_walk("one block", 0, 7, false, 0, |w| {
        w.u(1, 0); // compre
        w.u(1, 1); // mixmdate
        w.u(2, 0); // dmixmod
        w.u(3, 0); // ltrtcmixlev
        w.u(3, 0); // lorocmixlev
        w.u(3, 0); // ltrtsurmixlev
        w.u(3, 0); // lorosurmixlev
        w.u(1, 0); // pgmscle
        w.u(1, 0); // extpgmscle
        w.u(2, 0); // mixdef
        w.u(1, 1); // frmmixcfginfoe
        w.u(5, 0b10110); // blkmixcfginfo[0]
    });
    assert_walk("six blocks", 0, 7, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(1, 1); // mixmdate
        w.u(2, 0); // dmixmod
        w.u(3, 0); // ltrtcmixlev
        w.u(3, 0); // lorocmixlev
        w.u(3, 0); // ltrtsurmixlev
        w.u(3, 0); // lorosurmixlev
        w.u(1, 0); // pgmscle
        w.u(1, 0); // extpgmscle
        w.u(2, 0); // mixdef
        w.u(1, 1); // frmmixcfginfoe
        for block in 0..6 {
            let present = block == 2 || block == 5;
            w.u(1, u64::from(present));
            if present {
                w.u(5, 0b10110); // blkmixcfginfo[block]
            }
        }
    });
}

/// A dependent substream's custom channel map is read behind `chanmape` and
/// reported as-is (§E.1.3.1.8); its frames carry no programme-mix block, so
/// `infomdate` follows `mixmdate` directly.
#[test]
fn a_dependent_frame_reports_its_chanmap() {
    let es = frame(1, 2, false, 3, |w| {
        w.u(1, 0); // compre
        w.u(1, 1); // chanmape
        w.u(16, 0x1A00); // chanmap: Ls, Rs, Lrs/Rrs (Table E.1.4)
        w.u(1, 0); // mixmdate
    });
    let info = Ec3SyncframeInfo::from_es(&es).unwrap();
    assert_eq!(info.strmtyp, 1);
    assert_eq!(info.chanmap, Some(0x1A00));
    assert_eq!(info.bsmod, BSMS);
}
