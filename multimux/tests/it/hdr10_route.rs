//! HDR10 over a real MPEG-TS stream (issue #1428): the multivariant playlist
//! says `VIDEO-RANGE=PQ`, the init segment carries `colr`/`mdcv`/`clli`, and
//! external readers (ffprobe, GPAC, Apple `mediastreamvalidator`) agree.
//!
//! Real artifact: `private/fixtures/video/hdr10/meridian-hdr10-60f.ts` (see its
//! PROVENANCE.md). Every test drives the production path: `RouteHandle` +
//! `ProgramSegmenter::pump` over a `Trunk`, read back through the axum router.
//! External tools skip loudly when absent.

use crate::support::hdr_route::{
    Source, contains, dump_route, init_bytes, load, master_text, route_app, route_app_from,
    scratch, validator_comments,
};
use test_fixtures::process::{msv, tool_available};

use broadcast_hls::MasterPlaylist;

const HDR_FIXTURE: &str = "video/hdr10/meridian-hdr10-60f.ts";
const SDR_FIXTURE: &str = "ts/hevc/main10.ts";

const NAL_LENGTH_PREFIX: usize = 4;
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;

fn strip_sei_nals(data: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(data.len());
    let mut removed = 0;
    let mut at = 0;
    while at + NAL_LENGTH_PREFIX <= data.len() {
        let len = u32::from_be_bytes(data[at..at + NAL_LENGTH_PREFIX].try_into().unwrap()) as usize;
        let end = at + NAL_LENGTH_PREFIX + len;
        let nal_type = (data[at + NAL_LENGTH_PREFIX] >> HEVC_NAL_TYPE_SHIFT) & HEVC_NAL_TYPE_MASK;
        if nal_type == transmux::hdr::HEVC_PREFIX_SEI_NUT {
            removed += 1;
        } else {
            out.extend_from_slice(&data[at..end]);
        }
        at = end;
    }
    (out, removed)
}

fn without_in_band_sei(mut sources: Vec<Source>) -> Vec<Source> {
    for src in &mut sources {
        let transmux::CodecConfig::Hevc { config, .. } = &mut src.spec.config else {
            continue;
        };
        assert!(src.spec.hdr.is_some());
        config
            .config
            .arrays
            .retain(|a| a.nal_unit_type != transmux::hdr::HEVC_PREFIX_SEI_NUT);
        let mut removed = 0;
        for sample in &mut src.samples {
            let (data, n) = strip_sei_nals(&sample.data);
            removed += n;
            sample.data = data.into();
        }
        assert!(removed > 0, "the fixture carries in-band SEI to strip");
    }
    sources
}

use crate::support::shared;

#[tokio::test]
async fn hdr10_route_master_says_video_range_pq_and_main10_codecs() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    let app = route_app(HDR_FIXTURE);
    let text = master_text(&app).await;
    let master = MasterPlaylist::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert!(text.contains("VIDEO-RANGE=PQ"), "{text}");
    let codecs = master.variants[0].codecs.clone().expect("CODECS");
    assert!(
        codecs.starts_with("hvc1.2.4.L"),
        "Main 10 from the SPS: {codecs}"
    );
}

#[tokio::test]
async fn sdr_hevc_route_master_has_no_hdr_video_range() {
    if test_fixtures::require(SDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    let app = route_app(SDR_FIXTURE);
    let text = master_text(&app).await;
    assert!(
        !text.contains("VIDEO-RANGE=PQ") && !text.contains("VIDEO-RANGE=HLG"),
        "{text}"
    );
    let init = init_bytes(&app).await;
    assert!(!contains(&init, b"mdcv") && !contains(&init, b"clli"));
}

#[tokio::test]
async fn hdr10_route_init_carries_colr_mdcv_clli() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    let init = init_bytes(&route_app(HDR_FIXTURE)).await;
    for fourcc in [b"colr", b"mdcv", b"clli"] {
        assert!(contains(&init, fourcc), "{fourcc:?}");
    }
}

#[tokio::test]
async fn ffprobe_reads_mastering_display_and_content_light_level() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    if !tool_available("ffprobe", "-version") {
        eprintln!(
            "SKIP ffprobe_reads_mastering_display_and_content_light_level: ffprobe not on PATH"
        );
        return;
    }
    let app = route_app_from(without_in_band_sei(load(HDR_FIXTURE)));
    let dir = scratch("hdr10-ffprobe");
    dump_route(&app, &dir).await;
    let media = std::fs::read_to_string(dir.join("media.m3u8")).unwrap();
    let mut joined = Vec::new();
    for line in media.lines() {
        if line.starts_with("#EXT-X-MAP") {
            let s = line.find("URI=\"").unwrap();
            let uri = line[s + 5..].split('"').next().unwrap();
            joined.extend(std::fs::read(dir.join(uri)).unwrap());
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            joined.extend(std::fs::read(dir.join(line.trim())).unwrap());
        }
    }
    let file = dir.join("joined.mp4");
    std::fs::write(&file, joined).unwrap();
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_frames",
            "-read_intervals",
            "%+#1",
        ])
        .arg(&file)
        .output()
        .expect("ffprobe");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    for needle in [
        "color_transfer=smpte2084",
        "color_primaries=bt2020",
        "side_data_type=Mastering display metadata",
        "side_data_type=Content light level metadata",
        "max_content=1000",
        "max_average=400",
        "max_luminance=10000000/10000",
        "min_luminance=1/10000",
    ] {
        assert!(
            text.contains(needle),
            "ffprobe output lacks {needle}:\n{text}"
        );
    }
}

#[tokio::test]
async fn gpac_dumps_the_mdcv_and_clli_boxes() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none() {
        return;
    }
    if !tool_available("MP4Box", "-version") {
        eprintln!("SKIP gpac_dumps_the_mdcv_and_clli_boxes: MP4Box not on PATH");
        return;
    }
    let app = route_app(HDR_FIXTURE);
    let dir = scratch("hdr10-gpac");
    std::fs::create_dir_all(&dir).unwrap();
    let init = dir.join("init.mp4");
    std::fs::write(&init, init_bytes(&app).await).unwrap();
    let out = std::process::Command::new("MP4Box")
        .arg("-diso")
        .arg(&init)
        .current_dir(&dir)
        .output()
        .expect("MP4Box");
    let xml = std::fs::read_to_string(dir.join("init_info.xml"))
        .unwrap_or_else(|_| String::from_utf8_lossy(&out.stdout).to_string());
    for needle in [
        "MasteringDisplayColourVolumeBox",
        "ContentLightLevelBox",
        "13250",
        "34500",
        "10000000",
    ] {
        assert!(xml.contains(needle), "MP4Box -diso lacks {needle}:\n{xml}");
    }
}

#[tokio::test]
async fn mediastreamvalidator_reports_no_hdr_specific_finding() {
    if test_fixtures::require(HDR_FIXTURE, "hdr10_route").is_none()
        || test_fixtures::require(SDR_FIXTURE, "hdr10_route").is_none()
    {
        return;
    }
    if !msv::available() {
        eprintln!(
            "SKIP mediastreamvalidator_reports_no_hdr_specific_finding: mediastreamvalidator not on PATH"
        );
        return;
    }
    let (hdr, sdr) = std::thread::scope(|scope| {
        let hdr = scope.spawn(|| {
            shared::run_async(validator_comments(
                route_app(HDR_FIXTURE),
                "hdr10-validator",
            ))
        });
        let sdr = scope.spawn(|| {
            shared::run_async(validator_comments(route_app(SDR_FIXTURE), "sdr-validator"))
        });
        (
            hdr.join().expect("validator thread"),
            sdr.join().expect("validator thread"),
        )
    });
    for c in &hdr {
        eprintln!("VALIDATOR hdr: {c}");
    }
    for c in &sdr {
        eprintln!("VALIDATOR sdr: {c}");
    }
    let only_hdr: Vec<_> = hdr.iter().filter(|c| !sdr.contains(c)).collect();
    assert!(
        only_hdr.is_empty(),
        "findings only the HDR10 route has: {only_hdr:?}"
    );
}
