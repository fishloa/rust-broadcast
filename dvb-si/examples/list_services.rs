//! Advanced: demux a real MPEG-TS capture and list its services and tables.
//!
//! Run with: `cargo run -p dvb-si --example list_services` (needs the default
//! `ts` feature). Reads a committed French TNT capture (which carries an SDT)
//! at runtime.

use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;
use std::collections::BTreeMap;

const PKT: usize = 188;

fn main() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tnt-5w-12732v-isi6-10s.ts"
    );
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let mut demux = SiDemux::builder().build();
    let mut tables: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut services = Vec::new();

    for pkt in data.chunks(PKT) {
        if pkt.len() < PKT {
            break;
        }
        for event in demux.feed(pkt) {
            match event.table_section() {
                Ok(AnyTableSection::SdtSection(sdt)) => {
                    *tables.entry("SDT").or_default() += 1;
                    for service in &sdt.services {
                        for item in service.descriptors.iter().flatten() {
                            if let AnyDescriptor::Service(svc) = item {
                                services.push(format!(
                                    "{} — {} (\"{}\")",
                                    service.service_id,
                                    svc.service_type,
                                    svc.service_name.decode()
                                ));
                            }
                        }
                    }
                }
                Ok(AnyTableSection::PatSection(_)) => *tables.entry("PAT").or_default() += 1,
                Ok(AnyTableSection::PmtSection(_)) => *tables.entry("PMT").or_default() += 1,
                Ok(AnyTableSection::NitSection(_)) => *tables.entry("NIT").or_default() += 1,
                Ok(AnyTableSection::EitSection(_)) => *tables.entry("EIT").or_default() += 1,
                Ok(_) => *tables.entry("other").or_default() += 1,
                Err(_) => {}
            }
        }
    }

    println!("tables seen (distinct sections):");
    for (name, n) in &tables {
        println!("  {name:<6} {n}");
    }

    services.sort();
    services.dedup();
    println!("\nservices:");
    if services.is_empty() {
        println!("  (no SDT in this capture)");
    }
    for s in &services {
        println!("  {s}");
    }
}
