//! `dvb-tools dump <file.ts> [--json]` — SI section dump.
//!
//! Drives [`SiDemux`] over an aligned 188-byte `.ts` capture and prints one
//! line per emitted (changed) section. With `--json`, the decoded typed table
//! for each section is pretty-printed via `serde_json`.
//!
//! Behaviour is identical to the former `dvb-si/examples/si_dump.rs`.
use std::process::ExitCode;

use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{
    AnyDescriptor, DescriptorLoop, DescriptorRegistry, PDS_EACEM, PDS_NORDIG,
};
use dvb_si::tables::AnyTableSection;

use crate::util::{for_each_packet, read_file};

/// Label for an `AnyTableSection` — the macro-generated `name()`, with the
/// `table_id` appended for unknowns.
fn table_name(table: &AnyTableSection<'_>) -> String {
    match table {
        AnyTableSection::Unknown { table_id, .. } => format!("UNKNOWN(0x{table_id:02X})"),
        t => t.name().to_string(),
    }
}

/// Print one indented line per descriptor in `loop_`, decoded through `reg`
/// (so PDS-scoped private tags — e.g. the 0x83 logical_channel under an
/// EACEM/NorDig `private_data_specifier` — resolve instead of showing as
/// `UNKNOWN`). `Other` (runtime-registered) and `Unknown` carry their raw tag.
fn print_descriptors(loop_: &DescriptorLoop<'_>, reg: &DescriptorRegistry, indent: &str) {
    for item in loop_.iter_with(reg) {
        match item {
            Ok(d) => {
                let label = match &d {
                    AnyDescriptor::Unknown { tag, .. } => format!("UNKNOWN(0x{tag:02X})"),
                    AnyDescriptor::Other { tag, .. } => format!("{}(0x{tag:02X})", d.name()),
                    _ => d.name().to_string(),
                };
                println!("{indent}desc {label}");
            }
            Err(e) => println!("{indent}desc <parse error: {e}>"),
        }
    }
}

/// `dvb-tools dump <FILE> [--json]` — returns success on a clean run.
pub fn run(path: &str, json: bool) -> ExitCode {
    let data = match read_file(path, "dvb-tools dump") {
        Ok(d) => d,
        Err(code) => return code,
    };

    // Registry for the human-mode descriptor walk: enable the PDS-scoped 0x83
    // logical_channel built-in for the common EACEM/NorDig private_data_specifiers
    // so LCNs decode instead of showing as UNKNOWN (mirrors the `services` walk).
    let mut reg = DescriptorRegistry::new();
    reg.with_logical_channel_for_pds(PDS_EACEM)
        .with_logical_channel_for_pds(PDS_NORDIG);

    let mut demux = SiDemux::builder().build();
    for packet in for_each_packet(&data) {
        for event in demux.feed(&packet) {
            match event.table_section() {
                Ok(table) => {
                    if json {
                        match serde_json::to_string_pretty(&table) {
                            Ok(s) => println!("{s}"),
                            Err(e) => {
                                eprintln!("dvb-tools dump: serialize {}: {e}", event.pid());
                            }
                        }
                    } else {
                        let name = table_name(&table);
                        match event.version() {
                            Some(v) => println!(
                                "pid={} {name} v{v} sn={}",
                                event.pid(),
                                event.section_number().unwrap_or(0)
                            ),
                            None => println!("pid={} {name}", event.pid()),
                        }
                        match &table {
                            AnyTableSection::PmtSection(pmt) => {
                                print_descriptors(&pmt.program_info, &reg, "    ");
                                for st in &pmt.streams {
                                    println!(
                                        "    es pid=0x{:04X} stream_type={}",
                                        st.elementary_pid,
                                        st.stream_type.name()
                                    );
                                    print_descriptors(&st.es_info, &reg, "      ");
                                }
                            }
                            AnyTableSection::SdtSection(sdt) => {
                                for s in &sdt.services {
                                    println!(
                                        "    service 0x{:04X} running_status={}",
                                        s.service_id,
                                        s.running_status.name()
                                    );
                                    print_descriptors(&s.descriptors, &reg, "      ");
                                }
                            }
                            AnyTableSection::NitSection(nit) => {
                                print_descriptors(&nit.network_descriptors, &reg, "    ");
                                for ts in &nit.transport_streams {
                                    println!(
                                        "    ts 0x{:04X} onid=0x{:04X}",
                                        ts.transport_stream_id, ts.original_network_id
                                    );
                                    print_descriptors(&ts.descriptors, &reg, "      ");
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => eprintln!("pid={} parse error: {e}", event.pid()),
            }
        }
    }

    let s = demux.stats();
    eprintln!(
        "-- packets={} sections={} emitted={} suppressed={} crc_failures={} malformed={}",
        s.packets,
        s.sections_completed,
        s.emitted,
        s.suppressed,
        s.crc_failures,
        s.malformed_packets
    );
    ExitCode::SUCCESS
}
