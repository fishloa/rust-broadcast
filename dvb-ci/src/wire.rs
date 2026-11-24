//! Big-endian wire-field readers shared by the APDU parsers.
//!
//! Every multi-byte field on the CI wire carries its most-significant byte
//! first, so a field is read by copying its bytes into one sized array rather
//! than indexing them one at a time.

/// Read the big-endian 16-bit field at `offset` of `bytes`.
pub(crate) fn be_u16(bytes: &[u8], offset: usize) -> u16 {
    const FIELD_LEN: usize = core::mem::size_of::<u16>();
    let mut field = [0u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u16::from_be_bytes(field)
}
