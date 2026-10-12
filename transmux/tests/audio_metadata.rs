//! Typed PMT audio metadata + derived role (issue #1162 R1).
//!
//! Real artifact: `fixtures/ts/france2.ts` (committed DVB capture; PMT ES_info
//! per `france2-PROVENANCE.md`: `fre`, `qad`, `qaa` E-AC-3 audio tracks).
//! Synthetic cases are spec-valid descriptor loops built by hand from ETSI EN
//! 300 468 §6.2.19/§6.4.11 and carried through a real `TsMux` -> `TsDemux`
//! round trip so the descriptors travel inside a real PMT.

use broadcast_common::{Package, Unpackage};
use transmux::{
    AudioMetadata, AudioRole, AudioType, CodecConfig, EditorialClassification, MixType, TsDemux,
    TsMux,
};

const TAG_ISO_639: u8 = 0x0A;
const TAG_EXTENSION: u8 = 0x7F;
const EXT_SUPPLEMENTARY_AUDIO: u8 = 0x06;

/// `ISO_639_language_descriptor` with one entry.
fn iso639(lang: &[u8; 3], audio_type: u8) -> Vec<u8> {
    vec![TAG_ISO_639, 4, lang[0], lang[1], lang[2], audio_type]
}

/// `supplementary_audio_descriptor` (Table 153): mix_type(1) editorial(5)
/// reserved(1)=1 language_code_present(1).
fn supplementary(mix_independent: bool, editorial: u8, lang: Option<&[u8; 3]>) -> Vec<u8> {
    let flags =
        (u8::from(mix_independent) << 7) | (editorial << 2) | 0x02 | u8::from(lang.is_some());
    let mut body = vec![EXT_SUPPLEMENTARY_AUDIO, flags];
    if let Some(l) = lang {
        body.extend_from_slice(l);
    }
    let mut d = vec![TAG_EXTENSION, body.len() as u8];
    d.extend(body);
    d
}

fn role(loop_bytes: &[u8]) -> AudioRole {
    AudioMetadata::from_es_info(loop_bytes).role
}

#[test]
fn france2_real_pmt_roles() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/france2.ts");
    let media = TsDemux::new()
        .unpackage(&std::fs::read(path).unwrap())
        .unwrap();
    let by_pid = |pid: u16| {
        media
            .tracks
            .iter()
            .find(|t| t.spec.source_pid == Some(pid))
            .unwrap()
            .spec
            .audio_metadata()
    };
    // PID 0x82: `fre`, audio_type 0, no supplementary descriptor.
    let fre = by_pid(0x82);
    assert_eq!(fre.languages.len(), 1);
    assert_eq!(fre.languages[0].language, "fre");
    assert_eq!(fre.languages[0].audio_type, AudioType::Undefined);
    assert!(fre.supplementary.is_none());
    assert_eq!(fre.role, AudioRole::Main);
    // PID 0x83: `qad` + supplementary_audio (independent mix, editorial 0x01, `fra`).
    let qad = by_pid(0x83);
    assert_eq!(qad.languages[0].language, "qad");
    let sup = qad.supplementary.expect("supplementary_audio_descriptor");
    assert_eq!(sup.mix_type, MixType::Independent);
    assert_eq!(
        sup.editorial_classification,
        EditorialClassification::AudioDescription
    );
    assert_eq!(sup.language.as_deref(), Some("fra"));
    assert_eq!(qad.role, AudioRole::AudioDescription);
    // PID 0x84: `qaa`, nothing marks it as non-main.
    assert_eq!(by_pid(0x84).role, AudioRole::Main);
}

#[test]
fn role_rules() {
    // France 2 / TF1: `qad` + editorial 0x01.
    let mut france = iso639(b"qad", 0);
    france.extend(supplementary(true, 0x01, Some(b"fra")));
    assert_eq!(role(&france), AudioRole::AudioDescription);
    // `qad` alone, editorial 0x01 alone, audio_type 3 alone: each is enough.
    assert_eq!(role(&iso639(b"qad", 0)), AudioRole::AudioDescription);
    assert_eq!(
        role(&supplementary(true, 0x01, None)),
        AudioRole::AudioDescription
    );
    assert_eq!(role(&iso639(b"fra", 3)), AudioRole::AudioDescription);
    // ORF: hearing impaired via audio_type 2; or editorial 0x02.
    assert_eq!(role(&iso639(b"deu", 2)), AudioRole::HearingImpaired);
    assert_eq!(
        role(&supplementary(true, 0x02, None)),
        AudioRole::HearingImpaired
    );
    // ServusTV: MAIN audio tagged clean effects (audio_type 1) is still Main.
    assert_eq!(role(&iso639(b"deu", 1)), AudioRole::Main);
    assert_eq!(role(&iso639(b"deu", 0)), AudioRole::Main);
    // Annex J.4 main audio: editorial 0x00.
    let mut main = iso639(b"eng", 1);
    main.extend(supplementary(true, 0x00, None));
    assert_eq!(role(&main), AudioRole::Main);
    // Not classifiable: spoken subtitles, general audience, no descriptors,
    // garbage.
    assert_eq!(role(&supplementary(true, 0x03, None)), AudioRole::Unknown);
    assert_eq!(role(&supplementary(false, 0x17, None)), AudioRole::Unknown);
    assert_eq!(role(&[]), AudioRole::Unknown);
    assert_eq!(role(&[TAG_ISO_639, 40, 1, 2]), AudioRole::Unknown);
}

/// Each broadcaster pattern carried inside a real PMT: demux a committed
/// H.264+AAC TS, set the audio track's ES_info loop, mux, re-demux, and read
/// the metadata off the re-demuxed `TrackSpec`.
#[test]
fn synthetic_pmt_through_tsmux() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let bytes = std::fs::read(path).unwrap();
    let mut france = iso639(b"qad", 0);
    france.extend(supplementary(true, 0x01, Some(b"fra")));
    let cases: [(Vec<u8>, AudioRole, &str); 3] = [
        (france, AudioRole::AudioDescription, "qad"),
        (iso639(b"deu", 2), AudioRole::HearingImpaired, "deu"),
        (iso639(b"deu", 1), AudioRole::Main, "deu"),
    ];
    let is_audio = |c: &CodecConfig| matches!(c, CodecConfig::Aac { .. });
    for (loop_bytes, expected, lang) in cases {
        let mut media = TsDemux::new().unpackage(&bytes).unwrap();
        let audio = media
            .tracks
            .iter_mut()
            .find(|t| is_audio(&t.spec.config))
            .expect("audio track");
        audio.spec.es_info_descriptors = loop_bytes;
        let muxed = TsMux::new().package(&media).unwrap();
        let back = TsDemux::new().unpackage(&muxed).unwrap();
        let meta = back
            .tracks
            .iter()
            .find(|t| is_audio(&t.spec.config))
            .unwrap()
            .spec
            .audio_metadata();
        assert_eq!(meta.languages[0].language, lang);
        assert_eq!(meta.role, expected);
    }
}
