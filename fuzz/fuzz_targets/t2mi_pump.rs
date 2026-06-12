#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Use a well-known T2-MI PID (0x0006) for TS mode; also test raw mode.
    let mut pump_ts = dvb_t2mi::pump::T2miPump::new(0x0006);
    for chunk in data.chunks(188) {
        for _event in pump_ts.feed_ts(chunk) {}
    }

    let mut pump_raw = dvb_t2mi::pump::T2miPump::raw();
    for _event in pump_raw.feed_raw(data) {}
});
