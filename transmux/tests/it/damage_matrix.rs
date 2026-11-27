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

use broadcast_core::damage::{Damage as Report, DamageKind, DamagePolicy, Severity};
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
            if !matches!(bytes[at + 3], 0xB9..=0xBC) {
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
fn mkv_cluster_size_field(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    const CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
    let seen = bytes[from.min(bytes.len())..]
        .windows(4)
        .position(|w| w == CLUSTER)?;
    vint_at(bytes, from + seen + 4)
}

/// The Segment's own size VINT, which is wide enough for a large replacement
/// value without overwriting the first child.
fn mkv_segment_size_field(bytes: &[u8]) -> Option<(usize, usize)> {
    const SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
    let seen = bytes.windows(4).position(|w| w == SEGMENT)?;
    vint_at(bytes, seen + 4)
}

/// The (offset, width) of the EBML size VINT starting at `at`.
fn vint_at(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let first = *bytes.get(at)?;
    let width = first.leading_zeros() as usize + 1;
    (width <= 8 && at + width <= bytes.len()).then_some((at, width))
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

/// The VINT at `at`: its width in bytes and its value bits.
fn vint(bytes: &[u8], at: usize) -> Option<(usize, u64)> {
    let first = *bytes.get(at)?;
    let width = first.leading_zeros() as usize + 1;
    if width > 8 || at + width > bytes.len() {
        return None;
    }
    let mut value = u64::from(first) & u64::from(0xFFu8 >> width);
    for index in 1..width {
        value = (value << 8) | u64::from(*bytes.get(at + index)?);
    }
    Some((width, value))
}

/// The (offset, width) of a two-octet Matroska element's size VINT, for a
/// gapless element whose payload is one numeric scalar: the first occurrence
/// whose declared payload is 1..=8 bytes.
fn mkv_scalar_size_field(bytes: &[u8], id: [u8; 2]) -> Option<(usize, usize)> {
    const MAX_SCALAR: u64 = 8;
    bytes
        .windows(id.len())
        .position(|window| window == id)
        .and_then(|seen| {
            let at = seen + id.len();
            let (width, payload) = vint(bytes, at)?;
            (1..=MAX_SCALAR).contains(&payload).then_some((at, width))
        })
}

/// One measured matrix cell.
struct Row {
    format: Format,
    position: String,
    label: String,
    at: usize,
    damage: Damage,
    /// The seed the cell's damage was applied with, so a second run of the
    /// same cell sees the same bytes.
    seed: u64,
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
    type FieldLocator = fn(&[u8], usize) -> Option<(usize, usize)>;
    let locator: FieldLocator = match format {
        Format::Ts => ts_pes_length_field,
        Format::Ps => ps_pes_length_field,
        Format::Flv => flv_data_size_field,
        Format::Mkv => mkv_cluster_size_field,
    };
    let anchor = clean.len() / 2;
    if format == Format::Mkv {
        // The Cluster's size gets the zero; the Segment's own wide size gets
        // the maximum and one-past-the-end values, so a replacement VINT does
        // not overwrite the first child.
        if let Some((offset, width)) = mkv_cluster_size_field(clean, anchor) {
            cells.push((
                "mid:field-zero".to_string(),
                offset,
                Damage::VintSize { width, payload: 0 },
            ));
        }
        if let Some((offset, width)) = mkv_segment_size_field(clean) {
            let remaining = u64::try_from(clean.len().saturating_sub(offset)).unwrap_or(u64::MAX);
            let max = vint_max_payload(width);
            cells.push((
                "mid:field-max".to_string(),
                offset,
                Damage::VintSize {
                    width,
                    payload: max,
                },
            ));
            cells.push((
                "mid:field-past".to_string(),
                offset,
                Damage::VintSize {
                    width,
                    payload: remaining.min(max),
                },
            ));
        }
    } else if let Some((offset, width)) = locator(clean, anchor) {
        let remaining = u64::try_from(clean.len().saturating_sub(offset)).unwrap_or(u64::MAX);
        let values: [(&str, u64); 3] = [
            ("field-zero", 0),
            ("field-max", field_max(width)),
            ("field-past", remaining),
        ];
        for (label, value) in values {
            let damage = Damage::Field {
                width,
                value: u32::try_from(value.min(u64::from(u32::MAX))).unwrap_or(u32::MAX),
            };
            cells.push((format!("mid:{label}"), offset, damage));
        }
    }
    cells
}

fn measure(format: Format, clean_bytes: &[u8], clean: &DemuxOutcome) -> Vec<Row> {
    damage_cells(format, clean_bytes)
        .into_iter()
        .map(|(cell, at, damage)| {
            let seed = seed(format, &Position::Mid, &cell);
            let applied = damage::apply(clean_bytes, at, damage, seed);
            let damaged = format.demux(&applied);
            let summary = damage::compare(clean, &damaged);
            let (position, label) = cell.split_once(':').unwrap_or((cell.as_str(), ""));
            Row {
                format,
                position: position.to_string(),
                label: label.to_string(),
                at,
                damage,
                seed,
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

/// Why a cell may still error: the damage removed the only thing playback
/// needs. Every other cell must report the damage and carry on.
fn playback_impossible(damage: Damage, at: usize) -> Option<&'static str> {
    match damage {
        Damage::Truncate if at == 0 => Some("the input is empty: no track can be established"),
        _ => None,
    }
}

/// How many delivered samples may differ from the undamaged run: the one
/// container unit the damage touched.
fn allowed_extra(damage: Damage) -> usize {
    match damage {
        Damage::Truncate => 1,
        Damage::Zero | Damage::Random | Damage::Delete | Damage::Insert => 2,
        Damage::Field { .. } | Damage::VintSize { .. } => 1,
    }
}

/// A position whose damage sits in the last fifth of the file, where there is
/// no tail left to preserve.
fn in_the_tail(position: &str) -> bool {
    matches!(position, "last-unit" | "stride7")
}

/// Whether the damage falls inside the file's last Matroska Cluster. A
/// byte-shifting damage there leaves no following Cluster to resynchronise
/// to, so the rest of that Cluster is the container unit the damage touched.
fn in_last_cluster(format: Format, bytes: &[u8], at: usize) -> bool {
    const CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
    if format != Format::Mkv {
        return false;
    }
    bytes
        .windows(4)
        .rposition(|w| w == CLUSTER)
        .is_some_and(|last| at > last)
}

/// The rule the matrix enforces: an error only when playback is impossible;
/// every sample outside the damaged unit delivered byte-identically; the
/// timeline monotonic where the clean input's is; the tail after a mid-stream
/// damage preserved; and a structural loss reported.
#[test]
fn damage_matrix_cells_follow_the_best_effort_rule() {
    let mut violations = Vec::new();
    for format in Format::ALL {
        let (bytes, clean_outcome) = clean(format);
        let clean_monotonic = clean_outcome.monotonic();
        for row in measure(format, &bytes, &clean_outcome) {
            let s = &row.summary;
            let cell = format!(
                "{} {} {} @{}",
                row.format.name(),
                row.position,
                row.label,
                row.at
            );
            if let Some(_reason) = playback_impossible(row.damage, row.at) {
                continue;
            }
            if let Some(error) = &s.error {
                violations.push(format!("{cell}: errored: {error}"));
                continue;
            }
            if s.extra > allowed_extra(row.damage) {
                violations.push(format!(
                    "{cell}: {} delivered samples differ from the clean run",
                    s.extra
                ));
            }
            if clean_monotonic && !s.monotonic {
                violations.push(format!("{cell}: timestamps regressed"));
            }
            let tail_kept = s.after > 0
                || s.before.saturating_add(allowed_extra(row.damage)) >= clean_outcome.delivered();
            if row.position != "start"
                && !in_the_tail(&row.position)
                && !matches!(row.damage, Damage::Truncate)
                && !in_last_cluster(row.format, &bytes, row.at)
                && !tail_kept
            {
                violations.push(format!("{cell}: the tail after the damage is gone"));
            }
            if rows_structural_loss(row.damage) && s.lost(&clean_outcome) > 0 && s.findings == 0 {
                violations.push(format!("{cell}: a structural loss was not reported"));
            }
            if row.position != "start"
                && !matches!(row.damage, Damage::Truncate)
                && s.dropped_tracks > 0
            {
                violations.push(format!("{cell}: {} tracks vanished", s.dropped_tracks));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "{} cells break the rule:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// Whether a damage class removes container framing (so a silent loss of
/// samples is a defect) rather than only changing payload bytes (which a
/// demuxer that does not decode cannot detect).
fn rows_structural_loss(damage: Damage) -> bool {
    matches!(damage, Damage::Delete | Damage::Insert)
}

/// The payload offset of the first PSI section carried on `pid`.
fn psi_section(bytes: &[u8], pid: u16) -> Option<usize> {
    const PACKET: usize = 188;
    let mut at = 0usize;
    while at + PACKET <= bytes.len() {
        if bytes[at] == 0x47 {
            let packet_pid = (u16::from(bytes[at + 1] & 0x1F) << 8) | u16::from(bytes[at + 2]);
            let pusi = bytes[at + 1] & 0x40 != 0;
            let afc = (bytes[at + 3] >> 4) & 0x3;
            let mut payload = at + 4;
            if afc & 0x2 != 0 {
                payload += 1 + usize::from(bytes[at + 4]);
            }
            if packet_pid == pid && pusi && afc & 0x1 != 0 && payload < at + PACKET {
                let section = payload + 1 + usize::from(bytes[payload]);
                if section < at + PACKET {
                    return Some(section);
                }
            }
        }
        at += PACKET;
    }
    None
}

/// The PMT section's first byte in a TS image, via the PAT.
fn ts_pmt_section(bytes: &[u8]) -> Option<usize> {
    let pat = psi_section(bytes, 0)?;
    if bytes.get(pat) != Some(&0x00) {
        return None;
    }
    let section_len = (usize::from(bytes[pat + 1] & 0x0F) << 8) | usize::from(bytes[pat + 2]);
    let end = pat + 3 + section_len;
    let mut at = pat + 8;
    while at + 4 <= end && at + 4 <= bytes.len() {
        let program = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        let pid = (u16::from(bytes[at + 2] & 0x1F) << 8) | u16::from(bytes[at + 3]);
        if program != 0 {
            return psi_section(bytes, pid);
        }
        at += 4;
    }
    None
}

/// Broken metadata while the media is intact must be skipped, with the
/// pictures and sound unchanged.
#[test]
fn metadata_damage_does_not_stop_the_media() {
    // TS: damage the first PMT (a later, repeated one re-establishes the
    // tracks).
    let bytes = fixture(Format::Ts.fixture());
    let clean = Format::Ts.demux(&bytes);
    let pmt = ts_pmt_section(&bytes).expect("a PMT section");
    let damaged = damage::apply(&bytes, pmt, Damage::Random, 0x5151);
    let got = Format::Ts.demux(&damaged);
    assert!(got.error.is_none(), "TS: {:?}", got.error);
    assert_eq!(got.tracks.len(), clean.tracks.len(), "TS: tracks");
    assert_eq!(got.delivered(), clean.delivered(), "TS: samples");

    // FLV: the onMetaData script tag is informational.
    let bytes = fixture(Format::Flv.fixture());
    let clean = Format::Flv.demux(&bytes);
    let script = bytes
        .windows(11)
        .position(|w| w[0] == 18 && w[8..11] == [0, 0, 0])
        .expect("an FLV script tag");
    let damaged = damage::apply(&bytes, script + 13, Damage::Random, 0x5252);
    let got = Format::Flv.demux(&damaged);
    assert!(got.error.is_none(), "FLV: {:?}", got.error);
    assert_eq!(got.delivered(), clean.delivered(), "FLV: samples");

    // Matroska: the Cues index is optional.
    let bytes = fixture(Format::Mkv.fixture());
    let clean = Format::Mkv.demux(&bytes);
    let cues = bytes
        .windows(4)
        .rposition(|w| w == [0x1C, 0x53, 0xBB, 0x6B])
        .expect("a Cues element");
    let damaged = damage::apply(&bytes, cues + 8, Damage::Random, 0x5353);
    let got = Format::Mkv.demux(&damaged);
    assert!(got.error.is_none(), "mkv: {:?}", got.error);
    assert_eq!(got.delivered(), clean.delivered(), "mkv: samples");

    // Matroska: the gapless elements are metadata too. A broken `CodecDelay`
    // (RFC 9559 §5.1.4.1.6), `SeekPreRoll` (§5.1.4.1.26) or `DiscardPadding`
    // (§5.1.3.5.7) is skipped with a finding, and every coded sample stays.
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let clean = Format::Mkv.demux(&bytes);
    for (label, id) in [
        ("CodecDelay", [0x56, 0xAA]),
        ("SeekPreRoll", [0x56, 0xBB]),
        ("DiscardPadding", [0x75, 0xA2]),
    ] {
        let (at, width) = mkv_scalar_size_field(&bytes, id)
            .unwrap_or_else(|| panic!("mkv: no {label} element in the fixture"));
        let damaged = damage::apply(&bytes, at, Damage::VintSize { width, payload: 0 }, 0x5454);
        let got = Format::Mkv.demux(&damaged);
        assert!(got.error.is_none(), "mkv {label}: {:?}", got.error);
        assert_eq!(got.tracks.len(), clean.tracks.len(), "mkv {label}: tracks");
        assert_eq!(got.delivered(), clean.delivered(), "mkv {label}: samples");
        assert!(
            got.findings >= 1,
            "mkv {label}: the broken element was not reported"
        );
    }
}

/// An independent, mainstream demuxer must not deliver more than we do: this
/// is the "we are not worse than ffprobe" check on a sample of cells.
#[test]
fn a_damaged_file_is_not_worse_than_ffprobe() {
    use std::process::Command;
    use std::time::Duration;
    if !test_fixtures::process::tool_available("ffprobe", "-version") {
        eprintln!("skipping: ffprobe is not on PATH");
        return;
    }
    let dir = test_fixtures::process::scratch_dir("rz1-damage");
    let cells = [
        (Format::Ts, 41_266usize, Damage::Zero),
        (Format::Ts, 30_949, Damage::Delete),
        (Format::Flv, 31_237, Damage::Delete),
        (Format::Flv, 15_618, Damage::Random),
    ];
    for (index, (format, at, harm)) in cells.into_iter().enumerate() {
        let bytes = fixture(format.fixture());
        let clean = format.demux(&bytes);
        let damaged = damage::apply(&bytes, at, harm, 0x6000 + index as u64);
        let ours = format.demux(&damaged);
        let summary = damage::compare(&clean, &ours);
        let path = dir.join(format!("{}-{index}", format.name()));
        std::fs::write(&path, &damaged).expect("write the damaged copy");
        let mut command = Command::new("ffprobe");
        command.args([
            "-v",
            "error",
            "-show_packets",
            "-show_entries",
            "packet=codec_type",
            "-of",
            "csv",
        ]);
        command.arg(&path);
        let Ok(output) =
            test_fixtures::process::output_bounded(&mut command, Duration::from_secs(60))
        else {
            eprintln!("skipping {}: ffprobe did not run", format.name());
            continue;
        };
        let theirs = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.starts_with("packet,"))
            .count();
        println!(
            "{} at {at} {harm:?}: ours={} ffprobe={theirs}",
            format.name(),
            summary.delivered
        );
        assert!(
            summary.delivered.saturating_add(2) >= theirs,
            "{} at {at}: {} samples vs ffprobe's {theirs} packets",
            format.name(),
            summary.delivered
        );
    }
}

impl Format {
    /// Demux one image under `policy`, returning the error's own report when
    /// the policy stopped the run.
    fn demux_with(
        self,
        bytes: &[u8],
        policy: DamagePolicy,
    ) -> Result<DemuxOutcome, Box<Option<Report>>> {
        let collected = match self {
            Format::Ts => test_harness::collect::media_from_ts_with_policy(
                bytes,
                usize::MAX,
                transmux::AfdPersistence::UntilReplaced,
                policy,
            )
            .map_err(|error| match error {
                transmux::Error::Damage(refused) => Some(refused.damage().clone()),
                _ => None,
            }),
            Format::Ps => test_harness::collect::media_from_ps_with_policy(bytes, 4096, policy)
                .map_err(|error| match error {
                    transmux::Error::Damage(refused) => Some(refused.damage().clone()),
                    _ => None,
                }),
            Format::Flv => {
                test_harness::collect::media_from_flv_with_policy(bytes, usize::MAX, policy)
                    .map_err(|error| match error {
                        transmux::flv::FlvError::Damage(refused) => Some(refused.damage().clone()),
                        _ => None,
                    })
            }
            Format::Mkv => test_harness::collect::media_from_matroska_with_policy(
                bytes,
                CollectMode::Seekable,
                policy,
            )
            .map_err(|error| match error {
                transmux::Error::Damage(refused) => Some(refused.damage().clone()),
                _ => None,
            }),
        };
        collected
            .map(|collected| DemuxOutcome::from_collected(&collected))
            .map_err(|damage: Option<Report>| Box::new(damage))
    }
}

/// How far from the damaged bytes this format's resynchroniser may name the
/// place it recovered: one TS packet, one Program Stream pack, one FLV tag or
/// one Matroska element is the unit the walk restarts from.
fn report_reach(format: Format) -> u64 {
    match format {
        Format::Ts => 188,
        Format::Ps => 2048,
        Format::Flv => 4096,
        Format::Mkv => 4096,
    }
}

/// The run length a run-shaped damage removes or replaces.
fn damage_run(damage: Damage) -> u64 {
    match damage {
        Damage::Zero | Damage::Random | Damage::Delete | Damage::Insert => {
            u64::try_from(damage::run_len(damage)).unwrap_or(u64::MAX)
        }
        Damage::Truncate | Damage::Field { .. } | Damage::VintSize { .. } => 0,
    }
}

/// A report names the damage when its byte range (or the point it marks)
/// reaches the damaged run, within this format's resynchronisation reach.
fn names_the_damage(format: Format, at: usize, damage: Damage, report: &Report) -> bool {
    let Some(range) = report.location().byte_range() else {
        return false;
    };
    let at = u64::try_from(at).unwrap_or(u64::MAX);
    let reach = report_reach(format);
    let start = damage_run(damage);
    range.start() <= at.saturating_add(start).saturating_add(reach)
        && range.end().saturating_add(reach) >= at.saturating_sub(start)
}

/// A report a lost media unit always produces: the loss itself, never only a
/// note about metadata around it.
fn is_loss(kind: DamageKind) -> bool {
    matches!(
        kind,
        DamageKind::Resynchronised
            | DamageKind::MediaUnitDropped
            | DamageKind::MediaUnitDamaged
            | DamageKind::InputTruncated
            | DamageKind::MetadataSkipped
            | DamageKind::TrackDropped
    )
}

/// The one best-effort-reading contract, cell by cell: every skip a demuxer
/// makes is published as a `Damage` that names what happened, where in the
/// input and, where one is known, on which track — and no finding stands
/// without one.
#[test]
fn every_cell_publishes_its_damage_report() {
    let mut violations = Vec::new();
    for format in Format::ALL {
        let (bytes, clean_outcome) = clean(format);
        let total = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        for row in measure(format, &bytes, &clean_outcome) {
            let cell = format!(
                "{} {} {} @{}",
                row.format.name(),
                row.position,
                row.label,
                row.at
            );
            if playback_impossible(row.damage, row.at).is_some() {
                continue;
            }
            let damages = &row.damaged.damages;
            for damage in damages {
                if damage.what().is_empty() {
                    violations.push(format!("{cell}: a report names no structure"));
                }
                if let Some(range) = damage.location().byte_range()
                    && range.end() > total
                {
                    violations.push(format!(
                        "{cell}: report range {range} runs past the {total}-byte input"
                    ));
                }
            }
            if row.damaged.findings > 0 && damages.is_empty() {
                violations.push(format!(
                    "{cell}: {} findings but no damage report",
                    row.damaged.findings
                ));
            }
            let lost_content = matches!(row.damage, Damage::Delete | Damage::Insert)
                && row.summary.lost(&clean_outcome) > 0;
            if lost_content && damages.is_empty() {
                violations.push(format!("{cell}: lost samples with no report"));
            }
            if lost_content && !damages.iter().any(|damage| is_loss(damage.kind())) {
                violations.push(format!("{cell}: lost samples, only informational reports"));
            }
            if matches!(row.damage, Damage::Truncate)
                && !damages.is_empty()
                && !damages.iter().any(|damage| is_loss(damage.kind()))
            {
                violations.push(format!(
                    "{cell}: a truncated input reported as informational"
                ));
            }
            if !damages.is_empty()
                && !damages
                    .iter()
                    .any(|report| names_the_damage(row.format, row.at, row.damage, report))
            {
                violations.push(format!(
                    "{cell}: no report names the damaged bytes (reports at {:?})",
                    damages
                        .iter()
                        .map(|damage| damage.location().byte_range())
                        .collect::<Vec<_>>()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "{} cells break the damage-report contract:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// A caller that fails on degraded damage stops at the first such report, and
/// the stop carries that same report.
#[test]
fn a_failing_policy_stops_at_the_first_degraded_report() {
    let policy = DamagePolicy::new().fail_on(Severity::Degraded);
    let mut violations = Vec::new();
    for format in Format::ALL {
        let (bytes, clean_outcome) = clean(format);
        for row in measure(format, &bytes, &clean_outcome) {
            let cell = format!(
                "{} {} {} @{}",
                row.format.name(),
                row.position,
                row.label,
                row.at
            );
            if playback_impossible(row.damage, row.at).is_some() {
                continue;
            }
            let first = row
                .damaged
                .damages
                .iter()
                .find(|damage| damage.severity() == Severity::Degraded);
            let applied = damage::apply(&bytes, row.at, row.damage, row.seed);
            let stopped = format.demux_with(&applied, policy.clone());
            match (first, stopped.map_err(|boxed| *boxed)) {
                (Some(first), Err(Some(report))) => {
                    if report != first.clone() {
                        violations.push(format!(
                            "{cell}: stopped with {report:?}, first degraded report was {first:?}"
                        ));
                    }
                }
                (Some(first), Err(None)) => violations.push(format!(
                    "{cell}: a non-damage error instead of stopping at {first:?}"
                )),
                (Some(first), Ok(_)) => violations.push(format!(
                    "{cell}: the fail-on-degraded policy did not stop at {first:?}"
                )),
                (None, Ok(_)) | (None, Err(None)) => {}
                (None, Err(Some(report))) => violations.push(format!(
                    "{cell}: stopped at {report:?} with nothing to stop at"
                )),
            }
        }
    }
    assert!(
        violations.is_empty(),
        "{} cells break the damage policy:\n{}",
        violations.len(),
        violations.join("\n")
    );
}
