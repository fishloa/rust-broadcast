//! Exact-boundary conditions across the small codecs (audit 21, #2925): the
//! bit cursor at the last bit, the widest BCD carrier, the BER short/long form
//! split, a `Content-Range` whose last position equals the length, the
//! saturating rescales, the 40-bit-nanosecond clock, the language tag guard,
//! the interface-index guard and KLV offset walking.

use broadcast_common::bcd::{MAX_NIBBLES, bcd_to_decimal, decimal_to_bcd};
use broadcast_common::ber::{decode_ber_length, encode_ber_length_as};
use broadcast_common::bits::{BitError, BitReader, BitWriter};
use broadcast_common::content_range::ContentRange;
use broadcast_common::klv::{BerLength, KlvError, KlvItem, collect_klv_items, walk_klv_items};
use broadcast_common::net::{InterfaceError, JoinMismatch, MulticastInterface};
use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::{Rounding, Timescale, rescale_i64, rescale_u64};
use broadcast_common::traits::Serialize;

#[test]
fn skipping_the_exact_remaining_bits_lands_on_the_end() {
    let mut reader = BitReader::new(&[0b1010_1100, 0b0011_0101]);
    assert_eq!(reader.bits_remaining(), 16);
    assert!(reader.skip_bits(16).is_ok());
    assert_eq!(reader.bits_remaining(), 0);

    let mut reader = BitReader::new(&[0b1010_1100, 0b0011_0101]);
    assert_eq!(
        reader.skip_bits(17).err(),
        Some(BitError::OutOfBounds {
            needed_bits: 17,
            remaining_bits: 16,
        })
    );
    assert_eq!(reader.bits_remaining(), 0, "a failed skip leaves the end");

    let mut reader = BitReader::new(&[0xAA]);
    assert!(reader.skip_bits(3).is_ok());
    assert_eq!(reader.bits_remaining(), 5);
    assert!(reader.skip_bits(5).is_ok());
    assert_eq!(reader.bits_remaining(), 0);
    assert!(reader.skip_bits(0).is_ok());
}

#[test]
fn a_writer_reports_byte_alignment_from_its_cursor() {
    let mut buf = [0u8; 2];
    let mut writer = BitWriter::new(&mut buf);
    assert!(writer.is_byte_aligned());
    writer.write_bits(0b101, 3).expect("three bits fit");
    assert!(!writer.is_byte_aligned());
    writer.write_bits(0b10101, 5).expect("five more bits fit");
    assert!(writer.is_byte_aligned());
    assert_eq!(buf, [0b1011_0101, 0]);
}

#[test]
fn the_widest_bcd_carrier_round_trips_and_one_more_nibble_is_refused() {
    let packed = 0x9999_9999_9999_9999u64;
    assert_eq!(MAX_NIBBLES, 16);
    assert_eq!(bcd_to_decimal(packed, 16), Some(9_999_999_999_999_999));
    assert_eq!(decimal_to_bcd(9_999_999_999_999_999, 16), Some(packed));
    assert_eq!(bcd_to_decimal(packed, 17), None);
    assert_eq!(decimal_to_bcd(9_999_999_999_999_999, 17), None);
    assert_eq!(bcd_to_decimal(0x0000_0000_0000_0012, 2), Some(12));
    assert_eq!(decimal_to_bcd(12, 2), Some(0x12));
}

#[test]
fn the_ber_short_and_long_forms_meet_at_one_hundred_and_twenty_seven() {
    let mut buf = [0u8; 9];
    let one = BerLength::Fixed(core::num::NonZeroU8::new(1).expect("one is non-zero"));
    assert_eq!(encode_ber_length_as(127, one, &mut buf), Ok(1));
    assert_eq!(buf[0], 0x7F);
    assert_eq!(
        encode_ber_length_as(128, one, &mut buf),
        Err(KlvError::FixedBerLengthTooSmall { len: 128, width: 1 })
    );
    assert_eq!(decode_ber_length(&[0x7F]), Ok((127, 1)));
    assert_eq!(
        decode_ber_length(&[0x83, 0x01, 0x02, 0x03]),
        Ok((0x0001_0203, 4))
    );
    assert!(matches!(
        decode_ber_length(&[0x83, 0x01, 0x02]),
        Err(KlvError::BufferTooShort(_))
    ));
    assert_eq!(
        decode_ber_length(&[0x80]),
        Err(KlvError::BerIndefiniteLength)
    );
}

#[test]
fn a_content_range_may_end_at_the_last_byte_but_not_at_the_length() {
    let range = ContentRange::parse("bytes 0-9/10").expect("0-9 of 10 is the whole entity");
    assert_eq!(
        (range.first, range.last, range.complete_length),
        (0, 9, Some(10))
    );
    assert_eq!(range.len(), Some(10));
    assert!(!range.is_empty());
    assert!(ContentRange::parse("bytes 0-10/10").is_none());
    assert!(ContentRange::parse("bytes 9-0/10").is_none());
    let one = ContentRange::parse("bytes 5-5/10").expect("a one-byte range is satisfiable");
    assert_eq!(one.len(), Some(1));
    let middle = ContentRange::parse("bytes 2-9/10").expect("2-9 of 10");
    assert_eq!(middle.len(), Some(8));
    assert_eq!(
        ContentRange::parse("bytes 0-18446744073709551615/*")
            .expect("the whole u64 range, length unknown")
            .len(),
        None
    );
}

#[test]
fn a_rescale_off_the_end_saturates_on_its_own_side() {
    let one = Timescale::try_new(1).expect("one is non-zero");
    let two = Timescale::try_new(2).expect("two is non-zero");
    assert_eq!(
        rescale_i64(i64::MIN, one, two, Rounding::TowardZero),
        i64::MIN
    );
    assert_eq!(
        rescale_i64(i64::MAX, two, one, Rounding::TowardZero),
        i64::MAX / 2
    );
    assert_eq!(
        rescale_u64(u64::MAX, one, two, Rounding::TowardZero),
        u64::MAX
    );
    assert_eq!(
        rescale_u64(u64::MAX, two, one, Rounding::TowardZero),
        9_223_372_036_854_775_807
    );
}

#[test]
fn the_clock_converts_microseconds_to_nanoseconds_up_to_its_own_edge() {
    let widest = u64::MAX / 1_000;
    assert_eq!(
        Timestamp::from_micros(widest).as_nanos(),
        18_446_744_073_709_551_000
    );
    assert_eq!(Timestamp::from_micros(widest + 1).as_nanos(), u64::MAX);
    assert_eq!(Timestamp::from_micros(u64::MAX).as_nanos(), u64::MAX);
    assert_eq!(Timestamp::from_micros(1).as_nanos(), 1_000);
}

struct Inert;

impl Stage for Inert {
    type In<'a> = &'a [u8];
    type Out = ();
    type Error = core::convert::Infallible;

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        None
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn a_stage_with_no_timer_schedules_nothing() {
    let mut stage = Inert;
    assert_eq!(stage.next_deadline(), None);
    stage.on_deadline(Timestamp::from_micros(1));
    assert_eq!(stage.next_deadline(), None);
    assert_eq!(
        broadcast_common::stage::Demand::default(),
        stage.demand(),
        "an unscheduled stage asks for nothing in particular"
    );
}

#[test]
fn an_interface_index_over_the_u32_range_is_an_index_error_not_an_address() {
    assert_eq!(
        MulticastInterface::parse("4294967296"),
        Err(InterfaceError::Index("4294967296".to_string()))
    );
    assert_eq!(
        MulticastInterface::parse("99999999999"),
        Err(InterfaceError::Index("99999999999".to_string()))
    );
    assert_eq!(
        MulticastInterface::parse("1.2.3"),
        Err(InterfaceError::Address("1.2.3".to_string()))
    );
    assert_eq!(
        MulticastInterface::parse("4294967295"),
        Ok(MulticastInterface::V6Index(u32::MAX))
    );
    assert_eq!(
        MulticastInterface::parse("eth0"),
        Ok(MulticastInterface::Name("eth0".to_string()))
    );
}

fn key(last: u8) -> [u8; 16] {
    let mut label = [0u8; 16];
    label[15] = last;
    label
}

fn two_items() -> ([u8; 40], usize) {
    let first = KlvItem::new(key(0), &[1u8, 2, 3]);
    let second = KlvItem::new(key(9), &[4u8, 5]);
    let mut buf = [0u8; 40];
    let first_len = first
        .serialize_into(&mut buf)
        .expect("a 40-byte buffer holds it");
    let second_len = second
        .serialize_into(&mut buf[first_len..])
        .expect("the remainder holds it");
    (buf, first_len + second_len)
}

#[test]
fn a_klv_item_compares_its_key_and_value() {
    assert_eq!(
        KlvItem::new(key(0), &[1, 2, 3]),
        KlvItem::new(key(0), &[1, 2, 3])
    );
    assert_ne!(
        KlvItem::new(key(0), &[1, 2, 3]),
        KlvItem::new(key(9), &[1, 2, 3])
    );
    assert_ne!(
        KlvItem::new(key(0), &[1, 2, 3]),
        KlvItem::new(key(0), &[1, 2, 4])
    );
    assert_ne!(
        KlvItem::new(key(0), &[1, 2, 3]),
        KlvItem::new(key(0), &[1, 2])
    );
}

#[test]
fn walking_klv_items_reports_each_ones_byte_offset() {
    let (buf, total) = two_items();
    let mut offsets = Vec::new();
    walk_klv_items(&buf[..total], |offset, item| {
        offsets.push((offset, item.value.len()));
        Ok(())
    })
    .expect("both items parse");
    assert_eq!(offsets, vec![(0, 3), (20, 2)]);
    let collected = collect_klv_items(&buf[..total]).expect("both items parse");
    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0].0, 0);
    assert_eq!(collected[1].0, 20);
    assert_eq!(collected[1].1.value, [4, 5]);
}

#[cfg(feature = "lang")]
mod language_tags {
    use broadcast_common::bcp47_language;

    #[test]
    fn a_private_use_range_passes_through_and_a_digit_subtag_does_not() {
        assert_eq!(bcp47_language("qaa"), Some("qaa".to_string()));
        assert_eq!(bcp47_language("qtz"), Some("qtz".to_string()));
        assert_eq!(bcp47_language("q1a"), None);
        assert_eq!(bcp47_language("qaa9"), None);
        assert_eq!(
            bcp47_language("qzz"),
            None,
            "outside qaa..qtz it is not a private-use tag, and qzz is not a language"
        );
        assert_eq!(bcp47_language("qzx"), None);
    }

    #[test]
    fn a_tag_is_lowercased_and_must_be_three_letters() {
        assert_eq!(bcp47_language("QAA"), Some("qaa".to_string()));
        assert_eq!(bcp47_language("ENG"), Some("en".to_string()));
        assert_eq!(bcp47_language("en"), None);
        assert_eq!(bcp47_language("zz"), None);
        assert_eq!(bcp47_language("english"), None);
    }

    #[test]
    fn the_special_registry_codes_pass_through_unchanged() {
        for code in ["und", "mul", "mis", "zxx"] {
            assert_eq!(bcp47_language(code), Some(code.to_string()), "{code}");
        }
    }
}

#[test]
fn a_join_mismatch_names_its_reason() {
    assert!(JoinMismatch::Group.reason().contains("multicast group"));
    assert!(JoinMismatch::Family.reason().contains("address family"));
    assert!(JoinMismatch::SourceFamily.reason().contains("unicast"));
}
