//! Apple `mediastreamvalidator` as an independent HLS conformance oracle
//! (issue #870, part of the #866 epic).
//!
//! `media-doctor`'s own `check_hls_playlist`/`check_playlist` encode *our*
//! reading of RFC 8216/8216bis. Validating our own renderer with our own
//! checker is circular — a shared misreading of the spec passes both sides
//! (this is exactly how a hard-coded `EXT-X-VERSION = 9` over-declaration
//! survived undetected). `mediastreamvalidator` 1.25.36 is Apple's own HLS
//! conformance tool — the reference client's own validator — and is wired in
//! here as a genuinely independent second opinion.
//!
//! ## Tool shape (reverse-engineered during calibration, `-h`/`--help` does
//! not document the JSON schema)
//!
//! `mediastreamvalidator [--parse-playlist-only] --quiet -t <secs> -O
//! <path.json> <entry.m3u8>`, run with `current_dir` set to the directory
//! holding the playlist (and any real files it references), so a **relative
//! entry filename** resolves relative URIs exactly the way a real HTTP
//! origin's client would resolve them against the request URL.
//!
//! - **Exit code is always 0** — even on `parseFailed: true`. The JSON at
//!   `-O`'s path is the only authoritative signal; this harness always reads
//!   it, never the process exit status.
//! - **`--parse-playlist-only` (`-p`) does not fetch segment bytes at all**,
//!   for a Media Playlist *or* one recursively reached through a
//!   Multivariant Playlist's variants — it is purely playlist-*text*
//!   parsing, including of the whole referenced Playlist tree. It does
//!   **not** catch rules that need the segment's actual content or even just
//!   its byte length (e.g. `EXTINF` vs `#EXT-X-TARGETDURATION`, checked
//!   in `malformed_playlist_is_rejected` below only under a full run).
//! - **Every finding is a `"messages"` array**, either at the JSON root (a
//!   Multivariant-Playlist-level or Content-Steering-level problem) or
//!   nested under a `variants[i]`/`discontinuities[i]/segments[j]`/
//!   `validations[i]` entry. Each message carries `errorComment`,
//!   `errorDomain`, `errorRequirementLevel`, and (usually) `errorDetail`.
//! - **`errorRequirementLevel`**: empirically, `1` marks RFC "MUST"/
//!   "REQUIRED" language (a real conformance violation); `2` marks
//!   "SHOULD"/"RECOMMENDED" (a recommendation, not gated here — see
//!   [`Finding::is_error`]). A hard parse failure (missing `#EXTM3U`) instead
//!   surfaces as top-level `"parseFailed": true` plus a `CoreMediaErrorDomain`
//!   message — also treated as an error regardless of its numeric level.
//! - **A live Media Playlist (no `#EXT-X-ENDLIST`) blocks** waiting for a
//!   reload up to the default 300s timeout, even under `-p` — always pass an
//!   explicit low `-t`.
//!
//! ## Calibration (`spec_fixtures_calibrate`)
//!
//! Run first, against `fixtures/hls/spec/` (the RFC's own §9 example
//! playlists, committed for issue #869): if the validator rejected one of
//! these, the invocation above would be wrong, not the fixture. It doesn't —
//! every MUST-level finding across all 8 parseable spec fixtures is one of a
//! small number of *documented, explained* standalone-validation artifacts
//! (see [`is_known_standalone_artifact`]), never something new. `9.10`/`9.11`
//! are excluded outright (same reason `hls_fixture_corpus.rs` in `transmux`
//! excludes them): both embed the RFC's own bare `...` elision line, which
//! isn't `#`-prefixed and is read as a Media Segment URI with no preceding
//! `#EXTINF` — a permanent, genuine parse error, not a fixture defect.
//!
//! ## Local invocation (documented in the workspace `CLAUDE.md` command list)
//!
//! ```text
//! cargo test -p media-doctor --test mediastreamvalidator_oracle --all-features --locked
//! ```
//!
//! Requires macOS with `mediastreamvalidator` on `PATH` (Apple's Additional
//! Tools for Xcode, `/usr/local/bin/mediastreamvalidator`); every test in
//! this file skips itself — loudly, via `eprintln!` naming the reason — when
//! the binary is absent, so `cargo test` stays green on Linux/CI. Run with
//! `--nocapture` to see the skip line (and, on `spec_fixtures_calibrate`, the
//! full per-fixture finding dump) without a failure forcing it out.

#![cfg(feature = "serde")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use broadcast_hls::{
    CencScheme, IFrameVariant, LowLatencyConfig, MasterPlaylist, MediaPlaylist, MediaSegment,
    PartSpec, Variant, cenc_ext_x_key,
};
use serde_json::Value;
use transmux::cli::{Opts, Output, OutputFormat, run_bytes};

// ── Fixture + scratch-dir plumbing (mirrors transmux/tests/golden_gate.rs) ──

fn repo_fixture(rel: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join(rel)
}

fn ts_fixture_bytes() -> Vec<u8> {
    fs::read(repo_fixture("fixtures/ts/h264_aac.ts")).expect("h264_aac.ts fixture must exist")
}

/// A fresh, empty scratch directory under the workspace `target/` (already
/// gitignored) for one test case's rendered tree.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/mediastreamvalidator-tmp")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create mediastreamvalidator scratch dir");
    dir
}

// ── External-tool availability gate ─────────────────────────────────────────

fn validator_available() -> bool {
    Command::new("mediastreamvalidator")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Skip this test cleanly, but LOUDLY: a silently-skipped oracle reads as a
/// pass, which is worse than no oracle at all (issue #870).
macro_rules! skip_unless_validator_available {
    () => {
        if !validator_available() {
            eprintln!(
                "SKIP mediastreamvalidator_oracle: `mediastreamvalidator` not on PATH \
                 (macOS-only Apple HLS conformance tool; see CLAUDE.md's command list). \
                 This test is a no-op result on this host, not real coverage — run it on \
                 macOS to get the genuine check."
            );
            return;
        }
    };
}

// ── Running the tool + classifying its JSON output ──────────────────────────

/// One `"messages"` entry from the validator's JSON, plus the JSON-pointer-ish
/// path it was nested at (for readable failure output).
#[derive(Debug, Clone)]
struct Finding {
    path: String,
    comment: String,
    detail: Option<String>,
    level: i64,
    domain: String,
}

impl Finding {
    /// Gate on this finding: `errorRequirementLevel <= 1` is RFC "MUST"/
    /// "REQUIRED" language — a real conformance error. `>= 2` is SHOULD/MAY
    /// language (e.g. `CoreMediaErrorDomain`'s own "Unrecognized attribute in
    /// #EXT-X-KEY", empirically level 2) — a recommendation, not gated. A
    /// hard parse failure (top-level `"parseFailed": true`) is folded into
    /// `level` by [`collect_findings`] (forced to `1`) rather than checked
    /// here via `errorDomain`, because `CoreMediaErrorDomain` alone is NOT a
    /// reliable severity signal — it also carries non-fatal level-2 findings.
    fn is_error(&self) -> bool {
        self.level <= 1
    }
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] level={} {}: {}",
            self.path, self.level, self.domain, self.comment
        )?;
        if let Some(d) = &self.detail {
            write!(f, " ({d})")?;
        }
        Ok(())
    }
}

/// Run `mediastreamvalidator` over `entry` (a playlist filename inside
/// `dir`), returning every finding anywhere in its JSON output.
///
/// `parse_only` selects `--parse-playlist-only` (pure text-parse, no segment
/// or sub-manifest byte fetch — see module docs) vs a full validation run
/// (segment files are opened and decoded). `timeout_secs` is passed as `-t`;
/// always pass a low explicit value (a live/no-`#EXT-X-ENDLIST`
/// Media Playlist otherwise blocks for the 300s default waiting for a reload
/// that a static fixture file will never produce).
fn run_validator(dir: &Path, entry: &str, parse_only: bool, timeout_secs: u32) -> Vec<Finding> {
    // The JSON output path (`-O`) is deliberately OUTSIDE `dir`: for the
    // calibration test `dir` is the committed `fixtures/hls/spec/` tree, and
    // writing scratch output into a source directory would dirty a fixture
    // tree the test run itself doesn't own. Unique per call (a monotonic
    // counter) since `cargo test` runs test functions concurrently by
    // default and this file's own tests share one process.
    static CALL_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call_id = CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out_json = std::env::temp_dir().join(format!(
        "mediastreamvalidator-oracle-{}-{call_id}.json",
        std::process::id()
    ));

    let mut cmd = Command::new("mediastreamvalidator");
    cmd.current_dir(dir);
    if parse_only {
        cmd.arg("--parse-playlist-only");
    }
    cmd.arg("--quiet")
        .arg("-t")
        .arg(timeout_secs.to_string())
        .arg("-O")
        .arg(&out_json)
        .arg(entry);
    let out = cmd.output().expect("spawn mediastreamvalidator");

    let text = fs::read_to_string(&out_json).unwrap_or_else(|e| {
        panic!(
            "mediastreamvalidator produced no JSON at {} ({e}); \
             stdout={} stderr={}",
            out_json.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    });
    let json: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "parse mediastreamvalidator JSON at {}: {e}",
            out_json.display()
        )
    });
    let _ = fs::remove_file(&out_json);

    let mut findings = Vec::new();
    collect_findings(&json, "$", &mut findings);
    findings
}

/// Recursively walk the JSON tree collecting every `"messages"` array,
/// wherever it is nested (root, `variants[i]`, `discontinuities[i]/
/// segments[j]`, `validations[i]`, `validations[i]/steeringManifests[k]`,
/// ...) — the schema nests findings at whichever level they apply to, and is
/// not documented anywhere, so this walks generically rather than hardcoding
/// one path per finding kind.
fn collect_findings(value: &Value, path: &str, out: &mut Vec<Finding>) {
    match value {
        Value::Object(map) => {
            // A hard parse failure (missing `#EXTM3U`, calibrated in
            // `missing_extm3u_header_is_rejected`) marks the *object*
            // `"parseFailed": true`, not each message individually — fold it
            // into every sibling message's effective level here so
            // `Finding::is_error` can stay a plain `level <= 1` check.
            let parse_failed = map
                .get("parseFailed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(Value::Array(messages)) = map.get("messages") {
                for m in messages {
                    let level = m
                        .get("errorRequirementLevel")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    out.push(Finding {
                        path: path.to_string(),
                        comment: m
                            .get("errorComment")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        detail: m
                            .get("errorDetail")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        level: if parse_failed { 1 } else { level },
                        domain: m
                            .get("errorDomain")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    });
                }
            }
            for (k, v) in map {
                if k == "messages" {
                    continue;
                }
                collect_findings(v, &format!("{path}/{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect_findings(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn errors(findings: &[Finding]) -> Vec<&Finding> {
    findings.iter().filter(|f| f.is_error()).collect()
}

/// The gate: fail loudly, listing every error-level finding not excused by
/// `ignore`, if any exist.
fn assert_zero_errors_ignoring(
    findings: &[Finding],
    context: &str,
    ignore: impl Fn(&Finding) -> bool,
) {
    let errs: Vec<&Finding> = errors(findings)
        .into_iter()
        .filter(|f| !ignore(f))
        .collect();
    assert!(
        errs.is_empty(),
        "{context}: mediastreamvalidator reported {} error-level finding(s):\n{}",
        errs.len(),
        errs.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// The gate: fail loudly, listing every error-level finding, if any exist.
fn assert_zero_errors(findings: &[Finding], context: &str) {
    assert_zero_errors_ignoring(findings, context, |_| false);
}

// ── Calibration: the spec's own example playlists must validate clean ──────

/// Findings this harness treats as **explained artifacts of validating a
/// single spec-example file standalone**, not real conformance defects.
/// Every one of these is discussed in the module doc / `fixtures/hls/
/// MANIFEST.md`; anything NOT matched here fails `spec_fixtures_calibrate`
/// loudly, on the assumption that an unexplained MUST-level finding against
/// an RFC-authored example means the *invocation* is wrong.
fn is_known_standalone_artifact(f: &Finding) -> bool {
    match f.comment.as_str() {
        // 9.4/9.5/9.6/9.7/9.12 (Multivariant Playlists) reference sibling
        // Media Playlists that were never committed — issue #869's fixture
        // set is deliberately playlist-text-only (see MANIFEST.md). Even
        // under `--parse-playlist-only`, the validator still tries to open
        // every referenced Playlist (it walks the whole Playlist *tree*, not
        // just the entry file) and reports this when one doesn't exist. A
        // multivariant playlist a caller wants fully validated has to supply
        // real child playlists — exactly what `origin_multivariant_shape_
        // validates_clean` below does for our OWN renders.
        "Playlist does not exist at the specified path." => true,
        // 9.12's `EXT-X-CONTENT-STEERING` `SERVER-URI` is itself an
        // illustrative path (`/steering?video=00012`) with no backing file
        // in this fixture set, so it resolves to nothing.
        "Zero length steering manifest" | "Illegal MIME type" => true,
        // 9.6/9.7 use `CODECS="..."` — the RFC's own editorial elision
        // marker (the same artifact `MANIFEST.md` documents for the bare
        // `...` lines in 9.10/9.11), reused here *inline* as a placeholder
        // attribute value rather than a real RFC 6381 codec string.
        "Unrecognized codec" if f.detail.as_deref() == Some("Codec: ...") => true,
        // 9.12 deliberately defines Pathway `CDN-A`'s audio group "A"
        // (`NAME="English"`) and Pathway `CDN-B`'s audio group "B"
        // (`NAME="ENGLISH"`) with different members — that per-Pathway
        // difference is the entire point of Content Steering.
        // mediastreamvalidator's classic same-`TYPE`-group-consistency rule
        // predates Content Steering and doesn't know pathway scoping, so the
        // RFC's own example trips it.
        "Each Group of the same TYPE MUST have the same set of members" => true,
        _ => false,
    }
}

/// The 8 committed `fixtures/hls/spec/*.m3u8` files that actually parse (see
/// `fixtures/hls/MANIFEST.md`'s Tier 1 table). `9.10-daterange-scte35.m3u8`
/// and `9.11-low-latency-playlist.m3u8` are excluded on purpose: both embed
/// the RFC's own bare `...` elision line mid-playlist, which is not
/// `#`-prefixed and so parses as a Media Segment URI with no preceding
/// `#EXTINF` — a genuine, permanent parse error, not a fixture defect (the
/// same reasoning `transmux/tests/hls_fixture_corpus.rs` already documents
/// for excluding them from ITS parse-completeness assertions).
const SPEC_FIXTURES: &[&str] = &[
    "9.1-simple-media-playlist.m3u8",
    "9.2-live-media-playlist-https.m3u8",
    "9.3-encrypted-media-segments.m3u8",
    "9.4-multivariant-playlist.m3u8",
    "9.5-multivariant-with-iframes.m3u8",
    "9.6-multivariant-alternative-audio.m3u8",
    "9.7-multivariant-alternative-video.m3u8",
    "9.12-content-steering.m3u8",
    // 9.10-daterange-scte35.m3u8, 9.11-low-latency-playlist.m3u8: excluded,
    // see doc comment above.
];

#[test]
fn spec_fixtures_calibrate() {
    skip_unless_validator_available!();

    let spec_dir = repo_fixture("fixtures/hls/spec");

    for name in SPEC_FIXTURES {
        // `-t 5`: 9.2 is a "live" playlist (no `#EXT-X-ENDLIST`) per the RFC
        // example; without an explicit low timeout the validator blocks for
        // its 300s default waiting for a reload of a file that will never
        // change (an issue #870 calibration finding — see module docs).
        let findings = run_validator(&spec_dir, name, true, 5);

        eprintln!("=== {name} ===");
        if findings.is_empty() {
            eprintln!("  (clean)");
        }
        for f in &findings {
            eprintln!("  {f}");
        }

        let unexplained: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.is_error() && !is_known_standalone_artifact(f))
            .collect();
        assert!(
            unexplained.is_empty(),
            "{name}: unexplained mediastreamvalidator error(s) against an \
             RFC-authored example — the invocation is probably wrong, not \
             the fixture:\n{}",
            unexplained
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
}

// ── Negative proof: the harness must actually bite ──────────────────────────

/// `bad-extinf.m3u8` declares `EXTINF:15.0` against `TARGETDURATION:10` — RFC
/// 8216 §4.3.3.1 MUST. Calibration finding: `--parse-playlist-only` does
/// NOT evaluate this rule (it needs the segment to be processed); a full
/// validation run does. We supply the workspace's own real committed MPEG-TS
/// capture as the segment bytes so the validator has something genuinely
/// decodable to open — its content is irrelevant to this particular rule,
/// but a garbage/empty file would trip an unrelated "can't decode segment"
/// finding and muddy the proof.
#[test]
fn malformed_playlist_is_rejected() {
    skip_unless_validator_available!();

    let dir = scratch_dir("malformed-bad-extinf");
    fs::copy(
        repo_fixture("fixtures/hls/bad-extinf.m3u8"),
        dir.join("playlist.m3u8"),
    )
    .expect("copy bad-extinf.m3u8");
    fs::write(dir.join("seg0.ts"), ts_fixture_bytes()).expect("write seg0.ts");

    let findings = run_validator(&dir, "playlist.m3u8", false, 20);
    let errs = errors(&findings);
    assert!(
        !errs.is_empty(),
        "harness must FAIL on a deliberately malformed playlist (bad-extinf.m3u8) \
         — got zero error-level findings, which means this harness cannot be \
         trusted to catch a real regression"
    );
    assert!(
        errs.iter().any(|f| f
            .comment
            .contains("MUST be less than or equal to the target duration")),
        "expected the EXTINF > TARGETDURATION MUST-violation specifically, got:\n{}",
        errs.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// `missing-extm3u.m3u8` has no `#EXTM3U` header at all — a hard syntax
/// error the validator must reject even in `--parse-playlist-only` mode
/// (pure text parsing catches this; no segment needs to exist).
#[test]
fn missing_extm3u_header_is_rejected() {
    skip_unless_validator_available!();

    let dir = scratch_dir("malformed-missing-extm3u");
    fs::copy(
        repo_fixture("fixtures/hls/missing-extm3u.m3u8"),
        dir.join("playlist.m3u8"),
    )
    .expect("copy missing-extm3u.m3u8");

    let findings = run_validator(&dir, "playlist.m3u8", true, 5);
    let errs = errors(&findings);
    assert!(
        !errs.is_empty(),
        "harness must FAIL on a playlist missing #EXTM3U — got zero error-level findings"
    );
    assert!(
        errs.iter().any(|f| f.comment.contains("EXTM3U")),
        "expected a missing-#EXTM3U finding specifically, got:\n{}",
        errs.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

// ── Positive proof: every shape the origin can render passes clean ─────────

/// Classic TS-HLS (MPEG-2 TS media segments + `.m3u8`), rendered through the
/// SAME `transmux::cli::run_bytes` code path the shipped `transmux` binary
/// uses, from the workspace's real committed H.264/AAC TS capture — not a
/// hand-typed playlist. Full (non-`-p`) validation: real segment bytes are
/// opened and decoded, so this exercises both playlist-syntax rules AND
/// segment-content rules (the ones `--parse-playlist-only` cannot reach —
/// see `malformed_playlist_is_rejected` above).
#[test]
fn origin_ts_hls_output_validates_clean() {
    skip_unless_validator_available!();

    let ts = ts_fixture_bytes();
    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::TsHls,
            segment_duration: 1,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> TS-HLS");
    let (playlist, segments) = match out {
        Output::Manifest { text, segments } => (text, segments),
        _ => panic!("TS-HLS must produce a manifest + segments"),
    };

    let dir = scratch_dir("origin-ts-hls");
    fs::write(dir.join("out.m3u8"), &playlist).expect("write playlist");
    for (name, bytes) in &segments {
        fs::write(dir.join(name), bytes).expect("write segment");
    }

    let findings = run_validator(&dir, "out.m3u8", false, 30);
    assert_zero_errors(&findings, "origin TS-HLS output");
}

/// CMAF-HLS (fMP4 init + media segments + `.m3u8`), same real-capture /
/// same-code-path approach as the TS-HLS case above.
#[test]
fn origin_cmaf_hls_output_validates_clean() {
    skip_unless_validator_available!();

    let ts = ts_fixture_bytes();
    let out = run_bytes(
        &ts,
        &Opts {
            format: OutputFormat::Hls,
            segment_duration: 1,
            ..Opts::default()
        },
    )
    .expect("run_bytes TS -> CMAF-HLS");
    let (playlist, segments) = match out {
        Output::Manifest { text, segments } => (text, segments),
        _ => panic!("CMAF-HLS must produce a manifest + segments"),
    };

    let dir = scratch_dir("origin-cmaf-hls");
    fs::write(dir.join("out.m3u8"), &playlist).expect("write playlist");
    for (name, bytes) in &segments {
        fs::write(dir.join(name), bytes).expect("write segment");
    }

    let findings = run_validator(&dir, "out.m3u8", false, 30);
    assert_zero_errors(&findings, "origin CMAF-HLS output");
}

/// A Multivariant Playlist shape (`broadcast_hls::MasterPlaylist`) with real
/// child Media Playlists written alongside it, so the validator's recursive
/// Playlist-tree walk resolves cleanly (unlike the standalone spec fixtures
/// above, which reference files that were never meant to exist). Structural
/// (`--parse-playlist-only`) coverage only: generating byte-valid multi-
/// rendition CMAF content for every leaf here would need real per-rendition
/// encodes, which is out of scope for a playlist-shape check — the segment-
/// content rules are already exercised end-to-end by the two `origin_*`
/// tests above.
#[test]
fn origin_multivariant_shape_validates_clean() {
    skip_unless_validator_available!();

    let dir = scratch_dir("origin-multivariant");

    // Two variants, each pointing at a real (if minimal) child Media
    // Playlist — matching the shape `transmux`'s TS-HLS/CMAF-HLS packagers
    // never emit themselves (they only ever produce ONE Media Playlist),
    // but `broadcast-hls` callers (e.g. an ABR origin) can build directly.
    let child = MediaPlaylist {
        version: 3,
        target_duration: 6,
        segments: vec![MediaSegment {
            uri: "seg0.ts".into(),
            duration: 6.0,
            ..Default::default()
        }],
        endlist: true,
        ..Default::default()
    };
    fs::write(dir.join("low.m3u8"), child.to_m3u8()).expect("write low.m3u8");
    fs::write(dir.join("hi.m3u8"), child.to_m3u8()).expect("write hi.m3u8");

    let master = MasterPlaylist {
        variants: vec![
            Variant {
                bandwidth: 1_280_000,
                codecs: "avc1.64001f,mp4a.40.2".into(),
                resolution: Some((640, 360)),
                uri: "low.m3u8".into(),
                extra_attrs: vec![],
            },
            Variant {
                bandwidth: 2_560_000,
                codecs: "avc1.64001f,mp4a.40.2".into(),
                resolution: Some((1280, 720)),
                uri: "hi.m3u8".into(),
                extra_attrs: vec![],
            },
        ],
        ..Default::default()
    };
    fs::write(dir.join("master.m3u8"), master.to_m3u8()).expect("write master.m3u8");

    let findings = run_validator(&dir, "master.m3u8", true, 5);
    assert_zero_errors(&findings, "origin multivariant shape");
}

/// I-frame-only trick-play rendition entries (`#EXT-X-I-FRAME-STREAM-INF`) in
/// a Multivariant Playlist. Structural coverage only (see the multivariant
/// case above for why).
#[test]
fn origin_iframe_variant_shape_validates_clean() {
    skip_unless_validator_available!();

    let dir = scratch_dir("origin-iframe");

    let iframe_playlist = MediaPlaylist {
        version: 4,
        target_duration: 6,
        iframes_only: true,
        segments: vec![MediaSegment {
            uri: "iframe0.ts".into(),
            duration: 6.0,
            byte_range: Some(broadcast_hls::ByteRange {
                length: 4096,
                offset: Some(0),
            }),
            ..Default::default()
        }],
        endlist: true,
        ..Default::default()
    };
    fs::write(dir.join("iframe.m3u8"), iframe_playlist.to_m3u8()).expect("write iframe.m3u8");

    let master = MasterPlaylist {
        variants: vec![Variant {
            bandwidth: 2_560_000,
            codecs: "avc1.64001f,mp4a.40.2".into(),
            resolution: Some((1280, 720)),
            uri: "hi.m3u8".into(),
            extra_attrs: vec![],
        }],
        iframe_variants: vec![IFrameVariant {
            bandwidth: 150_000,
            codecs: Some("avc1.64001f".into()),
            resolution: Some((1280, 720)),
            uri: "iframe.m3u8".into(),
            extra_attrs: vec![],
        }],
        ..Default::default()
    };
    let full_pl = MediaPlaylist {
        version: 3,
        target_duration: 6,
        segments: vec![MediaSegment {
            uri: "seg0.ts".into(),
            duration: 6.0,
            ..Default::default()
        }],
        endlist: true,
        ..Default::default()
    };
    fs::write(dir.join("hi.m3u8"), full_pl.to_m3u8()).expect("write hi.m3u8");
    fs::write(dir.join("master.m3u8"), master.to_m3u8()).expect("write master.m3u8");

    let findings = run_validator(&dir, "master.m3u8", true, 5);
    assert_zero_errors(&findings, "origin I-frame variant shape");
}

/// Low-Latency HLS (parts + `#EXT-X-SERVER-CONTROL`/`#EXT-X-PART-INF`) on a
/// VOD (ended) Media Playlist, so the validator doesn't block waiting for a
/// live reload. Structural coverage only (see the multivariant case above).
#[test]
fn origin_low_latency_shape_validates_clean() {
    skip_unless_validator_available!();

    let dir = scratch_dir("origin-ll-hls");

    let playlist = MediaPlaylist {
        version: 9,
        target_duration: 4,
        segments: vec![MediaSegment {
            uri: "seg0.ts".into(),
            duration: 4.0,
            parts: vec![
                PartSpec {
                    uri: "seg0.0.m4s".into(),
                    duration: 1.0,
                    independent: true,
                    ..Default::default()
                },
                PartSpec {
                    uri: "seg0.1.m4s".into(),
                    duration: 1.0,
                    ..Default::default()
                },
                PartSpec {
                    uri: "seg0.2.m4s".into(),
                    duration: 1.0,
                    ..Default::default()
                },
                PartSpec {
                    uri: "seg0.3.m4s".into(),
                    duration: 1.0,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        endlist: true,
        low_latency: Some(LowLatencyConfig {
            part_target: 1.0,
            part_hold_back: 3.0,
            can_block_reload: true,
            ..LowLatencyConfig::default()
        }),
        ..Default::default()
    };
    fs::write(dir.join("out.m3u8"), playlist.to_m3u8()).expect("write out.m3u8");

    let findings = run_validator(&dir, "out.m3u8", true, 5);
    assert_zero_errors_ignoring(&findings, "origin LL-HLS shape", |f| {
        // Known mediastreamvalidator limitation, not a renderer defect:
        // verified (issue #870 calibration) that this fires unconditionally
        // whenever LL-HLS directives (`EXT-X-PART-INF`/`EXT-X-PART`) are
        // present, even standalone AND inside a real single-variant
        // Multivariant Playlist with no sibling Rendition at all. RFC
        // 8216bis's Low-Latency profile appendix explicitly permits zero
        // `EXT-X-RENDITION-REPORT` tags in that case: "...for each Media
        // Playlist (Rendition) in the Multivariant Playlist, EXCEPT for the
        // Media Playlist to which the EXT-X-RENDITION-REPORT tag is being
        // added" — with no OTHER Rendition, there is nothing left to except
        // from zero.
        f.comment == "No rendition reports in low-latency playlist"
    });
}

/// CENC/`cbcs`-encrypted CMAF (`#EXT-X-KEY` with the CENC `KEYFORMAT`).
/// Structural coverage only (see the multivariant case above).
#[test]
fn origin_encrypted_shape_validates_clean() {
    skip_unless_validator_available!();

    let dir = scratch_dir("origin-encrypted");

    let key_tag = cenc_ext_x_key(CencScheme::Cbcs, &[0xab; 16], "https://key.example/k")
        .expect("cbcs always yields an EXT-X-KEY tag");

    let playlist = MediaPlaylist {
        version: 6,
        target_duration: 6,
        extra_tags: vec![key_tag],
        segments: vec![MediaSegment {
            uri: "seg0.m4s".into(),
            duration: 6.0,
            ..Default::default()
        }],
        endlist: true,
        ..Default::default()
    };
    fs::write(dir.join("out.m3u8"), playlist.to_m3u8()).expect("write out.m3u8");

    let findings = run_validator(&dir, "out.m3u8", true, 5);
    assert_zero_errors(&findings, "origin encrypted (cbcs) shape");
}
