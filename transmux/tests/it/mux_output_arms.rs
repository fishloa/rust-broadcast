//! Guard: no production consumer of [`MuxOutput`] may silently ignore the
//! timeline-break variant (issue #4154).
//!
//! `MuxOutput::Discontinuity` exists so a stage whose break is out of band can
//! hand it to its caller. A consumer that matches the enum without an arm for
//! it drops the break exactly where the issue says it must not. Every `match`
//! over `MuxOutput` in this crate's non-test source must therefore name the
//! variant, and a `if let MuxOutput::…` (which ignores every other variant) is
//! refused outright.
//!
//! The plant in [`the_detector_trips_on_a_wildcard_only_match`] proves the
//! check bites; without it a detector that silently matched nothing would pass
//! this file too.

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Byte offsets of every `match` keyword outside strings and comments.
fn match_keywords(src: &str) -> Vec<usize> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'\'' => {
                if bytes.get(i + 1) == Some(&b'\\') {
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\'' {
                        i += 1;
                    }
                    i += 1;
                } else if bytes.get(i + 2) == Some(&b'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            b'm' if src[i..].starts_with("match")
                && (i == 0 || !is_word_byte(bytes[i - 1]))
                && bytes.get(i + 5).is_none_or(|b| !is_word_byte(*b)) =>
            {
                out.push(i);
                i += 5;
            }
            _ => i += 1,
        }
    }
    out
}

/// The `{ ... }` block that follows the `match` at `at`, if any.
fn match_block(src: &str, at: usize) -> Option<&str> {
    let bytes = src.as_bytes();
    let open = src[at..].find('{')? + at;
    let mut depth = 0i32;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn line_of(src: &str, at: usize) -> usize {
    src[..at].matches('\n').count() + 1
}

fn detector(file: &str, src: &str) -> Vec<String> {
    let mut findings = Vec::new();
    for at in match_keywords(src) {
        let Some(block) = match_block(src, at) else {
            continue;
        };
        // A block is over `MuxOutput` when one of its arm patterns names it;
        // a different enum's match may still *produce* a `MuxOutput` in its
        // body, which is not a consumer.
        let arms_over_output = block.lines().any(|line| {
            line.split_once("=>")
                .is_some_and(|(pattern, _)| pattern.contains("MuxOutput::"))
        });
        if arms_over_output && !block.contains("MuxOutput::Discontinuity") {
            findings.push(format!(
                "{file}:{}: a match over MuxOutput has no MuxOutput::Discontinuity arm",
                line_of(src, at)
            ));
        }
    }
    for (at, _) in src.match_indices("if let MuxOutput::") {
        let line = src[..at]
            .rfind('\n')
            .map_or(src, |offset| &src[offset + 1..]);
        let trimmed = line.trim_start();
        if trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("//")
            || trimmed.starts_with('*')
        {
            continue;
        }
        findings.push(format!(
            "{file}:{}: an `if let MuxOutput::…` ignores every other variant",
            line_of(src, at)
        ));
    }
    findings
}

#[test]
fn the_detector_trips_on_a_wildcard_only_match() {
    let planted = r#"
fn consume(output: MuxOutput) {
    match output {
        MuxOutput::Segment(segment) => use_segment(segment),
        _ => {}
    }
}
"#;
    let findings = detector("planted.rs", planted);
    assert_eq!(findings.len(), 1, "the plant must be found: {findings:?}");
    assert!(findings[0].starts_with("planted.rs:3:"));

    let planted_if_let = r#"
fn consume(output: MuxOutput) {
    if let MuxOutput::Segment(segment) = output {
        use_segment(segment);
    }
}
"#;
    assert_eq!(detector("planted.rs", planted_if_let).len(), 1);

    let clean = r#"
fn consume(output: MuxOutput) {
    match output {
        MuxOutput::Segment(segment) => use_segment(segment),
        MuxOutput::Discontinuity { track } => note(track),
        _ => {}
    }
}
"#;
    assert!(detector("clean.rs", clean).is_empty());
}

#[test]
fn no_non_test_consumer_ignores_the_discontinuity_variant() {
    let mut findings = Vec::new();
    for (file, text) in workspace_guards::src_files("transmux") {
        let source = workspace_guards::scan::non_test_source(&text);
        findings.extend(detector(&file, &source));
    }
    assert!(
        findings.is_empty(),
        "a production MuxOutput consumer can silently drop a timeline break: {findings:?}"
    );
}
