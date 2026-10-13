// Shared body of the per-crate `tests/secret_debug_guard.rs` tripwire (audit
// umbrella #1142, remediation item "redacting Debug impls for key material").
//
// `include!`d by each crate that holds secrets (passwords, tokens, stream keys,
// passphrases, keys, ICE credentials, ...). It scans `src/**/*.rs` outside the
// `#[cfg(test)]` module for a `#[derive(.. Debug ..)]` on a struct/enum whose
// body has a field with a secret-looking name: a derived `Debug` would print
// that field verbatim into any `{:?}` / `dbg!` / tracing field / panic message.
// Such a type needs a manual `Debug` that redacts the field, or a reasoned
// allowlist entry (a name that merely looks secret).
//
// This is a LEXICAL tripwire, not a proof: a secret held under an innocuous
// field name evades it. The per-type `{:?}`-does-not-contain-the-secret tests
// are the behavioural control.

use std::fs;
use std::path::Path;

/// Words (after splitting a field name on `_`) that mark a field as secret.
const SECRET_WORDS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "pwd",
    "secret",
    "token",
    "psk",
    "kek",
    "sek",
    "bearer",
    "credential",
    "credentials",
    "authorization",
];

fn secret_field_name(ident: &str) -> bool {
    let lower = ident.to_ascii_lowercase();
    let words: Vec<&str> = lower.split('_').filter(|w| !w.is_empty()).collect();
    words.iter().any(|w| SECRET_WORDS.contains(w))
        || matches!(words.last(), Some(&"key") | Some(&"keys"))
}

fn collect_rs(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).expect("read source"),
            ));
        }
    }
}

/// Source up to the first `#[cfg(test)]` that opens a `mod` (the test module).
fn non_test_prefix(src: &str) -> &str {
    let lines: Vec<&str> = src.lines().collect();
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("#[cfg(test)]")
            && lines
                .get(i + 1..i + 3)
                .is_some_and(|next| next.iter().any(|l| l.trim_start().starts_with("mod ")))
        {
            return &src[..offset];
        }
        offset += line.len() + 1;
    }
    src
}

/// Every `(type name, secret field name)` for a derived-`Debug` item in `src`.
fn derived_debug_secrets(src: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = non_test_prefix(src).lines().collect();
    let mut hits = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with("#[derive(") {
            i += 1;
            continue;
        }
        let mut derive = String::new();
        let mut j = i;
        loop {
            derive.push_str(lines[j]);
            if lines[j].contains(")]") || j + 1 >= lines.len() {
                break;
            }
            j += 1;
        }
        let has_debug = derive
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == "Debug");
        let mut k = j + 1;
        while k < lines.len() {
            let t = lines[k].trim_start();
            if t.starts_with("#[") || t.starts_with("///") || t.starts_with("//") || t.is_empty() {
                k += 1;
            } else {
                break;
            }
        }
        let Some(head) = lines.get(k).map(|l| l.trim_start()) else {
            break;
        };
        let head = head
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub(super) ")
            .trim_start_matches("pub ");
        let kind_len = if head.starts_with("struct ") {
            7
        } else if head.starts_with("enum ") {
            5
        } else {
            i = k.max(i + 1);
            continue;
        };
        let name: String = head[kind_len..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // Brace-matched body (a `;`-terminated tuple/unit struct has no named fields).
        let mut depth = 0i32;
        let mut started = false;
        let mut m = k;
        while m < lines.len() {
            let line = lines[m];
            if has_debug && started && depth >= 1 {
                let field = line.trim_start();
                let field = field
                    .trim_start_matches("pub(crate) ")
                    .trim_start_matches("pub(super) ")
                    .trim_start_matches("pub ");
                let ident: String = field
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty()
                    && field[ident.len()..].trim_start().starts_with(':')
                    && !field[ident.len()..].trim_start().starts_with("::")
                    && secret_field_name(&ident)
                {
                    hits.push((name.clone(), ident));
                }
            }
            for c in line.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        started = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if (started && depth <= 0) || (!started && line.trim_end().ends_with(';')) {
                break;
            }
            m += 1;
        }
        i = m + 1;
    }
    hits
}

/// Fails if any `#[derive(Debug)]` type under `<manifest_dir>/src` has a
/// secret-named field and is not in `allow` (`(file suffix, type, reason)`).
fn assert_no_derived_debug_on_secrets(manifest_dir: &str, allow: &[(&str, &str, &str)]) {
    let mut files = Vec::new();
    collect_rs(&Path::new(manifest_dir).join("src"), &mut files);
    assert!(!files.is_empty(), "no source files scanned");
    let mut offenders = Vec::new();
    for (path, src) in &files {
        for (ty, field) in derived_debug_secrets(src) {
            let allowed = allow
                .iter()
                .any(|(suffix, t, _)| path.ends_with(suffix) && *t == ty);
            if !allowed {
                offenders.push(format!("{path}: `{ty}` derives Debug over secret field `{field}`"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "derived Debug over secret-named fields (write a redacting Debug impl, or allowlist \
         a name that is not a secret with a reason):\n{}",
        offenders.join("\n")
    );
}

/// The scanner itself must bite: a derived Debug over a secret field is found,
/// a manual impl / a non-secret name / a test-module type is not.
#[test]
fn scanner_flags_a_derived_debug_secret_and_ignores_the_rest() {
    let bad = "#[derive(Debug, Clone)]\npub struct A {\n    pub stream_key: String,\n}\n";
    assert_eq!(
        derived_debug_secrets(bad),
        vec![("A".to_string(), "stream_key".to_string())]
    );
    let multi = "#[derive(\n    Clone,\n    Debug,\n)]\nstruct B {\n    bearer_token: Option<String>,\n}\n";
    assert_eq!(derived_debug_secrets(multi).len(), 1);
    let no_debug = "#[derive(Clone)]\npub struct C {\n    pub password: String,\n}\n";
    assert!(derived_debug_secrets(no_debug).is_empty());
    let innocent = "#[derive(Debug)]\npub struct D {\n    pub key_id: u32,\n    pub name: String,\n}\n";
    assert!(derived_debug_secrets(innocent).is_empty());
    let in_tests = "fn f() {}\n#[cfg(test)]\nmod tests {\n#[derive(Debug)]\nstruct E {\n    password: String,\n}\n}\n";
    assert!(derived_debug_secrets(in_tests).is_empty());
}

/// Fails if non-test source in `<manifest_dir>/src` hand-rolls a constant-time
/// comparison (an XOR accumulated with `|=`, or an XOR/OR `fold`) instead of
/// `subtle::ConstantTimeEq` (audit r09-O1, #1142). Lexical: a renamed helper
/// evades it.
#[allow(dead_code)] // not every including crate runs the compare scan (cipher crates XOR legitimately)
fn assert_no_handrolled_ct_compare(manifest_dir: &str) {
    let mut files = Vec::new();
    collect_rs(&Path::new(manifest_dir).join("src"), &mut files);
    let mut offenders = Vec::new();
    for (path, src) in &files {
        for (n, line) in non_test_prefix(src).lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if (code.contains("|=") && code.contains('^'))
                || (code.contains("fold(0") && code.contains('^'))
            {
                offenders.push(format!("{path}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "hand-rolled constant-time compare (use subtle::ConstantTimeEq):\n{}",
        offenders.join("\n")
    );
}
