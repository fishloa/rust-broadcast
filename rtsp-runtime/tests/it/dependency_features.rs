//! Dependency features follow the workspace rule: serde is opt-in in every
//! crate, so a dependency must not enable its own serde support unconditionally.

const MANIFEST: &str = include_str!("../../Cargo.toml");

fn dependency_line(name: &str) -> &'static str {
    MANIFEST
        .lines()
        .find(|line| line.trim_start().starts_with(name))
        .unwrap_or_else(|| panic!("a {name} dependency"))
}

#[test]
fn bytes_serde_support_is_gated_behind_the_serde_feature() {
    let bytes = dependency_line("bytes ");
    assert!(
        !bytes.contains("serde"),
        "bytes must not enable serde unconditionally: {bytes}"
    );

    let serde = MANIFEST
        .lines()
        .find(|line| line.trim_start().starts_with("serde = ["))
        .expect("a serde feature");
    assert!(
        serde.contains("bytes/serde"),
        "the serde feature must forward bytes/serde: {serde}"
    );
}
