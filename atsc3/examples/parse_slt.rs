//! Parse an SLT (Service List Table) from an LLS binary file.
//!
//! The LLS payload is gzip-compressed XML; this example decompresses it and
//! parses the SLT structure.
//!
//! Usage:
//!   cargo run --example parse_slt -- <slt-lls.bin>
//!
//! Run against the real captured fixture (issue #926/#943 — see
//! `fixtures/atsc3/PROVENANCE.md`):
//!   cargo run -p atsc3 --example parse_slt -- fixtures/atsc3/slt-lls-2019-01-07.bin

use broadcast_common::Parse;
use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <slt-lls.bin>", args[0]);
        process::exit(1);
    }

    let bytes = match fs::read(&args[1]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error reading {}: {}", args[1], e);
            process::exit(1);
        }
    };

    let envelope = match atsc3::lls::LlsEnvelope::parse(&bytes) {
        Ok(env) => env,
        Err(e) => {
            eprintln!("Error parsing LLS envelope: {}", e);
            process::exit(1);
        }
    };

    if envelope.table_id != atsc3::LlsTableId::Slt {
        eprintln!("Expected SLT (table_id=0x01), got {}", envelope.table_id,);
        process::exit(1);
    }

    let xml_bytes = match envelope.decompress() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error decompressing LLS payload: {}", e);
            process::exit(1);
        }
    };

    let xml = match std::str::from_utf8(&xml_bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("LLS payload is not valid UTF-8: {}", e);
            process::exit(1);
        }
    };

    let slt = match atsc3::slt::Slt::parse(xml) {
        Ok(slt) => slt,
        Err(e) => {
            eprintln!("Error parsing SLT XML: {}", e);
            process::exit(1);
        }
    };

    println!(
        "SLT — bsid={:?}, {} service(s):",
        slt.bsid,
        slt.services.len()
    );
    for svc in &slt.services {
        let name = svc.short_service_name.as_deref().unwrap_or("<unnamed>");
        println!(
            "  Service {} — \"{}\" (category: {})",
            svc.service_id, name, svc.service_category,
        );
        if let Some(ref bss) = svc.broadcast_svc_signaling {
            println!(
                "    SLS: protocol={}, v{}.{}",
                bss.sls_protocol, bss.sls_major_protocol_version, bss.sls_minor_protocol_version,
            );
        }
    }
}
