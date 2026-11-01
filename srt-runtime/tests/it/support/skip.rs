// The one place that decides what happens when an external oracle tool is
// missing. `include!`d by every libsrt-oracle test file (each integration test
// is its own crate, so a shared `mod` would warn about unused items there).
//
// A missing tool skips the test LOUDLY (`--nocapture` shows why): it is then a
// no-op pass, not coverage. Setting `SRT_REQUIRE_LIBSRT=1` makes it a hard
// failure instead — use it wherever the tools are supposed to be installed.

fn tool_available(tool: &str, version_arg: &str) -> bool {
    std::process::Command::new(tool)
        .arg(version_arg)
        .output()
        .is_ok_and(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
}

/// `skip_unless_tools!("srt-live-transmit" => "-version", ...)`: return early
/// (loudly) unless every tool runs.
macro_rules! skip_unless_tools {
    ($($tool:literal => $arg:literal),+ $(,)?) => {
        if !($(tool_available($tool, $arg))&&+) {
            assert!(
                std::env::var_os("SRT_REQUIRE_LIBSRT").is_none(),
                "SRT_REQUIRE_LIBSRT is set but a required oracle tool ({}) is not on PATH",
                [$($tool),+].join(", ")
            );
            eprintln!(
                "SKIP {}: needs {} on PATH. This is a NO-OP result on this host, not real \
                 coverage (set SRT_REQUIRE_LIBSRT=1 to make it a failure).",
                module_path!(),
                [$($tool),+].join(", ")
            );
            return;
        }
    };
}
