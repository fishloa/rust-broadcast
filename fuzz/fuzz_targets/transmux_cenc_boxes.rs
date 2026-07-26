#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the CENC per-sample aux-info box parsers — `senc` (ISO/IEC 23001-7
// §12.3), `saiz` (ISO/IEC 14496-12 §8.7.8) and `saio` (§8.7.9).
//
// These read third-party media (a protected fMP4 arriving over the network), and
// every one of them takes a sample/entry count straight off the wire before
// sizing a `Vec`. They must never panic and never allocate on an unverified
// count: `senc`'s `sample_count` at 0xFFFFFFFF in a 20-byte box previously asked
// the allocator for hundreds of GB and aborted the process.
//
// The first three bytes of the input drive the out-of-band parameters a real
// caller would supply from the track's `tenc`/FullBox header (per-sample IV
// size, version, flags), so the fuzzer explores every branch — including the
// degenerate "no per-sample IV and no subsample map" shape — rather than only
// the default one.
fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }
    let per_sample_iv_size = data[0];
    let version = data[1];
    let flags = u32::from(data[2]);
    let body = &data[3..];

    let _ = transmux::SampleEncryptionBox::parse_body(body, version, flags, per_sample_iv_size);
    let _ = transmux::SampleAuxInfoSizesBox::parse_body(body, version, flags);
    let _ = transmux::SampleAuxInfoOffsetsBox::parse_body(body, version, flags);
    // The full-box entry points too (they re-read version/flags from the bytes).
    let _ = transmux::SampleAuxInfoSizesBox::parse_box(data);
    let _ = transmux::SampleAuxInfoOffsetsBox::parse_box(data);
});
