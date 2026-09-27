# Changelog

> **Rename lineage:** this crate was published as `dvb-cc` through 0.2.0.
> Starting with 0.2.0 the crate directory was renamed to `cc-data`; a `dvb-cc`
> 0.2.1 shim re-exports everything from `cc-data = "0.2"`.

All notable changes to `cc-data` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **#1042**: `Cea708Decoder::push_triplets` unconditionally decoded and
  cleared the partial Caption Channel Packet buffer at the end of every
  call, destroying any CCP spanning more than one `cc_data()` access unit
  (CEA-708 §4/§5) — the common case, since a CCP is typically 32-128 bytes
  and one frame carries far fewer DTVCC bytes. The buffer is now kept
  across calls and only decoded once it is complete or a new packet starts.
- **#1043**: CEA-608 field-2 Miscellaneous Control Code pairs (first byte
  `0x15`/CC3, `0x1D`/CC4 — CTA-608-E §8.4 a/b) were never recognised; the
  fold only matched field-1's `0x14`/`0x1C`. RCL/EOC/EDM/RU2-4/CR and the
  rest of the misc-control set are now recognised on field 2.
- **#1107**: CEA-608 standard (untagged) characters always routed to a
  hardcoded CC1/CC3, instead of the channel targeted by the field's last
  control code — a CC2 caption's text was written into CC1's memory and
  interleaved with it. Now tracked and routed per field.
- **#1107**: a caption control code sent on field 2 while an XDS sub-packet
  was open was silently swallowed by the XDS-active check, which ran
  before the control-code check; CTA-608 allows a caption code to interrupt
  XDS. The control-code check now runs first.
- **#1107**: `Cea708Decoder`'s `Window::text()` kept interior blank rows as
  `"\n\n"`, which truncated the downstream WebVTT cue at the blank line.
  Blank rows (interior or trailing) are now dropped instead of emitted.
- **#1107**: `SetPenColor`'s edge colour (§8.10.5.10 parm3) was written
  into `Window::border_color`, clobbering the window border on every
  `SetPenColor`. It now has its own `Window::edge_color` field.
- **#1107**: `CcData::parse` didn't validate Table B.9's fixed bits
  (`reserved`/`zero_bit`/`one_bit`/`marker_bits`), and the reserved byte
  after `cc_count` (`em_data` in the shared ATSC A/53 lineage) was
  discarded and always re-serialized as `0xFF`, so a real stream whose
  value there wasn't `0xFF` didn't round-trip byte-exact. The fixed bits
  are now validated on parse (`Error::InvalidFixedBits`), and the reserved
  byte is preserved verbatim as `CcData::reserved_byte1`.

### Changed (breaking)
- **#1107**: `CcData` gained a new field, `reserved_byte1: u8` (see above).
  `Window` (cc-data's CEA-708 decode types) gained a new field,
  `edge_color: Color` (see above). Both are additive struct-literal breaks.

## [0.5.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

## [0.4.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

## [0.3.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.2.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.2.0] — 2026-06-21

### Changed (decode module — audit fixes, part of 0.2.0)
- `Cea608StyledChar::color` is now `Cea608Color` (new typed enum, CTA-608-E
  Tables 51/53) instead of `&'static str`. Variants: `White`, `Green`, `Blue`,
  `Cyan`, `Red`, `Yellow`, `Magenta`. Has `name()` + `impl_spec_display!`.
- `Window::border_type` is now `EdgeType` (reusing the existing screen enum;
  values 0–5 map exactly) instead of `u8`. `apply_window_style` initialises it
  to `EdgeType::None`; `set_window_attributes` decodes via `EdgeType::from_bits`.
- `Window::anchor_point` is now `AnchorPoint` (new typed enum, CTA-708-E §8.4.6,
  values 0–8) instead of `u8`. Variants: `TopLeft`/`TopCenter`/`TopRight`,
  `MiddleLeft`/`MiddleCenter`/`MiddleRight`, `BottomLeft`/`BottomCenter`/
  `BottomRight`. Has `name()` + `impl_spec_display!`.
- Fixed `Cea608Decoder` struct doctest: was passing `field2=true` (routes to CC3)
  while the comment claimed CC1; corrected to `field2=false` and added an
  `assert_eq!` assertion so the doctest now verifies decoded text.

### Added (decode module — audit fixes, part of 0.2.0)
- `tests/label_coverage.rs` drift-guard (issue #204 convention): scans `src/`
  for all `pub enum`s and fails if any lack a `Display` impl. Covers all 14
  public enums in cc-data; SKIP list documents `Error` and `WindowMapOp`.

### Added
- **Caption decode layer** (`decode` feature, default-on; additive → 0.2.0) — the
  CEA-608/708 character/control decode that interprets the demuxed caption byte
  pairs into displayed text. `no_std` + `alloc`, grounded in ANSI/CTA-608-E,
  ANSI/CTA-708-E and 47 CFR §79.102 (`cc-data/docs/decode/`).
  - `Cea608Decoder` — line-21 state machine: pop-on (RCL/EOC), roll-up
    (RU2/RU3/RU4 + CR), paint-on (RDC); Preamble Address Codes (row + indent +
    colour/italics/underline); mid-row codes; tab offsets; the standard / special
    / extended Western-European character sets (automatic backspace on extended
    chars); the four data channels CC1–CC4; control-code doubling; field-2 XDS
    detect-and-skip. Exposes a `Cea608Screen` (rows × `Cea608StyledChar` cells)
    and per-channel displayed text.
  - `Cea708Decoder` — DTVCC pipeline: Caption Channel Packet reassembly → Service
    Block parsing (incl. extended-service escape) → the C0/C1/G0/G1/G2/G3 command
    interpreter (DefineWindow DF0–7, SetWindowAttributes, SetCurrentWindow,
    Clear/Display/Hide/Toggle/Delete windows, SetPenAttributes/Color/Location,
    DLY/DLC/RST). Tracks the six standard services; exposes each service's window
    text (`Window` / `service_text`).
  - Typed display model: `Color` (2-bit RGB + 8-colour mapping), `Opacity`,
    `EdgeType`, `PenSize`, `PenOffset`, `FontStyle`, `Justify`, `PrintDirection`,
    `ScrollDirection`, `WindowState`, `Cea608Mode`, `Cea608Channel` — all with the
    project `name()` + `Display` label convention.
  - Both decode entry points are panic-free on arbitrary / truncated / malformed
    byte streams.
  - Two runnable examples (`decode_cea608`, `decode_cea708`).

## 0.1.0 — 2026-06-20

### Added
- Initial release. DVB closed-caption carriage `cc_data()` per ETSI TS 101 154
  §B.5, Table B.9:
  - `CcData` — `process_cc_data_flag` + the caption triplet loop, with byte-exact
    symmetric `Parse`/`Serialize` (computed `cc_count`, reserved/marker bits, 5-bit
    `cc_count` overflow guard).
  - `CcTriplet` / `CcType` — `cc_valid`, `cc_type` (CEA-608 field 1/2, CEA-708
    DTVCC data/start), `cc_data_1/2`.
  - `cea608()` / `cea708()` triplet split by `cc_type`.
- Two runnable examples (`parse_cc_data`, `build_cc_data`); `no_std` + `alloc`.
