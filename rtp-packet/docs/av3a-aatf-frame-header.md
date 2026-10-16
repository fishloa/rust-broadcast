# AATF frame header — T/AI 109.3-2023 Annex A

Source: `private/specs/t_ai_109_3_2023_avs3_immersive_audio_en.pdf`. Verified on
the rendered pages (PDF pp.127-133); the markdown extract is mangled around the
conditional rows of Tables A.4/A.6.

## Table A.4 `aatf_frame()` (verified on rendered page, PDF p.127-128)

```
syncword 12 | audio_codec_id 4 | anc_data_index 1
if audio_codec_id < 3 {
  aatf_frame_header()
  if audio_codec_id == 2: frame_error_check()
  byte_alignment()
  raw data block (ga_raw / ll_raw / hoa / ga_co by id and coding_profile)
}
if audio_codec_id == 0 || 1: frame_error_check()
```

## Table A.6 `aatf_frame_header()` (verified on rendered pages, PDF pp.128-129)

| Field | Bits | Condition |
|---|---|---|
| nn_type | 3 | id == 2 |
| coding_profile | 3 | |
| sampling_frequency_index | 4 | |
| sampling_frequency | 24 | id == 1 and index == 0xf |
| raw_frame_length | 16 | id != 2 |
| aatf_error_check() = crc_check | 8 (`rpchof`) | |
| channel_number_index | 7 | id == 0 |
| channel_number | {4; 8} | id == 1 |
| (id 2: channel_number_index / soundBedType / object_channel_number / bitrate_index[_per_channel] / order) | | per coding_profile |
| resolution | 2 | |
| bitrate_index | 4 | id == 0 or (id == 2 and coding_profile != 1) |
| bitstream_type | 1 | id == 0 |

## Tables A.5 / A.7

`frame_error_check()` and `aatf_error_check()` are each one 8-bit `crc_check`,
mnemonic `rpchof`.

> **Spec note:** T/AI 109.3 Tables A.5/A.7 name `crc_check` as `rpchof` but state
> no polynomial or initial value; the CRC is carried, not verified.

## `channel_number` semantics (verified on rendered page, PDF p.131)

> **Spec note:** "When audio_codec_id is 1 and the number of channels is less
> than 16, this field contains 4 bits. 0xf indicates extension, and others
> indicate the number of channels. When audio_codec_id is 1 and the number of
> channels is not less than 16, this field contains 8 bits."
> The text does not state whether the 8-bit form replaces the nibble or follows
> a `0xf` nibble, what the 8-bit value encodes, or how the width is known before
> the field is read; `0xf` (15) is itself "less than 16". Not implemented: the
> bytes after `aatf_error_check()` of a lossless frame stay in `AatfFrame::body`.
