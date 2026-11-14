//! Audit 18, finding 15 (#2699): an HLS sequence number is a
//! `broadcast_hls::MediaSequence`/`broadcast_hls::PartIndex` everywhere, so the
//! playlist, the segmenter and the client never disagree about its width.

use std::fs;
use std::path::Path;

/// The crates whose public API names an HLS sequence number.
const CRATES: [&str; 3] = ["broadcast-hls", "hls-runtime", "transmux"];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    true
}

/// A `pub` field or parameter carrying an HLS sequence number as a bare
/// integer, outside `#[cfg(test)]` code.
fn raw_sequence_uses(source: &str) -> Vec<(usize, String)> {
    let head = match source.find("#[cfg(test)]\nmod ") {
        Some(i) => &source[..i],
        None => source,
    };
    let mut hits = Vec::new();
    for (n, line) in head.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        for name in ["msn", "last_msn", "part_index"] {
            let public = t.starts_with("pub ");
            let names_field =
                t.starts_with(&format!("pub {name}: ")) || t.starts_with(&format!("pub {name} : "));
            let field = t.split_once(',').map_or(t, |(field, _)| field);
            let bare = matches!(
                field.rsplit_once(':').map(|(_, ty)| ty.trim()),
                Some("u64" | "u32" | "Option<u64>" | "Option<u32>")
            );
            if bare && public && names_field && !t.ends_with(')') {
                hits.push((n + 1, t.to_string()));
            }
        }
    }
    hits
}

#[test]
fn an_hls_sequence_number_is_never_a_bare_integer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    for krate in CRATES {
        let mut files = Vec::new();
        if !rust_files(&root.join(krate).join("src"), &mut files) {
            continue;
        }
        for path in files {
            scanned += 1;
            let text = fs::read_to_string(&path).expect("read source file");
            for (line_no, line) in raw_sequence_uses(&text) {
                hits.push(format!("{}:{line_no}: {line}", path.display()));
            }
        }
    }
    assert!(scanned > 0, "the scan found no sources to read");
    assert!(
        hits.is_empty(),
        "an HLS sequence number is a bare integer (use broadcast_hls::\
         MediaSequence/PartIndex):\n{}",
        hits.join("\n")
    );
}

/// The guard bites: the shape it bans is reported, the typed shape is not.
#[test]
fn the_guard_reports_a_bare_integer_sequence_and_accepts_the_typed_one() {
    let raw = "pub struct Part {\n    pub msn: u64,\n    pub msn: Option<u32>,\n}\n";
    assert_eq!(raw_sequence_uses(raw).len(), 2);
    let parameter = "pub fn follow_part(msn: u32) {}\n";
    assert!(
        raw_sequence_uses(parameter).is_empty(),
        "a private parameter is internal state, not the surface"
    );
    let typed = "pub struct Part {\n    pub msn: MediaSequence,\n}\nfn follow_part(msn: MediaSequence) {}\n";
    assert!(raw_sequence_uses(typed).is_empty());
    let doc = "/// not a field: pub msn: u64 in prose\n";
    assert!(raw_sequence_uses(doc).is_empty());
}
