//! `AvcParamSets::classify_nals` (an access unit already split into NAL units)
//! over the real field-coded broadcast capture `ts/gulli-opengop.ts`: it must
//! agree with `classify_au` on every sample, reproduce the capture's known
//! 146 frame-pairs + 1 lone field split, and report `Unknown` for no slice.

use broadcast_common::Unpackage;
use transmux::pipeline::CodecConfig;
use transmux::slice_header::{AvcParamSets, AvcPicture};
use transmux::{TsDemux, iter_length_prefixed_nals};

#[test]
fn classify_nals_agrees_with_classify_au_on_a_real_paff_capture() {
    let ts = test_fixtures::read("ts/gulli-opengop.ts");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC track");
    assert_eq!(track.samples.len(), 147);

    let mut by_nals = AvcParamSets::new();
    let mut by_au = AvcParamSets::new();
    let (mut frames, mut fields) = (0, 0);
    for sample in &track.samples {
        let nals: Vec<&[u8]> = iter_length_prefixed_nals(&sample.data).unwrap().into_iter().collect();
        let mut annexb = Vec::new();
        for n in &nals {
            annexb.extend([0, 0, 0, 1]);
            annexb.extend_from_slice(n);
        }
        let got = by_nals.classify_nals(nals.iter().copied());
        assert_eq!(got, by_au.classify_au(&annexb));
        match got {
            AvcPicture::Frame => frames += 1,
            AvcPicture::Field(_) => fields += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((frames, fields), (146, 1));
}

#[test]
fn no_slice_nal_is_unknown() {
    let mut ps = AvcParamSets::new();
    assert_eq!(ps.classify_nals(std::iter::empty::<&[u8]>()), AvcPicture::Unknown);
    assert_eq!(ps.classify_nals([&[0x09u8, 0xF0][..]]), AvcPicture::Unknown, "AUD only");
    assert_eq!(ps.classify_nals([&[][..]]), AvcPicture::Unknown, "empty NAL");
    let idr_without_parameter_sets = [0x65u8, 0x88, 0x84, 0x00];
    assert_eq!(
        ps.classify_nals([&idr_without_parameter_sets[..]]),
        AvcPicture::Unknown
    );
}
