//! The streaming [`PsScanner`] splits every committed Program Stream fixture
//! into the same structural units as the whole-buffer walker
//! ([`mpeg_ps::program_stream::scan_packs`]), regardless of how the bytes are
//! chunked.

use mpeg_ps::program_stream;
use mpeg_ps::{PsEvent, PsLimits, PsScanner};

/// A structural unit reduced to comparable data.
#[derive(Debug, PartialEq, Eq)]
enum Unit {
    Pack { scr: u64, stuffing: u8 },
    SystemHeader { rate_bound: u32 },
    ProgramStreamMap { version: u8, streams: usize },
    Pes { stream_id: u8, len: usize },
    Resync(u64),
    End,
}

fn fixtures() -> Vec<Vec<u8>> {
    let dir = test_fixtures::path("ps");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("fixtures/ps")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension == "ps" || extension == "mpg")
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "fixtures/ps must hold at least one stream"
    );
    files
        .into_iter()
        .map(|path| std::fs::read(&path).expect("read fixture"))
        .collect()
}

/// What the whole-buffer walker sees, in wire order.
fn whole_buffer_units(data: &[u8]) -> Vec<Unit> {
    let scan = program_stream::scan_packs(data);
    assert!(
        scan.skipped.is_empty(),
        "a committed fixture must walk without a skipped region"
    );
    assert!(
        scan.remaining.is_empty(),
        "a committed fixture must have no trailing bytes"
    );
    let mut units = Vec::new();
    for pack in &scan.packs {
        units.push(Unit::Pack {
            scr: pack.pack_header.scr.ticks(),
            stuffing: pack.pack_header.stuffing_length,
        });
        if let Some(header) = &pack.system_header {
            units.push(Unit::SystemHeader {
                rate_bound: header.rate_bound,
            });
        }
        if let Some(map) = &pack.psm {
            units.push(Unit::ProgramStreamMap {
                version: map.version,
                streams: map.elementary_stream_map.len(),
            });
        }
        for packet in &pack.pes_packets {
            units.push(Unit::Pes {
                stream_id: packet.stream_id.get(),
                len: packet.serialized_len(),
            });
        }
    }
    units.push(Unit::End);
    units
}

/// What the scanner sees when fed `chunk`-sized pieces.
fn scanner_units(data: &[u8], chunk: usize) -> Vec<Unit> {
    let mut scanner = PsScanner::new(PsLimits::new());
    let mut units = Vec::new();
    for piece in data.chunks(chunk) {
        scanner.feed(piece);
        while let Some(event) = scanner.poll().expect("a clean fixture never errors") {
            units.push(unit(event));
        }
        assert!(
            scanner.buffered_len() <= PsLimits::new().max_pes_bytes,
            "the scanner held more than one bounded PES"
        );
    }
    scanner.finish();
    while let Some(event) = scanner.poll().expect("a clean fixture never errors") {
        units.push(unit(event));
    }
    units
}

fn unit(event: PsEvent<'_>) -> Unit {
    match event {
        PsEvent::Pack(header) => Unit::Pack {
            scr: header.scr.ticks(),
            stuffing: header.stuffing_length,
        },
        PsEvent::SystemHeader(header) => Unit::SystemHeader {
            rate_bound: header.rate_bound,
        },
        PsEvent::ProgramStreamMap(map) => Unit::ProgramStreamMap {
            version: map.version,
            streams: map.elementary_stream_map.len(),
        },
        PsEvent::Pes { stream_id, packet } => Unit::Pes {
            stream_id,
            len: packet.len(),
        },
        PsEvent::Resync { skipped } => Unit::Resync(skipped),
        PsEvent::End => Unit::End,
        other => panic!("unexpected PsEvent variant: {other:?}"),
    }
}

#[test]
fn every_fixture_splits_identically_at_every_chunk_size() {
    for data in fixtures() {
        let reference = whole_buffer_units(&data);
        for chunk in [1, 188, 64 * 1024] {
            assert_eq!(
                scanner_units(&data, chunk),
                reference,
                "chunk size {chunk} must not change the units"
            );
        }
    }
}

#[test]
fn a_pes_packet_that_straddles_a_feed_is_emitted_once_and_whole() {
    let data = std::fs::read(test_fixtures::path("ps/h264-ac3.ps")).expect("fixture");
    let reference = whole_buffer_units(&data);
    let pes_total = reference
        .iter()
        .filter(|unit| matches!(unit, Unit::Pes { .. }))
        .count();
    assert!(pes_total > 0);
    // Byte-at-a-time is the harshest split of every PES and pack header.
    let units = scanner_units(&data, 1);
    assert_eq!(
        units
            .iter()
            .filter(|u| matches!(u, Unit::Pes { .. }))
            .count(),
        pes_total
    );
}
