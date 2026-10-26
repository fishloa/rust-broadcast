//! Every `pub enum` carries `#[non_exhaustive]` within the five lines above it.

mod common;

const SKIP: &[&str] = &[];

#[test]
fn every_public_enum_is_non_exhaustive() {
    let mut missing = Vec::new();
    for (_path, content) in common::scan::src_files() {
        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim_start().strip_prefix("pub enum ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() || SKIP.contains(&name.as_str()) {
                continue;
            }
            let start = i.saturating_sub(5);
            if !lines[start..i]
                .iter()
                .any(|l| l.trim_start().starts_with("#[non_exhaustive]"))
            {
                missing.push(name);
            }
        }
    }
    assert!(missing.is_empty(), "pub enum(s) missing #[non_exhaustive]: {missing:?}");
}
