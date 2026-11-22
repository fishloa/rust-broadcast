#![no_main]
use libfuzzer_sys::fuzz_target;

use broadcast_hls::{CustomTagName, MediaPlaylist, ParseOptions};
use dvr_runtime::{ArchiveContainer, PeriodNum};

// The first byte selects the container (bit 0: 0 = fMP4, 1 = TS) and the period
// (bits 1-7); the rest is the playlist. Both must match the marker line, so
// fixing them would leave the TS arm and every period but one unreachable.
fuzz_target!(|data: &[u8]| {
    let Some((selector, data)) = data.split_first() else {
        return;
    };
    let container = match selector & 1 {
        0 => ArchiveContainer::Fmp4,
        _ => ArchiveContainer::Ts,
    };
    let period = PeriodNum::new(u32::from(selector >> 1));
    let Ok(playlist) = dvr_runtime::read::parse_period_playlist(period, container, data) else {
        return;
    };
    let mut end = data.len();
    while end > 0 && data[end - 1] == 0 {
        end -= 1;
    }
    let complete = match data[..end].iter().rposition(|&b| b == b'\n') {
        Some(last) => &data[..last + 1],
        None => &[],
    };
    let Ok(text) = std::str::from_utf8(complete) else {
        return;
    };
    let Ok(name) = CustomTagName::new("EXT-X-RUSTBROADCAST-TIMING") else {
        return;
    };
    let options = ParseOptions::default().with_segment_scoped_tag(name);
    let Ok(parsed) = MediaPlaylist::parse_with_options(
        text,
        &broadcast_hls::limits::Limits::default(),
        &options,
    ) else {
        return;
    };
    let Ok(rendered) = parsed.to_m3u8() else {
        return;
    };
    let again = dvr_runtime::read::parse_period_playlist(period, container, rendered.as_bytes())
        .expect("a rendered archive playlist parses back");
    assert_eq!(again.segments, playlist.segments);
    assert_eq!(again.programmes, playlist.programmes);
    assert_eq!(again.closed, playlist.closed);
});
