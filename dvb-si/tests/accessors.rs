//! Public-API tests for the decoded getters / `set_*` encoders added so callers
//! don't hand-decode BCD/MJD wire fields (issues #37, #38). Each pair is
//! exercised round-trip; the encoders write the project's raw wire fields.

use core::time::Duration;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::eit::EitEvent;

fn eit_event() -> EitEvent<'static> {
    EitEvent {
        event_id: 1,
        start_time_raw: [0; 5],
        duration_raw: [0; 3],
        running_status: 0,
        free_ca_mode: false,
        descriptors: DescriptorLoop::new(&[]),
    }
}

#[test]
fn eit_event_duration_round_trips() {
    let mut ev = eit_event();
    ev.set_duration(Duration::from_secs(3600 + 30 * 60 + 45)).unwrap();
    // Wrote the duration field (not start_time), BCD-encoded HHMMSS.
    assert_eq!(ev.duration_raw, [0x01, 0x30, 0x45]);
    assert_eq!(ev.duration(), Some(Duration::from_secs(5445)));
}

#[test]
fn eit_event_set_duration_rejects_100_hours() {
    let mut ev = eit_event();
    assert!(ev.set_duration(Duration::from_secs(100 * 3600)).is_err());
}

#[cfg(feature = "chrono")]
#[test]
fn eit_event_start_time_round_trips() {
    use chrono::{Datelike, TimeZone, Timelike, Utc};
    let mut ev = eit_event();
    let dt = Utc.with_ymd_and_hms(2023, 6, 8, 12, 34, 56).unwrap();
    ev.set_start_time(dt).unwrap();
    let got = ev.start_time().unwrap();
    assert_eq!((got.year(), got.month(), got.day()), (2023, 6, 8));
    assert_eq!((got.hour(), got.minute(), got.second()), (12, 34, 56));
}

#[cfg(feature = "chrono")]
#[test]
fn tot_utc_time_round_trips() {
    use chrono::{TimeZone, Utc};
    use dvb_si::tables::tot::TotSection;
    let mut tot = TotSection {
        utc_time_raw: [0; 5],
        descriptors: DescriptorLoop::new(&[]),
    };
    let dt = Utc.with_ymd_and_hms(2024, 12, 31, 23, 59, 59).unwrap();
    tot.set_utc_time(dt).unwrap();
    assert_eq!(tot.utc_time(), Some(dt));
}

#[cfg(feature = "chrono")]
#[test]
fn tdt_utc_time_round_trips() {
    use chrono::{TimeZone, Utc};
    use dvb_si::tables::tdt::TdtSection;
    let mut tdt = TdtSection {
        utc_time_raw: [0; 5],
    };
    let dt = Utc.with_ymd_and_hms(2025, 1, 2, 3, 4, 5).unwrap();
    tdt.set_utc_time(dt).unwrap();
    assert_eq!(tdt.utc_time(), Some(dt));
}
