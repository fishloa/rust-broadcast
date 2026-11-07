#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../support/doctor_text.rs"]
mod doctor_text;

// Fuzz `media_doctor::check_hls_playlist` — the validator built on top of
// `broadcast_hls::MediaPlaylist::parse` / `MasterPlaylist::parse` — on
// arbitrary UTF-8 text. Must not panic on any input, however malformed or
// adversarial.
fuzz_target!(|data: &[u8]| doctor_text::check_text(data, media_doctor::check_hls_playlist));
