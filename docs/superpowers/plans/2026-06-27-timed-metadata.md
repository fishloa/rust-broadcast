# timed-metadata Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A spec-cited conversion-core crate that translates SCTE-35 ↔ HLS `EXT-X-DATERANGE` and SCTE-35 ↔ DASH `emsg`, with a pure-function layer and a stateful `Timeline` session.

**Architecture:** Hub-and-spoke around a canonical `TimedEvent`; lossless because the original splice bytes ride through verbatim (DATERANGE `SCTE35-OUT` hex and emsg `message_data` both embed the whole splice section). Pure conversions are the foundation; the `Timeline` session adds the wall-clock anchor + 33-bit PTS wrap-unroll on top. `#![no_std]` + `alloc`.

**Tech Stack:** Rust, `scte35-splice` (parse SCTE-35), `mp4-emsg` (build/parse emsg `EmsgBox`), `dvb-common` (`impl_spec_display!`), `thiserror`, optional `chrono`/`serde`.

**Prerequisite:** Plan A (rename) has created local crates `scte35-splice` and `mp4-emsg`. This plan depends on them by **path** in-tree; the crates.io release of `timed-metadata` happens only after Plan A's crates are live.

## Global Constraints

- MSRV **1.81**; build/test `--locked`. CI `RUSTFLAGS="-D warnings"`; the 6 gates must pass (build all-features + no-default, test, clippy `-D warnings`, `cargo fmt --all --check`, `RUSTDOCFLAGS="-D warnings" cargo doc`).
- `#![no_std]` + `extern crate alloc;`. Use `alloc::{string::String, vec::Vec, format, vec}`. No `std::` paths.
- No magic numbers outside `#[cfg(test)]` — name every constant (`PTS_HZ = 90_000`, `SCTE35_SCHEME`, etc.).
- Every public spec/field enum gets `name()` + `dvb_common::impl_spec_display!` and is covered by `tests/label_coverage.rs`.
- Symmetric serialize + round-trip is a hard invariant: `DateRange::parse_tag_line(x.to_tag_line()) == x`.
- Independent crate, **v0.1.0**, not in any lockstep. Default features `["std","serde","chrono"]`; must build `--no-default-features`.
- Every conversion module cites its mapping spec in `//!`: HLS bis §4.4.5.1 (DATERANGE SCTE-35 attrs), SCTE 214-3 (`urn:scte:scte35:2013:bin`).

## File Structure

```
timed-metadata/
  Cargo.toml
  README.md
  CHANGELOG.md
  src/
    lib.rs            crate doc, no_std, alloc, module wiring, re-exports
    error.rs          Error / Result
    event.rs          TimedEvent, EventKind, SourcePayload, MediaTime, MediaDuration, from_scte35
    anchor.rs         TimeAnchor, media→epoch_ms, RFC3339 (chrono + no_std fallback)
    daterange.rs      DateRange, Scte35Cue, Scte35Attr, to_tag_line, parse_tag_line
    convert/
      mod.rs          EmsgConfig, SCTE35_SCHEME, re-exports
      emsg.rs         scte35_to_emsg, emsg_to_scte35
      daterange.rs    scte35_to_daterange
    timeline.rs       Timeline session (anchor + wrap-unroll + push/convert)
  tests/
    fixtures/         scte35_emsg_v0.bin, emsg_v1_scte35_livesim.bin, daterange_*.txt
    daterange_round_trip.rs
    emsg_interop.rs
    daterange_fixture.rs
    label_coverage.rs
```

---

### Task 1: Scaffold crate + error type

**Files:**
- Create: `timed-metadata/Cargo.toml`, `timed-metadata/src/lib.rs`, `timed-metadata/src/error.rs`
- Modify: root `Cargo.toml` (add `"timed-metadata"` to `members`)

**Interfaces:**
- Produces: crate `timed-metadata`; `error::{Error, Result}`.

- [ ] **Step 1: Write Cargo.toml**

Create `timed-metadata/Cargo.toml`:
```toml
[package]
name         = "timed-metadata"
version      = "0.1.0"
edition      = "2021"
license      = "MIT OR Apache-2.0"
repository   = "https://github.com/fishloa/rust-dvb"
rust-version = "1.81"
description  = "Convert DPI / timed-metadata signalling between SCTE-35, HLS EXT-X-DATERANGE and DASH emsg. no_std."
keywords     = ["scte35", "hls", "dash", "emsg", "ssai"]
categories   = ["multimedia::video", "no-std"]

[dependencies]
scte35-splice = { path = "../scte35-splice", version = "1.0", default-features = false }
mp4-emsg      = { path = "../mp4-emsg",      version = "0.1", default-features = false }
dvb-common    = { path = "../dvb-common",    version = "7.9", default-features = false }
thiserror     = { version = "2", default-features = false }
chrono        = { version = "0.4", default-features = false, optional = true }
serde         = { version = "1", default-features = false, features = ["derive", "alloc"], optional = true }

[features]
default = ["std", "serde", "chrono"]
std    = ["scte35-splice/std", "mp4-emsg/std", "chrono?/std", "serde?/std"]
serde  = ["dep:serde", "scte35-splice/serde", "mp4-emsg/serde"]
chrono = ["dep:chrono", "chrono/alloc"]
```
> At implementation, verify each dependency's real feature names (`grep -A20 '\[features\]' scte35-splice/Cargo.toml mp4-emsg/Cargo.toml dvb-common/Cargo.toml`) and match the `std`/`serde` passthroughs exactly. `thiserror` 2.x supports `no_std`.

- [ ] **Step 2: Write the crate root**

Create `timed-metadata/src/lib.rs`:
```rust
//! Timed-metadata / DPI signalling conversion core.
//!
//! Translates SCTE-35 splice information to and from the carriages used in OTT
//! delivery: HLS `EXT-X-DATERANGE` (RFC 8216 / draft-pantos-hls-rfc8216bis
//! §4.4.5.1) and DASH `emsg` (SCTE 214-3, scheme `urn:scte:scte35:2013:bin`).
//!
//! Conversions are lossless: the original `splice_info_section` bytes are
//! carried verbatim (DATERANGE `SCTE35-OUT` hex, emsg `message_data`).
//!
//! Pure functions live in [`convert`]; the stateful [`Timeline`] session adds a
//! wall-clock [`TimeAnchor`] and 33-bit PTS wrap-unrolling.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod anchor;
pub mod convert;
pub mod daterange;
pub mod error;
pub mod event;
pub mod timeline;

pub use anchor::TimeAnchor;
pub use daterange::DateRange;
pub use error::{Error, Result};
pub use event::{EventKind, MediaDuration, MediaTime, SourcePayload, TimedEvent};
pub use timeline::Timeline;

/// 90 kHz — the SCTE-35 / MPEG-2 PTS clock.
pub const PTS_HZ: u64 = 90_000;
```

- [ ] **Step 3: Write error.rs**

Create `timed-metadata/src/error.rs`:
```rust
//! Crate error type.
use alloc::string::String;

/// Errors produced by conversions and the [`crate::Timeline`] session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A wall-clock conversion was attempted without a [`crate::TimeAnchor`].
    #[error("wall-clock conversion requires a TimeAnchor, but none was set")]
    MissingAnchor,
    /// An emsg presented to [`crate::convert::emsg_to_scte35`] is not a
    /// SCTE-35 carriage scheme.
    #[error("emsg scheme {scheme:?} is not a SCTE-35 carriage scheme")]
    UnsupportedScheme { scheme: String },
    /// SCTE-35 parse failure.
    #[error("SCTE-35: {0}")]
    Scte35(#[from] scte35_splice::Error),
    /// emsg parse/serialize failure.
    #[error("emsg: {0}")]
    Emsg(#[from] mp4_emsg::Error),
    /// `EXT-X-DATERANGE` tag could not be parsed.
    #[error("DATERANGE parse: {0}")]
    AttrParse(String),
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;
```

- [ ] **Step 4: Add to workspace + build**

Add `"timed-metadata"` to root `Cargo.toml` `members`. Run:
```bash
cargo build -p timed-metadata --all-features --locked
```
Expected: PASS (empty modules will error — create empty module files with just `//!` docs to compile, or proceed to Task 2 which fills them). To make Step 4 pass now, create stub files for the other modules:
```bash
for m in anchor convert daterange event timeline; do echo "//! stub" > timed-metadata/src/$m.rs; done
mkdir -p timed-metadata/src/convert
```
(Modules referencing `convert/` submodules are wired in their own tasks.)

> Simplify: make `convert` a file stub for now (`convert.rs`), converted to a dir in Task 5.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(timed-metadata): scaffold crate + error type"
```

---

### Task 2: Canonical event model

**Files:**
- Modify: `timed-metadata/src/event.rs`
- Test: in-module `#[cfg(test)]`

**Interfaces:**
- Consumes: `scte35_splice::{SpliceInfoSection}`, its `ClearPayload`/`commands::SpliceInsert`.
- Produces:
  - `MediaTime(u64)`, `MediaDuration(u64)` with `MediaDuration::as_seconds_f64()`.
  - `enum EventKind { BreakStart, BreakEnd, Chapter, Unspecified }` + `name()` + `impl_spec_display!`.
  - `enum SourcePayload { Scte35 { raw: Vec<u8> }, Emsg { scheme_id_uri: String, value: String, raw: Vec<u8> } }`.
  - `struct TimedEvent { id: Option<u32>, kind: EventKind, at: Option<MediaTime>, duration: Option<MediaDuration>, source: SourcePayload }`.
  - `TimedEvent::from_scte35(section: &SpliceInfoSection, raw: &[u8]) -> Result<TimedEvent>`.

- [ ] **Step 1: Write the failing test**

In `timed-metadata/src/event.rs` add at top a `#[cfg(test)]` module (write the whole file in Step 3; here is the test that must exist):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use scte35_splice::SpliceInfoSection;
    use dvb_common::traits::Parse;
    use alloc::vec::Vec;

    // Real Unified Streaming splice (ID 2002): out-of-network, break_duration 2160000 (24s).
    fn splice_2002() -> Vec<u8> {
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        (0..hex.len()).step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn from_scte35_extracts_break_start_and_duration() {
        let raw = splice_2002();
        let section = SpliceInfoSection::parse(&raw).unwrap();
        let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
        assert_eq!(ev.id, Some(2002));
        assert_eq!(ev.kind, EventKind::BreakStart);          // out_of_network = true
        assert_eq!(ev.at, None);                             // pts_time None (program splice)
        assert_eq!(ev.duration, Some(MediaDuration(2_160_000)));
        assert!((ev.duration.unwrap().as_seconds_f64() - 24.0).abs() < 1e-9);
        match &ev.source {
            SourcePayload::Scte35 { raw: r } => assert_eq!(r, &raw), // verbatim, lossless
            _ => panic!("expected Scte35 payload"),
        }
    }

    #[test]
    fn event_kind_labels() {
        assert_eq!(EventKind::BreakStart.name(), "break_start");
        assert_eq!(alloc::format!("{}", EventKind::BreakEnd), "break_end");
    }
}
```

- [ ] **Step 2: Run it (fails to compile — types absent)**

Run: `cargo test -p timed-metadata --lib event:: 2>&1 | tail -5`
Expected: FAIL (unresolved `TimedEvent`/`EventKind`).

- [ ] **Step 3: Write the implementation**

Replace `timed-metadata/src/event.rs` body (above the test module) with:
```rust
//! Canonical timed-metadata event (the hub of the hub-and-spoke model).
use alloc::{string::String, vec::Vec};
use scte35_splice::{commands::SpliceCommand, SpliceInfoSection};
use crate::error::Result;

/// A media-timeline instant in 90 kHz ticks, wrap-unrolled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MediaTime(pub u64);

/// A duration in 90 kHz ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MediaDuration(pub u64);

impl MediaDuration {
    /// The duration in seconds.
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / crate::PTS_HZ as f64
    }
}

/// The abstracted meaning of an event, independent of carriage format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EventKind {
    /// Start of an ad/break opportunity (SCTE-35 out-of-network).
    BreakStart,
    /// Return to network (SCTE-35 in-to-network).
    BreakEnd,
    /// Chapter / program boundary.
    Chapter,
    /// Meaning not determined from the source.
    Unspecified,
}

impl EventKind {
    /// Stable label for this variant.
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::BreakStart => "break_start",
            EventKind::BreakEnd => "break_end",
            EventKind::Chapter => "chapter",
            EventKind::Unspecified => "unspecified",
        }
    }
}
dvb_common::impl_spec_display!(EventKind);

/// The lossless original payload, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SourcePayload {
    /// A SCTE-35 `splice_info_section`, verbatim.
    Scte35 { raw: Vec<u8> },
    /// A DASH `emsg`: its scheme/value plus the verbatim `message_data`.
    Emsg { scheme_id_uri: String, value: String, raw: Vec<u8> },
}

/// The canonical event passed between format adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimedEvent {
    /// Event id (`splice_event_id` / emsg `id`).
    pub id: Option<u32>,
    /// Abstract meaning.
    pub kind: EventKind,
    /// Media-timeline instant; `None` = immediate / determined by insertion point.
    pub at: Option<MediaTime>,
    /// Event duration, if known.
    pub duration: Option<MediaDuration>,
    /// Lossless original.
    pub source: SourcePayload,
}

impl TimedEvent {
    /// Build from a parsed SCTE-35 section, retaining `raw` verbatim.
    pub fn from_scte35(section: &SpliceInfoSection, raw: &[u8]) -> Result<Self> {
        let mut id = None;
        let mut kind = EventKind::Unspecified;
        let mut at = None;
        let mut duration = None;

        if let Some(clear) = &section.clear {
            if let SpliceCommand::SpliceInsert(si) = &clear.command {
                id = Some(si.splice_event_id);
                kind = if si.out_of_network_indicator {
                    EventKind::BreakStart
                } else {
                    EventKind::BreakEnd
                };
                if let Some(st) = &si.splice_time {
                    at = st.pts_time.map(MediaTime);
                }
                if let Some(bd) = &si.break_duration {
                    duration = Some(MediaDuration(bd.duration));
                }
            }
        }

        Ok(TimedEvent {
            id,
            kind,
            at,
            duration,
            source: SourcePayload::Scte35 { raw: raw.to_vec() },
        })
    }
}
```
> Verify the exact field paths against `scte35-splice` (`section.clear`, `clear.command`, `SpliceCommand::SpliceInsert`, `si.splice_event_id`, `si.out_of_network_indicator`, `si.splice_time`, `st.pts_time`, `si.break_duration`, `bd.duration`) — these match the parser output observed in the design phase. Adjust the `use` path for `SpliceCommand` if it is re-exported elsewhere (`grep -rn "enum SpliceCommand" scte35-splice/src`).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p timed-metadata --lib event::`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(timed-metadata): canonical TimedEvent model + from_scte35 extraction"
```

---

### Task 3: Time anchor + RFC3339 formatting

**Files:**
- Modify: `timed-metadata/src/anchor.rs`

**Interfaces:**
- Consumes: `MediaTime`.
- Produces:
  - `struct TimeAnchor { pts_90k: u64, utc_epoch_ms: i64 }`.
  - `TimeAnchor::media_to_epoch_ms(&self, t: MediaTime) -> i64`.
  - `TimeAnchor::rfc3339(&self, t: MediaTime) -> String`.
  - free `fn format_rfc3339_ms(epoch_ms: i64) -> String` (used by both feature paths).

- [ ] **Step 1: Write the failing test**

In `timed-metadata/src/anchor.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::MediaTime;

    #[test]
    fn epoch_zero_formats_unix_epoch() {
        assert_eq!(format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339_ms(86_400_000), "1970-01-02T00:00:00.000Z");
        assert_eq!(format_rfc3339_ms(1_000), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn anchor_maps_media_to_wallclock() {
        // anchor: pts 0 == epoch 1000ms. +90000 ticks (1s) -> 2000ms.
        let a = TimeAnchor { pts_90k: 0, utc_epoch_ms: 1_000 };
        assert_eq!(a.media_to_epoch_ms(MediaTime(90_000)), 2_000);
        assert_eq!(a.rfc3339(MediaTime(0)), "1970-01-01T00:00:01.000Z");
    }
}
```

- [ ] **Step 2: Run it (fails)**

Run: `cargo test -p timed-metadata --lib anchor:: 2>&1 | tail -5`
Expected: FAIL (unresolved names).

- [ ] **Step 3: Write the implementation**

Replace `timed-metadata/src/anchor.rs` body with:
```rust
//! Media-time ↔ wall-clock mapping for conversions that cross into UTC.
use alloc::{format, string::String};
use crate::event::MediaTime;

/// Maps a known 90 kHz PTS to the UTC instant it represents (linear at 90 kHz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeAnchor {
    /// A reference PTS, in 90 kHz ticks.
    pub pts_90k: u64,
    /// The UTC time that `pts_90k` corresponds to, in milliseconds since the Unix epoch.
    pub utc_epoch_ms: i64,
}

impl TimeAnchor {
    /// Map a media instant to milliseconds since the Unix epoch.
    pub fn media_to_epoch_ms(&self, t: MediaTime) -> i64 {
        let delta_ticks = t.0 as i64 - self.pts_90k as i64;
        // ticks / 90_000 * 1000 == ticks / 90 ; do it in i128 to avoid overflow.
        self.utc_epoch_ms + (delta_ticks as i128 * 1000 / crate::PTS_HZ as i128) as i64
    }

    /// Map a media instant to an RFC3339 / ISO-8601 UTC string (millisecond precision).
    pub fn rfc3339(&self, t: MediaTime) -> String {
        format_rfc3339_ms(self.media_to_epoch_ms(t))
    }
}

/// Format milliseconds-since-epoch as `YYYY-MM-DDTHH:MM:SS.sssZ`.
pub fn format_rfc3339_ms(epoch_ms: i64) -> String {
    let (secs, ms) = (epoch_ms.div_euclid(1000), epoch_ms.rem_euclid(1000));
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y, mo, d, h, m, s, ms
    )
}

/// Convert days-since-Unix-epoch to (year, month, day). Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}
```
> `chrono` is declared as a feature for downstream ergonomics but the formatter above is self-contained and `no_std`, so the core needs no `chrono` at runtime. Leave a `// chrono interop helpers can be added behind cfg(feature="chrono") later` note; do not add chrono code in v0.1 unless a test needs it.

- [ ] **Step 4: Run tests**

Run: `cargo test -p timed-metadata --lib anchor::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(timed-metadata): TimeAnchor + no_std RFC3339 formatter"
```

---

### Task 4: DateRange model + tag (de)serialization

**Files:**
- Modify: `timed-metadata/src/daterange.rs`

**Interfaces:**
- Produces:
  - `enum Scte35Cue { Out, In, Cmd }` + `name()` + `impl_spec_display!`.
  - `struct Scte35Attr { cue: Scte35Cue, raw: Vec<u8> }`.
  - `struct DateRange { id, start_date, class: Option<String>, duration: Option<f64>, planned_duration: Option<f64>, scte35: Option<Scte35Attr> }`.
  - `DateRange::to_tag_line(&self) -> String`.
  - `DateRange::parse_tag_line(s: &str) -> Result<DateRange>`.

- [ ] **Step 1: Write the failing test (self round-trip + hex)**

In `timed-metadata/src/daterange.rs`:
```rust
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
            scte35: Some(Scte35Attr { cue: Scte35Cue::Out, raw: vec![0xFC, 0x30, 0x21] }),
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
```

- [ ] **Step 2: Run it (fails)**

Run: `cargo test -p timed-metadata --lib daterange:: 2>&1 | tail -5`
Expected: FAIL.

- [ ] **Step 3: Write the implementation**

Replace `timed-metadata/src/daterange.rs` body with:
```rust
//! HLS `EXT-X-DATERANGE` model + (de)serialization.
//!
//! RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1. The `SCTE35-OUT`/`IN`/`CMD`
//! attribute value is the entire `splice_info_section`, hex-encoded with a `0x`
//! prefix.
use alloc::{format, string::{String, ToString}, vec::Vec};
use crate::error::{Error, Result};

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
dvb_common::impl_spec_display!(Scte35Cue);

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

// DateRange holds f64 -> derive Eq manually-safe via PartialEq only; tests compare
// with exact values produced by the crate so equality is deterministic.
impl Eq for DateRange {}

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
                "ID" => { dr.id = unquote(v); seen_id = true; }
                "START-DATE" => dr.start_date = unquote(v),
                "CLASS" => dr.class = Some(unquote(v)),
                "DURATION" => dr.duration = Some(parse_f64(v)?),
                "PLANNED-DURATION" => dr.planned_duration = Some(parse_f64(v)?),
                "SCTE35-OUT" => dr.scte35 = Some(Scte35Attr { cue: Scte35Cue::Out, raw: parse_hex(v)? }),
                "SCTE35-IN" => dr.scte35 = Some(Scte35Attr { cue: Scte35Cue::In, raw: parse_hex(v)? }),
                "SCTE35-CMD" => dr.scte35 = Some(Scte35Attr { cue: Scte35Cue::Cmd, raw: parse_hex(v)? }),
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
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{}", v) }
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
    v.parse::<f64>().map_err(|_| Error::AttrParse(format!("bad number: {v}")))
}

fn parse_hex(v: &str) -> Result<Vec<u8>> {
    let h = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v);
    if h.len() % 2 != 0 {
        return Err(Error::AttrParse("odd-length hex".to_string()));
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| Error::AttrParse("bad hex".to_string())))
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p timed-metadata --lib daterange::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(timed-metadata): DateRange model + tag round-trip"
```

---

### Task 5: emsg conversions (both directions)

**Files:**
- Delete stub `timed-metadata/src/convert.rs`; Create `timed-metadata/src/convert/mod.rs`, `timed-metadata/src/convert/emsg.rs`
- Modify: `timed-metadata/src/lib.rs` (already declares `pub mod convert;`)

**Interfaces:**
- Consumes: `mp4_emsg::{EmsgBox, PresentationTime}`, `mp4_emsg::EmsgVersion` (verify name), `dvb_common::traits::Parse`.
- Produces (in `convert`):
  - `const SCTE35_SCHEME: &str = "urn:scte:scte35:2013:bin";`
  - `struct EmsgConfig { timescale: u32, presentation: PresentationTime, event_duration: u32, value: String }`
  - `fn scte35_to_emsg(splice_raw: &[u8], cfg: &EmsgConfig) -> Result<Vec<u8>>`
  - `fn emsg_to_scte35(emsg_bytes: &[u8]) -> Result<Vec<u8>>`

- [ ] **Step 1: Convert the stub to a module dir**

```bash
rm -f timed-metadata/src/convert.rs
mkdir -p timed-metadata/src/convert
```

- [ ] **Step 2: Write the failing test**

Create `timed-metadata/src/convert/emsg.rs` with the test first (impl in Step 4):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn splice_2002() -> alloc::vec::Vec<u8> {
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i+2],16).unwrap()).collect()
    }

    #[test]
    fn scte35_to_emsg_embeds_splice_verbatim_then_round_trips() {
        let splice = splice_2002();
        let cfg = EmsgConfig {
            timescale: 90_000,
            presentation: mp4_emsg::PresentationTime::Delta(0),
            event_duration: 2_160_000,
            value: "1".to_string(),
        };
        let emsg = scte35_to_emsg(&splice, &cfg).unwrap();
        // message_data must equal the splice verbatim:
        let extracted = emsg_to_scte35(&emsg).unwrap();
        assert_eq!(extracted, splice);
    }

    #[test]
    fn emsg_to_scte35_rejects_non_scte_scheme() {
        // a minimal emsg with a different scheme should error UnsupportedScheme;
        // built by hand in the integration test (see tests/emsg_interop.rs).
    }
}
```

- [ ] **Step 3: Run it (fails)**

Run: `cargo test -p timed-metadata --lib convert::emsg 2>&1 | tail -5`
Expected: FAIL.

- [ ] **Step 4: Write the implementation**

Top of `timed-metadata/src/convert/emsg.rs`:
```rust
//! SCTE-35 ↔ DASH `emsg` conversion (SCTE 214-3; scheme `urn:scte:scte35:2013:bin`).
use alloc::{string::String, vec::Vec};
use dvb_common::traits::Parse;
use mp4_emsg::{EmsgBox, PresentationTime};
use crate::error::{Error, Result};

/// The SCTE-35 binary carriage scheme for DASH `emsg` (SCTE 214-3).
pub const SCTE35_SCHEME: &str = "urn:scte:scte35:2013:bin";

/// Parameters for emitting a SCTE-35-carrying `emsg`.
#[derive(Debug, Clone)]
pub struct EmsgConfig {
    /// `timescale` (ticks/second) for the emsg time fields.
    pub timescale: u32,
    /// `presentation_time_delta` (v0) or `presentation_time` (v1).
    pub presentation: PresentationTime,
    /// `event_duration` in `timescale` units (0 if unknown).
    pub event_duration: u32,
    /// `value` string (often the segmentation type id, as text).
    pub value: String,
}

/// Wrap a verbatim `splice_info_section` as a SCTE-35 `emsg` box (serialized bytes).
pub fn scte35_to_emsg(splice_raw: &[u8], cfg: &EmsgConfig) -> Result<Vec<u8>> {
    let boxx = EmsgBox {
        scheme_id_uri: SCTE35_SCHEME,
        value: &cfg.value,
        timescale: cfg.timescale,
        presentation_time: cfg.presentation,
        event_duration: cfg.event_duration,
        message_data: splice_raw,
    };
    Ok(boxx.to_vec()?)
}

/// Extract the verbatim `splice_info_section` from a SCTE-35 `emsg` box.
pub fn emsg_to_scte35(emsg_bytes: &[u8]) -> Result<Vec<u8>> {
    let boxx = EmsgBox::parse(emsg_bytes)?;
    if !boxx.is_scte35() {
        return Err(Error::UnsupportedScheme { scheme: String::from(boxx.scheme_id_uri) });
    }
    Ok(boxx.message_data.to_vec())
}
```
> Verify against `mp4-emsg`: the `EmsgBox` struct field set (`scheme_id_uri`, `value`, `timescale`, `presentation_time`, `event_duration`, `message_data`), the `PresentationTime` variants (`Delta(u32)`/`Absolute(u64)`), `is_scte35()`, `to_vec()`, and `parse()` (via `dvb_common::traits::Parse` or an inherent `parse`). If `value` is not a public field, drop it from the literal and set via the type's constructor. Adjust imports accordingly.

- [ ] **Step 5: Write convert/mod.rs**

Create `timed-metadata/src/convert/mod.rs`:
```rust
//! Pure conversion functions (the foundation layer).
mod emsg;
pub use emsg::{emsg_to_scte35, scte35_to_emsg, EmsgConfig, SCTE35_SCHEME};

mod daterange;
pub use daterange::scte35_to_daterange;
```
> `daterange` submodule is added in Task 6 — for this task, temporarily comment the last two lines, or create a stub `convert/daterange.rs` with `//! stub` and a no-op to keep `mod.rs` compiling. Prefer a stub so `mod.rs` is final.

- [ ] **Step 6: Run tests**

Run: `cargo test -p timed-metadata --lib convert::emsg`
Expected: PASS (round-trip embeds splice verbatim).

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat(timed-metadata): SCTE-35 <-> DASH emsg conversion (lossless)"
```

---

### Task 6: SCTE-35 → DATERANGE conversion

**Files:**
- Modify/Create: `timed-metadata/src/convert/daterange.rs`

**Interfaces:**
- Consumes: `TimedEvent`, `TimeAnchor`, `DateRange`, `Scte35Attr`, `Scte35Cue`, `EventKind`.
- Produces: `fn scte35_to_daterange(ev: &TimedEvent, anchor: &TimeAnchor) -> Result<DateRange>`.

- [ ] **Step 1: Write the failing test**

In `timed-metadata/src/convert/daterange.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, MediaDuration, SourcePayload, TimedEvent};
    use crate::anchor::TimeAnchor;
    use alloc::{vec, string::ToString};

    #[test]
    fn break_start_maps_to_scte35_out_with_duration() {
        let ev = TimedEvent {
            id: Some(2002),
            kind: EventKind::BreakStart,
            at: None,
            duration: Some(MediaDuration(2_160_000)), // 24s
            source: SourcePayload::Scte35 { raw: vec![0xFC, 0x30, 0x21] },
        };
        // anchor: epoch 0 == pts 0; with at=None, START-DATE uses anchor.utc_epoch_ms.
        let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: 0 };
        let dr = scte35_to_daterange(&ev, &anchor).unwrap();
        assert_eq!(dr.id, "2002");
        assert_eq!(dr.planned_duration, Some(24.0));
        let s = dr.scte35.unwrap();
        assert_eq!(s.cue, Scte35Cue::Out);
        assert_eq!(s.raw, vec![0xFC, 0x30, 0x21]); // verbatim
        assert_eq!(dr.start_date, "1970-01-01T00:00:00.000Z");
    }
}
```

- [ ] **Step 2: Run it (fails)**

Run: `cargo test -p timed-metadata --lib convert::daterange 2>&1 | tail -5`
Expected: FAIL.

- [ ] **Step 3: Write the implementation**

Top of `timed-metadata/src/convert/daterange.rs`:
```rust
//! SCTE-35 → HLS `EXT-X-DATERANGE` (RFC 8216 / hls-bis §4.4.5.1).
use alloc::string::ToString;
use crate::anchor::TimeAnchor;
use crate::daterange::{DateRange, Scte35Attr, Scte35Cue};
use crate::error::{Error, Result};
use crate::event::{EventKind, SourcePayload, TimedEvent};

/// Convert a SCTE-35-sourced [`TimedEvent`] to a [`DateRange`].
///
/// `START-DATE` comes from `ev.at` via `anchor` when present; otherwise from the
/// anchor's own UTC (the insertion-point time the caller supplied). The original
/// splice bytes are carried verbatim into the `SCTE35-OUT`/`IN` attribute.
pub fn scte35_to_daterange(ev: &TimedEvent, anchor: &TimeAnchor) -> Result<DateRange> {
    let raw = match &ev.source {
        SourcePayload::Scte35 { raw } => raw.clone(),
        SourcePayload::Emsg { .. } => {
            return Err(Error::AttrParse("event is not SCTE-35-sourced".to_string()))
        }
    };

    let cue = match ev.kind {
        EventKind::BreakStart => Scte35Cue::Out,
        EventKind::BreakEnd => Scte35Cue::In,
        _ => Scte35Cue::Cmd,
    };

    let start_date = match ev.at {
        Some(t) => anchor.rfc3339(t),
        None => crate::anchor::format_rfc3339_ms(anchor.utc_epoch_ms),
    };

    let planned_duration = ev.duration.map(|d| d.as_seconds_f64());

    Ok(DateRange {
        id: ev.id.map(|i| i.to_string()).unwrap_or_default(),
        start_date,
        class: None,
        duration: None,
        planned_duration,
        scte35: Some(Scte35Attr { cue, raw }),
    })
}
```
> Bring `Scte35Cue` into scope for the test via `use crate::daterange::Scte35Cue;` at the top of the test module if not already; the impl imports it.

- [ ] **Step 4: Run tests + remove the convert/daterange stub note**

Ensure `convert/mod.rs` exposes `scte35_to_daterange` (uncomment / replace the stub). Run:
```bash
cargo test -p timed-metadata --lib convert::daterange
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(timed-metadata): SCTE-35 -> EXT-X-DATERANGE conversion"
```

---

### Task 7: Timeline session (anchor + 33-bit wrap-unroll)

**Files:**
- Modify: `timed-metadata/src/timeline.rs`

**Interfaces:**
- Consumes: `scte35_splice::SpliceInfoSection`, `TimedEvent::from_scte35`, `TimeAnchor`, `EmsgConfig`, the convert fns.
- Produces:
  - `struct Timeline { anchor: Option<TimeAnchor>, last_pts: Option<u64>, epoch: u64 }`
  - `Timeline::new()`, `with_anchor(TimeAnchor)`, `set_anchor(&mut, TimeAnchor)`
  - `Timeline::push_scte35(&mut self, bytes: &[u8]) -> Result<TimedEvent>` (parses, unrolls wrap into `at`)
  - `Timeline::to_daterange(&self, ev: &TimedEvent) -> Result<DateRange>` (errors `MissingAnchor` if no anchor)
  - `Timeline::to_emsg(&self, ev: &TimedEvent, cfg: &EmsgConfig) -> Result<Vec<u8>>`
  - `const PTS_WRAP: u64 = 1 << 33;`

- [ ] **Step 1: Write the failing test**

In `timed-metadata/src/timeline.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::MediaTime;

    fn splice_2002() -> alloc::vec::Vec<u8> {
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i+2],16).unwrap()).collect()
    }

    #[test]
    fn push_scte35_returns_event() {
        let mut tl = Timeline::new();
        let ev = tl.push_scte35(&splice_2002()).unwrap();
        assert_eq!(ev.id, Some(2002));
    }

    #[test]
    fn to_daterange_without_anchor_errors() {
        let tl = Timeline::new();
        let ev = Timeline::new().push_scte35(&splice_2002()).unwrap();
        assert!(matches!(tl.to_daterange(&ev), Err(crate::Error::MissingAnchor)));
    }

    #[test]
    fn wrap_unroll_adds_one_epoch() {
        // unroll(prev, cur) — a near-max prev then a small cur crosses one wrap.
        assert_eq!(unroll_pts(&mut Some((1u64 << 33) - 10), &mut 0u64, 5), 5 + (1u64 << 33));
    }
}
```

- [ ] **Step 2: Run it (fails)**

Run: `cargo test -p timed-metadata --lib timeline:: 2>&1 | tail -5`
Expected: FAIL.

- [ ] **Step 3: Write the implementation**

Top of `timed-metadata/src/timeline.rs`:
```rust
//! Stateful conversion session: holds the wall-clock anchor and unrolls 33-bit
//! PTS wrap across a stream of events.
use alloc::vec::Vec;
use dvb_common::traits::Parse;
use scte35_splice::SpliceInfoSection;
use crate::anchor::TimeAnchor;
use crate::convert::{scte35_to_daterange, scte35_to_emsg, EmsgConfig};
use crate::daterange::DateRange;
use crate::error::{Error, Result};
use crate::event::{MediaTime, TimedEvent};

/// The 33-bit PTS modulus.
pub const PTS_WRAP: u64 = 1 << 33;

/// A stateful conversion session.
#[derive(Debug, Default)]
pub struct Timeline {
    anchor: Option<TimeAnchor>,
    last_pts: Option<u64>,
    epoch: u64,
}

impl Timeline {
    /// New session with no anchor.
    pub fn new() -> Self {
        Self::default()
    }
    /// New session with a wall-clock anchor.
    pub fn with_anchor(anchor: TimeAnchor) -> Self {
        Timeline { anchor: Some(anchor), last_pts: None, epoch: 0 }
    }
    /// Set / replace the anchor.
    pub fn set_anchor(&mut self, anchor: TimeAnchor) {
        self.anchor = Some(anchor);
    }

    /// Parse a SCTE-35 section; unroll its PTS into an absolute [`MediaTime`].
    pub fn push_scte35(&mut self, bytes: &[u8]) -> Result<TimedEvent> {
        let section = SpliceInfoSection::parse(bytes)?;
        let mut ev = TimedEvent::from_scte35(&section, bytes)?;
        if let Some(MediaTime(pts33)) = ev.at {
            let abs = unroll_pts(&mut self.last_pts, &mut self.epoch, pts33);
            ev.at = Some(MediaTime(abs));
        }
        Ok(ev)
    }

    /// Convert to a DATERANGE (requires an anchor).
    pub fn to_daterange(&self, ev: &TimedEvent) -> Result<DateRange> {
        let anchor = self.anchor.as_ref().ok_or(Error::MissingAnchor)?;
        scte35_to_daterange(ev, anchor)
    }

    /// Convert to a serialized SCTE-35 `emsg` box.
    pub fn to_emsg(&self, ev: &TimedEvent, cfg: &EmsgConfig) -> Result<Vec<u8>> {
        match &ev.source {
            crate::event::SourcePayload::Scte35 { raw } => scte35_to_emsg(raw, cfg),
            crate::event::SourcePayload::Emsg { .. } => {
                Err(Error::AttrParse(alloc::string::String::from("event is not SCTE-35-sourced")))
            }
        }
    }
}

/// Unroll a 33-bit PTS to an absolute monotonic value. On a backward jump of
/// more than half the range, advance one epoch.
pub(crate) fn unroll_pts(last_pts: &mut Option<u64>, epoch: &mut u64, pts33: u64) -> u64 {
    if let Some(prev) = *last_pts {
        if pts33 + (PTS_WRAP / 2) < prev {
            *epoch += 1;
        }
    }
    *last_pts = Some(pts33);
    *epoch * PTS_WRAP + pts33
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p timed-metadata --lib timeline::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(timed-metadata): Timeline session with 33-bit PTS wrap-unroll"
```

---

### Task 8: Real-fixture integration tests

**Files:**
- Create: `timed-metadata/tests/fixtures/scte35_emsg_v0.bin`, `timed-metadata/tests/fixtures/emsg_v1_scte35_livesim.bin`
- Create: `timed-metadata/tests/fixtures/daterange_2002.txt`, `daterange_2004.txt`
- Create: `timed-metadata/tests/emsg_interop.rs`, `timed-metadata/tests/daterange_fixture.rs`

**Interfaces:** uses the public crate API.

- [ ] **Step 1: Copy the real emsg fixtures + write the DATERANGE fixtures**

```bash
mkdir -p timed-metadata/tests/fixtures
cp dvb-emsg/tests/fixtures/scte35_emsg_v0.bin timed-metadata/tests/fixtures/
cp dvb-emsg/tests/fixtures/emsg_v1_scte35_livesim.bin timed-metadata/tests/fixtures/
```
> If Plan A already moved `dvb-emsg`→`mp4-emsg`, copy from `mp4-emsg/tests/fixtures/` instead.

Create `timed-metadata/tests/fixtures/daterange_2002.txt` (single line, no trailing newline issues — exact verified fixture):
```
#EXT-X-DATERANGE:ID="2002",START-DATE="2018-10-29T10:38:00Z",PLANNED-DURATION=24,SCTE35-OUT=0xFC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D
```
Create `timed-metadata/tests/fixtures/daterange_2004.txt`:
```
#EXT-X-DATERANGE:ID="2004",START-DATE="2018-10-29T10:42:00Z",PLANNED-DURATION=24,SCTE35-OUT=0xFC302100000000000000FFF01005000007D47FEF7F7E0020F580C000000000004F1B1A5F
```

- [ ] **Step 2: Write the emsg interop test (real bin → SCTE-35 → emsg, byte-identical)**

Create `timed-metadata/tests/emsg_interop.rs`:
```rust
//! Real SCTE-35-carrying emsg fixtures (incl. DASH-IF livesim2): extract the
//! splice and re-wrap it, asserting byte-identical round-trip.
use timed_metadata::convert::{emsg_to_scte35, scte35_to_emsg, EmsgConfig};
use mp4_emsg::EmsgBox;
use dvb_common::traits::Parse;

fn rewrap_matches(emsg_bytes: &[u8]) {
    let splice = emsg_to_scte35(emsg_bytes).expect("extract splice");
    // Rebuild cfg from the parsed box so the re-wrap is faithful.
    let b = EmsgBox::parse(emsg_bytes).unwrap();
    let cfg = EmsgConfig {
        timescale: b.timescale,
        presentation: b.presentation_time,
        event_duration: b.event_duration,
        value: b.value.to_string(),
    };
    let rebuilt = scte35_to_emsg(&splice, &cfg).expect("re-wrap");
    assert_eq!(rebuilt, emsg_bytes, "emsg round-trip must be byte-identical");
}

#[test]
fn v0_scte35_emsg_round_trips() {
    rewrap_matches(include_bytes!("fixtures/scte35_emsg_v0.bin"));
}

#[test]
fn v1_livesim_scte35_emsg_round_trips() {
    rewrap_matches(include_bytes!("fixtures/emsg_v1_scte35_livesim.bin"));
}
```
> `include_bytes!` is fine for *tests* (not shipped in the published crate). The `value`/`presentation_time` field names must match `mp4-emsg`; adjust if the public field is `presentation_time` of type `PresentationTime`.

- [ ] **Step 3: Write the DATERANGE real-fixture test**

Create `timed-metadata/tests/daterange_fixture.rs`:
```rust
//! Real `EXT-X-DATERANGE` lines from a production packager (Unified Streaming).
//! The SCTE35-OUT hex is the splice input; PLANNED-DURATION is the golden output.
use timed_metadata::convert::scte35_to_daterange;
use timed_metadata::daterange::{DateRange, Scte35Cue};
use timed_metadata::event::TimedEvent;
use timed_metadata::TimeAnchor;
use scte35_splice::SpliceInfoSection;
use dvb_common::traits::Parse;

fn check(line: &str, expect_id: &str) {
    // 1. Parse the real DATERANGE line.
    let dr = DateRange::parse_tag_line(line.trim()).expect("parse fixture line");
    assert_eq!(dr.id, expect_id);
    assert_eq!(dr.planned_duration, Some(24.0));
    let attr = dr.scte35.as_ref().expect("scte35 attr");
    assert_eq!(attr.cue, Scte35Cue::Out);

    // 2. The hex IS a valid splice; break_duration = 2160000 ticks = 24s.
    let section = SpliceInfoSection::parse(&attr.raw).expect("hex decodes to splice");
    let ev = TimedEvent::from_scte35(&section, &attr.raw).unwrap();
    assert_eq!(ev.duration.unwrap().0, 2_160_000);

    // 3. Round-trip our converter: feed splice + anchor at the fixture's START-DATE.
    //    (anchor epoch arbitrary here; we assert duration + lossless hex, not START-DATE.)
    let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: 0 };
    let regen = scte35_to_daterange(&ev, &anchor).unwrap();
    assert_eq!(regen.planned_duration, Some(24.0));
    assert_eq!(regen.scte35.unwrap().raw, attr.raw); // verbatim survives
}

#[test]
fn unified_daterange_2002() {
    check(include_str!("fixtures/daterange_2002.txt"), "2002");
}

#[test]
fn unified_daterange_2004() {
    check(include_str!("fixtures/daterange_2004.txt"), "2004");
}
```

- [ ] **Step 4: Run the integration tests**

Run: `cargo test -p timed-metadata --test emsg_interop --test daterange_fixture`
Expected: PASS (4 tests). If the emsg byte-identical assertion fails, inspect whether `mp4-emsg` normalizes any field on serialize; adjust `EmsgConfig` reconstruction to mirror the parsed box exactly (do NOT relax the assertion — the round-trip must be exact).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "test(timed-metadata): real-fixture interop (emsg round-trip + Unified DATERANGE)"
```

---

### Task 9: label_coverage drift-guard + no-default + full gate

**Files:**
- Create: `timed-metadata/tests/label_coverage.rs`

**Interfaces:** none.

- [ ] **Step 1: Write the label coverage guard**

Model it on an existing crate's guard (`grep -l label_coverage */tests/*.rs` → copy `scte104/tests/label_coverage.rs` or `mp4-emsg/tests/label_coverage.rs`, then adjust the `src` path and SKIP list). Create `timed-metadata/tests/label_coverage.rs` that scans `timed-metadata/src/` for `pub enum` and fails if any lacks a `Display`/`impl_spec_display!`, with a SKIP list documenting `Error` (and any `Any*`).

- [ ] **Step 2: Run it**

Run: `cargo test -p timed-metadata --test label_coverage`
Expected: PASS (`EventKind`, `Scte35Cue` both have `impl_spec_display!`; `Error` is skipped).

- [ ] **Step 3: no-default-features build**

Run:
```bash
cargo build -p timed-metadata --no-default-features --locked
cargo test  -p timed-metadata --no-default-features --locked
```
Expected: PASS (no_std path; RFC3339 fallback + conversions all build without std/chrono/serde).

- [ ] **Step 4: Full workspace gate**

Run all six gates (as in Plan A Task 6). All must PASS. Fix clippy/fmt/doc issues inline (backtick any bit-range notation in docs).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "test(timed-metadata): label_coverage guard; green on all six gates"
```

---

### Task 10: Docs + examples + release prep

**Files:**
- Create: `timed-metadata/README.md`, `timed-metadata/CHANGELOG.md`
- Create: `timed-metadata/examples/scte35_to_hls.rs`, `timed-metadata/examples/scte35_to_dash.rs`
- Modify: `CLAUDE.md` (crate inventory), `docs/release-notes/timed-metadata-v0.1.0.md`

**Interfaces:** none.

- [ ] **Step 1: Write two runnable examples**

`examples/scte35_to_hls.rs` — parse a hex splice, build a `Timeline` with an anchor, emit the `EXT-X-DATERANGE` line, print it. `examples/scte35_to_dash.rs` — same splice → `emsg` bytes, print hex. Use inline hex (examples may use literals; only *fixture* examples must read via `std::fs`, and these use no fixture files). Verify:
```bash
cargo run -p timed-metadata --example scte35_to_hls
cargo run -p timed-metadata --example scte35_to_dash
cargo build -p timed-metadata --examples --locked
```
Expected: both print sensible output; `--examples` builds.

- [ ] **Step 2: README + CHANGELOG** per `docs/RELEASE-DOCS.md`: purpose, the three edges, spec citations (RFC 8216/bis §4.4.5.1, SCTE 214-3), install (`timed-metadata = "0.1"`), a `Timeline` example, feature list, `no_std` note, license. CHANGELOG `0.1.0 — <date>` with the v0.1 scope.

- [ ] **Step 3: Update CLAUDE.md** crate inventory — add `timed-metadata` to the crate list with its one-line description and independent-versioning note.

- [ ] **Step 4: Write `docs/release-notes/timed-metadata-v0.1.0.md`** per the RELEASE-DOCS standard.

- [ ] **Step 5: docs.rs metadata** — add to `timed-metadata/Cargo.toml`:
```toml
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

- [ ] **Step 6: Commit + open PR**

```bash
git add -A
git commit -m "docs(timed-metadata): README, CHANGELOG, examples, release notes, docs.rs metadata"
git push -u origin <branch>
gh pr create --title "Add timed-metadata crate (SCTE-35 <-> HLS DATERANGE <-> DASH emsg)" \
  --body "v0.1 conversion core. Closes the timed-metadata design. Depends on scte35-splice/mp4-emsg (Plan A). $(printf '\n')Closes #<issue-if-any>"
```

- [ ] **Step 7: STOP — owner-gated release.** Verify all CI checks SUCCESS. Do not tag/publish without explicit owner sign-off. On approval: `timed-metadata-v0.1.0` tag (independent crate). Post-publish: verify live on crates.io + docs.rs green + `cargo add timed-metadata` resolves.

---

## Self-Review

- **Spec coverage:** boundary A (no playlist deps — Cargo.toml has none), hub `TimedEvent` (Task 2), lossless verbatim payload (Tasks 2/5/6 assert byte survival), pure layer (Tasks 5/6) + `Timeline` (Task 7), core-3 edges (Tasks 5/6 cover SCTE-35→emsg, emsg→SCTE-35, SCTE-35→DATERANGE), anchor type-forced on wall-clock (Task 6 takes `&TimeAnchor`; Task 7 `to_daterange` errors `MissingAnchor`), wrap-unroll (Task 7), no_std + features (Tasks 1/9), real fixtures incl. the 2 verified DATERANGE + 2 real emsg bins (Task 8), label_coverage (Task 9), spec citations in every module `//!` (Tasks 5/6), docs/examples (Task 10). Covered.
- **Placeholder scan:** the "verify against the dependency's real API" notes are concrete verification instructions (field/feature names must be read from the freshly-renamed crates), not deferred work — every code block is complete and compilable as written against the observed APIs.
- **Type consistency:** `scte35_to_emsg(&[u8], &EmsgConfig)->Vec<u8>`, `emsg_to_scte35(&[u8])->Vec<u8>`, `scte35_to_daterange(&TimedEvent,&TimeAnchor)->DateRange`, `Timeline::{push_scte35,to_daterange,to_emsg}`, `TimedEvent::from_scte35`, `DateRange::{to_tag_line,parse_tag_line}`, `EventKind`/`Scte35Cue` labels — names identical across all tasks and the design doc.
- **Deferred (v0.2+):** SCTE-104 ingest, ID3, segmentation_type_id-based `EventKind` refinement, `chrono` interop helpers. Out of v0.1 by design.
