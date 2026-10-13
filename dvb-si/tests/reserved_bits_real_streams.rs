//! Real-stream reserved-bit survey for the ISO/IEC 13818-1 descriptors whose
//! serializers emit `reserved` bits (r03-W7). Walks every committed capture
//! (`fixtures/**/*.ts` through `SiDemux`, plus the TSDuck-compiled PMT `.bin`
//! files) and records the reserved-bit pattern each descriptor actually carries.
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::{
    avc_timing_and_hrd::AvcTimingAndHrdDescriptor, maximum_bitrate::MaximumBitrateDescriptor,
    mvc_extension::MvcExtensionDescriptor, smoothing_buffer::SmoothingBufferDescriptor,
    std::StdDescriptor, svc_extension::SvcExtensionDescriptor, system_clock::SystemClockDescriptor,
    video_stream::VideoStreamDescriptor,
};
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::pmt::PmtSection;
use mpeg_ts::ts::TS_PACKET_SIZE;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// Reserved bits of each byte (masked) per descriptor under survey, from the
/// spec tables in `docs/descriptors/iso_13818_1/`.
fn masks(tag: u8, body: &[u8]) -> Option<Vec<u8>> {
    let b = |i: usize| body.get(i).copied();
    match tag {
        // video_stream: reserved(5) is in byte 2, present only when MPEG_1_only_flag=0.
        0x02 if b(0)? & 0x04 == 0 => Some(vec![b(2)? & 0x1F]),
        0x0E => Some(vec![b(0)? & 0xC0]),
        0x0B => Some(vec![b(0)? & 0x40, b(1)? & 0x1F]),
        0x0C => Some(vec![b(2)? & 0x80]),
        0x10 => Some(vec![b(0)? & 0xC0, b(3)? & 0xC0]),
        0x11 => Some(vec![b(0)? & 0xFE]),
        0x30 => Some(vec![b(10)? & 0x1F, b(12)? & 0x01]),
        0x31 => Some(vec![b(4)? & 0x30]),
        0x2A => Some(vec![b(0)? & 0x7E]),
        _ => None,
    }
}

fn rt<'a, T: Parse<'a> + Serialize>(raw: &'a [u8], src: &str)
where
    <T as Parse<'a>>::Error: std::fmt::Debug,
    <T as Serialize>::Error: std::fmt::Debug,
{
    let d = T::parse(raw).unwrap_or_else(|e| panic!("{src}: parse {e:?}"));
    let mut out = vec![0u8; d.serialized_len()];
    d.serialize_into(&mut out).unwrap();
    assert_eq!(out, raw, "{src}: re-serialize differs from the real bytes");
}

/// Re-serializing each surveyed descriptor must reproduce the captured bytes
/// exactly — i.e. the emitted reserved bits equal what the real stream carries.
fn assert_byte_exact(tag: u8, body: &[u8], src: &str) {
    let mut raw = vec![tag, body.len() as u8];
    raw.extend_from_slice(body);
    match tag {
        0x02 => rt::<VideoStreamDescriptor>(&raw, src),
        0x0B => rt::<SystemClockDescriptor>(&raw, src),
        0x0E => rt::<MaximumBitrateDescriptor>(&raw, src),
        0x10 => rt::<SmoothingBufferDescriptor>(&raw, src),
        0x11 => rt::<StdDescriptor>(&raw, src),
        0x2A => rt::<AvcTimingAndHrdDescriptor>(&raw, src),
        0x30 => rt::<SvcExtensionDescriptor>(&raw, src),
        0x31 => rt::<MvcExtensionDescriptor>(&raw, src),
        _ => {}
    }
}

type Seen = BTreeMap<(u8, Vec<u8>), Vec<String>>;

fn survey_pmt(pmt: &PmtSection<'_>, src: &str, seen: &mut Seen) {
    let mut loops = vec![pmt.program_info];
    loops.extend(pmt.streams.iter().map(|s| s.es_info));
    for l in loops {
        for (tag, body) in l.raw_tags() {
            if let Some(m) = masks(tag, body) {
                assert_byte_exact(tag, body, src);
                seen.entry((tag, m)).or_default().push(src.to_string());
            }
        }
    }
}

#[test]
fn survey_real_reserved_bits() {
    let root = format!("{}/../fixtures", env!("CARGO_MANIFEST_DIR"));
    let mut seen = Seen::new();
    let mut bins = Vec::new();
    walk(Path::new(&root), "bin", &mut bins);
    for p in bins {
        if let Ok(data) = std::fs::read(&p)
            && let Ok(pmt) = PmtSection::parse(&data)
        {
            survey_pmt(&pmt, &p.display().to_string(), &mut seen);
        }
    }
    let mut tss = Vec::new();
    walk(Path::new(&root), "ts", &mut tss);
    for p in tss {
        let Ok(data) = std::fs::read(&p) else {
            continue;
        };
        let mut demux = SiDemux::builder().build();
        for chunk in data.chunks(TS_PACKET_SIZE) {
            if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
                continue;
            }
            for ev in demux.feed(chunk) {
                if let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() {
                    survey_pmt(&pmt, &p.display().to_string(), &mut seen);
                }
            }
        }
    }
    // Spec reserved patterns (all-ones per H.222.0); every real/oracle
    // occurrence must carry exactly these, and every flipped descriptor must
    // have been seen at least once so the check cannot silently go vacuous.
    let expect: [(u8, Vec<u8>); 8] = [
        (0x02, vec![0x1F]),
        (0x0B, vec![0x40, 0x1F]),
        (0x0E, vec![0xC0]),
        (0x10, vec![0xC0, 0xC0]),
        (0x11, vec![0xFE]),
        (0x2A, vec![0x7E]),
        (0x30, vec![0x1F, 0x01]),
        (0x31, vec![0x30]),
    ];
    for (tag, m) in &expect {
        assert!(
            seen.contains_key(&(*tag, m.clone())),
            "tag 0x{tag:02X} spec pattern not found in any fixture"
        );
    }
    for ((tag, m), srcs) in &seen {
        assert!(
            expect.iter().any(|(t, e)| t == tag && e == m),
            "tag 0x{tag:02X} real stream carries NON-spec reserved bits {m:02X?} in {srcs:?}"
        );
    }
}
