//! Test support helpers for `ts-fix` tests.

/// Deterministically corrupt the continuity counter in a TS packet.
///
/// The continuity counter is the 4-bit field in byte 3, bits [3:0].
/// This function zeros all continuity counters in the input stream.
pub fn zero_continuity_counters(data: &mut [u8]) {
    const CC_MASK: u8 = 0x0F;
    for chunk in data.chunks_mut(188) {
        if chunk.len() >= 4 {
            // Byte 3, bits [3:0] is the CC.
            chunk[3] &= !CC_MASK;
        }
    }
}

/// Corrupt the continuity counter in a TS packet using XOR.
///
/// Applies a different XOR value based on packet index (to create varied corruption).
pub fn xor_continuity_counters(data: &mut [u8], pattern: u8) {
    const CC_MASK: u8 = 0x0F;
    for (idx, chunk) in data.chunks_mut(188).enumerate() {
        if chunk.len() >= 4 {
            let xor_val = ((idx as u8) ^ pattern) & CC_MASK;
            chunk[3] = (chunk[3] & !CC_MASK) | ((chunk[3] ^ xor_val) & CC_MASK);
        }
    }
}
