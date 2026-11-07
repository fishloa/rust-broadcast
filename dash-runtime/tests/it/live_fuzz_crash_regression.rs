//! Regression guard for the `dash_client_live` fuzz crash (bwg2): after a
//! terminal output the client must not emit another output, even when the host
//! clock is updated. Runs the committed crash input through the same driver the
//! fuzz target uses.

use crate::common::live_fuzz_driver;

#[test]
fn dash_client_live_crash_regression() {
    let data = test_fixtures::read("dash/fuzz-regressions/dash_client_live-crash.bin");
    assert!(
        live_fuzz_driver::run(&data),
        "crash input is outside the driver's accepted size range"
    );
}
