## Table 65 — dvbDateTimeCodec
_§9.4.3.5.2, PDF pp. 69-69_

| Name | No. of bits | Identifier |
|---|---|---|
| dvbDateTimeCodec() { | | |
| dateTime Flag | 2 | bslbf |
| if (dateTime flag==00) { | | |
| dateTimeOfTVA | 64 | bslbf |
| } | | |
| if (dateTime flag==01) { | | |
| PublishedTime() | | |
| } | | |
| } | | |

