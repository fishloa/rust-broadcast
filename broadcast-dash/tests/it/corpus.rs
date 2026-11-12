use std::path::{Path, PathBuf};

pub const PUBLIC_PREFIX: &str = "fixtures/";
pub const PRIVATE_PREFIX: &str = "private/";
pub const ISO_EXAMPLES: &str = "specs/iso_iec_23009-1_dash_2026_ed6_schema";

pub fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "mpd") {
            out.push(path);
        }
    }
}

pub fn corpus() -> Vec<(String, PathBuf)> {
    let public_root = test_fixtures::root();
    let mut public = Vec::new();
    walk(&public_root, &mut public);
    let mut out: Vec<(String, PathBuf)> = public
        .into_iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(&public_root)
                .ok()?
                .to_string_lossy()
                .into_owned();
            (!rel.starts_with("golden/")).then(|| (format!("{PUBLIC_PREFIX}{rel}"), p))
        })
        .collect();
    if let Some(private_root) = test_fixtures::private_root() {
        let mut private = test_fixtures::private_files("", &["mpd"]);
        let examples = private_root.join(ISO_EXAMPLES);
        assert!(
            examples.is_dir(),
            "private checkout present but {} is missing",
            examples.display()
        );
        walk(&examples, &mut private);
        for p in private {
            let rel = p
                .strip_prefix(&private_root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            out.push((format!("{PRIVATE_PREFIX}{rel}"), p));
        }
    }
    out.sort();
    out
}
