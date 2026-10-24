---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.13 - Leap seconds (LeapSecondInformation; own-words implementer notes)
clauses: 5.13.1-5.13.4
printed_pages: 150-152
pdf_pages: 158-160
verbatim_source: private/specs-verified/dash/23009-1/05-11_to_5-15.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.13 Leap seconds

## What it defines

MPD-level information (**LeapSecondInformation**, child of `MPD`, 0..1, see [`05-3-1-hierarchy.md`](05-3-1-hierarchy.md)) that relates `MPD@availabilityStartTime` to leap seconds, so that a client can do all timing arithmetic with a constant 86400-second day.

## Normative behaviour (timing arithmetic)

Let ASO = `@availabilityStartLeapOffset`, NASO = `@nextAvailabilityStartLeapOffset`, T_chg = `@nextLeapChangeTime`.

- To use `availabilityStartTime` in calculations that assume a constant day length, **subtract the applicable offset** from it (offset in seconds).
- If current time < T_chg (or T_chg is absent): the applicable offset is ASO.
- If current time >= T_chg: the applicable offset is NASO, but its application may need to be delayed until the client's own wall clock has actually processed the leap second.
- A client whose clock is set once by a one-shot protocol (e.g. `urn:mpeg:dash:utc:http-xsdate:2014`) keeps using ASO until its clock is next synchronised after the leap second.
- If a leap-second correction was already applied to the MPD's times, the element should still be present with ASO = 0.
- Service providers should make sure OS-level "smearing" of the leap second does not corrupt published availability times.
- Informative: leap seconds occur at up to four times a year, can add or remove a second, and happen just before midnight UTC (23:59:60 on New Year's Eve is the common case) but fall in the middle of the day in other time zones.

## Updating the element (5.13.4)

1. After T_chg has passed (and long enough that leap-second-aware clients have resynchronised), move NASO into ASO and remove `@nextAvailabilityStartLeapOffset` and `@nextLeapChangeTime`.
2. At least `MPD@minimumUpdatePeriod` before the next leap-second change, publish new values for the two "next" attributes.
3. Both steps may be done in one update when the new values are already known.

Informative: this update is seamless for clients that process the element and for those that don't, but over many leap seconds clients that ignore it accumulate an error in their live-edge computation.

## Inventory (Table 46)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| LeapSecondInformation | element | LeapSecondInformationType | - | container |
| @availabilityStartLeapOffset | M | xs:integer | - | seconds to subtract from `MPD@availabilityStartTime` at MPD publication time |
| @nextAvailabilityStartLeapOffset | O | xs:integer | - | offset valid from the next leap second; omit if its timing is unknown |
| @nextLeapChangeTime | O | xs:dateTime | - | UTC time of the next leap second; omit if unknown |

```text
LeapSecondInformation (no children)
  attributes: availabilityStartLeapOffset, nextAvailabilityStartLeapOffset, nextLeapChangeTime
```

## Quirks

- 5.13.4 spells one attribute `@availablilityStartLeapOffset` (typo) and says the update "is normally be seamless".

Verbatim text: see `private/specs-verified/dash/23009-1/05-11_to_5-15.md` section 5.13 (printed pp. 150-152).
