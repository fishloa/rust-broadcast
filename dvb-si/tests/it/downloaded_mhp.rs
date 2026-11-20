//! Real-capture MHP/BIOP capture test — exercises DSM-CC object-carousel
//! parsing against a real Hot Bird 13E Italian mux (issue #218).
//!
//! Keyed on `private/fixtures/ts/hotbird-mhp-si.ts` (27 MB: the whole capture minus video/audio PES). When the capture is
//! present, scans for DSM-CC sections (table_id 0x3B/0x3C) and parses them as
//! `DsmccSection`, then attempts to decode `UnMessage` payloads (DSI/DII).
//!
//! When the capture is absent, the test **skips loudly** (private/ not checked out) so the suite passes
//! without the private fixtures repo.
#![cfg(feature = "ts")]

use std::fs;

use broadcast_common::Parse;
use dvb_si::carousel::{DownloadDataBlock, UnMessage};
use dvb_si::demux::{SiDemux, SiDemuxConfig};
use dvb_si::descriptors::ait::AnyAitDescriptor;
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::dsmcc::DsmccSection;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

const CAPTURE: &str = "hotbird-mhp-si";

/// Empirically known DSM-CC PIDs from this capture (may carry MHP carousels).
/// We scan all non-well-known PIDs between 0x0030–0x1FFE for DSM-CC sections.
const MIN_CAROUSEL_PID: u16 = 0x0030;
const MAX_CAROUSEL_PID: u16 = 0x1FFE;

#[test]
fn hotbird_mhp_dsmcc_parse() {
    let Some(path) = test_fixtures::require(&format!("ts/{CAPTURE}.ts"), "downloaded_mhp") else {
        return;
    };

    let data = fs::read(&path).expect("read capture");

    // Use a demux to get sections on well-known PIDs (NIT, SDT, ...) but
    // DSM-CC carousels are on service-specific PIDs that the demux doesn't
    // auto-track. We scan the TS packets directly for PUSI-located DSM-CC
    // sections by running a SectionReassembler per PID on demand.
    use std::collections::BTreeMap;
    let mut reassemblers: BTreeMap<u16, SectionReassembler> = BTreeMap::new();

    let mut dsmcc_sections = 0usize;
    let mut dsi_count = 0usize;
    let mut dii_count = 0usize;
    let mut ddb_count = 0usize;

    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        let pkt = match TsPacket::parse(chunk) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Only scan carousel-range PIDs (skip well-known SI).
        if pkt.header.pid < Pid::from_const(MIN_CAROUSEL_PID)
            || pkt.header.pid > Pid::from_const(MAX_CAROUSEL_PID)
        {
            continue;
        }

        if let Some(payload) = pkt.payload {
            let reasm = reassemblers.entry(pkt.header.pid.value()).or_default();
            reasm.feed(payload, pkt.header.pusi);
            while let Some(sec_bytes) = reasm.pop_section() {
                let table_id = sec_bytes[0];
                if table_id != 0x3B && table_id != 0x3C {
                    continue;
                }
                dsmcc_sections += 1;
                let ok = DsmccSection::parse(&sec_bytes);
                let section = match ok {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("downloaded_mhp: DSM-CC parse error: {e}");
                        continue;
                    }
                };
                match UnMessage::parse(section.payload) {
                    Ok(UnMessage::Dsi(_)) => dsi_count += 1,
                    Ok(UnMessage::Dii(_)) => dii_count += 1,
                    Ok(_) => {}
                    Err(_) => {
                        if section.table_id == 0x3C
                            && DownloadDataBlock::parse(section.payload).is_ok()
                        {
                            ddb_count += 1;
                        }
                    }
                }
            }
        }
    }

    eprintln!(
        "downloaded_mhp: {CAPTURE} — {dsmcc_sections} DSM-CC sections, \
         {dsi_count} DSI, {dii_count} DII, {ddb_count} DDB, \
         {} bytes processed",
        data.len()
    );

    assert_eq!(dsmcc_sections, 3355, "{CAPTURE}: DSM-CC sections");
    assert_eq!(dsi_count, 1305, "{CAPTURE}: DSI messages");
    assert_eq!(dii_count, 870, "{CAPTURE}: DII messages");
    assert_eq!(ddb_count, 1173, "{CAPTURE}: DDB messages");
}

/// AIT PIDs carrying the DVB-J application this capture signals (observed via
/// `application_signalling_descriptor` in the PMT es_info loop).
const AIT_PIDS: [u16; 2] = [0x1EC6, 0x1EC7];

/// Exercises the deferred DVB-J descriptors (#227) against real broadcast data:
/// this Hot Bird MHP mux carries `dvb_j_application` (AIT tag 0x03) and
/// `dvb_j_application_location` (AIT tag 0x04) in its AIT application loop.
/// Walking the AIT must decode them as the typed [`AnyAitDescriptor`] variants
/// (NOT `Unknown`), proving the new parsers bite on a real stream.
#[test]
fn hotbird_mhp_dvb_j_descriptors_typed() {
    let Some(path) = test_fixtures::require(&format!("ts/{CAPTURE}.ts"), "downloaded_mhp") else {
        return;
    };

    let data = fs::read(&path).expect("read capture");

    // The demux follows PAT → PMT well-known PIDs but not AIT PIDs (signalled
    // via application_signalling_descriptor), so add the observed AIT PIDs.
    let mut builder = SiDemuxConfig::default();
    for pid in AIT_PIDS {
        builder = builder.with_pid(Pid::from_wire_masked(pid));
    }
    let mut demux = SiDemux::new(builder);

    let mut ait_sections = 0usize;
    let mut dvb_j_app = 0usize;
    let mut dvb_j_location = 0usize;
    let mut dvb_j_param_count = 0usize;
    let mut dvb_j_param_bytes = 0usize;
    let mut unknown_dvb_j_tags = 0usize;

    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            let sec = match ev.table_section() {
                Ok(AnyTableSection::AitSection(ait)) => ait,
                _ => continue,
            };
            ait_sections += 1;
            for app in &sec.applications {
                for desc_res in app.ait_descriptors().iter() {
                    match desc_res.expect("per-app AIT descriptor must parse") {
                        AnyAitDescriptor::DvbJApplication(d) => {
                            dvb_j_app += 1;
                            for p in &d.parameters {
                                assert!(!p.is_empty(), "{CAPTURE}: empty dvb_j parameter");
                                assert!(
                                    p.iter().all(|b| b.is_ascii_graphic() || *b == b' '),
                                    "{CAPTURE}: non-ASCII dvb_j parameter"
                                );
                            }
                            dvb_j_param_count += d.parameters.len();
                            dvb_j_param_bytes +=
                                d.parameters.iter().map(|p| p.len()).sum::<usize>();
                        }
                        AnyAitDescriptor::DvbJApplicationLocation(d) => {
                            dvb_j_location += 1;
                            // base_directory shall be non-empty per the spec.
                            assert!(
                                !d.base_directory.is_empty(),
                                "{CAPTURE}: dvb_j_application_location base_directory empty"
                            );
                        }
                        // If the new parsers regressed, 0x03/0x04 would fall
                        // through to Unknown — catch that explicitly.
                        AnyAitDescriptor::Unknown {
                            tag: 0x03 | 0x04, ..
                        } => {
                            unknown_dvb_j_tags += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    eprintln!(
        "downloaded_mhp: {CAPTURE} — {ait_sections} AIT sections, \
         {dvb_j_app} dvb_j_application, {dvb_j_location} dvb_j_application_location, \
         {dvb_j_param_count} parameters ({dvb_j_param_bytes} bytes), \
         {unknown_dvb_j_tags} dvb_j tags left Unknown"
    );

    assert_eq!(
        unknown_dvb_j_tags, 0,
        "{CAPTURE}: dvb_j tags (0x03/0x04) must decode as typed variants, not Unknown"
    );
    assert_eq!(ait_sections, 2, "{CAPTURE}: AIT sections");
    assert_eq!(dvb_j_app, 2, "{CAPTURE}: dvb_j_application descriptors");
    assert_eq!(
        dvb_j_location, 2,
        "{CAPTURE}: dvb_j_application_location descriptors"
    );
    assert_eq!(
        dvb_j_param_count, 0,
        "{CAPTURE}: dvb_j_application parameter strings"
    );
    assert_eq!(
        dvb_j_param_bytes, 0,
        "{CAPTURE}: dvb_j_application parameter bytes"
    );
}
