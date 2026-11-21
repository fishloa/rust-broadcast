//! Per-route DVR configuration. The archive itself (its on-disk format, the
//! writer's roll and retention decisions, and catch-up read planning) is
//! [`dvr_runtime`]'s; this module only turns an operator's [`DvrConfig`] into
//! a [`dvr_runtime::ArchiveConfig`] and names the archive of a route.
//!
//! # On-disk layout
//!
//! `<archive_root>/<route_name>/` holds one media file (`pN.m4s` or `pN.ts`)
//! and one HLS media playlist (`pN.m3u8`) per period: the playlist indexes the
//! media file with `EXT-X-BYTERANGE`, carries the exact Trunk timing of each
//! segment in a vendor tag, and marks programme boundaries with
//! `EXT-X-DATERANGE`. See the `dvr-runtime` crate documentation for the
//! format. Archives written by multimux 0.20 and earlier (`pN.idx` JSON
//! sidecars) are not readable.
//!
//! # Period lifecycle
//!
//! A new period starts when recording starts, when the open period has been
//! open for `period_duration_secs`, when the fMP4 init changes, when the
//! tracked service's EIT present event changes (`dvb_service_id`), when the
//! byte budget of `retention_bytes` would be exceeded, and on a sequence gap.
//! The time-based period stays the hard cap even with `dvb_service_id` set,
//! so an EPG that never signals a transition cannot produce an unbounded
//! recording.
//!
//! # Retention
//!
//! Whole periods: `retention_periods` keeps at most that many closed
//! periods, `retention_bytes` at most that many bytes across every period
//! (the open one included). The playlist of an evicted period is removed
//! before its media file.
//!
//! # `ArchiveOverrun` in operator terms
//!
//! The per-route `overrun` field (default: `"gap"`) surfaces the three-way
//! trade from [`media_plane::trunk::ArchiveOverrun`]:
//!
//! - **`"gap"`** (default): when the live ring evicts a segment the recorder
//!   has not yet consumed, the recording gets a hole (the next segment opens
//!   a new period). The archive is incomplete but live ingest is unaffected.
//! - **`"stall"`**: the recorder applies back-pressure: segment publication
//!   blocks until the recorder consumes far enough. The archive is lossless,
//!   but a slow disk can stall live output for every viewer. A pin
//!   force-expired by the safety valve re-arms at the live edge and the
//!   segments produced meanwhile are lost (`multimux_dvr_pin_rearmed_total`).
//! - **`"terminate"`**: drop the recorder's pin when the live ring overruns;
//!   recording stops and the files already written are kept.
//!
//! # Recording does not perturb live serving
//!
//! The recorder drains its own pinning `SegmentCursor`, reading the same
//! `SegmentEntry` values every other cursor reads, and runs on the blocking
//! pool, never under a lock the live path needs.

use std::num::{NonZeroU64, NonZeroUsize};
use std::path::Path;
use std::time::Duration;

use dvr_runtime::fs::FsArchive;
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, ConfigError, RetentionLimits};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::ArchiveOverrun;
use serde::Deserialize;
use transmux::Container;

use crate::error::{ConfigField, MultimuxError};

/// Default period duration in seconds: **3 hours** (10800 s).
///
/// Covers a feature film in a single period. Retention quantises to the
/// period and a truncation costs up to one period; an operator can choose
/// otherwise via `period_duration_secs`.
const DEFAULT_PERIOD_DURATION_SECS: u64 = 10800;

/// Per-route DVR configuration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DvrConfig {
    /// Enable DVR recording for this route. `false` by default: recording
    /// must be explicitly opted in.
    #[serde(default)]
    pub enabled: bool,
    /// Filesystem path under which each route's archive directory is
    /// created. Required when `enabled` is `true`.
    #[serde(default)]
    pub archive_root: String,
    /// Period duration in seconds. Default: **10800** (3 hours). `0` means
    /// "never roll over by duration", accepted only when `dvb_service_id` or
    /// `retention_bytes` bounds the period instead.
    #[serde(default = "default_period_duration_secs")]
    pub period_duration_secs: u64,
    /// Keep at most this many closed periods; `0` disables count-based
    /// retention.
    #[serde(default)]
    pub retention_periods: usize,
    /// Keep at most this many total bytes across every period; `0` disables
    /// byte-based retention.
    #[serde(default)]
    pub retention_bytes: u64,
    /// The [`ArchiveOverrun`] policy of the recorder's pinning cursor; see
    /// [the module docs](self#archiveoverrun-in-operator-terms).
    #[serde(default)]
    pub overrun: ArchiveOverrunSerde,
    /// The `service_id` whose EIT present/following *actual* section
    /// (ETSI EN 300 468 §5.2.4, `table_id` `0x4E`) marks programme
    /// boundaries: a new present event rolls the period and is recorded as
    /// an `EXT-X-DATERANGE`. `None` (default) disables programme-aligned
    /// rolling.
    #[serde(default)]
    pub dvb_service_id: Option<u16>,
}

impl Default for DvrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            archive_root: String::new(),
            period_duration_secs: DEFAULT_PERIOD_DURATION_SECS,
            retention_periods: 0,
            retention_bytes: 0,
            overrun: ArchiveOverrunSerde::Gap,
            dvb_service_id: None,
        }
    }
}

fn default_period_duration_secs() -> u64 {
    DEFAULT_PERIOD_DURATION_SECS
}

/// Serde-friendly [`ArchiveOverrun`]: lowercase string tokens matching the
/// operator-facing names in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "lowercase")]
pub enum ArchiveOverrunSerde {
    /// [`ArchiveOverrun::Gap`].
    #[default]
    Gap,
    /// [`ArchiveOverrun::StallIngest`].
    Stall,
    /// [`ArchiveOverrun::Terminate`].
    Terminate,
}

impl ArchiveOverrunSerde {
    /// The configuration token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            ArchiveOverrunSerde::Gap => "gap",
            ArchiveOverrunSerde::Stall => "stall",
            ArchiveOverrunSerde::Terminate => "terminate",
        }
    }
}

broadcast_common::impl_spec_display!(ArchiveOverrunSerde);

impl From<ArchiveOverrunSerde> for ArchiveOverrun {
    fn from(v: ArchiveOverrunSerde) -> Self {
        match v {
            ArchiveOverrunSerde::Gap => ArchiveOverrun::Gap,
            ArchiveOverrunSerde::Stall => ArchiveOverrun::StallIngest,
            ArchiveOverrunSerde::Terminate => ArchiveOverrun::Terminate,
        }
    }
}

impl DvrConfig {
    /// Enable DVR recording for this route.
    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the archive root directory.
    #[must_use]
    pub fn with_archive_root(mut self, archive_root: String) -> Self {
        self.archive_root = archive_root;
        self
    }

    /// Set the period duration (seconds).
    #[must_use]
    pub fn with_period_duration_secs(mut self, period_duration_secs: u64) -> Self {
        self.period_duration_secs = period_duration_secs;
        self
    }

    /// Set count-based retention.
    #[must_use]
    pub fn with_retention_periods(mut self, retention_periods: usize) -> Self {
        self.retention_periods = retention_periods;
        self
    }

    /// Set byte-based retention.
    #[must_use]
    pub fn with_retention_bytes(mut self, retention_bytes: u64) -> Self {
        self.retention_bytes = retention_bytes;
        self
    }

    /// Set the archive-overrun policy.
    #[must_use]
    pub fn with_overrun(mut self, overrun: ArchiveOverrunSerde) -> Self {
        self.overrun = overrun;
        self
    }

    /// Set the DVB service id for programme-aligned rolling.
    #[must_use]
    pub fn with_dvb_service_id(mut self, dvb_service_id: u16) -> Self {
        self.dvb_service_id = Some(dvb_service_id);
        self
    }

    /// Validate this config. A disabled config is always valid.
    ///
    /// # Errors
    ///
    /// [`MultimuxError::ConfigInvalid`] naming [`ConfigField::ArchiveRoot`]
    /// for an empty `archive_root`, [`ConfigField::Retention`] when neither
    /// retention axis is set, and [`ConfigField::PeriodDurationSecs`] when
    /// nothing can ever roll or bound the open period.
    pub fn validate(&self) -> Result<(), MultimuxError> {
        if !self.enabled {
            return Ok(());
        }
        if self.archive_root.is_empty() {
            return Err(MultimuxError::ConfigInvalid {
                field: ConfigField::ArchiveRoot,
                reason: "archive_root must be set when DVR is enabled".to_string(),
            });
        }
        self.archive_config(ArchiveContainer::Ts, Duration::from_secs(1), 0)
            .validate()
            .map_err(config_error)
    }

    /// Whether configured retention can evict leading segments from the
    /// archive. A served playlist may claim `EXT-X-PLAYLIST-TYPE:EVENT` only
    /// when it cannot (RFC 8216 §6.2.2).
    #[must_use]
    pub fn retention_active(&self) -> bool {
        self.retention().is_active()
    }

    /// The [`ArchiveConfig`] for a route with this container and segmenter
    /// target duration, numbering its segments `seq_offset` above the
    /// Trunk's.
    ///
    /// # Errors
    ///
    /// As [`Self::validate`], plus [`ConfigField::Dvr`] for a zero target
    /// duration.
    pub fn to_archive_config(
        &self,
        container: Container,
        target_duration: Duration,
        seq_offset: u64,
    ) -> Result<ArchiveConfig, MultimuxError> {
        let config = self.archive_config(archive_container(container), target_duration, seq_offset);
        config.validate().map_err(config_error)?;
        Ok(config)
    }

    fn retention(&self) -> RetentionLimits {
        let mut retention = RetentionLimits::new();
        if let Some(periods) = NonZeroUsize::new(self.retention_periods) {
            retention = retention.with_periods(periods);
        }
        if let Some(bytes) = NonZeroU64::new(self.retention_bytes) {
            retention = retention.with_bytes(bytes);
        }
        retention
    }

    fn archive_config(
        &self,
        container: ArchiveContainer,
        target_duration: Duration,
        seq_offset: u64,
    ) -> ArchiveConfig {
        let period =
            (self.period_duration_secs > 0).then(|| Duration::from_secs(self.period_duration_secs));
        let config = ArchiveConfig::new(container, target_duration, self.retention())
            .with_period_duration(period)
            .with_overrun(self.overrun.into())
            .with_seq_offset(seq_offset);
        match self.dvb_service_id {
            Some(service) => config.with_programme_service(service),
            None => config,
        }
    }
}

/// The archive container recording a route with `container` produces.
#[must_use]
pub(crate) fn archive_container(container: Container) -> ArchiveContainer {
    match container {
        Container::MpegTs => ArchiveContainer::Ts,
        _ => ArchiveContainer::Fmp4,
    }
}

/// The archive name of a route.
///
/// # Errors
///
/// [`MultimuxError::ConfigInvalid`] naming [`ConfigField::RouteName`] when
/// the route name is not a safe single directory component.
pub(crate) fn archive_name(route_name: &str) -> Result<ArchiveName, MultimuxError> {
    ArchiveName::new(route_name).map_err(config_error)
}

fn config_error(error: ConfigError) -> MultimuxError {
    let field = match error {
        ConfigError::EmptyName
        | ConfigError::TraversalName
        | ConfigError::NameTooLong { .. }
        | ConfigError::SeparatorInName => ConfigField::RouteName,
        ConfigError::NoRetention => ConfigField::Retention,
        ConfigError::UnboundedPeriod => ConfigField::PeriodDurationSecs,
        _ => ConfigField::Dvr,
    };
    MultimuxError::ConfigInvalid {
        field,
        reason: error.to_string(),
    }
}

/// The highest sequence number the archive of `route_name` holds (`0` for an
/// empty or missing archive), so a restarted route numbers its segments above
/// everything already archived and [`FsArchive::scan`] never sees two periods
/// claim one sequence number. Scanned on the blocking pool.
///
/// # Errors
///
/// [`MultimuxError::ConfigInvalid`] for an unsafe route name, and
/// [`MultimuxError::ArchiveScan`] when the blocking scan did not run to
/// completion: the caller must fail the route rather than number it from `0`
/// over an archive that may hold higher sequences.
pub(crate) async fn archive_seq_floor(
    config: &DvrConfig,
    route_name: &str,
    container: Container,
) -> Result<SegmentSeq, MultimuxError> {
    let name = archive_name(route_name)?;
    let mut archive = FsArchive::open(
        Path::new(&config.archive_root),
        &name,
        archive_container(container),
    );
    floor_from_scan(move || archive.seq_floor()).await
}

async fn floor_from_scan<F>(scan: F) -> Result<SegmentSeq, MultimuxError>
where
    F: FnOnce() -> SegmentSeq + Send + 'static,
{
    tokio::task::spawn_blocking(scan)
        .await
        .map_err(MultimuxError::ArchiveScan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> DvrConfig {
        DvrConfig::default()
            .with_enabled(true)
            .with_archive_root("/var/archive".to_string())
            .with_retention_periods(4)
    }

    fn field_of(result: Result<(), MultimuxError>) -> Option<ConfigField> {
        match result {
            Err(MultimuxError::ConfigInvalid { field, .. }) => Some(field),
            _ => None,
        }
    }

    /// A scan that cannot run must fail the floor rather than read as sequence
    /// `0`: a route restarted over an existing archive would otherwise
    /// renumber from `0` and collide with archived segments.
    #[tokio::test]
    async fn a_failed_scan_is_an_error_not_a_zero_floor() {
        let result = floor_from_scan(|| panic!("scan failed")).await;
        assert!(matches!(result, Err(MultimuxError::ArchiveScan(_))));
    }

    #[test]
    fn the_default_period_is_three_hours() {
        assert_eq!(DvrConfig::default().period_duration_secs, 10800);
        assert_eq!(default_period_duration_secs(), 10800);
        let parsed: DvrConfig = serde_json::from_str("{}").expect("parse");
        assert_eq!(parsed.period_duration_secs, 10800);
        assert!(!parsed.enabled);
    }

    #[test]
    fn validate_names_each_failing_field() {
        assert_eq!(field_of(DvrConfig::default().validate()), None);
        assert!(
            DvrConfig::default()
                .with_archive_root(String::new())
                .validate()
                .is_ok(),
            "a disabled config is valid whatever it holds"
        );
        assert_eq!(field_of(enabled().validate()), None);
        assert!(enabled().validate().is_ok());
        assert_eq!(
            field_of(enabled().with_archive_root(String::new()).validate()),
            Some(ConfigField::ArchiveRoot)
        );
        assert_eq!(
            field_of(enabled().with_retention_periods(0).validate()),
            Some(ConfigField::Retention)
        );
        assert_eq!(
            field_of(enabled().with_period_duration_secs(0).validate()),
            Some(ConfigField::PeriodDurationSecs)
        );
        assert!(
            enabled()
                .with_period_duration_secs(0)
                .with_retention_bytes(1 << 20)
                .validate()
                .is_ok()
        );
        assert!(
            enabled()
                .with_period_duration_secs(0)
                .with_dvb_service_id(7)
                .validate()
                .is_ok()
        );
        assert!(
            enabled()
                .with_retention_periods(0)
                .with_retention_bytes(1)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn retention_is_active_on_either_axis() {
        assert!(enabled().retention_active());
        assert!(
            enabled()
                .with_retention_periods(0)
                .with_retention_bytes(1)
                .retention_active()
        );
        assert!(!enabled().with_retention_periods(0).retention_active());
    }

    #[test]
    fn builders_set_their_field() {
        assert_eq!(
            DvrConfig::default().with_dvb_service_id(7).dvb_service_id,
            Some(7)
        );
        assert_eq!(
            DvrConfig::default()
                .with_overrun(ArchiveOverrunSerde::Terminate)
                .overrun,
            ArchiveOverrunSerde::Terminate
        );
        assert_eq!(
            DvrConfig::default()
                .with_period_duration_secs(5)
                .period_duration_secs,
            5
        );
        assert_eq!(
            DvrConfig::default().with_retention_bytes(9).retention_bytes,
            9
        );
    }

    #[test]
    fn overrun_tokens_and_policies() {
        let cases = [
            (ArchiveOverrunSerde::Gap, "gap", ArchiveOverrun::Gap),
            (
                ArchiveOverrunSerde::Stall,
                "stall",
                ArchiveOverrun::StallIngest,
            ),
            (
                ArchiveOverrunSerde::Terminate,
                "terminate",
                ArchiveOverrun::Terminate,
            ),
        ];
        for (serde, token, policy) in cases {
            assert_eq!(serde.name(), token);
            assert_eq!(serde.to_string(), token);
            assert_eq!(ArchiveOverrun::from(serde), policy);
            let parsed: ArchiveOverrunSerde =
                serde_json::from_str(&format!("\"{token}\"")).expect("token");
            assert_eq!(parsed, serde);
            let config = enabled()
                .with_overrun(serde)
                .to_archive_config(Container::Fmp4, Duration::from_secs(2), 0)
                .expect("valid");
            assert_eq!(config.overrun, policy);
        }
    }

    #[test]
    fn the_archive_config_carries_every_setting() {
        let config = enabled()
            .with_period_duration_secs(60)
            .with_retention_bytes(4096)
            .with_dvb_service_id(0x0601)
            .to_archive_config(Container::MpegTs, Duration::from_secs(2), 41)
            .expect("valid");
        assert_eq!(config.container, ArchiveContainer::Ts);
        assert_eq!(config.target_duration, Duration::from_secs(2));
        assert_eq!(config.period_duration, Some(Duration::from_secs(60)));
        assert_eq!(config.retention.periods, NonZeroUsize::new(4));
        assert_eq!(config.retention.bytes, NonZeroU64::new(4096));
        assert_eq!(config.seq_offset, 41);
        assert_eq!(config.programme_service, Some(0x0601));

        let untimed = enabled()
            .with_period_duration_secs(0)
            .with_retention_bytes(1)
            .to_archive_config(Container::Fmp4, Duration::from_secs(1), 0)
            .expect("valid");
        assert_eq!(untimed.container, ArchiveContainer::Fmp4);
        assert_eq!(untimed.period_duration, None);
        assert_eq!(untimed.programme_service, None);

        assert!(matches!(
            enabled().to_archive_config(Container::Fmp4, Duration::ZERO, 0),
            Err(MultimuxError::ConfigInvalid {
                field: ConfigField::Dvr,
                ..
            })
        ));
    }

    #[test]
    fn an_unsafe_route_name_is_a_route_name_error() {
        for name in ["", "..", "a/b", "a\0b"] {
            assert!(
                matches!(
                    archive_name(name),
                    Err(MultimuxError::ConfigInvalid {
                        field: ConfigField::RouteName,
                        ..
                    })
                ),
                "{name:?}"
            );
        }
        assert_eq!(archive_name("cam").expect("safe").as_str(), "cam");
    }
}
