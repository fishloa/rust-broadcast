//! Print the run-time inventory the symphonia adapter reports.

use broadcast_codec::{Availability, Registry, render_inventory};
use codec_symphonia::SymphoniaProvider;

#[test]
fn prints_the_symphonia_inventory() {
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let providers = registry.providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name(), "symphonia");
    assert_eq!(providers[0].library(), Some("symphonia 0.6.1"));
    assert_eq!(providers[0].licence(), Some("MPL-2.0"));

    let entries = registry.inventory(true);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].probe().availability(), Availability::Available);

    let table = render_inventory(&entries);
    println!("{table}");
    assert!(table.contains("symphonia"));
    assert!(table.contains("MPL-2.0"));
}

#[test]
fn the_reported_library_version_is_the_locked_one() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
        .expect("the spike's Cargo.lock");
    let version = symphonia_version(&lock).expect("symphonia in Cargo.lock");
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let library = format!("symphonia {version}");
    assert_eq!(registry.providers()[0].library(), Some(library.as_str()));
}

fn symphonia_version(lock: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"symphonia\"" {
            let version = lines.next()?.trim();
            return version
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_owned);
        }
    }
    None
}
