#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../dash-runtime/tests/it/common/live_fuzz_driver.rs"]
mod driver;

fuzz_target!(|data: &[u8]| {
    driver::run(data, &test_fixtures::read);
});
