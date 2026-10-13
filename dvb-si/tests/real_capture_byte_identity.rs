//! Real-capture byte-identity survey (reserved-bit evidence hunt).
//!
//! Walks every `.ts`/`.m2ts`/`.mpg` capture under the gitignored
//! `.test-streams/` archive (skips cleanly when absent), demuxes PSI/SI with
//! `SiDemux`, and for
//!
//! * every table section: re-serializes the parsed section and compares with
//!   the original wire bytes (sections whose CRC is bad on the wire are
//!   counted separately, a re-serialized CRC is always good), and
//! * every descriptor in every PMT/SDT/NIT/BAT/EIT/CAT/TSDT/TOT loop:
//!   re-serializes the typed descriptor and compares with the original bytes.
//!
//! Policy (owner decision): parse is tolerant and serialize PRESERVES the
//! parsed reserved bits (typed fields; constructed values default to the
//! edition value), so a real sender's off-spec reserved bits re-serialize
//! byte-identically. [`TOLERATED`] therefore lists only what cannot be a
//! reserved-bit preservation: CRC-recompute noise and genuine sender bugs
//! that cannot parse. A *new* deviation (or any genuine serializer bug) fails
//! this test. Set `SURVEY_REPORT=1` to print the full per-descriptor table
//! (`--nocapture`).
#![cfg(feature = "ts")]

use broadcast_common::Serialize;
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{AnyDescriptor, DescriptorLoop, parse_loop};
use dvb_si::tables::AnyTableSection;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SYNC: u8 = 0x47;
const TS_LEN: usize = 188;
/// Packet strides probed: plain TS, M2TS/BDAV (4-byte prefix), RS-protected.
const STRIDES: [usize; 3] = [188, 192, 204];
/// Descriptor_tag of extension_descriptor (EN 300 468 §6.2.16).
const EXTENSION_TAG: u8 = 0x7F;
/// PMT stream_types whose elementary PID carries PSI-style sections
/// (ISO/IEC 13818-1 Table 2-34): private sections, DSM-CC A/B/C/D.
const SECTION_STREAM_TYPES: [u8; 5] = [0x05, 0x0A, 0x0B, 0x0C, 0x0D];

/// Known real-sender deviations, `(capture file-name prefix, survey key)`:
/// the capture's bytes differ from our spec-reserved-bit re-serialization.
/// Policy (owner decision): parse tolerantly, serialize the parsed reserved
/// bits; a deviation is listed here only if it is not a preservable reserved
/// region (e.g. section-level bits still pending preservation, or an
/// unparseable sender bug). A NEW deviation (new file, or a new
/// descriptor/table in an old file) fails the test; an entry that no longer
/// occurs is reported stale. The prefix is matched with `starts_with`
/// (`test-089.` covers the `test-089.*.ts` family cut from one capture).
const TOLERATED: &[(&str, &str)] = &[
    // Section-level reserved bits are now captured in typed fields and re-emitted, so every
    // real section re-serializes byte-identically (TDT test-120 / NIT test-016 / SDT / EIT /
    // AIT test-170 / PAT test-012,065 / DSM-CC / ST all graduated off this list).
    // test-062.ts is a TSDuck random-bits section stress capture: its `section_syntax_indicator`
    // is random too (0 on sections that carry a CRC_32). SSI is a structural decoded bit, not a
    // reserved bit, so the serializer re-emits the table's fixed SSI; the reserved bits proper
    // (private_indicator, reserved(2), version-byte reserved(2), bodies) are byte-exact here.
    ("test-062.", "PROGRAM_ASSOCIATION (tid 0x00)"),
    ("test-062.", "CONDITIONAL_ACCESS (tid 0x01)"),
    ("test-062.", "CONTAINER (tid 0x75)"),
    ("test-062.", "MPE_FEC (tid 0x78)"),
    ("test-062.", "MPE_IFEC (tid 0x7A)"),
    ("test-062.", "PROTECTION_MESSAGE (tid 0x7B)"),
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x, "ts" | "m2ts" | "mpg"))
        {
            out.push(p);
        }
    }
}

/// `(stride, offset_of_188_packet_in_stride, first_sync)` or `None`.
fn detect(data: &[u8]) -> Option<(usize, usize, usize)> {
    for &stride in &STRIDES {
        let pre = if stride == 192 { 4 } else { 0 };
        for start in 0..stride.min(data.len()) {
            if (0..5).all(|k| data.get(start + pre + k * stride) == Some(&SYNC)) {
                return Some((stride, pre, start));
            }
        }
    }
    None
}

#[derive(Default)]
struct Stat {
    total: u64,
    identical: u64,
    parse_err: u64,
    /// file -> (count, example)
    mismatch: BTreeMap<String, (u64, String)>,
}

#[derive(Default)]
struct Survey {
    sections: BTreeMap<String, Stat>,
    descs: BTreeMap<String, Stat>,
    bad_crc: u64,
    /// `(file, original hex, parse error)` of typed-parse failures (report only).
    parse_fail: Vec<(String, String, String)>,
    files: u64,
    files_with_si: u64,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

/// `byte_index:orig^re` pairs for the first few differing bytes.
fn diff(orig: &[u8], re: &[u8]) -> String {
    if orig.len() != re.len() {
        return format!("LEN orig={} re={}", orig.len(), re.len());
    }
    orig.iter()
        .zip(re)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .take(6)
        .map(|(i, (a, b))| format!("b{i}:{a:02X}->{b:02X}(xor {:02X})", a ^ b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn record(
    map: &mut BTreeMap<String, Stat>,
    key: String,
    file: &str,
    orig: &[u8],
    re: Option<Result<Vec<u8>, String>>,
) {
    let st = map.entry(key).or_default();
    st.total += 1;
    match re {
        None => st.identical += 1,
        Some(Err(_)) => st.parse_err += 1,
        Some(Ok(v)) if v == orig => st.identical += 1,
        Some(Ok(v)) => {
            let e = st.mismatch.entry(file.to_string()).or_insert_with(|| {
                (
                    0,
                    format!(
                        "{} | orig={} re={}",
                        diff(orig, &v),
                        hex(&orig[..orig.len().min(40)]),
                        hex(&v[..v.len().min(40)])
                    ),
                )
            });
            e.0 += 1;
        }
    }
}

fn survey_loop(s: &mut Survey, file: &str, seen: &mut BTreeSet<Vec<u8>>, l: &DescriptorLoop<'_>) {
    for (tag, body) in l.raw_tags() {
        let mut raw = vec![tag, body.len() as u8];
        raw.extend_from_slice(body);
        if !seen.insert(raw.clone()) {
            continue;
        }
        let first = parse_loop(&raw).next();
        let Some(Ok(d)) = first else {
            if let Some(Err(e)) = first {
                s.parse_fail
                    .push((file.to_string(), hex(&raw), format!("{e}")));
            }
            // typed parse failed: count under the tag
            let st = s
                .descs
                .entry(format!("0x{tag:02X} (parse-failed)"))
                .or_default();
            st.total += 1;
            st.parse_err += 1;
            continue;
        };
        if matches!(d, AnyDescriptor::Unknown { .. }) {
            continue;
        }
        let key = if tag == EXTENSION_TAG {
            format!(
                "0x{tag:02X}/ext 0x{:02X} {}",
                body.first().copied().unwrap_or(0),
                d.name()
            )
        } else {
            format!("0x{tag:02X} {}", d.name())
        };
        record(&mut s.descs, key, file, &raw, d.reserialize());
    }
}

fn survey_section(
    s: &mut Survey,
    file: &str,
    seen: &mut BTreeSet<Vec<u8>>,
    original: &[u8],
    crc_ok: bool,
    t: &AnyTableSection<'_>,
) {
    macro_rules! ser {
        ($t:expr) => {{
            let mut b = vec![0u8; $t.serialized_len()];
            $t.serialize_into(&mut b)
                .map(|_| b)
                .map_err(|e| format!("{e:?}"))
        }};
    }
    let re: Result<Vec<u8>, String> = match t {
        AnyTableSection::PatSection(t) => ser!(t),
        AnyTableSection::CatSection(t) => ser!(t),
        AnyTableSection::PmtSection(t) => ser!(t),
        AnyTableSection::TsdtSection(t) => ser!(t),
        AnyTableSection::DsmccSection(t) => ser!(t),
        AnyTableSection::NitSection(t) => ser!(t),
        AnyTableSection::SdtSection(t) => ser!(t),
        AnyTableSection::BatSection(t) => ser!(t),
        AnyTableSection::UntSection(t) => ser!(t),
        AnyTableSection::IntSection(t) => ser!(t),
        AnyTableSection::SatSection(t) => ser!(t),
        AnyTableSection::EitSection(t) => ser!(t),
        AnyTableSection::TdtSection(t) => ser!(t),
        AnyTableSection::RstSection(t) => ser!(t),
        AnyTableSection::StSection(t) => ser!(t),
        AnyTableSection::TotSection(t) => ser!(t),
        AnyTableSection::AitSection(t) => ser!(t),
        AnyTableSection::ContainerSection(t) => ser!(t),
        AnyTableSection::RctSection(t) => ser!(t),
        AnyTableSection::CitSection(t) => ser!(t),
        AnyTableSection::MpeFecSection(t) => ser!(t),
        AnyTableSection::RntSection(t) => ser!(t),
        AnyTableSection::MpeIfecSection(t) => ser!(t),
        AnyTableSection::ProtectionMessage(t) => ser!(t),
        AnyTableSection::DownloadableFontInfo(t) => ser!(t),
        AnyTableSection::DitSection(t) => ser!(t),
        AnyTableSection::SitSection(t) => ser!(t),
        AnyTableSection::MpeDatagram(t) => ser!(t),
        AnyTableSection::Unknown { .. } => return,
        _ => return,
    };
    if !crc_ok {
        s.bad_crc += 1;
        return;
    }
    let key = format!("{} (tid 0x{:02X})", t.name(), original[0]);
    record(&mut s.sections, key, file, original, Some(re));

    // Descriptor loops.
    let mut loops: Vec<&DescriptorLoop<'_>> = Vec::new();
    match t {
        AnyTableSection::PmtSection(p) => {
            loops.push(&p.program_info);
            loops.extend(p.streams.iter().map(|x| &x.es_info));
        }
        AnyTableSection::SdtSection(p) => loops.extend(p.services.iter().map(|x| &x.descriptors)),
        AnyTableSection::NitSection(p) => {
            loops.push(&p.network_descriptors);
            loops.extend(p.transport_streams.iter().map(|x| &x.descriptors));
        }
        AnyTableSection::BatSection(p) => {
            loops.push(&p.bouquet_descriptors);
            loops.extend(p.transport_streams.iter().map(|x| &x.descriptors));
        }
        AnyTableSection::EitSection(p) => loops.extend(p.events.iter().map(|x| &x.descriptors)),
        AnyTableSection::CatSection(p) => loops.push(&p.descriptors),
        AnyTableSection::TsdtSection(p) => loops.push(&p.descriptors),
        AnyTableSection::TotSection(p) => loops.push(&p.descriptors),
        _ => {}
    }
    for l in loops {
        survey_loop(s, file, seen, l);
    }
}

fn survey_file(s: &mut Survey, path: &Path) {
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    s.files += 1;
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    let Some((stride, pre, start)) = detect(&data) else {
        return;
    };
    // Pass 1: learn the PIDs that carry sections outside the well-known SI
    // set (private sections, DSM-CC, MPE) from the PMTs.
    let mut extra: BTreeSet<u16> = BTreeSet::new();
    {
        let mut d = SiDemux::builder().build();
        let mut p = start;
        while p + pre + TS_LEN <= data.len() {
            let pkt = &data[p + pre..p + pre + TS_LEN];
            p += stride;
            if pkt[0] != SYNC {
                continue;
            }
            for ev in d.feed(pkt) {
                if let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() {
                    for st in &pmt.streams {
                        if SECTION_STREAM_TYPES.contains(&st.stream_type.to_u8()) {
                            extra.insert(st.elementary_pid);
                        }
                    }
                }
            }
        }
    }
    let mut b = SiDemux::builder().emit_repeats(true);
    for pid in &extra {
        b = b.pid(mpeg_ts::pid::Pid::new(*pid));
    }
    let mut demux = b.build();
    let mut seen_sections: BTreeSet<Vec<u8>> = BTreeSet::new();
    let mut seen_desc: BTreeSet<Vec<u8>> = BTreeSet::new();
    let before = s.sections.values().map(|x| x.total).sum::<u64>();
    let mut pos = start;
    while pos + pre + TS_LEN <= data.len() {
        let pkt = &data[pos + pre..pos + pre + TS_LEN];
        pos += stride;
        if pkt[0] != SYNC {
            continue;
        }
        for ev in demux.feed(pkt) {
            let original: &[u8] = ev.bytes().as_ref();
            if !seen_sections.insert(original.to_vec()) {
                continue;
            }
            if let Ok(t) = ev.table_section() {
                survey_section(s, &file, &mut seen_desc, original, ev.crc_ok(), &t);
            }
        }
    }
    if s.sections.values().map(|x| x.total).sum::<u64>() > before {
        s.files_with_si += 1;
    }
}

#[test]
fn real_captures_reserialize_byte_identically() {
    let root = PathBuf::from(format!("{}/../.test-streams", env!("CARGO_MANIFEST_DIR")));
    if !root.exists() {
        eprintln!("SKIP: .test-streams absent (local-only real-capture archive)");
        return;
    }
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let mut s = Survey::default();
    for f in &files {
        survey_file(&mut s, f);
    }
    if std::env::var_os("SURVEY_REPORT").is_some() {
        eprintln!(
            "files={} with_si={} bad_crc_sections_skipped={}",
            s.files, s.files_with_si, s.bad_crc
        );
        for (title, m) in [("SECTIONS", &s.sections), ("DESCRIPTORS", &s.descs)] {
            eprintln!("== {title}: key | total | identical | parse_err | mismatching");
            for (k, st) in m {
                let mm: u64 = st.mismatch.values().map(|x| x.0).sum();
                eprintln!(
                    "{k} | {} | {} | {} | {mm}",
                    st.total, st.identical, st.parse_err
                );
                for (f, (n, ex)) in &st.mismatch {
                    eprintln!("    {f}: x{n}: {ex}");
                }
            }
        }
    }
    if std::env::var_os("SURVEY_REPORT").is_some() {
        for (f, h, e) in &s.parse_fail {
            eprintln!("PARSE-FAIL {f}: {e} | {h}");
        }
    }
    let tolerated = |f: &str, k: &str| TOLERATED.iter().any(|(p, tk)| f.starts_with(p) && *tk == k);
    let mut failures = Vec::new();
    for (k, st) in s.sections.iter().chain(s.descs.iter()) {
        for (f, (n, ex)) in &st.mismatch {
            if !tolerated(f, k) {
                failures.push(format!("{f}: {k} x{n}: {ex}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "real captures re-serialize differently (new deviation or serializer bug):\n{}",
        failures.join("\n")
    );
    // The allowlist must not go stale: every tolerated pair must still occur.
    for (p, k) in TOLERATED {
        let hit = s
            .sections
            .get(*k)
            .into_iter()
            .chain(s.descs.get(*k))
            .any(|st| st.mismatch.keys().any(|f| f.starts_with(p)));
        assert!(hit, "stale TOLERATED entry ({p}, {k}): remove it");
    }
}
