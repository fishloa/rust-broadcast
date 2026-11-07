//! Loader for the committed `ffprobe -show_packets` oracle CSVs
//! (`codec_type,stream_index,pts,dts,duration,size,keyframe`; `#` comment lines).

/// One oracle row. `pts`/`dts`/`duration`/`keyframe` are `None` for an empty or
/// `N/A` field; the strict accessors panic on `None` so a test that needs the
/// value fails with the row in the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketRow {
    pub codec_type: String,
    pub stream_index: u32,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub duration: Option<i64>,
    pub size: usize,
    pub keyframe: Option<bool>,
}

impl PacketRow {
    /// # Panics
    /// If the row carries no PTS.
    #[must_use]
    pub fn pts(&self) -> i64 {
        self.pts
            .unwrap_or_else(|| panic!("oracle row has no pts: {self:?}"))
    }

    /// # Panics
    /// If the row carries no DTS.
    #[must_use]
    pub fn dts(&self) -> i64 {
        self.dts
            .unwrap_or_else(|| panic!("oracle row has no dts: {self:?}"))
    }

    /// # Panics
    /// If the row carries no duration.
    #[must_use]
    pub fn duration(&self) -> i64 {
        self.duration
            .unwrap_or_else(|| panic!("oracle row has no duration: {self:?}"))
    }

    /// # Panics
    /// If the row carries no keyframe flag.
    #[must_use]
    pub fn is_keyframe(&self) -> bool {
        self.keyframe
            .unwrap_or_else(|| panic!("oracle row has no keyframe flag: {self:?}"))
    }
}

fn optional<T: std::str::FromStr>(field: &str, line: &str) -> Option<T> {
    let field = field.trim();
    if field.is_empty() || field == "N/A" {
        return None;
    }
    Some(
        field
            .parse()
            .unwrap_or_else(|_| panic!("bad oracle field {field:?} in row {line:?}")),
    )
}

/// Parse oracle CSV text.
///
/// # Panics
/// On a row with fewer than 7 columns or an unparsable field.
#[must_use]
pub fn parse_packets(text: &str) -> Vec<PacketRow> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            assert!(f.len() >= 7, "oracle row must have 7 columns: {line:?}");
            PacketRow {
                codec_type: f[0].trim().to_string(),
                stream_index: optional(f[1], line)
                    .unwrap_or_else(|| panic!("oracle row has no stream index: {line:?}")),
                pts: optional(f[2], line),
                dts: optional(f[3], line),
                duration: optional(f[4], line),
                size: optional(f[5], line)
                    .unwrap_or_else(|| panic!("oracle row has no size: {line:?}")),
                keyframe: optional::<u8>(f[6], line).map(|k| k == 1),
            }
        })
        .collect()
}

/// The oracle rows of `fixtures/<rel>`.
///
/// # Panics
/// If the fixture cannot be read or a row is malformed.
#[must_use]
pub fn packets(rel: &str) -> Vec<PacketRow> {
    parse_packets(&crate::read_to_string(rel))
}

/// The rows whose `codec_type` is `codec_type`, in file order.
#[must_use]
pub fn of_type<'a>(rows: &'a [PacketRow], codec_type: &str) -> Vec<&'a PacketRow> {
    rows.iter().filter(|r| r.codec_type == codec_type).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stamped_and_unstamped_rows_and_skips_comments() {
        assert!(parse_packets("# only a comment\n\n").is_empty());
        let rows = parse_packets("# c\nvideo,0,90,60,30,1234,1\naudio,1,N/A,,10,5,0\n\n");
        assert_eq!(
            rows,
            vec![
                PacketRow {
                    codec_type: "video".into(),
                    stream_index: 0,
                    pts: Some(90),
                    dts: Some(60),
                    duration: Some(30),
                    size: 1234,
                    keyframe: Some(true),
                },
                PacketRow {
                    codec_type: "audio".into(),
                    stream_index: 1,
                    pts: None,
                    dts: None,
                    duration: Some(10),
                    size: 5,
                    keyframe: Some(false),
                },
            ]
        );
        assert_eq!(of_type(&rows, "audio").len(), 1);
        assert_eq!(rows[0].pts(), 90);
    }

    #[test]
    #[should_panic(expected = "7 columns")]
    fn short_row_panics() {
        let _ = parse_packets("video,0,1\n");
    }

    #[test]
    #[should_panic(expected = "no pts")]
    fn strict_accessor_panics_on_missing_value() {
        let rows = parse_packets("audio,1,N/A,N/A,10,5,0\n");
        let _ = rows[0].pts();
    }
}
