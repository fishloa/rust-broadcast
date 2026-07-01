//! HLS playlist generation — RFC 8216.
//!
//! Produces `#EXTM3U`-formatted media and master playlists from structured
//! data, suitable for VOD and live CMAF workflows.

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
    pub fn to_m3u8(&self) -> String {
        let mut s = String::new();
        s.push_str("#EXTM3U\n");
        s.push_str(&format!("#EXT-X-VERSION:{}\n", self.version));
        s.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", self.target_duration));
        s.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.media_sequence));

        for tag in &self.extra_tags {
            s.push_str(tag);
            s.push('\n');
        }

        for seg in &self.segments {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_playlist_basic() {
        let pl = MediaPlaylist {
            version: 3,
            target_duration: 10,
            media_sequence: 0,
            segments: vec![
                MediaSegment {
                    uri: "seg0.m4s".into(),
                    duration: 9.009,
                },
                MediaSegment {
                    uri: "seg1.m4s".into(),
                    duration: 9.009,
                },
                MediaSegment {
                    uri: "seg2.m4s".into(),
                    duration: 3.003,
                },
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
    }

    #[test]
    fn media_playlist_no_endlist() {
        let pl = MediaPlaylist {
            version: 7,
            target_duration: 6,
            media_sequence: 42,
            segments: vec![MediaSegment {
                uri: "seg.m4s".into(),
                duration: 6.000,
            }],
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
        let pl = MediaPlaylist {
            version: 3,
            target_duration: 10,
            media_sequence: 0,
            segments: vec![MediaSegment {
                uri: "s.m4s".into(),
                duration: 9.0,
            }],
            endlist: false,
            extra_tags: vec![],
        };
        let out = pl.to_m3u8();
        assert!(out.contains("#EXTINF:9.000,\n"));
    }
}
