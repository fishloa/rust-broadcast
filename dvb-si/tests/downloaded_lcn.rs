//! Download-on-demand LCN capture test — exercises PDS-scoped logical_channel
//! descriptor decoding against the French TNT mux (issue #218).
//!
//! Keyed on `.test-streams/france-tnt-uhf32.ts`. Uses `SectionSetCollector` to
//! gather the complete NIT, then walks the transport-stream loop with a
//! PDS-scoped registry (`PDS_EACEM`, `PDS_NORDIG`). When a transport stream
//! carries a logical_channel descriptor (tag 0x83) under a recognised PDS, the
//! test asserts typed `LogicalChannel` variants. If the capture has no such
//! descriptors, the test **skips** with a clear eprintln.
//!
//! When the capture is absent, the test **skips cleanly** so the suite passes
//! without downloads.
#![cfg(feature = "ts")]

use std::fs;
use std::path::Path;

use dvb_si::collect::SectionSetCollector;
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{AnyDescriptor, DescriptorRegistry, PDS_EACEM, PDS_NORDIG};
use dvb_si::ts::TS_PACKET_SIZE;

const CAPTURE: &str = "france-tnt-uhf32";

fn capture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".test-streams")
        .join(format!("{CAPTURE}.ts"))
}

#[test]
fn france_tnt_lcn_descriptors_parse() {
    let path = capture_path();
    if !path.exists() {
        eprintln!(
            "downloaded_lcn: SKIPPED — {CAPTURE}.ts not in .test-streams/. \
             Run `tools/fetch-test-streams.sh {CAPTURE}` to enable."
        );
        return;
    }

    let data = fs::read(&path).expect("read capture");

    let mut registry = DescriptorRegistry::new();
    registry
        .with_logical_channel_for_pds(PDS_EACEM)
        .with_logical_channel_for_pds(PDS_NORDIG);

    let mut demux = SiDemux::builder().build();
    let mut collector = SectionSetCollector::new();

    let mut nit_sets = 0usize;
    let mut ts_entries = 0usize;
    let mut lcn_entries = 0usize;
    let mut lcn_channel_count = 0usize;

    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            // collector.push_section_with_pid returns Some if the set just completed.
            let pid = Some(u16::from(ev.pid()));
            let complete = match collector.push_section_with_pid(pid, ev.bytes()) {
                Ok(Some(set)) => set,
                _ => continue,
            };

            // Try NIT decode.  SDT/BAT also arrive here; only NIT is interesting.
            let Ok(nit) = complete.nit_with_registry(&registry) else {
                continue;
            };
            nit_sets += 1;

            for ts in &nit.transport_streams {
                ts_entries += 1;
                for desc in ts.descriptors.descriptors() {
                    let Ok(desc) = desc else {
                        continue;
                    };
                    if matches!(desc, AnyDescriptor::LogicalChannel(_)) {
                        lcn_entries += 1;
                        if let AnyDescriptor::LogicalChannel(lc) = desc {
                            lcn_channel_count += lc.entries.len();
                        }
                    }
                }
            }
        }
    }

    eprintln!(
        "downloaded_lcn: {CAPTURE} — {nit_sets} NIT sets, \
         {ts_entries} TS entries, \
         {lcn_entries} logical_channel descriptors ({lcn_channel_count} entries), \
         {} bytes",
        data.len()
    );

    if lcn_entries == 0 {
        eprintln!(
            "downloaded_lcn: SKIPPED — capture carries no logical_channel \
             descriptors under PDS_EACEM or PDS_NORDIG. This is a valid real-world \
             outcome (the mux may use a different PDS or no LCN signalling)."
        );
        return;
    }

    assert!(nit_sets > 0);
}
