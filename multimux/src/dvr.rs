//! Per-route DVR configuration. The archive itself (its on-disk format, the
//! writer's roll and retention decisions, and catch-up read planning) is
//! [`dvr_runtime`]'s; this module only turns an operator's [`DvrConfig`] into
//! a [`dvr_runtime::ArchiveConfig`] and names the archive of a route.
//!
//! # On-disk layout
//!
//! Every program of a route records into its own archive, and each archive
//! holds exactly one service, chosen by service, never by the program's
//! position in the stream. The default archive `<archive_root>/<route_name>/`
//! holds:
//!
//! - on a non-TS source (no `program_number`), its single program;
//! - on a TS input with `dvb_service_id` set, only the program whose
//!   `program_number` is that service id, wherever it sits in the PAT;
//! - on a TS input without `dvb_service_id`, its program only while the
//!   input carries exactly one program. Once the route has announced more
//!   than one (or found per-program archives on disk at start), every
//!   numbered program records into its own archive, the one that was in
//!   the default archive included (its earlier recording stays there),
//!   from its next archive poll. Set `dvb_service_id` on an MPTS input to
//!   choose the service the default catch-up URL (no `program` parameter)
//!   serves.
//!
//! Every other program records into the sibling
//! `<archive_root>/<route_name>@<program_number>/`, its MPEG-2
//! `program_number` (the DVB `service_id`) in canonical decimal
//! (`news@1538`). A non-default program with no `program_number` is not
//! recorded. A route name never contains `@`
//! (`crate::config::Route::validate_standalone`), so a program archive can
//! never be another route's archive, and a route with DVR enabled is at most
//! [`MAX_DVR_ROUTE_NAME_LEN`] bytes so every program's archive name is a
//! valid [`ArchiveName`]. Each program archive has its own lock, its own
//! sequence floor and its own retention: the route's `retention_periods` and
//! `retention_bytes` apply to each program archive separately, so a route's
//! disk use is bounded by its number of programs times `retention_bytes`.
//!
//! An archive directory holds one media file (`pN.m4s` or `pN.ts`)
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
//! tracked service's EIT present event changes (`dvb_service_id`; see
//! [`DvrConfig::dvb_service_id`] for which service each program tracks), when the
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

use std::collections::BTreeMap;
use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;

use dvr_runtime::fs::FsArchive;
use dvr_runtime::{
    ArchiveConfig, ArchiveContainer, ArchiveName, ConfigError, MAX_ARCHIVE_NAME_LEN,
    RetentionLimits,
};
use media_plane::trunk::ArchiveOverrun;
use serde::Deserialize;
use transmux::Container;

use crate::error::{ConfigField, MultimuxError};

/// Separates the route name from the `program_number` in the archive name of
/// a non-default program (`news@1538`). Never valid in a route name.
pub(crate) const PROGRAM_ARCHIVE_SEPARATOR: char = '@';

/// Decimal digits of the largest `program_number`.
const PROGRAM_NUMBER_MAX_DIGITS: usize = (u16::MAX.ilog10() + 1) as usize;

/// The longest name of a route with DVR enabled: every program archive name
/// (`<route>@<program_number>`) must still be a valid [`ArchiveName`].
pub const MAX_DVR_ROUTE_NAME_LEN: usize =
    MAX_ARCHIVE_NAME_LEN - PROGRAM_ARCHIVE_SEPARATOR.len_utf8() - PROGRAM_NUMBER_MAX_DIGITS;

/// Which archive of its route a program records into (see
/// [the module docs](self#on-disk-layout)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ProgramArchive {
    /// The default program's, `<route>/`.
    Default,
    /// The archive of the program with this `program_number`,
    /// `<route>@<program_number>/`.
    Program(u16),
}

impl ProgramArchive {
    /// The archive of the program the route registers as `program`, whose
    /// tracks carry `program_number`, chosen by service, never by the
    /// program's position in the stream (see
    /// [the module docs](self#on-disk-layout)):
    ///
    /// - no `program_number` (a non-TS source): the default archive for the
    ///   route's single program, none (not recorded) for any other;
    /// - `dvb_service_id` set: the default archive only for the program whose
    ///   `program_number` it is, `<route>@<program_number>` for every other;
    /// - otherwise the default archive while the input carries one program
    ///   (`multi_program` false), `<route>@<program_number>` once it has
    ///   announced more than one.
    pub(crate) fn of(
        program: media_plane::ProgramId,
        program_number: Option<u16>,
        dvb_service_id: Option<u16>,
        multi_program: bool,
    ) -> Option<ProgramArchive> {
        let Some(program_number) = program_number else {
            return (program == crate::route::SPTS_PROGRAM_ID).then_some(ProgramArchive::Default);
        };
        let default = match dvb_service_id {
            Some(service) => service == program_number,
            None => !multi_program,
        };
        Some(if default {
            ProgramArchive::Default
        } else {
            ProgramArchive::Program(program_number)
        })
    }
}

/// The archived sequence floor of each program archive of a route; a program
/// without an archive is absent.
pub(crate) type ArchiveFloors = BTreeMap<ProgramArchive, u32>;

/// `a` raised, per program, to `b`.
pub(crate) fn merge_floors(mut a: ArchiveFloors, b: &ArchiveFloors) -> ArchiveFloors {
    for (program, floor) in b {
        let entry = a.entry(*program).or_default();
        *entry = (*entry).max(*floor);
    }
    a
}

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
    ///
    /// On a multi-program (MPTS) input this also selects which service the
    /// default archive (`<route>/`) records: the program whose
    /// `program_number` is this `service_id` (ETSI EN 300 468 §5.2.3: a
    /// service's `service_id` is its MPEG-2 `program_number`), and every
    /// other program, recording into `<route>@<program_number>/`, tracks its
    /// own service. See [the module docs](self#on-disk-layout).
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

    /// This config as it applies to `archive`: the tracked service of a
    /// non-default program is its own (see [`Self::dvb_service_id`]).
    #[must_use]
    pub(crate) fn for_program(&self, archive: ProgramArchive) -> DvrConfig {
        let mut config = self.clone();
        if let ProgramArchive::Program(program_number) = archive {
            config.dvb_service_id = self.dvb_service_id.map(|_| program_number);
        }
        config
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

/// The name of `archive` of route `route_name` (see
/// [the module docs](self#on-disk-layout)).
///
/// # Errors
///
/// As [`archive_name`].
pub(crate) fn program_archive_name(
    route_name: &str,
    archive: ProgramArchive,
) -> Result<ArchiveName, MultimuxError> {
    match archive {
        ProgramArchive::Default => archive_name(route_name),
        ProgramArchive::Program(program_number) => archive_name(&format!(
            "{route_name}{PROGRAM_ARCHIVE_SEPARATOR}{program_number}"
        )),
    }
}

/// The `program_number` whose archive of route `route_name` the directory
/// `dir_name` names. The route prefix matches ignoring ASCII case, so a
/// route renamed only in case still finds its archives on a case-insensitive
/// filesystem; the caller opens the archive under the route's own spelling.
fn program_of_archive(route_name: &str, dir_name: &str) -> Option<u16> {
    let prefix = dir_name.get(..route_name.len())?;
    if !prefix.eq_ignore_ascii_case(route_name) {
        return None;
    }
    let digits = dir_name
        .get(route_name.len()..)?
        .strip_prefix(PROGRAM_ARCHIVE_SEPARATOR)?;
    let program_number: u16 = digits.parse().ok()?;
    (program_number != 0 && program_number.to_string() == digits).then_some(program_number)
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

/// The highest sequence number each program archive of `route_name` holds
/// (the default program's always present, `0` for an empty or missing
/// archive), so a restarted route numbers each program's segments above
/// everything already in that program's archive and [`FsArchive::scan`] never
/// sees two periods claim one sequence number. Each archive is opened under
/// the route's own spelling of its name. Scanned on the blocking pool.
///
/// # Errors
///
/// [`MultimuxError::ConfigInvalid`] for an unsafe route name,
/// [`MultimuxError::ArchiveList`] when the archive root exists but cannot be
/// listed, [`MultimuxError::ArchiveRead`] when a program archive or one of
/// its period playlists exists but cannot be read, and
/// [`MultimuxError::ArchiveScan`] when the blocking scan did not
/// run to completion: the caller must fail the route rather than number it
/// from `0` over an archive that may hold higher sequences.
pub(crate) async fn archive_seq_floors(
    config: &DvrConfig,
    route_name: &str,
    container: Container,
) -> Result<ArchiveFloors, MultimuxError> {
    archive_name(route_name)?;
    let root = std::path::PathBuf::from(&config.archive_root);
    let route_name = route_name.to_owned();
    let container = archive_container(container);
    floor_from_scan(move || {
        let mut archives = vec![ProgramArchive::Default];
        archives.extend(list_program_archives(&root, &route_name)?);
        let mut floors = ArchiveFloors::new();
        for archive in archives {
            let name = program_archive_name(&route_name, archive)?;
            let floor = FsArchive::open(&root, &name, container)
                .seq_floor()
                .map_err(|source| MultimuxError::ArchiveRead {
                    archive: name.as_str().to_owned(),
                    source,
                })?
                .map_or(0, |seq| seq.value());
            floors.insert(archive, floor);
        }
        Ok(floors)
    })
    .await?
}

/// The non-default program archives of `route_name` under `root`; none when
/// `root` does not exist yet.
fn list_program_archives(
    root: &std::path::Path,
    route_name: &str,
) -> Result<Vec<ProgramArchive>, MultimuxError> {
    let listing_error = |source| MultimuxError::ArchiveList {
        root: root.to_string_lossy().into_owned(),
        source,
    };
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(listing_error(e)),
    };
    let mut archives = Vec::new();
    for entry in entries {
        let entry = entry.map_err(listing_error)?;
        if let Some(program_number) = entry
            .file_name()
            .to_str()
            .and_then(|name| program_of_archive(route_name, name))
        {
            archives.push(ProgramArchive::Program(program_number));
        }
    }
    Ok(archives)
}

async fn floor_from_scan<F, T>(scan: F) -> Result<T, MultimuxError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(scan)
        .await
        .map_err(MultimuxError::ArchiveScan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::ids::SegmentSeq;

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
        let result = floor_from_scan(|| -> SegmentSeq { panic!("scan failed") }).await;
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
    fn program_archive_names_round_trip_and_reject_lookalikes() {
        assert_eq!(
            program_archive_name("cam", ProgramArchive::Default)
                .expect("default")
                .as_str(),
            "cam"
        );
        assert_eq!(
            program_archive_name("cam", ProgramArchive::Program(1538))
                .expect("program")
                .as_str(),
            "cam@1538"
        );
        assert_eq!(program_of_archive("cam", "cam@1538"), Some(1538));
        assert_eq!(program_of_archive("cam", "CAM@1538"), Some(1538));
        for other in [
            "cam",
            "cam@",
            "cam@0",
            "cam@01538",
            "cam@x",
            "cama@1",
            "cam@1@2",
            "cam@65536",
        ] {
            assert_eq!(program_of_archive("cam", other), None, "{other}");
        }
        assert_eq!(MAX_DVR_ROUTE_NAME_LEN, 249);
    }

    #[test]
    fn the_default_archive_is_chosen_by_service_never_by_position() {
        use crate::route::SPTS_PROGRAM_ID;
        use ProgramArchive::{Default, Program};
        use media_plane::ProgramId;
        let cases = [
            (SPTS_PROGRAM_ID, None, None, false, Some(Default)),
            (ProgramId(1), None, None, true, None),
            (
                SPTS_PROGRAM_ID,
                Some(0x0601),
                Some(0x0602),
                true,
                Some(Program(0x0601)),
            ),
            (
                ProgramId(1),
                Some(0x0602),
                Some(0x0602),
                true,
                Some(Default),
            ),
            (
                ProgramId(2),
                Some(0x0602),
                Some(0x0602),
                false,
                Some(Default),
            ),
            (SPTS_PROGRAM_ID, Some(0x0601), None, false, Some(Default)),
            (
                SPTS_PROGRAM_ID,
                Some(0x0601),
                None,
                true,
                Some(Program(0x0601)),
            ),
            (
                ProgramId(1),
                Some(0x0602),
                None,
                true,
                Some(Program(0x0602)),
            ),
        ];
        for (program, program_number, service, multi, expected) in cases {
            assert_eq!(
                ProgramArchive::of(program, program_number, service, multi),
                expected,
                "{program:?} {program_number:?} {service:?} {multi}"
            );
        }
    }

    #[test]
    fn a_non_default_program_tracks_its_own_service() {
        let config = enabled().with_dvb_service_id(0x0601);
        assert_eq!(
            config.for_program(ProgramArchive::Default).dvb_service_id,
            Some(0x0601)
        );
        assert_eq!(
            config
                .for_program(ProgramArchive::Program(0x0602))
                .dvb_service_id,
            Some(0x0602)
        );
        assert_eq!(
            enabled()
                .for_program(ProgramArchive::Program(0x0602))
                .dvb_service_id,
            None
        );
    }

    /// An archive root that exists but cannot be listed fails the floor scan
    /// instead of reading as no program archives (floor `0`).
    #[tokio::test]
    async fn an_unlistable_archive_root_is_an_error_not_zero_floors() {
        let root =
            std::env::temp_dir().join(format!("multimux-dvr-unlistable-{}", std::process::id()));
        let _ = std::fs::remove_file(&root);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::write(&root, b"not a directory").expect("file");
        let config = enabled().with_archive_root(root.to_string_lossy().into_owned());
        let result = archive_seq_floors(&config, "cam", Container::Fmp4).await;
        assert!(
            matches!(result, Err(MultimuxError::ArchiveList { .. })),
            "{result:?}"
        );
        let _ = std::fs::remove_file(&root);
    }

    /// A program archive that exists but cannot be listed fails the floor
    /// scan instead of reading as an empty archive (floor `0`).
    #[tokio::test]
    async fn an_unreadable_program_archive_is_an_error_not_a_zero_floor() {
        let root = std::env::temp_dir().join(format!(
            "multimux-dvr-unreadable-archive-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(root.join("cam"), b"not a directory").expect("file");
        let config = enabled().with_archive_root(root.to_string_lossy().into_owned());
        let result = archive_seq_floors(&config, "cam", Container::Fmp4).await;
        assert!(
            matches!(result, Err(MultimuxError::ArchiveRead { ref archive, .. }) if archive == "cam"),
            "{result:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
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
