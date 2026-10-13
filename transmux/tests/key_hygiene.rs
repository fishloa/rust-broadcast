//! Content-key hygiene (audit umbrella #1142): keys never print through
//! `Debug`, are compared in constant time, and are wiped when their owner
//! drops.
#![cfg(feature = "cenc")]

use transmux::KeyMap;
use transmux::cenc_encrypt::CencEncryptor;

const KID: [u8; 16] = [0x11; 16];
const KEY: [u8; 16] = [0xA5; 16];

#[test]
fn key_map_debug_lists_kids_but_never_key_bytes() {
    let map = KeyMap::new().with_key(KID, KEY);
    let text = format!("{map:?}");
    // 0xA5 == 165: a derived Debug over the BTreeMap would print `165, 165`.
    assert!(!text.contains("165"), "{text}");
    assert!(text.contains("17"), "kids stay visible: {text}");
}

#[test]
fn key_map_equality_distinguishes_keys_kids_and_sizes() {
    let a = KeyMap::new().with_key(KID, KEY);
    assert_eq!(a, KeyMap::new().with_key(KID, KEY));
    let mut other_key = KEY;
    other_key[15] ^= 1; // differs only in the LAST byte
    assert_ne!(a, KeyMap::new().with_key(KID, other_key));
    assert_ne!(a, KeyMap::new().with_key([0x22; 16], KEY));
    assert_ne!(a, KeyMap::new());
    assert_ne!(
        a,
        KeyMap::new().with_key(KID, KEY).with_key([0x33; 16], KEY)
    );
}

#[test]
fn encryptor_debug_never_prints_the_key() {
    let enc = CencEncryptor::new(KEY);
    let text = format!("{enc:?}");
    assert!(!text.contains("165"), "{text}");
    assert!(text.contains("redacted"), "{text}");
}

/// Run only the encryptor's `Drop` while keeping its storage alive, then look
/// for the key bytes in that storage. The key is an inline array, so nothing
/// is freed and reading the storage back afterwards is sound.
#[test]
#[allow(unsafe_code)]
fn encryptor_wipes_its_key_on_drop() {
    fn contains_key(bytes: &[u8]) -> bool {
        bytes.windows(KEY.len()).any(|w| w == KEY)
    }
    let mut slot = core::mem::ManuallyDrop::new(CencEncryptor::new(KEY));
    let len = core::mem::size_of::<CencEncryptor>();
    let ptr = core::ptr::addr_of!(*slot).cast::<u8>();
    // SAFETY: `slot` is live, aligned storage of `len` bytes; the struct holds
    // a `[u8; 16]` and a `u64`, so every byte is initialised (no padding is
    // read as a value that matters: u8 has no validity invariant).
    let before = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    assert!(contains_key(&before), "the key is stored inline");
    // SAFETY: runs `Drop` exactly once; `ManuallyDrop` never drops it again.
    unsafe { core::ptr::drop_in_place(&mut *slot) };
    // SAFETY: as above; the storage is still owned by `slot`.
    let after = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    assert!(
        !contains_key(&after),
        "the key must be wiped by Drop: {after:?}"
    );
}
