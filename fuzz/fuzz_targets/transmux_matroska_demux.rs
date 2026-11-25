#![no_main]

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use libfuzzer_sys::fuzz_target;
use transmux::matroska_demux::{MatroskaDemux, MatroskaDemuxConfig, MatroskaInput};

fuzz_target!(|data: &[u8]| {
    let mut demux = MatroskaDemux::sequential(MatroskaDemuxConfig::default());
    let _ = demux.feed(MatroskaInput::Next(data), Timestamp::ZERO);
    let _ = demux.feed(MatroskaInput::Eof, Timestamp::ZERO);
    let _ = demux.finish();
    while demux.poll().is_some() {}
});
