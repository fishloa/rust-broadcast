//! Big-endian wire-field readers shared by the program-stream parsers.
//!
//! A multi-byte field on the wire is read by copying its bytes into one sized
//! array rather than indexing them one at a time.

/// Read the big-endian 16-bit field at `offset` of `bytes`.
pub(crate) fn read_u16_at(bytes: &[u8], offset: usize) -> u16 {
    const FIELD_LEN: usize = core::mem::size_of::<u16>();
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u16::from_be_bytes(field)
}

/// Read the big-endian 32-bit field at `offset` of `bytes`.
pub(crate) fn read_u32_at(bytes: &[u8], offset: usize) -> u32 {
    const FIELD_LEN: usize = core::mem::size_of::<u32>();
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u32::from_be_bytes(field)
}
