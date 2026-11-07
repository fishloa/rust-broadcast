#![no_main]

use dvb_ci_runtime::session::SessionLayer;
use dvb_ci_runtime::transport::Transport;
use libfuzzer_sys::fuzz_target;

const TCID: u8 = 1;

fuzz_target!(|data: &[u8]| {
    let Some((&mode, frame)) = data.split_first() else {
        return;
    };

    let mut transport = Transport::new(TCID);
    let _ = transport.init();
    let out = transport.on_frame(frame);
    for spdu in &out.spdus {
        let mut layer = SessionLayer::new();
        let _ = layer.on_spdu(spdu, |_| mode & 1 == 0);
    }

    let mut layer = SessionLayer::new();
    let _ = layer.on_spdu(frame, |_| mode & 1 == 0);
});
