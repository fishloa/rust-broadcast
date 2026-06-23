//! `dvb-tools epg <file.ts> [--json]` — EPG schedule via [`EpgStore`].
//!
//! Constructs an `EpgStore`, drives `SiDemux` over the file, and feeds every
//! section into the store. The store is given every section's bytes (with PID
//! context for disambiguation); it ignores non-EIT sections internally and
//! accepts SDT sections too — service names are derived from the SDT
//! descriptor loop.
//!
//! With `--json`, the store is pretty-printed via serde. Without, a text view:
//! one block per service with one indented line per event.
use std::process::ExitCode;

use dvb_si::collect::SectionSetCollector;
use dvb_si::demux::SiDemux;
use dvb_si::epg::EpgStore;
use dvb_si::TableId;

use crate::util::{for_each_packet, read_file};

/// `table_id`s carrying a Service Description Table (actual + other transport).
/// SDT sections are routed to [`EpgStore::feed_sdt`] so service names attach to
/// the EIT-derived schedule — `feed_with_pid` only ingests EIT.
const SDT_TABLE_IDS: [u8; 2] = [
    TableId::ServiceDescriptionActual as u8,
    TableId::ServiceDescriptionOther as u8,
];

pub fn run(path: &str, json: bool) -> ExitCode {
    let data = match read_file(path, "dvb-tools epg") {
        Ok(d) => d,
        Err(code) => return code,
    };

    let mut demux = SiDemux::builder().build();
    let mut store = EpgStore::new();
    // `EpgStore::feed_with_pid` ingests EIT only; SDT must be reassembled
    // separately and handed to `feed_sdt` to attach service names.
    let mut sdt_collector = SectionSetCollector::new();
    for packet in for_each_packet(&data) {
        for event in demux.feed(&packet) {
            let pid = u16::from(event.pid());
            let table_id = event.table_id();
            let bytes: &[u8] = event.bytes();
            // EIT → schedule (ignore errors so a malformed/short-form section
            // can't abort the run; non-EIT completes are simply dropped here).
            let _ = store.feed_with_pid(Some(pid), bytes);
            // SDT → service names.
            if SDT_TABLE_IDS.contains(&table_id) {
                if let Ok(Some(complete)) = sdt_collector.push_section_with_pid(Some(pid), bytes) {
                    if let Ok(sdt) = complete.sdt() {
                        store.feed_sdt(&sdt);
                    }
                }
            }
        }
    }

    if json {
        match serde_json::to_string_pretty(&store) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("dvb-tools epg: serialize: {e}"),
        }
    } else {
        for key in store.services() {
            let label = store
                .service_name(key)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "(no name) orig={} ts={} svc={}",
                        key.original_network_id, key.transport_stream_id, key.service_id
                    )
                });
            println!("service 0x{0:04X}  {label}", key.service_id);
            match store.events(key) {
                Some(events) => {
                    for evt in events {
                        let start = evt
                            .start_time
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_else(|| "?".to_string());
                        let duration = evt
                            .duration
                            .map(|d| {
                                format!(
                                    "{}h{:02}m{:02}s",
                                    d.as_secs() / 3600,
                                    (d.as_secs() / 60) % 60,
                                    d.as_secs() % 60
                                )
                            })
                            .unwrap_or_else(|| "?".to_string());
                        let name = evt.event_name.as_deref().unwrap_or("?");
                        println!(
                            "    event_id=0x{0:04X}  start={start}  duration={duration}  \"{name}\"",
                            evt.event_id
                        );
                    }
                }
                None => println!("    (no events)"),
            }
        }
    }

    eprintln!(
        "-- services={} events={}",
        store.service_count(),
        store.event_count()
    );
    ExitCode::SUCCESS
}
