//! A report of content a reader skipped, and the caller's policy for it.
//!
//! Best-effort reading is the workspace rule: a reader parses as well as it
//! can and continues, skips broken metadata and broken frames, and refuses
//! only when what is missing makes playback impossible. Whenever it skips
//! something it says so through this module, so the player can choose to log,
//! warn, fail or repair — the library never decides what a skip means.
//!
//! Three pieces:
//!
//! - [`Damage`] — plain data: what happened to the content ([`DamageKind`]),
//!   how bad it is ([`Severity`]), where in the input it happened
//!   ([`DamageLocation`]), which structure it was ([`Damage::what`]), the
//!   typed cause where a shared error kind applies ([`DamageCause`]), and
//!   whether the skipped bytes are still available verbatim.
//! - [`DamagePolicy`] — the caller's decision per severity and, optionally,
//!   per kind: [`Disposition::Report`] (the default for everything: continue
//!   and report) or [`Disposition::Fail`] (stop with a [`DamageError`]
//!   carrying the same report). There is no silent setting; a report is
//!   always produced.
//! - [`DamagePolicy::admit`] — the one helper a reader calls on a report: it
//!   hands the report back to publish, or the typed error to stop with.
//!
//! A reader's existing output channel gains one variant carrying a [`Damage`]
//! (`DemuxEvent::Damage`, `SessionEvent::Damage`, `Event::Damage`); it is
//! never a callback stored in the reader, so a reader stays sans-IO.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::cenc::FOURCC_LEN;
use crate::error;
use crate::timescale::Timescale;

/// A byte range in the input, as far as the reader knows it.
///
/// A zero-length range marks a loss whose bytes are not in the input at all —
/// packets dropped upstream, for example: [`start`](Self::start) is then the
/// offset at which the loss begins. A range with a length covers bytes the
/// reader held and discarded, or still holds
/// ([`Damage::recoverable_bytes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ByteRange {
    start: u64,
    len: u64,
}

impl ByteRange {
    /// A range starting at `start` and covering `len` bytes.
    #[must_use]
    pub const fn new(start: u64, len: u64) -> Self {
        Self { start, len }
    }

    /// Offset of the first byte, from the start of the input.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Number of bytes covered.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.len
    }

    /// Whether the range covers no bytes (a point of loss).
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Offset one past the last byte covered.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.start.saturating_add(self.len)
    }
}

impl core::fmt::Display for ByteRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "bytes {}..{}", self.start, self.end())
    }
}

/// A media time range in a stated time base.
///
/// Both ends are ticks of [`timescale`](Self::timescale), so a consumer can
/// compare the range with a sample's `pts` without guessing a rate. `end` is
/// exclusive; `end == start` marks a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaTimeRange {
    timescale: Timescale,
    start: i64,
    end: i64,
}

impl MediaTimeRange {
    /// A range from `start` (inclusive) to `end` (exclusive) in `timescale`.
    #[must_use]
    pub const fn new(timescale: Timescale, start: i64, end: i64) -> Self {
        Self {
            timescale,
            start,
            end,
        }
    }

    /// A single instant in `timescale`.
    #[must_use]
    pub const fn at(timescale: Timescale, tick: i64) -> Self {
        Self::new(timescale, tick, tick)
    }

    /// The time base the two ends are expressed in.
    #[must_use]
    pub const fn timescale(self) -> Timescale {
        self.timescale
    }

    /// First tick covered.
    #[must_use]
    pub const fn start(self) -> i64 {
        self.start
    }

    /// One past the last tick covered.
    #[must_use]
    pub const fn end(self) -> i64 {
        self.end
    }
}

impl core::fmt::Display for MediaTimeRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}..{} @ {} Hz", self.start, self.end, self.timescale)
    }
}

/// Where in the input a [`Damage`] happened, as far as the reader knows.
///
/// Every part is optional: a reader never fabricates a location it did not
/// observe. Build one with the setters and attach it with
/// [`Damage::with_location`], or use [`Damage`]'s own `at_*`/`for_*` builders.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DamageLocation {
    byte_range: Option<ByteRange>,
    track: Option<u32>,
    media_time: Option<MediaTimeRange>,
    ordinal: Option<u64>,
}

impl DamageLocation {
    /// An empty location.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            byte_range: None,
            track: None,
            media_time: None,
            ordinal: None,
        }
    }

    /// The byte range, when the input has byte positions.
    #[must_use]
    pub const fn byte_range(&self) -> Option<ByteRange> {
        self.byte_range
    }

    /// The track the damage belongs to, when it belongs to one.
    #[must_use]
    pub const fn track(&self) -> Option<u32> {
        self.track
    }

    /// The media time range, when the damaged content had timing.
    #[must_use]
    pub const fn media_time(&self) -> Option<MediaTimeRange> {
        self.media_time
    }

    /// The ordinal of the affected unit within its stream, when the reader
    /// counts units.
    #[must_use]
    pub const fn ordinal(&self) -> Option<u64> {
        self.ordinal
    }

    /// Record a byte range.
    #[must_use]
    pub const fn with_byte_range(mut self, range: ByteRange) -> Self {
        self.byte_range = Some(range);
        self
    }

    /// Record the owning track.
    #[must_use]
    pub const fn with_track(mut self, track: u32) -> Self {
        self.track = Some(track);
        self
    }

    /// Record a media time range.
    #[must_use]
    pub const fn with_media_time(mut self, range: MediaTimeRange) -> Self {
        self.media_time = Some(range);
        self
    }

    /// Record the ordinal of the affected unit.
    #[must_use]
    pub const fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = Some(ordinal);
        self
    }
}

/// What a reader did to the content when it found it broken.
///
/// Deliberately format-independent and small: it names the effect on the
/// content, not the box, packet or element that caused it (that is
/// [`Damage::what`]). `#[non_exhaustive]`: a new way to continue past damage
/// adds a variant.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DamageKind {
    /// Metadata was skipped: the reader left it out of its output.
    MetadataSkipped,
    /// Metadata was carried uninterpreted: the reader passed it through
    /// without understanding it.
    MetadataUninterpreted,
    /// A media unit was dropped and is not delivered.
    MediaUnitDropped,
    /// A media unit was delivered, but flagged as damaged.
    MediaUnitDamaged,
    /// The reader resynchronised: bytes were skipped to find the next valid
    /// unit.
    Resynchronised,
    /// The input was truncated.
    InputTruncated,
    /// The timeline was repaired: a timestamp was rebased or a gap closed.
    TimelineRepaired,
    /// A whole track could not be established and was dropped.
    TrackDropped,
    /// A caller-supplied limit was reached and the content above it was
    /// dropped.
    LimitReached,
}

impl DamageKind {
    /// A short, stable label for this kind — used by `Display`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            DamageKind::MetadataSkipped => "metadata skipped",
            DamageKind::MetadataUninterpreted => "metadata uninterpreted",
            DamageKind::MediaUnitDropped => "media unit dropped",
            DamageKind::MediaUnitDamaged => "media unit damaged",
            DamageKind::Resynchronised => "resynchronised",
            DamageKind::InputTruncated => "input truncated",
            DamageKind::TimelineRepaired => "timeline repaired",
            DamageKind::TrackDropped => "track dropped",
            DamageKind::LimitReached => "limit reached",
        }
    }
}

crate::impl_spec_display!(DamageKind);

/// How much a [`Damage`] costs the viewer.
///
/// No "fatal" level: a failure the reader cannot continue past is an error,
/// not a report.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Severity {
    /// Nothing a viewer would notice (an uninterpreted tag, a repaired
    /// timestamp, a skipped reserved field).
    Informational,
    /// Content is missing or altered (a dropped unit, a damaged frame, a
    /// truncated tail).
    Degraded,
}

impl Severity {
    /// A short, stable label for this severity — used by `Display`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Severity::Informational => "informational",
            Severity::Degraded => "degraded",
        }
    }
}

crate::impl_spec_display!(Severity);

/// The typed cause of a [`Damage`], as the workspace's shared error kinds.
///
/// A reader whose own error type carries one of [`crate::error`]'s kinds
/// passes it through here, so a caller can match on the cause without parsing
/// text. A failure with no shared kind (a format-specific parse error) leaves
/// [`Damage::cause`] `None` rather than inventing one.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageCause {
    /// The unit was shorter than the structure it declared.
    BufferTooShort(error::BufferTooShort),
    /// The unit declared a length the input did not hold.
    LengthMismatch(error::LengthMismatch),
    /// A fixed marker did not match.
    InvalidSync(error::InvalidSync),
    /// A protected block's CRC disagreed.
    CrcMismatch(error::CrcMismatch),
    /// A reserved field held a value the format does not allow.
    ReservedBits(error::ReservedBits),
    /// A field held a value the format does not allow.
    InvalidValue(error::InvalidValue),
    /// A field was structurally invalid.
    InvalidField(error::InvalidField),
    /// A version the reader does not support.
    UnsupportedVersion(error::UnsupportedVersion),
    /// A value did not fit the field it belonged in.
    FieldOverflow(error::FieldOverflow),
}

impl DamageCause {
    /// A short, stable label for this cause — used by `Display`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            DamageCause::BufferTooShort(_) => "buffer too short",
            DamageCause::LengthMismatch(_) => "length mismatch",
            DamageCause::InvalidSync(_) => "invalid sync",
            DamageCause::CrcMismatch(_) => "crc mismatch",
            DamageCause::ReservedBits(_) => "reserved bits",
            DamageCause::InvalidValue(_) => "invalid value",
            DamageCause::InvalidField(_) => "invalid field",
            DamageCause::UnsupportedVersion(_) => "unsupported version",
            DamageCause::FieldOverflow(_) => "field overflow",
        }
    }
}

impl core::fmt::Display for DamageCause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DamageCause::BufferTooShort(e) => e.fmt(f),
            DamageCause::LengthMismatch(e) => e.fmt(f),
            DamageCause::InvalidSync(e) => e.fmt(f),
            DamageCause::CrcMismatch(e) => e.fmt(f),
            DamageCause::ReservedBits(e) => e.fmt(f),
            DamageCause::InvalidValue(e) => e.fmt(f),
            DamageCause::InvalidField(e) => e.fmt(f),
            DamageCause::UnsupportedVersion(e) => e.fmt(f),
            DamageCause::FieldOverflow(e) => e.fmt(f),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for DamageCause {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

macro_rules! cause_from {
    ($ty:ty, $variant:ident) => {
        impl From<$ty> for DamageCause {
            fn from(value: $ty) -> Self {
                DamageCause::$variant(value)
            }
        }
    };
}

cause_from!(error::BufferTooShort, BufferTooShort);
cause_from!(error::LengthMismatch, LengthMismatch);
cause_from!(error::InvalidSync, InvalidSync);
cause_from!(error::CrcMismatch, CrcMismatch);
cause_from!(error::ReservedBits, ReservedBits);
cause_from!(error::InvalidValue, InvalidValue);
cause_from!(error::InvalidField, InvalidField);
cause_from!(error::UnsupportedVersion, UnsupportedVersion);
cause_from!(error::FieldOverflow, FieldOverflow);

/// One thing a reader skipped, with everything it knows about it.
///
/// Plain data, private fields, builder-style construction:
///
/// ```
/// use broadcast_core::damage::{Damage, DamageKind, Severity};
///
/// let report = Damage::new(DamageKind::MediaUnitDropped, Severity::Degraded, "PES packet")
///     .at_byte_range(broadcast_core::damage::ByteRange::new(1_234, 188))
///     .for_track(1);
/// assert_eq!(report.kind(), DamageKind::MediaUnitDropped);
/// assert_eq!(report.location().byte_range().map(|r| r.start()), Some(1_234));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Damage {
    kind: DamageKind,
    severity: Severity,
    location: DamageLocation,
    what: &'static str,
    fourcc: Option<[u8; FOURCC_LEN]>,
    cause: Option<DamageCause>,
    recoverable_bytes: bool,
}

impl Damage {
    /// A report of `kind` at `severity` about the structure `what`, named in
    /// the format's own terms (`"trun"`, `"PES packet"`, `"onMetaData"`).
    #[must_use]
    pub const fn new(kind: DamageKind, severity: Severity, what: &'static str) -> Self {
        Self {
            kind,
            severity,
            location: DamageLocation::new(),
            what,
            fourcc: None,
            cause: None,
            recoverable_bytes: false,
        }
    }

    /// What happened to the content.
    #[must_use]
    pub const fn kind(&self) -> DamageKind {
        self.kind
    }

    /// How much it costs the viewer.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Where in the input it happened, as far as the reader knows.
    #[must_use]
    pub const fn location(&self) -> DamageLocation {
        self.location
    }

    /// The structure that was damaged, in the format's own terms.
    #[must_use]
    pub const fn what(&self) -> &'static str {
        self.what
    }

    /// The box/atom type, for a format whose structures are four-character
    /// codes.
    #[must_use]
    pub const fn fourcc(&self) -> Option<[u8; FOURCC_LEN]> {
        self.fourcc
    }

    /// The typed cause, when the reader's failure is one of the workspace's
    /// shared error kinds.
    #[must_use]
    pub const fn cause(&self) -> Option<DamageCause> {
        self.cause
    }

    /// Whether the skipped bytes are still available verbatim to the caller.
    ///
    /// `false` means they were discarded (or never arrived). `true` means the
    /// reader kept them, so a caller that wants to repair can ask for them;
    /// the range is in [`DamageLocation::byte_range`].
    #[must_use]
    pub const fn recoverable_bytes(&self) -> bool {
        self.recoverable_bytes
    }

    /// Replace the whole location.
    #[must_use]
    pub const fn with_location(mut self, location: DamageLocation) -> Self {
        self.location = location;
        self
    }

    /// Record where in the input the damage is.
    #[must_use]
    pub const fn at_byte_range(mut self, range: ByteRange) -> Self {
        self.location = self.location.with_byte_range(range);
        self
    }

    /// Record the track the damaged content belongs to.
    #[must_use]
    pub const fn for_track(mut self, track: u32) -> Self {
        self.location = self.location.with_track(track);
        self
    }

    /// Record the media time of the damaged content.
    #[must_use]
    pub const fn at_media_time(mut self, range: MediaTimeRange) -> Self {
        self.location = self.location.with_media_time(range);
        self
    }

    /// Record the ordinal of the affected unit within its stream.
    #[must_use]
    pub const fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.location = self.location.with_ordinal(ordinal);
        self
    }

    /// Record a four-character box/atom type.
    #[must_use]
    pub const fn with_fourcc(mut self, fourcc: [u8; FOURCC_LEN]) -> Self {
        self.fourcc = Some(fourcc);
        self
    }

    /// Record the typed cause.
    #[must_use]
    pub const fn with_cause(mut self, cause: DamageCause) -> Self {
        self.cause = Some(cause);
        self
    }

    /// Record whether the skipped bytes are still available verbatim.
    #[must_use]
    pub const fn with_recoverable_bytes(mut self, recoverable: bool) -> Self {
        self.recoverable_bytes = recoverable;
        self
    }
}

impl core::fmt::Display for Damage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} {} in {}", self.severity, self.kind, self.what)?;
        if let Some(range) = self.location.byte_range() {
            write!(f, " at {range}")?;
        }
        if let Some(track) = self.location.track() {
            write!(f, " (track {track})")?;
        }
        if let Some(time) = self.location.media_time() {
            write!(f, " at media time {time}")?;
        }
        if let Some(cause) = self.cause {
            write!(f, ": {cause}")?;
        }
        Ok(())
    }
}

/// What a [`DamagePolicy`] says to do with one report.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Disposition {
    /// Continue, and publish the report.
    #[default]
    Report,
    /// Stop with a [`DamageError`] carrying the report.
    Fail,
}

impl Disposition {
    /// A short, stable label for this disposition — used by `Display`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Disposition::Report => "report",
            Disposition::Fail => "fail",
        }
    }
}

crate::impl_spec_display!(Disposition);

/// The caller's decision about what a [`Damage`] means.
///
/// One setting per [`Severity`], plus optional per-[`DamageKind`] overrides
/// (checked first). The default reports everything and fails on nothing:
/// best effort. There is no "ignore silently" setting — a reader always
/// produces the report; the policy only decides whether it also stops.
///
/// ```
/// use broadcast_core::damage::{Damage, DamageKind, DamagePolicy, Severity};
///
/// let policy = DamagePolicy::new().fail_on(Severity::Degraded);
/// let report = Damage::new(DamageKind::MediaUnitDropped, Severity::Degraded, "PES packet");
/// assert!(policy.admit(report).is_err());
/// let quiet = Damage::new(DamageKind::MetadataSkipped, Severity::Informational, "tag");
/// assert!(policy.admit(quiet).is_ok());
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DamagePolicy {
    informational: Disposition,
    degraded: Disposition,
    kinds: Vec<(DamageKind, Disposition)>,
}

impl Default for DamagePolicy {
    fn default() -> Self {
        Self {
            informational: Disposition::Report,
            degraded: Disposition::Report,
            kinds: Vec::new(),
        }
    }
}

impl DamagePolicy {
    /// Every severity reports, fails on nothing (the default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set what every report of `severity` means.
    #[must_use]
    pub fn set_severity(mut self, severity: Severity, disposition: Disposition) -> Self {
        match severity {
            Severity::Informational => self.informational = disposition,
            Severity::Degraded => self.degraded = disposition,
        }
        self
    }

    /// Fail on every report of `severity`.
    #[must_use]
    pub fn fail_on(self, severity: Severity) -> Self {
        self.set_severity(severity, Disposition::Fail)
    }

    /// Override what a report of `kind` means, whatever its severity.
    #[must_use]
    pub fn set_kind(mut self, kind: DamageKind, disposition: Disposition) -> Self {
        self.kinds.retain(|(k, _)| *k != kind);
        self.kinds.push((kind, disposition));
        self
    }

    /// The decision for one report: a per-kind override if there is one,
    /// otherwise the severity setting.
    #[must_use]
    pub fn decision(&self, damage: &Damage) -> Disposition {
        if let Some((_, disposition)) = self.kinds.iter().find(|(kind, _)| *kind == damage.kind()) {
            return *disposition;
        }
        match damage.severity() {
            Severity::Informational => self.informational,
            Severity::Degraded => self.degraded,
        }
    }

    /// Decide what one report means: `Ok(damage)` hands it back to publish and
    /// continue, `Err(error)` is the typed stop carrying the same report.
    ///
    /// # Errors
    /// [`DamageError`] when the policy's decision for this report is
    /// [`Disposition::Fail`].
    pub fn admit(&self, damage: Damage) -> Result<Damage, DamageError> {
        match self.decision(&damage) {
            Disposition::Report => Ok(damage),
            Disposition::Fail => Err(DamageError::new(damage)),
        }
    }
}

/// The typed stop a [`DamagePolicy`] raises for a report it fails on.
///
/// Carries the same [`Damage`] the reader would otherwise have published, so
/// a caller that handles the error sees everything the report says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageError {
    /// Boxed so a reader's `Result<_, DamageError>` stays small.
    damage: Box<Damage>,
}

impl DamageError {
    /// Wrap a report as a stop.
    #[must_use]
    pub fn new(damage: Damage) -> Self {
        Self {
            damage: Box::new(damage),
        }
    }

    /// The report this stop carries.
    #[must_use]
    pub fn damage(&self) -> &Damage {
        &self.damage
    }

    /// Take the report back out.
    #[must_use]
    pub fn into_damage(self) -> Damage {
        *self.damage
    }
}

impl core::fmt::Display for DamageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "damage policy refused: {}", &*self.damage)
    }
}

impl core::error::Error for DamageError {}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::string::ToString;

    fn dropped() -> Damage {
        Damage::new(
            DamageKind::MediaUnitDropped,
            Severity::Degraded,
            "PES packet",
        )
        .at_byte_range(ByteRange::new(1_880, 188))
        .for_track(3)
        .at_media_time(MediaTimeRange::new(crate::timescale!(90_000), 900, 1_800))
        .with_ordinal(7)
    }

    #[test]
    fn builders_compose_and_accessors_read_back() {
        let damage = dropped()
            .with_fourcc(*b"trun")
            .with_cause(DamageCause::InvalidSync(error::InvalidSync {
                what: "PES start code",
                expected: 1,
                found: 9,
            }))
            .with_recoverable_bytes(true);
        assert_eq!(damage.kind(), DamageKind::MediaUnitDropped);
        assert_eq!(damage.severity(), Severity::Degraded);
        assert_eq!(damage.what(), "PES packet");
        assert_eq!(damage.fourcc(), Some(*b"trun"));
        assert!(damage.recoverable_bytes());
        assert!(damage.cause().is_some());
        let location = damage.location();
        assert_eq!(location.byte_range(), Some(ByteRange::new(1_880, 188)));
        assert_eq!(location.track(), Some(3));
        assert_eq!(location.ordinal(), Some(7));
        let time = location.media_time().expect("media time");
        assert_eq!(time.timescale().get(), 90_000);
        assert_eq!((time.start(), time.end()), (900, 1_800));
    }

    #[test]
    fn a_location_defaults_to_empty_never_fabricated() {
        let damage = Damage::new(DamageKind::MetadataSkipped, Severity::Informational, "tag");
        let location = damage.location();
        assert_eq!(location.byte_range(), None);
        assert_eq!(location.track(), None);
        assert_eq!(location.media_time(), None);
        assert_eq!(location.ordinal(), None);
        assert_eq!(damage.cause(), None);
        assert_eq!(damage.fourcc(), None);
        assert!(!damage.recoverable_bytes());
    }

    #[test]
    fn a_zero_length_range_marks_absent_bytes() {
        let range = ByteRange::new(752, 0);
        assert!(range.is_empty());
        assert_eq!(range.start(), 752);
        assert_eq!(range.end(), 752);
        assert_eq!(ByteRange::new(2, 3).end(), 5);
    }

    #[test]
    fn the_default_policy_reports_everything() {
        let policy = DamagePolicy::new();
        for severity in [Severity::Informational, Severity::Degraded] {
            let damage = Damage::new(DamageKind::MediaUnitDropped, severity, "unit");
            assert_eq!(policy.decision(&damage), Disposition::Report);
            assert!(policy.admit(damage).is_ok());
        }
    }

    #[test]
    fn failing_on_a_severity_stops_with_the_same_report() {
        let policy = DamagePolicy::new().fail_on(Severity::Degraded);
        let damage = dropped();
        let error = policy.admit(damage.clone()).expect_err("degraded fails");
        assert_eq!(error.damage(), &damage);
        assert_eq!(error.into_damage(), damage);
    }

    #[test]
    fn a_kind_override_beats_the_severity_setting() {
        let policy = DamagePolicy::new()
            .fail_on(Severity::Degraded)
            .set_kind(DamageKind::MediaUnitDropped, Disposition::Report);
        let damage = dropped();
        assert_eq!(policy.decision(&damage), Disposition::Report);
        assert!(policy.admit(damage).is_ok());

        let reverse = DamagePolicy::new().set_kind(DamageKind::MetadataSkipped, Disposition::Fail);
        let quiet = Damage::new(DamageKind::MetadataSkipped, Severity::Informational, "tag");
        assert!(reverse.admit(quiet).is_err());
    }

    #[test]
    fn every_shared_error_kind_converts_to_a_cause() {
        let cases = [
            DamageCause::from(error::BufferTooShort {
                need: 4,
                have: 1,
                what: "unit",
            }),
            DamageCause::from(error::LengthMismatch {
                what: "unit",
                declared: 9,
                actual: 7,
            }),
            DamageCause::from(error::InvalidSync {
                what: "sync",
                expected: 1,
                found: 0,
            }),
            DamageCause::from(error::CrcMismatch {
                what: "section",
                computed: 1,
                expected: 2,
            }),
            DamageCause::from(error::ReservedBits {
                field: "r",
                expected: 1,
                found: 0,
            }),
            DamageCause::from(error::InvalidValue {
                field: "v",
                value: 9,
                reason: "out of range",
            }),
            DamageCause::from(error::InvalidField {
                field: "f",
                reason: "missing",
            }),
            DamageCause::from(error::UnsupportedVersion {
                what: "RTP",
                found: 9,
            }),
            DamageCause::from(error::FieldOverflow {
                field: "l",
                value: 9,
                max: 1,
            }),
        ];
        for cause in cases {
            assert!(!cause.to_string().is_empty());
            assert!(!cause.name().is_empty());
        }
    }

    #[test]
    fn display_names_the_content_the_place_and_the_cause() {
        let damage = dropped().with_cause(DamageCause::InvalidSync(error::InvalidSync {
            what: "PES start code",
            expected: 1,
            found: 9,
        }));
        let text = damage.to_string();
        assert!(text.contains("degraded media unit dropped in PES packet"));
        assert!(text.contains("bytes 1880..2068"));
        assert!(text.contains("track 3"));
        assert!(text.contains("media time 900..1800 @ 90000 Hz"));
        assert!(text.contains("invalid PES start code"));
        assert_eq!(
            DamageError::new(damage).to_string(),
            alloc::format!("damage policy refused: {text}")
        );
    }

    #[test]
    fn kind_severity_and_disposition_labels() {
        assert_eq!(DamageKind::MediaUnitDropped.name(), "media unit dropped");
        assert_eq!(DamageKind::TrackDropped.to_string(), "track dropped");
        assert_eq!(Severity::Degraded.name(), "degraded");
        assert_eq!(Severity::Informational.to_string(), "informational");
        assert_eq!(Disposition::Report.name(), "report");
        assert_eq!(Disposition::Fail.to_string(), "fail");
        assert_eq!(Disposition::default(), Disposition::Report);
    }

    #[test]
    fn damage_and_policy_are_cloneable_plain_data() {
        let policy = DamagePolicy::new().fail_on(Severity::Degraded);
        assert_eq!(policy.clone(), policy);
        let damage = dropped();
        assert_eq!(damage.clone(), damage);
        assert_ne!(
            dropped(),
            Damage::new(DamageKind::TrackDropped, Severity::Degraded, "track")
        );
    }
}
