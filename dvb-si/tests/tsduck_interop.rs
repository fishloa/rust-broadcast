//! Interop test: decode a PMT compiled by TSDuck (the reference DVB toolkit,
//! `tstabcomp`) carrying MPEG-2 systems descriptors, and assert our typed
//! decode matches what TSDuck encoded.
//!
//! Fixture `tsduck-mpeg-descriptors-pmt.bin` was produced from
//! `tsduck-mpeg-descriptors-pmt.xml` via `tstabcomp -c`. This is independent
//! cross-validation — TSDuck encodes, we decode — far stronger than a
//! self-round-trip. In particular TSDuck sets the HEVC reserved bits [3:2] to
//! '1', so the `HDR_WCG_idc` (bits [1:0]) assertion catches any parser that
//! reads it from the wrong bit position.

use dvb_common::Parse;
use dvb_si::descriptors::hdr_wcg_idc::HdrWcgIdc;
use dvb_si::descriptors::video_stream::FrameRateCode;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::pmt::PmtSection;

const PMT: &[u8] = include_bytes!("fixtures/tsduck-mpeg-descriptors-pmt.bin");

#[test]
fn decodes_tsduck_compiled_mpeg_descriptors() {
    let pmt = PmtSection::parse(PMT).expect("TSDuck PMT section must parse");

    let mut seen_video = false;
    let mut seen_audio = false;
    let mut seen_avc = false;
    let mut seen_hevc = false;

    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            match desc.expect("descriptor must parse") {
                AnyDescriptor::VideoStream(d) => {
                    seen_video = true;
                    assert!(d.multiple_frame_rate_flag);
                    assert_eq!(d.frame_rate_code, FrameRateCode::Frame29_97); // code 4
                    assert!(!d.mpeg_1_only_flag);
                    assert_eq!(d.profile_and_level_indication, Some(0x4D));
                    assert_eq!(d.chroma_format, Some(1));
                }
                AnyDescriptor::AudioStream(d) => {
                    seen_audio = true;
                    assert!(d.id);
                    assert_eq!(d.layer, 2);
                    assert!(!d.free_format_flag);
                }
                AnyDescriptor::AvcVideo(d) => {
                    seen_avc = true;
                    assert_eq!(d.profile_idc, 0x64);
                    assert!(d.constraint_set0_flag);
                    assert!(d.constraint_set3_flag);
                    assert!(d.constraint_set5_flag);
                    assert_eq!(d.avc_compatible_flags, 0x03);
                    assert_eq!(d.level_idc, 0x29);
                    assert!(d.avc_24_hour_picture_flag);
                }
                AnyDescriptor::HevcVideo(d) => {
                    seen_hevc = true;
                    assert_eq!(d.profile_space, 1);
                    assert!(d.tier_flag);
                    assert_eq!(d.profile_idc, 2);
                    assert_eq!(d.profile_compatibility_indication, 0xDEAD_BEEF);
                    assert_eq!(d.copied_44bits, 0x123_4567_89AB);
                    assert_eq!(d.level_idc, 0x99);
                    assert!(d.hevc_still_present_flag);
                    // TSDuck sets reserved bits [3:2] to 1; HDR_WCG_idc is bits [1:0].
                    // A parser reading the wrong bits would decode NoIndication (3).
                    assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
                    let ts = d.temporal_sub.expect("temporal sub-block present");
                    assert_eq!(ts.temporal_id_min, 3);
                    assert_eq!(ts.temporal_id_max, 6);
                }
                _ => {}
            }
        }
    }

    assert!(seen_video, "video_stream_descriptor not decoded");
    assert!(seen_audio, "audio_stream_descriptor not decoded");
    assert!(seen_avc, "AVC_video_descriptor not decoded");
    assert!(seen_hevc, "HEVC_video_descriptor not decoded");
}

const PMT2: &[u8] = include_bytes!("fixtures/tsduck-metadata-j2k-pmt.bin");

#[test]
fn decodes_tsduck_compiled_conditional_descriptors() {
    use dvb_si::descriptors::decoder_config_flags::DecoderConfigFlags;
    use dvb_si::descriptors::metadata::DecoderConfig;
    use dvb_si::descriptors::metadata_format::MetadataFormat;
    use dvb_si::descriptors::mpeg_carriage_flags::MpegCarriageFlags;

    let pmt = PmtSection::parse(PMT2).expect("TSDuck PMT section must parse");
    let mut seen_ptr = false;
    let mut seen_meta = false;
    let mut seen_j2k = false;

    for stream in &pmt.streams {
        for desc in stream.es_info.iter() {
            match desc.expect("descriptor must parse") {
                AnyDescriptor::MetadataPointer(d) => {
                    seen_ptr = true;
                    assert_eq!(d.metadata_application_format, 0x0010);
                    assert_eq!(d.metadata_format, MetadataFormat::TeM);
                    assert_eq!(d.metadata_service_id, 0x07);
                    assert_eq!(d.mpeg_carriage_flags, MpegCarriageFlags::SameTs);
                    assert_eq!(d.program_number, Some(0x1234));
                }
                AnyDescriptor::Metadata(d) => {
                    seen_meta = true;
                    assert_eq!(d.metadata_application_format, 0x0011);
                    assert_eq!(d.metadata_format, MetadataFormat::AppFormat);
                    assert_eq!(d.metadata_service_id, 0x09);
                    assert_eq!(d.decoder_config_flags, DecoderConfigFlags::OtherService);
                    match d.decoder_config {
                        DecoderConfig::OtherService(ref o) => {
                            assert_eq!(o.decoder_config_metadata_service_id, 0x2A);
                        }
                        _ => panic!("expected OtherService decoder_config"),
                    }
                }
                AnyDescriptor::J2kVideo(d) => {
                    seen_j2k = true;
                    assert!(!d.extended_capability_flag);
                    assert!(d.extended_capability.is_none());
                    assert_eq!(d.profile_and_level, 0x0102);
                    assert_eq!(d.horizontal_size, 1920);
                    assert_eq!(d.vertical_size, 1080);
                    assert_eq!(d.color_specification, Some(0x01));
                    assert!(!d.still_mode);
                    assert!(d.interlaced_video);
                }
                _ => {}
            }
        }
    }
    assert!(seen_ptr, "metadata_pointer_descriptor not decoded");
    assert!(seen_meta, "metadata_descriptor not decoded");
    assert!(seen_j2k, "J2K_video_descriptor not decoded");
}
