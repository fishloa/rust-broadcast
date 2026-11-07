#![no_main]

use libfuzzer_sys::fuzz_target;
use ts_fix::{PcrRestamp, PidFilter, Stuffing, TsFix};

const TS_PACKET_SIZE: usize = 188;
const FILTER_SERVICE: u16 = 1;
const PADDING_RATE: f64 = 1.5;
const BITRATE_BPS: u64 = 10_000_000;

fuzz_target!(|data: &[u8]| {
    let Some((&mode, stream)) = data.split_first() else {
        return;
    };
    let builder = TsFix::builder().repair_continuity().regen_psi();
    let builder = match mode % 4 {
        0 => builder.restamp_pcr(PcrRestamp::interpolate()),
        1 => builder.restamp_pcr(PcrRestamp::from_bitrate(BITRATE_BPS)),
        2 => builder
            .filter_pids(PidFilter::service(FILTER_SERVICE))
            .honor_pcr_discontinuity(),
        _ => builder.stuffing(Stuffing::pad_to(PADDING_RATE)),
    };
    let Ok(mut fix) = builder.build() else {
        return;
    };
    let mut emitted = 0usize;
    for packet in stream.chunks_exact(TS_PACKET_SIZE) {
        let _ = fix.push(packet, |out| {
            assert_eq!(out.len(), TS_PACKET_SIZE);
            emitted += 1;
        });
    }
    fix.finish(|out| assert_eq!(out.len(), TS_PACKET_SIZE));
    let _ = emitted;
});
