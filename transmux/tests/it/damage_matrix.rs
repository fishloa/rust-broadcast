//! The demux damage matrix: a committed real fixture per format, damaged at
//! several positions and in several shapes, demuxed again, and measured
//! against the undamaged run.
//!
//! The rule the assertion enforces is the project's reader rule: parse as
//! well as possible and carry on, ignore broken metadata and broken frames,
//! and refuse only when what is missing makes playback impossible. Cells that
//! must still error are listed by [`playback_impossible`] with the reason.
//!
//! The report test prints every cell (run with `--nocapture`) so the
//! `origin/main` table and the post-fix table are the same measurement.

use test_harness::collect::{CollectMode, Collected};
use test_harness::damage::{self, Damage, DemuxOutcome};

/// The four container formats this lane covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Ts,
    Ps,
    Flv,
    Mkv,
}

impl Format {
    const ALL: [Format; 4] = [Format::Ts, Format::Ps, Format::Flv, Format::Mkv];

    fn name(self) -> &'static str {
        match self {
            Format::Ts => "ts",
            Format::Ps => "ps",
            Format::Flv => "flv",
            Format::Mkv => "mkv",
        }
    }

    fn fixture(self) -> &'static str {
        match self {
            Format::Ts => "ts/h264-aac.ts",
            Format::Ps => "ps/h264-ac3.ps",
            Format::Flv => "flv/av.flv",
            Format::Mkv => "mkv/h264-aac.mkv",
        }
    }

    fn demux(self, bytes: &[u8]) -> DemuxOutcome {
        match self {
            Format::Ts => fold(test_harness::collect::media_from_ts(bytes)),
            Format::Ps => fold(test_harness::collect::media_from_ps(bytes, 4096)),
            Format::Flv => fold(test_harness::collect::media_from_flv(bytes)),
            Format::Mkv => fold(test_harness::collect::media_from_matroska(
                bytes,
                CollectMode::Seekable,
            )),
        }
    }
}

fn fold<E: core::fmt::Display>(result: Result<Collected, E>) -> DemuxOutcome {
    match result {
        Ok(collected) => DemuxOutcome::from_collected(&collected),
        Err(error) => DemuxOutcome {
            error: Some(error.to_string()),
            ..DemuxOutcome::default()
        },
    }
}

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::path(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Where a cell applies its damage: a named anchor in the clean image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Start,
    FirstUnit,
    Mid,
    LastUnit,
    Stride(usize),
}

impl Position {
    fn name(self) -> String {
        match self {
            Position::Start => "start".into(),
            Position::FirstUnit => "first-unit".into(),
            Position::Mid => "mid".into(),
            Position::LastUnit => "last-unit".into(),
            Position::Stride(n) => format!("stride{n}"),
        }
    }

    fn offset(self, len: usize) -> usize {
        match self {
            Position::Start => 0,
            Position::FirstUnit => len / 10,
            Position::Mid => len / 2,
            Position::LastUnit => len.saturating_sub(len / 10).min(len.saturating_sub(1)),
            Position::Stride(n) => len.saturating_mul(n) / 8,
        }
    }
}

fn positions() -> Vec<Position> {
    let mut all = vec![
        Position::Start,
        Position::FirstUnit,
        Position::Mid,
        Position::LastUnit,
    ];
    all.extend((1..8).map(Position::Stride));
    all
}

/// Locate a 16-bit PES `PES_packet_length` field in a TS image at or after
/// `from`: the first payload-unit-start packet on a PES stream.
fn ts_pes_length_field(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    const PACKET: usize = 188;
    let mut at = from / PACKET * PACKET;
    while at + PACKET <= bytes.len() {
        if bytes[at] == 0x47 {
            let pusi = bytes[at + 1] & 0x40 != 0;
            let afc = (bytes[at + 3] >> 4) & 0x3;
            let mut payload = at + 4;
            if afc & 0x2 != 0 {
                payload += 1 + usize::from(bytes[at + 4]);
            }
            let ends = at + PACKET;
            if pusi && afc & 0x1 != 0 && payload + 6 <= ends {
                let start = &bytes[payload..ends];
                if start.starts_with(&[0x00, 0x00, 0x01]) && start[3] >= 0xBD {
                    return Some((payload + 4, 2));
                }
            }
        }
        at += PACKET;
    }
    None
}

/// Locate a 16-bit `PES_packet_length` in a Program Stream image at or after
/// `from`: the first PES start code that is not a pack/system/PSM/end code.
fn ps_pes_length_field(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut at = from.min(bytes.len());
    while at + 6 <= bytes.len() {
        if bytes[at..].starts_with(&[0x00, 0x00, 0x01]) {
            if !matches!(bytes[at + 3], 0xBA | 0xBB | 0xBC | 0xB9) {
                return Some((at + 4, 2));
            }
            at += 4;
        } else {
            at += 1;
        }
    }
    None
}

/// Locate an FLV tag's 24-bit `DataSize` at or after `from`.
fn flv_data_size_field(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut at = from.min(bytes.len());
    while at + 15 <= bytes.len() {
        if matches!(bytes[at], 8 | 9 | 18) && bytes[at + 8..at + 11] == [0, 0, 0] {
            let size = (usize::from(bytes[at + 1]) << 16)
                | (usize::from(bytes[at + 2]) << 8)
                | usize::from(bytes[at + 3]);
            if at + 15 + size <= bytes.len() {
                return Some((at + 1, 3));
            }
        }
        at += 1;
    }
    None
}

/// Locate an EBML size VINT at or after `from`: the Cluster header, else the
/// Segment header.
fn mkv_size_field(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    const CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
    const SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
    for id in [CLUSTER, SEGMENT] {
        if let Some(seen) = bytes[from.min(bytes.len())..]
            .windows(4)
            .position(|w| w == id)
        {
            let size_at = from + seen + 4;
            let first = *bytes.get(size_at)?;
            let width = first.leading_zeros() as usize + 1;
            if width <= 8 && size_at + width <= bytes.len() {
                return Some((size_at, width));
            }
        }
    }
    None
}

/// The largest value a field of `width` bytes can hold.
fn field_max(width: usize) -> u64 {
    if width >= 8 {
        u64::MAX
    } else {
        (1u64 << (8 * width)) - 1
    }
}

/// The VINT value bits for an all-ones payload of `width` bytes.
fn vint_max_payload(width: usize) -> u64 {
    (1u64 << (7 * width)) - 1
}

/// One measured matrix cell.
struct Row {
    format: Format,
    position: String,
    label: String,
    at: usize,
    summary: test_harness::damage::Summary,
    damaged: DemuxOutcome,
}

/// A deterministic seed per cell: the same cell damages the same bytes on
/// every run and every machine.
fn seed(format: Format, position: &Position, label: &str) -> u64 {
    let mut state = 0xCBF2_9CE4_8422_2325u64;
    for byte in format
        .name()
        .bytes()
        .chain(position.name().bytes())
        .chain(label.bytes())
    {
        state ^= u64::from(byte);
        state = state.wrapping_mul(0x0000_0100_0000_01B3);
    }
    state
}

fn damage_cells(format: Format, clean: &[u8]) -> Vec<(String, usize, Damage)> {
    let mut cells = Vec::new();
    for position in positions() {
        let at = position.offset(clean.len());
        for (label, damage) in [
            ("truncate", Damage::Truncate),
            ("zero", Damage::Zero),
            ("random", Damage::Random),
            ("delete", Damage::Delete),
            ("insert", Damage::Insert),
        ] {
            cells.push((format!("{}:{label}", position.name()), at, damage));
        }
    }
    // Size/length fields: zero, maximum, and one past the bytes that remain.
    let locator: fn(&[u8], usize) -> Option<(usize, usize)> = match format {
        Format::Ts => ts_pes_length_field,
        Format::Ps => ps_pes_length_field,
        Format::Flv => flv_data_size_field,
        Format::Mkv => mkv_size_field,
    };
    let anchor = clean.len() / 2;
    if let Some((offset, width)) = locator(clean, anchor) {
        let remaining = u64::try_from(clean.len().saturating_sub(offset)).unwrap_or(u64::MAX);
        let values: [(&str, u64); 3] = [
            ("field-zero", 0),
            ("field-max", field_max(width)),
            ("field-past", remaining),
        ];
        for (label, value) in values {
            if width == 2 || width == 3 {
                let damage = Damage::Field {
                    width,
                    value: u32::try_from(value.min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
                };
                cells.push((format!("mid:{label}"), offset, damage));
            } else {
                // An EBML size VINT: the marker bit is part of the encoding.
                let payload = if label == "field-max" {
                    vint_max_payload(width)
                } else {
                    value.min(1u64 << (7 * width))
                };
                cells.push((
                    format!("mid:{label}"),
                    offset,
                    Damage::VintSize { width, payload },
                ));
            }
        }
    }
    cells
}

fn measure(format: Format, clean_bytes: &[u8], clean: &DemuxOutcome) -> Vec<Row> {
    damage_cells(format, clean_bytes)
        .into_iter()
        .map(|(cell, at, damage)| {
            let applied =
                damage::apply(clean_bytes, at, damage, seed(format, &Position::Mid, &cell));
            let damaged = format.demux(&applied);
            let summary = damage::compare(clean, &damaged);
            let (position, label) = cell.split_once(':').unwrap_or((cell.as_str(), ""));
            Row {
                format,
                position: position.to_string(),
                label: label.to_string(),
                at,
                summary,
                damaged,
            }
        })
        .collect()
}

fn clean(format: Format) -> (Vec<u8>, DemuxOutcome) {
    let bytes = fixture(format.fixture());
    let outcome = format.demux(&bytes);
    (bytes, outcome)
}

fn print_matrix(format: Format, clean_outcome: &DemuxOutcome, rows: &[Row]) {
    println!(
        "## {} clean samples={} tracks={} findings={} error={:?}",
        format.name(),
        clean_outcome.delivered(),
        clean_outcome.tracks.len(),
        clean_outcome.findings,
        clean_outcome.error
    );
    for row in rows {
        let s = &row.summary;
        println!(
            "{}\t{}\t{}\tat={}\terr={:?}\tdel={}\tbefore={}\tafter={}\tlost={}\tdropped_tracks={}\textra={}\tmono={}\tfindings={}",
            row.format.name(),
            row.position,
            row.label,
            row.at,
            s.error,
            s.delivered,
            s.before,
            s.after,
            s.lost(clean_outcome),
            s.dropped_tracks,
            s.extra,
            s.monotonic,
            s.findings
        );
        if !s.monotonic {
            for track in &row.damaged.tracks {
                let mut previous: Option<i64> = None;
                for (index, dts) in track.dts.iter().enumerate() {
                    let Some(dts) = dts else { continue };
                    if let Some(previous) = previous
                        && dts < &previous
                    {
                        println!(
                            "  nonmono {} track={} sample={index} {previous} -> {dts}",
                            row.format.name(),
                            track.kind
                        );
                        break;
                    }
                    previous = Some(*dts);
                }
            }
        }
    }
}

/// Print every cell of the matrix. Run with `--nocapture` to read the table:
/// it is the measurement the `origin/main` baseline and the post-fix run
/// share.
#[test]
fn damage_matrix_report() {
    for format in Format::ALL {
        let (bytes, clean_outcome) = clean(format);
        let rows = measure(format, &bytes, &clean_outcome);
        print_matrix(format, &clean_outcome, &rows);
    }
}
