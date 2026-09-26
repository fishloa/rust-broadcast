//! `dvb-tools services <file.ts>` — SDT service tree with LCNs.
//!
//! Drives `SiDemux` over an aligned 188-byte `.ts` capture, feeds SDT/NIT
//! sections into a `SectionSetCollector` keyed by PID, and prints one line
//! per service sorted by LCN (services without an LCN sort last, by
//! service_id).
//!
//! A `service_id` is only unique within one transport stream's namespace,
//! identified by `(original_network_id, transport_stream_id)` (ETSI EN 300
//! 468 §5.2.2/§5.2.3 — SDT-actual, SDT-other, and every transport-stream
//! entry in a NIT can each carry a service with the same numeric
//! `service_id` belonging to a different TS). Every table row is therefore
//! keyed by `(original_network_id, transport_stream_id, service_id)`, never
//! `service_id` alone.
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

/// Unique key for a service across the whole multiplex universe seen:
/// `(original_network_id, transport_stream_id, service_id)` — ETSI EN 300
/// 468 §5.2.2/§5.2.3. `service_id` alone is only unique inside one TS.
type ServiceKey = (u16, u16, u16);

/// One row in the service table.
struct ServiceRow {
    /// original_network_id of the TS this service belongs to.
    original_network_id: u16,
    /// transport_stream_id of the TS this service belongs to.
    transport_stream_id: u16,
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

/// Pick the LCN for `key` from a complete NIT (None if no entry).
fn lookup_lcn(map: &HashMap<ServiceKey, u16>, key: ServiceKey) -> Option<u16> {
    map.get(&key).copied()
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
        "LCN {lcn_str}  onid=0x{:04X} tsid=0x{:04X} service=0x{:04X}  {type_name:<32}  \"{name}\"  running={}",
        row.original_network_id,
        row.transport_stream_id,
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
    let mut services: HashMap<ServiceKey, ServiceRow> = HashMap::new();
    // Logical channel numbers, keyed the same way as `services` so an LCN
    // entry from one TS's NIT loop never matches a same-numbered service_id
    // from a different TS.
    let mut lcn_map: HashMap<ServiceKey, u16> = HashMap::new();
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
            } else if NIT_TABLE_IDS.contains(&table_id)
                && let Some(complete) = collect_event(&mut collector, pid_u16, &bytes)
            {
                nit_seen += 1;
                if let Ok(nit) = complete.nit_with_registry(&lcn_registry) {
                    absorb_nit(&nit, &mut lcn_map);
                }
            }
        }
    }

    if sdt_seen == 0 {
        eprintln!("dvb-tools services: no SDT seen (stream is empty or has no SDT)");
        return ExitCode::SUCCESS;
    }

    // Build sort: (has_lcn ? 0 : 1, lcn_or_max, key).
    let rows: Vec<(Option<u16>, ServiceKey, &ServiceRow)> = services
        .values()
        .map(|row| {
            let key = (
                row.original_network_id,
                row.transport_stream_id,
                row.service_id,
            );
            (lookup_lcn(&lcn_map, key), key, row)
        })
        .collect();
    let mut rows = rows;
    rows.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => x.cmp(&y).then(a.1.cmp(&b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });

    let with_lcn = rows.iter().filter(|(lcn, _, _)| lcn.is_some()).count();
    for (_, key, row) in &rows {
        print_row(row, lookup_lcn(&lcn_map, *key));
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

fn absorb_sdt<'a>(sdt: &'a CompleteSdt<'a>, services: &mut HashMap<ServiceKey, ServiceRow>) {
    for service in &sdt.services {
        let (name, type_name) = service_descriptor_view(service);
        let key = (
            sdt.original_network_id,
            sdt.transport_stream_id,
            service.service_id,
        );
        services.insert(
            key,
            ServiceRow {
                original_network_id: sdt.original_network_id,
                transport_stream_id: sdt.transport_stream_id,
                service_id: service.service_id,
                type_name,
                name,
                running_status: service.running_status,
            },
        );
    }
}

fn absorb_nit<'a>(
    nit: &'a dvb_si::collect::CompleteNit<'a>,
    lcn_map: &mut HashMap<ServiceKey, u16>,
) {
    for ts in &nit.transport_streams {
        for item in ts.descriptors.descriptors().iter().flatten() {
            if let AnyDescriptor::LogicalChannel(lcd) = item {
                for entry in &lcd.entries {
                    let key = (
                        ts.original_network_id,
                        ts.transport_stream_id,
                        entry.service_id,
                    );
                    lcn_map.insert(key, entry.logical_channel_number);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two services sharing the same numeric `service_id` but belonging to
    /// different transport streams must not collide in either map — the
    /// bug this replaces keyed by `service_id` alone (audit finding
    /// W-DT-2 / issue #1100).
    #[test]
    fn same_service_id_different_ts_do_not_collide() {
        let key_a: ServiceKey = (0x0001, 0x0006, 0x0301);
        let key_b: ServiceKey = (0x0001, 0x0007, 0x0301);
        let mut services: HashMap<ServiceKey, ServiceRow> = HashMap::new();
        services.insert(
            key_a,
            ServiceRow {
                original_network_id: key_a.0,
                transport_stream_id: key_a.1,
                service_id: key_a.2,
                type_name: "digital television service",
                name: Some("Service A".to_string()),
                running_status: RunningStatus::Running,
            },
        );
        services.insert(
            key_b,
            ServiceRow {
                original_network_id: key_b.0,
                transport_stream_id: key_b.1,
                service_id: key_b.2,
                type_name: "digital television service",
                name: Some("Service B".to_string()),
                running_status: RunningStatus::Running,
            },
        );
        assert_eq!(
            services.len(),
            2,
            "same service_id on two TSs must not collide"
        );

        let mut lcn_map: HashMap<ServiceKey, u16> = HashMap::new();
        lcn_map.insert(key_a, 101);
        lcn_map.insert(key_b, 202);
        assert_eq!(lookup_lcn(&lcn_map, key_a), Some(101));
        assert_eq!(lookup_lcn(&lcn_map, key_b), Some(202));
    }
}
