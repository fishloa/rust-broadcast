//! HLS `EXT-X-DATERANGE` model + (de)serialization.
//!
//! RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1. The `SCTE35-OUT`/`IN`/`CMD`
//! attribute value is the entire `splice_info_section`, hex-encoded with a `0x`
//! prefix.
use crate::error::{Error, Result};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

/// Which SCTE-35 attribute carries the splice on a DATERANGE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Scte35Cue {
    /// `SCTE35-OUT` — start of break.
    Out,
    /// `SCTE35-IN` — return from break.
    In,
    /// `SCTE35-CMD` — other splice command.
    Cmd,
}

impl Scte35Cue {
    /// Stable label.
    pub fn name(&self) -> &'static str {
        match self {
            Scte35Cue::Out => "out",
            Scte35Cue::In => "in",
            Scte35Cue::Cmd => "cmd",
        }
    }
    fn attr_key(&self) -> &'static str {
        match self {
            Scte35Cue::Out => "SCTE35-OUT",
            Scte35Cue::In => "SCTE35-IN",
            Scte35Cue::Cmd => "SCTE35-CMD",
        }
    }
}
broadcast_common::impl_spec_display!(Scte35Cue);

/// A SCTE-35 attribute on a DATERANGE: the cue kind plus the raw splice bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Scte35Attr {
    /// OUT / IN / CMD.
    pub cue: Scte35Cue,
    /// The verbatim `splice_info_section` bytes (emitted as `0x`-prefixed hex).
    pub raw: Vec<u8>,
}

/// An `EXT-X-DATERANGE` tag.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DateRange {
    /// `ID` (quoted).
    pub id: String,
    /// `START-DATE` (quoted, ISO-8601/RFC3339).
    pub start_date: String,
    /// `CLASS` (quoted), if present.
    pub class: Option<String>,
    /// `DURATION` in seconds.
    pub duration: Option<f64>,
    /// `PLANNED-DURATION` in seconds.
    pub planned_duration: Option<f64>,
    /// SCTE-35 attribute, if present.
    pub scte35: Option<Scte35Attr>,
}

// DateRange carries f64 fields, so it is `PartialEq` only (no `Eq`). Tests
// compare values the crate produced, so equality is deterministic in practice.

const TAG: &str = "#EXT-X-DATERANGE:";

impl DateRange {
    /// Serialize to a single `#EXT-X-DATERANGE:` line. Attribute order is fixed
    /// (ID, START-DATE, CLASS, DURATION, PLANNED-DURATION, SCTE35-*) so that
    /// `parse_tag_line` round-trips byte-identically.
    pub fn to_tag_line(&self) -> String {
        let mut out = String::from(TAG);
        out.push_str(&format!("ID=\"{}\"", self.id));
        out.push_str(&format!(",START-DATE=\"{}\"", self.start_date));
        if let Some(c) = &self.class {
            out.push_str(&format!(",CLASS=\"{}\"", c));
        }
        if let Some(d) = self.duration {
            out.push_str(&format!(",DURATION={}", fmt_f64(d)));
        }
        if let Some(d) = self.planned_duration {
            out.push_str(&format!(",PLANNED-DURATION={}", fmt_f64(d)));
        }
        if let Some(s) = &self.scte35 {
            out.push_str(&format!(",{}=0x{}", s.cue.attr_key(), to_hex_upper(&s.raw)));
        }
        out
    }

    /// Parse one `#EXT-X-DATERANGE:` line.
    pub fn parse_tag_line(s: &str) -> Result<DateRange> {
        let body = s
            .strip_prefix(TAG)
            .ok_or_else(|| Error::AttrParse("missing #EXT-X-DATERANGE: prefix".to_string()))?;
        let mut dr = DateRange {
            id: String::new(),
            start_date: String::new(),
            class: None,
            duration: None,
            planned_duration: None,
            scte35: None,
        };
        let mut seen_id = false;
        for (k, v) in split_attrs(body) {
            match k {
                "ID" => {
                    dr.id = unquote(v);
                    seen_id = true;
                }
                "START-DATE" => dr.start_date = unquote(v),
                "CLASS" => dr.class = Some(unquote(v)),
                "DURATION" => dr.duration = Some(parse_f64(v)?),
                "PLANNED-DURATION" => dr.planned_duration = Some(parse_f64(v)?),
                "SCTE35-OUT" => {
                    dr.scte35 = Some(Scte35Attr {
                        cue: Scte35Cue::Out,
                        raw: parse_hex(v)?,
                    })
                }
                "SCTE35-IN" => {
                    dr.scte35 = Some(Scte35Attr {
                        cue: Scte35Cue::In,
                        raw: parse_hex(v)?,
                    })
                }
                "SCTE35-CMD" => {
                    dr.scte35 = Some(Scte35Attr {
                        cue: Scte35Cue::Cmd,
                        raw: parse_hex(v)?,
                    })
                }
                _ => {} // unknown attributes ignored (spec-extensible)
            }
        }
        if !seen_id {
            return Err(Error::AttrParse("DATERANGE missing ID".to_string()));
        }
        Ok(dr)
    }
}

fn fmt_f64(v: f64) -> String {
    // Integer-valued durations render without a trailing ".0" to match common output.
    // Avoid f64::fract() (std-only intrinsic in no_std); use cast comparison instead.
    let trunc = v as i64;
    if v == trunc as f64 {
        format!("{}", trunc)
    } else {
        format!("{}", v)
    }
}

fn to_hex_upper(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        s.push_str(&format!("{:02X}", byte));
    }
    s
}

fn unquote(v: &str) -> String {
    v.trim_matches('"').to_string()
}

fn parse_f64(v: &str) -> Result<f64> {
    v.parse::<f64>()
        .map_err(|_| Error::AttrParse(format!("bad number: {v}")))
}

fn parse_hex(v: &str) -> Result<Vec<u8>> {
    let h = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);
    if h.len() % 2 != 0 {
        return Err(Error::AttrParse("odd-length hex".to_string()));
    }
    (0..h.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&h[i..i + 2], 16)
                .map_err(|_| Error::AttrParse("bad hex".to_string()))
        })
        .collect()
}

/// Split `K=V,K=V` honouring quoted values (commas inside quotes are not separators).
fn split_attrs(body: &str) -> Vec<(&str, &str)> {
    let mut pairs = Vec::new();
    let bytes = body.as_bytes();
    let (mut start, mut in_q) = (0usize, false);
    let mut i = 0;
    while i <= bytes.len() {
        let at_end = i == bytes.len();
        let c = if at_end { b',' } else { bytes[i] };
        match c {
            b'"' => in_q = !in_q,
            b',' if !in_q => {
                let field = &body[start..i];
                if let Some(eq) = field.find('=') {
                    pairs.push((&field[..eq], &field[eq + 1..]));
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, vec};

    fn sample() -> DateRange {
        DateRange {
            id: "2002".to_string(),
            start_date: "2018-10-29T10:38:00.000Z".to_string(),
            class: None,
            duration: None,
            planned_duration: Some(24.0),
            scte35: Some(Scte35Attr {
                cue: Scte35Cue::Out,
                raw: vec![0xFC, 0x30, 0x21],
            }),
        }
    }

    #[test]
    fn tag_round_trips_byte_identical() {
        let dr = sample();
        let line = dr.to_tag_line();
        assert!(line.starts_with("#EXT-X-DATERANGE:"));
        assert!(line.contains("SCTE35-OUT=0xFC3021"));
        let back = DateRange::parse_tag_line(&line).unwrap();
        assert_eq!(back, dr);
    }

    #[test]
    fn cue_labels() {
        assert_eq!(Scte35Cue::Out.name(), "out");
        assert_eq!(alloc::format!("{}", Scte35Cue::In), "in");
    }
}
