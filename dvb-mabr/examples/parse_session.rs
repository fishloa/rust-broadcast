//! Parse a DVB-MABR gateway configuration XML file and print its sessions.
//!
//! Usage: cargo run -p dvb-mabr --example parse_session -- <config.xml>

use broadcast_common::bounded::{Limit, read_file_bounded_string};
use dvb_mabr::MulticastGatewayConfiguration;
use std::env;
use std::path::Path;
use std::process;

/// A whole MABR session-configuration document, bounded by the ceiling the XML
/// reader accepts for any configuration this crate parses.
const CONFIG_LIMIT: Limit = Limit::new(
    "MABR configuration",
    broadcast_common::xml_chars::MAX_XML_BYTES as u64,
);

fn main() {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("Usage: parse_session <config.xml>");
            process::exit(1);
        }
    };

    let xml = read_file_bounded_string(Path::new(&path), CONFIG_LIMIT).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        process::exit(1);
    });

    let config = MulticastGatewayConfiguration::parse_str(&xml).unwrap_or_else(|e| {
        eprintln!("parse error: {e}");
        process::exit(1);
    });

    println!("Schema version: {}", config.schema_version);
    println!("Sessions: {}", config.sessions.len());

    for (i, session) in config.sessions.iter().enumerate() {
        println!("\n  Session [{i}]:");
        println!("    Service ID: {}", session.service_identifier);
        println!("    Manifests:  {}", session.manifest_locators.len());
        println!("    Transports: {}", session.transport_sessions.len());
    }
}
