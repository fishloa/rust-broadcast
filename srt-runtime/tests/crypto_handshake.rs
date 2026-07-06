//! Integration test: §6.1.5 Key Material Exchange piggybacked on the
//! Caller-Listener handshake (`draft-sharabayko-srt-01` §4.3.1 exchange +
//! §6.1.5, curated at `specs/rules/srt-crypto.md`), issue #621.
//!
//! Drives [`srt_runtime::caller::CallerHandshake`] and
//! [`srt_runtime::listener::ListenerHandshake`] to a full byte-level
//! induction → conclusion exchange (parsing each side's `Send` output the
//! same way a real peer would) with [`srt_runtime::handshake_sm::CryptoConfig`]
//! configured on both sides, and checks the *security property*, not just
//! that the handshake completes:
//!
//! - Same passphrase on both sides: both peers land on
//!   [`srt_runtime::handshake_sm::NegotiatedParams::sek`]/`salt` that are
//!   byte-identical (§6.1.5: "the responder echoes the same KM message back
//!   to prove it derived the same SEK") — verified by having *both* sides
//!   independently AES-CTR encrypt the same known plaintext with their own
//!   negotiated `sek`/`salt` and the same packet sequence number
//!   ([`srt_runtime::crypto::aes_ctr_apply`]) and asserting the ciphertexts
//!   match byte-for-byte, not merely that `sek == sek`.
//! - Different passphrases: the Listener's RFC 3394 unwrap fails (wrong
//!   KEK), the handshake is rejected
//!   ([`srt_runtime::handshake_sm::RejectionReason::BadSecret`]) on *both*
//!   sides (the Listener detects it directly; the Caller learns it from the
//!   Listener's rejection packet), and — independently of the handshake
//!   outcome — the two passphrases are shown to derive different KEKs
//!   from the same Salt, so there is no path by which they could coincide
//!   on the same SEK.

#![cfg(feature = "crypto")]

use srt_runtime::caller::{CallerHandshake, CallerHandshakeState};
use srt_runtime::handshake_sm::{CryptoConfig, HandshakeConfig, HandshakeOutput, RejectionReason};
use srt_runtime::listener::{ListenerHandshake, ListenerHandshakeState};
use srt_runtime::packet::{ControlPacket, EncryptionField};

const PKT_SEQ_NO: u32 = 0x0000_2A2A;

fn config_with_crypto(crypto: CryptoConfig) -> HandshakeConfig {
    HandshakeConfig {
        encryption_field: EncryptionField::Aes128,
        crypto: Some(crypto),
        ..HandshakeConfig::default()
    }
}

/// Drives one full induction → conclusion exchange between a fresh
/// [`CallerHandshake`] and [`ListenerHandshake`], returning every
/// [`HandshakeOutput`] each side ever produced (in call order: caller
/// induction is not an "output" of `feed`, so it is prepended manually by
/// the caller of this helper if needed). Stops as soon as either side
/// reaches a terminal state (`Connected`/`Rejected`/`TimedOut`).
struct ExchangeResult {
    caller_outputs: Vec<HandshakeOutput>,
    listener_outputs: Vec<HandshakeOutput>,
}

fn run_exchange(caller: &mut CallerHandshake, listener: &mut ListenerHandshake) -> ExchangeResult {
    let mut caller_outputs = Vec::new();
    let mut listener_outputs = Vec::new();

    // 1. Caller -> INDUCTION
    let induction = caller.start().unwrap();

    // 2. Listener sees INDUCTION -> INDUCTION response
    let pkt = ControlPacket::parse(&induction).unwrap();
    let outs = listener.feed(&pkt).unwrap();
    listener_outputs.extend(outs.clone());
    let listener_induction_resp = match &outs[0] {
        HandshakeOutput::Send(b) => b.clone(),
        other => panic!("expected Send, got {other:?}"),
    };

    // 3. Caller sees INDUCTION response -> CONCLUSION (offers Key Material).
    let pkt = ControlPacket::parse(&listener_induction_resp).unwrap();
    let outs = caller.feed(&pkt).unwrap();
    caller_outputs.extend(outs.clone());
    let conclusion = match &outs[0] {
        HandshakeOutput::Send(b) => b.clone(),
        other => panic!("expected Send, got {other:?}"),
    };
    if matches!(caller.state(), CallerHandshakeState::Rejected) {
        return ExchangeResult {
            caller_outputs,
            listener_outputs,
        };
    }

    // 4. Listener sees CONCLUSION -> either its own CONCLUSION response +
    //    Connected (echoing Key Material back), or a rejection.
    let pkt = ControlPacket::parse(&conclusion).unwrap();
    let outs = listener.feed(&pkt).unwrap();
    listener_outputs.extend(outs.clone());
    let listener_conclusion_resp = match &outs[0] {
        HandshakeOutput::Send(b) => b.clone(),
        other => panic!("expected Send, got {other:?}"),
    };
    if matches!(listener.state(), ListenerHandshakeState::Rejected) {
        // 5. Feed the Listener's rejection packet back to the Caller so it
        //    also observes the failure.
        let pkt = ControlPacket::parse(&listener_conclusion_resp).unwrap();
        let outs = caller.feed(&pkt).unwrap();
        caller_outputs.extend(outs);
        return ExchangeResult {
            caller_outputs,
            listener_outputs,
        };
    }

    // 6. Caller sees the Listener's CONCLUSION response -> Connected.
    let pkt = ControlPacket::parse(&listener_conclusion_resp).unwrap();
    let outs = caller.feed(&pkt).unwrap();
    caller_outputs.extend(outs);

    ExchangeResult {
        caller_outputs,
        listener_outputs,
    }
}

#[test]
fn same_passphrase_negotiates_identical_sek_verified_by_matching_ciphertext() {
    let passphrase = b"correct horse battery staple".to_vec();
    let salt = [0x11u8; srt_runtime::crypto::SALT_LEN];
    let sek = vec![0x42u8; 16]; // AES-128, matching EncryptionField::Aes128.

    let caller_cfg = config_with_crypto(CryptoConfig {
        passphrase: passphrase.clone(),
        salt,
        sek: sek.clone(),
    });
    let listener_cfg = config_with_crypto(CryptoConfig {
        passphrase,
        salt: [0u8; srt_runtime::crypto::SALT_LEN], // unused on the responder side
        sek: Vec::new(),                            // unused on the responder side
    });

    let mut caller = CallerHandshake::new(0x1111_1111, caller_cfg);
    let mut listener = ListenerHandshake::new(0x2222_2222, 0xC0FF_EE00, listener_cfg);

    run_exchange(&mut caller, &mut listener);

    assert_eq!(caller.state(), CallerHandshakeState::Connected);
    assert_eq!(listener.state(), ListenerHandshakeState::Connected);

    let caller_negotiated = caller.negotiated().unwrap();
    let listener_negotiated = listener.negotiated().unwrap();

    let caller_sek = caller_negotiated
        .sek
        .clone()
        .expect("caller negotiated a SEK");
    let listener_sek = listener_negotiated
        .sek
        .clone()
        .expect("listener negotiated a SEK");
    let caller_salt = caller_negotiated.salt.expect("caller negotiated a Salt");
    let listener_salt = listener_negotiated
        .salt
        .expect("listener negotiated a Salt");

    // The wiring's whole point: not merely "handshake completed", but
    // "completed carrying the SAME SEK/Salt".
    assert_eq!(
        caller_sek, listener_sek,
        "negotiated SEKs must be identical"
    );
    assert_eq!(
        caller_sek, sek,
        "the negotiated SEK is the one the Caller generated"
    );
    assert_eq!(
        caller_salt, listener_salt,
        "negotiated Salts must be identical"
    );
    assert_eq!(caller_salt, salt);

    // Prove it the way the exit gate demands: independently AES-CTR
    // encrypt the same plaintext with each side's own negotiated
    // SEK+Salt+seqno and require byte-identical ciphertext.
    let plaintext = b"SRT payload encryption handshake wiring test.".to_vec();

    let mut caller_ciphertext = plaintext.clone();
    srt_runtime::crypto::aes_ctr_apply(
        &caller_sek,
        &caller_salt,
        PKT_SEQ_NO,
        &mut caller_ciphertext,
    )
    .unwrap();

    let mut listener_ciphertext = plaintext.clone();
    srt_runtime::crypto::aes_ctr_apply(
        &listener_sek,
        &listener_salt,
        PKT_SEQ_NO,
        &mut listener_ciphertext,
    )
    .unwrap();

    assert_eq!(
        caller_ciphertext, listener_ciphertext,
        "both peers' independently-derived SEK+Salt must produce identical ciphertext"
    );
    assert_ne!(
        caller_ciphertext, plaintext,
        "encryption must change the bytes"
    );
}

#[test]
fn different_passphrases_are_rejected_and_never_share_a_sek() {
    let salt = [0x22u8; srt_runtime::crypto::SALT_LEN];
    let sek = vec![0x55u8; 16];

    let caller_cfg = config_with_crypto(CryptoConfig {
        passphrase: b"passphrase A".to_vec(),
        salt,
        sek,
    });
    let listener_cfg = config_with_crypto(CryptoConfig {
        passphrase: b"passphrase B (different)".to_vec(),
        salt: [0u8; srt_runtime::crypto::SALT_LEN],
        sek: Vec::new(),
    });

    let mut caller = CallerHandshake::new(0x3333_3333, caller_cfg);
    let mut listener = ListenerHandshake::new(0x4444_4444, 0xDEAD_BEEF, listener_cfg);

    let result = run_exchange(&mut caller, &mut listener);

    // The Listener detects the RFC 3394 wrap-integrity failure directly
    // (wrong KEK derived from the wrong passphrase) and rejects with
    // `BadSecret`, per `draft-sharabayko-srt-01` §6.1.5's "it does not have
    // the SEK" case.
    assert_eq!(listener.state(), ListenerHandshakeState::Rejected);
    assert!(
        result
            .listener_outputs
            .contains(&HandshakeOutput::Rejected(RejectionReason::BadSecret)),
        "listener must reject with BadSecret, got {:?}",
        result.listener_outputs
    );

    // The Caller learns of the failure from the Listener's rejection packet
    // and never reaches `Connected` — so it never believes it shares a SEK
    // with a peer that could not actually derive one.
    assert_eq!(caller.state(), CallerHandshakeState::Rejected);
    assert!(
        result
            .caller_outputs
            .contains(&HandshakeOutput::Rejected(RejectionReason::BadSecret)),
        "caller must observe the rejection, got {:?}",
        result.caller_outputs
    );
    assert!(caller.negotiated().is_none());
    assert!(listener.negotiated().is_none());

    // Independently of the handshake outcome: assert *inequality*, not just
    // "no panic" — the two passphrases must derive different KEKs from the
    // same Salt, so unwrapping the Caller's wrapped SEK under the Listener's
    // (wrong) KEK cannot coincidentally recover the same key.
    let kek_a = srt_runtime::crypto::derive_kek(b"passphrase A", &salt, 16).unwrap();
    let kek_b = srt_runtime::crypto::derive_kek(b"passphrase B (different)", &salt, 16).unwrap();
    assert_ne!(
        kek_a, kek_b,
        "different passphrases must derive different KEKs"
    );
}
