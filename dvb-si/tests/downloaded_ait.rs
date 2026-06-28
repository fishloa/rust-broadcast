//! Download-on-demand AIT capture test — exercises AIT descriptor paths
//! against the real French TNT HbbTV mux (issue #218).
//!
//! Keyed on `.test-streams/france-tnt-uhf32.ts`. When the capture is present,
//! feeds it through `SiDemux`, collects AIT sections (table_id 0x74), and
//! walks each application's `ait_descriptors()` + the common descriptor loop
//! via `common_ait_descriptors()`, asserting typed `AnyAitDescriptor` variants
//! and no parse errors.
//!
//! When the capture is absent, the test **skips cleanly** so the suite passes
//! without downloads.
#![cfg(feature = "ts")]

use std::fs;
use std::path::Path;

use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::TS_PACKET_SIZE;

const CAPTURE: &str = "france-tnt-uhf32";

fn capture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(format!("{CAPTURE}.ts"))
}

#[test]
fn france_tnt_ait_descriptors_parse() {
    let path = capture_path();
    if !path.exists() {
        eprintln!(
            "downloaded_ait: SKIPPED — {CAPTURE}.ts not in .test-streams/. \
             Run `tools/fetch-test-streams.sh {CAPTURE}` to enable."
        );
        return;
    }

    let data = fs::read(&path).expect("read capture");

    // AIT PIDs observed in this capture (0x010E, 0x0302, 0x00AA).
    // The demux follows PAT → PMT well-known PIDs, but AIT PIDs are
    // signalled via application_signalling_descriptor, so add them explicitly.
    let mut demux = SiDemux::builder()
        .pid(Pid::new(0x010E))
        .pid(Pid::new(0x0302))
        .pid(Pid::new(0x00AA))
        .build();
    let mut ait_sections = 0usize;
    let mut applications = 0usize;
    let mut app_name_count = 0usize;
    let mut app_desc_count = 0usize;
    let mut transport_protocol_count = 0usize;
    let mut total_ait_descriptors = 0usize;
    let mut total_common_descriptors = 0usize;

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

            for desc_res in sec.common_ait_descriptors().iter() {
                total_common_descriptors += 1;
                let desc = desc_res.expect("common AIT descriptor must parse");
                match desc {
                    dvb_si::descriptors::ait::AnyAitDescriptor::ApplicationName(_) => {
                        app_name_count += 1;
                    }
                    dvb_si::descriptors::ait::AnyAitDescriptor::Application(_) => {
                        app_desc_count += 1;
                    }
                    dvb_si::descriptors::ait::AnyAitDescriptor::TransportProtocol(_) => {
                        transport_protocol_count += 1;
                    }
                    _ => {}
                }
            }

            for app in &sec.applications {
                applications += 1;
                for desc_res in app.ait_descriptors().iter() {
                    total_ait_descriptors += 1;
                    let desc = desc_res.expect("per-app AIT descriptor must parse");
                    match desc {
                        dvb_si::descriptors::ait::AnyAitDescriptor::ApplicationName(_) => {
                            app_name_count += 1;
                        }
                        dvb_si::descriptors::ait::AnyAitDescriptor::Application(_) => {
                            app_desc_count += 1;
                        }
                        dvb_si::descriptors::ait::AnyAitDescriptor::TransportProtocol(_) => {
                            transport_protocol_count += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    eprintln!(
        "downloaded_ait: {CAPTURE} — {ait_sections} AIT sections, \
         {applications} applications, \
         {app_name_count} app-name, {app_desc_count} app, \
         {transport_protocol_count} transport-protocol descriptors; \
         {total_ait_descriptors} total per-app descriptors, \
         {total_common_descriptors} total common descriptors, \
         {} bytes",
        data.len()
    );

    assert!(
        ait_sections > 0,
        "{CAPTURE}: expected at least one AIT section from a HbbTV mux"
    );
}
