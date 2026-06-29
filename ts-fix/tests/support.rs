//! Test support helpers for `ts-fix` tests.

#![allow(dead_code)]

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

/// Add null packets (PID 0x1FFF) at regular intervals to a stream.
///
/// This creates a synthetic stream with null packets for testing drop/pad operations.
pub fn add_null_packets(data: &[u8], every_n_packets: usize) -> Vec<u8> {
    let mut output = Vec::new();
    for (idx, chunk) in data.chunks(188).enumerate() {
        output.extend_from_slice(chunk);
        if (idx + 1) % every_n_packets == 0 {
            // Insert a null packet after every `every_n_packets` real packets.
            output.extend_from_slice(&make_null_packet());
        }
    }
    output
}

/// Construct a standard null TS packet (PID 0x1FFF, all padding).
fn make_null_packet() -> [u8; 188] {
    let mut pkt = [0xFF_u8; 188];
    pkt[0] = 0x47; // Sync byte
    pkt[1] = 0x1F; // PID upper 5 bits
    pkt[2] = 0xFF; // PID lower 8 bits
    pkt[3] = 0x10; // Adaptation field control = 01, CC = 0
    pkt
}

/// Extract the PID from bytes 1-2 of a TS packet header.
pub fn extract_pid(pkt: &[u8]) -> u16 {
    if pkt.len() < 3 {
        return 0;
    }
    let b1 = pkt[1];
    let b2 = pkt[2];
    (((b1 & 0x1F) as u16) << 8) | (b2 as u16)
}
