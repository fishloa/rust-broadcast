//! `no_panic_on_arbitrary_input` — feeds truncated/random bytes to every
//! public packet parser and asserts none of them ever panics. A parse failure
//! is expected and fine (it comes back as `Err`); a panic is not.
//!
//! Uses a small deterministic xorshift PRNG (no external fuzzing dependency
//! needed for this smoke-level gate), seeded from a fixed constant, so the
//! test is reproducible.

use srt_runtime::SrtPacket;
use srt_runtime::packet::{
    ControlPacket, DataPacket, GroupMembershipExtension, HandshakeExtensions, HsExtMessage,
    KeyMaterial, NakPacket,
};

struct XorShift(u64);

impl XorShift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

const ITERATIONS: usize = 20_000;
const MAX_LEN: usize = 96;
/// Bound on how many entries a lazy loop iterator may yield before we treat
/// non-termination as a bug — the loss list / extension list can never
/// legitimately contain more entries than there are bytes.
const MAX_LOOP_ITEMS: usize = MAX_LEN + 4;

#[test]
fn no_panic_on_arbitrary_input() {
    let mut rng = XorShift(0x5EED_C0FF_EE15_5EAF);
    let mut buf = [0u8; MAX_LEN];

    for _ in 0..ITERATIONS {
        let len = (rng.next_u64() as usize) % (MAX_LEN + 1);
        rng.fill(&mut buf[..len]);
        let input = &buf[..len];

        // Top-level dispatcher and every type-specific parser a caller might
        // reach for directly.
        let _ = SrtPacket::parse(input);
        let _ = DataPacket::parse(input);
        let _ = KeyMaterial::parse(input);
        let _ = HsExtMessage::parse(input);
        let _ = GroupMembershipExtension::parse(input);

        // Lazily-walked loops must terminate and never panic on malformed
        // input when constructed directly from raw bytes...
        let nak = NakPacket {
            timestamp: 0,
            dest_socket_id: 0,
            raw_loss_list: input,
        };
        for (i, entry) in nak.entries().enumerate() {
            let _ = entry;
            assert!(
                i <= MAX_LOOP_ITEMS,
                "NAK loss-list iterator did not terminate for {input:?}"
            );
        }

        let exts = HandshakeExtensions(input);
        for (i, block) in exts.iter().enumerate() {
            let _ = block;
            assert!(
                i <= MAX_LOOP_ITEMS,
                "handshake extension iterator did not terminate for {input:?}"
            );
        }

        // ...and when reached through the real dispatcher, which also
        // exercises the per-block decode helpers and the reserved-field
        // validation on the way in.
        if let Ok(ctrl) = ControlPacket::parse(input) {
            match ctrl {
                ControlPacket::Handshake(h) => {
                    for (i, block) in h.extensions.iter().enumerate() {
                        if let Ok(b) = block {
                            let _ = b.as_hs_ext_message();
                            let _ = b.as_key_material();
                            let _ = b.as_stream_id();
                            let _ = b.as_group_membership();
                        }
                        assert!(
                            i <= MAX_LOOP_ITEMS,
                            "handshake extension loop via dispatch did not terminate"
                        );
                    }
                }
                ControlPacket::Nak(n) => {
                    for (i, entry) in n.entries().enumerate() {
                        let _ = entry;
                        assert!(
                            i <= MAX_LOOP_ITEMS,
                            "NAK loop via dispatch did not terminate"
                        );
                    }
                }
                ControlPacket::UserDefined(u) => {
                    let _ = u.as_key_material();
                }
                _ => {}
            }
        }
    }
}
