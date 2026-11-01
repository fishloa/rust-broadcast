//! Rec. ITU-T H.222.0 (08/2023) descriptors and sections added in the
//! "H.222 descriptors" story: interop with TSDuck, and table-section dispatch.
//!
//! **Real captures: none.** The private corpus (`private/fixtures`, 48 TS
//! files incl. the Fraunhofer MPEG-H set, hotbird-mhp-si, france-tnt, the TSDuck
//! mirror) carries none of these tags: a PMT/CAT/TSDT scan found no
//! descriptor 0x2D/0x2E/0x33-0x37/0x39/0x3A and no MPEG-2 extension tag other
//! than 0x08-0x0E, and the only table_id 0x04-0x0B / 0x38-0x39 "sections" are
//! CRC-bad noise inside the random-data capture `test-062.ts`. The vectors here
//! are therefore (a) TSDuck-encoded and (b) hand-built from the spec tables
//! (in-module unit tests).
//!
//! # TSDuck provenance
//!
//! `tests/fixtures/tsduck-h222-descriptors-pmt.bin` is the output of TSDuck 3.44-4676 `tstabcomp -c
//! tsduck-h222-descriptors-pmt.xml -o tsduck-h222-descriptors-pmt.bin`
//! (input XML sha256 `980ac471943900504245e182eb6e90fdc9d3611daa8b253ea43c65069cd3ec0d`,
//! output `.bin` sha256 `0d18a0fbec41f46989eb48568fe62d243d0ae8853c57b0915a29149e7f8cc46c`,
//! 256 bytes). TSDuck writes the spec `reserved` fill, so every reserved field
//! below is its default. The XML input is committed next to it (`.xml`).
//!
//! Not covered by TSDuck (spec-example vectors only): `MPEG-4_text_descriptor`
//! (TSDuck parses TextConfig itself), `ODUpdate`, `timed_metadata_extension`,
//! `JXS_video_descriptor`, and the three section types.

use broadcast_common::{Parse, Serialize};
use dvb_si::descriptors::evc_video::EvcVideoDescriptor;
use dvb_si::descriptors::hdr_wcg_idc::HdrWcgIdc;
use dvb_si::descriptors::mpeg2_ext_codecs::{ConfigurationType, MediaTypeIdc};
use dvb_si::descriptors::mpeg2_ext_hevc::HevcTileSubstreamDetail;
use dvb_si::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
use dvb_si::descriptors::mpeg4_audio_extension::Mpeg4AudioExtensionDescriptor;
use dvb_si::descriptors::mvc_operation_point::MvcOperationPointDescriptor;
use dvb_si::descriptors::stereoscopic::{
    Mpeg2StereoscopicVideoFormatDescriptor, StereoscopicProgramInfoDescriptor,
    StereoscopicServiceType, StereoscopicVideoInfoDescriptor, StereoscopicVideoView,
    UpsamplingFactor,
};
use dvb_si::descriptors::transport_profile::{TransportProfile, TransportProfileDescriptor};
use dvb_si::descriptors::vvc_video::VvcVideoDescriptor;
use dvb_si::error::Error;
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::pmt::PmtSection;

/// The TSDuck-compiled PMT, committed next to its XML input.
fn pmt_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tsduck-h222-descriptors-pmt.bin"
    ))
    .expect("committed TSDuck fixture")
}

/// Every descriptor of the PMT (program_info first, then each ES loop), raw.
fn descriptors() -> Vec<Vec<u8>> {
    let bytes = pmt_bytes();
    let pmt = PmtSection::parse(&bytes).expect("TSDuck PMT parses");
    // `raw_tags` yields (tag, body); rebuild the full TLV.
    let tlv = |(tag, body): (u8, &[u8])| {
        let mut v = vec![tag, u8::try_from(body.len()).unwrap()];
        v.extend_from_slice(body);
        v
    };
    let mut out: Vec<Vec<u8>> = pmt.program_info.raw_tags().map(tlv).collect();
    for s in &pmt.streams {
        out.extend(s.es_info.raw_tags().map(tlv));
    }
    out
}

fn get(tag: u8, ext: Option<u8>) -> Vec<u8> {
    descriptors()
        .into_iter()
        .find(|d| d[0] == tag && ext.is_none_or(|e| d[2] == e))
        .unwrap_or_else(|| panic!("descriptor {tag:#04x}/{ext:?} not in the TSDuck PMT"))
}

fn rt<T>(bytes: &[u8]) -> T
where
    T: for<'a> Parse<'a, Error = Error> + Serialize<Error = Error>,
{
    let d = T::parse(bytes).unwrap();
    let mut out = vec![0u8; d.serialized_len()];
    d.serialize_into(&mut out).unwrap();
    assert_eq!(out, bytes, "byte-identical re-serialization");
    d
}

fn ext(e: u8) -> Mpeg2ExtensionBody {
    rt::<Mpeg2ExtensionDescriptor>(&get(0x3F, Some(e))).body
}

#[test]
fn every_descriptor_in_the_tsduck_pmt_is_typed_and_round_trips() {
    let all = descriptors();
    assert_eq!(all.len(), 22);
    for d in &all {
        match d[0] {
            // TSDuck 3.44 writes ext 0x13 with one more SubstreamOffset per pattern
            // than Table 2-131 prints; our parser follows the spec and rejects those
            // bytes (pinned in `hevc_subregion_follows_the_spec_and_diverges_from_tsduck`).
            0x3F if d[2] == 0x13 => {
                assert!(Mpeg2ExtensionDescriptor::parse(d).is_err());
            }
            0x3F => {
                let e = rt::<Mpeg2ExtensionDescriptor>(d);
                assert!(
                    !matches!(e.body, Mpeg2ExtensionBody::Other { .. }),
                    "ext {:#04x} fell through to Other",
                    d[2]
                );
            }
            tag => {
                // Dispatch through AnyDescriptor must not fall to Unknown either.
                let any = dvb_si::descriptors::parse_loop(d).next().unwrap().unwrap();
                assert!(
                    !matches!(any, dvb_si::descriptors::AnyDescriptor::Unknown { .. }),
                    "tag {tag:#04x} fell through to Unknown"
                );
            }
        }
    }
}

#[test]
fn top_level_descriptors_match_tsduck_values() {
    let d: Mpeg2StereoscopicVideoFormatDescriptor = rt(&get(0x34, None));
    assert_eq!(d.arrangement_type, Some(3));

    let d: StereoscopicProgramInfoDescriptor = rt(&get(0x35, None));
    assert_eq!(
        d.stereoscopic_service_type,
        StereoscopicServiceType::FrameCompatible
    );
    assert_eq!(d.reserved, 0x1F);

    let d: TransportProfileDescriptor = rt(&get(0x37, None));
    assert_eq!(d.transport_profile, TransportProfile::Adaptive);
    assert_eq!(d.private_data, [0xDE, 0xAD]);

    let d: MvcOperationPointDescriptor = rt(&get(0x33, None));
    assert_eq!(d.profile_idc, 118);
    assert_eq!(
        d.constraint_set_flags,
        [true, false, true, false, false, true]
    );
    assert_eq!(d.avc_compatible_flags, 1);
    let op = &d.levels[0].operation_points[0];
    assert_eq!(d.levels[0].level_idc, 40);
    assert_eq!(
        (op.applicable_temporal_id, op.num_target_output_views),
        (3, 2)
    );
    assert_eq!(
        op.es_references
            .iter()
            .map(|e| e.es_reference)
            .collect::<Vec<_>>(),
        [3, 4]
    );

    let d: Mpeg4AudioExtensionDescriptor = rt(&get(0x2E, None));
    assert_eq!(d.audio_profile_level_indications, [0x2F, 0x30]);
    assert_eq!(d.audio_specific_config.as_deref(), Some(&[0x12, 0x10][..]));

    let d: VvcVideoDescriptor = rt(&get(0x39, None));
    assert_eq!(d.profile_idc, 1);
    assert!(d.progressive_source_flag && d.frame_only_constraint_flag);
    assert_eq!(d.level_idc, 0x33);
    assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
    assert_eq!(d.video_properties_tag, 2);
    let t = d.temporal_layer_subset.unwrap();
    assert_eq!((t.temporal_id_min, t.temporal_id_max), (1, 3));

    let d: EvcVideoDescriptor = rt(&get(0x3A, None));
    assert_eq!((d.toolset_idc_h, d.toolset_idc_l), (1, 2));
    assert!(d.picture_24hr_present_flag && !d.still_present_flag);
    assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::WcgOnly);
    let t = d.temporal_layer_subset.unwrap();
    assert_eq!((t.temporal_id_min, t.temporal_id_max), (0, 2));
}

#[test]
fn stereoscopic_video_info_base_and_additional() {
    let all = descriptors();
    let infos: Vec<StereoscopicVideoInfoDescriptor> =
        all.iter().filter(|d| d[0] == 0x36).map(|d| rt(d)).collect();
    assert_eq!(infos.len(), 2);
    assert!(matches!(
        infos[0].view,
        StereoscopicVideoView::Base {
            leftview_flag: true,
            ..
        }
    ));
    assert!(matches!(
        infos[1].view,
        StereoscopicVideoView::Additional {
            usable_as_2d: true,
            horizontal_upsampling_factor: UpsamplingFactor::SameAsBaseView,
            vertical_upsampling_factor: UpsamplingFactor::Half,
            ..
        }
    ));
}

#[test]
fn extension_bodies_match_tsduck_values() {
    let Mpeg2ExtensionBody::HevcTimingAndHrd(d) = ext(0x03) else {
        panic!()
    };
    assert!(d.hrd_management_valid_flag);
    assert_eq!(d.target_schedule_idx, Some(3));
    let p = d.picture_timing.unwrap();
    assert_eq!(p.nk.map(|nk| (nk.n, nk.k)), Some((1, 300)));
    assert_eq!(p.num_units_in_tick, 1001);

    assert!(matches!(ext(0x04), Mpeg2ExtensionBody::AfExtensions(_)));

    let Mpeg2ExtensionBody::HevcOperationPoint(d) = ext(0x05) else {
        panic!()
    };
    assert_eq!(d.profile_tier_levels.len(), 1);
    let ptl = &d.profile_tier_levels[0];
    assert_eq!(ptl.general_profile_idc, 1);
    assert_eq!(ptl.general_profile_space, 0);
    assert!(!ptl.general_tier_flag);
    assert_eq!(ptl.general_profile_compatibility_flags, 0);
    assert_eq!((ptl.copied_44bits, ptl.general_level_idc), (0, 0));
    let op = &d.operation_points[0];
    assert_eq!(op.target_ols, 2);
    assert_eq!(op.es.len(), 2);
    assert!(op.es[0].prepend_dependencies && !op.es[1].prepend_dependencies);
    assert_eq!(op.layers.len(), 2);
    assert!(op.layers[0].output_layer_flag && !op.layers[1].output_layer_flag);
    assert_eq!(op.applicable_temporal_id, 6);
    let fr = op.frame_rate.as_ref().unwrap();
    assert_eq!(
        (fr.constant_frame_rate_info_idc, fr.frame_rate_indicator),
        (2, 50)
    );
    assert_eq!(
        (op.avg_bit_rate, op.max_bit_rate),
        (Some(8000), Some(10000))
    );

    let Mpeg2ExtensionBody::HevcHierarchyExtension(d) = ext(0x06) else {
        panic!()
    };
    assert_eq!(d.extension_dimension_bits, 3);
    assert_eq!(
        (
            d.hierarchy_layer_index,
            d.temporal_id,
            d.nuh_layer_id,
            d.hierarchy_channel
        ),
        (5, 2, 1, 4)
    );
    assert_eq!(
        d.embedded_layers
            .iter()
            .map(|l| l.index)
            .collect::<Vec<_>>(),
        [1, 3]
    );

    let Mpeg2ExtensionBody::GreenExtension(d) = ext(0x07) else {
        panic!()
    };
    assert_eq!(d.constant_backlight_voltage_time_intervals, [0x10, 0x20]);
    assert_eq!(d.max_variations, [0x30]);

    let Mpeg2ExtensionBody::QualityExtension(d) = ext(0x0F) else {
        panic!()
    };
    assert_eq!(d.field_size_bytes, 2);
    assert_eq!(d.metric_codes, [0x5053_4E52, 0x5353_494D]);

    let Mpeg2ExtensionBody::HevcTileSubstream(d) = ext(0x12) else {
        panic!()
    };
    assert_eq!(d.substream_id, 5);
    let HevcTileSubstreamDetail::Additional(v) = d.detail else {
        panic!()
    };
    assert_eq!(v.len(), 2);
    assert!(v[0].flag && !v[1].flag);
    assert_eq!((v[0].substream_id, v[1].substream_id), (1, 2));

    let Mpeg2ExtensionBody::VvcTimingAndHrd(d) = ext(0x15) else {
        panic!()
    };
    assert!(!d.hrd_management_valid_flag);
    assert_eq!(d.picture_timing.unwrap().num_units_in_tick, 1001);

    let Mpeg2ExtensionBody::EvcTimingAndHrd(d) = ext(0x16) else {
        panic!()
    };
    assert!(d.hrd_management_valid_flag);
    assert_eq!(d.picture_timing.unwrap().nk.map(|nk| nk.k), Some(300));

    let Mpeg2ExtensionBody::LcevcVideo(d) = ext(0x17) else {
        panic!()
    };
    assert_eq!(
        (
            d.lcevc_stream_tag,
            d.profile_idc,
            d.level_idc,
            d.sublevel_idc
        ),
        (7, 3, 5, 1)
    );
    assert!(d.processed_planes_type_flag && !d.picture_type_bit_flag && d.field_type_bit_flag);
    assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
    assert_eq!(d.video_properties_tag, 2);

    let Mpeg2ExtensionBody::LcevcLinkage(d) = ext(0x18) else {
        panic!()
    };
    assert_eq!(d.lcevc_stream_tags, [0x10, 0x20]);

    let Mpeg2ExtensionBody::MediaServiceKind(d) = ext(0x19) else {
        panic!()
    };
    let e = &d.entries[0];
    assert!(!e.media_description_flag);
    assert_eq!(e.media_type_idc, MediaTypeIdc::Audio);
    let id = e.identifier.as_ref().unwrap();
    assert_eq!((id.id_length_code, id.id_type), (2, 0x0200));
    assert_eq!(id.media_id, b"ABCD");
    let l = &e.languages[0];
    assert_eq!(l.configuration_type, ConfigurationType::Complete);
    assert_eq!(l.language_code, b"en");
    assert_eq!(l.media_service_types, [1, 9]);
}

#[test]
fn hevc_subregion_follows_the_spec_and_diverges_from_tsduck() {
    use dvb_si::descriptors::mpeg2_ext_hevc::HevcSubregion;
    // Table 2-131 bounds the SubstreamOffset loop by `k < SubstreamCountMinus1[i]`
    // and §2.6.126 calls the field "the number of HEVC tile substreams to be
    // appended", so a layout with SubstreamCountMinus1 = 1 carries ONE offset per
    // pattern. TSDuck 3.44 (the committed PMT) writes SubstreamCountMinus1 = 1
    // followed by TWO offsets (01, ff), i.e. one more than the spec loop.
    //   3f 0e 13 | 84 10 5a | 91 01 3c 07 80 04 38 81 | 01 ff
    let raw = get(0x3F, Some(0x13));
    assert_eq!(
        raw,
        [
            0x3F, 0x0E, 0x13, 0x84, 0x10, 0x5A, 0x91, 0x01, 0x3C, 0x07, 0x80, 0x04, 0x38, 0x81,
            0x01, 0xFF
        ]
    );
    // Our parser reads the single spec-mandated offset (01) and then, because N1
    // is implied by the descriptor size, takes the stray 0xFF as the start of a
    // second layout, which is truncated: the TSDuck bytes are rejected, not
    // silently reinterpreted.
    assert!(matches!(
        Mpeg2ExtensionDescriptor::parse(&raw),
        Err(Error::BufferTooShort { .. })
    ));
    // The offsets TSDuck wrote are exactly what the spec loop reads when the
    // count says two: patching SubstreamCountMinus1 to 2 yields one clean layout.
    let mut patched = raw.clone();
    patched[7] = 0x02;
    let d = rt::<Mpeg2ExtensionDescriptor>(&patched);
    let Mpeg2ExtensionBody::HevcSubregion(HevcSubregion { layouts, .. }) = d.body else {
        panic!("ext 0x13 must be typed");
    };
    assert_eq!(layouts.len(), 1);
    assert_eq!(layouts[0].patterns, [vec![1i8, -1]]);
    assert_eq!(layouts[0].preamble.unwrap().preamble_substream_id, 0x11);
}

#[test]
fn virtual_segmentation_follows_the_spec_and_diverges_from_tsduck() {
    use dvb_si::descriptors::mpeg2_ext_misc::{VirtualSegmentation, VirtualSegmentationBoundary};
    // Table 2-129 / §2.6.121: maximum_duration_length_minus_1 holds MDL - 1
    // ("Length, in bytes (minus one)"), maximum_duration is MDL*8 + 5 bits.
    // TSDuck 3.44 writes the 2-bit field as 1 = MDL itself. Fixture
    // `tsduck-h222-virtual-segmentation-pmt.bin` (tstabcomp of the committed
    // `.xml`: ticks=100, partition 0 explicit with a 13-bit duration 0x1000,
    // partition 1 a dependent partition on PID 0x0200) carries:
    //   3f 0c 10 | 5f 00 03 23 | 8f 30 00 | 1f 5f 10 07
    let bin = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tsduck-h222-virtual-segmentation-pmt.bin"
    ))
    .expect("committed TSDuck fixture");
    let b = [
        0x3F, 0x0C, 0x10, 0x5F, 0x00, 0x03, 0x23, 0x8F, 0x30, 0x00, 0x1F, 0x5F, 0x10, 0x07,
    ];
    assert!(
        bin.windows(b.len()).any(|w| w == b),
        "TSDuck fixture carries the vector"
    );
    // Per the spec the field value 1 means MDL = 2, a 21-bit duration: partition 0
    // consumes 30 00 1f, partition 1 then starts at 5f and is truncated. Rejected.
    assert!(matches!(
        Mpeg2ExtensionDescriptor::parse(&b),
        Err(Error::BufferTooShort { .. })
    ));
    // The same bytes with the field patched to 0 (MDL = 1, a 13-bit duration, what
    // TSDuck meant) parse cleanly under the spec reading.
    let mut patched = b;
    patched[6] = 0x21;
    let d = rt::<Mpeg2ExtensionDescriptor>(&patched);
    let Mpeg2ExtensionBody::VirtualSegmentation(VirtualSegmentation {
        config: Some(cfg), ..
    }) = d.body
    else {
        panic!("ext 0x10 must be typed");
    };
    assert_eq!(cfg.timescale.unwrap().ticks_per_second, 100);
    assert_eq!(cfg.partitions.len(), 2);
    assert!(matches!(
        cfg.partitions[0].boundary,
        VirtualSegmentationBoundary::Explicit {
            maximum_duration: 0x1000
        }
    ));
    assert!(matches!(
        cfg.partitions[1].boundary,
        VirtualSegmentationBoundary::Reference {
            boundary_pid: 0x0200,
            ..
        }
    ));
}

#[test]
fn new_sections_dispatch_through_any_table_section() {
    use dvb_si::tables::green_access_unit_section::GreenAccessUnitSection;
    use dvb_si::tables::iso14496_section::Iso14496Section;
    use dvb_si::tables::metadata_section::{MetadataSection, SectionFragmentIndication};

    fn bytes<T: Serialize<Error = Error>>(t: &T) -> Vec<u8> {
        let mut b = vec![0u8; t.serialized_len()];
        t.serialize_into(&mut b).unwrap();
        b
    }
    for tid in [0x04u8, 0x05, 0x08] {
        let b = bytes(&Iso14496Section::new(tid, 1, 2, true, 0, 0, &[1, 2, 3]));
        assert!(matches!(
            AnyTableSection::parse(&b).unwrap(),
            AnyTableSection::Iso14496Section(s) if s.table_id == tid
        ));
    }
    let b = bytes(&MetadataSection::new(
        7,
        SectionFragmentIndication::Complete,
        1,
        0,
        0,
        &[9, 9],
    ));
    assert!(matches!(
        AnyTableSection::parse(&b).unwrap(),
        AnyTableSection::MetadataSection(_)
    ));
    let b = bytes(&GreenAccessUnitSection::new(90_000, &[1]));
    assert!(matches!(
        AnyTableSection::parse(&b).unwrap(),
        AnyTableSection::GreenAccessUnitSection(_)
    ));
    // table_ids with no syntax in the primary text stay Unknown (bytes kept).
    for tid in [0x07u8, 0x0A, 0x0B, 0x38, 0x39] {
        let mut b = vec![0u8; 12];
        b[0] = tid;
        b[1] = 0xB0;
        b[2] = 9;
        assert!(matches!(
            AnyTableSection::parse(&b).unwrap(),
            AnyTableSection::Unknown { table_id, .. } if table_id == tid
        ));
    }
}
