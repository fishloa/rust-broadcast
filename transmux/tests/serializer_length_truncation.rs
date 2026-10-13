//! Serializer length/count/value truncation guard for `transmux` (#1129).
//!
//! Two halves:
//!
//! 1. Behavioural: version-0 FullBoxes carry 32-bit `creation_time` /
//!    `modification_time` / `duration` / `media_time`. A 64-bit value above
//!    `u32::MAX` used to be written as `value as u32` (silently wrapped); it
//!    must be a `FieldOverflow` error, while `u32::MAX` itself and every
//!    version-1 value still serialize.
//! 2. Lexical: no production line narrows `<expr>.len()`/`.count()` with
//!    `as u8|u16|u32` unless its enclosing function also range-checks (a
//!    `fit_*`/`try_from`/`MAX`/`>` comparison), or it is on the reasoned
//!    allowlist below.

use broadcast_common::Serialize;
use transmux::{MediaHeaderBox, MovieHeaderBox, ProducerReferenceTimeBox, TrackHeaderBox, TrackId};

const OVER_U32: u64 = 0x1_0000_0000;

fn serialize<T: Serialize>(t: &T) -> Result<Vec<u8>, T::Error> {
    let mut buf = vec![0u8; t.serialized_len()];
    t.serialize_into(&mut buf)?;
    Ok(buf)
}

fn mvhd(version: u8, ct: u64, mt: u64, dur: u64) -> MovieHeaderBox {
    MovieHeaderBox {
        version,
        flags: 0,
        creation_time: ct,
        modification_time: mt,
        timescale: 1000,
        duration: dur,
        rate: 0x0001_0000,
        volume: 0x0100,
        matrix: [0; 9],
        next_track_id: TrackId::new(2),
    }
}

fn tkhd(version: u8, ct: u64, mt: u64, dur: u64) -> TrackHeaderBox {
    TrackHeaderBox {
        version,
        flags: 3,
        creation_time: ct,
        modification_time: mt,
        track_id: TrackId::new(1),
        duration: dur,
        layer: 0,
        alternate_group: 0,
        volume: 0,
        matrix: [0; 9],
        width: 0,
        height: 0,
    }
}

fn mdhd(version: u8, ct: u64, mt: u64, dur: u64) -> MediaHeaderBox {
    MediaHeaderBox {
        version,
        flags: 0,
        creation_time: ct,
        modification_time: mt,
        timescale: 90_000,
        duration: dur,
        language: 0x55C4,
    }
}

fn prft(version: u8, media_time: u64) -> ProducerReferenceTimeBox {
    ProducerReferenceTimeBox {
        version,
        flags: 0,
        reference_track_id: TrackId::new(1),
        ntp_timestamp: 1,
        media_time,
    }
}

#[test]
fn v0_header_boxes_reject_values_above_u32() {
    // (creation, modification, duration): one over-range field at a time.
    let cases = [
        (OVER_U32, 0, 0),
        (0, OVER_U32, 0),
        (0, 0, OVER_U32),
        (u64::MAX, 0, 0),
    ];
    for (ct, mt, dur) in cases {
        assert!(
            serialize(&mvhd(0, ct, mt, dur)).is_err(),
            "mvhd {ct} {mt} {dur}"
        );
        assert!(
            serialize(&tkhd(0, ct, mt, dur)).is_err(),
            "tkhd {ct} {mt} {dur}"
        );
        assert!(
            serialize(&mdhd(0, ct, mt, dur)).is_err(),
            "mdhd {ct} {mt} {dur}"
        );
        // Version 1 holds the same values in 64 bits.
        assert!(serialize(&mvhd(1, ct, mt, dur)).is_ok());
        assert!(serialize(&tkhd(1, ct, mt, dur)).is_ok());
        assert!(serialize(&mdhd(1, ct, mt, dur)).is_ok());
    }
    assert!(serialize(&prft(0, OVER_U32)).is_err());
    assert!(serialize(&prft(1, OVER_U32)).is_ok());
}

#[test]
fn v0_header_boxes_accept_u32_max() {
    let m = u64::from(u32::MAX);
    let v = serialize(&mvhd(0, m, m, m)).expect("mvhd v0 u32::MAX");
    assert_eq!(&v[12..16], &u32::MAX.to_be_bytes());
    assert!(serialize(&tkhd(0, m, m, m)).is_ok());
    assert!(serialize(&mdhd(0, m, m, m)).is_ok());
    assert!(serialize(&prft(0, m)).is_ok());
}

// ---------------------------------------------------------------------------
// Lexical guard
// ---------------------------------------------------------------------------

/// `(file, exact trimmed line, reason)` — narrowing sites with no range check
/// in the same function, each justified. A stale entry (line no longer
/// present) fails the guard, so the list cannot rot.
const ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "src/annexb.rs",
        "out.extend_from_slice(&(nal.len() as u32).to_be_bytes());",
        "infallible util over a caller slice; NAL > 4 GiB needs a >4 GiB \
         input slice. Every demux path feeding it is capped at 4 MiB \
         (ts_demux MAX_PES_BUFFER_BYTES).",
    ),
    (
        "src/klv.rs",
        "out.push(BER_LONG_FORM_FLAG | (value_bytes.len() as u8));",
        "value_bytes is the minimal big-endian bytes of a u64: 1..=8 long.",
    ),
    (
        "src/flv.rs",
        "out.extend_from_slice(&(s.len() as u16).to_be_bytes());",
        "amf0_string is only ever called with the crate's own static \
         property names (`onMetaData`, `duration`, ...), a few bytes each.",
    ),
    (
        "src/flv.rs",
        "out.extend_from_slice(&(props.len() as u32).to_be_bytes());",
        "props is the onMetaData property list built from at most 8 \
         static entries just above.",
    ),
    (
        "src/rtp.rs",
        "out.extend_from_slice(&(nal.len() as u32).to_be_bytes());",
        "length_prefix_nals: NALs come from RTP depacketisation, bounded by \
         the reassembler caps (FU-A / STAP-A), never near 4 GiB.",
    ),
];

fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `.len()`/`.count()` immediately narrowed with `as u8|u16|u32`.
fn narrows_len(line: &str) -> bool {
    let mut rest = line;
    while let Some(i) = rest.find(')') {
        let (head, tail) = rest.split_at(i + 1);
        rest = tail;
        if head.ends_with(".len()") || head.ends_with(".count()") {
            let t = tail.trim_start();
            for w in ["as u8", "as u16", "as u32"] {
                if t.strip_prefix(w).is_some_and(|r| {
                    r.chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                }) {
                    return true;
                }
            }
        }
    }
    false
}

fn has_guard(text: &str) -> bool {
    ["fit_", "FieldOverflow", "try_from", "::MAX", "_MAX", "MAX_"]
        .iter()
        .any(|t| text.contains(t))
        || text.contains("> 0x")
        || text.lines().any(|l| {
            let l = l.trim_start();
            (l.starts_with("if ") || l.contains("&& ")) && (l.contains(" > ") || l.contains(" >= "))
        })
}

#[test]
fn len_narrowing_sites_are_range_checked_or_allowlisted() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files(&root.join("src"), &mut files);
    assert!(files.len() > 20, "scan walked only {} files", files.len());

    let mut problems = Vec::new();
    let mut used = vec![false; ALLOWLIST.len()];
    for path in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let body = std::fs::read_to_string(path).unwrap();
        let prod = &body[..body.find("#[cfg(test)]").unwrap_or(body.len())];
        let lines: Vec<&str> = prod.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with("//") || !narrows_len(t) {
                continue;
            }
            if let Some(k) = ALLOWLIST.iter().position(|(f, l, _)| *f == rel && *l == t) {
                used[k] = true;
                continue;
            }
            let mut start = n;
            while start > 0 && !lines[start].contains("fn ") {
                start -= 1;
            }
            if !has_guard(&lines[start..=n].join("\n")) {
                problems.push(format!("{rel}:{}: unchecked length narrowing: {t}", n + 1));
            }
        }
    }
    for (k, u) in used.iter().enumerate() {
        assert!(
            *u,
            "stale allowlist entry: {} :: {}",
            ALLOWLIST[k].0, ALLOWLIST[k].1
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn guard_detector_bites() {
    assert!(narrows_len("buf[0] = x.len() as u8;"));
    assert!(narrows_len("f(&(a.b.count() as u32));"));
    assert!(!narrows_len("let n = x.len() as u64;"));
    assert!(!narrows_len("let n = x.len() as usize;"));
    assert!(!has_guard("fn f() { buf[0] = x.len() as u8; }"));
    assert!(has_guard("fn f() { let n = fit_u8(x.len(), \"a\")?; }"));
    assert!(has_guard("fn f() {\n if x.len() > 255 { return; }\n }"));
}
