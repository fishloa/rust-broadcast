#![no_main]

use broadcast_matroska::Limits;
use broadcast_matroska::reader::{Mode, Reader, Source, Step};
use libfuzzer_sys::fuzz_target;

const SUPPLY_SIZES: [usize; 6] = [1, 3, 7, 64, 1024, usize::MAX];

fuzz_target!(|data: &[u8]| {
    let len = data.len() as u64;
    for source in [Source::Seekable { len }, Source::ForwardOnly] {
        for chunk in SUPPLY_SIZES {
            let mut reader = Reader::new(source, Mode::Permissive, Limits::new());
            let mut steps = 0usize;
            loop {
                match reader.step() {
                    Ok(Step::Need(range)) => {
                        if range.offset > len {
                            break;
                        }
                        let want = range.len.min(chunk as u64).min(len - range.offset);
                        let start = range.offset as usize;
                        let end = start + want as usize;
                        if reader.supply(range.offset, &data[start..end]).is_err() {
                            break;
                        }
                    }
                    Ok(Step::Done) => break,
                    Ok(Step::Tracks { tracks, .. }) => {
                        if let Some(track) = tracks.first() {
                            let number = track.track_number();
                            let _ = reader.seek_to(number, 0);
                        }
                        steps += 1;
                    }
                    Ok(_) => {
                        steps += 1;
                        if steps > 1_000_000 {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
});
