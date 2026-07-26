//! RTMP handshake — C0/C1/C2 and S0/S1/S2 (Adobe RTMP 1.0 §5.2).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §2 (Handshake) for the wire layout:
//! C0/S0 (§5.2.2), C1/S1 (§5.2.3), C2/S2 (§5.2.4), and the handshake sequence
//! diagram (§5.2.5).
//!
//! # Scope: simple handshake only
//!
//! This module implements only the **simple** (plain) handshake described by
//! §5.2 itself: C0/C1/C2 and S0/S1/S2 carry no HMAC-SHA256 digest and no
//! "complex handshake" key-exchange scheme (that scheme is an Adobe Flash
//! Media Server addition, not part of the RTMP 1.0 spec text transcribed in
//! `docs/rtmp.md`). This is sufficient for interoperating with real-world
//! publishers such as ffmpeg and OBS Studio, which fall back to (or always
//! use) the simple handshake for `rtmp://` publish. Complex-handshake support
//! is out of scope for this crate.
//!
//! # No wall clock in the sans-IO core
//!
//! This crate has no socket or clock of its own (see the crate-root sans-IO
//! contract doc). `time` in S1 and `time2` in S2 are therefore not read from
//! a real clock: [`Handshake::new`] uses `0` for both and a fixed,
//! non-cryptographic filler pattern for S1's random bytes (see
//! [`default_random_fill`]); [`Handshake::with_time_and_random`] lets a
//! caller supply real values instead. Per §5.2.3/§5.2.4 neither field is
//! required to be meaningful (the spec itself calls the bandwidth estimate
//! they enable "unlikely to be useful"), and no `rand`-style dependency is
//! pulled in to generate them.

use broadcast_common::{Parse, Serialize};

use crate::RtmpError;

type Result<T> = core::result::Result<T, RtmpError>;

/// RTMP version this handshake implements/advertises (§5.2.2): `3`.
pub const RTMP_VERSION: u8 = 3;

/// Wire length in bytes of C1/S1 and C2/S2 (§5.2.3/§5.2.4): `1536`.
pub const HANDSHAKE_PACKET_LEN: usize = 1536;

/// Wire length in bytes of C0/S0 (§5.2.2): `1`.
const VERSION_LEN: usize = 1;
/// Byte width of the `time`/`zero`/`time2` fields.
const FIELD_LEN: usize = 4;
/// Byte width of the `random bytes`/`random echo` field:
/// `HANDSHAKE_PACKET_LEN` minus two 4-byte fields.
const RANDOM_LEN: usize = HANDSHAKE_PACKET_LEN - FIELD_LEN - FIELD_LEN;

/// C0 (client→server) or S0 (server→client): the 1-byte RTMP version
/// (§5.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version(pub u8);

impl<'a> Parse<'a> for Version {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < VERSION_LEN {
            return Err(RtmpError::BufferTooShort {
                need: VERSION_LEN,
                have: bytes.len(),
                what: "C0/S0 version",
            });
        }
        Ok(Version(bytes[0]))
    }
}

impl Serialize for Version {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        VERSION_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < VERSION_LEN {
            return Err(RtmpError::BufferTooShort {
                need: VERSION_LEN,
                have: buf.len(),
                what: "C0/S0 version output",
            });
        }
        buf[0] = self.0;
        Ok(VERSION_LEN)
    }
}

/// C1 (client→server) or S1 (server→client): the 1536-byte time/zero/random
/// handshake packet (§5.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakePacket {
    /// `time` — timestamp epoch for this endpoint's future chunks. May be 0
    /// or arbitrary (§5.2.3).
    pub time: u32,
    /// `zero` — MUST be all zeros on the wire (§5.2.3).
    pub zero: u32,
    /// `random bytes` — arbitrary data distinguishing this handshake from
    /// the peer's; no cryptographic randomness required (§5.2.3).
    pub random: [u8; RANDOM_LEN],
}

impl<'a> Parse<'a> for HandshakePacket {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HANDSHAKE_PACKET_LEN {
            return Err(RtmpError::BufferTooShort {
                need: HANDSHAKE_PACKET_LEN,
                have: bytes.len(),
                what: "C1/S1",
            });
        }
        let time = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let zero = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mut random = [0u8; RANDOM_LEN];
        random.copy_from_slice(&bytes[2 * FIELD_LEN..HANDSHAKE_PACKET_LEN]);
        Ok(HandshakePacket { time, zero, random })
    }
}

impl Serialize for HandshakePacket {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        HANDSHAKE_PACKET_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < HANDSHAKE_PACKET_LEN {
            return Err(RtmpError::BufferTooShort {
                need: HANDSHAKE_PACKET_LEN,
                have: buf.len(),
                what: "C1/S1 output",
            });
        }
        buf[0..FIELD_LEN].copy_from_slice(&self.time.to_be_bytes());
        buf[FIELD_LEN..2 * FIELD_LEN].copy_from_slice(&self.zero.to_be_bytes());
        buf[2 * FIELD_LEN..HANDSHAKE_PACKET_LEN].copy_from_slice(&self.random);
        Ok(HANDSHAKE_PACKET_LEN)
    }
}

/// C2 (client→server) or S2 (server→client): the 1536-byte near-echo packet
/// (§5.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EchoPacket {
    /// `time` — MUST equal the peer's S1 `time` (for C2) or C1 `time` (for
    /// S2) (§5.2.4).
    pub time: u32,
    /// `time2` — MUST be the timestamp at which the peer's previous packet
    /// (S1 or C1) was read (§5.2.4).
    pub time2: u32,
    /// `random echo` — MUST equal the peer's S1/C1 `random bytes`, verbatim
    /// (§5.2.4).
    pub random_echo: [u8; RANDOM_LEN],
}

impl<'a> Parse<'a> for EchoPacket {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HANDSHAKE_PACKET_LEN {
            return Err(RtmpError::BufferTooShort {
                need: HANDSHAKE_PACKET_LEN,
                have: bytes.len(),
                what: "C2/S2",
            });
        }
        let time = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let time2 = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mut random_echo = [0u8; RANDOM_LEN];
        random_echo.copy_from_slice(&bytes[2 * FIELD_LEN..HANDSHAKE_PACKET_LEN]);
        Ok(EchoPacket {
            time,
            time2,
            random_echo,
        })
    }
}

impl Serialize for EchoPacket {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        HANDSHAKE_PACKET_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < HANDSHAKE_PACKET_LEN {
            return Err(RtmpError::BufferTooShort {
                need: HANDSHAKE_PACKET_LEN,
                have: buf.len(),
                what: "C2/S2 output",
            });
        }
        buf[0..FIELD_LEN].copy_from_slice(&self.time.to_be_bytes());
        buf[FIELD_LEN..2 * FIELD_LEN].copy_from_slice(&self.time2.to_be_bytes());
        buf[2 * FIELD_LEN..HANDSHAKE_PACKET_LEN].copy_from_slice(&self.random_echo);
        Ok(HANDSHAKE_PACKET_LEN)
    }
}

/// A fixed, non-cryptographic 1528-byte fill pattern for a server's own S1
/// `random bytes` when the caller has no specific bytes to supply. Per
/// §5.2.3 the field only needs to "distinguish this handshake from the
/// peer's" — no cryptographic randomness is required, so a deterministic
/// repeating byte pattern is spec-conformant and keeps this crate free of a
/// `rand`-style dependency.
#[must_use]
pub fn default_random_fill() -> [u8; RANDOM_LEN] {
    let mut random = [0u8; RANDOM_LEN];
    for (i, b) in random.iter_mut().enumerate() {
        *b = (i & 0xFF) as u8;
    }
    random
}

/// Server-side handshake driver state (§5.2.5's Uninitialized → Version Sent
/// → Ack Sent → Handshake Done, collapsed to the three states this driver
/// actually distinguishes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandshakeState {
    /// Awaiting C0+C1 from the client (spec's "Uninitialized").
    WaitC0C1,
    /// S0+S1+S2 sent; awaiting C2 from the client (spec's "Version
    /// Sent"/"Ack Sent" collapsed into one wait).
    WaitC2,
    /// C2 received; handshake complete (spec's "Handshake Done").
    Done,
}

/// Sans-IO server-side RTMP handshake driver (§5.2, simple handshake only —
/// see the module doc).
///
/// Drive it by feeding inbound bytes to [`read`](Self::read); it returns any
/// outbound reply bytes, how many input bytes it consumed, and whether the
/// handshake is now complete. It never touches a socket or clock itself.
#[derive(Debug)]
pub struct Handshake {
    state: HandshakeState,
    /// Written into our own S1 `time` field.
    local_time: u32,
    /// Written into our own S1 `random bytes` field.
    local_random: [u8; RANDOM_LEN],
    /// Written into our own S2 `time2` field (see the module doc: this core
    /// has no wall clock, so this is caller-supplied or `0`, not a real
    /// read-timestamp).
    read_time: u32,
}

impl Default for Handshake {
    fn default() -> Self {
        Self::new()
    }
}

impl Handshake {
    /// A new server-side handshake in the initial (`WaitC0C1`) state, using
    /// `0` for S1's `time`/S2's `time2` and [`default_random_fill`] for S1's
    /// random bytes. Use [`with_time_and_random`](Self::with_time_and_random)
    /// to supply real values instead.
    #[must_use]
    pub fn new() -> Self {
        Self::with_time_and_random(0, default_random_fill(), 0)
    }

    /// A new server-side handshake with caller-supplied S1 `time`, S1
    /// `random bytes`, and S2 `time2` values.
    #[must_use]
    pub fn with_time_and_random(
        local_time: u32,
        local_random: [u8; RANDOM_LEN],
        read_time: u32,
    ) -> Self {
        Self {
            state: HandshakeState::WaitC0C1,
            local_time,
            local_random,
            read_time,
        }
    }

    /// True once C2 has been received and the handshake is complete.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.state == HandshakeState::Done
    }

    /// Feed inbound bytes to the handshake driver.
    ///
    /// - In `WaitC0C1`, requires a full C0(1)+C1(1536) = 1537 bytes at the
    ///   front of `input`; on success returns the S0+S1+S2 reply
    ///   (1+1536+1536 = 3073 bytes), consumes 1537 input bytes, and advances
    ///   to `WaitC2`.
    /// - In `WaitC2`, requires a full C2(1536) bytes; on success returns no
    ///   reply bytes, consumes 1536 input bytes, and advances to `Done`.
    /// - In `Done`, is a no-op: returns no reply, consumes 0 bytes, and
    ///   reports done.
    ///
    /// Returns `(reply_bytes, consumed, done)`.
    ///
    /// # Errors
    /// [`RtmpError::BufferTooShort`] if `input` does not yet hold a full
    /// C0+C1 (`WaitC0C1`) or C2 (`WaitC2`). This driver does not buffer
    /// partial input itself — callers should re-invoke `read` once more
    /// bytes have arrived.
    pub fn read(&mut self, input: &[u8]) -> Result<(Vec<u8>, usize, bool)> {
        match self.state {
            HandshakeState::WaitC0C1 => {
                let need = VERSION_LEN + HANDSHAKE_PACKET_LEN;
                if input.len() < need {
                    return Err(RtmpError::BufferTooShort {
                        need,
                        have: input.len(),
                        what: "C0+C1",
                    });
                }
                let _c0 = Version::parse(&input[..VERSION_LEN])?;
                let c1 = HandshakePacket::parse(&input[VERSION_LEN..need])?;

                let s0 = Version(RTMP_VERSION);
                let s1 = HandshakePacket {
                    time: self.local_time,
                    zero: 0,
                    random: self.local_random,
                };
                let s2 = EchoPacket {
                    time: c1.time,
                    time2: self.read_time,
                    random_echo: c1.random,
                };

                let reply_len = VERSION_LEN + HANDSHAKE_PACKET_LEN + HANDSHAKE_PACKET_LEN;
                let mut reply = vec![0u8; reply_len];
                s0.serialize_into(&mut reply[..VERSION_LEN])?;
                s1.serialize_into(&mut reply[VERSION_LEN..VERSION_LEN + HANDSHAKE_PACKET_LEN])?;
                s2.serialize_into(&mut reply[VERSION_LEN + HANDSHAKE_PACKET_LEN..])?;

                self.state = HandshakeState::WaitC2;
                Ok((reply, need, false))
            }
            HandshakeState::WaitC2 => {
                if input.len() < HANDSHAKE_PACKET_LEN {
                    return Err(RtmpError::BufferTooShort {
                        need: HANDSHAKE_PACKET_LEN,
                        have: input.len(),
                        what: "C2",
                    });
                }
                let _c2 = EchoPacket::parse(&input[..HANDSHAKE_PACKET_LEN])?;
                self.state = HandshakeState::Done;
                Ok((Vec::new(), HANDSHAKE_PACKET_LEN, true))
            }
            HandshakeState::Done => Ok((Vec::new(), 0, true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterned_random(seed: u8) -> [u8; RANDOM_LEN] {
        let mut r = [0u8; RANDOM_LEN];
        for (i, b) in r.iter_mut().enumerate() {
            *b = seed.wrapping_add((i & 0xFF) as u8);
        }
        r
    }

    // ── C0/S0 round-trip ─────────────────────────────────────────────────

    #[test]
    fn version_round_trip_build_serialize_parse() {
        let v = Version(RTMP_VERSION);
        let mut buf = [0u8; VERSION_LEN];
        let n = v.serialize_into(&mut buf).unwrap();
        assert_eq!(n, VERSION_LEN);
        let parsed = Version::parse(&buf).unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn version_round_trip_parse_serialize_byte_identical() {
        let bytes = [3u8];
        let v = Version::parse(&bytes).unwrap();
        let mut buf = [0u8; VERSION_LEN];
        v.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn version_short_input_is_buffer_too_short() {
        let bytes: [u8; 0] = [];
        assert!(matches!(
            Version::parse(&bytes),
            Err(RtmpError::BufferTooShort {
                need: VERSION_LEN,
                have: 0,
                ..
            })
        ));
    }

    // ── C1/S1 round-trip ─────────────────────────────────────────────────

    #[test]
    fn handshake_packet_round_trip_build_serialize_parse() {
        let hp = HandshakePacket {
            time: 0x1122_3344,
            zero: 0,
            random: patterned_random(0xAB),
        };
        let mut buf = [0u8; HANDSHAKE_PACKET_LEN];
        let n = hp.serialize_into(&mut buf).unwrap();
        assert_eq!(n, HANDSHAKE_PACKET_LEN);
        let parsed = HandshakePacket::parse(&buf).unwrap();
        assert_eq!(parsed, hp);
    }

    #[test]
    fn handshake_packet_round_trip_parse_serialize_byte_identical() {
        let mut bytes = [0u8; HANDSHAKE_PACKET_LEN];
        bytes[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        // bytes[4..8] left as the required-zero `zero` field.
        for (i, b) in bytes[8..].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7);
        }
        let hp = HandshakePacket::parse(&bytes).unwrap();
        assert_eq!(hp.time, 0xDEAD_BEEF);
        assert_eq!(hp.zero, 0);
        let mut buf = [0u8; HANDSHAKE_PACKET_LEN];
        hp.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "C1/S1 byte-identical round trip");
    }

    #[test]
    fn handshake_packet_short_input_is_buffer_too_short() {
        let bytes = [0u8; HANDSHAKE_PACKET_LEN - 1];
        assert!(matches!(
            HandshakePacket::parse(&bytes),
            Err(RtmpError::BufferTooShort {
                need: HANDSHAKE_PACKET_LEN,
                have,
                ..
            }) if have == HANDSHAKE_PACKET_LEN - 1
        ));
    }

    // ── C2/S2 round-trip ─────────────────────────────────────────────────

    #[test]
    fn echo_packet_round_trip_build_serialize_parse() {
        let ep = EchoPacket {
            time: 0x0102_0304,
            time2: 0x0506_0708,
            random_echo: patterned_random(0x5A),
        };
        let mut buf = [0u8; HANDSHAKE_PACKET_LEN];
        let n = ep.serialize_into(&mut buf).unwrap();
        assert_eq!(n, HANDSHAKE_PACKET_LEN);
        let parsed = EchoPacket::parse(&buf).unwrap();
        assert_eq!(parsed, ep);
    }

    #[test]
    fn echo_packet_round_trip_parse_serialize_byte_identical() {
        let mut bytes = [0u8; HANDSHAKE_PACKET_LEN];
        bytes[0..4].copy_from_slice(&0x1111_2222u32.to_be_bytes());
        bytes[4..8].copy_from_slice(&0x3333_4444u32.to_be_bytes());
        for (i, b) in bytes[8..].iter_mut().enumerate() {
            *b = (i as u8) ^ 0x5A;
        }
        let ep = EchoPacket::parse(&bytes).unwrap();
        let mut buf = [0u8; HANDSHAKE_PACKET_LEN];
        ep.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "C2/S2 byte-identical round trip");
    }

    #[test]
    fn echo_packet_short_input_is_buffer_too_short() {
        let bytes = [0u8; 10];
        assert!(matches!(
            EchoPacket::parse(&bytes),
            Err(RtmpError::BufferTooShort {
                need: HANDSHAKE_PACKET_LEN,
                have: 10,
                ..
            })
        ));
    }

    // ── FSM happy path ────────────────────────────────────────────────────

    fn build_c0_c1(client_time: u32, client_random: [u8; RANDOM_LEN]) -> Vec<u8> {
        let mut v = vec![0u8; VERSION_LEN + HANDSHAKE_PACKET_LEN];
        v[0] = RTMP_VERSION;
        let c1 = HandshakePacket {
            time: client_time,
            zero: 0,
            random: client_random,
        };
        c1.serialize_into(&mut v[VERSION_LEN..]).unwrap();
        v
    }

    fn build_c2(time: u32, time2: u32, random_echo: [u8; RANDOM_LEN]) -> Vec<u8> {
        let c2 = EchoPacket {
            time,
            time2,
            random_echo,
        };
        let mut v = vec![0u8; HANDSHAKE_PACKET_LEN];
        c2.serialize_into(&mut v).unwrap();
        v
    }

    #[test]
    fn fsm_happy_path_reaches_done_and_s2_echoes_c1() {
        let client_time = 0x0000_1234;
        let client_random = patterned_random(0x77);

        let mut hs = Handshake::new();
        assert!(!hs.is_done());

        let c0_c1 = build_c0_c1(client_time, client_random);
        let (reply, consumed, done) = hs.read(&c0_c1).unwrap();

        assert_eq!(consumed, VERSION_LEN + HANDSHAKE_PACKET_LEN);
        assert!(!done);
        assert!(!hs.is_done());
        assert_eq!(
            reply.len(),
            VERSION_LEN + HANDSHAKE_PACKET_LEN + HANDSHAKE_PACKET_LEN,
            "S0+S1+S2 must total 3073 bytes"
        );

        // S0.
        let s0 = Version::parse(&reply[..VERSION_LEN]).unwrap();
        assert_eq!(s0.0, RTMP_VERSION);

        // S1 (not checked for content beyond length — S1 is our own data).
        let s1_start = VERSION_LEN;
        let s1_end = s1_start + HANDSHAKE_PACKET_LEN;
        let _s1 = HandshakePacket::parse(&reply[s1_start..s1_end]).unwrap();

        // S2 — MUST echo C1's time and random bytes (§5.2.4).
        let s2 = EchoPacket::parse(&reply[s1_end..]).unwrap();
        assert_eq!(s2.time, client_time, "S2.time must echo C1.time");
        assert_eq!(
            s2.random_echo, client_random,
            "S2.random_echo must equal C1's random bytes verbatim"
        );

        // Client now sends C2, echoing S1's time/random (use S1's actual
        // values, which the client would have parsed out of the reply).
        let s1 = HandshakePacket::parse(&reply[s1_start..s1_end]).unwrap();
        let c2 = build_c2(s1.time, 0, s1.random);
        let (reply2, consumed2, done2) = hs.read(&c2).unwrap();
        assert_eq!(consumed2, HANDSHAKE_PACKET_LEN);
        assert!(done2);
        assert!(hs.is_done());
        assert!(
            reply2.is_empty(),
            "C2 receipt produces no further reply bytes"
        );
    }

    #[test]
    fn fsm_partial_c1_is_buffer_too_short() {
        let mut hs = Handshake::new();
        // C0 present, C1 truncated by one byte.
        let full = build_c0_c1(0, patterned_random(1));
        let partial = &full[..full.len() - 1];
        let err = hs.read(partial).unwrap_err();
        assert!(matches!(err, RtmpError::BufferTooShort { .. }));
        assert!(!hs.is_done(), "state must not advance on a short read");
    }

    #[test]
    fn fsm_partial_c2_is_buffer_too_short() {
        let mut hs = Handshake::new();
        let c0_c1 = build_c0_c1(0, patterned_random(2));
        let (_reply, _consumed, done) = hs.read(&c0_c1).unwrap();
        assert!(!done);

        let full_c2 = build_c2(0, 0, patterned_random(2));
        let partial_c2 = &full_c2[..full_c2.len() - 1];
        let err = hs.read(partial_c2).unwrap_err();
        assert!(matches!(err, RtmpError::BufferTooShort { .. }));
        assert!(
            !hs.is_done(),
            "state must not advance to Done on a short C2 read"
        );
    }

    #[test]
    fn fsm_done_is_idempotent_noop() {
        let mut hs = Handshake::new();
        let c0_c1 = build_c0_c1(0, patterned_random(3));
        hs.read(&c0_c1).unwrap();
        let full_c2 = build_c2(0, 0, patterned_random(3));
        hs.read(&full_c2).unwrap();
        assert!(hs.is_done());

        let (reply, consumed, done) = hs.read(&[]).unwrap();
        assert!(reply.is_empty());
        assert_eq!(consumed, 0);
        assert!(done);
    }

    // ── Mutation-check sentinels ──────────────────────────────────────────
    // These pin the exact behaviors a broken serializer/FSM could silently
    // drop; see the module-level test suite as a whole for the mutation
    // scenarios the brief calls out (dropped random tail, non-echoing S2).

    #[test]
    fn handshake_packet_serialize_writes_full_random_tail() {
        let hp = HandshakePacket {
            time: 1,
            zero: 0,
            random: [0xFFu8; RANDOM_LEN],
        };
        let mut buf = [0u8; HANDSHAKE_PACKET_LEN];
        hp.serialize_into(&mut buf).unwrap();
        assert!(
            buf[8..].iter().all(|&b| b == 0xFF),
            "every random byte must be written, not just a prefix"
        );
    }

    #[test]
    fn echo_packet_random_echo_must_differ_from_local_pattern_to_catch_non_echo_bugs() {
        // Sanity check that our two "seeds" actually produce different byte
        // sequences, so a test asserting S2.random_echo == client_random
        // would fail if the FSM instead echoed its *own* S1 random.
        assert_ne!(patterned_random(0x77), default_random_fill());
    }
}
