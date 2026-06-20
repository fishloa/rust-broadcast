# Date-Time objects (date_time_enq / date_time)

_Source: EN 50221 §8.5.2, Tables 31-32 (PDF p. 35), render-verified_

The Date-Time resource (resource_identifier `00240041`) lets an application obtain the
current date and time. The application sends a Date-Time Enquiry; the host replies
with a Date-Time object — once if `response_interval` is zero, or periodically every
`response_interval` seconds otherwise. In a DVB-compliant host the time is derived
from the Time and Date Table (TDT/TOT).

## Table 31 — Date-Time Enquiry object coding

apdu_tag `Tdate_time_enq` = `9F 84 40`, Direction app `<---` host (sent by app to host).

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `date_time_enq () {` | | |
| &nbsp;&nbsp;date_time_enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;response_interval | 8 | uimsbf |
| `}` | | |

`response_interval` — if zero, the response is a single date_time object immediately.
If non-zero, a date_time object is sent immediately, then further date_time objects
every `response_interval` seconds.

## Table 32 — Date-Time object coding

apdu_tag `Tdate_time` = `9F 84 41`, Direction host `--->` app.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `date_time () {` | | |
| &nbsp;&nbsp;date_time_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 5 or 7 | | |
| &nbsp;&nbsp;UTC_time | 40 | bslbf |
| &nbsp;&nbsp;local_offset&nbsp;&nbsp;/* optional */ | 16 | tcimsbf |
| `}` | | |

Field notes:
- `UTC_time` — UTC date and time to the nearest second, coded as Modified Julian Day
  plus hours/minutes/seconds (BCD) as described in reference [4] (ETSI EN 300 468,
  the same MJD+BCD encoding as TDT/TOT). 40 bits = 5 bytes.
- `local_offset` — optional, 16-bit two's-complement (`tcimsbf`). If present it codes
  the current offset between UTC and local time as a signed number of minutes:
  `Local Time = UTC_time + local_offset`. The host provides it only when it has
  reliable knowledge; otherwise it omits the field (length_field = 5). When present,
  length_field = 7.

> NOTE: The PDF labels both Table 31 and Table 32 as "Date-Time Enquiry object
> coding" (typo in the spec); Table 32's syntax is clearly the `date_time()` object.
