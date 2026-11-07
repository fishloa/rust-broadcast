//! Byte-level pieces shared by the fuzz targets and the in-repo fuzz drivers.
//!
//! Nothing here depends on a protocol crate: the DASH targets and the
//! `dash-runtime` live driver both build their answers from the same fixture
//! world, damage them the same way and cap their inputs with the same
//! constants, so those live once, here.

use std::sync::OnceLock;

pub const MAX_INPUT: usize = 1 << 20;
pub const INPUT_CAP_256K: usize = 256 * 1024;
pub const MICROS: u64 = 1_000_000;
pub const POOL: usize = 4;
pub const CAPS: [u32; 4] = [0, 1, 3, 8];
pub const PREFER_HOST: u8 = 0b1_0000;
pub const PIN_ORIGIN: u8 = 0b10_0000;
pub const USE_ATO: u8 = 0b100_0000;
pub const DEFAULT_PUBLISH_US: i64 = 1_788_000_000 * MICROS as i64;
pub const STEPS_US: [u64; 6] = [
    1_000,
    50_000,
    500_000,
    2 * MICROS,
    30 * MICROS,
    700 * MICROS,
];
pub const RESYNC_US: [u64; 4] = [MICROS, 30 * MICROS, 600 * MICROS, u64::MAX];
pub const GRACE_US: [u64; 3] = [0, MICROS, 5 * MICROS];
pub const HTTP_DATES: [&str; 3] = [
    "Mon, 26 Oct 2026 09:18:20 GMT",
    "Thu, 01 Jan 1970 00:00:00 GMT",
    "not a date",
];
pub const LIVE_MPDS: [&str; 6] = [
    "dash/live-number/live.mpd",
    "dash/live-timeline/live.mpd",
    "dash/livesim2/segtimeline_time_a.mpd",
    "dash/livesim2/segtimeline_time_b.mpd",
    "dash/livesim2/segtimelinenr_a.mpd",
    "dash/livesim2/segtimelinenr_b.mpd",
];

const VIDEO_SEGS: [&str; POOL] = [
    "dash/live-number/seg-0-005.m4s",
    "dash/live-number/seg-0-006.m4s",
    "dash/live-number/seg-0-007.m4s",
    "dash/live-number/seg-0-008.m4s",
];
const AUDIO_SEGS: [&str; POOL] = [
    "dash/live-number/seg-1-005.m4s",
    "dash/live-number/seg-1-006.m4s",
    "dash/live-number/seg-1-007.m4s",
    "dash/live-number/seg-1-008.m4s",
];
const VIDEO_INIT: &str = "dash/live-number/init-0.m4s";
const AUDIO_INIT: &str = "dash/live-number/init-1.m4s";
const NAME_HASH_MUL: usize = 31;
const FLIP_STRIDE: usize = 7;
const ARBITRARY_PER_AUX: usize = 8;
const UTC_JUNK_LEN: usize = 5_000;
const UTC_MODES: u8 = 5;
const SECS_PER_DAY: i64 = 86_400;
const DAYS_USED: i64 = 28;

static WORLD: OnceLock<DashWorld> = OnceLock::new();

/// The DASH fixtures a live-client fuzz run answers from.
pub struct DashWorld {
    pub mpds: Vec<Vec<u8>>,
    video_segs: Vec<Vec<u8>>,
    audio_segs: Vec<Vec<u8>>,
    video_init: Vec<u8>,
    audio_init: Vec<u8>,
}

impl DashWorld {
    /// The process-wide world, loading [`LIVE_MPDS`] then `extra_mpds` through
    /// `load` on first use.
    pub fn get_or_load(load: &dyn Fn(&str) -> Vec<u8>, extra_mpds: &[&str]) -> &'static Self {
        WORLD.get_or_init(|| Self {
            mpds: LIVE_MPDS
                .iter()
                .chain(extra_mpds)
                .map(|rel| load(rel))
                .collect(),
            video_segs: VIDEO_SEGS.iter().map(|rel| load(rel)).collect(),
            audio_segs: AUDIO_SEGS.iter().map(|rel| load(rel)).collect(),
            video_init: load(VIDEO_INIT),
            audio_init: load(AUDIO_INIT),
        })
    }

    /// The world loaded by [`Self::get_or_load`].
    ///
    /// # Panics
    /// If no world has been loaded yet.
    #[must_use]
    pub fn loaded() -> &'static Self {
        WORLD.get().expect("fixtures are loaded by `get_or_load`")
    }

    /// The fixture body the URL names (init or media, audio or video), cut to
    /// `range` when one is given.
    #[must_use]
    pub fn real_body(&self, url: &str, range: Option<(u64, u64)>) -> Option<Vec<u8>> {
        let path = url.split('?').next()?;
        let name = path.rsplit('/').next()?;
        let audio = path.contains("-1-") || path.contains("A48") || path.contains("init-1");
        let all: &[u8] = if name.contains("init") {
            if audio {
                &self.audio_init
            } else {
                &self.video_init
            }
        } else if audio {
            &self.audio_segs[name_hash(name) % POOL]
        } else {
            &self.video_segs[name_hash(name) % POOL]
        };
        slice_range(all, range)
    }
}

fn name_hash(name: &str) -> usize {
    name.bytes().fold(0usize, |h, b| {
        h.wrapping_mul(NAME_HASH_MUL).wrapping_add(usize::from(b))
    })
}

/// `all` whole, or the `(offset, length)` window of it clamped to its end.
#[must_use]
pub fn slice_range(all: &[u8], range: Option<(u64, u64)>) -> Option<Vec<u8>> {
    match range {
        None => Some(all.to_vec()),
        Some((off, len)) => {
            let start = usize::try_from(off).ok()?;
            let end = start.saturating_add(usize::try_from(len).ok()?);
            all.get(start..end.min(all.len())).map(<[u8]>::to_vec)
        }
    }
}

/// Up to `aux * 8` bytes taken off the front of `rest`.
pub fn take_arbitrary(rest: &mut &[u8], aux: u8) -> Vec<u8> {
    let take = (usize::from(aux) * ARBITRARY_PER_AUX).min(rest.len());
    let (head, tail) = rest.split_at(take);
    *rest = tail;
    head.to_vec()
}

/// Cut `aux + 1` bytes off the tail.
pub fn truncate_tail(body: &mut Vec<u8>, aux: u8) {
    body.truncate(body.len().saturating_sub(usize::from(aux) + 1));
}

/// Invert one byte, chosen by `aux`; a no-op on an empty body.
pub fn flip_byte(body: &mut [u8], aux: u8) {
    if !body.is_empty() {
        let at = usize::from(aux) * FLIP_STRIDE % body.len();
        body[at] ^= 0xFF;
    }
}

/// The MPD a run starts from: `mpds[selector - 1]`, or, for selector 0, the
/// next `len_hi:len_lo` (mod the remaining bytes) fuzz bytes themselves.
pub fn pick_mpd<M: AsRef<[u8]>>(
    mpds: &[M],
    selector: u8,
    len_hi: u8,
    len_lo: u8,
    rest: &mut &[u8],
) -> Vec<u8> {
    let selector = usize::from(selector) % (mpds.len() + 1);
    if selector == 0 {
        let len = (usize::from(len_hi) << 8 | usize::from(len_lo)) % (rest.len() + 1);
        let (m, tail) = rest.split_at(len);
        *rest = tail;
        m.to_vec()
    } else {
        mpds[selector - 1].as_ref().to_vec()
    }
}

/// A `UTCTiming` answer body: an ISO time (modes 0 and 1), fuzz bytes, empty, or junk.
pub fn utc_body(mode: u8, aux: u8, now: u64, base_unix: i64, rest: &mut &[u8]) -> Vec<u8> {
    match mode % UTC_MODES {
        0 | 1 => {
            let unix = base_unix.saturating_add(i64::try_from(now).unwrap_or(0));
            let secs = unix.div_euclid(MICROS as i64);
            let ms = unix.rem_euclid(MICROS as i64) / 1_000;
            let days = secs.div_euclid(SECS_PER_DAY);
            let tod = secs.rem_euclid(SECS_PER_DAY);
            format!(
                "2026-10-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                1 + days.rem_euclid(DAYS_USED),
                tod / 3_600,
                tod % 3_600 / 60,
                tod % 60,
                ms
            )
            .into_bytes()
        }
        2 => take_arbitrary(rest, aux),
        3 => Vec::new(),
        _ => vec![b'9'; UTC_JUNK_LEN],
    }
}
