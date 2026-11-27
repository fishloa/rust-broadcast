#![no_main]

use broadcast_core::Parse;
use libfuzzer_sys::fuzz_target;

// The de-facto structures a real file carries, on arbitrary bytes: a
// Matroska `Tags` element and its typed statistics, and the iTunes
// `iTunSMPB` value. Each is a pure read, so any input must either be
// refused or yield a value.
fuzz_target!(|data: &[u8]| {
    if let Ok(tags) = broadcast_matroska::matroska::Tags::parse(data) {
        for tag in tags.tags() {
            let statistics = tag.statistics();
            let _ = statistics.duration_nanoseconds();
            let _ = statistics.bits_per_second();
            let _ = statistics.number_of_frames();
            let _ = statistics.number_of_bytes();
            let _ = statistics.encoder();
            let _ = statistics.encoder_settings();
            let _ = statistics.writing_app();
            let _ = statistics.writing_date();
            let _ = statistics.tags_list();
            for malformed in statistics.malformed() {
                let _ = malformed.name;
                let _ = malformed.value;
            }
            if let Some(targets) = tag.targets() {
                for uid in targets.track_uids() {
                    let _ = tags.statistics_for_track(uid).count();
                }
            }
        }
    }
    if let Ok(text) = core::str::from_utf8(data) {
        if let Ok(trim) = isobmff::metadata::Itunsmpb::parse_text(text) {
            let _ = trim.priming();
            let _ = trim.padding();
            let _ = trim.original_samples();
        }
    }
});
