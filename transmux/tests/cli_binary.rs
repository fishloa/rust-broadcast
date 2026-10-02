//! `transmux` **binary** gate — runs the built CLI against a fixture and checks
//! the resulting files with an independent tool (ffprobe / ffmpeg).
//!
//! The other CLI test (`cli.rs`) drives the library entry points directly.
//! This one exists because the audit fix wave found defects that only appear
//! when the *binary* is run end to end: the sample-merge order and the
//! per-segment `#EXTINF` accounting are both decisions taken in
//! `transmux::cli::package`, and an oracle on the produced files is the only
//! way to see them.
//!
//! Every test skips **loudly** (prints why) when its oracle is missing, so a
//! machine without ffmpeg stays green.

#![cfg(feature = "cli")]

mod common;

use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

/// The built `transmux` binary (cargo sets this for a package's own bins).
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_transmux")
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/cli-binary-tmp")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run the CLI: `<in> -o <out> -f <format> [extra…]`.
fn run_cli(input: &std::path::Path, out: &std::path::Path, format: &str, extra: &[&str]) {
    let status = Command::new(bin())
        .arg(input)
        .arg("-o")
        .arg(out)
        .arg("-f")
        .arg(format)
        .args(extra)
        .output()
        .expect("spawn the transmux binary");
    assert!(
        status.status.success(),
        "transmux {format} failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

/// `ffprobe -count_packets` → `{codec_type: nb_read_packets}` for one file.
fn packet_counts(path: &std::path::Path) -> Vec<(String, u64)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_packets",
            "-show_entries",
            "stream=codec_type,nb_read_packets",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(
        out.status.success(),
        "ffprobe rejected {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split(',');
            let kind = it.next()?.to_string();
            // csv=p=0 prints `codec_type,nb` but the packet count is the last
            // numeric field; take whichever parses.
            let count = l.split(',').rev().find_map(|f| f.parse::<u64>().ok())?;
            Some((kind, count))
        })
        .collect()
}

/// Frames of the first video stream, decoded, as md5 lines.
fn video_framemd5(path: &std::path::Path) -> Vec<String> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v", "-f", "framemd5", "-"])
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg failed on {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split(',').nth(6).map(|s| s.trim().to_string()))
        .collect()
}

/// Item 1: every segment of an `h264_aac` HLS output carries **both** streams,
/// and the per-segment counts sum to the source's own totals.
#[test]
fn hls_segments_each_carry_audio_and_video() {
    if !have("ffprobe") || !have("ffmpeg") {
        eprintln!("SKIP cli_binary: ffprobe/ffmpeg not on PATH");
        return;
    }
    let dir = scratch("hls-av");
    let input = fixtures().join("ts/h264_aac.ts");
    let m3u8 = dir.join("pl.m3u8");
    run_cli(&input, &m3u8, "hls", &["--segment-duration", "2"]);

    let text = std::fs::read_to_string(&m3u8).expect("playlist");
    let segments: Vec<PathBuf> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| dir.join(l))
        .collect();
    assert!(
        segments.len() >= 2,
        "need several segments, got {}:\n{text}",
        segments.len()
    );

    // A media segment is `styp`+`moof`+`mdat` with no `moov`, so a player (and
    // ffprobe) needs the `#EXT-X-MAP` init segment prepended — concatenate them
    // into a scratch file per segment.
    let init = std::fs::read(dir.join("init.mp4")).expect("init.mp4 written");
    let src = packet_counts(&input);
    let mut video_total = 0u64;
    let mut audio_total = 0u64;
    for (i, seg) in segments.iter().enumerate() {
        let mut whole = init.clone();
        whole.extend_from_slice(&std::fs::read(seg).expect("segment bytes"));
        let joined = dir.join(format!("joined{i}.mp4"));
        std::fs::write(&joined, &whole).unwrap();
        let counts = packet_counts(&joined);
        let v = counts
            .iter()
            .find(|(k, _)| k == "video")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        let a = counts
            .iter()
            .find(|(k, _)| k == "audio")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert!(
            v > 0,
            "segment {i} ({}) has no video packets: {counts:?}",
            seg.display()
        );
        assert!(
            a > 0,
            "segment {i} ({}) has no audio packets — the merge order lost it: {counts:?}",
            seg.display()
        );
        video_total += v;
        audio_total += a;
    }

    let source_v = src
        .iter()
        .find(|(k, _)| k == "video")
        .map(|(_, n)| *n)
        .unwrap();
    let source_a = src
        .iter()
        .find(|(k, _)| k == "audio")
        .map(|(_, n)| *n)
        .unwrap();
    assert_eq!(
        video_total, source_v,
        "video packet count must be conserved across segments"
    );
    assert_eq!(
        audio_total, source_a,
        "audio packet count must be conserved across segments"
    );

    // Independent decode: concatenating the segments must reproduce the source
    // frames exactly.
    let concat = dir.join("concat.mp4");
    let mut bytes = init;
    for seg in &segments {
        bytes.extend_from_slice(&std::fs::read(seg).expect("segment bytes"));
    }
    std::fs::write(&concat, &bytes).unwrap();
    assert_eq!(
        video_framemd5(&concat),
        video_framemd5(&input),
        "decoded frames of the concatenated segments must equal the source's"
    );
}

fn have_msv() -> Option<&'static str> {
    [
        "/usr/local/bin/mediastreamvalidator",
        "mediastreamvalidator",
    ]
    .into_iter()
    .find(|p| {
        std::path::Path::new(p).exists() || {
            let mut c = Command::new(p);
            c.arg("--help");
            common::run_bounded(c, common::PROBE_DEADLINE, "mediastreamvalidator --help")
                .is_ok_and(|o| o.status.code().is_some())
        }
    })
}

/// Item 2: the playlist's `#EXTINF` values are the segments' real durations,
/// `#EXT-X-TARGETDURATION` is at least the largest of them (RFC 8216 §4.3.3.1),
/// and the segments are media-only — the `#EXT-X-MAP` init file carries the
/// `ftyp`/`moov`, so a segment must not repeat them.
#[test]
fn hls_playlist_durations_are_real_and_segments_are_media_only() {
    if !have("ffprobe") {
        eprintln!("SKIP cli_binary: ffprobe not on PATH");
        return;
    }
    let dir = scratch("hls-durations");
    let input = fixtures().join("ts/h264_aac.ts");
    let m3u8 = dir.join("pl.m3u8");
    run_cli(&input, &m3u8, "hls", &["--segment-duration", "2"]);

    let text = std::fs::read_to_string(&m3u8).expect("playlist");
    let extinfs: Vec<f64> = text
        .lines()
        .filter_map(|l| l.strip_prefix("#EXTINF:"))
        .filter_map(|l| l.split(',').next()?.parse::<f64>().ok())
        .collect();
    assert!(extinfs.len() >= 2, "several segments expected:\n{text}");

    // Not all the same value: the segments are keyframe-cut and have different
    // real lengths, so an "evenly divided" placeholder is detectable.
    assert!(
        extinfs.windows(2).any(|w| (w[0] - w[1]).abs() > 1e-3),
        "every #EXTINF was identical ({extinfs:?}) — the durations are not the real ones:\n{text}"
    );

    // TARGETDURATION >= every EXTINF (§4.3.3.1).
    let target: f64 = text
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-TARGETDURATION:"))
        .and_then(|v| v.trim().parse().ok())
        .expect("TARGETDURATION present");
    let max = extinfs.iter().copied().fold(0.0f64, f64::max);
    assert!(
        target >= max - 1e-9,
        "TARGETDURATION {target} < max EXTINF {max}"
    );
    assert_eq!(target, max.ceil(), "TARGETDURATION is ceil(max EXTINF)");

    // The playlist's total equals the container's real duration (ffprobe reads
    // the playlist, so this is measured on the produced files).
    let probed = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(&m3u8)
        .output()
        .expect("ffprobe playlist");
    let real: f64 = String::from_utf8_lossy(&probed.stdout)
        .trim()
        .parse()
        .unwrap();
    let sum: f64 = extinfs.iter().sum();
    assert!(
        (sum - real).abs() < 0.05,
        "playlist total {sum} differs from the real duration {real}"
    );

    // Media-only segments: no ftyp/moov (the #EXT-X-MAP names them), and each
    // starts with styp.
    for i in 0..extinfs.len() {
        let bytes = std::fs::read(dir.join(format!("seg{i}.m4s"))).expect("segment");
        assert_eq!(&bytes[4..8], b"styp", "seg{i} must begin with styp");
        let has = |fourcc: &[u8; 4]| bytes.windows(4).take(4096).any(|w| w == fourcc);
        assert!(
            !has(b"moov") && !has(b"ftyp"),
            "seg{i} must not repeat ftyp/moov — the #EXT-X-MAP init supplies them"
        );
    }
}

/// Item 2, independent validator: Apple's `mediastreamvalidator` must process
/// every segment and report no MUST-fix issue. Skips loudly when absent.
#[test]
fn hls_passes_mediastreamvalidator() {
    let Some(msv) = have_msv() else {
        eprintln!("SKIP cli_binary: mediastreamvalidator not on PATH (macOS-only oracle)");
        return;
    };
    let dir = scratch("hls-msv");
    let input = fixtures().join("ts/h264_aac.ts");
    let m3u8 = dir.join("pl.m3u8");
    run_cli(&input, &m3u8, "hls", &["--segment-duration", "2"]);

    let mut cmd = Command::new(msv);
    cmd.arg("-t")
        .arg(common::MSV_TOOL_TIMEOUT_SECS.to_string())
        .arg(&m3u8);
    let out = common::run_bounded(cmd, common::MSV_DEADLINE, "mediastreamvalidator")
        .expect("spawn validator");
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(
        text.contains("Processed 2 out of 2 segments"),
        "validator did not process both segments (a segment missing its init, or \
         duplicate ftyp/moov):\n{text}"
    );
    assert!(
        !text.contains("MUST fix issues"),
        "validator reports MUST-fix issues:\n{text}"
    );
    assert!(
        !text.contains("Error injecting segment data"),
        "validator could not inject a segment:\n{text}"
    );
}

/// `xmllint --noout` well-formedness, when available. Returns `None` when the
/// tool is missing so the caller can skip loudly.
fn xmllint_ok(path: &std::path::Path) -> Option<bool> {
    let out = Command::new("xmllint")
        .arg("--noout")
        .arg(path)
        .output()
        .ok()?;
    Some(out.status.success())
}

/// Item 4: `--ll` writes a **static** MPD over the static files it produces —
/// never a `dynamic` one whose live edge a player cannot resolve — with no
/// `UTCTiming` unless the caller names a source, and with the LL availability
/// signalling intact.
#[test]
fn ll_dash_mpd_is_static_and_utc_timing_is_opt_in() {
    let dir = scratch("ll-dash");
    let input = fixtures().join("ts/h264_aac.ts");

    // 1. Default: static, VOD duration, no UTCTiming.
    let default_mpd = dir.join("default.mpd");
    run_cli(
        &input,
        &default_mpd,
        "dash",
        &["--ll", "--segment-duration", "2"],
    );
    let text = std::fs::read_to_string(&default_mpd).expect("mpd");

    assert!(
        text.contains("type=\"static\""),
        "a file-based CLI must emit a static MPD:\n{text}"
    );
    assert!(
        !text.contains("type=\"dynamic\""),
        "no dynamic MPD over static files:\n{text}"
    );
    assert!(
        text.contains("mediaPresentationDuration="),
        "a static MPD carries the presentation duration:\n{text}"
    );
    assert!(
        !text.contains("<UTCTiming"),
        "no UTCTiming unless --utc-timing-url is given:\n{text}"
    );
    assert!(
        !text.contains("availabilityStartTime"),
        "a static MPD has no availabilityStartTime (it is live-only):\n{text}"
    );
    // The low-latency signalling is what `--ll` is for; it must remain.
    assert!(
        text.contains("availabilityTimeComplete=\"false\""),
        "LL availability signalling must survive:\n{text}"
    );

    // 2. Opt-in UTCTiming with the caller's URL.
    let utc_mpd = dir.join("utc.mpd");
    run_cli(
        &input,
        &utc_mpd,
        "dash",
        &[
            "--ll",
            "--segment-duration",
            "2",
            "--utc-timing-url",
            "https://time.example/t",
        ],
    );
    let utc = std::fs::read_to_string(&utc_mpd).expect("mpd");
    assert!(
        utc.contains("schemeIdUri=\"urn:mpeg:dash:utc:http-head:2014\""),
        "the registered 2014 scheme:\n{utc}"
    );
    assert!(
        utc.contains("value=\"https://time.example/t\""),
        "the caller's URL, not a hardcoded one:\n{utc}"
    );
    assert!(
        !utc.contains("akamai"),
        "no third-party time server baked into the output:\n{utc}"
    );

    // 3. Independent well-formedness, when xmllint is present.
    match xmllint_ok(&default_mpd) {
        Some(true) => {}
        Some(false) => panic!("xmllint rejects the MPD as malformed:\n{text}"),
        None => eprintln!("SKIP cli_binary: xmllint not on PATH (XML well-formedness check)"),
    }

    // 4. Every `SegmentTemplate@media` file a player would request exists.
    for name in [
        "init-stream1.m4s",
        "chunk-stream1-1.m4s",
        "init-stream2.m4s",
    ] {
        assert!(
            dir.join(name).exists(),
            "the MPD addresses {name}, which was not written"
        );
    }
}

/// Item 4 root cause: this ffmpeg build has no MPEG-DASH demuxer, so `ffprobe`
/// cannot open *any* MPD — ours or its own. Assert that finding rather than
/// letting it read as a defect in our output, and validate with ffprobe when a
/// `dash` demuxer does exist.
#[test]
fn ffprobe_dash_demuxer_availability_is_reported() {
    let demuxers = Command::new("ffprobe")
        .arg("-demuxers")
        .output()
        .expect("spawn ffprobe");
    let listing = String::from_utf8_lossy(&demuxers.stdout);
    let has_dash = listing.lines().any(|l| {
        let mut it = l.split_whitespace();
        matches!(it.next(), Some(t) if t == "D") && matches!(it.nth(1), Some(n) if n == "dash")
    });

    if !has_dash {
        eprintln!(
            "SKIP cli_binary: this ffmpeg build has no `dash` demuxer \
             (`ffprobe -demuxers` lists only webm_dash_manifest), so ffprobe \
             cannot open our MPD — or ffmpeg's own reference MPD in \
             fixtures/dash/. Validating with xmllint + the file-existence checks \
             in ll_dash_mpd_is_static_and_utc_timing_is_opt_in instead."
        );
        // The reference MPD must fail the same way, proving the limitation is
        // the tool's, not our output's.
        let ref_mpd = fixtures().join("dash/manifest.mpd");
        if ref_mpd.exists() {
            let out = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "format=nb_streams",
                    "-of",
                    "csv=p=0",
                ])
                .arg(&ref_mpd)
                .output()
                .expect("ffprobe reference");
            assert!(
                !out.status.success(),
                "ffprobe opened ffmpeg's own reference MPD — the local build does have \
                 a dash demuxer after all, so our MPD must be fixed, not excused"
            );
        }
        return;
    }

    // A dash demuxer exists: our MPD must resolve.
    let dir = scratch("dash-ffprobe");
    let input = fixtures().join("ts/h264_aac.ts");
    let mpd = dir.join("d.mpd");
    run_cli(&input, &mpd, "dash", &["--segment-duration", "2"]);
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=nb_streams",
            "-of",
            "csv=p=0",
        ])
        .arg(&mpd)
        .output()
        .expect("ffprobe mpd");
    assert!(
        out.status.success(),
        "ffprobe rejected our MPD: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let n: u32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    assert_eq!(n, 2, "the MPD must resolve both Representations, got {n}");
}

/// Item 5: the DASH Representations are **aligned** — segment 2 of every
/// AdaptationSet starts at the same instant — and no `SegmentTemplate@duration`
/// is fabricated as zero.
///
/// `segmentAlignment="true"` (ISO/IEC 23009-1 §5.3.7.2) is a promise a client
/// relies on to switch between Representations; segmenting each track on its own
/// gave video 2.0 s and audio 2.02 s boundaries.
#[test]
fn dash_representations_are_aligned_and_never_zero_duration() {
    if !have("ffprobe") || !have("ffmpeg") {
        eprintln!("SKIP cli_binary: ffprobe/ffmpeg not on PATH");
        return;
    }
    let dir = scratch("dash-aligned");
    let input = fixtures().join("ts/h264_aac.ts");
    let mpd = dir.join("d.mpd");
    run_cli(&input, &mpd, "dash", &["--segment-duration", "2"]);
    let text = std::fs::read_to_string(&mpd).expect("mpd");

    assert!(
        text.contains("segmentAlignment=\"true\""),
        "the MPD declares alignment:\n{text}"
    );

    // Every declared duration is non-zero and no larger than the target plus
    // one frame (the sample that crosses the boundary closes the segment).
    let mut scales = Vec::new();
    for line in text.lines().filter(|l| l.contains("SegmentTemplate")) {
        let timescale = attr_u64(line, "timescale").expect("timescale");
        let duration = attr_u64(line, "duration").expect("duration");
        assert!(
            duration > 0,
            "a zero SegmentTemplate@duration is unplayable: {line}"
        );
        assert!(timescale > 0, "timescale must be non-zero: {line}");
        let secs = duration as f64 / timescale as f64;
        assert!(
            secs <= 2.0 + 0.1,
            "declared segment duration {secs} s exceeds the 2 s target: {line}"
        );
        scales.push(timescale);
    }
    assert_eq!(
        scales.len(),
        2,
        "one SegmentTemplate per Representation:\n{text}"
    );

    // The two Representations' declared durations describe the same span
    // (within one audio frame: a track's sample that crosses the boundary
    // closes the segment). Segmenting per track gave 2.0 s against 2.02 s.
    let declared: Vec<f64> = text
        .lines()
        .filter(|l| l.contains("SegmentTemplate"))
        .map(|l| attr_u64(l, "duration").unwrap() as f64 / attr_u64(l, "timescale").unwrap() as f64)
        .collect();
    assert!(
        (declared[0] - declared[1]).abs() <= 0.03,
        "the Representations' first segments span {} s and {} s, more than one \
         audio frame apart",
        declared[0],
        declared[1]
    );

    // Alignment: the tfdt of segment 2 of each Representation, in seconds via
    // that Representation's declared timescale. MP4Box reads the tfdt out of the
    // produced files (a bare media segment has no `moov`).
    match which("MP4Box") {
        Some(mp4box) => {
            let mut starts = Vec::new();
            for (i, id) in ["1", "2"].iter().enumerate() {
                let path = dir.join(format!("chunk-stream{id}-2.m4s"));
                let out = Command::new(mp4box)
                    .arg("-info")
                    .arg(&path)
                    .output()
                    .expect("MP4Box -info");
                let report = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                let tfdt = report
                    .split("TFDT ")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.trim_end_matches('}').parse::<u64>().ok())
                    .expect("MP4Box reports the segment's TFDT");
                starts.push(tfdt as f64 / scales[i] as f64);
            }
            assert_eq!(starts.len(), 2, "two Representations expected");
            assert!(
                (starts[0] - starts[1]).abs() < 1e-9,
                "segment 2 starts at {} s (video) but {} s (audio), not aligned",
                starts[0],
                starts[1]
            );
        }
        None => eprintln!("SKIP cli_binary: MP4Box not on PATH (tfdt alignment check)"),
    }

    // Every Representation's init + segments must decode cleanly, in full.
    for id in ["1", "2"] {
        let mut whole = std::fs::read(dir.join(format!("init-stream{id}.m4s"))).unwrap();
        let mut seg_no = 1;
        loop {
            let p = dir.join(format!("chunk-stream{id}-{seg_no}.m4s"));
            if !p.exists() {
                break;
            }
            whole.extend_from_slice(&std::fs::read(&p).unwrap());
            seg_no += 1;
        }
        let joined = dir.join(format!("joined{id}.mp4"));
        std::fs::write(&joined, &whole).unwrap();
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&joined)
            .args(["-f", "null", "-"])
            .output()
            .expect("ffmpeg decode");
        assert!(
            out.status.success() && out.stderr.is_empty(),
            "Representation {id} does not decode cleanly: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // And the concatenation must carry the whole source for this
        // Representation's stream.
        let counts = packet_counts(&joined);
        let kind = if id == "1" { "video" } else { "audio" };
        let got = counts
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        let want = packet_counts(&input)
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(
            got, want,
            "Representation {id} must carry every {kind} packet"
        );
    }
}

/// The integer value of `key="value"` in `line`.
fn attr_u64(line: &str, key: &str) -> Option<u64> {
    let needle = format!("{key}=\"");
    let rest = line.split(&needle).nth(1)?;
    rest.split('"').next()?.parse().ok()
}

/// Locate a tool by absolute path or on PATH.
fn which(tool: &str) -> Option<&'static str> {
    for p in ["/opt/homebrew/bin/", "/usr/local/bin/", "/usr/bin/"] {
        let full = format!("{p}{tool}");
        if std::path::Path::new(&full).exists() {
            return Some(&*Box::leak(full.into_boxed_str()));
        }
    }
    Command::new(tool)
        .arg("--version")
        .output()
        .ok()
        .map(|_| &*Box::leak(tool.to_string().into_boxed_str()))
}

/// Item 6, independent parser: the emitted MPD is well-formed XML and every
/// `AdaptationSet` carries a unique `@id` (ISO/IEC 23009-1 §5.3.3.1).
///
/// `xmllint` is the independent parser; the test skips loudly without it rather
/// than falling back to a hand-rolled reader.
#[test]
fn dash_mpd_ids_are_unique_and_xml_is_well_formed() {
    let dir = scratch("dash-ids");
    let input = fixtures().join("ts/h264_aac.ts");
    let mpd = dir.join("d.mpd");
    run_cli(&input, &mpd, "dash", &["--segment-duration", "2"]);
    let text = std::fs::read_to_string(&mpd).expect("mpd");

    // Unique AdaptationSet@id, counted from the raw text (the ids are short and
    // unambiguous) — an independent XML reader confirms the document parses.
    let ids: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("<AdaptationSet "))
        .filter_map(|l| attr_str(l, "id"))
        .collect();
    assert_eq!(ids.len(), 2, "two AdaptationSet elements:\n{text}");
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        ids.len(),
        "AdaptationSet@id must be unique: {ids:?}"
    );

    match which("xmllint") {
        Some(xmllint) => {
            let out = Command::new(xmllint).arg("--noout").arg(&mpd).output();
            let Ok(out) = out else {
                eprintln!("SKIP cli_binary: xmllint unavailable");
                return;
            };
            assert!(
                out.status.success(),
                "xmllint rejects the MPD as malformed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        None => eprintln!("SKIP cli_binary: xmllint not on PATH (MPD well-formedness)"),
    }
}

/// The string value of `key="value"` in `line`.
fn attr_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=\"");
    line.split(&needle).nth(1)?.split('"').next()
}

/// The bounded runner must kill a child that outlives its deadline and fail
/// loudly (a `Command::output()` here would block for the child's lifetime).
#[cfg(unix)]
#[test]
#[should_panic(expected = "hard deadline")]
fn bounded_runner_kills_a_hung_tool() {
    const HUNG_SLEEP_SECS: &str = "600";
    let mut c = Command::new("sleep");
    c.arg(HUNG_SLEEP_SECS);
    let _ = common::run_bounded(c, std::time::Duration::from_millis(300), "sleep");
}
