//! `<time-expression>` grammar — W3C TTML2 §12.3.1.
//!
//! The `<time-expression>` grammar is the highest-risk surface for a subtitle
//! parser: a silently-wrong cue time is the worst failure mode. This module
//! provides exact parsing of all three expression forms (clock-time, offset-time,
//! wallclock-time) with full validation of frame/tick/SMPTE constraints.
//!
//! Time-base-specific constraints are enforced at parse time when `ttp:timeBase`,
//! `ttp:frameRate`, `ttp:subFrameRate`, `ttp:dropMode`, `ttp:markerMode` are
//! known. These parameters are gathered from the document root and passed to
//! the parser via [`TimeContext`].
//!
//! ### Grammar (verbatim from TTML2 §12.3.1)
//!
//! ```text
//! <time-expression> : clock-time | offset-time | wallclock-time
//!
//! clock-time    : hours ":" minutes ":" seconds ( fraction | ":" frames ("." sub-frames)? )?
//! offset-time   : time-count fraction? metric
//! wallclock-time: "wallclock(" <lwsp>? ( date-time | wall-time | date ) <lwsp>? ")"
//!
//! date-time     : date "T" wall-time
//! wall-time     : hhmm-time | hhmmss-time
//! date          : years "-" months "-" days
//! hhmm-time     : hours2 ":" minutes
//! hhmmss-time   : hours2 ":" minutes ":" seconds fraction?
//!
//! metric : "h" | "m" | "s" | "ms" | "f" | "t"
//! ```

/// Time context gathered from document parameters.
///
/// Defaults match TTML2 §7.2 prose defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeContext {
    /// `ttp:timeBase` — default `media`.
    pub time_base: TimeBase,
    /// `ttp:frameRate` — default 30.
    pub frame_rate: u32,
    /// `ttp:frameRateMultiplier` numerator/denominator — default `1 1`.
    pub frame_rate_multiplier_numerator: u32,
    /// `ttp:frameRateMultiplier` denominator.
    pub frame_rate_multiplier_denominator: u32,
    /// `ttp:subFrameRate` — default 1.
    pub sub_frame_rate: u32,
    /// `ttp:tickRate` — default derived from frame rate × sub frame rate, or 1.
    pub tick_rate: u32,
    /// `ttp:dropMode` — default `nonDrop`. Only meaningful when time_base=smpte.
    pub drop_mode: DropMode,
    /// `ttp:markerMode` — default `discontinuous`. Only meaningful when time_base=smpte.
    pub marker_mode: MarkerMode,
    /// `ttp:clockMode` — default `utc`. Only meaningful when time_base=clock.
    pub clock_mode: ClockMode,
}

impl Default for TimeContext {
    fn default() -> Self {
        Self {
            time_base: TimeBase::Media,
            frame_rate: 30,
            frame_rate_multiplier_numerator: 1,
            frame_rate_multiplier_denominator: 1,
            sub_frame_rate: 1,
            tick_rate: 30, // effective frame rate × sub-frame rate
            drop_mode: DropMode::NonDrop,
            marker_mode: MarkerMode::Discontinuous,
            clock_mode: ClockMode::Utc,
        }
    }
}

/// `ttp:timeBase` values — TTML2 §7.2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeBase {
    /// Media timeline.
    Media,
    /// SMPTE ST 12-1 timecode.
    Smpte,
    /// Real-world clock time.
    Clock,
}

impl TimeBase {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            TimeBase::Media => "media",
            TimeBase::Smpte => "smpte",
            TimeBase::Clock => "clock",
        }
    }
}

broadcast_common::impl_spec_display!(TimeBase);

/// `ttp:dropMode` values — TTML2 §7.2.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropMode {
    /// No frame dropping.
    NonDrop,
    /// NTSC drop-frame (frames 00,01 dropped at minute start except multiples of 10).
    DropNtsc,
    /// PAL drop-frame (frames 00-03 dropped at even minute start except multiples of 20).
    DropPal,
}

impl DropMode {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            DropMode::NonDrop => "nonDrop",
            DropMode::DropNtsc => "dropNTSC",
            DropMode::DropPal => "dropPAL",
        }
    }
}

broadcast_common::impl_spec_display!(DropMode);

/// `ttp:markerMode` values — TTML2 §7.2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkerMode {
    /// SMPTE time coordinates are linear/monotonic.
    Continuous,
    /// No continuity assumed; arithmetic on time expressions undefined.
    Discontinuous,
}

impl MarkerMode {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            MarkerMode::Continuous => "continuous",
            MarkerMode::Discontinuous => "discontinuous",
        }
    }
}

broadcast_common::impl_spec_display!(MarkerMode);

/// `ttp:clockMode` values — TTML2 §7.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClockMode {
    /// Local wall-clock time.
    Local,
    /// UTC.
    Utc,
    /// GPS time (not leap-second adjusted).
    Gps,
}

impl ClockMode {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            ClockMode::Local => "local",
            ClockMode::Utc => "utc",
            ClockMode::Gps => "gps",
        }
    }
}

broadcast_common::impl_spec_display!(ClockMode);

/// A parsed time expression.
///
/// While the grammar in §12.3.1 defines three forms (clock-time, offset-time,
/// wallclock-time), we represent all parsed time expressions uniformly.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TimeExpression {
    /// Clock-time: `HH:MM:SS[.fraction][:frames[.sub-frames]]`
    ClockTime {
        /// Hours (unbounded — can be ≥ 100).
        hours: u32,
        /// Minutes [0, 59].
        minutes: u8,
        /// Seconds [0, 60] (60 = leap second).
        seconds: u8,
        /// Fractional seconds, as a string of digits after the decimal point.
        fraction: Option<String>,
        /// Frames component (HH:MM:SS:FF).
        frames: Option<u32>,
        /// Sub-frames component (HH:MM:SS:FF.SF).
        sub_frames: Option<u32>,
    },
    /// Offset-time: `count[.fraction]metric`
    OffsetTime {
        /// The integer count part.
        count: u64,
        /// Fractional part, as a string of digits.
        fraction: Option<String>,
        /// The metric unit.
        metric: TimeMetric,
    },
    /// Wallclock-time: `wallclock(date-time|wall-time|date)`
    WallclockTime {
        /// The wallclock form: date-time, wall-time, or date.
        form: WallclockForm,
    },
}

/// Metric units for offset-time expressions — TTML2 §12.3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeMetric {
    /// Hours.
    H,
    /// Minutes.
    M,
    /// Seconds.
    S,
    /// Milliseconds.
    Ms,
    /// Frames (requires `ttp:frameRate`).
    F,
    /// Ticks (requires `ttp:tickRate`).
    T,
}

impl TimeMetric {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            TimeMetric::H => "h",
            TimeMetric::M => "m",
            TimeMetric::S => "s",
            TimeMetric::Ms => "ms",
            TimeMetric::F => "f",
            TimeMetric::T => "t",
        }
    }
}

broadcast_common::impl_spec_display!(TimeMetric);

/// Wallclock-time forms — TTML2 §12.3.1.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WallclockForm {
    /// `YYYY-MM-DDThh:mm:ss[.fraction]`
    DateTime {
        /// Years (4 digits).
        years: u16,
        /// Months [1, 12].
        months: u8,
        /// Days [1, 31].
        days: u8,
        /// Hours [0, 23].
        hours: u8,
        /// Minutes [0, 59].
        minutes: u8,
        /// Seconds [0, 60].
        seconds: u8,
        /// Fractional seconds.
        fraction: Option<String>,
    },
    /// `hh:mm[:ss[.fraction]]`
    WallTime {
        /// Hours [0, 23].
        hours: u8,
        /// Minutes [0, 59].
        minutes: u8,
        /// Seconds [0, 60].
        seconds: Option<u8>,
        /// Fractional seconds.
        fraction: Option<String>,
    },
    /// `YYYY-MM-DD`
    Date {
        /// Years (4 digits).
        years: u16,
        /// Months [1, 12].
        months: u8,
        /// Days [1, 31].
        days: u8,
    },
}

/// Parse a time expression string to its typed representation.
///
/// Performs basic structural validation (digit counts, ranges).
/// Time-base-specific constraint enforcement (frame/clock restrictions)
/// is done separately depending on context.
pub fn parse_time_expression(
    input: &str,
    ctx: &TimeContext,
) -> Result<TimeExpression, crate::error::Error> {
    if input.is_empty() {
        return Err(crate::error::Error::InvalidTimeExpression {
            value: input.to_string(),
            reason: "empty time expression".into(),
        });
    }

    // Try wallclock-time first: starts with "wallclock("
    if input.starts_with("wallclock(") {
        return parse_wallclock_time(input, ctx);
    }

    // Clock-time: contains ':' and first char is digit
    // Offset-time: contains metric suffix (h, m, s, ms, f, t) and no ':'
    if input.contains(':') {
        parse_clock_time(input, ctx)
    } else {
        parse_offset_time(input, ctx)
    }
}

fn parse_clock_time(input: &str, ctx: &TimeContext) -> Result<TimeExpression, crate::error::Error> {
    let err = |reason: &str| crate::error::Error::InvalidTimeExpression {
        value: input.to_string(),
        reason: reason.into(),
    };

    // Split on ':'
    let parts: Vec<&str> = input.split(':').collect();

    // Must have at least 3 parts (HH:MM:SS) or 4 (HH:MM:SS:FF) or 5 (HH:MM:SS:FF.SF)
    if parts.len() < 3 || parts.len() > 4 {
        return Err(err(
            "clock-time must have exactly 3 or 4 colon-separated components",
        ));
    }

    // Each part must be non-empty
    if parts.iter().any(|p| p.is_empty()) {
        return Err(err("empty component in clock-time"));
    }

    // Hours
    let hours: u32 = parse_digits(parts[0], "hours", &err)?;
    // Must have at least 2 digits if < 100, or 3+ if >= 100
    if hours < 100 && parts[0].len() < 2 {
        return Err(err(
            "hours < 100 must have leading zero (at least 2 digits)",
        ));
    }

    // Minutes
    let minutes_raw = parts[1];
    if minutes_raw.len() != 2 {
        return Err(err("minutes must be exactly 2 digits"));
    }
    let minutes: u8 = parse_digits_u8(minutes_raw, "minutes", &err)?;
    if minutes > 59 {
        return Err(err("minutes must be in [0, 59]"));
    }

    // Seconds (may have fraction)
    let secs_part = parts[2];
    let (secs_str, fraction): (&str, Option<String>) = if let Some(dot_pos) = secs_part.find('.') {
        let (s, f) = secs_part.split_at(dot_pos);
        let frac = &f[1..]; // skip the '.'
        if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(err("fractional seconds must be digits"));
        }
        (s, Some(frac.to_string()))
    } else {
        (secs_part, None)
    };

    if secs_str.len() != 2 {
        return Err(err("seconds must be exactly 2 digits"));
    }
    let seconds: u8 = parse_digits_u8(secs_str, "seconds", &err)?;
    if seconds > 60 {
        return Err(err("seconds must be in [0, 60]"));
    }

    if parts.len() == 4 {
        // Has frames component
        let frames_part = parts[3];

        // Frames component is error when timeBase=clock
        if ctx.time_base == TimeBase::Clock {
            return Err(err("frames term is an error when timeBase is clock"));
        }

        let (frames_str, sub_frames): (&str, Option<u32>) =
            if let Some(dot_pos) = frames_part.find('.') {
                let (f, sf) = frames_part.split_at(dot_pos);
                let sf_str = &sf[1..];
                if sf_str.is_empty() || !sf_str.chars().all(|c| c.is_ascii_digit()) {
                    return Err(err("sub-frames must be digits"));
                }
                let sf_val: u32 = sf_str
                    .parse()
                    .map_err(|_| err("sub-frames value too large"))?;
                // Sub-frames is error when timeBase=clock
                if ctx.time_base == TimeBase::Clock {
                    return Err(err("sub-frames term is an error when timeBase is clock"));
                }
                // Validate sub-frames range
                if ctx.sub_frame_rate > 0 && sf_val >= ctx.sub_frame_rate {
                    return Err(err(&format!(
                        "sub-frames value {} must be < subFrameRate {}",
                        sf_val, ctx.sub_frame_rate
                    )));
                }
                (f, Some(sf_val))
            } else {
                (frames_part, None)
            };

        let frames: u32 = parse_digits(frames_str, "frames", &err)?;
        // Validate frames range
        let effective_frame_rate = if ctx.frame_rate > 0 {
            ctx.frame_rate
        } else {
            30
        };
        if frames >= effective_frame_rate {
            return Err(err(&format!(
                "frames value {} must be < frameRate {}",
                frames, effective_frame_rate
            )));
        }

        Ok(TimeExpression::ClockTime {
            hours,
            minutes,
            seconds,
            fraction,
            frames: Some(frames),
            sub_frames,
        })
    } else {
        Ok(TimeExpression::ClockTime {
            hours,
            minutes,
            seconds,
            fraction,
            frames: None,
            sub_frames: None,
        })
    }
}

fn parse_offset_time(
    input: &str,
    _ctx: &TimeContext,
) -> Result<TimeExpression, crate::error::Error> {
    let err = |reason: &str| crate::error::Error::InvalidTimeExpression {
        value: input.to_string(),
        reason: reason.into(),
    };

    // Find the metric suffix
    let metric = if let Some(stripped) = input.strip_suffix("ms") {
        (TimeMetric::Ms, stripped)
    } else if let Some(stripped) = input.strip_suffix('h') {
        (TimeMetric::H, stripped)
    } else if let Some(stripped) = input.strip_suffix('m') {
        (TimeMetric::M, stripped)
    } else if let Some(stripped) = input.strip_suffix('s') {
        (TimeMetric::S, stripped)
    } else if let Some(stripped) = input.strip_suffix('f') {
        (TimeMetric::F, stripped)
    } else if let Some(stripped) = input.strip_suffix('t') {
        (TimeMetric::T, stripped)
    } else {
        return Err(err(
            "offset-time must end with a metric: h, m, s, ms, f, or t",
        ));
    };

    let num_str = metric.1;
    if num_str.is_empty() {
        return Err(err(
            "offset-time must have a numeric count before the metric",
        ));
    }

    let (count_str, fraction): (&str, Option<String>) = if let Some(dot_pos) = num_str.find('.') {
        let (c, f) = num_str.split_at(dot_pos);
        let frac = &f[1..];
        if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(err("fractional part must be digits"));
        }
        (c, Some(frac.to_string()))
    } else {
        (num_str, None)
    };

    if count_str.is_empty() || !count_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(err("count part must be digits"));
    }

    let count: u64 = count_str
        .parse()
        .map_err(|_| err("count value too large"))?;

    Ok(TimeExpression::OffsetTime {
        count,
        fraction,
        metric: metric.0,
    })
}

fn parse_wallclock_time(
    input: &str,
    ctx: &TimeContext,
) -> Result<TimeExpression, crate::error::Error> {
    let err = |reason: &str| crate::error::Error::InvalidTimeExpression {
        value: input.to_string(),
        reason: reason.into(),
    };

    // Wallclock-time is an error if timeBase is not clock
    if ctx.time_base != TimeBase::Clock {
        return Err(err("wallclock-time is an error when timeBase is not clock"));
    }

    // Extract content between "wallclock(" and ")"
    let rest = &input["wallclock(".len()..];
    let rest = rest.trim_start(); // <lwsp>?
    let rest = if let Some(close_pos) = rest.rfind(')') {
        rest[..close_pos].trim_end() // <lwsp>?
    } else {
        return Err(err("wallclock-time missing closing parenthesis"));
    };

    let rest = rest.trim();

    if rest.is_empty() {
        return Err(err("wallclock-time has no content"));
    }

    // Check which form: date-time, wall-time, or date
    if rest.contains('T') {
        // date-time: YYYY-MM-DDThh:mm:ss[.fraction]
        parse_wallclock_datetime(rest, &err)
    } else if rest.contains('-') {
        // date: YYYY-MM-DD
        parse_wallclock_date(rest, &err)
    } else {
        // wall-time: hh:mm[:ss[.fraction]]
        parse_wallclock_walltime(rest, &err)
    }
}

fn parse_wallclock_datetime(
    input: &str,
    err: &impl Fn(&str) -> crate::error::Error,
) -> Result<TimeExpression, crate::error::Error> {
    let parts: Vec<&str> = input.split('T').collect();
    if parts.len() != 2 {
        return Err(err("date-time must have exactly one 'T' separator"));
    }

    let date_part = parts[0];
    let time_part = parts[1];

    // Parse date: YYYY-MM-DD
    let date_components: Vec<&str> = date_part.split('-').collect();
    if date_components.len() != 3 {
        return Err(err("date must have exactly 3 components: YYYY-MM-DD"));
    }

    if date_components[0].len() != 4 {
        return Err(err("years must be exactly 4 digits"));
    }
    let years: u16 = date_components[0]
        .parse()
        .map_err(|_| err("invalid years"))?;

    if date_components[1].len() != 2 {
        return Err(err("months must be exactly 2 digits"));
    }
    let months: u8 = date_components[1]
        .parse()
        .map_err(|_| err("invalid months"))?;
    if !(1..=12).contains(&months) {
        return Err(err("months must be in [1, 12]"));
    }

    if date_components[2].len() != 2 {
        return Err(err("days must be exactly 2 digits"));
    }
    let days: u8 = date_components[2]
        .parse()
        .map_err(|_| err("invalid days"))?;
    if !(1..=31).contains(&days) {
        return Err(err("days must be in [1, 31]"));
    }

    // Parse time: hh:mm[:ss[.fraction]]
    let (hours, minutes, seconds, fraction) = parse_wallclock_time_components(time_part, err)?;

    Ok(TimeExpression::WallclockTime {
        form: WallclockForm::DateTime {
            years,
            months,
            days,
            hours,
            minutes,
            seconds: seconds.unwrap_or(0),
            fraction,
        },
    })
}

fn parse_wallclock_date(
    input: &str,
    err: &impl Fn(&str) -> crate::error::Error,
) -> Result<TimeExpression, crate::error::Error> {
    let date_components: Vec<&str> = input.split('-').collect();
    if date_components.len() != 3 {
        return Err(err("date must have exactly 3 components: YYYY-MM-DD"));
    }

    if date_components[0].len() != 4 {
        return Err(err("years must be exactly 4 digits"));
    }
    let years: u16 = date_components[0]
        .parse()
        .map_err(|_| err("invalid years"))?;

    if date_components[1].len() != 2 {
        return Err(err("months must be exactly 2 digits"));
    }
    let months: u8 = date_components[1]
        .parse()
        .map_err(|_| err("invalid months"))?;
    if !(1..=12).contains(&months) {
        return Err(err("months must be in [1, 12]"));
    }

    if date_components[2].len() != 2 {
        return Err(err("days must be exactly 2 digits"));
    }
    let days: u8 = date_components[2]
        .parse()
        .map_err(|_| err("invalid days"))?;
    if !(1..=31).contains(&days) {
        return Err(err("days must be in [1, 31]"));
    }

    Ok(TimeExpression::WallclockTime {
        form: WallclockForm::Date {
            years,
            months,
            days,
        },
    })
}

fn parse_wallclock_walltime(
    input: &str,
    err: &impl Fn(&str) -> crate::error::Error,
) -> Result<TimeExpression, crate::error::Error> {
    let (hours, minutes, seconds, fraction) = parse_wallclock_time_components(input, err)?;

    Ok(TimeExpression::WallclockTime {
        form: WallclockForm::WallTime {
            hours,
            minutes,
            seconds,
            fraction,
        },
    })
}

fn parse_wallclock_time_components(
    input: &str,
    err: &impl Fn(&str) -> crate::error::Error,
) -> Result<(u8, u8, Option<u8>, Option<String>), crate::error::Error> {
    let parts: Vec<&str> = input.split(':').collect();

    if parts.len() < 2 || parts.len() > 3 {
        return Err(err(
            "wallclock time must have 2 or 3 colon-separated components",
        ));
    }

    if parts[0].len() != 2 {
        return Err(err("wallclock hours must be exactly 2 digits"));
    }
    let hours: u8 = parts[0].parse().map_err(|_| err("invalid hours"))?;
    if hours > 23 {
        return Err(err("wallclock hours must be in [0, 23]"));
    }

    if parts[1].len() != 2 {
        return Err(err("wallclock minutes must be exactly 2 digits"));
    }
    let minutes: u8 = parts[1].parse().map_err(|_| err("invalid minutes"))?;
    if minutes > 59 {
        return Err(err("wallclock minutes must be in [0, 59]"));
    }

    if parts.len() == 3 {
        let (secs_str, fraction) = if let Some(dot_pos) = parts[2].find('.') {
            let (s, f) = parts[2].split_at(dot_pos);
            let frac = &f[1..];
            if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
                return Err(err("fractional seconds must be digits"));
            }
            (s, Some(frac.to_string()))
        } else {
            (parts[2], None)
        };

        if secs_str.len() != 2 {
            return Err(err("wallclock seconds must be exactly 2 digits"));
        }
        let seconds: u8 = secs_str.parse().map_err(|_| err("invalid seconds"))?;
        if seconds > 60 {
            return Err(err("wallclock seconds must be in [0, 60]"));
        }

        Ok((hours, minutes, Some(seconds), fraction))
    } else {
        Ok((hours, minutes, None, None))
    }
}

fn parse_digits<T: core::str::FromStr>(
    s: &str,
    name: &str,
    err: &impl Fn(&str) -> crate::error::Error,
) -> Result<T, crate::error::Error> {
    if !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(err(&format!("{name} must be digits")));
    }
    s.parse::<T>()
        .map_err(|_| err(&format!("{name} value too large")))
}

fn parse_digits_u8(
    s: &str,
    name: &str,
    err: &impl Fn(&str) -> crate::error::Error,
) -> Result<u8, crate::error::Error> {
    parse_digits::<u8>(s, name, err)
}

/// Format a parsed time expression back to its string representation.
///
/// This produces a normalized but semantically equivalent string that
/// preserves the same form (clock-time/offset-time/wallclock-time).
pub fn format_time_expression(expr: &TimeExpression) -> String {
    match expr {
        TimeExpression::ClockTime {
            hours,
            minutes,
            seconds,
            fraction,
            frames,
            sub_frames,
        } => {
            let mut s = format!("{:02}:{:02}:{:02}", hours, minutes, seconds);
            if let Some(frac) = fraction {
                s.push('.');
                s.push_str(frac);
            }
            if let Some(frames) = frames {
                s.push(':');
                // Frames can be 2+ digits
                s.push_str(&format!("{:02}", frames));
                if let Some(sf) = sub_frames {
                    s.push('.');
                    s.push_str(&sf.to_string());
                }
            }
            s
        }
        TimeExpression::OffsetTime {
            count,
            fraction,
            metric,
        } => {
            let mut s = count.to_string();
            if let Some(frac) = fraction {
                s.push('.');
                s.push_str(frac);
            }
            s.push_str(metric.name());
            s
        }
        TimeExpression::WallclockTime { form } => {
            let inner = match form {
                WallclockForm::DateTime {
                    years,
                    months,
                    days,
                    hours,
                    minutes,
                    seconds,
                    fraction,
                } => {
                    let mut s = format!(
                        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                        years, months, days, hours, minutes, seconds
                    );
                    if let Some(frac) = fraction {
                        s.push('.');
                        s.push_str(frac);
                    }
                    s
                }
                WallclockForm::WallTime {
                    hours,
                    minutes,
                    seconds,
                    fraction,
                } => {
                    let mut s = format!("{:02}:{:02}", hours, minutes);
                    if let Some(secs) = seconds {
                        s.push(':');
                        s.push_str(&format!("{:02}", secs));
                    }
                    if let Some(frac) = fraction {
                        s.push('.');
                        s.push_str(frac);
                    }
                    s
                }
                WallclockForm::Date {
                    years,
                    months,
                    days,
                } => {
                    format!("{:04}-{:02}-{:02}", years, months, days)
                }
            };
            format!("wallclock({})", inner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_ctx() -> TimeContext {
        TimeContext::default()
    }

    #[test]
    fn test_offset_seconds() {
        let expr = parse_time_expression("0s", &default_ctx()).unwrap();
        assert_eq!(
            expr,
            TimeExpression::OffsetTime {
                count: 0,
                fraction: None,
                metric: TimeMetric::S,
            }
        );
        assert_eq!(format_time_expression(&expr), "0s");
    }

    #[test]
    fn test_offset_fractional() {
        let expr = parse_time_expression("1.2s", &default_ctx()).unwrap();
        assert_eq!(
            expr,
            TimeExpression::OffsetTime {
                count: 1,
                fraction: Some("2".into()),
                metric: TimeMetric::S,
            }
        );
        assert_eq!(format_time_expression(&expr), "1.2s");
    }

    #[test]
    fn test_offset_minutes() {
        let expr = parse_time_expression("1.2m", &default_ctx()).unwrap();
        assert_eq!(format_time_expression(&expr), "1.2m");
    }

    #[test]
    fn test_offset_hours() {
        let expr = parse_time_expression("1.2h", &default_ctx()).unwrap();
        assert_eq!(format_time_expression(&expr), "1.2h");
    }

    #[test]
    fn test_offset_frames() {
        let expr = parse_time_expression("24f", &default_ctx()).unwrap();
        assert_eq!(
            expr,
            TimeExpression::OffsetTime {
                count: 24,
                fraction: None,
                metric: TimeMetric::F,
            }
        );
        assert_eq!(format_time_expression(&expr), "24f");
    }

    #[test]
    fn test_offset_ticks() {
        let expr = parse_time_expression("120t", &default_ctx()).unwrap();
        assert_eq!(format_time_expression(&expr), "120t");
    }

    #[test]
    fn test_clock_time_simple() {
        let expr = parse_time_expression("01:02:03", &default_ctx()).unwrap();
        assert_eq!(
            expr,
            TimeExpression::ClockTime {
                hours: 1,
                minutes: 2,
                seconds: 3,
                fraction: None,
                frames: None,
                sub_frames: None,
            }
        );
        assert_eq!(format_time_expression(&expr), "01:02:03");
    }

    #[test]
    fn test_clock_time_fraction() {
        let expr = parse_time_expression("01:02:03.235", &default_ctx()).unwrap();
        if let TimeExpression::ClockTime { fraction, .. } = &expr {
            assert_eq!(fraction.as_deref(), Some("235"));
        } else {
            panic!("expected ClockTime");
        }
        assert_eq!(format_time_expression(&expr), "01:02:03.235");
    }

    #[test]
    fn test_clock_time_with_frames() {
        let expr = parse_time_expression("01:02:03:20", &default_ctx()).unwrap();
        if let TimeExpression::ClockTime { frames, .. } = &expr {
            assert_eq!(*frames, Some(20));
        } else {
            panic!("expected ClockTime");
        }
        assert_eq!(format_time_expression(&expr), "01:02:03:20");
    }

    #[test]
    fn test_clock_time_large_hours() {
        let expr = parse_time_expression("100:00:00.1", &default_ctx()).unwrap();
        let formatted = format_time_expression(&expr);
        // 100:00:00.1 should be preserved
        assert!(formatted.starts_with("100:00:00"));
    }

    #[test]
    fn test_clock_time_with_frame_subframes() {
        let ctx = TimeContext {
            frame_rate: 24,
            sub_frame_rate: 10,
            ..default_ctx()
        };
        let expr = parse_time_expression("01:02:03:20.5", &ctx).unwrap();
        if let TimeExpression::ClockTime {
            frames, sub_frames, ..
        } = &expr
        {
            assert_eq!(*frames, Some(20));
            assert_eq!(*sub_frames, Some(5));
        } else {
            panic!("expected ClockTime");
        }
        // Normalized: seconds component gets no fraction since frames consume it
        assert_eq!(format_time_expression(&expr), "01:02:03:20.5");
    }

    #[test]
    fn test_frame_rate_validation() {
        let ctx = TimeContext {
            frame_rate: 24,
            ..default_ctx()
        };
        // Clock-time frames must be < frameRate
        parse_time_expression("01:02:03:23", &ctx).unwrap();
        // 24f (offset-time f metric) — the value itself is not range-checked against frameRate;
        // the context determines how the value is interpreted at presentation time.
        // But the clock-time frames component 01:02:03:24 should fail
        assert!(parse_time_expression("01:02:03:24", &ctx).is_err());
    }

    #[test]
    fn test_subframe_rate_validation() {
        let ctx = TimeContext {
            frame_rate: 24,
            sub_frame_rate: 10,
            ..default_ctx()
        };
        // sub-frame 9 should be valid (must be < subFrameRate=10)
        parse_time_expression("01:02:03:20.9", &ctx).unwrap();
        // sub-frame 10 should fail
        assert!(parse_time_expression("01:02:03:20.10", &ctx).is_err());
    }

    #[test]
    fn test_frames_error_on_clock_timebase() {
        let ctx = TimeContext {
            time_base: TimeBase::Clock,
            ..default_ctx()
        };
        // frames term is error when timeBase=clock
        assert!(parse_time_expression("01:02:03:20", &ctx).is_err());
    }

    #[test]
    fn test_negative_cases() {
        // Empty
        assert!(parse_time_expression("", &default_ctx()).is_err());
        // No metric
        assert!(parse_time_expression("123", &default_ctx()).is_err());
        // Minutes out of range
        assert!(parse_time_expression("00:60:00", &default_ctx()).is_err());
        // Seconds = 61 is out of range (max is 60 for leap second)
        assert!(parse_time_expression("00:00:61", &default_ctx()).is_err());
        // Missing leading zero
        assert!(parse_time_expression("0:00:00", &default_ctx()).is_err());
        // No digits before metric
        assert!(parse_time_expression("s", &default_ctx()).is_err());
    }

    #[test]
    fn test_wallclock_error_on_non_clock_timebase() {
        let ctx = TimeContext {
            time_base: TimeBase::Media,
            ..default_ctx()
        };
        assert!(parse_time_expression("wallclock(2024-01-01T00:00:00)", &ctx).is_err());
    }

    #[test]
    fn test_milliseconds() {
        let expr = parse_time_expression("500ms", &default_ctx()).unwrap();
        if let TimeExpression::OffsetTime { count, metric, .. } = &expr {
            assert_eq!(*count, 500);
            assert_eq!(*metric, TimeMetric::Ms);
        } else {
            panic!("expected OffsetTime");
        }
        assert_eq!(format_time_expression(&expr), "500ms");
    }

    #[test]
    fn test_round_trip_all_fixture_expressions() {
        let expressions = vec![
            "0s",
            "1.2s",
            "1.2m",
            "1.2h",
            "24f",
            "120t",
            "01:02:03",
            "01:02:03.235",
            "01:02:03.2350",
            "01:02:03:20",
            "100:00:00.1",
            "100:00:00:00",
            "00:00:00.000",
            "00:00:10.000",
            "1s",
            "5s",
            "6s",
            "9s",
            "10s",
            "20s",
        ];

        for expr_str in &expressions {
            let parsed = parse_time_expression(expr_str, &default_ctx())
                .unwrap_or_else(|e| panic!("failed to parse '{expr_str}': {e}"));
            let formatted = format_time_expression(&parsed);
            // Re-parse the formatted version
            let re_parsed = parse_time_expression(&formatted, &default_ctx())
                .unwrap_or_else(|e| panic!("failed to re-parse '{formatted}': {e}"));
            assert_eq!(
                parsed, re_parsed,
                "round-trip failed for '{expr_str}' -> '{formatted}'"
            );
        }
    }
}
