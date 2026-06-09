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

#[test]
fn satellite_delivery_decoded_accessors() {
    use dvb_common::Parse;
    use dvb_si::descriptors::satellite_delivery_system::SatelliteDeliverySystemDescriptor;
    // tag, len, freq 0x11725000, orbital 0x1920, flags 0x00, symbol_rate+fec.
    let raw = [
        0x43, 11, 0x11, 0x72, 0x50, 0x00, 0x19, 0x20, 0x00, 0x02, 0x75, 0x00, 0x00,
    ];
    let mut d = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
    assert_eq!(d.frequency_hz(), Some(11_725_000_000)); // 11.725 GHz
    assert_eq!(d.symbol_rate_sps(), Some(27_500_000)); // 27.5 Msym/s
    assert_eq!(d.orbital_position_deg(), Some(192.0));

    // Setters round-trip at the field resolutions.
    d.set_frequency_hz(12_500_750_000).unwrap();
    assert_eq!(d.frequency_hz(), Some(12_500_750_000));
    d.set_symbol_rate_sps(22_000_000).unwrap();
    assert_eq!(d.symbol_rate_sps(), Some(22_000_000));
    d.set_orbital_position_deg(28.5).unwrap();
    assert_eq!(d.orbital_position_deg(), Some(28.5));
}

#[test]
fn cable_delivery_decoded_accessors() {
    use dvb_common::Parse;
    use dvb_si::descriptors::cable_delivery_system::CableDeliverySystemDescriptor;
    let raw = [
        0x44, 11, 0x03, 0x46, 0x00, 0x00, 0xFF, 0xF1, 0x05, 0x00, 0x00, 0x00, 0x03,
    ];
    let mut d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
    assert_eq!(d.frequency_hz(), Some(346_000_000)); // 346.0000 MHz, 100 Hz resolution

    d.set_frequency_hz(474_000_100).unwrap(); // multiple of 100 Hz
    assert_eq!(d.frequency_hz(), Some(474_000_100));
}

#[test]
fn terrestrial_delivery_centre_frequency_hz() {
    use dvb_common::Parse;
    use dvb_si::descriptors::terrestrial_delivery_system::TerrestrialDeliverySystemDescriptor;
    let raw = [
        0x5A, 11, 0x04, 0xA8, 0x58, 0xF0, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let mut d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
    // 0x04A858F0 = 78_141_680 units of 10 Hz = 781.4168 MHz.
    assert_eq!(d.centre_frequency_hz(), 781_416_800);

    d.set_centre_frequency_hz(490_000_000).unwrap(); // multiple of 10 Hz
    assert_eq!(d.centre_frequency_hz(), 490_000_000);
}
