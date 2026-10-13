# Tables 9-33 to 9-36 (§9.8.11 - §9.8.13)

_Source: ANSI/SCTE 104 2023, PDF pp.66-69, transcribed directly from the PDF text
layer (the older `operations.md` extraction is truncated after the §9.8.11
prose, so these tables were missing; #1103 W5)._

## Table 9-33: insert_audio_descriptor (§9.8.11)

| Syntax | Bytes | Type |
|--------|-------|------|
| `insert_audio_descriptor() {` | | |
| &nbsp;&nbsp;audio_count | 1 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<audio_count; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;component_tag | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ISO_code | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Bit_Stream_Mode | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Num_Channels | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Full_Srvc_Audio | 1 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Every field: "Per SCTE 35 Table 28, audio_descriptor()".

## Table 9-34: insert_audio_provisioning (§9.8.12)

| Syntax | Bytes | Type |
|--------|-------|------|
| `insert_audio_provisioning() {` | | |
| &nbsp;&nbsp;audio_count | 1 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<audio_count; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;channel_mode | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Codec_index | 1 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

`audio_count` - the number of audio services under the control of this
Automation/Compression system. `channel_mode` - per SCTE 242-1 Table 1.
`codec_index` - Table 9-35.

## Table 9-35: values for codec_index

| Codec_index | Value assigned |
|-------------|----------------|
| reserved | 0 |
| AC-3 | 1 |
| E-AC-3 | 2 |
| AAC | 3 |
| reserved | 4 |
| AC-4 | 5 |
| MPEG-H | 6 |

## Table 9-36: insert_alternate_break_duration (§9.8.13)

| Syntax | Bytes | Type |
|--------|-------|------|
| `insert_alternate_break_duration_data() {` | | |
| &nbsp;&nbsp;alternate_break_duration_data | 4 | uimsbf |
| `}` | | |

`alternate_break_duration` - a 32-bit duration of the insertion in
**milliseconds**; zero means the resulting SCTE 35 `splice_info()` uses the
`break_duration()`. Ignored for `splice_insert_type` values other than
`spliceStart_normal` and `spliceStart_immediate`.
