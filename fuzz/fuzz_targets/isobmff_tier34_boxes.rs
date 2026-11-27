#![no_main]

use broadcast_core::Parse;
use libfuzzer_sys::fuzz_target;

// Every tier 3 and tier 4 box parser of issue #4167, reached from one selector
// byte. A box body is untrusted wire data: a count or a presence flag read from
// it must never drive an allocation or an out-of-bounds read.
fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn sample_entry(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut payload = vec![0u8; 6];
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(body);
    boxed(fourcc, &payload)
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    match selector % 27 {
        0 => {
            let _ = isobmff::UriBox::parse(&boxed(*b"uri ", body));
        }
        1 => {
            let _ = isobmff::UriInitBox::parse(&boxed(*b"uriI", body));
        }
        2 => {
            let _ = isobmff::TextConfigBox::parse(&boxed(*b"txtC", body));
        }
        3 => {
            let _ = isobmff::MimeBox::parse(&boxed(*b"mime", body));
        }
        4 => {
            let _ = isobmff::XmlBox::parse(&boxed(*b"xml ", body));
        }
        5 => {
            let _ = isobmff::BinaryXmlBox::parse(&boxed(*b"bxml", body));
        }
        6 => {
            let _ = isobmff::CopyrightBox::parse(&boxed(*b"cprt", body));
        }
        7 => {
            let _ = isobmff::MetadataKeyDeclarationBox::parse(&boxed(*b"keyd", body));
        }
        8 => {
            let _ = isobmff::MetadataLocaleBox::parse(&boxed(*b"loca", body));
        }
        9 => {
            let _ = isobmff::MetadataSetupBox::parse(&boxed(*b"setu", body));
        }
        10 => {
            let _ = isobmff::MetadataKeyBox::parse(&boxed(*b"ky01", body));
        }
        11 => {
            let _ = isobmff::MetadataKeyTableBox::parse(&boxed(*b"keys", body));
        }
        12 => {
            let _ = isobmff::XmlMetaDataSampleEntry::parse(&sample_entry(*b"metx", body));
        }
        13 => {
            let _ = isobmff::TextMetaDataSampleEntry::parse(&sample_entry(*b"mett", body));
        }
        14 => {
            let _ = isobmff::UriMetaDataSampleEntry::parse(&sample_entry(*b"urim", body));
        }
        15 => {
            let _ = isobmff::BoxedMetaDataSampleEntry::parse(&sample_entry(*b"mebx", body));
        }
        16 => {
            let _ = isobmff::NullMediaHeaderBox::parse(&boxed(*b"nmhd", body));
            let _ = isobmff::HintMediaHeaderBox::parse(&boxed(*b"hmhd", body));
        }
        17 => {
            let _ = isobmff::TypeCombinationBox::parse(&boxed(*b"tyco", body));
            let _ = isobmff::ExtendedTypeBox::parse(&boxed(*b"etyp", body));
            let _ = isobmff::OriginalFileTypeBox::parse(&boxed(*b"otyp", body));
        }
        18 => {
            let _ = isobmff::IdentifiedMediaDataBox::parse(&boxed(*b"imda", body));
            let _ = isobmff::IdentifiedMediaDataBox::parse_identification(&boxed(*b"imda", body));
        }
        19 => {
            let _ = isobmff::SubTrackInformationBox::parse(&boxed(*b"stri", body));
            let _ = isobmff::SubTrackDefinitionBox::parse(&boxed(*b"strd", body));
            let _ = isobmff::SubTrackBox::parse(&boxed(*b"strk", body));
        }
        20 => {
            let _ = isobmff::MovieExtendsHeaderBox::parse(&boxed(*b"mehd", body));
            let _ = isobmff::VolumetricVisualMediaHeaderBox::parse(&boxed(*b"vvhd", body));
        }
        21 => {
            let _ = isobmff::RestrictedSchemeInfoBox::parse(&boxed(*b"rinf", body));
            let _ = isobmff::ScrambleSchemeInfoBox::parse(&boxed(*b"scrb", body));
            let _ = isobmff::CompleteTrackInfoBox::parse(&boxed(*b"cinf", body));
        }
        22 => {
            let _ = isobmff::CompatibleSchemeTypeBox::parse(&boxed(*b"csch", body));
            let _ = isobmff::SamplePackingInformationBox::parse(&boxed(*b"spki", body));
        }
        23 => {
            let _ = isobmff::AmbientViewingEnvironmentBox::parse(body);
            let _ = isobmff::ContentColourVolumeBox::parse(body);
        }
        24 => {
            let _ = isobmff::AudioRenderingIndicationBox::parse(&boxed(*b"ardi", body));
            let _ = isobmff::AudioElementDescriptionBox::parse(&boxed(*b"aedb", body));
            let _ = isobmff::AudioElementSelectionDescriptionBox::parse(&boxed(*b"aesd", body));
        }
        25 => {
            let _ = isobmff::AudioElementBox::parse(&boxed(*b"aelm", body));
            let _ = isobmff::AudioElementSelectionBox::parse(&boxed(*b"aesb", body));
            let _ = isobmff::AudioElementPosInteractivityPolarBox::parse(&boxed(*b"aepp", body));
            let _ = isobmff::AudioElementProminenceInteractivityBox::parse(&boxed(*b"aepr", body));
        }
        _ => {
            let _ = isobmff::AlternativeStartupSequencePropertiesBox::parse(&boxed(*b"assp", body));
            if let Ok(trep) = isobmff::TrackExtensionPropertiesBox::parse(&boxed(*b"trep", body)) {
                let _ = trep.alternative_startup();
            }
        }
    }
});
