//! Behavioural half of the `& MASK` guard (#1129 class): a struct field wider
//! than its wire bit-field must make `serialize_into` return
//! `Error::FieldOverflow`, never wrap into a different valid-looking value.
//! Each case pins that the widest legal value still round-trips byte-exactly,
//! so rejecting everything cannot satisfy it. The lexical half is
//! `serialize_mask_guard.rs`.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::component::ComponentDescriptor;
use dvb_si::descriptors::short_smoothing_buffer::ShortSmoothingBufferDescriptor;
use dvb_si::descriptors::system_clock::SystemClockDescriptor;
use dvb_si::descriptors::terrestrial_delivery_system::{
    Bandwidth, TerrestrialDeliverySystemDescriptor,
};
use dvb_si::error::Error;

fn ser<T: Serialize<Error = Error>>(d: &T) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; d.serialized_len()];
    d.serialize_into(&mut buf)?;
    Ok(buf)
}

fn is_overflow<T>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::FieldOverflow(_)))
}

#[test]
fn system_clock_field_widths() {
    let ok = SystemClockDescriptor::new(true, 0x3F, 0x07);
    let b = ser(&ok).unwrap();
    assert_eq!(SystemClockDescriptor::parse(&b).unwrap(), ok);
    let edits: [fn(&mut SystemClockDescriptor); 4] = [
        |d| d.clock_accuracy_integer = 0x40,
        |d| d.clock_accuracy_exponent = 0x08,
        |d| d.reserved_1 = 0x02,
        |d| d.reserved_5 = 0x20,
    ];
    for edit in edits {
        let mut d = ok.clone();
        edit(&mut d);
        assert!(is_overflow(ser(&d)), "{d:?}");
    }
}

#[test]
fn short_smoothing_buffer_field_widths() {
    // sb_size(2) | sb_leak_rate(6)
    let bytes = [0x61, 0x01, 0xFF];
    let ok = ShortSmoothingBufferDescriptor::parse(&bytes).unwrap();
    assert_eq!(ser(&ok).unwrap(), bytes);
    let mut a = ok.clone();
    a.sb_size = 4;
    assert!(is_overflow(ser(&a)));
    let mut b = ok;
    b.sb_leak_rate = 0x40;
    assert!(is_overflow(ser(&b)));
}

#[test]
fn component_nibbles() {
    // stream_content_ext(4)|stream_content(4), type, tag, "eng"
    let bytes = [0x50, 0x06, 0xF1, 0x03, 0x01, b'e', b'n', b'g'];
    let ok = ComponentDescriptor::parse(&bytes).unwrap();
    assert_eq!(ser(&ok).unwrap(), bytes);
    let mut a = ok.clone();
    a.stream_content_ext = 0x10;
    assert!(is_overflow(ser(&a)));
    let mut b = ok;
    b.stream_content = 0x10;
    assert!(is_overflow(ser(&b)));
}

#[test]
fn terrestrial_reserved_enum_payloads_are_errors() {
    let mut ok = TerrestrialDeliverySystemDescriptor::default();
    ok.bandwidth = Bandwidth::Reserved(0b110);
    assert!(ser(&ok).is_ok());
    let mut bad = TerrestrialDeliverySystemDescriptor::default();
    bad.bandwidth = Bandwidth::Reserved(0x16);
    assert!(is_overflow(ser(&bad)));
}
