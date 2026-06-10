use super::ExtensionDescriptor;
use dvb_common::{Parse, Serialize};

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

pub(crate) fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
