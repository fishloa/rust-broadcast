//! Public-API tests for the decoded getters / `set_*` encoders added so callers
//! don't hand-decode BCD/MJD wire fields (issues #37, #38). Each pair is
//! exercised round-trip; the encoders write the project's raw wire fields.

use core::time::Duration;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::RunningStatus;
use dvb_si::tables::eit::EitEvent;

fn eit_event() -> EitEvent<'static> {
    EitEvent::new(
        1,
        [0; 5],
        [0; 3],
        RunningStatus::Undefined,
        false,
        DescriptorLoop::new(&[]),
    )
}

#[test]
fn eit_event_duration_round_trips() {
    let mut ev = eit_event();
    ev.set_duration(Duration::from_secs(3600 + 30 * 60 + 45))
        .unwrap();
    // Wrote the duration field (not start_time), BCD-encoded HHMMSS.
    assert_eq!(ev.duration_raw(), [0x01, 0x30, 0x45]);
    assert_eq!(ev.duration(), Some(Duration::from_secs(5445)));
}

#[test]
fn eit_event_set_duration_rejects_100_hours() {
    let mut ev = eit_event();
    assert!(matches!(
        ev.set_duration(Duration::from_secs(100 * 3600)),
        Err(dvb_si::Error::ValueOutOfRange { .. })
    ));
}

#[cfg(feature = "jiff")]
fn utc(y: i16, mo: i8, d: i8, h: i8, mi: i8, s: i8) -> jiff::Timestamp {
    jiff::tz::TimeZone::UTC
        .to_timestamp(jiff::civil::date(y, mo, d).at(h, mi, s, 0))
        .unwrap()
}

#[cfg(feature = "jiff")]
#[test]
fn eit_event_start_time_round_trips() {
    let mut ev = eit_event();
    let dt = utc(2023, 6, 8, 12, 34, 56);
    ev.set_start_time(dt).unwrap();
    assert_eq!(ev.start_time_utc(), Some(dt));
    let got = jiff::tz::TimeZone::UTC.to_datetime(ev.start_time_utc().unwrap());
    assert_eq!((got.year(), got.month(), got.day()), (2023, 6, 8));
    assert_eq!((got.hour(), got.minute(), got.second()), (12, 34, 56));
}

#[cfg(feature = "jiff")]
#[test]
fn tot_utc_time_round_trips() {
    use dvb_si::tables::tot::TotSection;
    let mut tot = TotSection::new([0; 5], DescriptorLoop::new(&[]));
    let dt = utc(2024, 12, 31, 23, 59, 59);
    tot.set_utc_time(dt).unwrap();
    assert_eq!(tot.utc_time(), Some(dt));
}

#[cfg(feature = "jiff")]
#[test]
fn tdt_utc_time_round_trips() {
    use dvb_si::tables::tdt::TdtSection;
    let mut tdt = TdtSection::new([0; 5]);
    let dt = utc(2025, 1, 2, 3, 4, 5);
    tdt.set_utc_time(dt).unwrap();
    assert_eq!(tdt.utc_time(), Some(dt));
}

#[test]
fn satellite_delivery_decoded_accessors() {
    use broadcast_common::Parse;
    use dvb_si::descriptors::satellite_delivery_system::SatelliteDeliverySystemDescriptor;
    // tag, len, freq 0x01172500 (11.72500 GHz), orbital 0x1920, flags 0x00, symbol_rate+fec.
    let raw = [
        0x43, 11, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, 0x00, 0x02, 0x75, 0x00, 0x00,
    ];
    let mut d = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
    assert_eq!(d.frequency_hz(), Some(11_725_000_000)); // 11.72500 GHz
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
    use broadcast_common::Parse;
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
    use broadcast_common::Parse;
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

#[test]
fn pdc_label_components_round_trip() {
    use dvb_si::descriptors::pdc::PdcDescriptor;
    let mut d = PdcDescriptor::new(0);
    d.set_pil(15, 6, 20, 30).unwrap();
    assert_eq!(
        (d.pil_day(), d.pil_month(), d.pil_hour(), d.pil_minute()),
        (15, 6, 20, 30)
    );
    // day exceeds its 5-bit field.
    assert!(matches!(
        d.set_pil(32, 1, 1, 1),
        Err(dvb_si::Error::ValueOutOfRange { .. })
    ));
}

#[test]
fn frequency_list_decoded_accessors() {
    use dvb_si::descriptors::frequency_list::{CodingType, FrequencyListDescriptor};

    // Cable: 100 Hz units.
    let mut cable = FrequencyListDescriptor::new(CodingType::Cable, vec![]);
    cable
        .set_centre_frequencies_hz(&[346_000_000, 474_000_000])
        .unwrap();
    assert_eq!(
        cable.centre_frequencies_hz(),
        vec![Some(346_000_000), Some(474_000_000)]
    );

    // Satellite: 10 kHz units (consistent with the satellite delivery descriptor).
    let mut sat = FrequencyListDescriptor::new(CodingType::Satellite, vec![]);
    sat.set_centre_frequencies_hz(&[11_725_000_000]).unwrap();
    assert_eq!(sat.centre_frequencies_hz(), vec![Some(11_725_000_000)]);

    // Terrestrial: binary u32 in 10 Hz units (§6.2.13.4).
    let mut terr = FrequencyListDescriptor::new(CodingType::Terrestrial, vec![]);
    terr.set_centre_frequencies_hz(&[781_416_800]).unwrap();
    assert_eq!(terr.centre_frequencies_hz(), vec![Some(781_416_800)]);

    // Undefined coding cannot be interpreted or encoded.
    let undef = FrequencyListDescriptor::new(CodingType::Undefined, vec![[0x01, 0x17, 0x25, 0x00]]);
    assert_eq!(undef.centre_frequencies_hz(), vec![None]);
    let mut undef_mut = FrequencyListDescriptor::new(CodingType::Undefined, vec![]);
    assert!(matches!(
        &(undef_mut.set_centre_frequencies_hz(&[1_000])),
        Err(dvb_si::error::Error::ValueOutOfRange { .. })
    ));
}

#[cfg(feature = "jiff")]
#[test]
fn local_time_offset_decoded_accessors() {
    use dvb_si::descriptors::local_time_offset::LocalTimeOffsetEntry;
    use dvb_si::text::LangCode;
    use jiff::SignedDuration as Duration;

    let mut e = LocalTimeOffsetEntry::new(LangCode(*b"GBR"), 0, false, 0, [0; 5], 0);

    e.set_offsets(Duration::from_hours(1), Duration::from_hours(2))
        .unwrap();
    assert!(!e.local_time_offset_negative);
    assert_eq!(e.local_time_offset(), Some(Duration::from_hours(1)));
    assert_eq!(e.next_time_offset(), Some(Duration::from_hours(2)));

    // Negative (west) offsets share the single polarity bit.
    e.set_offsets(Duration::from_mins(-330), Duration::from_mins(-270))
        .unwrap();
    assert!(e.local_time_offset_negative);
    assert_eq!(e.local_time_offset(), Some(Duration::from_mins(-330)));

    // Mixed signs can't be represented by one polarity bit.
    assert!(matches!(
        &(e.set_offsets(Duration::from_hours(1), Duration::from_hours(-1))),
        Err(dvb_si::error::Error::ValueOutOfRange { .. })
    ));

    let dt = utc(2024, 3, 31, 1, 0, 0);
    e.set_time_of_change(dt).unwrap();
    assert_eq!(e.time_of_change(), Some(dt));
}
