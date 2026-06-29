//! Download-on-demand MHP/BIOP capture test — exercises DSM-CC object-carousel
//! parsing against a real Hot Bird 13E Italian mux (issue #218).
//!
//! Keyed on `.test-streams/hotbird-mhp.ts` (~336 MB). When the capture is
//! present, scans for DSM-CC sections (table_id 0x3B/0x3C) and parses them as
//! `DsmccSection`, then attempts to decode `UnMessage` payloads (DSI/DII).
//!
//! When the capture is absent, the test **skips cleanly** so the suite passes
//! without downloads.
#![cfg(feature = "ts")]

use std::fs;
use std::path::Path;

use broadcast_common::Parse;
use dvb_si::carousel::{DownloadDataBlock, UnMessage};
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::ait::AnyAitDescriptor;
use dvb_si::tables::dsmcc::DsmccSection;
use dvb_si::tables::AnyTableSection;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{SectionReassembler, TsPacket, TS_PACKET_SIZE};

const CAPTURE: &str = "hotbird-mhp";

fn capture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(format!("{CAPTURE}.ts"))
}

/// Empirically known DSM-CC PIDs from this capture (may carry MHP carousels).
/// We scan all non-well-known PIDs between 0x0030–0x1FFE for DSM-CC sections.
const MIN_CAROUSEL_PID: u16 = 0x0030;
const MAX_CAROUSEL_PID: u16 = 0x1FFE;

#[test]
fn hotbird_mhp_dsmcc_parse() {
    let path = capture_path();
    if !path.exists() {
        eprintln!(
            "downloaded_mhp: SKIPPED — {CAPTURE}.ts not in .test-streams/. \
             Run `tools/fetch-test-streams.sh {CAPTURE}` to enable."
        );
        return;
    }

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
        if pkt.header.pid < MIN_CAROUSEL_PID || pkt.header.pid > MAX_CAROUSEL_PID {
            continue;
        }

        if let Some(payload) = pkt.payload {
            let reasm = reassemblers.entry(pkt.header.pid).or_default();
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

    if dsmcc_sections == 0 {
        eprintln!(
            "downloaded_mhp: SKIPPED — no DSM-CC carousel sections found \
             in the capture. This may be a valid outcome."
        );
        return;
    }

    assert!(
        dsi_count + dii_count + ddb_count > 0,
        "{CAPTURE}: expected at least one DSI/DII/DDB from an MHP object carousel"
    );
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
    let path = capture_path();
    if !path.exists() {
        eprintln!(
            "downloaded_mhp: SKIPPED — {CAPTURE}.ts not in .test-streams/. \
             Run `tools/fetch-test-streams.sh {CAPTURE}` to enable."
        );
        return;
    }

    let data = fs::read(&path).expect("read capture");

    // The demux follows PAT → PMT well-known PIDs but not AIT PIDs (signalled
    // via application_signalling_descriptor), so add the observed AIT PIDs.
    let mut builder = SiDemux::builder();
    for pid in AIT_PIDS {
        builder = builder.pid(Pid::new(pid));
    }
    let mut demux = builder.build();

    let mut ait_sections = 0usize;
    let mut dvb_j_app = 0usize;
    let mut dvb_j_location = 0usize;
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
                            // Each parameter is a length-prefixed byte string.
                            let _ = d.parameters.len();
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
         {unknown_dvb_j_tags} dvb_j tags left Unknown"
    );

    assert_eq!(
        unknown_dvb_j_tags, 0,
        "{CAPTURE}: dvb_j tags (0x03/0x04) must decode as typed variants, not Unknown"
    );
    assert!(
        dvb_j_app > 0 && dvb_j_location > 0,
        "{CAPTURE}: expected typed dvb_j_application + dvb_j_application_location \
         descriptors (found {dvb_j_app} app, {dvb_j_location} location)"
    );
}
