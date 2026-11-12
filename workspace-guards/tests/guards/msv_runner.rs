//! Tripwire: `mediastreamvalidator` must only be invoked through the shared
//! runner in `test-fixtures/src/process/msv.rs`.

#[test]
fn no_direct_mediastreamvalidator_command_outside_shared_runner() {
    let needle = "Command::new(\"mediastreamvalidator\")";
    let mut bad = Vec::new();
    for member in workspace_guards::workspace_members() {
        let root = workspace_guards::crate_dir(&member);
        for path in workspace_guards::rs_files(&root) {
            let rel = path
                .strip_prefix(workspace_guards::workspace_root())
                .expect("under workspace root")
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "test-fixtures/src/process/msv.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            if text.contains(needle) {
                bad.push(rel);
            }
        }
    }
    assert!(
        bad.is_empty(),
        "direct mediastreamvalidator Command outside shared runner: {}",
        bad.join(", ")
    );
}
