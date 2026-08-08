#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // DVB Common Scrambling Algorithm (CSA2) — scramble/descramble round-trip.
    // First 8 bytes are the control word (ControlWord); rest is payload.

    if data.len() < 8 {
        return;
    }

    let cw_bytes = &data[0..8];
    let payload = &data[8..];

    // Construct the control word
    let mut cw_array = [0u8; 8];
    cw_array.copy_from_slice(cw_bytes);
    let cw = dvb_csa::ControlWord(cw_array);

    // Scramble the payload (requires mutable copy)
    let mut scrambled = payload.to_vec();
    dvb_csa::scramble(&cw, &mut scrambled);

    // Descramble and verify round-trip (requires mutable copy)
    let mut descrambled = scrambled.clone();
    dvb_csa::descramble(&cw, &mut descrambled);

    // Round-trip invariant: descramble(scramble(x)) == x
    assert_eq!(payload, &descrambled[..payload.len()], "DVB-CSA round-trip mismatch");
});
