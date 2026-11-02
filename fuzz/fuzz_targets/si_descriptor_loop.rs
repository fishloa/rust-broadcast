#![no_main]

use libfuzzer_sys::fuzz_target;

use broadcast_common::Serialize;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::descriptors::DescriptorRegistry;
use dvb_si::descriptors::atsc_caption_service::AtscCaptionServiceDescriptor;

fuzz_target!(|data: &[u8]| {
    for item in dvb_si::descriptors::parse_loop(data) {
        let _ = item;
    }

    let mut registry = DescriptorRegistry::new();
    registry.with_atsc_caption_service();
    for (base, registered) in dvb_si::descriptors::parse_loop(data).zip(registry.parse_loop(data)) {
        let (Ok(base_desc), Ok(registered_desc)) = (base, registered) else {
            continue;
        };
        let AnyDescriptor::Other { tag, value } = registered_desc else {
            continue;
        };
        if tag != dvb_si::descriptors::atsc_caption_service::TAG {
            continue;
        }
        let Some(descriptor) = value.downcast_ref::<AtscCaptionServiceDescriptor>() else {
            continue;
        };
        let AnyDescriptor::Unknown { body, .. } = base_desc else {
            continue;
        };
        let mut original = vec![
            dvb_si::descriptors::atsc_caption_service::TAG,
            body.len() as u8,
        ];
        original.extend_from_slice(body);

        let mut out = vec![0u8; descriptor.serialized_len()];
        let written = descriptor.serialize_into(&mut out).expect("serialize");
        assert_eq!(&out[..written], original.as_slice());
    }
});
