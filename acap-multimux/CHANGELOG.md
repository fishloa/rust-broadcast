# Changelog

All notable changes to `acap-multimux` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- **The config store has never worked on any camera** (issue #955, blocks
  #954's H.265 hardware verification). `AxParameterStore::store` called
  `axparameter::Parameter::set("Config", …)` on a parameter that was never
  `add`ed — confirmed on an ARTPEC-8 camera:
  `param.cgi?action=list&group=acap-multimux` returned `Error -1 getting
  param in group`. `AxParameterStore::new` now calls `Parameter::add` (with
  `Config::default()` as the initial value) if the parameter doesn't exist
  yet, matching the vendored `axparameter_example` app's own
  add-then-ignore-`ParamAdded` idiom so a second start (every restart, since
  `manifest.json` sets `runMode: "respawn"`) doesn't fail just because the
  parameter now exists.
- **A broken config backend was indistinguishable from an unconfigured
  one.** `ConfigStore::load` used to discard the backend's error and return
  `Config::default()` either way, which is why the parameter-store bug above
  went unnoticed for a month: the app *looked* like it was running fine.
  `load` now returns a `LoadOutcome` (`Stored`/`Unset`/`Broken(reason)`)
  instead of a bare `Config`; a `Broken` outcome is surfaced through
  `/admin/status`'s `last_error` (via a new `StatusHandle::set_config_error`
  slot, kept separate from the capture pipeline's own `last_error` so a
  pipeline retry can't silently erase it).
- **`/admin/status` reported `current_segment`/`current_part`/`frames` as
  permanent zeros while media was flowing** — measured on the same camera:
  the LL-HLS playlist's `#EXT-X-MEDIA-SEQUENCE` climbed from 2 to 5 over 12
  seconds while `/admin/status` stood still at `0`/`0`/`0`. `StatusHandle`
  was never updated by the capture pipeline. The VDO capture loop
  (`run_vdo_capture` in the `acap-multimux` binary) now increments the frame
  counter once per `feed()` call and updates the segment/part position from
  the program's `Trunk` (`last_closed_segment` + `parts_in_segment`) on every
  iteration.

### Changed

- `ConfigStore::load` returns `admin::LoadOutcome` instead of `Config`
  (breaking change to this crate's internal, `publish = false` trait — no
  crates.io consumer is affected).
