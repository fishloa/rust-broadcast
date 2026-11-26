//! The NAL/Annex B layer on a real committed H.264 elementary stream.

use broadcast_bitstream::annexb::{
    annexb_to_length_prefixed, iter_annexb_nals, iter_length_prefixed_nals,
    length_prefixed_to_annexb,
};
use broadcast_bitstream::au::{AccessUnitSplitter, first_nal_start, split_access_units};
use broadcast_bitstream::nal::{AvcNalUnitType, NalCodec, nal_unit_type};

fn real_annexb() -> Vec<u8> {
    test_fixtures::read("candidates/h264.annexb")
}

#[test]
fn real_stream_splits_into_nals_and_round_trips_byte_identically() {
    let annexb = real_annexb();
    let nals: Vec<&[u8]> = iter_annexb_nals(&annexb).collect();
    assert!(nals.len() > 1, "a real stream carries several NAL units");
    for nal in &nals {
        assert!(!nal.is_empty());
        let ty = nal_unit_type(NalCodec::Avc, nal).expect("a complete NAL header");
        let typed = AvcNalUnitType::from_u8(ty);
        assert_eq!(typed.code(), ty);
    }

    // Length-prefixed form is canonical: converting back and again yields the
    // same bytes, and every NAL survives.
    let canonical = length_prefixed_to_annexb(&annexb_to_length_prefixed(&annexb))
        .expect("the fixture's NAL units frame correctly");
    let again = length_prefixed_to_annexb(&annexb_to_length_prefixed(&canonical))
        .expect("canonical form re-frames");
    assert_eq!(again, canonical, "length <-> Annex B is byte-identical");
    assert_eq!(
        iter_length_prefixed_nals(&annexb_to_length_prefixed(&canonical))
            .expect("canonical form parses")
            .len(),
        nals.len()
    );
}

#[test]
fn real_stream_splits_into_access_units() {
    let annexb = real_annexb();
    let first = first_nal_start(&annexb).expect("the fixture opens with a start code");
    let units = split_access_units(NalCodec::Avc, &annexb);
    assert!(units.len() > 1, "a real stream carries several pictures");
    let concatenated: Vec<u8> = units.iter().flatten().copied().collect();
    assert_eq!(
        concatenated,
        &annexb[first..],
        "the unit bytes are the input from its first start code onward, verbatim"
    );

    let mut splitter = AccessUnitSplitter::new(NalCodec::Avc);
    for chunk in annexb.chunks(188) {
        splitter
            .push(chunk)
            .expect("unbounded splitter accepts the stream");
    }
    splitter.finish();
    let mut count = 0usize;
    while splitter.pop().is_some() {
        count += 1;
    }
    assert_eq!(count, units.len());
}
