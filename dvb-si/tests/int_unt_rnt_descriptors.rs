//! INT / UNT / RNT scoped descriptor namespaces on reference-toolkit data
//! (issue #1285).
//!
//! The only INT/UNT/RNT sections in the real-capture corpus carrying these
//! descriptors are the ones TSDuck's `tstabcomp` encodes from its own XML
//! (`tsduck-test`, BSD-2): `reference/test-022` (an INT with all 15 INT
//! descriptors), `reference/test-060` (a UNT with the UNT descriptors and the
//! shared target descriptors) and `reference/test-115` (an RNT with
//! `RAR_over_DVB_stream`, `RAR_over_IP` and `RNT_scan`). They are the
//! cross-tool oracle, NOT broadcast captures: no broadcast capture in the
//! corpus (hotbird, france-tnt, the tsduck inputs) carries any of them except
//! the plain INT `input/test-016.ts`, whose loop descriptors are walked here
//! too. The hotbird captures were scanned: every table_id-0x4B/0x4C/0x79
//! look-alike there fails its CRC (scrambled payload), so none is a section.
//!
//! For every section: the whole section re-serializes byte-identically; every
//! loop walks with no parse error; every typed descriptor re-serializes to the
//! exact original entry bytes; and the set of descriptor names the sections
//! really contain is asserted (so a descriptor silently falling to `Unknown`
//! or `Si` fails).
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::int::AnyIntDescriptor;
use dvb_si::descriptors::rnt::AnyRntDescriptor;
use dvb_si::descriptors::unt::AnyUntDescriptor;
use dvb_si::tables::int::IntSection;
use dvb_si::tables::rnt::RntSection;
use dvb_si::tables::unt::UntSection;
use std::collections::BTreeSet;

const TS_LEN: usize = 188;
const SYNC: u8 = 0x47;

fn crc_ok(sec: &[u8]) -> bool {
    broadcast_common::crc32_mpeg2::compute(sec) == 0
}

/// Minimal PSI reassembly (pointer_field aware) of a 188-byte-packet file;
/// returns every CRC-valid section whose table_id is `tid`.
fn sections_of(ts: &[u8], tid: u8) -> Vec<Vec<u8>> {
    use std::collections::BTreeMap;
    let mut open: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    let mut out: Vec<Vec<u8>> = Vec::new();
    let take = |buf: &mut Vec<u8>, out: &mut Vec<Vec<u8>>| {
        while buf.len() >= 3 && buf[0] != 0xFF {
            let len = ((usize::from(buf[1]) & 0x0F) << 8 | usize::from(buf[2])) + 3;
            if buf.len() < len {
                return;
            }
            let sec: Vec<u8> = buf.drain(..len).collect();
            if sec[0] == tid && crc_ok(&sec) && !out.contains(&sec) {
                out.push(sec);
            }
        }
        buf.clear();
    };
    for p in ts.chunks_exact(TS_LEN) {
        if p[0] != SYNC {
            continue;
        }
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        let afc = (p[3] >> 4) & 3;
        if afc & 1 == 0 {
            continue;
        }
        let mut i = 4;
        if afc & 2 != 0 {
            i += 1 + usize::from(p[4]);
        }
        let Some(pl) = p.get(i..).filter(|s| !s.is_empty()) else {
            continue;
        };
        if p[1] & 0x40 != 0 {
            let ptr = usize::from(pl[0]);
            if let Some(buf) = open.get_mut(&pid) {
                buf.extend_from_slice(&pl[1..(1 + ptr).min(pl.len())]);
                take(buf, &mut out);
            }
            let mut buf = pl[(1 + ptr).min(pl.len())..].to_vec();
            take(&mut buf, &mut out);
            if !buf.is_empty() {
                open.insert(pid, buf);
            } else {
                open.remove(&pid);
            }
        } else if let Some(buf) = open.get_mut(&pid) {
            buf.extend_from_slice(pl);
            take(buf, &mut out);
        }
    }
    out
}

/// Split a raw loop into its `(tag, full entry bytes)` TLV entries.
fn entries(raw: &[u8]) -> Vec<(u8, &[u8])> {
    let mut v = Vec::new();
    let mut p = 0;
    while p + 2 <= raw.len() {
        let end = p + 2 + usize::from(raw[p + 1]);
        v.push((raw[p], &raw[p..end]));
        p = end;
    }
    assert_eq!(p, raw.len(), "loop is not a whole TLV sequence");
    v
}

fn ser<T: Serialize>(t: &T) -> Vec<u8>
where
    T::Error: std::fmt::Debug,
{
    let mut b = vec![0u8; t.serialized_len()];
    t.serialize_into(&mut b).expect("serialize");
    b
}

fn walk_int(raw: &[u8], names: &mut BTreeSet<&'static str>) {
    let typed: Vec<_> = dvb_si::descriptors::int::parse_int_loop(raw).collect();
    for ((tag, full), d) in entries(raw).into_iter().zip(typed) {
        // TSDuck encodes an *empty* IP/MAC_generic_stream_location selector by
        // omitting the mandatory `selector_length_flag` byte (7-byte body, EN 301 192
        // V1.7.1 Table 33 requires 8): a spec violation we report as a parse error.
        if tag == 0x15 && full.len() == 2 + 7 {
            assert!(
                d.is_err(),
                "7-byte generic_stream_location must be rejected"
            );
            continue;
        }
        let d = d.unwrap_or_else(|e| panic!("INT tag {tag:#04x}: {e}"));
        assert!(
            !matches!(d, AnyIntDescriptor::Unknown { .. }),
            "INT tag {tag:#04x} fell to Unknown"
        );
        assert_eq!(d.reserialize().unwrap().unwrap(), full, "INT {}", d.name());
        names.insert(d.name());
    }
}

fn walk_unt(raw: &[u8], names: &mut BTreeSet<&'static str>) {
    let typed: Vec<_> = dvb_si::descriptors::unt::parse_unt_loop(raw).collect();
    for ((tag, full), d) in entries(raw).into_iter().zip(typed) {
        let d = d.unwrap_or_else(|e| panic!("UNT tag {tag:#04x}: {e}"));
        assert!(
            !matches!(d, AnyUntDescriptor::Unknown { .. }),
            "UNT tag {tag:#04x} fell to Unknown"
        );
        assert_eq!(d.reserialize().unwrap().unwrap(), full, "UNT {}", d.name());
        names.insert(d.name());
    }
}

fn walk_rnt(raw: &[u8], names: &mut BTreeSet<&'static str>) {
    let typed: Vec<_> = dvb_si::descriptors::rnt::parse_rnt_loop(raw).collect();
    for ((tag, full), d) in entries(raw).into_iter().zip(typed) {
        let d = d.unwrap_or_else(|e| panic!("RNT tag {tag:#04x}: {e}"));
        assert_eq!(d.reserialize().unwrap().unwrap(), full, "RNT {}", d.name());
        if matches!(d, AnyRntDescriptor::Si(_)) {
            assert!(
                !(0x40..=0x42).contains(&tag),
                "RNT 0x40-0x42 must never reach SI"
            );
        }
        names.insert(d.name());
    }
}

fn load(rel: &str) -> Option<Vec<u8>> {
    let p = test_fixtures::tsduck(rel, "int_unt_rnt_descriptors")?;
    Some(std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())))
}

/// TSDuck test-022: an INT with all 15 INT descriptors (plus the empty
/// variants TSDuck also emits). TSDuck's INT XML has a single descriptor
/// loop, so they are all in the platform loop.
#[test]
fn tsduck_test_022_int_has_every_int_descriptor() {
    let Some(ts) = load("reference/test-022/test-022.pack.ts") else {
        return;
    };
    let secs = sections_of(&ts, 0x4C);
    assert_eq!(secs.len(), 1, "one INT section");
    let int = IntSection::parse(&secs[0]).unwrap();
    assert_eq!(ser(&int), secs[0], "whole-section byte identity");
    assert_eq!(int.platform_id, 0x0012_3456);
    let mut names = BTreeSet::new();
    walk_int(int.platform_descriptors.raw(), &mut names);
    let expected: BTreeSet<&str> = [
        "TARGET_SMARTCARD",
        "TARGET_MAC_ADDRESS",
        "TARGET_SERIAL_NUMBER",
        "TARGET_IP_ADDRESS",
        "TARGET_IPV6_ADDRESS",
        "IP_MAC_PLATFORM_NAME",
        "IP_MAC_PLATFORM_PROVIDER_NAME",
        "TARGET_MAC_ADDRESS_RANGE",
        "TARGET_IP_SLASH",
        "TARGET_IP_SOURCE_SLASH",
        "TARGET_IPV6_SLASH",
        "TARGET_IPV6_SOURCE_SLASH",
        "IP_MAC_STREAM_LOCATION",
        "ISP_ACCESS_MODE",
        "IP_MAC_GENERIC_STREAM_LOCATION",
    ]
    .into_iter()
    .collect();
    assert_eq!(names, expected);

    // Cross-tool values from TSDuck's own decompile of the same section
    // (reference/test-022/test-022.decompile.xml).
    let mut seen_loc = false;
    let mut seen_generic = 0;
    for d in int.platform_descriptors.iter() {
        let Ok(d) = d else { continue }; // the TSDuck 7-byte generic location
        match d {
            AnyIntDescriptor::IpMacStreamLocation(l) => {
                assert_eq!(
                    (
                        l.network_id,
                        l.original_network_id,
                        l.transport_stream_id,
                        l.service_id,
                        l.component_tag
                    ),
                    (0x1234, 0x5678, 0x9ABC, 0x6543, 0x21)
                );
                seen_loc = true;
            }
            AnyIntDescriptor::IpMacGenericStreamLocation(g) => {
                seen_generic += 1;
                assert_eq!(g.interactive_network_id, 0x1234);
            }
            AnyIntDescriptor::IpMacPlatformName(n) => {
                assert_eq!(n.language_code.0, *b"foo");
                assert_eq!(n.text.raw(), b"Bar Text");
            }
            _ => {}
        }
    }
    assert!(seen_loc);
    assert_eq!(
        seen_generic, 1,
        "the other TSDuck generic location is the rejected 7-byte form"
    );
}

/// TSDuck test-060: UNTs with the UNT descriptors and the shared targets.
#[test]
fn tsduck_test_060_unt_descriptors() {
    let Some(ts) = load("reference/test-060/test-060.pack.ts") else {
        return;
    };
    let secs = sections_of(&ts, 0x4B);
    assert_eq!(secs.len(), 2, "two UNT sections");
    let mut names = BTreeSet::new();
    for s in &secs {
        let unt = UntSection::parse(s).unwrap();
        // Known pre-existing limitation (issue #1298): an empty
        // compatibilityDescriptor written as length=2/count=0 re-serializes as
        // length=0. Whole-section identity is asserted for every section whose
        // compatibility descriptors are not empty; the descriptor loops (the
        // subject of this test) are asserted byte-exact either way.
        if unt
            .platforms
            .iter()
            .all(|p| !p.compatibility_descriptor.descriptors.is_empty())
        {
            assert_eq!(&ser(&unt), s, "whole-section byte identity");
        } else {
            assert_eq!(
                ser(&unt).len() + 2,
                s.len(),
                "only the #1298 empty-compat form differs"
            );
        }
        walk_unt(unt.common_descriptors.raw(), &mut names);
        for p in &unt.platforms {
            for pair in &p.target_operational_pairs {
                walk_unt(pair.target_descriptors.raw(), &mut names);
                walk_unt(pair.operational_descriptors.raw(), &mut names);
            }
        }
    }
    println!("UNT descriptors present in test-060: {names:?}");
    for n in [
        "SCHEDULING",
        "UPDATE",
        "SSU_LOCATION",
        "MESSAGE",
        "SSU_EVENT_NAME",
        "TARGET_MAC_ADDRESS",
        "TARGET_IP_ADDRESS",
        "SSU_SUBGROUP_ASSOCIATION",
        "ENHANCED_MESSAGE",
        "SSU_URI",
    ] {
        assert!(names.contains(n), "test-060 UNT lacks {n}: {names:?}");
    }
}

/// TSDuck test-115: an RNT with the three RNT-specific descriptors.
#[test]
fn tsduck_test_115_rnt_descriptors() {
    let Some(ts) = load("reference/test-115/test-115.pack.ts") else {
        return;
    };
    let secs = sections_of(&ts, 0x79);
    assert!(!secs.is_empty());
    let mut names = BTreeSet::new();
    for s in &secs {
        let rnt = RntSection::parse(s).unwrap();
        assert_eq!(&ser(&rnt), s, "whole-section byte identity");
        walk_rnt(rnt.common_descriptors.raw(), &mut names);
        for rp in &rnt.resolution_providers {
            walk_rnt(rp.descriptors.raw(), &mut names);
            for ca in &rp.crid_authorities {
                walk_rnt(ca.descriptors.raw(), &mut names);
            }
        }
    }
    for n in ["RAR_OVER_DVB_STREAM", "RAR_OVER_IP", "RNT_SCAN"] {
        assert!(names.contains(n), "test-115 RNT lacks {n}: {names:?}");
    }
}

/// The one broadcast-capture INT (tsduck-test input/test-016.ts): every
/// descriptor in its loops is walked in the INT namespace and re-serializes.
#[test]
fn tsduck_test_016_capture_int_loops_round_trip() {
    let Some(ts) = load("input/test-016.ts") else {
        return;
    };
    let secs = sections_of(&ts, 0x4C);
    assert!(!secs.is_empty());
    let mut names = BTreeSet::new();
    for s in &secs {
        let int = IntSection::parse(s).unwrap();
        assert_eq!(&ser(&int), s);
        walk_int(int.platform_descriptors.raw(), &mut names);
        for pair in &int.loops {
            walk_int(pair.target_descriptors.raw(), &mut names);
            walk_int(pair.operational_descriptors.raw(), &mut names);
        }
    }
    println!("INT descriptors present in test-016: {names:?}");
}
