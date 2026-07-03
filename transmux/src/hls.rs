//! HLS playlist generation — RFC 8216.
//!
//! Produces `#EXTM3U`-formatted media and master playlists from structured
//! data, suitable for VOD and live CMAF workflows.
//!
//! # Discontinuity support
//!
//! The playlist model supports RFC 8216 discontinuity signalling:
//!
//! - **`#EXT-X-DISCONTINUITY`** (RFC 8216 §4.3.4.3) — a marker emitted
//!   immediately before the `#EXTINF` of a discontinuous [`MediaSegment`]
//!   (one whose [`MediaSegment::discontinuous`] flag is `true`). It signals
//!   a break in the media timeline between the preceding segment and the one
//!   that follows it (change in encoding, timestamps, tracks, or format).
//!
//! - **`#EXT-X-DISCONTINUITY-SEQUENCE`** (RFC 8216 §4.3.3.3) — a header
//!   tag equal to the count of discontinuities that have already rolled off
//!   the front of a live/sliding-window playlist. Emitted as
//!   `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` when `n > 0`; absent (defaulting
//!   to 0) otherwise.
//!
//! Use [`Segmenter::mark_discontinuity`](crate::Segmenter::mark_discontinuity)
//! to mark the next cut as discontinuous; the segmenter also auto-detects
//! init-segment changes and marks those cuts automatically.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A single media segment in a media playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSegment {
    /// The segment URI (e.g. `"seg0.m4s"`).
    pub uri: String,
    /// The segment duration in seconds (e.g. `9.009`).
    pub duration: f64,
    /// If `true`, emit `#EXT-X-DISCONTINUITY` immediately before this
    /// segment's `#EXTINF` line — RFC 8216 §4.3.4.3.
    pub discontinuous: bool,
}

/// A media playlist (`#EXTM3U` / `#EXTINF` / ...).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    /// `#EXT-X-VERSION`
    pub version: u8,
    /// `#EXT-X-TARGETDURATION` — must be >= the max rounded segment duration.
    pub target_duration: u32,
    /// `#EXT-X-MEDIA-SEQUENCE`
    pub media_sequence: u64,
    /// `#EXT-X-DISCONTINUITY-SEQUENCE` (RFC 8216 §4.3.3.3) — the count of
    /// discontinuities that have already rolled off the front of a live
    /// sliding-window playlist and are no longer represented by any in-window
    /// `#EXT-X-DISCONTINUITY` tag. Emitted as
    /// `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` when `n > 0`; omitted when `0`
    /// (which is the implicit default per the spec).
    pub discontinuity_sequence: u64,
    /// Ordered list of segments.
    pub segments: Vec<MediaSegment>,
    /// If `true`, append `#EXT-X-ENDLIST`.
    pub endlist: bool,
    /// Extra tag lines emitted verbatim before segment entries
    /// (e.g. `#EXT-X-DATERANGE:...`).
    pub extra_tags: Vec<String>,
}

impl MediaPlaylist {
    /// Render this media playlist as an RFC 8216 `#EXTM3U` string.
    ///
    /// Emits `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` after the media-sequence
    /// header when `discontinuity_sequence > 0` (RFC 8216 §4.3.3.3), and
    /// `#EXT-X-DISCONTINUITY` immediately before the `#EXTINF` of every
    /// segment whose [`MediaSegment::discontinuous`] flag is `true`
    /// (RFC 8216 §4.3.4.3).
    pub fn to_m3u8(&self) -> String {
        let mut s = String::new();
        s.push_str("#EXTM3U\n");
        s.push_str(&format!("#EXT-X-VERSION:{}\n", self.version));
        s.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", self.target_duration));
        s.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.media_sequence));
        if self.discontinuity_sequence > 0 {
            s.push_str(&format!(
                "#EXT-X-DISCONTINUITY-SEQUENCE:{}\n",
                self.discontinuity_sequence
            ));
        }

        for tag in &self.extra_tags {
            s.push_str(tag);
            s.push('\n');
        }

        for seg in &self.segments {
            if seg.discontinuous {
                s.push_str("#EXT-X-DISCONTINUITY\n");
            }
            // Format with exactly 3 decimal places per RFC 8216 examples.
            s.push_str(&format!("#EXTINF:{:.3},\n", seg.duration));
            s.push_str(&seg.uri);
            s.push('\n');
        }

        if self.endlist {
            s.push_str("#EXT-X-ENDLIST\n");
        }

        s
    }
}

/// A variant stream entry in a master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// `BANDWIDTH` in bits per second.
    pub bandwidth: u32,
    /// `CODECS` string (e.g. `"avc1.64001e,mp4a.40.2"`).
    pub codecs: String,
    /// `RESOLUTION` as `(width, height)`, if present.
    pub resolution: Option<(u32, u32)>,
    /// URI of the media playlist for this variant.
    pub uri: String,
}

/// A master playlist (`#EXTM3U` / `#EXT-X-STREAM-INF` / ...).
#[derive(Debug, Clone, PartialEq)]
pub struct MasterPlaylist {
    /// `#EXT-X-VERSION`
    pub version: u8,
    /// Ordered list of variant streams.
    pub variants: Vec<Variant>,
}

impl MasterPlaylist {
    /// Render this master playlist as an RFC 8216 `#EXTM3U` string.
    pub fn to_m3u8(&self) -> String {
        let mut s = String::new();
        s.push_str("#EXTM3U\n");
        s.push_str(&format!("#EXT-X-VERSION:{}\n", self.version));

        for var in &self.variants {
            s.push_str(&format!(
                "#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{}\"",
                var.bandwidth, var.codecs,
            ));
            if let Some((w, h)) = var.resolution {
                s.push_str(&format!(",RESOLUTION={}x{}", w, h));
            }
            s.push('\n');
            s.push_str(&var.uri);
            s.push('\n');
        }

        s
    }
}

/// Auto-detect init-segment changes across a sequence of segments and mark the
/// first segment that follows an init change as discontinuous (RFC 8216 §4.3.4.3).
///
/// `entries` is an ordered list of `(init_bytes, segment)` pairs — one per
/// media segment in playlist order. For each segment after the first, if its
/// init bytes differ from the preceding segment's, `segment.discontinuous` is
/// set to `true`. The first segment is never marked (no preceding context).
///
/// This is the building block for playlist assemblers that splice content from
/// multiple sources with different `EXT-X-MAP` init segments: detect changes
/// once, then pass the updated `MediaSegment` list to [`MediaPlaylist`].
///
/// # Example
/// ```
/// use transmux::hls::{mark_init_discontinuities, MediaSegment};
/// let init_a = b"moov_a" as &[u8];
/// let init_b = b"moov_b" as &[u8];
/// let mut seg0 = MediaSegment { uri: "s0.m4s".into(), duration: 5.0, discontinuous: false };
/// let mut seg1 = MediaSegment { uri: "s1.m4s".into(), duration: 5.0, discontinuous: false };
/// let mut seg2 = MediaSegment { uri: "s2.m4s".into(), duration: 5.0, discontinuous: false };
/// let mut entries: Vec<(&[u8], &mut MediaSegment)> = vec![
///     (init_a, &mut seg0),
///     (init_b, &mut seg1),
///     (init_b, &mut seg2),
/// ];
/// mark_init_discontinuities(&mut entries);
/// assert!(!entries[0].1.discontinuous);
/// assert!(entries[1].1.discontinuous);   // init changed: a → b
/// assert!(!entries[2].1.discontinuous);  // same init
/// ```
pub fn mark_init_discontinuities(entries: &mut [(&[u8], &mut MediaSegment)]) {
    if entries.len() < 2 {
        return;
    }
    // Walk the slice as a sliding window: [prev | cur..].
    // `split_at_mut` gives two non-overlapping sub-slices so we can hold an
    // immutable read of `prev.0` while mutating `cur.1.discontinuous`.
    for i in 1..entries.len() {
        let (head, tail) = entries.split_at_mut(i);
        let prev_init: &[u8] = head[i - 1].0;
        let cur = &mut tail[0];
        if cur.0 != prev_init {
            cur.1.discontinuous = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(uri: &str, duration: f64) -> MediaSegment {
        MediaSegment {
            uri: uri.into(),
            duration,
            discontinuous: false,
        }
    }

    fn seg_disc(uri: &str, duration: f64) -> MediaSegment {
        MediaSegment {
            uri: uri.into(),
            duration,
            discontinuous: true,
        }
    }

    fn playlist(segments: Vec<MediaSegment>) -> MediaPlaylist {
        MediaPlaylist {
            version: 3,
            target_duration: 10,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments,
            endlist: true,
            extra_tags: vec![],
        }
    }

    #[test]
    fn media_playlist_basic() {
        let pl = MediaPlaylist {
            version: 3,
            target_duration: 10,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![
                seg("seg0.m4s", 9.009),
                seg("seg1.m4s", 9.009),
                seg("seg2.m4s", 3.003),
            ],
            endlist: true,
            extra_tags: vec![
                "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0"
                    .into(),
            ],
        };
        let out = pl.to_m3u8();
        assert!(out.starts_with("#EXTM3U\n"));
        assert!(out.contains("#EXT-X-TARGETDURATION:10\n"));
        assert!(out.contains("#EXT-X-MEDIA-SEQUENCE:0\n"));
        assert_eq!(out.matches("#EXTINF:").count(), 3);
        assert!(out.ends_with("#EXT-X-ENDLIST\n"));
        // Check extra tag is present before segments.
        assert!(out.contains("#EXT-X-DATERANGE:ID=\"ad-1\""));
        // No discontinuity sequence when 0.
        assert!(!out.contains("#EXT-X-DISCONTINUITY-SEQUENCE"));
    }

    #[test]
    fn media_playlist_no_endlist() {
        let pl = MediaPlaylist {
            version: 7,
            target_duration: 6,
            media_sequence: 42,
            discontinuity_sequence: 0,
            segments: vec![seg("seg.m4s", 6.000)],
            endlist: false,
            extra_tags: vec![],
        };
        let out = pl.to_m3u8();
        assert!(out.starts_with("#EXTM3U\n"));
        assert!(out.contains("#EXT-X-VERSION:7\n"));
        assert!(!out.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn master_playlist_basic() {
        let pl = MasterPlaylist {
            version: 6,
            variants: vec![
                Variant {
                    bandwidth: 300_000,
                    codecs: "avc1.64001e,mp4a.40.2".into(),
                    resolution: Some((640, 360)),
                    uri: "v300/index.m3u8".into(),
                },
                Variant {
                    bandwidth: 800_000,
                    codecs: "avc1.640028,mp4a.40.2".into(),
                    resolution: Some((1280, 720)),
                    uri: "v800/index.m3u8".into(),
                },
            ],
        };
        let out = pl.to_m3u8();
        assert!(out.starts_with("#EXTM3U\n"));
        assert_eq!(out.matches("#EXT-X-STREAM-INF:").count(), 2);
        assert!(out.contains("v300/index.m3u8"));
        assert!(out.contains("v800/index.m3u8"));
        assert!(out.contains("RESOLUTION=640x360"));
        assert!(out.contains("RESOLUTION=1280x720"));
    }

    #[test]
    fn master_playlist_no_resolution() {
        let pl = MasterPlaylist {
            version: 6,
            variants: vec![Variant {
                bandwidth: 1_000_000,
                codecs: "avc1.640028".into(),
                resolution: None,
                uri: "v1k/index.m3u8".into(),
            }],
        };
        let out = pl.to_m3u8();
        assert!(!out.contains("RESOLUTION"));
        assert!(out.contains("#EXT-X-STREAM-INF:BANDWIDTH=1000000,CODECS=\"avc1.640028\""));
    }

    #[test]
    fn extinf_three_decimals() {
        let pl = playlist(vec![seg("s.m4s", 9.0)]);
        let out = pl.to_m3u8();
        assert!(out.contains("#EXTINF:9.000,\n"));
    }

    // --- discontinuity tag tests ---

    #[test]
    fn discontinuity_tag_emitted_before_extinf() {
        // seg1 is discontinuous; the tag must appear before its #EXTINF.
        let pl = playlist(vec![
            seg("s0.m4s", 5.0),
            seg_disc("s1.m4s", 5.0),
            seg("s2.m4s", 5.0),
        ]);
        let out = pl.to_m3u8();
        assert_eq!(out.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
        // The tag must immediately precede the #EXTINF for s1.
        let disc_pos = out.find("#EXT-X-DISCONTINUITY\n").unwrap();
        let extinf_pos = out.find("#EXTINF:5.000,\n#s1.m4s\n").unwrap_or_else(|| {
            // Find the position of "s1.m4s" in the output and trace back to its #EXTINF.
            let s1_pos = out.find("s1.m4s\n").unwrap();
            // The #EXTINF line starts 11 chars before "5.000,\n" — find it preceding s1.
            out[..s1_pos].rfind("#EXTINF:").unwrap()
        });
        assert!(
            disc_pos < extinf_pos,
            "#EXT-X-DISCONTINUITY must appear before #EXTINF of s1"
        );
        // The discontinuity tag must be the line immediately before #EXTINF:
        let tag_end = disc_pos + "#EXT-X-DISCONTINUITY\n".len();
        assert!(
            out[tag_end..].starts_with("#EXTINF:"),
            "#EXT-X-DISCONTINUITY must be immediately before #EXTINF, got: {:?}",
            &out[tag_end..tag_end + 20]
        );
    }

    #[test]
    fn no_discontinuity_tag_when_all_continuous() {
        let pl = playlist(vec![
            seg("s0.m4s", 5.0),
            seg("s1.m4s", 5.0),
            seg("s2.m4s", 5.0),
        ]);
        let out = pl.to_m3u8();
        assert!(
            !out.contains("#EXT-X-DISCONTINUITY\n"),
            "no tag when all segments are continuous"
        );
    }

    #[test]
    fn discontinuity_sequence_emitted_when_nonzero() {
        let pl = MediaPlaylist {
            version: 3,
            target_duration: 6,
            media_sequence: 5,
            discontinuity_sequence: 2,
            segments: vec![seg("s5.m4s", 6.0)],
            endlist: false,
            extra_tags: vec![],
        };
        let out = pl.to_m3u8();
        assert!(
            out.contains("#EXT-X-DISCONTINUITY-SEQUENCE:2\n"),
            "header must be present when n>0"
        );
    }

    #[test]
    fn discontinuity_sequence_absent_when_zero() {
        let pl = playlist(vec![seg("s0.m4s", 6.0)]);
        let out = pl.to_m3u8();
        assert!(
            !out.contains("#EXT-X-DISCONTINUITY-SEQUENCE"),
            "header must be absent when n==0"
        );
    }
}
