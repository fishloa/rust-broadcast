//! Lexical source scanning shared by the guard tests (a tripwire, not a proof).

pub fn src_files() -> Vec<(String, String)> {
    workspace_guards::src_files("dash-runtime")
}

pub use workspace_guards::scan::non_test_source;

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
