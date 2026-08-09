//! Internal `std`-gated shims around the `metrics` facade macros.
//!
//! `metrics::counter!`/`metrics::gauge!` expand to code that is not
//! no_std-compatible (their dynamic-label path reaches for `std`), so every
//! metric-recording call site in this crate goes through one of the three
//! macros below instead of calling the facade directly. Under this crate's
//! `std` feature they forward to the real facade; under `--no-default-features`
//! they evaluate every argument (so a caller's local bindings are never
//! reported "unused" purely because metrics recording was compiled out) and
//! then discard the result — every check in this crate still runs, it is
//! simply not reported. This is the one place that distinction lives, so a
//! reader auditing "does this crate genuinely no_std-build" only has to read
//! this file, not every call site.

#[cfg(feature = "std")]
macro_rules! record_counter {
    ($name:expr) => {
        ::metrics::counter!($name).increment(1)
    };
    ($name:expr, $($k:expr => $v:expr),+ $(,)?) => {
        ::metrics::counter!($name, $($k => $v),+).increment(1)
    };
}
#[cfg(not(feature = "std"))]
macro_rules! record_counter {
    ($name:expr) => {
        let _ = $name;
    };
    ($name:expr, $($k:expr => $v:expr),+ $(,)?) => {
        let _ = ($name, $(($k, $v)),+);
    };
}
pub(crate) use record_counter;

/// Like [`record_counter`], but incrementing by a caller-supplied count
/// rather than 1 (loss-report counters: `skipped` may be > 1).
///
/// Only defined under `std`: its only callers live in
/// [`crate::trunk_bridge`], which is itself `std`-only (see that module's
/// docs) — unlike [`record_counter`]/[`record_gauge`], there is no `!std`
/// call site for this one to be a no-op shim for.
#[cfg(feature = "std")]
macro_rules! record_counter_by {
    ($name:expr, $n:expr) => {
        ::metrics::counter!($name).increment($n)
    };
}
#[cfg(feature = "std")]
pub(crate) use record_counter_by;

#[cfg(feature = "std")]
macro_rules! record_gauge {
    ($name:expr, $val:expr) => {
        ::metrics::gauge!($name).set($val)
    };
    ($name:expr, $val:expr, $($k:expr => $v:expr),+ $(,)?) => {
        ::metrics::gauge!($name, $($k => $v),+).set($val)
    };
}
#[cfg(not(feature = "std"))]
macro_rules! record_gauge {
    ($name:expr, $val:expr) => {
        let _ = ($name, $val);
    };
    ($name:expr, $val:expr, $($k:expr => $v:expr),+ $(,)?) => {
        let _ = ($name, $val, $(($k, $v)),+);
    };
}
pub(crate) use record_gauge;

#[cfg(test)]
mod tests {
    /// Both feature configurations must accept the exact call shapes every
    /// call site in this crate uses — a bite against the arm patterns
    /// drifting apart between the `std`/`no_std` halves of each macro.
    #[test]
    fn every_call_shape_compiles() {
        const NAME: &str = "x";
        let pid = alloc::string::String::from("0x0100");
        record_counter!(NAME);
        record_counter!(NAME, "k" => "v");
        record_counter!(NAME, "k" => "v", "k2" => pid.clone());
        record_gauge!(NAME, 1.0f64);
        record_gauge!(NAME, 1.0f64, "pid" => pid);
    }

    /// `record_counter_by!` is `std`-only (see its own doc) — its call-shape
    /// bite lives in a separate, `std`-gated test rather than the shared one
    /// above, which must also compile under `--no-default-features`.
    #[cfg(feature = "std")]
    #[test]
    fn record_counter_by_call_shape_compiles() {
        const NAME: &str = "x";
        record_counter_by!(NAME, 3u64);
    }
}
