//! `dvb-tools services <file.ts>` — SDT service tree with LCNs.
//!
//! Drives `SiDemux` over an aligned 188-byte `.ts` capture, feeds SDT/NIT
//! sections into a `SectionSetCollector` keyed by PID, and prints one line
//! per service sorted by LCN (services without an LCN sort last, by
//! service_id).
//!
//! Empty streams (no SDT seen) are not an error — note to stderr and exit
//! successfully.
use std::collections::HashMap;
use std::process::ExitCode;

use dvb_si::TableId;
use dvb_si::collect::{CompleteSdt, CompleteSdtService, SectionSetCollector};
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{AnyDescriptor, DescriptorRegistry, PDS_EACEM, PDS_NORDIG};
use dvb_si::tables::RunningStatus;

use crate::util::{for_each_packet, read_file};

/// `table_id`s carrying a Service Description Table (actual + other transport).
const SDT_TABLE_IDS: [u8; 2] = [
    TableId::ServiceDescriptionActual as u8,
    TableId::ServiceDescriptionOther as u8,
];
/// `table_id`s carrying a Network Information Table (actual + other network).
const NIT_TABLE_IDS: [u8; 2] = [
    TableId::NetworkInformationActual as u8,
    TableId::NetworkInformationOther as u8,
];

/// One row in the service table.
struct ServiceRow {
    /// service_id from the SDT entry.
    service_id: u16,
    /// Display-friendly type name from the service descriptor (SDT inner loop).
    type_name: &'static str,
    /// Annex A decoded service name. `None` if the SDT entry carries no
    /// service descriptor.
    name: Option<String>,
    /// 3-bit running_status from the SDT entry.
    running_status: RunningStatus,
}

/// Push one demux event into the collector. Errors (CRC, short-form) are
/// silently dropped — we only want valid long-form SDT/NIT sections.
fn collect_event(
    collector: &mut SectionSetCollector,
    pid: u16,
    bytes: &[u8],
) -> Option<dvb_si::collect::CompleteSectionSet> {
    collector
        .push_section_with_pid(Some(pid), bytes)
        .ok()
        .flatten()
}

/// Extract the service name + service_type from a single SDT service's
/// descriptor loop. Defaults to a placeholder name and "reserved/unknown"
/// when no service descriptor is present.
fn service_descriptor_view(service: &CompleteSdtService<'_>) -> (Option<String>, &'static str) {
    let mut name = None;
    let mut type_name = "reserved/unknown";
    for item in service.descriptors.descriptors().iter().flatten() {
        if let AnyDescriptor::Service(sd) = item {
            name = Some(sd.service_name.to_string());
            type_name = sd.service_type.name();
            break;
        }
    }
    (name, type_name)
}

/// Pick the LCN for `service_id` from a complete NIT (None if no entry).
fn lookup_lcn(map: &HashMap<u16, u16>, service_id: u16) -> Option<u16> {
    map.get(&service_id).copied()
}

/// Render one service line for stdout.
fn print_row(row: &ServiceRow, lcn: Option<u16>) {
    let lcn_str = match lcn {
        Some(n) => format!("{n:>4}"),
        None => "   -".to_string(),
    };
    let type_name = row.type_name;
    let name = row.name.as_deref().unwrap_or("(no service descriptor)");
    println!(
        "LCN {lcn_str}  service 0x{:04X}  {type_name:<32}  \"{name}\"  running={}",
        row.service_id,
        row.running_status.name()
    );
}

pub fn run(path: &str) -> ExitCode {
    let data = match read_file(path, "dvb-tools services") {
        Ok(d) => d,
        Err(code) => return code,
    };

    let mut demux = SiDemux::builder().build();
    let mut collector = SectionSetCollector::new();
    let mut services: HashMap<u16, ServiceRow> = HashMap::new();
    // SDT original_network_id + transport_stream_id of the last complete SDT
    // — used to combine with the SDT-only PMT/NIT picture when no LCN
    // descriptor is present.
    let mut lcn_map: HashMap<u16, u16> = HashMap::new();
    // logical_channel (0x83) is PDS-scoped: enable it for the common LCN
    // private_data_specifiers so the NIT walk decodes LCNs instead of Unknown.
    let mut lcn_registry = DescriptorRegistry::new();
    lcn_registry
        .with_logical_channel_for_pds(PDS_EACEM)
        .with_logical_channel_for_pds(PDS_NORDIG);
    let mut sdt_seen = 0u32;
    let mut nit_seen = 0u32;

    for packet in for_each_packet(&data) {
        for event in demux.feed(&packet) {
            let table_id = event.table_id();
            let bytes = event.bytes().to_vec();
            // SDT (other/current) — table_id 0x42 / 0x46. NIT — 0x40 / 0x41.
            let pid_u16 = u16::from(event.pid());
            if SDT_TABLE_IDS.contains(&table_id) {
                if let Some(complete) = collect_event(&mut collector, pid_u16, &bytes) {
                    sdt_seen += 1;
                    if let Ok(sdt) = complete.sdt() {
                        absorb_sdt(&sdt, &mut services);
                    }
                }
            } else if NIT_TABLE_IDS.contains(&table_id) {
                if let Some(complete) = collect_event(&mut collector, pid_u16, &bytes) {
                    nit_seen += 1;
                    if let Ok(nit) = complete.nit_with_registry(&lcn_registry) {
                        absorb_nit(&nit, &mut lcn_map);
                    }
                }
            }
        }
    }

    if sdt_seen == 0 {
        eprintln!("dvb-tools services: no SDT seen (stream is empty or has no SDT)");
        return ExitCode::SUCCESS;
    }

    // Build sort: (has_lcn ? 0 : 1, lcn_or_max, service_id).
    let rows: Vec<(Option<u16>, u16, &ServiceRow)> = services
        .values()
        .map(|row| (lookup_lcn(&lcn_map, row.service_id), row.service_id, row))
        .collect();
    let mut rows = rows;
    rows.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => x.cmp(&y).then(a.1.cmp(&b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });

    let with_lcn = rows.iter().filter(|(lcn, _, _)| lcn.is_some()).count();
    for (_, _, row) in &rows {
        print_row(row, lookup_lcn(&lcn_map, row.service_id));
    }
    eprintln!(
        "-- services={} with_lcn={} sdt_collected={} nit_collected={}",
        services.len(),
        with_lcn,
        sdt_seen,
        nit_seen
    );
    ExitCode::SUCCESS
}

fn absorb_sdt<'a>(sdt: &'a CompleteSdt<'a>, services: &mut HashMap<u16, ServiceRow>) {
    for service in &sdt.services {
        let (name, type_name) = service_descriptor_view(service);
        services.insert(
            service.service_id,
            ServiceRow {
                service_id: service.service_id,
                type_name,
                name,
                running_status: service.running_status,
            },
        );
    }
}

fn absorb_nit<'a>(nit: &'a dvb_si::collect::CompleteNit<'a>, lcn_map: &mut HashMap<u16, u16>) {
    for ts in &nit.transport_streams {
        for item in ts.descriptors.descriptors().iter().flatten() {
            if let AnyDescriptor::LogicalChannel(lcd) = item {
                for entry in &lcd.entries {
                    lcn_map.insert(entry.service_id, entry.logical_channel_number);
                }
            }
        }
    }
}
