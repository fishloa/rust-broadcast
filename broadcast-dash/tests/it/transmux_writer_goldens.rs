use broadcast_dash::Mpd;

/// Goldens that cannot be reproduced from the parsed model, with the reason.
/// These must actually differ; all others must reproduce byte-identically.
const DIVERGENT: &[(&str, &str)] = &[
    (
        "dash-mpd-frac-main.golden",
        "transmux floating-point duration formatting defect (PT0.30000000000000004S)",
    ),
    (
        "dash-mpd-window3600-main.golden",
        "transmux writes PT3600S but canonical form is PT1H",
    ),
    (
        "dash-mpd-window60-main.golden",
        "transmux writes PT60S but canonical form is PT1M",
    ),
];

#[test]
fn transmux_writer_goldens_reproduce_byte_identical() {
    let root = test_fixtures::root();
    let dir = root.join("golden/dash");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            (name.ends_with(".mpd") || name.ends_with(".golden")) && name != "mpd-parse.golden"
        })
        .collect();
    files.sort();
    assert!(
        files.len() >= 20,
        "expected at least 20 golden files, found {}",
        files.len()
    );

    let mut checked = 0usize;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let original = std::fs::read_to_string(path).unwrap();
        let is_divergent = DIVERGENT.iter().any(|(n, _)| *n == name);
        let mpd = match Mpd::parse(original.as_bytes()) {
            Ok(m) => m,
            Err(_) => {
                if is_divergent {
                    checked += 1;
                    continue;
                }
                panic!("{name}: failed to parse");
            }
        };
        let rendered = mpd.to_xml().unwrap();
        if is_divergent {
            assert_ne!(
                rendered, original,
                "{name}: expected to diverge but now reproduces; remove from DIVERGENT"
            );
        } else {
            if rendered != original {
                let a: Vec<&str> = rendered.lines().collect();
                let b: Vec<&str> = original.lines().collect();
                let n = a.len().max(b.len());
                for i in 0..n {
                    let la = a.get(i).copied().unwrap_or("<missing>");
                    let lb = b.get(i).copied().unwrap_or("<missing>");
                    if la != lb {
                        panic!(
                            "{name}: first diff at line {}\n  got:      {}\n  expected: {}",
                            i + 1,
                            la,
                            lb
                        );
                    }
                }
                panic!("{name}: trailing newline differs");
            }
        }
        checked += 1;
    }
    assert!(checked >= 20);
}
