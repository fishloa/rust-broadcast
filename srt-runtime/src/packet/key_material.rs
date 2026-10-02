//! Key Material message — `draft-sharabayko-srt-01` §3.2.2, Figures 10-11.
//!
//! Carried either as a Handshake Extension (§3.2.1.2, `Extension Type`
//! `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP`) or as the CIF of a User-Defined control
//! packet (`Subtype` `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP`, §3.2.2).
//!
//! ```text
//! word0   S(1) V(3) PT(4) | Sign(16) | Resv1(6) KK(2)
//! word1   KEKI(32)
//! word2   Cipher(8) Auth(8) SE(8) Resv2(8)
//! word3   Resv3(16) SLen/4(8) KLen/4(8)
//! ..      Salt (SLen bytes)
//! ..      ICV (8 bytes) | xSEK (KLen bytes) | [oSEK (KLen bytes)]
//! ```
//!
//! `S`, `V`, `PT`, `Sign`, `Resv1`, `Resv2`, `Resv3` are fixed-value fields
//! (the spec gives each a mandated `value = {..}`); they are validated on
//! parse and not stored (matching the crate's reserved-bit policy — see the
//! crate root docs), except `PT`, which acts as this struct's discriminating
//! magic number (must be `2`, "Keying Material Message").
//!
//! This module only carries the wrapped-key *bytes* — it performs no AES
//! key-wrap/unwrap. Actual encryption/decryption is an explicit follow-up
//! (see the crate root docs).

use super::{Error, Result, be32, put_be32};

const S_FIXED: u8 = 0;
const V_FIXED: u8 = 1;
const PT_KEYING_MATERIAL: u8 = 2;
const SIGN_FIXED: u16 = 0x2029; // 'HAI' PnP Vendor ID, big-endian.
const RESV1_FIXED: u8 = 0;
const RESV2_FIXED: u8 = 0;
const RESV3_FIXED: u16 = 0;

/// `KK` wire values of the Key Material message (§3.2.2). Distinct from the
/// data-packet `KK` field ([`super::EncryptionKeyField`]) — same 2-bit shape,
/// different meaning.
pub const KM_KK_NO_SEK: u8 = 0b00;
/// Even key provided.
pub const KM_KK_EVEN: u8 = 0b01;
/// Odd key provided.
pub const KM_KK_ODD: u8 = 0b10;
/// Both keys provided.
pub const KM_KK_BOTH: u8 = 0b11;

/// `KK`: which SEK(s) (odd/even) this Key Material message provides
/// (`draft-sharabayko-srt-01` §3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum KmKeyFlag {
    /// `00b`: no SEK provided (spec: "invalid extension format") — decodable
    /// from the 2-bit field, but [`KeyMaterial::parse`] and
    /// [`KeyMaterial::serialize_into`] both refuse a message carrying it.
    NoSek,
    /// `01b`: even key provided.
    Even,
    /// `10b`: odd key provided.
    Odd,
    /// `11b`: both even and odd keys provided.
    Both,
}

impl KmKeyFlag {
    /// Decode the 2-bit `KK` field.
    pub fn from_bits(v: u8) -> Self {
        match v & 0b11 {
            KM_KK_EVEN => KmKeyFlag::Even,
            KM_KK_ODD => KmKeyFlag::Odd,
            KM_KK_BOTH => KmKeyFlag::Both,
            _ => KmKeyFlag::NoSek,
        }
    }

    /// The 2-bit wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            KmKeyFlag::NoSek => KM_KK_NO_SEK,
            KmKeyFlag::Even => KM_KK_EVEN,
            KmKeyFlag::Odd => KM_KK_ODD,
            KmKeyFlag::Both => KM_KK_BOTH,
        }
    }

    /// Number of SEKs this flag indicates (`n` in the Wrap-field length
    /// formula, §3.2.2: `n = (KK + 1) / 2`).
    pub fn key_count(self) -> u8 {
        match self {
            KmKeyFlag::NoSek => 0,
            KmKeyFlag::Even | KmKeyFlag::Odd => 1,
            KmKeyFlag::Both => 2,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            KmKeyFlag::NoSek => "no SEK",
            KmKeyFlag::Even => "even key",
            KmKeyFlag::Odd => "odd key",
            KmKeyFlag::Both => "both keys",
        }
    }
}

broadcast_common::impl_spec_display!(KmKeyFlag);

/// `Cipher` wire values (§3.2.2).
pub const CIPHER_NONE: u8 = 0;
/// AES-CTR.
pub const CIPHER_AES_CTR: u8 = 2;

/// `Cipher`: encryption cipher and mode (`draft-sharabayko-srt-01` §3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Cipher {
    /// `0`: none, or a KEKI-indexed crypto context.
    None,
    /// `2`: AES-CTR (SP800-38A).
    AesCtr,
    /// A value not defined above (includes `1`).
    Reserved(u8),
}

impl Cipher {
    /// Decode the 8-bit `Cipher` field.
    pub fn from_bits(v: u8) -> Self {
        match v {
            CIPHER_NONE => Cipher::None,
            CIPHER_AES_CTR => Cipher::AesCtr,
            other => Cipher::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            Cipher::None => CIPHER_NONE,
            Cipher::AesCtr => CIPHER_AES_CTR,
            Cipher::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Cipher::None => "none / KEKI-indexed",
            Cipher::AesCtr => "AES-CTR",
            Cipher::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(Cipher, Reserved);

/// `Authentication` wire values (§3.2.2).
pub const KM_AUTH_NONE: u8 = 0;

/// `Authentication`: message authentication code algorithm
/// (`draft-sharabayko-srt-01` §3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum KmAuth {
    /// `0`: none, or a KEKI-indexed crypto context (the only defined value).
    None,
    /// A value not defined above.
    Reserved(u8),
}

impl KmAuth {
    /// Decode the 8-bit `Auth` field.
    pub fn from_bits(v: u8) -> Self {
        match v {
            KM_AUTH_NONE => KmAuth::None,
            other => KmAuth::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            KmAuth::None => KM_AUTH_NONE,
            KmAuth::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            KmAuth::None => "none / KEKI-indexed",
            KmAuth::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(KmAuth, Reserved);

/// `SE` (Stream Encapsulation) wire values (§3.2.2).
pub const STREAM_ENCAP_UNSPECIFIED: u8 = 0;
/// MPEG-TS/UDP.
pub const STREAM_ENCAP_MPEG_TS_UDP: u8 = 1;
/// MPEG-TS/SRT.
pub const STREAM_ENCAP_MPEG_TS_SRT: u8 = 2;

/// `SE`: stream encapsulation (`draft-sharabayko-srt-01` §3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StreamEncapsulation {
    /// `0`: unspecified, or a KEKI-indexed crypto context.
    Unspecified,
    /// `1`: MPEG-TS/UDP.
    MpegTsUdp,
    /// `2`: MPEG-TS/SRT.
    MpegTsSrt,
    /// A value not defined above.
    Reserved(u8),
}

impl StreamEncapsulation {
    /// Decode the 8-bit `SE` field.
    pub fn from_bits(v: u8) -> Self {
        match v {
            STREAM_ENCAP_UNSPECIFIED => StreamEncapsulation::Unspecified,
            STREAM_ENCAP_MPEG_TS_UDP => StreamEncapsulation::MpegTsUdp,
            STREAM_ENCAP_MPEG_TS_SRT => StreamEncapsulation::MpegTsSrt,
            other => StreamEncapsulation::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u8 {
        match self {
            StreamEncapsulation::Unspecified => STREAM_ENCAP_UNSPECIFIED,
            StreamEncapsulation::MpegTsUdp => STREAM_ENCAP_MPEG_TS_UDP,
            StreamEncapsulation::MpegTsSrt => STREAM_ENCAP_MPEG_TS_SRT,
            StreamEncapsulation::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            StreamEncapsulation::Unspecified => "unspecified / KEKI-indexed",
            StreamEncapsulation::MpegTsUdp => "MPEG-TS/UDP",
            StreamEncapsulation::MpegTsSrt => "MPEG-TS/SRT",
            StreamEncapsulation::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(StreamEncapsulation, Reserved);

/// Key Material message (`draft-sharabayko-srt-01` §3.2.2, Figures 10-11).
///
/// [`core::fmt::Debug`] prints only the lengths of the wrapped keys and the
/// ICV: with the Salt they are exactly what an offline attack on the
/// passphrase needs, so they must not reach a log by way of `{:?}`
/// (issue #1142).
#[derive(Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyMaterial<'a> {
    /// Which SEK(s) [`Self::x_sek`] / [`Self::o_sek`] carry.
    pub kk: KmKeyFlag,
    /// Key Encryption Key Index (big-endian; `0` = default stream key).
    pub keki: u32,
    /// Encryption cipher and mode.
    pub cipher: Cipher,
    /// Message authentication code algorithm.
    pub auth: KmAuth,
    /// Stream encapsulation.
    pub se: StreamEncapsulation,
    /// Salt / IV (`SLen` bytes; `0` if absent, else 16 bytes / 128 bits per
    /// the only length the spec defines). Not serialized by the `serde`
    /// feature (with the wrapped keys it permits an offline passphrase
    /// attack), nor shown by `Debug`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub salt: &'a [u8],
    /// 64-bit AES key-wrap Integrity Check Vector. Not serialized by `serde`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub icv: [u8; 8],
    /// The (even or odd, per [`Self::kk`]) SEK, wrapped. `KLen` bytes
    /// (16/24/32, matching the handshake's `Encryption Field`). Not serialized
    /// by `serde`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub x_sek: &'a [u8],
    /// The odd SEK, wrapped, present only when [`Self::kk`] is
    /// [`KmKeyFlag::Both`] (same length as [`Self::x_sek`]). Not serialized by
    /// `serde`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub o_sek: Option<&'a [u8]>,
}

impl core::fmt::Debug for KeyMaterial<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KeyMaterial")
            .field("kk", &self.kk)
            .field("keki", &self.keki)
            .field("cipher", &self.cipher)
            .field("auth", &self.auth)
            .field("se", &self.se)
            .field(
                "salt",
                &format_args!("<redacted, {} bytes>", self.salt.len()),
            )
            .field("icv", &format_args!("<redacted, {} bytes>", self.icv.len()))
            .field(
                "x_sek",
                &format_args!("<redacted, {} bytes>", self.x_sek.len()),
            )
            .field(
                "o_sek",
                &format_args!("<redacted, {} bytes>", self.o_sek.map_or(0, <[u8]>::len)),
            )
            .finish()
    }
}

impl<'a> KeyMaterial<'a> {
    /// Parse a Key Material message from `bytes` (exactly the message —
    /// either a handshake extension's contents or a User-Defined control
    /// packet's CIF).
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "key material fixed header",
            });
        }
        let word0 = be32(bytes, 0);
        let s = (word0 >> 31) as u8;
        let v = ((word0 >> 28) & 0b111) as u8;
        let pt = ((word0 >> 24) & 0b1111) as u8;
        let sign = ((word0 >> 8) & 0xFFFF) as u16;
        let resv1 = ((word0 >> 2) & 0b0011_1111) as u8;
        let kk_bits = (word0 & 0b11) as u8;

        if s != S_FIXED {
            return Err(Error::InvalidKeyMaterial {
                field: "S",
                reason: "must be 0",
            });
        }
        if v != V_FIXED {
            return Err(Error::InvalidKeyMaterial {
                field: "V",
                reason: "must be 1",
            });
        }
        if pt != PT_KEYING_MATERIAL {
            return Err(Error::InvalidKeyMaterial {
                field: "PT",
                reason: "must be 2 (Keying Material Message)",
            });
        }
        if sign != SIGN_FIXED {
            return Err(Error::InvalidKeyMaterial {
                field: "Sign",
                reason: "must be 0x2029 ('HAI' PnP Vendor ID)",
            });
        }
        if resv1 != RESV1_FIXED {
            return Err(Error::InvalidKeyMaterial {
                field: "Resv1",
                reason: "must be 0",
            });
        }
        let kk = KmKeyFlag::from_bits(kk_bits);
        if kk == KmKeyFlag::NoSek {
            // §3.2.2: `KK` = `00b` "No SEK is provided", which the draft
            // itself calls an invalid extension format. Accepting it would
            // admit a value `serialize_into` (which needs a `KLen`-sized
            // xSEK) cannot reproduce — a parse the round-trip invariant
            // cannot honour — so it is refused here and in serialize.
            return Err(Error::InvalidKeyMaterial {
                field: "KK",
                reason: "00b (no SEK provided) is an invalid Key Material message (§3.2.2)",
            });
        }

        let keki = be32(bytes, 4);

        let word2 = be32(bytes, 8);
        let cipher = Cipher::from_bits((word2 >> 24) as u8);
        let auth = KmAuth::from_bits((word2 >> 16) as u8);
        let se = StreamEncapsulation::from_bits((word2 >> 8) as u8);
        let resv2 = (word2 & 0xFF) as u8;
        if resv2 != RESV2_FIXED {
            return Err(Error::InvalidKeyMaterial {
                field: "Resv2",
                reason: "must be 0",
            });
        }

        let word3 = be32(bytes, 12);
        let resv3 = (word3 >> 16) as u16;
        let slen4 = ((word3 >> 8) & 0xFF) as usize;
        let klen4 = (word3 & 0xFF) as usize;
        if resv3 != RESV3_FIXED {
            return Err(Error::InvalidKeyMaterial {
                field: "Resv3",
                reason: "must be 0",
            });
        }

        let slen = slen4 * 4;
        let klen = klen4 * 4;
        if !matches!(klen, 16 | 24 | 32) {
            return Err(Error::InvalidKeyMaterial {
                field: "KLen",
                reason: "must be 16, 24, or 32 bytes (AES-128/192/256)",
            });
        }

        let mut offset = 16usize;
        let salt = bytes
            .get(offset..offset + slen)
            .ok_or(Error::BufferTooShort {
                need: offset + slen,
                have: bytes.len(),
                what: "key material salt",
            })?;
        offset += slen;

        let n = usize::from(kk.key_count());
        let wrap_len = 8 + n * klen;
        let wrap = bytes
            .get(offset..offset + wrap_len)
            .ok_or(Error::BufferTooShort {
                need: offset + wrap_len,
                have: bytes.len(),
                what: "key material wrap field",
            })?;
        offset += wrap_len;

        if offset != bytes.len() {
            return Err(Error::UnexpectedTrailingBytes {
                what: "key material message",
                extra: bytes.len() - offset,
            });
        }

        let mut icv = [0u8; 8];
        icv.copy_from_slice(&wrap[0..8]);
        // `KK` is not `NoSek` (refused above), so there is always an xSEK.
        let x_sek = &wrap[8..8 + klen];
        let o_sek = if n == 2 {
            Some(&wrap[8 + klen..8 + 2 * klen])
        } else {
            None
        };

        Ok(KeyMaterial {
            kk,
            keki,
            cipher,
            auth,
            se,
            salt,
            icv,
            x_sek,
            o_sek,
        })
    }

    /// Number of bytes [`Self::serialize_into`] will write.
    pub fn serialized_len(&self) -> usize {
        16 + self.salt.len() + 8 + self.x_sek.len() + self.o_sek.map_or(0, <[u8]>::len)
    }

    /// Serialize this Key Material message into `buf`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if !self.salt.len().is_multiple_of(4) {
            return Err(Error::InvalidKeyMaterial {
                field: "Salt",
                reason: "length must be a whole number of 4-byte words",
            });
        }
        // `SLen/4` is an 8-bit wire field (§3.2.2): a salt over 1020 bytes
        // would otherwise shift into the reserved `Resv3` bits, misframing
        // the message with `Ok` (#1129). Checked in `usize` before the
        // 8-bit field is packed below.
        broadcast_common::len::fit_bits((self.salt.len() / 4) as u64, 8, "Salt length / 4 (SLen)")
            .map_err(|e| Error::FieldTooWide {
                what: e.field,
                value: e.value,
                bits: 8,
            })?;
        if !matches!(self.x_sek.len(), 16 | 24 | 32) {
            return Err(Error::InvalidKeyMaterial {
                field: "xSEK",
                reason: "length must be 16, 24, or 32 bytes",
            });
        }
        if self.kk == KmKeyFlag::NoSek {
            return Err(Error::InvalidKeyMaterial {
                field: "KK",
                reason: "00b (no SEK provided) is an invalid Key Material message (§3.2.2)",
            });
        }
        let expects_both = self.kk == KmKeyFlag::Both;
        match (&self.o_sek, expects_both) {
            (Some(o), true) if o.len() == self.x_sek.len() => {}
            (None, false) => {}
            _ => {
                return Err(Error::InvalidKeyMaterial {
                    field: "KK/oSEK",
                    reason: "oSEK must be present with the same length as xSEK iff KK is Both",
                });
            }
        }

        let word0 = (u32::from(S_FIXED) << 31)
            | (u32::from(V_FIXED) << 28)
            | (u32::from(PT_KEYING_MATERIAL) << 24)
            | (u32::from(SIGN_FIXED) << 8)
            | (u32::from(RESV1_FIXED) << 2)
            | u32::from(self.kk.to_bits());
        put_be32(buf, 0, word0);
        put_be32(buf, 4, self.keki);
        let word2 = (u32::from(self.cipher.to_bits()) << 24)
            | (u32::from(self.auth.to_bits()) << 16)
            | (u32::from(self.se.to_bits()) << 8)
            | u32::from(RESV2_FIXED);
        put_be32(buf, 8, word2);
        let slen4 = (self.salt.len() / 4) as u32;
        let klen4 = (self.x_sek.len() / 4) as u32;
        let word3 = (u32::from(RESV3_FIXED) << 16) | (slen4 << 8) | klen4;
        put_be32(buf, 12, word3);

        let mut off = 16;
        buf[off..off + self.salt.len()].copy_from_slice(self.salt);
        off += self.salt.len();
        buf[off..off + 8].copy_from_slice(&self.icv);
        off += 8;
        buf[off..off + self.x_sek.len()].copy_from_slice(self.x_sek);
        off += self.x_sek.len();
        if let Some(o) = self.o_sek {
            buf[off..off + o.len()].copy_from_slice(o);
            off += o.len();
        }
        debug_assert_eq!(off, len);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_even_only() -> KeyMaterial<'static> {
        KeyMaterial {
            kk: KmKeyFlag::Even,
            keki: 0,
            cipher: Cipher::AesCtr,
            auth: KmAuth::None,
            se: StreamEncapsulation::MpegTsSrt,
            salt: &[0xAA; 16],
            icv: [1, 2, 3, 4, 5, 6, 7, 8],
            x_sek: &[0xEE; 16],
            o_sek: None,
        }
    }

    #[test]
    fn round_trips_hand_computed_bytes() {
        let km = sample_even_only();
        let mut buf = alloc::vec![0u8; km.serialized_len()];
        let n = km.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 16 + 16 + 8 + 16);

        // word0: S=0,V=1,PT=2,Sign=0x2029,Resv1=0,KK=01
        let expected_word0 = (1u32 << 28) | (2u32 << 24) | (0x2029u32 << 8) | 0b01u32;
        assert_eq!(&buf[0..4], &expected_word0.to_be_bytes());
        // word3: Resv3=0, SLen/4=4, KLen/4=4
        assert_eq!(&buf[12..16], &[0, 0, 4, 4]);
        assert_eq!(&buf[16..32], &[0xAAu8; 16][..]);
        assert_eq!(&buf[32..40], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&buf[40..56], &[0xEEu8; 16][..]);

        assert_eq!(KeyMaterial::parse(&buf).unwrap(), km);
    }

    #[test]
    fn both_keys_round_trip() {
        let km = KeyMaterial {
            kk: KmKeyFlag::Both,
            keki: 7,
            cipher: Cipher::AesCtr,
            auth: KmAuth::None,
            se: StreamEncapsulation::MpegTsSrt,
            salt: &[0; 16],
            icv: [0; 8],
            x_sek: &[1; 24],
            o_sek: Some(&[2; 24]),
        };
        let mut buf = alloc::vec![0u8; km.serialized_len()];
        km.serialize_into(&mut buf).unwrap();
        assert_eq!(KeyMaterial::parse(&buf).unwrap(), km);
    }

    #[test]
    fn no_salt_round_trips() {
        let km = KeyMaterial {
            kk: KmKeyFlag::Odd,
            keki: 0,
            cipher: Cipher::None,
            auth: KmAuth::None,
            se: StreamEncapsulation::Unspecified,
            salt: &[],
            icv: [9; 8],
            x_sek: &[3; 32],
            o_sek: None,
        };
        let mut buf = alloc::vec![0u8; km.serialized_len()];
        km.serialize_into(&mut buf).unwrap();
        assert_eq!(KeyMaterial::parse(&buf).unwrap(), km);
    }

    #[test]
    fn bad_signature_errs_without_panic() {
        let km = sample_even_only();
        let mut buf = alloc::vec![0u8; km.serialized_len()];
        km.serialize_into(&mut buf).unwrap();
        buf[1] = 0x00; // corrupt the Sign field
        assert!(matches!(
            KeyMaterial::parse(&buf),
            Err(Error::InvalidKeyMaterial { field: "Sign", .. })
        ));
    }

    #[test]
    fn inconsistent_kk_o_sek_errs() {
        let mut km = sample_even_only();
        km.o_sek = Some(&[0xEE; 16]); // KK=Even must not carry oSEK
        let mut buf = alloc::vec![0u8; 16 + 16 + 8 + 16 + 16];
        assert!(matches!(
            km.serialize_into(&mut buf),
            Err(Error::InvalidKeyMaterial { .. })
        ));
    }

    /// A salt of 1024 bytes (`SLen/4` = 256) cannot fit the 8-bit `SLen`
    /// wire field (max 255, i.e. 1020 bytes; #1129): unfixed, `(slen4 as u8)`
    /// (well, the packed `u32` word) wraps 256 to 0 and shifts into the
    /// reserved `Resv3` bits, misframing the message with `Ok`.
    #[test]
    fn oversized_salt_length_errors() {
        let salt = alloc::vec![0xAAu8; 1024];
        let km = KeyMaterial {
            kk: KmKeyFlag::Even,
            keki: 0,
            cipher: Cipher::AesCtr,
            auth: KmAuth::None,
            se: StreamEncapsulation::MpegTsSrt,
            salt: &salt,
            icv: [0; 8],
            x_sek: &[0xEE; 16],
            o_sek: None,
        };
        let mut buf = alloc::vec![0u8; km.serialized_len()];
        let err = km.serialize_into(&mut buf).unwrap_err();
        assert!(
            matches!(err, Error::FieldTooWide { bits: 8, .. }),
            "expected FieldTooWide(bits=8) for SLen, got {err:?}"
        );
    }

    /// The boundary: exactly 1020 bytes (`SLen/4` = 255, the 8-bit max)
    /// still serializes and round-trips.
    #[test]
    fn max_salt_length_round_trips() {
        let salt = alloc::vec![0xAAu8; 1020];
        let km = KeyMaterial {
            kk: KmKeyFlag::Even,
            keki: 0,
            cipher: Cipher::AesCtr,
            auth: KmAuth::None,
            se: StreamEncapsulation::MpegTsSrt,
            salt: &salt,
            icv: [0; 8],
            x_sek: &[0xEE; 16],
            o_sek: None,
        };
        let mut buf = alloc::vec![0u8; km.serialized_len()];
        km.serialize_into(&mut buf).unwrap();
        let parsed = KeyMaterial::parse(&buf).unwrap();
        assert_eq!(parsed.salt, salt.as_slice());
    }

    /// r08-SRT-W13: `KK = 00b` ("no SEK provided", which §3.2.2 calls an
    /// invalid format) used to parse to an empty xSEK that `serialize_into`
    /// then refused — a parsed value that could not be re-serialized. It is
    /// now refused at both ends, so no such value exists.
    #[test]
    fn kk_no_sek_is_refused_by_parse_and_serialize() {
        let mut buf = alloc::vec![0u8; sample_even_only().serialized_len()];
        sample_even_only().serialize_into(&mut buf).unwrap();
        // Rewrite `KK` (low 2 bits of word 0) to 00b; keep the 16-byte xSEK so
        // only the flag is wrong.
        buf[3] &= !0b11;
        assert!(matches!(
            KeyMaterial::parse(&buf),
            Err(Error::InvalidKeyMaterial { field: "KK", .. })
        ));
        // The same, with no xSEK bytes at all (the shape the old parser
        // accepted): truncate to header + salt + ICV.
        let short = &buf[..16 + 16 + 8];
        assert!(matches!(
            KeyMaterial::parse(short),
            Err(Error::InvalidKeyMaterial { field: "KK", .. })
        ));

        let mut km = sample_even_only();
        km.kk = KmKeyFlag::NoSek;
        let mut out = alloc::vec![0u8; km.serialized_len()];
        assert!(matches!(
            km.serialize_into(&mut out),
            Err(Error::InvalidKeyMaterial { field: "KK", .. })
        ));
    }

    /// Every message `parse` accepts re-serializes byte-identically (the
    /// symmetry the NoSek case broke), over all `KK` values and key lengths.
    #[test]
    fn every_accepted_message_reserializes_byte_identically() {
        for kk in [KmKeyFlag::Even, KmKeyFlag::Odd, KmKeyFlag::Both] {
            for klen in [16usize, 24, 32] {
                let x = alloc::vec![0x5Au8; klen];
                let o = alloc::vec![0xA5u8; klen];
                let km = KeyMaterial {
                    kk,
                    keki: 3,
                    cipher: Cipher::AesCtr,
                    auth: KmAuth::None,
                    se: StreamEncapsulation::MpegTsSrt,
                    salt: &[7u8; 16],
                    icv: [9; 8],
                    x_sek: &x,
                    o_sek: (kk == KmKeyFlag::Both).then_some(o.as_slice()),
                };
                let mut wire = alloc::vec![0u8; km.serialized_len()];
                km.serialize_into(&mut wire).unwrap();
                let parsed = KeyMaterial::parse(&wire).unwrap();
                let mut again = alloc::vec![0u8; parsed.serialized_len()];
                parsed.serialize_into(&mut again).unwrap();
                assert_eq!(again, wire, "kk={kk:?} klen={klen}");
            }
        }
    }

    /// #1142: `{:?}` of a Key Material message shows the lengths of the wrapped
    /// keys and ICV, never their bytes (with the salt they permit an offline
    /// passphrase attack).
    #[test]
    fn debug_output_redacts_wrapped_keys() {
        let km = KeyMaterial {
            kk: KmKeyFlag::Both,
            keki: 0,
            cipher: Cipher::AesCtr,
            auth: KmAuth::None,
            se: StreamEncapsulation::MpegTsSrt,
            salt: &[0xAA; 16],
            icv: [0xBB; 8],
            x_sek: &[0xEE; 16],
            o_sek: Some(&[0xDD; 16]),
        };
        for dump in [alloc::format!("{km:?}"), alloc::format!("{km:#?}")] {
            for needle in ["238", "187", "221", "170", "0xee", "ee,"] {
                assert!(!dump.contains(needle), "{needle:?} leaked: {dump}");
            }
            assert!(
                dump.contains("x_sek: <redacted, 16 bytes>")
                    || dump.contains("x_sek: <redacted, 16 bytes>,"),
                "{dump}"
            );
            assert!(dump.contains("o_sek: <redacted, 16 bytes>"), "{dump}");
        }
    }

    /// Strings a leak of `bytes` would contain, in every form a `Debug` dump,
    /// `{:x?}` dump or JSON array would print them (the first six bytes are a
    /// long enough run to be unambiguous).
    fn needles(bytes: &[u8]) -> alloc::vec::Vec<alloc::string::String> {
        let head = &bytes[..6];
        let join = |sep: &str| {
            head.iter()
                .map(|b| alloc::format!("{b}"))
                .collect::<alloc::vec::Vec<_>>()
                .join(sep)
        };
        alloc::vec![
            join(", "),
            join(","),
            head.iter()
                .map(|b| alloc::format!("{b:x}"))
                .collect::<alloc::vec::Vec<_>>()
                .join(", "),
            head.iter().map(|b| alloc::format!("{b:02x}")).collect(),
        ]
    }

    /// Distinctive, non-repeating key material (a fixed pseudo-random run, so
    /// the needle cannot occur by chance in unrelated numbers).
    fn distinctive(seed: u8, len: usize) -> alloc::vec::Vec<u8> {
        let mut x = u32::from(seed)
            .wrapping_mul(2_654_435_761)
            .wrapping_add(12_345);
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                u8::try_from(x >> 24).unwrap()
            })
            .collect()
    }

    /// #1142: neither `Debug` nor serde output carries the salt, the ICV or
    /// either wrapped key.
    #[test]
    fn debug_and_serde_output_carry_no_key_bytes() {
        let salt = distinctive(1, 16);
        let x = distinctive(2, 32);
        let o = distinctive(3, 32);
        let icv: [u8; 8] = distinctive(4, 8).try_into().unwrap();
        let km = KeyMaterial {
            kk: KmKeyFlag::Both,
            keki: 0,
            cipher: Cipher::AesCtr,
            auth: KmAuth::None,
            se: StreamEncapsulation::MpegTsSrt,
            salt: &salt,
            icv,
            x_sek: &x,
            o_sek: Some(&o),
        };
        #[cfg(feature = "serde")]
        let json: Option<alloc::string::String> = {
            let json = serde_json::to_string(&km).expect("serialize");
            assert!(
                json.contains("kk"),
                "the non-secret fields are still serialized"
            );
            Some(json)
        };
        #[cfg(not(feature = "serde"))]
        let json: Option<alloc::string::String> = None;
        let dumps: alloc::vec::Vec<alloc::string::String> = [
            alloc::format!("{km:?}"),
            alloc::format!("{km:#?}"),
            alloc::format!("{km:x?}"),
        ]
        .into_iter()
        .chain(json)
        .collect();
        for (name, secret) in [
            ("salt", &salt[..]),
            ("icv", &icv[..]),
            ("x_sek", &x[..]),
            ("o_sek", &o[..]),
        ] {
            for needle in needles(secret) {
                for dump in &dumps {
                    assert!(
                        !dump.contains(&needle),
                        "{name} leaked as {needle:?} in {dump}"
                    );
                }
            }
        }
    }
}
