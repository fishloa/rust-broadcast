//! HLS `EXT-X-DATERANGE` model + (de)serialization.
//!
//! RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1. The `SCTE35-OUT`/`IN`/`CMD`
//! attribute value is the entire `splice_info_section`, hex-encoded with a `0x`
//! prefix.
use crate::error::{Error, Result};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use broadcast_hls::{AttrValue, parse_attribute_list, render_attribute_list};

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
    /// Every other attribute (e.g. a caller's own `X-COM-EXAMPLE-AD-ID`, or
    /// `END-DATE`/`END-ON-NEXT`) not modeled as a typed field above,
    /// preserved losslessly for round-trip (issue #1140 / audit
    /// r12-TM-W4 — previously dropped on parse). Sorted by name on parse
    /// (deterministic); rendered after the fixed-order fields above.
    pub extra_attrs: Vec<(String, AttrValue)>,
}

// DateRange carries f64 fields, so it is `PartialEq` only (no `Eq`). Tests
// compare values the crate produced, so equality is deterministic in practice.

const TAG: &str = "#EXT-X-DATERANGE:";

impl DateRange {
    /// Serialize to a single `#EXT-X-DATERANGE:` line. Attribute order is fixed
    /// (ID, START-DATE, CLASS, DURATION, PLANNED-DURATION, SCTE35-*) so that
    /// `parse_tag_line` round-trips byte-identically.
    ///
    /// Every attribute value goes through [`AttrValue::quoted`]/[`bare`] and
    /// is rendered with the shared [`broadcast_hls::render_attribute_list`]
    /// (issue #1140 / audit r12-TM-W4): `id`/`class` are frequently sourced
    /// from an upstream SCTE-35 segmentation descriptor's
    /// `segmentation_upid` (caller/network data), so a `"`, CR or LF in
    /// either is rejected here rather than breaking the attribute list.
    /// `duration`/`planned_duration` must also be finite and non-negative —
    /// `Error::InvalidDuration` otherwise (NaN previously rendered as the
    /// bare token `NaN`).
    ///
    /// [`bare`]: AttrValue::bare
    pub fn to_tag_line(&self) -> Result<String> {
        let mut attrs = vec![
            (String::from("ID"), AttrValue::quoted(self.id.clone())?),
            (
                String::from("START-DATE"),
                AttrValue::quoted(self.start_date.clone())?,
            ),
        ];
        if let Some(c) = &self.class {
            attrs.push((String::from("CLASS"), AttrValue::quoted(c.clone())?));
        }
        if let Some(d) = self.duration {
            attrs.push((
                String::from("DURATION"),
                AttrValue::bare(checked_fmt_f64("DURATION", d)?)?,
            ));
        }
        if let Some(d) = self.planned_duration {
            attrs.push((
                String::from("PLANNED-DURATION"),
                AttrValue::bare(checked_fmt_f64("PLANNED-DURATION", d)?)?,
            ));
        }
        if let Some(s) = &self.scte35 {
            // A hex token this crate itself formats, never caller-freeform
            // text, so `bare` cannot fail.
            attrs.push((
                String::from(s.cue.attr_key()),
                AttrValue::bare(format!("0x{}", to_hex_upper(&s.raw)))?,
            ));
        }
        // Unknown attributes preserved from parse (or set programmatically)
        // — already validated `AttrValue`s, so nothing to check here.
        attrs.extend(self.extra_attrs.iter().cloned());
        // `render_attribute_list` always prefixes each entry with `,`
        // (broadcast-hls's convention for appending to an already-started
        // attribute list); build the whole list through it and drop the
        // single leading comma rather than special-casing the first entry.
        let mut body = String::new();
        render_attribute_list(&mut body, &attrs);
        let mut out = String::from(TAG);
        out.push_str(body.trim_start_matches(','));
        Ok(out)
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
            extra_attrs: Vec::new(),
        };
        // The shared workspace tokenizer (issue #1140 T12): quoted values
        // already have their surrounding `"` stripped.
        let (map, quoted) = parse_attribute_list(body);
        if let Some(v) = map.get("ID") {
            dr.id = v.clone();
        } else {
            return Err(Error::AttrParse("DATERANGE missing ID".to_string()));
        }
        if let Some(v) = map.get("START-DATE") {
            dr.start_date = v.clone();
        }
        if let Some(v) = map.get("CLASS") {
            dr.class = Some(v.clone());
        }
        if let Some(v) = map.get("DURATION") {
            dr.duration = Some(parse_checked_f64("DURATION", v)?);
        }
        if let Some(v) = map.get("PLANNED-DURATION") {
            dr.planned_duration = Some(parse_checked_f64("PLANNED-DURATION", v)?);
        }
        if let Some(v) = map.get("SCTE35-OUT") {
            dr.scte35 = Some(Scte35Attr {
                cue: Scte35Cue::Out,
                raw: parse_hex(v)?,
            });
        } else if let Some(v) = map.get("SCTE35-IN") {
            dr.scte35 = Some(Scte35Attr {
                cue: Scte35Cue::In,
                raw: parse_hex(v)?,
            });
        } else if let Some(v) = map.get("SCTE35-CMD") {
            dr.scte35 = Some(Scte35Attr {
                cue: Scte35Cue::Cmd,
                raw: parse_hex(v)?,
            });
        }
        // Every other attribute (X-*, END-DATE, END-ON-NEXT, …) is kept
        // losslessly, not dropped (issue #1140 / audit r12-TM-W4).
        dr.extra_attrs = filter_extra_attrs(&map, &quoted)?;
        Ok(dr)
    }
}

/// From an already-parsed attribute map, collect every attribute whose name
/// is not one of `DateRange`'s typed fields into a `Vec<(name, AttrValue)>`,
/// sorted by name for deterministic serialization (same pattern as
/// `broadcast_hls`'s own `filter_extra_attrs` — issue #1140 / audit
/// r12-TM-W4). `quoted` decides [`AttrValue::quoted`] vs. [`AttrValue::bare`]
/// per entry: the recorded wire form, not a guess.
fn filter_extra_attrs(
    map: &BTreeMap<String, String>,
    quoted: &BTreeSet<String>,
) -> Result<Vec<(String, AttrValue)>> {
    const KNOWN: &[&str] = &[
        "ID",
        "START-DATE",
        "CLASS",
        "DURATION",
        "PLANNED-DURATION",
        "SCTE35-OUT",
        "SCTE35-IN",
        "SCTE35-CMD",
    ];
    map.iter()
        .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
        .map(|(k, v)| {
            let value = if quoted.contains(k) {
                AttrValue::quoted(v.clone())?
            } else {
                AttrValue::bare(v.clone())?
            };
            Ok((k.clone(), value))
        })
        .collect()
}

/// Format a non-negative, finite seconds value, rejecting NaN/infinite/
/// negative `v` (issue #1140 / audit r13-BH-W4-class): an unchecked format
/// would render `DURATION=NaN`, syntax the §4.2 grammar has no notation
/// for.
fn checked_fmt_f64(what: &'static str, v: f64) -> Result<String> {
    if !v.is_finite() || v < 0.0 {
        return Err(Error::InvalidDuration { what, value: v });
    }
    // Integer-valued durations render without a trailing ".0" to match common output.
    // Avoid f64::fract() (std-only intrinsic in no_std); use cast comparison instead.
    let trunc = v as i64;
    Ok(if v == trunc as f64 {
        format!("{trunc}")
    } else {
        format!("{v}")
    })
}

/// Parse a decimal-floating-point seconds attribute, rejecting a
/// NaN/infinite/negative value at parse time rather than letting it reach
/// [`checked_fmt_f64`] on the next render.
fn parse_checked_f64(what: &'static str, v: &str) -> Result<f64> {
    let parsed: f64 = v
        .parse()
        .map_err(|_| Error::AttrParse(format!("bad number: {v}")))?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(Error::InvalidDuration {
            what,
            value: parsed,
        });
    }
    Ok(parsed)
}

fn to_hex_upper(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        s.push_str(&format!("{byte:02X}"));
    }
    s
}

fn parse_hex(v: &str) -> Result<Vec<u8>> {
    let h = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .unwrap_or(v);
    if !h.len().is_multiple_of(2) {
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
            extra_attrs: Vec::new(),
        }
    }

    #[test]
    fn tag_round_trips_byte_identical() {
        let dr = sample();
        let line = dr.to_tag_line().unwrap();
        assert!(line.starts_with("#EXT-X-DATERANGE:"));
        assert!(line.contains("SCTE35-OUT=0xFC3021"));
        let back = DateRange::parse_tag_line(&line).unwrap();
        assert_eq!(back, dr);
    }

    /// Audit r12-TM-W4 (issue #1140): an unknown quoted-string attribute
    /// (`X-COM-EXAMPLE-AD-ID`) and an unknown bare/hex attribute (`X-FOO`)
    /// must survive parse -> render byte-identically, not be dropped.
    #[test]
    fn unknown_attributes_round_trip_byte_identical() {
        let mut dr = sample();
        dr.extra_attrs = vec![
            (
                "X-COM-EXAMPLE-AD-ID".to_string(),
                AttrValue::quoted("ad-42").unwrap(),
            ),
            ("X-FOO".to_string(), AttrValue::bare("0x1A").unwrap()),
        ];
        let line = dr.to_tag_line().unwrap();
        assert!(line.contains(r#"X-COM-EXAMPLE-AD-ID="ad-42""#));
        assert!(line.contains("X-FOO=0x1A"));
        let back = DateRange::parse_tag_line(&line).unwrap();
        assert_eq!(back, dr, "unknown attributes must round-trip");
        assert_eq!(back.extra_attrs, dr.extra_attrs);
    }

    #[test]
    fn cue_labels() {
        assert_eq!(Scte35Cue::Out.name(), "out");
        assert_eq!(alloc::format!("{}", Scte35Cue::In), "in");
    }

    /// Audit r12-TM-W4: a `"`, CR or LF in `ID` (frequently sourced from an
    /// upstream `segmentation_upid`) must be rejected, not injected into
    /// the DATERANGE attribute list.
    #[test]
    fn to_tag_line_rejects_injection_in_id() {
        let mut dr = sample();
        dr.id = "2002\"\r\n#EXT-X-ENDLIST".to_string();
        assert!(matches!(
            dr.to_tag_line().unwrap_err(),
            Error::HlsAttrValue(_)
        ));
    }

    #[test]
    fn to_tag_line_rejects_injection_in_class() {
        let mut dr = sample();
        dr.class = Some("ad\"break".to_string());
        assert!(matches!(
            dr.to_tag_line().unwrap_err(),
            Error::HlsAttrValue(_)
        ));
    }

    /// Audit r13-BH-W4-class: NaN/infinite/negative durations must be
    /// rejected, never rendered as the bare token `NaN`/`inf`.
    #[test]
    fn to_tag_line_rejects_non_finite_and_negative_durations() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let mut dr = sample();
            dr.duration = Some(bad);
            assert!(matches!(
                dr.to_tag_line().unwrap_err(),
                Error::InvalidDuration { .. }
            ));

            let mut dr = sample();
            dr.planned_duration = Some(bad);
            assert!(matches!(
                dr.to_tag_line().unwrap_err(),
                Error::InvalidDuration { .. }
            ));
        }
    }

    #[test]
    fn parse_rejects_non_finite_duration() {
        let line = "#EXT-X-DATERANGE:ID=\"x\",START-DATE=\"2020-01-01T00:00:00Z\",DURATION=nan";
        assert!(matches!(
            DateRange::parse_tag_line(line).unwrap_err(),
            Error::InvalidDuration { .. }
        ));
    }
}
