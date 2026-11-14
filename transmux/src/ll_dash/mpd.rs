//! LL-DASH MPD signalling — the `std`-only half of [`crate::ll_dash`]: the
//! [`LlDashPackager`] builds the whole-segment MPD model with [`DashPackager`],
//! adds the low-latency attributes and `<ServiceDescription>` to it and renders
//! it through `broadcast-dash` (see the parent module's docs for the spec
//! clauses).

use alloc::string::{String, ToString};
use core::time::Duration;

use broadcast_dash::utc_timing::UtcScheme;
use broadcast_dash::{Latency, Mpd, PlaybackRate, ServiceDescription, UtcTiming};

use broadcast_common::Package;

use crate::dash::DashPackager;
use crate::error::{Error, Result};
use crate::media::Media;
use broadcast_common::units::MILLIS_PER_SEC;

/// `ServiceDescription@id` of the one description this packager emits.
const SERVICE_DESCRIPTION_ID: u64 = 0;

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
    /// Target end-to-end latency (`Latency@target`), written as whole
    /// milliseconds.
    pub latency_target: Duration,
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
        latency_target: Duration,
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
            latency_target,
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
        let mut mpd = self.base.build_mpd(media)?;
        self.apply_ll(&mut mpd);
        mpd.to_xml()
            .map_err(|_| Error::InvalidInput("MPD serialisation failed"))
    }
}

impl LlDashPackager {
    /// Add the LL-DASH signalling to the whole-segment `mpd`: a
    /// `ServiceDescription` (and the optional `UTCTiming`) and the two LL
    /// availability attributes on every `SegmentTemplate`.
    fn apply_ll(&self, mpd: &mut Mpd) {
        let ato = round_millis(self.availability_time_offset());
        for template in mpd
            .periods
            .iter_mut()
            .flat_map(|p| p.adaptation_sets.iter_mut())
            .flat_map(|a| a.representations.iter_mut())
            .filter_map(|r| r.segment.template.as_mut())
        {
            template.multi.base.availability_time_offset = Some(ato);
            template.multi.base.availability_time_complete = Some(false);
        }

        let mut latency = Latency::default();
        latency.target = Some(u64::try_from(self.latency_target.as_millis()).unwrap_or(u64::MAX));
        let mut sd = ServiceDescription::default();
        sd.id = Some(SERVICE_DESCRIPTION_ID);
        sd.latency = Some(latency);
        if let Some((min, max)) = self.playback_rate {
            let mut rate = PlaybackRate::default();
            rate.min = Some(format_secs(min));
            rate.max = Some(format_secs(max));
            sd.playback_rate = Some(rate);
        }
        mpd.service_descriptions.push(sd);

        if let Some((scheme, value)) = &self.utc_timing {
            let mut timing = UtcTiming::default();
            timing.scheme = UtcScheme::from_scheme_id_uri(scheme);
            timing.scheme_id_uri = scheme.clone();
            timing.value = value.clone();
            mpd.utc_timings.push(timing);
        }
    }
}

/// Round a seconds value to whole milliseconds.
fn round_millis(v: f64) -> f64 {
    (v * (MILLIS_PER_SEC as f64)).round() / (MILLIS_PER_SEC as f64)
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
    use broadcast_dash::{AdaptationSet, Period, Representation, SegmentTemplate};

    fn mpd_with_one_template() -> Mpd {
        let mut rep = Representation::default();
        rep.segment.template = Some(SegmentTemplate::default());
        let mut set = AdaptationSet::default();
        set.representations.push(rep);
        let mut period = Period::default();
        period.adaptation_sets.push(set);
        let mut mpd = Mpd::default();
        mpd.periods.push(period);
        mpd
    }

    #[test]
    fn apply_ll_sets_the_availability_fields_on_every_template() {
        let packager =
            LlDashPackager::new(4.0, 1.0, Duration::from_secs(3), "2026-01-01T00:00:00Z").unwrap();
        let mut mpd = mpd_with_one_template();
        packager.apply_ll(&mut mpd);
        let template = mpd.periods[0].adaptation_sets[0].representations[0]
            .segment
            .template
            .as_ref()
            .unwrap();
        assert_eq!(template.multi.base.availability_time_offset, Some(3.0));
        assert_eq!(template.multi.base.availability_time_complete, Some(false));
        assert_eq!(mpd.service_descriptions.len(), 1);
        assert_eq!(
            mpd.service_descriptions[0].latency.as_ref().unwrap().target,
            Some(3000)
        );
        assert!(mpd.utc_timings.is_empty());
    }

    #[test]
    fn apply_ll_adds_utc_timing_and_playback_rate_when_configured() {
        let packager =
            LlDashPackager::new(4.0, 1.0, Duration::from_secs(3), "2026-01-01T00:00:00Z")
                .unwrap()
                .with_playback_rate(0.96, 1.04)
                .with_utc_timing(UTCTIMING_DIRECT_2014, "2026-01-01T00:00:00Z");
        let mut mpd = mpd_with_one_template();
        packager.apply_ll(&mut mpd);
        let rate = mpd.service_descriptions[0].playback_rate.as_ref().unwrap();
        assert_eq!(rate.min.as_deref(), Some("0.96"));
        assert_eq!(rate.max.as_deref(), Some("1.04"));
        assert_eq!(mpd.utc_timings[0].value, "2026-01-01T00:00:00Z");
    }
}
