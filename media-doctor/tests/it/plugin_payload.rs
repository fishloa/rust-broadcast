//! `media-doctor check --plugin <name>` (Phase 4 slice 8c): a data track's
//! samples decoded through a payload plug-in.
//!
//! The data packets are real MISB ST 0601 UAS Datalink Local Sets from
//! `private/fixtures/misb/` (provenance: ST 0902.5 Annex C). The TS carrier is
//! built at test time with `TsMuxStage`, as the source capture publishes no
//! UAS Datalink Local Set over MPEG-2 TS; its `stream_type` is the PMT-table
//! constant for PES private data (`StreamType::PesPrivateData`, ISO/IEC
//! 13818-1 Table 2-34), not a local literal.

use std::process::Command;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_common::timescale::Timescale;
use test_harness::collect::drive;
use test_harness::collect::{Collected as Media, CollectedTrack as Track};
use transmux::TsMuxStage;
use transmux::ir::{CodecConfig, DataCarriage, DataIdentity, Sample, TrackId, TrackSpec};
use transmux::mux_stage::MuxOutput;

const WHO: &str = "media_doctor::plugin_payload";
const RUN_DEADLINE: Duration = Duration::from_secs(60);
const MEDIA_TIMESCALE: u32 = 90_000;

fn timescale() -> Timescale {
    Timescale::try_new(MEDIA_TIMESCALE).expect("a non-zero timescale")
}

/// A one-track TS carrying `packets` as a PES-private-data elementary stream.
fn build_ts(packets: &[Vec<u8>]) -> Vec<u8> {
    let spec = TrackSpec::new(
        TrackId::new(1),
        timescale(),
        CodecConfig::Data {
            identity: DataIdentity::Ts {
                stream_type: dvb_si::tables::pmt::StreamType::PesPrivateData.to_u8(),
                descriptors: Vec::new(),
                carriage: DataCarriage::Pes,
            },
        },
    );
    let samples: Vec<Sample> = packets
        .iter()
        .enumerate()
        .map(|(index, packet)| {
            let dts = i64::try_from(index)
                .expect("a small sample index")
                .saturating_mul(i64::from(MEDIA_TIMESCALE));
            Sample::timed(packet.clone(), dts, dts, None)
        })
        .collect();
    let media = Media::new(vec![Track::new(spec, samples)], timescale());
    let mut stage = TsMuxStage::new(None);
    let outputs = drive(&mut stage, &media, Timestamp::ZERO).expect("mux the data track");
    let mut ts = Vec::new();
    for output in outputs {
        if let MuxOutput::Bytes(bytes) = output {
            ts.extend_from_slice(&bytes);
        }
    }
    assert!(!ts.is_empty(), "the muxer produced TS bytes");
    ts
}

fn run_media_doctor(args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_media-doctor");
    test_fixtures::process::output_bounded(Command::new(exe).args(args), RUN_DEADLINE)
        .expect("run media-doctor")
}

#[test]
fn cli_plugin_decodes_a_ts_data_track() {
    let Some(packet) = test_fixtures::require_bytes("misb/DynamicOnlyMISMMSPacketData.bin", WHO)
    else {
        return;
    };
    let ts = build_ts(&[packet.clone(), packet]);
    let path = crate::cli_routing::scratch("klv-plugin.ts", &ts);

    let out = run_media_doctor(&[
        "check",
        "-i",
        path.to_str().expect("a UTF-8 scratch path"),
        "--plugin",
        "motion-imagery-metadata",
    ]);
    assert!(
        out.status.success(),
        "media-doctor failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[motion-imagery-metadata]"),
        "the plug-in is named in the output: {stdout}"
    );
    assert!(
        stdout.contains("checksum_valid: true"),
        "the checksum is reported as a value: {stdout}"
    );
    let tag_5 = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("tag_5: "))
        .unwrap_or_else(|| panic!("no decoded tag 5 in: {stdout}"));
    let degrees: f64 = tag_5.parse().expect("tag 5 renders as a number");
    assert!(
        (degrees - 159.974_365).abs() < 0.01,
        "ST 0601 §8's tag 5 example is 159.974365°, got {degrees}"
    );
    let sample_lines = stdout.matches("sample ").count();
    assert_eq!(sample_lines, 2, "every data-track sample decodes: {stdout}");
}

#[test]
fn an_unknown_plugin_is_a_usage_error_listing_the_compiled_in_ones() {
    let out = run_media_doctor(&[
        "check",
        "-i",
        "does-not-exist.ts",
        "--plugin",
        "not-a-plugin",
    ]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a usage error exits 2: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("compiled-in plug-ins: motion-imagery-metadata"),
        "the error lists the compiled-in plug-ins: {stderr}"
    );
}
