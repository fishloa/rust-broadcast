//! Report model: [`Severity`], [`Finding`], and [`Report`].

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

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

/// Packet / byte-stream location for a diagnostic finding.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Location {
    /// 0-based TS packet index within the stream.
    pub packet: usize,
    /// PID on which the issue was detected (0 if unknown / N/A).
    pub pid: u16,
}

impl Location {
    /// Create a new location.
    pub fn new(packet: usize, pid: u16) -> Self {
        Self { packet, pid }
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
            self.severity, self.rule_id, self.location.packet, self.location.pid, self.message
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
            "Findings: {} error(s), {} warning(s), {} info(s)",
            errs, warns, infos
        )?;
        for (i, finding) in self.findings.iter().enumerate() {
            writeln!(f, "{:>4}. {}", i + 1, finding)?;
        }
        Ok(())
    }
}
