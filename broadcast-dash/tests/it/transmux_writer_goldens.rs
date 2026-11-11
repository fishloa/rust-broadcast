use broadcast_dash::Mpd;

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
            (name.ends_with(".mpd") || name.ends_with(".golden"))
                && name != "mpd-parse.golden"
                && name != "dash-mpd-frac-main.golden"
                && name != "dash-mpd-window3600-main.golden"
                && name != "dash-mpd-window60-main.golden"
                && name != "ll-dash-mpd.golden"
                && name != "lldash-plain.mpd"
                && name != "lldash-rate.mpd"
                && name != "lldash-timeline.mpd"
                && name != "lldash-utc.mpd"
        })
        .collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let original = std::fs::read_to_string(&path).unwrap();
        let mpd = Mpd::parse(original.as_bytes()).unwrap_or_else(|e| panic!("{name}: parse {e}"));
        let rendered = mpd.to_xml().unwrap_or_else(|e| panic!("{name}: write {e}"));
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
}
