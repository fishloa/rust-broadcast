//! `no_panic_on_arbitrary_input` — feeds truncated/random bytes to every
//! public packet parser and asserts none of them ever panics. A parse failure
//! is expected and fine (it comes back as `Err`); a panic is not.
//!
//! Uses a small deterministic xorshift PRNG (no external fuzzing dependency
//! needed for this smoke-level gate), seeded from a fixed constant, so the
//! test is reproducible.

use core::time::Duration;

use srt_runtime::arq::{Receiver, Sender};
use srt_runtime::caller::CallerHandshake;
use srt_runtime::listener::ListenerHandshake;
use srt_runtime::packet::{
    AckCif, AckPacket, ControlPacket, DataPacket, GroupMembershipExtension, HandshakeExtensions,
    HsExtMessage, KeyMaterial, NakPacket,
};
use srt_runtime::rendezvous::RendezvousHandshake;
use srt_runtime::tsbpd::TsbpdScheduler;
use srt_runtime::{HandshakeConfig, SrtPacket};

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
            match &ctrl {
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

            // Feed the (possibly malformed) parsed packet into fresh handshake
            // state machines at every state that accepts inbound handshake
            // packets. `Err` (or `Rejected`) is expected and fine; a panic is
            // not — this is the SM-level analogue of the parser fuzz above.
            let mut caller_awaiting_induction = CallerHandshake::new(1, HandshakeConfig::default());
            caller_awaiting_induction.start().unwrap();
            let _ = caller_awaiting_induction.feed(&ctrl);

            let mut caller_awaiting_conclusion =
                CallerHandshake::new(1, HandshakeConfig::default());
            caller_awaiting_conclusion.start().unwrap();
            let _ = caller_awaiting_conclusion.feed(&good_induction_response(2));
            let _ = caller_awaiting_conclusion.feed(&ctrl);

            let mut listener_idle =
                ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
            let _ = listener_idle.feed(&ctrl);

            let mut listener_awaiting_conclusion =
                ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
            let _ = listener_awaiting_conclusion.feed(&good_induction_request(2));
            let _ = listener_awaiting_conclusion.feed(&ctrl);

            // Rendezvous engine (§4.3.2): fuzz every reachable state —
            // Waving (fresh, never even started against real input),
            // Attention (after a real WAVEAHAND), and Initiated (after a
            // real WAVEAHAND + a real role-appropriate CONCLUSION).
            let mut rdv_waving =
                RendezvousHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
            rdv_waving.start().unwrap();
            let _ = rdv_waving.feed(&ctrl);

            let mut rdv_attention =
                RendezvousHandshake::new(1, 0xFFFF_FFFF, HandshakeConfig::default());
            rdv_attention.start().unwrap();
            let _ = rdv_attention.feed(&good_peer_wavehand(2, 0));
            let _ = rdv_attention.feed(&ctrl);

            let mut rdv_initiated =
                RendezvousHandshake::new(1, 0xFFFF_FFFF, HandshakeConfig::default());
            rdv_initiated.start().unwrap();
            let _ = rdv_initiated.feed(&good_peer_wavehand(2, 0));
            let _ = rdv_initiated.feed(&good_peer_empty_conclusion(2, 0));
            let _ = rdv_initiated.feed(&ctrl);
        }
    }
}

/// Hostile input to the stateful engines (as opposed to the parsers above):
/// arbitrary sequence numbers, timestamps and clocks, huge or malformed NAK
/// loss lists, ACKs for sequence numbers never sent, and handshake packets
/// after the handshake finished. Nothing may panic, and the receiver's loss
/// tracking must stay inside the flow window however far a packet jumps.
#[test]
fn no_panic_and_bounded_state_on_hostile_engine_input() {
    const WINDOW: u32 = 8192;
    let mut rng = XorShift(0xBAD5_EED5_0FF1_CE01);
    let mut now = Duration::ZERO;

    // --- ARQ receiver: any sequence number, any (non-monotonic) clock. ---
    let mut receiver = Receiver::new(0xAAAA, 77, WINDOW);
    for _ in 0..ITERATIONS {
        now += Duration::from_micros(rng.next_u64() % 40_000);
        let seq = match rng.next_u64() % 4 {
            0 => (rng.next_u64() as u32) & 0x7FFF_FFFF, // anywhere in the space
            1 => rng.next_u64() as u32,                 // not even 31 bits
            2 => 77u32.wrapping_add((rng.next_u64() % 64) as u32), // near the cursor
            _ => 77u32
                .wrapping_add(WINDOW)
                .wrapping_add((rng.next_u64() % 3) as u32), // at the edge
        };
        let _ = receiver.feed_data(seq, now);
        if rng.next_u64().is_multiple_of(8) {
            for datagram in receiver.tick(now) {
                // Every datagram it emits must be a well-formed control packet.
                assert!(ControlPacket::parse(&datagram).is_ok());
            }
        }
        assert!(
            receiver.loss_list_len() <= WINDOW as usize + 1,
            "loss list grew past the flow window to {}",
            receiver.loss_list_len()
        );
    }

    // --- ARQ sender: random NAK bytes and ACKs for sequence numbers it never sent. ---
    let mut sender = Sender::new(0xBBBB);
    const SENT: u32 = 200;
    for seq in 0..SENT {
        sender
            .on_data(seq, seq, b"payload", Duration::ZERO)
            .expect("in-range numbers");
    }
    let mut raw = [0u8; MAX_LEN];
    for _ in 0..ITERATIONS {
        let len = (rng.next_u64() as usize) % (MAX_LEN + 1);
        rng.fill(&mut raw[..len]);
        sender.on_nak(&NakPacket {
            timestamp: 0,
            dest_socket_id: 0,
            raw_loss_list: &raw[..len],
        });
        if rng.next_u64().is_multiple_of(16) {
            let _ = sender.tick(Duration::from_millis(1));
        }
        if rng.next_u64().is_multiple_of(64) {
            let cif = AckCif::Light {
                last_ack_seq: (rng.next_u64() as u32) & 0x7FFF_FFFF,
            };
            let _ = sender.on_ack(
                &AckPacket {
                    ack_number: 0,
                    timestamp: 0,
                    dest_socket_id: 0,
                    cif,
                },
                Duration::ZERO,
            );
        }
        assert!(sender.buffered_count() <= SENT as usize);
        assert!(sender.pending_retransmit_count() <= SENT as usize);
    }

    // --- TSBPD: arbitrary sequence numbers and timestamps (including wraps). ---
    let mut tsbpd = TsbpdScheduler::new(0, -1_000, 120, 0, true, None);
    let mut clock = Duration::ZERO;
    for _ in 0..ITERATIONS {
        clock += Duration::from_micros(rng.next_u64() % 5_000);
        let seq = (rng.next_u64() as u32) & 0x7FFF_FFFF;
        let _ = tsbpd.feed_data(seq, rng.next_u64() as u32, clock);
        let _ = tsbpd.tick(clock);
    }

    // --- A finished handshake keeps receiving handshake-shaped garbage. ---
    let mut connected = ListenerHandshake::new(1, 0xC0FF_EE00, HandshakeConfig::default());
    let mut caller = CallerHandshake::new(2, HandshakeConfig::default());
    let send = |outputs: Vec<srt_runtime::HandshakeOutput>| {
        outputs
            .into_iter()
            .find_map(|o| match o {
                srt_runtime::HandshakeOutput::Send(b) => Some(b),
                _ => None,
            })
            .expect("a datagram to send")
    };
    let induction = caller.start().unwrap();
    let induction_response = send(connected.feed_bytes(&induction).unwrap());
    let conclusion = send(caller.feed_bytes(&induction_response).unwrap());
    connected.feed_bytes(&conclusion).unwrap();
    assert_eq!(
        connected.state(),
        srt_runtime::ListenerHandshakeState::Connected
    );
    let mut buf = [0u8; MAX_LEN];
    for _ in 0..ITERATIONS {
        let len = (rng.next_u64() as usize) % (MAX_LEN + 1);
        rng.fill(&mut buf[..len]);
        if let Ok(ctrl) = ControlPacket::parse(&buf[..len]) {
            let _ = connected.feed(&ctrl);
        }
    }
    // However much garbage it was fed, it is still connected (or has at worst
    // answered a repeat of the connected peer's CONCLUSION).
    assert_eq!(
        connected.state(),
        srt_runtime::ListenerHandshakeState::Connected
    );
}

/// A well-formed peer WAVEAHAND (§4.3.2, L2156-2165) with a cookie guaranteed
/// lower than the fuzz-target's own (`0xFFFF_FFFF`), so the fuzz-target
/// always resolves to Initiator — used only to advance a
/// [`RendezvousHandshake`] fuzz target past `Waving` before feeding it fuzzed
/// bytes.
fn good_peer_wavehand(peer_socket_id: u32, peer_cookie: u32) -> ControlPacket<'static> {
    use srt_runtime::packet::{
        EncryptionField, HandshakeExtensionFlags, HandshakePacket, HandshakeType,
    };
    ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 0,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Wavehand,
        srt_socket_id: peer_socket_id,
        syn_cookie: peer_cookie,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    })
}

/// A well-formed peer CONCLUSION with no extensions — used only to advance an
/// Initiator [`RendezvousHandshake`] fuzz target from `Attention` to
/// `Initiated` (§4.3.2.2, L2312-2318) before feeding it fuzzed bytes.
fn good_peer_empty_conclusion(peer_socket_id: u32, peer_cookie: u32) -> ControlPacket<'static> {
    use srt_runtime::packet::{
        EncryptionField, HandshakeExtensionFlags, HandshakePacket, HandshakeType,
    };
    ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 1,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Conclusion,
        srt_socket_id: peer_socket_id,
        syn_cookie: peer_cookie,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    })
}

/// A well-formed Caller INDUCTION, used only to advance a fuzz-target
/// [`ListenerHandshake`] to `AwaitingConclusion` before feeding it fuzzed
/// bytes.
fn good_induction_request(caller_socket_id: u32) -> ControlPacket<'static> {
    use srt_runtime::packet::{
        EncryptionField, HandshakeExtensionFlags, HandshakePacket, HandshakeType,
    };
    ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 0,
        version: 4,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(2),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Induction,
        srt_socket_id: caller_socket_id,
        syn_cookie: 0,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    })
}

/// A well-formed Listener INDUCTION response, used only to advance a
/// fuzz-target [`CallerHandshake`] to `AwaitingConclusionResponse` before
/// feeding it fuzzed bytes.
fn good_induction_response(listener_socket_id: u32) -> ControlPacket<'static> {
    use srt_runtime::packet::{
        EncryptionField, HandshakeExtensionFlags, HandshakePacket, HandshakeType,
    };
    ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: 1,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0x4A17),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: HandshakeType::Induction,
        srt_socket_id: listener_socket_id,
        syn_cookie: 0xC0FF_EE00,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    })
}
