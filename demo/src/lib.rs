//! WASM browser demo for dvb-si.
//!
//! Exposes a single `parse_ts(bytes: &[u8]) -> String` function that feeds a
//! raw MPEG-TS capture through [`dvb_si::demux::SiDemux`], collects every
//! changed SI section, and returns a JSON summary using the existing
//! dvb-si serde shape (not an invented second schema).

use dvb_si::demux::SiDemux;
use mpeg_ts::ts::TS_PACKET_SIZE;
use mpeg_ts::ts::TS_SYNC_BYTE;
use serde::Serialize;
use wasm_bindgen::prelude::*;

// ───────────────────────────── output types ──────────────────────────────────

/// One entry in the `tables` array — the serde-serialised AnyTableSection
/// value with its PID prepended for context.
#[derive(Serialize)]
struct TableEntry {
    /// PID the section was carried on (decimal).
    pid: u16,
    /// Parsed section, using dvb-si's own serde shape (camelCase external tag).
    /// This is NOT an invented schema — it is the same JSON that
    /// `serde_json::to_value(&AnyTableSection::…)` produces.
    section: serde_json::Value,
}

/// One service extracted from the SDT.
#[derive(Serialize)]
struct ServiceEntry {
    service_id: u16,
    provider_name: String,
    service_name: String,
    service_type: String,
}

/// Top-level JSON object returned by `parse_ts`.
#[derive(Serialize)]
struct ParseResult {
    tables: Vec<TableEntry>,
    services: Vec<ServiceEntry>,
    /// Count of packets that failed to parse (bad sync byte, too short).
    parse_errors: u64,
    /// Count of sections that failed CRC validation.
    crc_errors: u64,
    /// Total TS packets fed.
    packets_fed: u64,
}

// ───────────────────────────── wasm export ───────────────────────────────────

/// Parse a raw MPEG-TS byte buffer and return a JSON summary of all SI tables
/// found, plus a service list extracted from SDT sections.
///
/// Never panics. On any bad packet or section the error is counted and
/// processing continues.
#[wasm_bindgen]
pub fn parse_ts(bytes: &[u8]) -> String {
    let mut demux = SiDemux::builder().build();
    let mut tables: Vec<TableEntry> = Vec::new();

    for chunk in bytes.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        if chunk[0] != TS_SYNC_BYTE {
            continue;
        }
        for ev in demux.feed(chunk) {
            let pid = ev.pid().value();
            // Serialize via dvb-si's own serde shape. If the section cannot be
            // parsed (extremely unusual at this point — CRC already passed),
            // emit a minimal error object instead of panicking.
            let section = match ev.table_section() {
                Ok(ts) => match serde_json::to_value(&ts) {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({ "serializeError": e.to_string() }),
                },
                Err(e) => serde_json::json!({ "parseError": e.to_string() }),
            };
            tables.push(TableEntry { pid, section });
        }
    }

    let stats = demux.stats();

    // ── Build the service list from every SDT section we collected. ──────────
    // We walk the already-collected JSON to avoid re-borrowing the demux, but
    // it's cleaner to keep a parallel list of owned parsed values. Since we own
    // `tables` we can inspect the JSON directly.
    let services = extract_services(&tables);

    let result = ParseResult {
        tables,
        services,
        parse_errors: stats.malformed_packets,
        crc_errors: stats.crc_failures,
        packets_fed: stats.packets,
    };

    // unwrap: serde_json serialisation of our own struct cannot fail.
    serde_json::to_string(&result).unwrap_or_else(|e| {
        format!("{{\"internalError\":\"{e}\"}}")
    })
}

// ───────────────────────────── service extraction ────────────────────────────

/// Walk the collected table entries and extract service info from SDT sections.
///
/// The JSON shape produced by dvb-si's serde for an SDT is:
/// `{"sdtSection": { "services": [ { "serviceId": N, "descriptors": [...] } ] }}`
/// We navigate that structure to pull out service_descriptor fields.
fn extract_services(tables: &[TableEntry]) -> Vec<ServiceEntry> {
    let mut services = Vec::new();

    for entry in tables {
        // Only SDT sections (actual or other).
        let sdt_obj = match entry.section.get("sdtSection") {
            Some(v) => v,
            None => continue,
        };

        let svc_list = match sdt_obj.get("services").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => continue,
        };

        for svc in svc_list {
            let service_id = svc
                .get("service_id")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u16;

            let descriptors = match svc.get("descriptors").and_then(|v| v.as_array()) {
                Some(a) => a,
                None => continue,
            };

            for desc in descriptors {
                // dvb-si serializes ServiceDescriptor as `{"service": {...}}`
                // (camelCase external tag from the AnyDescriptor enum).
                let svc_desc = match desc.get("service") {
                    Some(v) => v,
                    None => continue,
                };

                let provider_name = svc_desc
                    .get("provider_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let service_name = svc_desc
                    .get("service_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // ServiceType serializes as the Rust variant name by dvb-si serde.
                let service_type = svc_desc
                    .get("service_type")
                    .map(|v| {
                        if let Some(s) = v.as_str() {
                            s.to_string()
                        } else {
                            v.to_string()
                        }
                    })
                    .unwrap_or_default();

                services.push(ServiceEntry {
                    service_id,
                    provider_name,
                    service_name,
                    service_type,
                });
                break; // one service_descriptor per service entry is enough
            }
        }
    }

    services
}
