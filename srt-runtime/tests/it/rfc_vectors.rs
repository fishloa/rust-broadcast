//! RFC 3394 AES key wrap and RFC 8018/RFC 6070 PBKDF2-HMAC-SHA1 vectors,
//! plus the SRT KEK derivation (issue #1653).
//!
//! Ground truth:
//! - `private/specs-verified/ietf/srt-runtime/rfc3394-aes-key-wrap.md`
//!   (RFC 3394 §2.2 algorithms and the six §4 test vectors)
//! - `private/specs-verified/ietf/srt-runtime/rfc8018-pbkdf2.md`
//!   (RFC 8018 §5.2 PBKDF2)
//! - `private/specs-verified/ietf/srt-runtime/rfc6070-pbkdf2-hmac-sha1-vectors.md`
//!   (RFC 6070 §2 PBKDF2-HMAC-SHA1 vectors)
//! - `private/specs-verified/rules/srt-crypto.md` (SRT: `KEK = PBKDF2(passphrase,
//!   LSB(64, Salt), 2048, KLen)`, SEK wrapped with RFC 3394)
//!
//! Every vector runs through `srt_runtime::crypto`'s own entry points, never
//! the RustCrypto crates directly (the pbkdf2 crate is used only where an
//! independent computation is explicitly required).

#![cfg(feature = "crypto")]

use srt_runtime::Error;
use srt_runtime::crypto::{derive_kek, pbkdf2_hmac_sha1, unwrap_sek, wrap_sek};

// ---------------------------------------------------------------------------
// RFC 3394 §4 — all six key-wrap vectors (128/192/256-bit KEKs).
// ---------------------------------------------------------------------------

struct Rfc3394Vector {
    name: &'static str,
    kek_hex: &'static str,
    key_data_hex: &'static str,
    ciphertext_hex: &'static str,
}

const VECTORS: &[Rfc3394Vector] = &[
    Rfc3394Vector {
        name: "4.1 128-bit KEK / 128-bit key data",
        kek_hex: "000102030405060708090A0B0C0D0E0F",
        key_data_hex: "00112233445566778899AABBCCDDEEFF",
        ciphertext_hex: "1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5",
    },
    Rfc3394Vector {
        name: "4.2 192-bit KEK / 128-bit key data",
        kek_hex: "000102030405060708090A0B0C0D0E0F1011121314151617",
        key_data_hex: "00112233445566778899AABBCCDDEEFF",
        ciphertext_hex: "96778B25AE6CA435F92B5B97C050AED2468AB8A17AD84E5D",
    },
    Rfc3394Vector {
        name: "4.3 256-bit KEK / 128-bit key data",
        kek_hex: "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        key_data_hex: "00112233445566778899AABBCCDDEEFF",
        ciphertext_hex: "64E8C3F9CE0F5BA263E9777905818A2A93C8191E7D6E8AE7",
    },
    Rfc3394Vector {
        name: "4.4 192-bit KEK / 192-bit key data",
        kek_hex: "000102030405060708090A0B0C0D0E0F1011121314151617",
        key_data_hex: "00112233445566778899AABBCCDDEEFF0001020304050607",
        ciphertext_hex: "031D33264E15D33268F24EC260743EDCE1C6C7DDEE725A936BA814915C6762D2",
    },
    Rfc3394Vector {
        name: "4.5 256-bit KEK / 192-bit key data",
        kek_hex: "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        key_data_hex: "00112233445566778899AABBCCDDEEFF0001020304050607",
        ciphertext_hex: "A8F9BC1612C68B3FF6E6F4FBE30E71E4769C8B80A32CB8958CD5D17D6B254DA1",
    },
    Rfc3394Vector {
        name: "4.6 256-bit KEK / 256-bit key data",
        kek_hex: "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        key_data_hex: "00112233445566778899AABBCCDDEEFF000102030405060708090A0B0C0D0E0F",
        ciphertext_hex: "28C9F404C4B810F4CBCCB35CFB87F8263F5786E2D80ED326CBC7F0E71A99F43BFB988B9B7A02DD21",
    },
];

#[test]
fn rfc3394_wrap_vectors() {
    for v in VECTORS {
        let kek = test_fixtures::hex_bytes(v.kek_hex);
        let key_data = test_fixtures::hex_bytes(v.key_data_hex);
        let expected_ct = test_fixtures::hex_bytes(v.ciphertext_hex);

        let (icv, wrapped) =
            wrap_sek(&kek, &key_data).unwrap_or_else(|e| panic!("{} wrap failed: {e}", v.name));
        let mut got = Vec::with_capacity(expected_ct.len());
        got.extend_from_slice(&icv);
        got.extend_from_slice(&wrapped);
        assert_eq!(got, expected_ct, "{} wrap mismatch", v.name);
    }
}

#[test]
fn rfc3394_unwrap_vectors() {
    for v in VECTORS {
        let kek = test_fixtures::hex_bytes(v.kek_hex);
        let key_data = test_fixtures::hex_bytes(v.key_data_hex);
        let ct = test_fixtures::hex_bytes(v.ciphertext_hex);

        let mut icv = [0u8; 8];
        icv.copy_from_slice(&ct[..8]);
        let wrapped = &ct[8..];

        let recovered = unwrap_sek(&kek, &icv, wrapped)
            .unwrap_or_else(|e| panic!("{} unwrap failed: {e}", v.name));
        assert_eq!(
            recovered.as_slice(),
            key_data.as_slice(),
            "{} unwrap mismatch",
            v.name
        );
    }
}

#[test]
fn rfc3394_unwrap_rejects_corruption() {
    // Flip one bit of a valid ciphertext; unwrap must fail with a typed error.
    let v = &VECTORS[0];
    let kek = test_fixtures::hex_bytes(v.kek_hex);
    let ct = test_fixtures::hex_bytes(v.ciphertext_hex);

    let mut corrupted = ct.clone();
    // Flip a bit in the wrapped portion (not the ICV alone is enough, but flip
    // in the key data region to exercise the integrity check).
    let idx = corrupted.len() - 1;
    corrupted[idx] ^= 0x01;

    let mut icv = [0u8; 8];
    icv.copy_from_slice(&corrupted[..8]);
    let wrapped = &corrupted[8..];

    let err = unwrap_sek(&kek, &icv, wrapped).expect_err("corrupted ciphertext must fail unwrap");
    assert!(
        matches!(
            err,
            Error::InvalidField {
                what: "AES key wrap",
                ..
            }
        ),
        "expected typed InvalidField error, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// RFC 6070 PBKDF2-HMAC-SHA1 vectors (all except the 16777216-iteration one).
// ---------------------------------------------------------------------------

struct Pbkdf2Vector {
    name: &'static str,
    password: &'static [u8],
    salt: &'static [u8],
    iterations: u32,
    dk_len: usize,
    expected_hex: &'static str,
}

const PBKDF2_VECTORS: &[Pbkdf2Vector] = &[
    Pbkdf2Vector {
        name: "RFC6070 #1 c=1",
        password: b"password",
        salt: b"salt",
        iterations: 1,
        dk_len: 20,
        expected_hex: "0c60c80f961f0e71f3a9b524af6012062fe037a6",
    },
    Pbkdf2Vector {
        name: "RFC6070 #2 c=2",
        password: b"password",
        salt: b"salt",
        iterations: 2,
        dk_len: 20,
        expected_hex: "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957",
    },
    Pbkdf2Vector {
        name: "RFC6070 #3 c=4096",
        password: b"password",
        salt: b"salt",
        iterations: 4096,
        dk_len: 20,
        expected_hex: "4b007901b765489abead49d926f721d065a429c1",
    },
    Pbkdf2Vector {
        name: "RFC6070 #5 long password/salt c=4096",
        password: b"passwordPASSWORDpassword",
        salt: b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
        iterations: 4096,
        dk_len: 25,
        expected_hex: "3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038",
    },
    Pbkdf2Vector {
        name: "RFC6070 #6 embedded NUL c=4096",
        password: b"pass\0word",
        salt: b"sa\0lt",
        iterations: 4096,
        dk_len: 16,
        expected_hex: "56fa6aa75548099dcc37d7f03425e0c3",
    },
];

#[test]
fn rfc6070_pbkdf2_vectors() {
    for v in PBKDF2_VECTORS {
        let mut out = vec![0u8; v.dk_len];
        pbkdf2_hmac_sha1(v.password, v.salt, v.iterations, &mut out);
        let expected = test_fixtures::hex_bytes(v.expected_hex);
        assert_eq!(out, expected, "{} mismatch", v.name);
    }
}

#[test]
fn rfc6070_pbkdf2_long_vector() {
    if std::env::var_os("RUST_BROADCAST_SOAK").as_deref() != Some(std::ffi::OsStr::new("1")) {
        eprintln!("SKIP rfc6070_pbkdf2_long_vector: set RUST_BROADCAST_SOAK=1 to run");
        return;
    }
    let mut out = vec![0u8; 20];
    pbkdf2_hmac_sha1(b"password", b"salt", 16777216, &mut out);
    let expected = test_fixtures::hex_bytes("eefe3d61cd4da4e4e9945b3d6ba2158c2634e984");
    assert_eq!(out, expected, "RFC6070 #4 (16777216 iterations) mismatch");
}

// ---------------------------------------------------------------------------
// SRT KEK derivation.
// ---------------------------------------------------------------------------

#[test]
fn srt_kek_uses_low_64_bits_of_salt() {
    // LSB(64, Salt): the draft's salt argument to PBKDF2 is the least
    // significant 64 bits of the 128-bit big-endian-wire Salt. In this
    // crate's `derive_kek` that is the LAST 8 bytes of the 16-byte array
    // (`salt[8..]`), because the wire Salt is big-endian and the low 64 bits
    // occupy its trailing bytes. Hence two salts differing only in the first
    // 8 bytes (the MSBs) yield the same KEK, while two differing only in the
    // last 8 bytes yield different KEKs.
    let passphrase = b"secret passphrase";
    let klen = 16usize;

    // Same low 8 bytes, different high 8 bytes -> same KEK.
    let mut a = [0u8; 16];
    a[..8].copy_from_slice(&[0x11; 8]);
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&[0x22; 8]);
    // low 8 bytes identical (zero)
    let kek_a = derive_kek(passphrase, &a, klen).unwrap();
    let kek_b = derive_kek(passphrase, &b, klen).unwrap();
    assert_eq!(
        kek_a, kek_b,
        "salts differing only in first 8 bytes must give the same KEK"
    );

    // Same high 8 bytes, different low 8 bytes -> different KEKs.
    let mut c = [0u8; 16];
    c[8..].copy_from_slice(&[0x33; 8]);
    let mut d = [0u8; 16];
    d[8..].copy_from_slice(&[0x44; 8]);
    let kek_c = derive_kek(passphrase, &c, klen).unwrap();
    let kek_d = derive_kek(passphrase, &d, klen).unwrap();
    assert_ne!(
        kek_c, kek_d,
        "salts differing only in last 8 bytes must give different KEKs"
    );
}

#[test]
fn srt_kek_uses_2048_iterations() {
    let passphrase = b"correct horse battery staple";
    let salt = [0xABu8; 16];
    let klen = 32usize;

    // Independent computation through the lowest-level primitive with 2048.
    let mut expected = vec![0u8; klen];
    pbkdf2_hmac_sha1(passphrase, &salt[8..], 2048, &mut expected);

    let kek = derive_kek(passphrase, &salt, klen).unwrap();
    assert_eq!(
        kek.as_slice(),
        expected.as_slice(),
        "KEK must equal PBKDF2 at 2048 iterations"
    );

    // 2047 iterations must give a different KEK.
    let mut at_2047 = vec![0u8; klen];
    pbkdf2_hmac_sha1(passphrase, &salt[8..], 2047, &mut at_2047);
    assert_ne!(
        kek.as_slice(),
        at_2047.as_slice(),
        "2047 iterations must differ from 2048"
    );
}
