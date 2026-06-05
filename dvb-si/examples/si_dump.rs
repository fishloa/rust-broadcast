//! `si_dump` — demux a `.ts` capture and print the SI tables it carries.
//!
//! Usage:
//!
//! ```text
//! cargo run -p dvb-si --example si_dump -- <file.ts> [--json]
//! ```
//!
//! Default: one line per emitted (changed) section, e.g.
//! `pid=0x0012 EVENT_INFORMATION v3 sn=0`. With `--json`: the decoded typed
//! table for each section, pretty-printed (requires the `serde` feature, on by
//! default). A stats summary is printed at the end.

use std::process::ExitCode;

use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTable;
use dvb_si::traits::TableDef;
use dvb_si::ts::TS_PACKET_SIZE;

/// SCREAMING_SNAKE `NAME` for an `AnyTable` variant, via each type's `TableDef`.
fn table_name(table: &AnyTable<'_>) -> String {
    use dvb_si::tables;
    match table {
        AnyTable::Pat(_) => tables::pat::Pat::NAME.to_string(),
        AnyTable::Cat(_) => tables::cat::Cat::NAME.to_string(),
        AnyTable::Pmt(_) => tables::pmt::Pmt::NAME.to_string(),
        AnyTable::Tsdt(_) => tables::tsdt::Tsdt::NAME.to_string(),
        AnyTable::DsmccSection(_) => tables::dsmcc::DsmccSection::NAME.to_string(),
        AnyTable::Nit(_) => tables::nit::Nit::NAME.to_string(),
        AnyTable::Sdt(_) => tables::sdt::Sdt::NAME.to_string(),
        AnyTable::Bat(_) => tables::bat::Bat::NAME.to_string(),
        AnyTable::Unt(_) => tables::unt::Unt::NAME.to_string(),
        AnyTable::Int(_) => tables::int::Int::NAME.to_string(),
        AnyTable::Sat(_) => tables::sat::Sat::NAME.to_string(),
        AnyTable::Eit(_) => tables::eit::Eit::NAME.to_string(),
        AnyTable::Tdt(_) => tables::tdt::Tdt::NAME.to_string(),
        AnyTable::Rst(_) => tables::rst::Rst::NAME.to_string(),
        AnyTable::St(_) => tables::st::St::NAME.to_string(),
        AnyTable::Tot(_) => tables::tot::Tot::NAME.to_string(),
        AnyTable::Ait(_) => tables::ait::Ait::NAME.to_string(),
        AnyTable::Container(_) => tables::container::Container::NAME.to_string(),
        AnyTable::Rct(_) => tables::rct::Rct::NAME.to_string(),
        AnyTable::Cit(_) => tables::cit::Cit::NAME.to_string(),
        AnyTable::MpeFec(_) => tables::mpe_fec::MpeFec::NAME.to_string(),
        AnyTable::Rnt(_) => tables::rnt::Rnt::NAME.to_string(),
        AnyTable::MpeIfec(_) => tables::mpe_ifec::MpeIfec::NAME.to_string(),
        AnyTable::ProtectionMessage(_) => {
            tables::protection_message::ProtectionMessageSection::NAME.to_string()
        }
        AnyTable::DownloadableFontInfo(_) => {
            tables::downloadable_font_info::DownloadableFontInfoSection::NAME.to_string()
        }
        AnyTable::Dit(_) => tables::dit::Dit::NAME.to_string(),
        AnyTable::Sit(_) => tables::sit::Sit::NAME.to_string(),
        AnyTable::MpeDatagram(_) => tables::mpe::MpeDatagramSection::NAME.to_string(),
        AnyTable::Unknown { table_id, .. } => format!("UNKNOWN(0x{table_id:02X})"),
        // `AnyTable` is #[non_exhaustive]; any future variant prints its id.
        _ => "OTHER".to_string(),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut path: Option<String> = None;
    let mut json = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => {
                eprintln!("usage: si_dump <file.ts> [--json]");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("si_dump: unknown option {other}");
                return ExitCode::FAILURE;
            }
            other => path = Some(other.to_string()),
        }
    }

    let Some(path) = path else {
        eprintln!("usage: si_dump <file.ts> [--json]");
        return ExitCode::FAILURE;
    };

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("si_dump: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut demux = SiDemux::builder().build();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue; // skip non-aligned / short tail
        }
        for event in demux.feed(chunk) {
            match event.table() {
                Ok(table) => {
                    if json {
                        match serde_json::to_string_pretty(&table) {
                            Ok(s) => println!("{s}"),
                            Err(e) => eprintln!("si_dump: serialize {}: {e}", event.pid()),
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
