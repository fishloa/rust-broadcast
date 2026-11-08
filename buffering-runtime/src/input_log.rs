//! Input log: record what a host fed a [`BufferController`] and replay it offline.
//!
//! A log is plain text, one row per call, header [`Input::CSV_HEADER`]
//! (`kind,now_us,a,b`). The first rows after the header describe the controller
//! (`config` rows, one per [`ControllerConfig`] field, and a `seed` row when a
//! stall-memory seed was used); the remaining rows are the [`Input`]s in call
//! order:
//!
//! | kind | `a` | `b` |
//! |---|---|---|
//! | `chunk` | bytes | last PCR, 27 MHz, or empty |
//! | `media` | track key | media end, 27 MHz |
//! | `tick` | `1` with a position, `0` without | playback position, 27 MHz, or empty |
//! | `register`, `unregister` | track key | empty |
//! | `hint` | [`Hint::name`] token | empty |
//! | `config` (`now_us` 0) | [`ConfigKey::name`] token | value (`true`/`false` for flags) |
//! | `seed` (`now_us` 0) | peak, microseconds | age, microseconds |
//!
//! [`InputLog::replay`] feeds the inputs through one fresh controller built from the recorded
//! configuration, so the same log gives the same decisions on any host. Logs recorded before
//! `config` rows existed parse with [`InputLog::parse_with_config`].

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use crate::config::{ControllerConfig, Micros, Ticks27};
use crate::controller::BufferController;
use crate::error::Error;
use crate::events::{Event, Hint};
use crate::stall_memory::StallMemorySnapshot;

const KIND_CHUNK: &str = "chunk";
const KIND_MEDIA: &str = "media";
const KIND_TICK: &str = "tick";
const KIND_HINT: &str = "hint";
const KIND_REGISTER: &str = "register";
const KIND_UNREGISTER: &str = "unregister";
const KIND_CONFIG: &str = "config";
const KIND_SEED: &str = "seed";
const HAS_POSITION: &str = "1";
const NO_POSITION: &str = "0";
const HINT_CODE_FIRST: u32 = 1;
const CONFIG_NOW_US: Micros = 0;

/// One call a host made into a [`BufferController`].
///
/// Times are the host's monotonic microseconds; positions and media ends are 27 MHz ticks.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Input {
    /// [`BufferController::on_chunk`]: bytes arrived.
    Chunk {
        /// Arrival time.
        now_us: Micros,
        /// Chunk length.
        bytes: u32,
        /// Last PCR of the chunk, 27 MHz.
        pcr: Option<Ticks27>,
    },
    /// [`BufferController::on_track_media`]: a track's buffered media now ends at `end`.
    TrackMedia {
        /// Report time.
        now_us: Micros,
        /// Track key.
        track: u32,
        /// Media end, 27 MHz.
        end: Ticks27,
    },
    /// [`BufferController::on_tick`]: the renderer's playback position.
    Tick {
        /// Tick time.
        now_us: Micros,
        /// Playback position, 27 MHz.
        pts: Option<Ticks27>,
    },
    /// [`BufferController::register_track`].
    Register {
        /// Time of the call.
        now_us: Micros,
        /// Track key.
        track: u32,
    },
    /// [`BufferController::unregister_track`].
    Unregister {
        /// Time of the call.
        now_us: Micros,
        /// Track key.
        track: u32,
    },
    /// [`BufferController::on_hint`].
    Hint {
        /// Hint time.
        now_us: Micros,
        /// The hint.
        hint: Hint,
    },
}

impl Input {
    /// Header row of a text log.
    pub const CSV_HEADER: &'static str = "kind,now_us,a,b";

    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Chunk { .. } => KIND_CHUNK,
            Self::TrackMedia { .. } => KIND_MEDIA,
            Self::Tick { .. } => KIND_TICK,
            Self::Register { .. } => KIND_REGISTER,
            Self::Unregister { .. } => KIND_UNREGISTER,
            Self::Hint { .. } => KIND_HINT,
        }
    }

    /// The time of the call.
    #[must_use]
    pub fn now_us(&self) -> Micros {
        match self {
            Self::Chunk { now_us, .. }
            | Self::TrackMedia { now_us, .. }
            | Self::Tick { now_us, .. }
            | Self::Register { now_us, .. }
            | Self::Unregister { now_us, .. }
            | Self::Hint { now_us, .. } => *now_us,
        }
    }

    /// The same input with its time moved `origin_us` earlier (saturating).
    #[must_use]
    pub fn rebased(self, origin_us: Micros) -> Self {
        let mut out = self;
        let (Self::Chunk { now_us, .. }
        | Self::TrackMedia { now_us, .. }
        | Self::Tick { now_us, .. }
        | Self::Register { now_us, .. }
        | Self::Unregister { now_us, .. }
        | Self::Hint { now_us, .. }) = &mut out;
        *now_us = now_us.saturating_sub(origin_us);
        out
    }

    /// Makes the call this input records on `controller`. Replay is a loop over this.
    pub fn apply(&self, controller: &mut BufferController) {
        match *self {
            Self::Chunk { now_us, bytes, pcr } => controller.on_chunk(now_us, bytes, pcr),
            Self::TrackMedia { now_us, track, end } => {
                controller.on_track_media(now_us, track, end);
            }
            Self::Tick { now_us, pts } => controller.on_tick(now_us, pts),
            Self::Register { track, .. } => controller.register_track(track),
            Self::Unregister { track, .. } => controller.unregister_track(track),
            Self::Hint { now_us, hint } => controller.on_hint(now_us, hint),
        }
    }

    /// Writes one row (columns of [`Input::CSV_HEADER`]) without a trailing newline.
    ///
    /// # Errors
    /// Whatever `out` reports.
    pub fn write_row(&self, out: &mut impl Write) -> core::fmt::Result {
        match self {
            Self::Chunk { now_us, bytes, pcr } => {
                write!(out, "{KIND_CHUNK},{now_us},{bytes},")?;
                write_opt(out, *pcr)
            }
            Self::TrackMedia { now_us, track, end } => {
                write!(out, "{KIND_MEDIA},{now_us},{track},{end}")
            }
            Self::Tick { now_us, pts } => {
                let has = if pts.is_some() {
                    HAS_POSITION
                } else {
                    NO_POSITION
                };
                write!(out, "{KIND_TICK},{now_us},{has},")?;
                write_opt(out, *pts)
            }
            Self::Register { now_us, track } => write!(out, "{KIND_REGISTER},{now_us},{track},"),
            Self::Unregister { now_us, track } => {
                write!(out, "{KIND_UNREGISTER},{now_us},{track},")
            }
            Self::Hint { now_us, hint } => write!(out, "{KIND_HINT},{now_us},{},", hint.name()),
        }
    }

    /// Parses one input row; the inverse of [`Input::write_row`]. A missing trailing column is
    /// read as empty.
    ///
    /// # Errors
    /// [`InputLogError`] naming the row and the reason; `config`, `seed` and header rows are not
    /// inputs.
    pub fn parse_row(line: &str) -> Result<Self, InputLogError> {
        let row = Row::split(line)?;
        let now_us = row.number("now_us", row.now)?;
        match row.kind {
            KIND_CHUNK => Ok(Self::Chunk {
                now_us,
                bytes: row.number("bytes", row.a)?,
                pcr: row.optional("pcr", row.b)?,
            }),
            KIND_MEDIA => Ok(Self::TrackMedia {
                now_us,
                track: row.number("track", row.a)?,
                end: row.number("end", row.b)?,
            }),
            KIND_TICK => {
                let pts = if row.a == HAS_POSITION {
                    Some(row.number("pts", row.b)?)
                } else if row.a == NO_POSITION {
                    None
                } else {
                    return Err(row.fail(Reason::BadNumber {
                        field: "has_position",
                    }));
                };
                Ok(Self::Tick { now_us, pts })
            }
            KIND_REGISTER => Ok(Self::Register {
                now_us,
                track: row.number("track", row.a)?,
            }),
            KIND_UNREGISTER => Ok(Self::Unregister {
                now_us,
                track: row.number("track", row.a)?,
            }),
            KIND_HINT => {
                let hint = (HINT_CODE_FIRST..)
                    .map_while(Hint::from_code)
                    .find(|h| h.name() == row.a)
                    .ok_or_else(|| row.fail(Reason::UnknownHint))?;
                Ok(Self::Hint { now_us, hint })
            }
            _ => Err(row.fail(Reason::UnknownKind)),
        }
    }
}
broadcast_common::impl_spec_display!(Input);

fn write_opt(out: &mut impl Write, v: Option<u64>) -> core::fmt::Result {
    match v {
        Some(v) => write!(out, "{v}"),
        None => Ok(()),
    }
}

struct Row<'a> {
    line: &'a str,
    kind: &'a str,
    now: &'a str,
    a: &'a str,
    b: &'a str,
}

impl<'a> Row<'a> {
    fn split(line: &'a str) -> Result<Self, InputLogError> {
        let mut f = line.trim_end_matches('\r').split(',');
        let row = Self {
            line,
            kind: f.next().unwrap_or(""),
            now: f.next().unwrap_or(""),
            a: f.next().unwrap_or(""),
            b: f.next().unwrap_or(""),
        };
        if row.now.is_empty() {
            return Err(row.fail(Reason::MissingField { field: "now_us" }));
        }
        Ok(row)
    }

    fn fail(&self, reason: Reason) -> InputLogError {
        InputLogError {
            line: self.line.to_string(),
            reason,
        }
    }

    fn number<T: core::str::FromStr>(
        &self,
        field: &'static str,
        v: &str,
    ) -> Result<T, InputLogError> {
        if v.is_empty() {
            return Err(self.fail(Reason::MissingField { field }));
        }
        v.parse()
            .map_err(|_| self.fail(Reason::BadNumber { field }))
    }

    fn optional<T: core::str::FromStr>(
        &self,
        field: &'static str,
        v: &str,
    ) -> Result<Option<T>, InputLogError> {
        if v.is_empty() {
            return Ok(None);
        }
        v.parse()
            .map(Some)
            .map_err(|_| self.fail(Reason::BadNumber { field }))
    }
}

/// Why a log row or log failed to parse or replay.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The first line is not [`Input::CSV_HEADER`].
    BadHeader,
    /// The row kind is not one of the log's kinds.
    UnknownKind,
    /// A required column is empty or absent.
    MissingField {
        /// The column.
        field: &'static str,
    },
    /// A column is not a number of the required type.
    BadNumber {
        /// The column.
        field: &'static str,
    },
    /// The `hint` token names no [`Hint`].
    UnknownHint,
    /// The `config` token names no [`ConfigKey`].
    UnknownConfigKey,
    /// The recorded configuration fails [`ControllerConfig::validate`].
    InvalidConfig(Error),
    /// The log has no `config` rows; use [`InputLog::parse_with_config`].
    NoConfig,
}

impl Reason {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::BadHeader => "bad header",
            Self::UnknownKind => "unknown kind",
            Self::MissingField { .. } => "missing field",
            Self::BadNumber { .. } => "bad number",
            Self::UnknownHint => "unknown hint",
            Self::UnknownConfigKey => "unknown config key",
            Self::InvalidConfig(_) => "invalid config",
            Self::NoConfig => "no config",
        }
    }
}
broadcast_common::impl_spec_display!(Reason);

/// A log row that could not be read.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputLogError {
    /// The offending line.
    pub line: String,
    /// Why it was rejected.
    pub reason: Reason,
}

impl core::fmt::Display for InputLogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.reason)?;
        match self.reason {
            Reason::MissingField { field } | Reason::BadNumber { field } => {
                write!(f, " ({field})")?;
            }
            Reason::InvalidConfig(e) => write!(f, " ({e})")?,
            _ => {}
        }
        write!(f, " in row {:?}", self.line)
    }
}

impl core::error::Error for InputLogError {}

macro_rules! config_keys {
    ($($variant:ident => $token:literal, $($path:ident).+ : $ty:ty;)*) => {
        /// A [`ControllerConfig`] field as it appears in a `config` row.
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum ConfigKey {
            $(
                #[doc = concat!("`", stringify!($($path).+), "`.")]
                $variant,
            )*
        }

        impl ConfigKey {
            /// Every key, in the order a log writes them.
            pub const ALL: &'static [ConfigKey] = &[$(ConfigKey::$variant),*];

            /// The token written in column `a` of a `config` row.
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant => $token,)*
                }
            }

            /// The key with this token.
            #[must_use]
            pub fn from_name(token: &str) -> Option<Self> {
                match token {
                    $($token => Some(Self::$variant),)*
                    _ => None,
                }
            }

            fn write_value(&self, cfg: &ControllerConfig, out: &mut impl Write) -> core::fmt::Result {
                match self {
                    $(Self::$variant => write!(out, "{}", cfg.$($path).+),)*
                }
            }

            fn set(&self, cfg: &mut ControllerConfig, value: &str) -> Option<()> {
                match self {
                    $(Self::$variant => cfg.$($path).+ = value.parse::<$ty>().ok()?,)*
                }
                Some(())
            }
        }
    };
}

config_keys! {
    JitterMinWindow => "jitter.min_window_us", jitter.min_window_us: u64;
    JitterInterval => "jitter.interval_us", jitter.interval_us: u64;
    JitterOutlier => "jitter.outlier_us", jitter.outlier_us: u64;
    JitterBucket => "jitter.bucket_us", jitter.bucket_us: u32;
    JitterBuckets => "jitter.buckets", jitter.buckets: u16;
    JitterForget => "jitter.forget_q15", jitter.forget_q15: u16;
    JitterQuantile => "jitter.quantile_permille", jitter.quantile_permille: u16;
    JitterHold => "jitter.hold_us", jitter.hold_us: u64;
    JitterDecay => "jitter.decay_us", jitter.decay_us: u64;
    JitterMaxSamples => "jitter.max_samples", jitter.max_samples: u16;
    JitterDiscontinuity => "jitter.discontinuity_us", jitter.discontinuity_us: u64;
    PolicyMin => "policy.min_us", policy.min_us: u64;
    PolicyInitial => "policy.initial_us", policy.initial_us: u64;
    PolicyMax => "policy.max_us", policy.max_us: u64;
    PolicyK1 => "policy.k1_q8", policy.k1_q8: u16;
    PolicyMargin => "policy.margin_us", policy.margin_us: u64;
    PolicyK2 => "policy.k2_q8", policy.k2_q8: u16;
    PolicyAfterRebuffer => "policy.after_rebuffer_q8", policy.after_rebuffer_q8: u16;
    PolicyMaxStepDown => "policy.max_step_down_us_per_s", policy.max_step_down_us_per_s: u32;
    PolicyReprimeFloor => "policy.reprime_floor_us", policy.reprime_floor_us: u64;
    PolicyReprimeHold => "policy.reprime_hold_us", policy.reprime_hold_us: u64;
    StarveThreshold => "starve_threshold_us", starve_threshold_us: u64;
    NodataAfter => "nodata_after_us", nodata_after_us: u64;
    TrackSilenceTimeout => "track_silence_timeout_us", track_silence_timeout_us: u64;
    TrackGranularity => "track_granularity", track_granularity: bool;
    GranularityHold => "granularity_hold_us", granularity_hold_us: u64;
    GranularityDecay => "granularity_decay_us", granularity_decay_us: u64;
}
broadcast_common::impl_spec_display!(ConfigKey);

/// A complete, self-describing input log: the controller configuration, the optional stall-memory
/// seed, and the inputs in call order.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputLog {
    /// Configuration the controller was built from.
    pub config: ControllerConfig,
    /// Stall-memory seed the controller was built with.
    pub seed: Option<StallMemorySnapshot>,
    /// The recorded calls, in order.
    pub inputs: Vec<Input>,
}

impl InputLog {
    /// A log of `inputs` for a controller built from `config` and `seed`.
    #[must_use]
    pub fn new(
        config: ControllerConfig,
        seed: Option<StallMemorySnapshot>,
        inputs: Vec<Input>,
    ) -> Self {
        Self {
            config,
            seed,
            inputs,
        }
    }

    /// Writes the header, the `config` rows, the `seed` row if any, then one row per input, each
    /// line ending in a newline.
    ///
    /// # Errors
    /// Whatever `out` reports.
    pub fn write(&self, out: &mut impl Write) -> core::fmt::Result {
        writeln!(out, "{}", Input::CSV_HEADER)?;
        for key in ConfigKey::ALL {
            write!(out, "{KIND_CONFIG},{CONFIG_NOW_US},{},", key.name())?;
            key.write_value(&self.config, out)?;
            out.write_char('\n')?;
        }
        if let Some(s) = &self.seed {
            writeln!(
                out,
                "{KIND_SEED},{CONFIG_NOW_US},{},{}",
                s.peak_us, s.age_us
            )?;
        }
        for input in &self.inputs {
            input.write_row(out)?;
            out.write_char('\n')?;
        }
        Ok(())
    }

    /// Parses a log written by [`InputLog::write`].
    ///
    /// # Errors
    /// [`InputLogError`] for a bad row, or [`Reason::NoConfig`] when the log has no `config` rows.
    pub fn parse(text: &str) -> Result<Self, InputLogError> {
        let (log, had_config) = Self::parse_inner(text, ControllerConfig::DEFAULT)?;
        if had_config {
            Ok(log)
        } else {
            Err(InputLogError {
                line: String::new(),
                reason: Reason::NoConfig,
            })
        }
    }

    /// Parses a log; `fallback` is used when it has no `config` rows (logs recorded before
    /// `config` rows existed). Recorded `config` rows win over `fallback`.
    ///
    /// # Errors
    /// [`InputLogError`] for a bad row.
    pub fn parse_with_config(
        text: &str,
        fallback: ControllerConfig,
    ) -> Result<Self, InputLogError> {
        Self::parse_inner(text, fallback).map(|(log, _)| log)
    }

    fn parse_inner(text: &str, fallback: ControllerConfig) -> Result<(Self, bool), InputLogError> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header = lines.next().unwrap_or("");
        if header.trim_end_matches('\r') != Input::CSV_HEADER {
            return Err(InputLogError {
                line: header.to_string(),
                reason: Reason::BadHeader,
            });
        }
        let mut config = fallback;
        let mut had_config = false;
        let mut seed = None;
        let mut inputs = Vec::new();
        for line in lines {
            let row = Row::split(line)?;
            match row.kind {
                KIND_CONFIG => {
                    let key = ConfigKey::from_name(row.a)
                        .ok_or_else(|| row.fail(Reason::UnknownConfigKey))?;
                    key.set(&mut config, row.b)
                        .ok_or_else(|| row.fail(Reason::BadNumber { field: "value" }))?;
                    had_config = true;
                }
                KIND_SEED => {
                    seed = Some(StallMemorySnapshot::new(
                        row.number("peak_us", row.a)?,
                        row.number("age_us", row.b)?,
                    ));
                }
                _ => inputs.push(Input::parse_row(line)?),
            }
        }
        config.validate().map_err(|e| InputLogError {
            line: String::new(),
            reason: Reason::InvalidConfig(e),
        })?;
        Ok((
            Self {
                config,
                seed,
                inputs,
            },
            had_config,
        ))
    }

    /// Moves every input's time so the first input is at 0.
    pub fn rebase(&mut self) {
        let Some(origin) = self.inputs.first().map(Input::now_us) else {
            return;
        };
        for input in &mut self.inputs {
            *input = input.rebased(origin);
        }
    }

    /// A controller built from the recorded configuration and seed.
    ///
    /// # Errors
    /// [`Error`] when the configuration is invalid.
    pub fn controller(&self) -> Result<BufferController, Error> {
        BufferController::new(self.config, self.seed)
    }

    /// Replays every input through a fresh [`InputLog::controller`] and returns each event with
    /// the time of the input that produced it.
    ///
    /// # Errors
    /// [`Error`] when the recorded configuration is invalid.
    pub fn replay(&self) -> Result<Vec<(Micros, Event)>, Error> {
        let mut controller = self.controller()?;
        let mut events = Vec::new();
        for input in &self.inputs {
            input.apply(&mut controller);
            while let Some(e) = controller.poll_event() {
                events.push((input.now_us(), e));
            }
        }
        Ok(events)
    }
}
