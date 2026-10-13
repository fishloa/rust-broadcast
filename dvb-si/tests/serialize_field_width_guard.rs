//! Serializers must reject a value wider than its wire field instead of
//! masking it into a different, valid-looking value (audit r03-W3 class).
//! Each case also pins that the widest legal value still round-trips, so the
//! guard cannot be "satisfied" by rejecting everything.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::{
    multiplex_buffer_utilization::MultiplexBufferUtilizationDescriptor,
    mvc_extension::MvcExtensionDescriptor, smoothing_buffer::SmoothingBufferDescriptor,
    target_background_grid::TargetBackgroundGridDescriptor, video_window::VideoWindowDescriptor,
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
fn smoothing_buffer_22_bit_fields() {
    let ok = SmoothingBufferDescriptor::new(0x3F_FFFF, 0x3F_FFFF);
    let b = ser(&ok).unwrap();
    assert_eq!(SmoothingBufferDescriptor::parse(&b).unwrap(), ok);
    assert!(is_overflow(ser(&SmoothingBufferDescriptor::new(
        0x40_0000, 1
    ))));
    assert!(is_overflow(ser(&SmoothingBufferDescriptor::new(
        1, 0x40_0000
    ))));
}

#[test]
fn multiplex_buffer_utilization_15_bit_bounds() {
    let ok = MultiplexBufferUtilizationDescriptor::new(true, 0x7FFF, 0x7FFF);
    let b = ser(&ok).unwrap();
    assert_eq!(MultiplexBufferUtilizationDescriptor::parse(&b).unwrap(), ok);
    assert!(is_overflow(ser(
        &MultiplexBufferUtilizationDescriptor::new(true, 0x8000, 0)
    )));
    assert!(is_overflow(ser(
        &MultiplexBufferUtilizationDescriptor::new(true, 0, 0x8000)
    )));
}

#[test]
fn mvc_extension_10_bit_indices_and_3_bit_temporal_ids() {
    let mut ok = MvcExtensionDescriptor::new(1, 2);
    ok.view_order_index_min = 0x3FF;
    ok.view_order_index_max = 0x3FF;
    ok.temporal_id_start = 7;
    ok.temporal_id_end = 7;
    let b = ser(&ok).unwrap();
    assert_eq!(MvcExtensionDescriptor::parse(&b).unwrap(), ok);

    for edit in [
        |d: &mut MvcExtensionDescriptor| d.view_order_index_min = 0x400,
        |d: &mut MvcExtensionDescriptor| d.view_order_index_max = 0x400,
        |d: &mut MvcExtensionDescriptor| d.temporal_id_start = 8,
        |d: &mut MvcExtensionDescriptor| d.temporal_id_end = 8,
    ] {
        let mut d = ok;
        edit(&mut d);
        assert!(is_overflow(ser(&d)));
    }
}

#[test]
fn target_background_grid_14_14_4_bits() {
    let ok = TargetBackgroundGridDescriptor {
        horizontal_size: 0x3FFF,
        vertical_size: 0x3FFF,
        aspect_ratio_information: 0x0F,
    };
    let b = ser(&ok).unwrap();
    assert_eq!(TargetBackgroundGridDescriptor::parse(&b).unwrap(), ok);
    for (h, v, a) in [(0x4000, 0, 0), (0, 0x4000, 0), (0, 0, 0x10)] {
        let d = TargetBackgroundGridDescriptor {
            horizontal_size: h,
            vertical_size: v,
            aspect_ratio_information: a,
        };
        assert!(is_overflow(ser(&d)), "{h:#x} {v:#x} {a:#x}");
    }
}

#[test]
fn video_window_14_14_4_bits() {
    let ok = VideoWindowDescriptor {
        horizontal_offset: 0x3FFF,
        vertical_offset: 0x3FFF,
        window_priority: 0x0F,
    };
    let b = ser(&ok).unwrap();
    assert_eq!(VideoWindowDescriptor::parse(&b).unwrap(), ok);
    for (h, v, p) in [(0x4000, 0, 0), (0, 0x4000, 0), (0, 0, 0x10)] {
        let d = VideoWindowDescriptor {
            horizontal_offset: h,
            vertical_offset: v,
            window_priority: p,
        };
        assert!(is_overflow(ser(&d)), "{h:#x} {v:#x} {p:#x}");
    }
}
