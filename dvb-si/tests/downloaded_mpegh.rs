//! Real-capture test: the MPEG-H 3D audio descriptors in the PMT of the
//! Fraunhofer-IIS `mpegh-test-content` transport-layer streams
//! (`private/fixtures/ts/mpegh-*.ts`; provenance in its `PROVENANCE.md`,
//! re-fetch via `tools/fetch-test-streams.sh mpegh-cicp01-baseline ...`).
//!
//! The streams are CC BY-NC-ND 4.0, so they live only in the private fixtures
//! repo, never the public tree. Each test **skips loudly** when `private/` is
//! not checked out. Expected values were read from the PMT bytes (dumped
//! independently with a script and with TSDuck: `Elementary stream: type 0x2D`,
//! `MPEG extended descriptor: MPEG-H 3D Audio (0x08)`).
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::pmt::StreamType;
use mpeg_ts::ts::TS_PACKET_SIZE;

/// `(file, expected ES_info loop bytes of the 0x2D stream)`.
fn pmt_es_descriptor_bytes(name: &str) -> Option<(StreamType, Vec<Vec<u8>>)> {
    let data = test_fixtures::require_bytes(&format!("ts/{name}.ts"), "downloaded_mpegh")?;
    let mut demux = SiDemux::builder().build();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        for ev in demux.feed(chunk) {
            let Ok(AnyTableSection::PmtSection(pmt)) = ev.table_section() else {
                continue;
            };
            for s in &pmt.streams {
                if s.stream_type.to_u8() == 0x2D {
                    let mut raw = Vec::new();
                    for d in s.es_info.iter() {
                        let d = d.expect("descriptor parses");
                        let AnyDescriptor::Mpeg2Extension(x) = &d else {
                            continue;
                        };
                        let mut b = vec![0u8; x.serialized_len()];
                        let n = x.serialize_into(&mut b).unwrap();
                        b.truncate(n);
                        raw.push(b);
                    }
                    return Some((s.stream_type, raw));
                }
            }
        }
    }
    panic!("{name}: no PMT stream with stream_type 0x2D found");
}

fn check(name: &str, expected: &[u8], profile: u8, layout: u8, compat: Option<&[u8]>) {
    let Some((st, raw)) = pmt_es_descriptor_bytes(name) else {
        return;
    };
    assert_eq!(st, StreamType::MhasAudioMain);
    assert_eq!(
        raw,
        vec![expected.to_vec()],
        "re-serialised descriptor must equal the PMT bytes"
    );
    let d = Mpeg2ExtensionDescriptor::parse(expected).unwrap();
    let Mpeg2ExtensionBody::MpegH3dAudio(a) = d.body else {
        panic!("expected MPEG-H 3D audio descriptor")
    };
    assert_eq!(a.profile_level_indication, profile);
    assert_eq!(a.reference_channel_layout, layout);
    assert_eq!(a.compatible_sets.as_deref(), compat);
    assert!(!a.interactivity_enabled);
}

#[test]
fn cicp01_baseline() {
    check(
        "mpegh-cicp01-baseline",
        &[0x3F, 0x04, 0x08, 0x10, 0x7F, 0xC1],
        0x10,
        1,
        None,
    );
}

#[test]
fn cicp01_low_complexity_with_baseline_compatible_set() {
    check(
        "mpegh-cicp01-lc",
        &[0x3F, 0x06, 0x08, 0x0B, 0x3F, 0xC1, 0x01, 0x10],
        0x0B,
        1,
        Some(&[0x10]),
    );
}

#[test]
fn cicp06_lc() {
    check(
        "mpegh-cicp06-lc",
        &[0x3F, 0x06, 0x08, 0x0C, 0x3F, 0xC6, 0x01, 0x11],
        0x0C,
        6,
        Some(&[0x11]),
    );
}

#[test]
fn cicp13_baseline() {
    check(
        "mpegh-cicp13-baseline",
        &[0x3F, 0x04, 0x08, 0x13, 0x7F, 0xCD],
        0x13,
        13,
        None,
    );
}
