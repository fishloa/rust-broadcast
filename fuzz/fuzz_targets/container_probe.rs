#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `container-probe`'s format detection on arbitrary bytes. The probe is a
// pure read over a caller-supplied slice, so the contract is absolute: it must
// never panic, never read out of bounds, and always terminate within its
// budget — however malformed, truncated, or adversarial the input.
//
// Several probers walk attacker-influenced length fields (ISOBMFF box sizes,
// EBML varints, MXF BER lengths, ADTS/MP3 frame lengths). Each must bound its
// own reads; this target is what proves it.
//
// Both entry points run. `probe_with_budget` is driven with a budget derived
// from the input so the fuzzer explores budgets larger than the buffer (the
// harness must clamp), smaller than it (a genuine early cut), and zero.
fuzz_target!(|data: &[u8]| {
    let _ = container_probe::probe(data);

    if !data.is_empty() {
        let budget = usize::from(data[0]) * 512;
        let _ = container_probe::probe_with_budget(data, budget);
        let _ = container_probe::probe_with_budget(data, 0);
        let _ = container_probe::probe_with_budget(data, data.len());
    }
});
