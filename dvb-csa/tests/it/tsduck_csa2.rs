//! TSDuck CSA2 scrambler oracle: `private/fixtures/crypto/csa2-scrambled-cw0123456789-abcdef.ts`
//! is `csa2-plain.ts` with PID 720 scrambled by `tsp -P scrambler --cw 0123456789ABCDEF`
//! (see that directory's PROVENANCE.md). An independent implementation produced
//! the ciphertext, so agreement here is not a self-consistency check.
//!
//! TSDuck applies DVB-CSA control-word entropy reduction before keying: bytes 3
//! and 7 of `0123456789ABCDEF` are replaced by the checksums of the three
//! preceding bytes (`0x01+0x23+0x45 = 0x69`, `0x89+0xAB+0xCD = 0x201`, so `0x01`, mod 256).
//! The adjusted word below is the one that reproduces the fixture (found
//! empirically; the raw word does not).
use dvb_csa::ControlWord;
use dvb_csa::ts::{KeyParity, descramble_ts_packet, scramble_ts_packet};

const PACKET: usize = 188;
const SCRAMBLED_PID: u16 = 720;
const SCRAMBLED_PACKETS: usize = 3256;
const CW: [u8; 8] = [0x01, 0x23, 0x45, 0x69, 0x89, 0xAB, 0xCD, 0x01];
const TSC_MASK: u8 = 0xC0;
const TSC_EVEN: u8 = 0x80;
const TSC_ODD: u8 = 0xC0;

fn pid(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])
}

fn fixtures() -> Option<(Vec<u8>, Vec<u8>)> {
    let plain = test_fixtures::require_bytes("crypto/csa2-plain.ts", "dvb-csa tsduck_csa2")?;
    let scrambled = test_fixtures::require_bytes(
        "crypto/csa2-scrambled-cw0123456789-abcdef.ts",
        "dvb-csa tsduck_csa2",
    )?;
    Some((plain, scrambled))
}

fn parity_of(packet: &[u8]) -> KeyParity {
    match packet[3] & TSC_MASK {
        TSC_EVEN => KeyParity::Even,
        TSC_ODD => KeyParity::Odd,
        other => panic!("PID 720 packet with transport_scrambling_control bits {other:#04x}"),
    }
}

#[test]
fn descrambling_tsduck_output_recovers_the_plain_stream_byte_exact() {
    let Some((plain, scrambled)) = fixtures() else {
        return;
    };
    assert_eq!(plain.len(), scrambled.len());
    assert_eq!(plain.len() % PACKET, 0);
    let cw = ControlWord(CW);
    let mut descrambled = 0;
    for (i, (p, s)) in plain
        .chunks_exact(PACKET)
        .zip(scrambled.chunks_exact(PACKET))
        .enumerate()
    {
        let mut packet: [u8; PACKET] = s.try_into().unwrap();
        if pid(s) == SCRAMBLED_PID && s[3] & TSC_MASK != 0 {
            let parity = parity_of(s);
            descramble_ts_packet(&cw, parity, &mut packet).unwrap();
            descrambled += 1;
        }
        assert_eq!(&packet[..], p, "packet {i} differs from the plain stream");
    }
    assert_eq!(descrambled, SCRAMBLED_PACKETS);
}

#[test]
fn scrambling_the_plain_stream_reproduces_tsduck_output_byte_exact() {
    let Some((plain, scrambled)) = fixtures() else {
        return;
    };
    let cw = ControlWord(CW);
    let mut scrambled_count = 0;
    for (i, (p, s)) in plain
        .chunks_exact(PACKET)
        .zip(scrambled.chunks_exact(PACKET))
        .enumerate()
    {
        let mut packet: [u8; PACKET] = p.try_into().unwrap();
        if pid(p) == SCRAMBLED_PID && s[3] & TSC_MASK != 0 {
            scramble_ts_packet(&cw, parity_of(s), &mut packet).unwrap();
            scrambled_count += 1;
        }
        assert_eq!(&packet[..], s, "packet {i} differs from TSDuck's output");
    }
    assert_eq!(scrambled_count, SCRAMBLED_PACKETS);
}

#[test]
fn every_other_pid_is_untouched_by_the_oracle() {
    let Some((plain, scrambled)) = fixtures() else {
        return;
    };
    let mut other = 0;
    for (p, s) in plain
        .chunks_exact(PACKET)
        .zip(scrambled.chunks_exact(PACKET))
    {
        if pid(p) != SCRAMBLED_PID {
            assert_eq!(p, s);
            other += 1;
        }
    }
    assert!(other > 0);
}
