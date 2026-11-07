#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../dash-runtime/tests/it/common/live_fuzz_driver.rs"]
mod driver;

fuzz_target!(|data: &[u8]| {
    driver::run(data, &|rel| {
        std::fs::read(format!("{}/../fixtures/{rel}", env!("CARGO_MANIFEST_DIR")))
            .expect("committed fixture")
    });
});
