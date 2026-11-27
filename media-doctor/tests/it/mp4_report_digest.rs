//! Pin media-doctor's MP4 container report before the streaming check
//! (streaming-first Phase 3 slice 5): the report for every committed MP4
//! fixture must not change when the container check moves onto
//! `transmux::mp4_demux::Mp4Demux`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use media_doctor::{Report, check_container_codec};

const EXTS: &[&str] = &["mp4", "m4a", "m4v", "mov"];

const TABLE: &[(&str, &[&str])] = &[
    ("mp4/aac-sgpd.mp4", &[]),
    ("mp4/av1.mp4", &[]),
    ("mp4/cenc-av-enca.mp4", &[]),
    ("mp4/boxes/chnl-ipcm.mp4", &[]),
    ("mp4/boxes/cprt.mp4", &[]),
    ("mp4/boxes/hint-hmhd.mp4", &[]),
    ("mp4/boxes/hint-latm.mp4", &[]),
    ("mp4/boxes/hint-mtu-copy.mp4", &[]),
    ("mp4/boxes/hint-offsets.mp4", &[]),
    ("mp4/boxes/loudness.m4a", &[]),
    ("mp4/boxes/ssix-sidx.mp4", &[]),
    ("mp4/boxes/track-description.mp4", &[]),
    ("mp4/boxes/track-group-ster.mp4", &[]),
    ("mp4/boxes/tsel.mp4", &[]),
    ("mp4/cenc-boxes/aac-cenc.mp4", &[]),
    ("mp4/cenc-boxes/pcm-clear.mp4", &[]),
    ("mp4/cenc-boxes/v1-mvhd.mp4", &[]),
    ("mp4/cenc-layouts/clear-ffmpeg-base-moof.mp4", &[]),
    ("mp4/cenc-layouts/clear-ffmpeg-default.mp4", &[]),
    ("mp4/cenc-layouts/clear-ffmpeg-omit.mp4", &[]),
    (
        "mp4/cenc-layouts/clear-multitrun-base-moof-implicit.mp4",
        &[],
    ),
    ("mp4/cenc-layouts/clear-multitrun-omit-implicit.mp4", &[]),
    ("mp4/cenc-layouts/clear.mp4", &[]),
    ("mp4/cenc-layouts/enc-base-moof-explicit.mp4", &[]),
    ("mp4/cenc-layouts/enc-base-moof-implicit.mp4", &[]),
    ("mp4/cenc-layouts/enc-base-moof-none.mp4", &[]),
    ("mp4/cenc-layouts/enc-default-explicit.mp4", &[]),
    ("mp4/cenc-layouts/enc-default-implicit.mp4", &[]),
    ("mp4/cenc-layouts/enc-default-none.mp4", &[]),
    ("mp4/cenc-layouts/enc-omit-explicit.mp4", &[]),
    ("mp4/cenc-layouts/enc-omit-implicit.mp4", &[]),
    ("mp4/cenc-layouts/enc-omit-none.mp4", &[]),
    ("mp4/cenc-mixed-tracks.mp4", &[]),
    ("mp4/cenc.mp4", &[]),
    ("mp4/clap-vs-crop.mp4", &[]),
    ("mp4/cmaf-av-subtitle/av-aac-subtitle-frag.mp4", &[]),
    ("mp4/cmaf-av-subtitle/av-frag.mp4", &[]),
    ("mp4/cmaf-av-subtitle/av-subtitle-frag.mp4", &[]),
    ("mp4/cmaf/av-frag.mp4", &[]),
    ("mp4/colr-hdr.mp4", &[]),
    ("mp4/flac.mp4", &[]),
    ("mp4/frag/av1.frag.mp4", &[]),
    ("mp4/frag/flac.frag.mp4", &[]),
    ("mp4/frag/h264-high.frag.mp4", &[]),
    ("mp4/frag/hevc-main.frag.mp4", &[]),
    ("mp4/frag/mp3.frag.mp4", &[]),
    ("mp4/frag/mpeg2video.frag.mp4", &[]),
    ("mp4/frag/mpegh-stereo.frag.mp4", &[]),
    ("mp4/frag/opus.frag.mp4", &[]),
    ("mp4/frag/vp9.frag.mp4", &[]),
    ("mp4/frag/vvc.frag.mp4", &[]),
    ("mp4/h264-aac-frag.mp4", &[]),
    ("mp4/h264-aac-prog.mp4", &[]),
    ("mp4/h264-cbcs.mp4", &[]),
    ("mp4/h264-cenc.mp4", &[]),
    ("mp4/h264-high.mp4", &[]),
    ("mp4/h264-sidx.mp4", &[]),
    ("mp4/hevc-frag.mp4", &[]),
    ("mp4/hevc-main.mp4", &[]),
    ("mp4/hevc-vps-timing.mp4", &[]),
    ("mp4/hevc-vui-field-hrd.mp4", &[]),
    ("mp4/hevc-vui-field.mp4", &[]),
    ("mp4/hevc-vui-hrd.mp4", &[]),
    ("mp4/hevc-vui-ntsc.mp4", &[]),
    ("mp4/llcmaf/ll-chunked.mp4", &[]),
    ("mp4/noaud/h264-noaud.mp4", &[]),
    ("mp4/noaud/hevc-noaud.mp4", &[]),
    ("mp4/opus.mp4", &[]),
    ("mp4/piff-sample-encrypted.mp4", &[]),
    ("mp4/prft.mp4", &[]),
    ("mp4/progressive/av-faststart.mp4", &[]),
    ("mp4/progressive/av-prog.mp4", &[]),
    ("mp4/stpp.mp4", &[]),
    ("mp4/tref-avcp.mp4", &[]),
    ("mp4/uuid-boxes/base-clear.mp4", &[]),
    ("mp4/uuid-boxes/moov-uuid.mp4", &[]),
    ("mp4/uuid-boxes/segment-uuid.mp4", &[]),
    ("mp4/vp9.mp4", &[]),
    ("private/fixtures/mp4/camera/gopro-hero6-ble.mp4", &[]),
    (
        "private/fixtures/mp4/camera/gopro-karma.mp4",
        &[
            "[error] [container-sps-dimension-mismatch] packet=0 pid=0x0001 Sample entry declares 854x480 on track 1 but the embedded SPS decodes to 864x480 — ITU-T H.264 §7.4.2.1.1 cropped coded geometry",
        ],
    ),
    ("private/fixtures/mp4/camera/gopro-max-heromode.mp4", &[]),
    (
        "private/fixtures/mp4/melodie-avc3.mp4",
        &[
            "[info] [avc-interlaced-content] packet=0 pid=0x0001 AVC SPS on track 1 has frame_mbs_only_flag=0 (interlaced coding tools enabled) — ITU-T H.264 §7.3.2.1.1",
        ],
    ),
    (
        "private/fixtures/mp4/quicktime/iphone-x-live-photo.mov",
        &[],
    ),
    ("private/fixtures/mp4/quicktime/lavf-aac-tmcd.mov", &[]),
    ("private/fixtures/mp4/quicktime/lavf-pcm24.mov", &[]),
    (
        "private/fixtures/mp4/sarswitch-avc3.mp4",
        &[
            "[info] [avc-interlaced-content] packet=0 pid=0x0001 AVC SPS on track 1 has frame_mbs_only_flag=0 (interlaced coding tools enabled) — ITU-T H.264 §7.3.2.1.1",
        ],
    ),
    ("private/fixtures/mp4/tags/afconvert-aac.m4a", &[]),
    ("private/fixtures/mp4/tags/lavf-tagged.m4a", &[]),
    ("private/fixtures/mp4/tags/mp4box-chapters-nero.m4a", &[]),
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTS.contains(&e))
        {
            out.push(path);
        }
    }
}

fn digest(bytes: &[u8]) -> Vec<String> {
    let mut report = Report::new();
    check_container_codec(bytes, &mut report);
    report.findings().iter().map(|f| f.to_string()).collect()
}

fn public_rows() -> Vec<(String, Vec<String>)> {
    let mut paths = Vec::new();
    walk(&test_fixtures::root().join("mp4"), &mut paths);
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let rel = path
                .strip_prefix(test_fixtures::root())
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.display().to_string());
            (
                rel,
                digest(&fs::read(path).expect("read committed MP4 fixture")),
            )
        })
        .collect()
}

fn private_rows() -> Vec<(String, Vec<String>)> {
    let Some(root) = test_fixtures::private_root() else {
        println!("private fixtures are not checked out; skipping the private MP4 rows");
        return Vec::new();
    };
    test_fixtures::private_files("mp4", EXTS)
        .iter()
        .map(|path| {
            let rel = path
                .strip_prefix(&root)
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.display().to_string());
            (
                format!("private/{rel}"),
                digest(&fs::read(path).expect("read private MP4 fixture")),
            )
        })
        .collect()
}

#[test]
fn mp4_fixture_container_reports_are_pinned() {
    let mut actual: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (label, findings) in public_rows().into_iter().chain(private_rows()) {
        actual.insert(label, findings);
    }
    let pinned: BTreeMap<&str, &[&str]> = TABLE.iter().copied().collect();

    for (label, expected) in &pinned {
        match actual.get(*label) {
            Some(got) => {
                let expected: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
                assert_eq!(&expected, got, "container report changed for {label}");
            }
            None => assert!(
                label.starts_with("private/"),
                "committed fixture {label} is not in the pinned report table"
            ),
        }
    }
    for label in actual.keys() {
        assert!(
            pinned.contains_key(label.as_str()),
            "fixture {label} has no pinned container report"
        );
    }
}
