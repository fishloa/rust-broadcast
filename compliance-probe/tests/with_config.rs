//! `Probe::with_config` must hand the caller's TR 101 290 thresholds to the
//! underlying monitor rather than silently using the defaults (audit a23cp
//! #3428).

use core::time::Duration;

use compliance_probe::Probe;
use dvb_conformance::Config;
use mpeg_ts::ts::TS_PACKET_SIZE;

/// Arrival interval of one 188-byte packet, 40 µs: a full DVB multiplex rate.
const PACKET_INTERVAL: Duration = Duration::from_micros(40);
/// 40 packets span 1.6 ms of stream.
const PACKET_COUNT: usize = 40;
/// Much tighter than the 500 ms default PAT interval, but far above the
/// per-packet spacing, so the PAT absence is unambiguous.
const TIGHT_PAT_INTERVAL: Duration = Duration::from_micros(100);
/// ETSI TR 101 290 §5.2 Table 5.0a indicator 1.3.a.
const PAT_ERROR_INDICATOR: &str = "PAT_error_2";

fn null_packet(cc: u8) -> [u8; TS_PACKET_SIZE] {
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    packet[0] = 0x47;
    packet[1] = 0x1F;
    packet[2] = 0xFF;
    packet[3] = 0x10 | (cc & 0x0F);
    packet
}

fn pat_error_count(probe: &Probe) -> u64 {
    probe
        .indicator_counts()
        .iter()
        .filter(|count| count.indicator == PAT_ERROR_INDICATOR)
        .map(|count| count.count)
        .sum()
}

#[test]
fn with_config_applies_the_callers_pat_interval() {
    let mut tight = Config::default();
    tight.pat_max_interval = TIGHT_PAT_INTERVAL;

    let mut default_probe = Probe::new();
    let mut tight_probe = Probe::with_config(tight);
    for index in 0..PACKET_COUNT {
        let at = PACKET_INTERVAL * u32::try_from(index).expect("fits u32");
        let cc = u8::try_from(index & 0x0F).expect("masked to 4 bits");
        default_probe.feed_ts_packet(&null_packet(cc), at);
        tight_probe.feed_ts_packet(&null_packet(cc), at);
    }

    assert_eq!(
        pat_error_count(&default_probe),
        0,
        "the 500 ms default PAT interval cannot elapse in 1.6 ms of stream"
    );
    assert!(
        pat_error_count(&tight_probe) >= 1,
        "a 100 µs PAT interval must time out within 1.6 ms of stream; \
         with_config ignoring the caller's config leaves this at 0"
    );
}
