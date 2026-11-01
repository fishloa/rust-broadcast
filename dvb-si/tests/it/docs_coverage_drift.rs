//! Drift guard for the generated coverage documentation.
//!
//! `tools/gen-dvbsi-coverage.py` derives `dvb-si/docs/COVERAGE.md`, the "At a
//! glance" blocks of `README.md` and the crate-root `//!` docs, and the
//! spec-transcription index in `docs/README.md` from the crate's
//! single sources of truth (the `declare_tables!` / `declare_descriptors!` /
//! `declare_extension_bodies!` / `declare_ait_descriptors!` lists, each
//! module's spec citation, the `DescriptorRegistry` opt-ins, the `Charset`
//! enum, the vendored registries) joined with `dvb-si/docs/coverage/allocations.tsv`.
//!
//! This test re-runs the generator in `--check` mode. It fails when a table or
//! descriptor is added, removed, re-cited or re-scoped without regenerating the
//! docs — so the counts and "Not implemented" rows a reader sees can never rot.
//! Fix a failure by running `python3 tools/gen-dvbsi-coverage.py` from the
//! workspace root and committing the result.
//!
//! The generator is Python (stdlib only). CI runners always have `python3`;
//! a missing interpreter is a hard failure, never a skip — a skipped drift
//! guard is a guard that does not exist.

use std::path::Path;
use std::process::Command;

#[test]
fn generated_coverage_docs_are_current() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("dvb-si lives inside the workspace");
    let script = workspace.join("tools").join("gen-dvbsi-coverage.py");
    assert!(script.is_file(), "generator missing: {}", script.display());

    let output = Command::new("python3")
        .arg(&script)
        .arg("--check")
        .current_dir(workspace)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "could not run `python3 {}` ({e}); the docs drift guard needs a Python 3 \
                 interpreter on PATH",
                script.display()
            )
        });

    assert!(
        output.status.success(),
        "dvb-si coverage docs are stale or the generator failed (exit {:?}).\n\
         Run `python3 tools/gen-dvbsi-coverage.py` from the workspace root and commit the result.\n\
         --- generator stderr ---\n{}\n--- generator stdout ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}
