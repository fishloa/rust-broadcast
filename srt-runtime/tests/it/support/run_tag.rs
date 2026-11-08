const TAG_LEN: usize = 4;

/// A tag no other test in this or a concurrently running copy of the binary
/// shares, stamped into every datagram a test feeds so it can tell its own
/// data from a stray.
fn unique_run_tag() -> u32 {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::process::id().rotate_left(8) ^ seq
}

fn tagged_chunk(tag: u32, index: u32, len: usize) -> Vec<u8> {
    let mut c = vec![0x5A; len];
    c[..TAG_LEN].copy_from_slice(&tag.to_be_bytes());
    c[TAG_LEN..2 * TAG_LEN].copy_from_slice(&index.to_be_bytes());
    c
}

fn chunk_tag_matches(payload: &[u8], tag: u32) -> bool {
    payload.len() >= TAG_LEN && payload[..TAG_LEN] == tag.to_be_bytes()
}
