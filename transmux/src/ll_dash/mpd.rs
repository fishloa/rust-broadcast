//! LL-DASH MPD signalling — the `std`-only half of [`crate::ll_dash`]: the
//! [`LlDashPackager`] renders a whole-segment MPD with [`DashPackager`], then
//! rewrites it with `quick-xml` to add the low-latency attributes and
//! `<ServiceDescription>` (see the parent module's docs for the spec clauses).

use alloc::string::{String, ToString};

use quick_xml::NsReader;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use broadcast_common::Package;
use broadcast_common::xml_chars::namespace_of;

use crate::dash::{DashPackager, MPD_NAMESPACE};
use crate::error::{Error, Result};
use crate::media::Media;

/// Local name of the MPD root element (ISO/IEC 23009-1 §5.3.1.2).
const MPD_ELEMENT: &str = "MPD";
/// Local name of `SegmentTemplate` (ISO/IEC 23009-1 §5.3.9.4).
const SEGMENT_TEMPLATE: &str = "SegmentTemplate";
/// Reported when the base MPD this packager just rendered fails to re-parse
/// (an invariant violation, surfaced as an error rather than a panic).
const BASE_MPD_NOT_WELL_FORMED: &str = "base MPD is not well-formed XML";

// ===========================================================================
// LlDashPackager — low-latency DASH MPD
// ===========================================================================

/// Render a **low-latency** MPEG-DASH MPD (ISO/IEC 23009-1 + DASH-IF LL IOP).
///
/// Wraps the whole-segment [`DashPackager`] and post-processes its XML to add the
/// LL-DASH availability signalling: `availabilityTimeComplete="false"` +
/// `availabilityTimeOffset` on each `SegmentTemplate`, and a top-level
/// `<ServiceDescription>` with a `<Latency>` target (+ optional `<PlaybackRate>`).
/// Always emits `type="dynamic"` with an `availabilityStartTime`.
///
/// See the module docs for the exact spec clauses and why
/// `<ProducerReferenceTime>` is omitted.
#[derive(Debug, Clone)]
pub struct LlDashPackager {
    /// The underlying whole-segment packager (forced `dynamic` on package).
    pub base: DashPackager,
    /// Nominal segment duration in seconds (the `availabilityTimeOffset` base).
    pub segment_duration_secs: f64,
    /// Chunk duration in seconds. `availabilityTimeOffset = segment − chunk`.
    pub chunk_duration_secs: f64,
    /// Target end-to-end latency in **milliseconds** (`Latency@target`).
    pub latency_target_ms: u32,
    /// Optional catch-up playback rate bounds (`PlaybackRate@min`/`@max`).
    pub playback_rate: Option<(f64, f64)>,
    /// `MPD/UTCTiming` (§5.8.4.11) as `(schemeIdUri, value)`: the registered
    /// scheme the client uses to obtain wall-clock UTC (e.g.
    /// [`UTCTIMING_HTTP_HEAD_2014`] with an HTTP(S) URL whose `Date` header is the
    /// time source), and the scheme's argument. `None` omits the element.
    pub utc_timing: Option<(String, String)>,
}

/// `urn:mpeg:dash:utc:http-head:2014` (ISO/IEC 23009-1 §5.8.4.11, Table 30): the
/// `@value` is an HTTP(S) URL whose response `Date:` header carries the time.
pub const UTCTIMING_HTTP_HEAD_2014: &str = "urn:mpeg:dash:utc:http-head:2014";
/// `urn:mpeg:dash:utc:http-xsdate:2014` (ISO/IEC 23009-1 §5.8.5.7): the
/// `@value` URL returns an XSdate body.
pub const UTCTIMING_HTTP_XSDATE_2014: &str = "urn:mpeg:dash:utc:http-xsdate:2014";
/// `urn:mpeg:dash:utc:http-iso:2014` (§5.8.5.7): the `@value` URL returns an
/// ISO 8601 date-time body.
pub const UTCTIMING_HTTP_ISO_2014: &str = "urn:mpeg:dash:utc:http-iso:2014";
/// `urn:mpeg:dash:utc:ntp:2014` (§5.8.5.7): the `@value` is a white-space
/// separated list of NTP servers.
pub const UTCTIMING_NTP_2014: &str = "urn:mpeg:dash:utc:ntp:2014";
/// `urn:mpeg:dash:utc:http-ntp:2014` (§5.8.5.7): the `@value` URL returns an NTP
/// timestamp body.
pub const UTCTIMING_HTTP_NTP_2014: &str = "urn:mpeg:dash:utc:http-ntp:2014";
/// `urn:mpeg:dash:utc:direct:2014` (§5.8.5.7): the `@value` is itself the UTC
/// time, as an XSdate.
pub const UTCTIMING_DIRECT_2014: &str = "urn:mpeg:dash:utc:direct:2014";
impl LlDashPackager {
    /// Build an LL-DASH packager. `availability_start_time` is the wall-clock
    /// `MPD@availabilityStartTime` (ISO-8601 UTC). The `availabilityTimeOffset`
    /// is derived as `segment_duration_secs − chunk_duration_secs`.
    /// `MPD@publishTime` (mandatory for a dynamic MPD, ISO/IEC 23009-1 §5.3.1.2
    /// Table 3) is not defaulted: set `base.publish_time`, and advance it on
    /// every updated MPD.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if the durations are not positive/finite, or the
    /// chunk duration exceeds the segment duration (no LL benefit / negative ATO).
    pub fn new(
        segment_duration_secs: f64,
        chunk_duration_secs: f64,
        latency_target_ms: u32,
        availability_start_time: impl Into<String>,
    ) -> Result<Self> {
        if !(segment_duration_secs.is_finite() && segment_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "segment_duration_secs must be positive and finite",
            ));
        }
        if !(chunk_duration_secs.is_finite() && chunk_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "chunk_duration_secs must be positive and finite",
            ));
        }
        if chunk_duration_secs > segment_duration_secs {
            return Err(Error::InvalidInput(
                "chunk_duration_secs must not exceed segment_duration_secs",
            ));
        }
        let base = DashPackager {
            dynamic: true,
            availability_start_time: Some(availability_start_time.into()),
            ..DashPackager::default()
        };
        Ok(Self {
            base,
            segment_duration_secs,
            chunk_duration_secs,
            latency_target_ms,
            playback_rate: None,
            utc_timing: None,
        })
    }

    /// Set the optional catch-up `<PlaybackRate min max>` (DASH-IF LL IOP).
    pub fn with_playback_rate(mut self, min: f64, max: f64) -> Self {
        self.playback_rate = Some((min, max));
        self
    }

    /// Set the `MPD/UTCTiming` element (ISO/IEC 23009-1 §5.8.4.11): `scheme` is
    /// one of the registered `urn:mpeg:dash:utc:*:2014` values (see the
    /// [`UTCTIMING_HTTP_HEAD_2014`] family) and `value` its argument (a URL, an
    /// NTP host list, or — for `direct` — the UTC time itself). Without it a
    /// `dynamic` MPD's `availabilityStartTime` is unusable to a client whose
    /// clock does not already agree with the server's (audit r05-W27c).
    pub fn with_utc_timing(mut self, scheme: impl Into<String>, value: impl Into<String>) -> Self {
        self.utc_timing = Some((scheme.into(), value.into()));
        self
    }

    /// The `availabilityTimeOffset` in seconds (`segment − chunk`, DASH-IF LL IOP
    /// §4.3). Always `>= 0` by construction.
    pub fn availability_time_offset(&self) -> f64 {
        (self.segment_duration_secs - self.chunk_duration_secs).max(0.0)
    }
}

impl Package for LlDashPackager {
    type Media = Media;
    type Output = String;
    type Error = Error;

    /// Render the LL-DASH MPD for `media`.
    ///
    /// # Errors
    /// Propagates [`DashPackager`] errors (e.g. empty track list, or no
    /// `base.publish_time`: the LL-DASH MPD is always `dynamic`).
    fn package(&mut self, media: &Media) -> Result<String> {
        let base_xml = self.base.package(media)?;
        self.inject_ll(&base_xml)
    }
}

impl LlDashPackager {
    /// Post-process the base MPD XML: add `<ServiceDescription>` (and the
    /// optional `<UTCTiming>`) as the first children of `<MPD>`, and the LL
    /// attributes onto every `<SegmentTemplate>`.
    ///
    /// The base MPD is re-read with `quick-xml` and re-emitted event by event
    /// (every untouched tag and attribute passes through verbatim; text is
    /// trimmed at run edges, see below), so the rewrite never depends on the base writer's line layout.
    fn inject_ll(&self, xml: &str) -> Result<String> {
        let ato_str = format_secs(self.availability_time_offset());

        let mut reader = NsReader::from_str(xml);
        // Whitespace-only text between elements is dropped (the indenting writer
        // re-lays it out); a text run split by a reference loses its edge spaces,
        // which is harmless because the base writer only emits base64 text.
        reader.config_mut().trim_text(true);
        let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
        // Writing to an in-memory `Vec` cannot fail, so write results are
        // intentionally discarded.
        let mut utc_written = false;
        loop {
            let (resolved, event) = reader
                .read_resolved_event()
                .map_err(|_| Error::InvalidInput(BASE_MPD_NOT_WELL_FORMED))?;
            let in_mpd = matches!(
                namespace_of(resolved),
                Ok(Some(ref ns)) if ns == MPD_NAMESPACE
            );
            match event {
                Event::Eof => break,
                // 1. availabilityTimeComplete + availabilityTimeOffset on every SegmentTemplate
                Event::Start(e) if in_mpd && e.local_name().as_ref() == SEGMENT_TEMPLATE => {
                    let _ = writer.write_event(Event::Start(self.ll_template(&e, &ato_str)));
                }
                Event::Empty(e) if in_mpd && e.local_name().as_ref() == SEGMENT_TEMPLATE => {
                    let _ = writer.write_event(Event::Empty(self.ll_template(&e, &ato_str)));
                }
                // 2. Insert <ServiceDescription> immediately after the <MPD ...> open tag.
                Event::Start(e) if in_mpd && e.local_name().as_ref() == MPD_ELEMENT => {
                    let _ = writer.write_event(Event::Start(e));
                    self.write_service_description(&mut writer);
                }
                // 3. <UTCTiming> is a direct child of <MPD> but the schema places it
                //    after <Period>, so emit it once the Period closes.
                Event::End(e) if in_mpd && e.local_name().as_ref() == "Period" && !utc_written => {
                    let _ = writer.write_event(Event::End(e));
                    if let Some((scheme, value)) = &self.utc_timing {
                        let mut timing = BytesStart::new("UTCTiming");
                        timing.push_attribute(("schemeIdUri", scheme.as_str()));
                        timing.push_attribute(("value", value.as_str()));
                        let _ = writer.write_event(Event::Empty(timing));
                    }
                    utc_written = true;
                }
                other => {
                    let _ = writer.write_event(other);
                }
            }
        }
        let mut out = String::from_utf8_lossy(&writer.into_inner()).into_owned();
        out.push('\n');
        Ok(out)
    }

    /// `tag` with the two LL attributes appended (after the existing ones).
    fn ll_template<'a>(&self, tag: &BytesStart<'a>, ato: &str) -> BytesStart<'a> {
        let mut out = tag.to_owned();
        out.push_attribute(("availabilityTimeOffset", ato));
        out.push_attribute(("availabilityTimeComplete", "false"));
        out
    }

    /// Write the `<ServiceDescription>` block, then `<UTCTiming>`.
    fn write_service_description(&self, w: &mut Writer<Vec<u8>>) {
        let mut sd = BytesStart::new("ServiceDescription");
        sd.push_attribute(("id", "0"));
        let _ = w.write_event(Event::Start(sd));
        let mut latency = BytesStart::new("Latency");
        latency.push_attribute(("target", self.latency_target_ms.to_string().as_str()));
        let _ = w.write_event(Event::Empty(latency));
        if let Some((min, max)) = self.playback_rate {
            let mut rate = BytesStart::new("PlaybackRate");
            rate.push_attribute(("min", format_secs(min).as_str()));
            rate.push_attribute(("max", format_secs(max).as_str()));
            let _ = w.write_event(Event::Empty(rate));
        }
        let _ = w.write_event(Event::End(BytesEnd::new("ServiceDescription")));
    }
}

/// Format a non-negative seconds value with up to three decimal places, trailing
/// zeros trimmed (e.g. `1.5`, `2`, `0.033`). Integer math only — no `std` float
/// formatting intrinsic beyond core `Display`.
fn format_secs(v: f64) -> String {
    let millis = (v * 1000.0 + 0.5) as u64;
    let whole = millis / 1000;
    let frac = millis % 1000;
    if frac == 0 {
        return whole.to_string();
    }
    let mut f = format!("{frac:03}");
    while f.ends_with('0') {
        f.pop();
    }
    format!("{whole}.{f}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

    fn packager() -> LlDashPackager {
        LlDashPackager::new(4.0, 1.0, 3000, "2026-01-01T00:00:00Z").unwrap()
    }

    #[test]
    fn foreign_namespace_look_alikes_are_passed_through_untouched() {
        let xml = format!(
            r#"<MPD xmlns="{NS}" xmlns:x="urn:x"><x:MPD/><Period><x:SegmentTemplate/><SegmentTemplate/></Period><x:Period/></MPD>"#
        );
        let out = packager().inject_ll(&xml).unwrap();
        assert_eq!(out.matches("availabilityTimeOffset").count(), 1, "{out}");
        assert!(out.contains("<x:SegmentTemplate/>"), "{out}");
        assert_eq!(out.matches("<ServiceDescription").count(), 1, "{out}");
    }

    #[test]
    fn a_foreign_mpd_root_gets_no_service_description() {
        let xml = r#"<MPD xmlns="urn:other"><Period><SegmentTemplate/></Period></MPD>"#;
        let out = packager().inject_ll(xml).unwrap();
        assert!(!out.contains("ServiceDescription"), "{out}");
        assert!(!out.contains("availabilityTimeOffset"), "{out}");
    }

    #[test]
    fn utc_timing_follows_only_a_period_in_the_mpd_namespace() {
        let xml = format!(
            r#"<MPD xmlns="{NS}" xmlns:x="urn:x"><x:Period></x:Period><Period><SegmentTemplate/></Period></MPD>"#
        );
        let out = packager()
            .with_utc_timing(UTCTIMING_DIRECT_2014, "2026-01-01T00:00:00Z")
            .inject_ll(&xml)
            .unwrap();
        let timing = out.find("<UTCTiming").unwrap();
        let foreign = out.find("</x:Period>").unwrap();
        let own = out.rfind("<Period").unwrap();
        assert!(foreign < own && own < timing, "{out}");
        assert_eq!(out.matches("<UTCTiming").count(), 1, "{out}");
    }
}
