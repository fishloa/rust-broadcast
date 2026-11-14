//! Issue #2652: a `PcrSample` carries the wire `mpeg_ts::Pcr`, not an arbitrary
//! 27 MHz integer.
//!
//! A PCR field is a 33-bit base plus a **9-bit** extension (ISO/IEC 13818-1
//! §2.4.3.5), so a capture can carry an extension in `300..=511` — a value off
//! the 27 MHz clock that `Pcr::from_parts` refuses. Parsing stays lossless (the
//! byte-exact round-trip invariant) and the recovered timeline keeps that
//! extension exactly; a flat 27 MHz integer could not name such a value at all.

use broadcast_common::Unpackage;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{
    AdaptationField, AdaptationFieldControl, ContinuityCounter, Pcr, ScramblingControl, TsHeader,
};
use mpeg_ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};
use transmux::TsDemux;

/// The TS header length.
const TS_HEADER_LEN: usize = 4;
/// Bytes of the encoded PCR/OPCR field (33-bit base, 6 reserved bits, 9-bit
/// extension).
const PCR_FIELD_LEN: usize = 6;
/// The two trailing bytes of the PCR field that carry the 9-bit extension: its
/// high bit inside the first and its low 8 bits in the second.
const PCR_FIELD_EXT_BYTES: usize = 2;
/// The 9-bit extension's top bit inside the first extension byte (bit 0; bits
/// 6-1 are the reserved bits and bit 7 is the base's last bit).
const PCR_EXT_HIGH_BIT: u8 = 0x01;
/// `PCR_base` of the synthetic observation (90 kHz units).
const BASE: u64 = 1_000_000;
/// The PCR extension modulus: 27 MHz / 90 kHz.
const PCR_EXT_MODULUS: u64 = 300;
/// `PCR_ext` past the 27 MHz clock's own modulus (300), inside the 9-bit field.
const OFF_CLOCK_EXT: u16 = 511;
/// The PCR PID of the synthetic packet.
const PCR_PID: u16 = 0x0100;

fn pcr_packet(pcr: Pcr, field_bytes: [u8; PCR_FIELD_LEN]) -> Vec<u8> {
    let mut af = AdaptationField::new();
    af.pcr = Some(pcr);
    let body_len = af.serialized_len();
    let mut packet = vec![0u8; TS_PACKET_SIZE];
    packet[0] = TS_SYNC_BYTE;
    let header = TsHeader {
        tei: false,
        pusi: false,
        pid: Pid::from_const(PCR_PID),
        scrambling: ScramblingControl::NotScrambled,
        adaptation_field_control: AdaptationFieldControl::AdaptationAndPayload,
        continuity_counter: ContinuityCounter::ZERO,
    };
    header
        .serialize_into(&mut packet[0..TS_HEADER_LEN])
        .expect("a 4-byte TS header fits");
    // Byte 4 is the `adaptation_field_length`; the adaptation-field body (flags
    // byte first) follows immediately, and the PCR field right after the flags.
    packet[TS_HEADER_LEN] = u8::try_from(body_len).expect("an adaptation field is one length byte");
    af.serialize_into(&mut packet[TS_HEADER_LEN + 1..TS_HEADER_LEN + 1 + body_len])
        .expect("the adaptation field fits the rest of the packet");
    let field_start = packet
        .windows(PCR_FIELD_LEN)
        .position(|w| w == pcr.to_field_bytes());
    let start = field_start.expect("the packet carries the encoded PCR field");
    packet[start..start + PCR_FIELD_LEN].copy_from_slice(&field_bytes);
    packet
}

fn demux_one(packet: &[u8]) -> transmux::PcrSample {
    let media = TsDemux::new()
        .unpackage(packet)
        .expect("an adaptation-field PCR packet demuxes");
    *media.pcr.first().expect("the packet carries a PCR")
}

#[test]
fn a_pcr_sample_carries_the_wire_pcr_and_no_arbitrary_integer() {
    let on_clock_pcr = Pcr::from_parts(BASE, 0).expect("ext 0 is on the 27 MHz clock");
    let on_clock = demux_one(&pcr_packet(on_clock_pcr, on_clock_pcr.to_field_bytes()));
    assert_eq!(on_clock.pcr, on_clock_pcr);
    assert_eq!(on_clock.pcr.base(), BASE);
    assert_eq!(
        on_clock.pcr_27mhz(),
        Some(BASE * PCR_EXT_MODULUS),
        "an on-clock PCR reads straight back as its 27 MHz value"
    );
    assert_eq!(
        on_clock.pid,
        Pid::from_const(PCR_PID),
        "the observed PID is kept"
    );

    // The same field with the wire extension's 9 bits set to 511: an off-clock
    // value `Pcr::from_parts` cannot build, which only a lossless parse keeps.
    let mut field_bytes = on_clock_pcr.to_field_bytes();
    let ext_bytes = PCR_FIELD_LEN - PCR_FIELD_EXT_BYTES;
    field_bytes[ext_bytes] |= PCR_EXT_HIGH_BIT;
    field_bytes[ext_bytes + 1] = u8::try_from(OFF_CLOCK_EXT & u16::from(u8::MAX))
        .expect("the extension's low byte is the low 8 bits of a 9-bit field");

    let pkt = pcr_packet(on_clock_pcr, field_bytes);

    let sample = demux_one(&pkt);
    assert_eq!(
        sample.pcr.extension(),
        OFF_CLOCK_EXT,
        "the wire 9-bit extension survives into the IR sample"
    );
    assert_eq!(sample.pcr.base(), BASE, "the 33-bit base is unchanged");
    assert!(
        sample.pcr.to_clock27().is_err(),
        "an extension in 300..=511 is off the 27 MHz clock"
    );
    assert_eq!(
        sample.pcr_27mhz(),
        None,
        "an off-clock extension has no 27 MHz reading, so the accessor cannot \
         hand back a wrapped or truncated integer"
    );
}
