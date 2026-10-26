<a id="xs-datetime"></a>

# xs:dateTime - lexical forms and the subset the DASH client accepts

Sources:

- W3C XML Schema Part 2: Datatypes (Second Edition), W3C Recommendation 28 October 2004, section 3.2.7 `dateTime`
  (3.2.7.1 lexical representation, 3.2.7.2 canonical representation, 3.2.7.3 timezones), <https://www.w3.org/TR/xmlschema-2/#dateTime>.
  Fetched 2026-10-26 and read.
- ISO/IEC 23009-1:2022 clause 5.8.5.7, the `DIRECT-VALUE` rule, read in the private verbatim copy
  (`private/specs-verified/dash/23009-1/05-8.md`) and recorded in [`23009-1/05-8-5-schemes.md`](23009-1/05-8-5-schemes.md).

Licence: W3C Recommendations are published under the W3C Document License (<https://www.w3.org/copyright/document-license/>), which
allows copying with attribution. This file is our own-words notes with short quotations marked as such. Everything under "The accepted subset" is
**our policy**, not the specification.

## Where the DASH client reads one

`MPD@availabilityStartTime`, `@availabilityEndTime`, `@publishTime`, `@nextLeapChangeTime`-style attributes (Table 3 and the leap-second
clause), `Patch@publishTime`/`@originalPublishTime`, the body of a `urn:mpeg:dash:utc:http-xsdate:2014` response, and the `@value` of
`urn:mpeg:dash:utc:direct:2014` (clause 5.8.5.7). A `http-iso:2014` body is "an ISO 8601 time code"; we read it with the same grammar.

## What the datatype allows (XSD 1.0 section 3.2.7.1, own words)

Lexical form: `'-'? yyyy '-' mm '-' dd 'T' hh ':' mm ':' ss ('.' s+)? (zzzzzz)?`

- `'-'? yyyy`: four or more digits, optional leading minus; with more than four digits no leading zeros; `0000` prohibited in 1.0
  ("there is no year 0"); a plus sign is not permitted. (XSD 1.1 permits `0000`; ISO 23009-1 references XML Schema part 2 without edition.)
- `mm`, `dd`, `hh`, `mm`, `ss` are each two digits; every field is bounded by the next-higher one (day 29 does not exist in February 2002);
  `ss` is whole seconds below 60 (there is no `:60` form).
- `hh` = `24` is permitted only when minutes, seconds and fraction are zero, and means the first instant of the next day.
- `'.' s+`: a fraction of one or more digits, of unbounded length.
- timezone `zzzzzz`: `Z` or `('+'|'-') hh ':' mm`; hour at most 14, minutes at most 59, and if the hour is 14 the minutes must be 00.
  `Z`, `+00:00` and `-00:00` are the same instant. Quoted rule: "Timezones are durations with (integer-valued) hour and minute properties
  (with the hour magnitude limited to at most 14...)".
- A value with a timezone denotes a UTC instant after conversion; a value without one is an "untimezoned" local time and is
  deliberately **not** comparable to a timezoned one in the value space.
- Canonical form: no `24:00:00`, no trailing zero in the fraction, `Z` for the zone (so `+00:00` is not canonical).
- Whitespace: the type's whiteSpace facet is `collapse`, so leading and trailing whitespace is not part of the value.

`DIRECT-VALUE` (ISO 23009-1 5.8.5.7, ABNF per RFC 5234): `["-"] 4DIGIT "-" 2DIGIT "-" 2DIGIT "T" 2DIGIT ":" 2DIGIT ":" 2DIGIT [ "Z" / ("+" / "-") 2DIGIT ":" 2DIGIT ]`.
It has **no fractional seconds**, though real servers send them (see the Akamai example below); we accept the XSD superset.

## The accepted subset (policy)

Accepted (the XSD form with these restrictions):

1. Optional surrounding ASCII whitespace is trimmed first (an HTTP body ends in a newline).
2. Year: exactly four digits, `0001` to `9999`. Rejected: a minus sign, five or more digits, `0000`, a plus sign.
3. Month 01-12, day valid for that month and year (Gregorian, leap years by the usual rule), hour 00-23, minute 00-59, second 00-59.
4. `24:00:00` with an all-zero fraction is accepted and means 00:00:00 of the next day (the one XSD-valid form that calendar libraries
   often refuse); `24` with any other minute, second or non-zero fraction is rejected.
5. Fraction: `.` followed by one or more digits; digits beyond nanoseconds are truncated (not rounded). No fraction means zero.
6. Timezone: `Z`, or `+hh:mm`/`-hh:mm` with the XSD bounds (hour at most 14, minute at most 59, and `14:00` as the largest magnitude).
7. **No timezone: read as UTC.** The XSD value space calls this ambiguous; DASH makes every wall-clock attribute a UTC instant, so
   treating an untimezoned MPD value as UTC is the only useful reading. This is a deliberate policy, not a spec rule.
8. The result must fit the instant type the runtime uses (`jiff::Timestamp`, whose latest instant is `9999-12-30T22:00:00.999999999Z`, so the last hours of 9999 are
   not representable): out of range is an error, never a wrap.

Rejected (each is an error, never silently coerced): lowercase `t` or `z`; a space instead of `T`; basic format without separators
(`20261026T083904Z`); date only; week/ordinal dates; `,` as the decimal mark; a timezone as `+hh`, `+hhmm` or `+h:mm`; hour offset above 14 or `+14:30`; `:60`
(leap second); a fraction with no digits (`08:39:04.Z`); empty string; surrounding text; an embedded space.

## Examples (instants are Unix seconds, hand-checked with an independent calendar implementation)

| Input | Result | Notes |
| --- | --- | --- |
| `1970-01-01T00:00:00Z` | 0 | epoch zero is a value, not "absent" (livesim2 `availabilityStartTime`) |
| `2026-10-26T08:39:04.952Z` | 1793003944.952 | Akamai `time.akamai.com/?iso&ms` body shape; fraction beyond a `DIRECT-VALUE` |
| `2026-10-26T09:18:20.766Z` | 1793006300.766 | ffmpeg live `availabilityStartTime` (`fixtures/dash/live-timeline/live.mpd` style) |
| `2002-10-10T12:00:00-05:00` | 1034269200 | the W3C example: equals `2002-10-10T17:00:00Z` |
| `2026-03-01T00:00:00+14:00` | 1772272800 | largest accepted zone |
| `2026-04-01T00:00:00+05:30` | 1774981800 | half-hour zone |
| `2026-02-01T00:00:00` | 1769904000 | no zone: UTC by policy rule 7 |
| `2026-02-01T24:00:00Z` | 1769990400 | rule 4: equals `2026-02-02T00:00:00Z` |
| `2024-02-29T12:00:00Z` | 1709208000 | valid leap day |
| `0001-01-01T00:00:00Z` | -62135596800 | first accepted year |
| `9999-12-30T22:00:00Z` | 253402207200 | latest accepted instant (rule 8) |
| `2026-10-26T08:39:04.123456789012Z` | 1793003944.123456789 | rule 5: extra digits truncated |
| `  2026-02-01T00:00:00Z\n` | 1769904000 | rule 1 |
| `2025-02-29T12:00:00Z` | error | no 29 February in 2025 |
| `2026-13-01T00:00:00Z` | error | month 13 |
| `2016-12-31T23:59:60Z` | error | no leap-second form |
| `2026-02-01T24:00:01Z` | error | rule 4 |
| `-0001-01-01T00:00:00Z` | error | rule 2: negative year |
| `12026-01-01T00:00:00Z` | error | rule 2: five-digit year |
| `0000-01-01T00:00:00Z` | error | rule 2 |
| `2026-02-01T00:00:00+14:30` | error | zone beyond +14:00 |
| `2026-02-01T00:00:00+0100` | error | zone needs the colon |
| `2026-02-01t00:00:00z` | error | case |
| `2026-02-01 00:00:00Z` | error | separator |
| `20260201T000000Z` | error | basic format |
| `2026-02-01T00:00:00.Z` | error | empty fraction |
| `9999-12-31T23:59:59Z` | error | beyond the latest instant (rule 8) |
