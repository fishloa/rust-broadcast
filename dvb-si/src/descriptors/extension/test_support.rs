use super::ExtensionDescriptor;
use broadcast_common::{Parse, Serialize};

pub(crate) fn wrap(tag_ext: u8, sel: &[u8]) -> Vec<u8> {
    let mut v = vec![super::TAG, (sel.len() + 1) as u8, tag_ext];
    v.extend_from_slice(sel);
    v
}

pub(crate) fn round_trip(d: &ExtensionDescriptor) {
    let mut buf = vec![0u8; d.serialized_len()];
    d.serialize_into(&mut buf).unwrap();
    let re = ExtensionDescriptor::parse(&buf).unwrap();
    assert_eq!(*d, re);
}
