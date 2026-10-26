//! Lexical source scanning shared by the guard tests (a tripwire, not a proof).

use std::fs;
use std::path::Path;

pub fn rs_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).expect("read source"),
            ));
        }
    }
}

pub fn src_files() -> Vec<(String, String)> {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files
}

pub fn non_test_source(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[cfg(test)]")
            && let Some(m) = next_item_is_mod(&lines, i)
        {
            i = if lines[m].trim_end().ends_with(';') {
                m + 1
            } else {
                block_end(&lines, m) + 1
            };
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

fn next_item_is_mod(lines: &[&str], attr: usize) -> Option<usize> {
    let mut j = attr + 1;
    while j < lines.len() && lines[j].trim_start().starts_with("#[") {
        j += 1;
    }
    let t = lines.get(j)?.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t);
    t.starts_with("mod ").then_some(j)
}

fn block_end(lines: &[&str], from: usize) -> usize {
    let (mut depth, mut seen_open) = (0i64, false);
    for (k, line) in lines.iter().enumerate().skip(from) {
        let (d, opened) = brace_delta(line);
        depth += d;
        seen_open |= opened;
        if seen_open && depth <= 0 {
            return k;
        }
    }
    lines.len() - 1
}

fn brace_delta(line: &str) -> (i64, bool) {
    let (mut delta, mut seen) = (0i64, false);
    let mut chars = line.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '/' if chars.peek() == Some(&'/') => break,
            '\'' => {
                let rest: String = chars.clone().take(3).collect();
                if let Some(end) = rest.find('\'') {
                    for _ in 0..=end {
                        chars.next();
                    }
                }
            }
            '{' => {
                delta += 1;
                seen = true;
            }
            '}' => delta -= 1,
            _ => {}
        }
    }
    (delta, seen)
}

/// Every `(path, line number, needle)` hit of `needles` in non-test source,
/// skipping comment lines and `(file suffix, needle)` allowlist entries.
pub fn hits(needles: &[&str], allow: &[(&str, &str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, src) in src_files() {
        for (n, line) in non_test_source(&src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in needles {
                if line.contains(needle)
                    && !allow
                        .iter()
                        .any(|(f, nd, _)| path.ends_with(f) && nd == needle)
                {
                    out.push(format!("{path}:{}: {needle}", n + 1));
                }
            }
        }
    }
    out
}
