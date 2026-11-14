//! Report model: [`Severity`], [`Finding`], and [`Report`].

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use mpeg_ts::pid::Pid;
use transmux::TrackId;

/// Severity of a diagnostic finding.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Severity {
    /// Informational — not necessarily a problem.
    Info,
    /// Something likely wrong but not necessarily stream-breaking.
    Warning,
    /// A definite error (e.g. lost sync byte, invalid CRC).
    Error,
}

impl Severity {
    /// Human-readable label for this severity level.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

broadcast_common::impl_spec_display!(Severity);

/// The stream a [`Location`] names: a transport-stream PID, a container-level
/// media track, or nothing at all.
///
/// A [`Pid`] (13 bits) and a [`TrackId`] (ISO/IEC 14496-12:2026 `track_ID`,
/// 32 bits) are different quantities, so neither widens into the other.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamLocator {
    /// A transport-stream PID.
    Ts(Pid),
    /// A container-level media track (ISO/IEC 14496-12:2026 `track_ID`).
    Track(TrackId),
    /// No stream applies (a playlist line, an MPD element, a container-level
    /// check with no track).
    Unknown,
}

impl fmt::Display for StreamLocator {
    /// The bare number a report prints, `0` for [`StreamLocator::Unknown`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ts(pid) => write!(f, "{}", pid.value()),
            Self::Track(id) => write!(f, "{}", id.value()),
            Self::Unknown => f.write_str("0"),
        }
    }
}

impl fmt::UpperHex for StreamLocator {
    /// The `pid=0x{:04X}` field a report prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ts(pid) => fmt::UpperHex::fmt(pid, f),
            Self::Track(id) => fmt::UpperHex::fmt(&id.value(), f),
            Self::Unknown => fmt::UpperHex::fmt(&0u16, f),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for StreamLocator {
    /// The bare number a report prints, so every JSON document that carried
    /// the old `pid` integer is byte-identical.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let raw = match self {
            Self::Ts(pid) => u32::from(pid.value()),
            Self::Track(id) => id.value(),
            Self::Unknown => 0,
        };
        s.serialize_u32(raw)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for StreamLocator {
    /// A number in the PID range is a [`StreamLocator::Ts`], anything above
    /// it a [`StreamLocator::Track`], and the old unknown `0`…
    ///
    /// …is the one number that cannot round-trip both ways: PID 0 is the
    /// Program Association Table on real transport streams, so a bare `0`
    /// reads back as [`StreamLocator::Unknown`], not a stream on PID 0.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = u32::deserialize(d)?;
        if raw == 0 {
            return Ok(Self::Unknown);
        }
        u16::try_from(raw)
            .ok()
            .and_then(|raw_pid| Pid::try_new(raw_pid).ok())
            .map_or_else(
                || Ok(Self::Track(TrackId::new(raw))),
                |pid| Ok(Self::Ts(pid)),
            )
    }
}

/// Packet / byte-stream location for a diagnostic finding.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Location {
    /// 0-based TS packet index within the stream.
    pub packet: usize,
    /// The stream the issue was detected on.
    #[cfg_attr(feature = "serde", serde(rename = "pid"))]
    pub stream: StreamLocator,
}

impl Location {
    /// A location with no stream: a playlist line, an MPD element, or a
    /// container-level check with no track.
    #[must_use]
    pub fn new(packet: usize) -> Self {
        Self {
            packet,
            stream: StreamLocator::Unknown,
        }
    }

    /// A location on a transport-stream PID.
    #[must_use]
    pub fn at_pid(packet: usize, pid: Pid) -> Self {
        Self {
            packet,
            stream: StreamLocator::Ts(pid),
        }
    }

    /// A location on a container-level media track (ISO/IEC 14496-12:2026 `track_ID`).
    #[must_use]
    pub fn at_track(packet: usize, track_id: TrackId) -> Self {
        Self {
            packet,
            stream: StreamLocator::Track(track_id),
        }
    }
}

/// A single diagnostic finding produced by a [`Diagnostic`](crate::Diagnostic) check.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Finding {
    /// Severity level.
    pub severity: Severity,
    /// Stream location.
    pub location: Location,
    /// Machine-readable rule identifier (e.g. `"sync-byte"`).
    pub rule_id: String,
    /// Human-readable explanation.
    pub message: String,
}

impl Finding {
    /// Create a new finding.
    pub fn new(
        severity: Severity,
        location: Location,
        rule_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            location,
            rule_id: rule_id.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] [{}] packet={} pid=0x{:04X} {}",
            self.severity, self.rule_id, self.location.packet, self.location.stream, self.message
        )
    }
}

/// A collection of [`Finding`]s produced by one or more diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Report {
    findings: Vec<Finding>,
}

impl Report {
    /// Create a new empty report.
    pub const fn new() -> Self {
        Self {
            findings: Vec::new(),
        }
    }

    /// Append a finding.
    pub fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    /// Iterate over findings.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// Number of findings.
    pub fn len(&self) -> usize {
        self.findings.len()
    }

    /// True if no findings.
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.findings.is_empty() {
            writeln!(f, "No issues found.")?;
            return Ok(());
        }
        let errs = self
            .findings
            .iter()
            .filter(|x| x.severity == Severity::Error)
            .count();
        let warns = self
            .findings
            .iter()
            .filter(|x| x.severity == Severity::Warning)
            .count();
        let infos = self
            .findings
            .iter()
            .filter(|x| x.severity == Severity::Info)
            .count();
        writeln!(
            f,
            "Findings: {errs} error(s), {warns} warning(s), {infos} info(s)"
        )?;
        for (i, finding) in self.findings.iter().enumerate() {
            writeln!(f, "{:>4}. {}", i + 1, finding)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_pid_keeps_a_typed_pid_and_at_track_keeps_32_bits() {
        // Every valid PID is carried as a typed PID, never a widened integer.
        for raw in [0x0000u16, 0x0100, 0x1FFE, 0x1FFF] {
            let pid = Pid::try_new(raw).unwrap();
            assert_eq!(Location::at_pid(7, pid).stream, StreamLocator::Ts(pid));
            assert_eq!(Location::at_pid(7, pid).packet, 7);
        }
        // A track id above 16 bits is not aliased.
        let track = TrackId::new(0x0001_0064);
        assert_eq!(
            Location::at_track(0, track).stream,
            StreamLocator::Track(track)
        );
        // An out-of-range PID cannot be turned into a typed location at all.
        assert!(matches!(
            Pid::try_new(0x2064),
            Err(mpeg_ts::pid::PidOutOfRange { .. })
        ));
    }

    #[test]
    fn a_track_id_and_a_pid_with_the_same_number_are_distinct_locations() {
        let pid = Pid::try_new(0x0100).unwrap();
        let track = TrackId::new(0x0100);
        assert_ne!(
            Location::at_pid(0, pid).stream,
            Location::at_track(0, track).stream
        );
        assert_eq!(Location::new(3).stream, StreamLocator::Unknown);
        // The report still renders the bare number, `0` when unknown.
        use alloc::string::ToString;
        assert_eq!(StreamLocator::Ts(pid).to_string(), "256");
        assert_eq!(StreamLocator::Track(track).to_string(), "256");
        assert_eq!(StreamLocator::Unknown.to_string(), "0");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn location_json_still_carries_the_bare_pid_number() {
        // A TS PID serializes as the integer the old `pid: u32` field held.
        let finding = Finding::new(
            Severity::Error,
            Location::at_pid(42, Pid::try_new(0x0100).unwrap()),
            "sync-byte",
            "bad sync",
        );
        let json = serde_json::to_value(&finding).unwrap();
        assert_eq!(json["location"]["pid"], serde_json::json!(0x0100));
        let back: Finding = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.location.stream,
            StreamLocator::Ts(Pid::try_new(0x100).unwrap())
        );

        // A track id above the PID range is not aliased onto a PID.
        let track = TrackId::new(0x0001_0064);
        let json = serde_json::to_value(StreamLocator::Track(track)).unwrap();
        assert_eq!(json, serde_json::json!(0x0001_0064u32));
        let back: StreamLocator = serde_json::from_value(json).unwrap();
        assert_eq!(back, StreamLocator::Track(track));
    }
}
