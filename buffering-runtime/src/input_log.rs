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
//!
//! A span value is its whole microsecond count (`jitter.min_window,2000000`) and a location is
//! its 27 MHz tick count; both are the units every log has always carried. A token recorded
//! before the config named spans without a unit suffix (`jitter.min_window_us`,
//! `policy.max_step_down_us_per_s`) is still read, so an archived log replays unchanged.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use core::time::Duration;

use crate::config::ControllerConfig;
use crate::controller::BufferController;
use crate::error::Error;
use crate::events::{Event, Hint};
use crate::stall_memory::StallMemorySnapshot;
use crate::time::{Clock27, duration_between, duration_micros, micros_as_duration};

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
const CONFIG_NOW: &str = "0";

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
        now_us: Duration,
        /// Chunk length.
        bytes: u32,
        /// Last PCR of the chunk, 27 MHz.
        pcr: Option<Clock27>,
    },
    /// [`BufferController::on_track_media`]: a track's buffered media now ends at `end`.
    TrackMedia {
        /// Report time.
        now_us: Duration,
        /// Track key.
        track: u32,
        /// Media end, 27 MHz.
        end: Clock27,
    },
    /// [`BufferController::on_tick`]: the renderer's playback position.
    Tick {
        /// Tick time.
        now_us: Duration,
        /// Playback position, 27 MHz.
        pts: Option<Clock27>,
    },
    /// [`BufferController::register_track`].
    Register {
        /// Time of the call.
        now_us: Duration,
        /// Track key.
        track: u32,
    },
    /// [`BufferController::unregister_track`].
    Unregister {
        /// Time of the call.
        now_us: Duration,
        /// Track key.
        track: u32,
    },
    /// [`BufferController::on_hint`].
    Hint {
        /// Hint time.
        now_us: Duration,
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
    pub fn now_us(&self) -> Duration {
        match self {
            Self::Chunk { now_us, .. }
            | Self::TrackMedia { now_us, .. }
            | Self::Tick { now_us, .. }
            | Self::Register { now_us, .. }
            | Self::Unregister { now_us, .. }
            | Self::Hint { now_us, .. } => *now_us,
        }
    }

    /// The same input with its time moved `origin` earlier (saturating).
    #[must_use]
    pub fn rebased(self, origin_us: Duration) -> Self {
        let mut out = self;
        let (Self::Chunk { now_us, .. }
        | Self::TrackMedia { now_us, .. }
        | Self::Tick { now_us, .. }
        | Self::Register { now_us, .. }
        | Self::Unregister { now_us, .. }
        | Self::Hint { now_us, .. }) = &mut out;
        *now_us = duration_between(origin_us, *now_us);
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
                write!(out, "{KIND_CHUNK},{},{bytes},", duration_micros(*now_us))?;
                write_opt(out, pcr.map(|p| p.ticks()))
            }
            Self::TrackMedia { now_us, track, end } => {
                write!(
                    out,
                    "{KIND_MEDIA},{},{track},{}",
                    duration_micros(*now_us),
                    end.ticks()
                )
            }
            Self::Tick { now_us, pts } => {
                let has = if pts.is_some() {
                    HAS_POSITION
                } else {
                    NO_POSITION
                };
                write!(out, "{KIND_TICK},{},{has},", duration_micros(*now_us))?;
                write_opt(out, pts.map(|p| p.ticks()))
            }
            Self::Register { now_us, track } => {
                write!(out, "{KIND_REGISTER},{},{track},", duration_micros(*now_us))
            }
            Self::Unregister { now_us, track } => {
                write!(
                    out,
                    "{KIND_UNREGISTER},{},{track},",
                    duration_micros(*now_us)
                )
            }
            Self::Hint { now_us, hint } => {
                write!(
                    out,
                    "{KIND_HINT},{},{},",
                    duration_micros(*now_us),
                    hint.name()
                )
            }
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
        let now_us = micros_as_duration(row.micros("now_us", row.now)?);
        match row.kind {
            KIND_CHUNK => Ok(Self::Chunk {
                now_us,
                bytes: row.number("bytes", row.a)?,
                pcr: row.optional_position("pcr", row.b)?,
            }),
            KIND_MEDIA => Ok(Self::TrackMedia {
                now_us,
                track: row.number("track", row.a)?,
                end: row.position("end", row.b)?,
            }),
            KIND_TICK => {
                let pts = if row.a == HAS_POSITION {
                    Some(row.position("pts", row.b)?)
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

    /// A whole-microsecond count of the row (every time column carries
    /// microseconds).
    fn micros(&self, field: &'static str, v: &str) -> Result<u64, InputLogError> {
        self.number(field, v)
    }

    /// A 27 MHz position of the row.
    fn position(&self, field: &'static str, v: &str) -> Result<Clock27, InputLogError> {
        let ticks = self.number(field, v)?;
        Clock27::from_27mhz(ticks).map_err(|_| self.fail(Reason::BadNumber { field }))
    }

    /// An optional 27 MHz position of the row.
    fn optional_position(
        &self,
        field: &'static str,
        v: &str,
    ) -> Result<Option<Clock27>, InputLogError> {
        if v.is_empty() {
            return Ok(None);
        }
        self.position(field, v).map(Some)
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

/// A `config` row value printer: an integer field as its decimal digits.
fn id_out<T: core::fmt::Display>(v: T) -> T {
    v
}

fn id_in<T: core::str::FromStr>(value: &str) -> Option<T> {
    value.parse().ok()
}

/// A `config` row value printer: a span as its whole microsecond count (the
/// unit a log has always carried), and the way back.
fn span_out(span: Duration) -> u64 {
    duration_micros(span)
}

fn span_in(value: &str) -> Option<Duration> {
    value.parse::<u64>().ok().map(micros_as_duration)
}

fn flag_out(v: bool) -> bool {
    v
}

fn flag_in(value: &str) -> Option<bool> {
    value.parse().ok()
}

macro_rules! config_keys {
    ($($variant:ident => $token:literal, $($path:ident).+ : $ty:ty, $fmt:ident, $parse:ident, $legacy:literal;)*) => {
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

            /// The key with this token, or with the unit-suffixed token an
            /// older log wrote for the same field (the spans lost their `_us`
            /// suffix when they became `Duration`s).
            #[must_use]
            pub fn from_name_or_legacy(token: &str) -> Option<Self> {
                if let Some(key) = Self::from_name(token) {
                    return Some(key);
                }
                match token {
                    $($legacy => Some(Self::$variant),)*
                    _ => None,
                }
            }

            fn write_value(&self, cfg: &ControllerConfig, out: &mut impl Write) -> core::fmt::Result {
                match self {
                    $(Self::$variant => write!(out, "{}", $fmt(cfg.$($path).+)),)*
                }
            }

            fn set(&self, cfg: &mut ControllerConfig, value: &str) -> Option<()> {
                match self {
                    $(Self::$variant => cfg.$($path).+ = $parse(value)?,)*
                }
                Some(())
            }
        }
    };
}

config_keys! {
    JitterMinWindow => "jitter.min_window", jitter.min_window: Duration, span_out, span_in, "jitter.min_window_us";
    JitterInterval => "jitter.interval", jitter.interval: Duration, span_out, span_in, "jitter.interval_us";
    JitterOutlier => "jitter.outlier", jitter.outlier: Duration, span_out, span_in, "jitter.outlier_us";
    JitterBucket => "jitter.bucket", jitter.bucket: Duration, span_out, span_in, "jitter.bucket_us";
    JitterBuckets => "jitter.buckets", jitter.buckets: u16, id_out, id_in, "jitter.buckets";
    JitterForget => "jitter.forget_q15", jitter.forget_q15: u16, id_out, id_in, "jitter.forget_q15";
    JitterQuantile => "jitter.quantile_permille", jitter.quantile_permille: u16, id_out, id_in, "jitter.quantile_permille";
    JitterHold => "jitter.hold", jitter.hold: Duration, span_out, span_in, "jitter.hold_us";
    JitterDecay => "jitter.decay", jitter.decay: Duration, span_out, span_in, "jitter.decay_us";
    JitterMaxSamples => "jitter.max_samples", jitter.max_samples: u16, id_out, id_in, "jitter.max_samples";
    JitterDiscontinuity => "jitter.discontinuity", jitter.discontinuity: Duration, span_out, span_in, "jitter.discontinuity_us";
    PolicyMin => "policy.min", policy.min: Duration, span_out, span_in, "policy.min_us";
    PolicyInitial => "policy.initial", policy.initial: Duration, span_out, span_in, "policy.initial_us";
    PolicyMax => "policy.max", policy.max: Duration, span_out, span_in, "policy.max_us";
    PolicyK1 => "policy.k1_q8", policy.k1_q8: u16, id_out, id_in, "policy.k1_q8";
    PolicyMargin => "policy.margin", policy.margin: Duration, span_out, span_in, "policy.margin_us";
    PolicyK2 => "policy.k2_q8", policy.k2_q8: u16, id_out, id_in, "policy.k2_q8";
    PolicyAfterRebuffer => "policy.after_rebuffer_q8", policy.after_rebuffer_q8: u16, id_out, id_in, "policy.after_rebuffer_q8";
    PolicyMaxStepDown => "policy.max_step_down_per_s", policy.max_step_down_per_s: Duration, span_out, span_in, "policy.max_step_down_us_per_s";
    PolicyReprimeFloor => "policy.reprime_floor", policy.reprime_floor: Duration, span_out, span_in, "policy.reprime_floor_us";
    PolicyReprimeHold => "policy.reprime_hold", policy.reprime_hold: Duration, span_out, span_in, "policy.reprime_hold_us";
    StarveThreshold => "starve_threshold", starve_threshold: Duration, span_out, span_in, "starve_threshold_us";
    NodataAfter => "nodata_after", nodata_after: Duration, span_out, span_in, "nodata_after_us";
    TrackSilenceTimeout => "track_silence_timeout", track_silence_timeout: Duration, span_out, span_in, "track_silence_timeout_us";
    TrackGranularity => "track_granularity", track_granularity: bool, flag_out, flag_in, "track_granularity";
    GranularityHold => "granularity_hold", granularity_hold: Duration, span_out, span_in, "granularity_hold_us";
    GranularityDecay => "granularity_decay", granularity_decay: Duration, span_out, span_in, "granularity_decay_us";
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
            write!(out, "{KIND_CONFIG},{CONFIG_NOW},{},", key.name())?;
            key.write_value(&self.config, out)?;
            out.write_char('\n')?;
        }
        if let Some(s) = &self.seed {
            writeln!(
                out,
                "{KIND_SEED},{CONFIG_NOW},{},{}",
                duration_micros(s.peak),
                duration_micros(s.age)
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
                    let key = ConfigKey::from_name_or_legacy(row.a)
                        .ok_or_else(|| row.fail(Reason::UnknownConfigKey))?;
                    key.set(&mut config, row.b)
                        .ok_or_else(|| row.fail(Reason::BadNumber { field: "value" }))?;
                    had_config = true;
                }
                KIND_SEED => {
                    seed = Some(StallMemorySnapshot::new(
                        micros_as_duration(row.micros("peak", row.a)?),
                        micros_as_duration(row.micros("age", row.b)?),
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
    pub fn replay(&self) -> Result<Vec<(Duration, Event)>, Error> {
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
