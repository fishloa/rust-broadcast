# `fixtures/sub/`

WebVTT/SRT fixtures for `caption-convert` (and any other consumer of plain
WebVTT/SRT text).

## `header_meta.vtt`

Hand-written (synthetic, no third-party content), following W3C WebVTT
(<https://www.w3.org/TR/webvtt1/>) SS4.1's own canonical example of a
`WEBVTT` header carrying free-form metadata text lines (`Kind:`/`Language:`)
before the first blank line — used by real WebVTT producers (e.g. YouTube's
caption exporter) as an informal MIME-header-style convention, distinct from
the `X-TIMESTAMP-MAP` header this crate already handled (RFC 8216 SS3.5,
issue #974).

Exercises audit finding CV-W1 / issue #1109 (reopens #974): `parse_webvtt`
used to fail the *whole document* when the header contained anything other
than `X-TIMESTAMP-MAP`, because the header text fell through into the
cue-block grouper and was misread as a cue identifier with no timing line
after it.

Independent oracle: ffmpeg 8.1.2 accepts this file as valid WebVTT and
converts it to the same two cues:

```sh
ffprobe -v error -show_entries stream=codec_name \
  -of default=noprint_wrappers=1 header_meta.vtt
# codec_name=webvtt

ffmpeg -v error -i header_meta.vtt -f srt -
# 1
# 00:00:01,000 --> 00:00:03,000
# Hello header test
#
# 2
# 00:00:04,000 --> 00:00:06,000
# Second cue
```

## `cap.vtt`, `sintel-en.srt`

Pre-existing fixtures — see `caption-convert/tests/webvtt_srt_fixture.rs`
and `caption-convert/tests/srt_real_fixture.rs` for their usage.
