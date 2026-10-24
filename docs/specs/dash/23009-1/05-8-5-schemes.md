---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.8.5 - Specific descriptor scheme definitions, incl. every scheme URI (own-words implementer notes)
clauses: 5.8.5.1-5.8.5.14.2
printed_pages: 122-133
pdf_pages: 130-141
verbatim_source: private/specs-verified/dash/23009-1/05-8.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.8.5 Specific scheme definitions

This clause assigns concrete `@schemeIdUri` values (and value grammars) to the generic descriptors of 5.8.4 ([`05-8-descriptors.md`](05-8-descriptors.md)). Complete list:

## Scheme URI register

| Scheme URI | Used with | Clause | Value syntax (summary) |
|---|---|---|---|
| `urn:mpeg:dash:mp4protection:2011` | ContentProtection | 5.8.5.2.2 | 4CC of the Scheme Type Box, optional ":" + version (1-8 hex digits) |
| `urn:mpeg:dash:13818:1:CA_descriptor:2011` | ContentProtection | 5.8.5.2.3 | 4 lower-case hex digits = CA_system_ID |
| `urn:mpeg:dash:14496:10:frame_packing_arrangement_type:2011` | FramePacking (AVC video) | 5.8.5.3 | VideoFramePackingType (ISO/IEC 23091-2), 1-2 digits |
| `urn:mpeg:dash:13818:1:stereo_video_format_type:2011` | FramePacking (MPEG-2 video) | 5.8.5.3 | same grammar |
| `urn:mpeg:dash:23003:3:audio_channel_configuration:2011` | AudioChannelConfiguration | 5.8.5.4 | ChannelConfiguration of ISO/IEC 23091-3 (Table 68 of 23003-3) |
| `urn:mpeg:dash:outputChannelPositionList:2012` | AudioChannelConfiguration | 5.8.5.4 | space-separated OutputChannelPosition values (23091-3) |
| `urn:mpeg:dash:role:2011` | Role (also usable for Accessibility) | 5.8.5.5 | one of 15 tokens (below) |
| `urn:mpeg:dash:stereoid:2011` | Role | 5.8.5.6 | view indicators `l<i>` / `r<j>` |
| `urn:mpeg:dash:utc:ntp:2014` | UTCTiming | 5.8.5.7 | NTP servers |
| `urn:mpeg:dash:utc:sntp:2014` | UTCTiming | 5.8.5.7 | SNTP servers |
| `urn:mpeg:dash:utc:http-head:2014` | UTCTiming | 5.8.5.7 | HTTP URLs, HEAD, use Date header |
| `urn:mpeg:dash:utc:http-xsdate:2014` | UTCTiming | 5.8.5.7 | HTTP URLs, GET, body = xs:dateTime |
| `urn:mpeg:dash:utc:http-iso:2014` | UTCTiming | 5.8.5.7 | HTTP URLs, GET, body = ISO 8601 time |
| `urn:mpeg:dash:utc:http-ntp:2014` | UTCTiming | 5.8.5.7 | HTTP URLs, GET, body = NTP timestamp |
| `urn:mpeg:dash:utc:direct:2014` | UTCTiming | 5.8.5.7 | the time itself (xs:dateTime form) |
| `urn:mpeg:dash:audio-receiver-mix:2014` | Essential/SupplementalProperty | 5.8.5.8 | AdaptationSet@id |
| `urn:mpeg:dash:mpd-as-linking:2015` | Essential/SupplementalProperty (+EmptyAdaptationSet) | 5.8.5.9 | URL [+ whitespace + timeOffset=n] |
| `urn:mpeg:dash:sai:2015` | SupplementalProperty (AS or SubRepresentation) | 5.8.5.10 | free string (sub-asset id) |
| `urn:mpeg:dash:client-authentication:2015` | value of EssentialProperty@id | 5.8.5.11 | - (id token) |
| `urn:mpeg:dash:content-authorization:2015` | value of EssentialProperty@id | 5.8.5.11 | - (id token) |
| `urn:mpeg:dash:audio-interactivity:2016` | Essential/SupplementalProperty | 5.8.5.12 | owned by the codec |
| `urn:mpeg:dash:qr-equivalence:2019` | SupplementalProperty | 5.8.5.13 | Adaptation Set ids |
| `urn:mpeg:dash:output-protection:hdcp:2020` | OutputProtection | 5.8.5.14.2 | HDCP version token (Table 36) |
| `urn:mpeg:dash:preselection:2016` | Essential/SupplementalProperty | 5.3.11.2 | see [`05-3-11-preselection.md`](05-3-11-preselection.md) |
| `urn:mpeg:dash:reset:2016`, `urn:mpeg:dash:fallback:2016`, `urn:mpeg:dash:mpd-chaining:2016`, `urn:mpeg:dash:period-continuity:2015`, `urn:mpeg:dash:period-connectivity:2015` | MPD / Adaptation Set descriptors | 5.4.2, 5.11, 5.3.2 | see those notes |

(Schemes of other clauses are listed so the whole register is in one place; event-scheme URNs are in [`05-10-events.md`](05-10-events.md).)

## 5.8.5.1 General

These definitions only reserve URIs for existing code points and give simple building blocks; the descriptor mechanism itself stays open to other schemes.

## 5.8.5.2 Content protection schemes

These two URIs only name the *encryption mode*, they are not full DRM schemes (they do not follow the 5.8.4.1.6 template) and are meant to sit next to DRM-specific ContentProtection elements.

**MP4 protection scheme** (ISO/IEC 14496-12 content)
- `@value` is mandatory: the four-character code from the Scheme Type Box of the Protection Scheme Information Box, percent-encoded as in RFC 8141, optionally followed by ":" and a version number of up to eight hex digits (leading zeros may be omitted).
- Value grammar (print's rule names `MP4-PROTECTION-VALUE`, `VERSIONED-4CC`, `CCCC`, `CCCC-VERSION`): four letters, optionally a colon and 1-8 hex digits. The same listing in the print also carries an unused rule `CA-DESCRIPTOR-VALUE` (four lower-case hex digits).
- `@robustness` shall not be present.
- If the Scheme Type Box value is one defined by ISO/IEC 23001-7 (CENC): the author is encouraged to add DRM-specific ContentProtection elements whose `@schemeIdUri` carries a UUID (RFC 4122); the value semantics are governed by that scheme; `@cenc:default_KID` (namespace `urn:mpeg:cenc:2013`) should be present; a player may read it to learn whether the key is already held and may fetch a licence in advance using a `cenc:pssh` element, whose carriage is recommended.

**CA_descriptor scheme** (MPEG-2 TS): `@value` = the 16-bit CA_system_ID of the `CA_descriptor` (ISO/IEC 13818-1) as four lower-case hex digits; `@robustness` shall not be present.

## 5.8.5.3 Frame packing

Two URIs, one per video codec family; in both the value is the `VideoFramePackingType` code of ISO/IEC 23091-2; the value is one or two decimal digits (print's rule names `FRAME-PACKING-ARRANGEMENT-TYPE-VALUE`, `STEREO-VIDEO-FORMAT-TYPE`, `VIDEO-FRAME-PACKING-TYPE`, the second being printed as an alias of the third).

## 5.8.5.4 Audio channel configuration

- `urn:mpeg:dash:23003:3:audio_channel_configuration:2011`: value as `ChannelConfiguration` of ISO/IEC 23091-3 (mapping per Table 68 of ISO/IEC 23003-3).
- `urn:mpeg:dash:outputChannelPositionList:2012`: explicit speaker positions; value is a space-delimited list of `OutputChannelPosition` values (23091-3). The text's example for a 7.1 layout (2 high, 5 mid, 0.1 low) is "2 0 1 4 5 3 17 18".

## 5.8.5.5 DASH role scheme (Table 34)

`urn:mpeg:dash:role:2011`; the Role value is assigned to Adaptation Sets that contain the media component type to which the role belongs. The print's rule `ROLE-VALUE` enumerates exactly the tokens below.

| Value | Meaning (gloss) | Media types |
|---|---|---|
| caption | captions (open/burned-in captions are marked as video only) | video, text |
| subtitle | subtitles | video, text |
| main | primary content intended for presentation when nothing else says otherwise | any |
| alternate | alternative to a main component of the same type; should carry other descriptors saying how it differs | any |
| supplementary | supports a component of a different type (e.g. ambient audio under primary video) | any |
| commentary | commentary experience (e.g. director's), typically audio | audio, text |
| dub | content in a different language than the original (dubbed audio, translated captions) | audio, text |
| description | textual or audio description of visual content | audio, text |
| sign | sign-language interpretation of audio | video |
| metadata | information for application-specific processing | text, application |
| enhanced-audio-intelligibility | improved dialogue intelligibility | audio |
| emergency | information about a current emergency (life, health, safety, property, response) | any |
| forced-subtitle | text shown even when no other text track is selected (dialogue clarification, alternate languages, on-screen text) | text |
| easyreader | simplified/reduced captions as in the cited US regulation | text, video |
| karaoke | lyrics text in the song's language (SMPTE ST 2067-2 cited) | any |

Informative notes: audio+video of a normal programme are both "main"; if one is clearly secondary use "supplementary". Values like subtitle/caption/description/sign/metadata can be mapped to the HTML5 track "kind". The table prints "Main" capitalised while the grammar lists "main".

## 5.8.5.6 Multiple views scheme

`urn:mpeg:dash:stereoid:2011` on a Role of a video ContentComponent or Adaptation Set (an Adaptation Set with several views must give each video Content Component such a Role). It tells which views form stereo pairs and which is left/right when N views combine into M valid pairs. `@value`: space-delimited indicators `l<i>` or `r<j>` (non-negative decimals). Stereo pair i (0 <= i < M) = the view whose Role has `l<i>` (left) plus the view whose Role has `r<i>` (right). All left indicators precede all right ones and each group is in ascending index order. Grammar (print's rule name `STEROID-VALUE`, sic): all left indicators, then all right indicators, separated by single spaces; an indicator is the letter l or r followed by decimal digits.

## 5.8.5.7 DASH UTC timing schemes (Table 35) - the `UTCTiming` element

Purpose: let the client obtain wall-clock time synchronised with the clock used to generate the MPD. Each method names a protocol and the `@value` grammar:

| `@schemeIdUri` | What the client does with `@value` | `@value` grammar |
|---|---|---|
| `urn:mpeg:dash:utc:ntp:2014` | whitespace-separated NTP servers (RFC 5905), recommended not exclusive (other NTP servers allowed); multiple servers improve accuracy | NTP-VALUE |
| `urn:mpeg:dash:utc:sntp:2014` | same with SNTP (RFC 5905) | NTP-VALUE |
| `urn:mpeg:dash:utc:http-head:2014` | HTTP HEAD (RFC 7230) to the listed URLs; wall-clock time is the HTTP `Date` header | HTTP-VALUE |
| `urn:mpeg:dash:utc:http-xsdate:2014` | HTTP GET; response body holds the time as xs:dateTime (XML Schema part 2) | HTTP-VALUE |
| `urn:mpeg:dash:utc:http-iso:2014` | HTTP GET; body holds the time as an ISO 8601 time code | HTTP-VALUE |
| `urn:mpeg:dash:utc:http-ntp:2014` | HTTP GET; body holds the time as an NTP timestamp (RFC 5905 format) | HTTP-VALUE |
| `urn:mpeg:dash:utc:direct:2014` | the `@value` itself is the time as xs:dateTime | DIRECT-VALUE |

Value grammars (the print's rule names): `NTP-VALUE` is one or more servers (host with optional port) separated by white space, host/port per the RFC the text cites as 3896 with the RFC 6874 augmentation; `HTTP-VALUE` is one or more HTTP URLs (RFC 1738) separated by white space; `DIRECT-VALUE` is an ISO 8601 style date-time `[-]YYYY-MM-DDThh:mm:ss` optionally followed by `Z` or a signed `hh:mm` offset.

All time values are based on a wall clock synchronised with the MPD generator's clock. Element-level rules (order = preference, free choice by the client) are in [`05-8-descriptors.md`](05-8-descriptors.md); the UTCTiming element is also reused in the Producer Reference Time element ([`05-12-producer-reference-time.md`](05-12-producer-reference-time.md)) and a dynamic-MPD client uses the resulting clock for availability computations ([`05-3-9-5-segment-information-derivation.md`](05-3-9-5-segment-information-derivation.md)).

Quirks (as printed): the sentence assigning HTTP-VALUE lists http-head, http-xsdate, http-iso and then `urn:mpeg:dash:utc:ntp:2014` (not http-ntp), so http-ntp is not named there although its row says "see below"; the http-ntp row repeats "formatted according to"; the ISO row cites "ISO/IEC 8601"; the `ntp` and `sntp` rows cite RFC 5905 for both NTP and SNTP.

## 5.8.5.8 Audio receiver mix

`urn:mpeg:dash:audio-receiver-mix:2014` in an Essential/SupplementalProperty: the media engine must mix two audio Adaptation Sets before playback. `@value` = `AdaptationSet@id` of the audio set this one must be mixed with (print's rule names `AUDIO-RECEIVER_MIX-VALUE`, `ADAPTATION-SET-ID`: decimal digits). Typical use: a music-and-effects set plus several dialogue-language sets that depend on it. The requirement is one-way: A selected requires B does not mean B selected requires A.

## 5.8.5.9 Adaptation Set linking

`urn:mpeg:dash:mpd-as-linking:2015` says the same Adaptation Set can be found in another MPD. Used with EssentialProperty together with an `EmptyAdaptationSet`, or with SupplementalProperty. Value: a URL to the other MPD including Period/Adaptation Set anchors as defined in C.4.2, optionally followed by whitespace and `timeOffset=<signed integer>` (difference of media time between linked and originating MPD, in `@timescale` units of the linked sets; absent = identical media times). Print's rule names: `MPD-AS-LINKING-VALUE`, `MPD-URL`, `TIMELINE-OFFSET`. Only valid on Adaptation Sets (regular or empty); an Adaptation Set with neither Representation nor `@xlink` must be an empty one marked with an Essential descriptor. Informative: used with Spatial Relationship Description (same `source_id`) for server-based mosaics.

## 5.8.5.10 Sub-asset identifier

`urn:mpeg:dash:sai:2015` on a SupplementalProperty at Adaptation Set or Sub-Representation level; equal `@value` in different Periods means at least the same sub-asset; one set/sub-representation may carry several (it then represents several identified parts). The same scheme+value must not appear on different Adaptation Sets or Sub-Representations inside one Period. Value: any visible-character string (print's rule name `SAI-VALUE`).

## 5.8.5.11 Client authentication and content access authorisation

Generic signalling for external schemes (OMAP, OAuth 2.0, SAML, GAA, GBA are named). Access can also be controlled outside the MPD (tokens checked by a CDN, segment encryption). Where the MPD itself is open, EssentialProperty descriptors at the right level signal the method (`@schemeIdUri` = method, `@value` = scheme-specific data). With several options, EssentialProperty `@id` marks functional equivalents; absent other information it may be `urn:mpeg:dash:client-authentication:2015` (client authentication) or `urn:mpeg:dash:content-authorization:2015` (content access authorisation). A client chooses one element of an `@id` group whose scheme it recognises and runs that protocol; a client that does not support the signalled authorisation cannot play. The subclause imposes no requirement on the authentication itself.

## 5.8.5.12 Audio interactivity

`urn:mpeg:dash:audio-interactivity:2016` (Essential or Supplemental): the audio content (Adaptation Set, Preselection or Representation) contains components enabled for user interaction through metadata (gain, position, mute). Use Supplemental if the UI is optional, Essential if a UI is required to play. `@value` and its semantics belong to the codec.

## 5.8.5.13 Quality equivalence

`urn:mpeg:dash:qr-equivalence:2019` in a SupplementalProperty at MPD, Period or Preselection level (several may exist): the `@qualityRanking` values of the listed Adaptation Sets are comparable. `@value` = Adaptation Set ids (the text says comma-separated while the printed grammar `QR-EQUIVALENCE-VALUE` separates ids with white space); without `@value` all Adaptation Sets below it are equivalent.

## 5.8.5.14 Output protection schemes

`urn:mpeg:dash:output-protection:hdcp:2020`; `@value` is the minimum HDCP version the device output must satisfy; the values form an ordered list, increasing downward (Table 36): 1.0, 1.1, 1.2, 1.3, 1.4, 2.0, 2.2, 2.3 (each meaning "HDCP x.y-compliant device"; there is no 2.1).

Verbatim text: see `private/specs-verified/dash/23009-1/05-8.md` sections 5.8.5.1-5.8.5.14 (printed pp. 122-133).
