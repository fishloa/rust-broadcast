# caption-convert 0.2.0

_Released 2026-10-05._

Minor (0.x breaking-class) release. Its own code change is one bug fix in `parse_webvtt`; the version moves from 0.1.1 to 0.2.0 because the crate now builds against a new caret epoch of its siblings (`timed-metadata` 0.5 -> 0.6, `cc-data` 0.5 -> 0.6). Per the workspace's epoch-purity rule that is a major-class change for a 0.x crate, so a consumer that also depends on those siblings directly must move them in lockstep. The public API of `caption-convert` itself is unchanged.

## Fix

- **`parse_webvtt` no longer fails the whole document on extra WebVTT header lines (#1109, reopens #974).** The header block is every line directly after the `WEBVTT` signature up to the first blank line (W3C WebVTT section 4.1). Only `X-TIMESTAMP-MAP` was handled; any other line there, such as the `Kind:` or `Language:` hint some encoders emit, fell into the cue-block grouper and was misread as a cue identifier with no timing line, so the entire parse returned an error. The rest of the header block is now skipped and the result is flagged `lossy` (`ParsedWebVtt::lossy`), the same way other constructs the crate's `Cue` model cannot represent are reported. Behaviour change to be aware of: input that used to return `Err` now returns `Ok` with `lossy == true`, and the extra header text is not preserved in the output.

## Dependency changes (verified against the Cargo.toml diff)

```toml
# before (0.1.1)
broadcast-common = { ..., version = "9.3", default-features = false }
timed-metadata   = { ..., version = "0.5", default-features = false }
cc-data          = { ..., version = "0.5", default-features = false, optional = true }
# after (0.2.0)
broadcast-common = { ..., version = "9.4", default-features = false }
timed-metadata   = { ..., version = "0.6", default-features = false }
cc-data          = { ..., version = "0.6", default-features = false, optional = true }
```

`dvb-vbi` stays on `0.4` (satisfied by 0.4.1). Read these together with the sibling notes, because they change what the converters see:

- `cc-data-0.6.0.md`: CEA-708 captions spanning several `cc_data()` units are no longer destroyed, CEA-608 field-2 control codes and channel routing are fixed, and `CcData::parse` rejects non-conforming reserved bits with `Error::InvalidFixedBits`. CEA-608/708 to WebVTT/SRT output through the `cc-data` feature will therefore differ (more complete) from 0.1.1 on real streams.
- `timed-metadata-0.6.0.md`: the Teletext extractor now bit-reverses `txt_data_block` through `dvb-vbi`'s `txt_data_block_logical()` (see `dvb-vbi-0.4.1.md`), which affects the `teletext` feature.
- `broadcast-common-9.4.0.md`.

The crate's own source otherwise changed only in tests (the Teletext test builder now writes `txt_data_block` in wire bit order, and the CEA test fixture sets the new `reserved_byte1: 0xFF`).

---

Published from tag `caption-convert-v0.2.0`.
