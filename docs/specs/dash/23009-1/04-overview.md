<a id="c4"></a>

# ISO/IEC 23009-1:2022 clause 4 - overview (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), clause 4, printed pp. 10-21 (PDF 18-29). Verbatim, page-verified text:
`private/specs-verified/dash/23009-1/04-overview.md`. Page citations below are printed pages (PDF = printed + 8). See [`INDEX.md`](INDEX.md).

<a id="c4-1"></a>

## 4.1 What the document standardises (p. 10-11)

- Two formats: the MPD (XML description of a bounded or unbounded presentation, announcing Segment addresses and their context) and the Segment
  formats (the HTTP response body for a GET, or a ranged GET per RFC 7233, of a resource the MPD names).
- Addresses are HTTP-URLs, optionally with a byte range, or data URLs.
- A "DASH Client" is whatever consumes an MPD and fetches Segments. The document gives it no normative behaviour; only the formats at the client
  interface are in scope.
- Figure 1 (redraw below): authoring/preparation feeds an MPD delivery function and a Segment delivery function; both reach the client, possibly via an HTTP cache.

```text
 [own redraw of Figure 1, printed p. 11]
 Media Presentation Preparation (conceptual)
        |-- MPD ----------> MPD Delivery Function ----------------> DASH Client
        '-- Segments -----> Segment Delivery Function --(HTTP cache)--> DASH Client
```

<a id="c4-2"></a>

## 4.2 Informative client model (p. 11-12)

The formats are shaped around a client made of an access engine (reads the MPD, builds requests, receives Segments or parts) and a media engine
(decodes and renders). The engine hands MPD selection metadata (codecs, languages, accessibility ...) to the application's selection logic and
gets the chosen Adaptation Sets back. Annex K (not transcribed) adds a Service Description to steer client operation.

```text
 [own redraw of Figure 2, printed p. 12]
 MPD ------------------> DASH access engine --media + timing--> Media engine --> output
 Segments -------------> (selection metadata <-> Selection Logic)   ^
                         '------------- events + timing ------------'
```

<a id="c4-3"></a>

## 4.3 Data model (p. 12-15)

Model, outermost to innermost (Figure 3 redraw below):

1. Media Presentation = sequence of Periods along time.
2. Period = a span with a stable set of choices (bitrates, languages, ...); may group Preselections.
3. Adaptation Set = interchangeable encodings of one or more content components (for example main video, main audio, captions, audio description each get their own;
   a multiplex may be one set).
4. Representation = one deliverable encoding; any single Representation of a set suffices to render its components. Putting Representations in one
   set is the author's promise that they are perceptually equivalent, so clients may switch among them.
5. Segments (and optional Sub-Representations) inside a Representation. A Segment is the largest piece fetched by one request. Initialization Segments carry static
   metadata; Media Segments carry samples and advance the timeline; a self-initializing Segment does both. An MPD byte range means the Segment sits inside a bigger resource.

```text
 [own redraw of Figure 3, printed p. 13]   horizontal = time, vertical = choice
 Media Presentation
  '- Period -- Period -- ...
       '- (Preselection) - Adaptation Set - Adaptation Set ...
              '- Representation - Representation ...
                     '- Initialization Segment | Media Segment ... ; Sub-Representation (optional)
```

Timeline facts an implementer must keep apart:

- **Media Presentation timeline**: common to all components of a Period; accurate timing lives inside the Segments, which is what permits sync and seamless switching.
- **Segment availability times**: wall-clock instants at which each URL starts answering. For static presentations every Segment is available at once; for dynamic ones
  availability tracks the Segment's position on the timeline. Clients compare wall-clock time with availability before requesting.
- A Segment's duration is the media duration at normal speed; Segments of one Representation are typically equal or near-equal, and may differ between Representations (down to
  one Segment for the whole Representation). Short Segments bound live latency from below. Segments cannot grow while being fetched (each is a complete object), though chunked
  transfer can still be used on the wire.
- A Segment may be split into Subsegments (whole access units; in ISOBMFF, whole movie fragments) described by a Segment Index giving time range and byte range of each.
- Switching is simplest at SAPs; the simplest deployment makes every (Sub)Segment start at a SAP with boundaries aligned across the set's Representations. Profiles may require the indicators.
- Static (on-demand) MPDs are one fixed document. Dynamic (live) MPDs may be refreshed; template addressing can avoid having to refresh for every new Segment.
- Events (aperiodic, timed, usually with duration) may sit in the MPD or inside Representations: MPD-update notices, ad-insertion cues, application events.

<a id="c4-4"></a>

## 4.4 Protocols (p. 14-15)

Client and server are expected to follow the HTTP/1.1 RFC set 7230-7235 (GET and partial GET are the normal methods); HTTP brings caching, redirection, authentication,
TLS (RFC 2818) and cookies (RFC 6265). The formats may travel over any object-delivery protocol that binds an HTTP-URL to a delivered object.

<a id="c4-5"></a>

## 4.5 Stream and Representation properties (p. 15-17)

<a id="c4-5-1"></a>

### 4.5.1 Seamless switching (p. 15)

A switch A to B at media time t is "seamless" when the output equals decoding A from the start up to t and B from the start from t. Time-aligned Representations of one
set permit this at any t in principle, but it can need two parallel decodes, so authors provide regular switch opportunities and may signal them. Three concepts support it:
stream access points (4.5.2), non-overlapping Segments/Subsegments (4.5.3) and Segment concatenation (4.5.4). The text states they are neither necessary nor sufficient for seamlessness.

<a id="c4-5-2"></a>

### 4.5.2 Stream access points (p. 15-16)

- SAP semantics and types are those of ISO/IEC 14496-12; the same type numbers apply here.
- Per SAP the properties I_SAP, T_SAP, I_SAU, T_DEC, T_EPT, T_PTF come from ISO/IEC 14496-12. T_SAP is the earliest presentation time such that all access units at or after it decode correctly from
  data starting at byte position I_SAP.
- Informal mapping of the six SAP types (the spec's NOTE): 1 = closed-GOP, first-in-decode is first-in-presentation; 2 = closed-GOP, but the first decoded unit is not first presented;
  3 = open-GOP (some following units undecodable with presentation time below T_SAP); 4 = gradual decoder refresh (likewise undecodable leading units); 5 and 6 = some unit after I_SAP is
  undecodable and presented after T_DEC, with T_DEC respectively being / not being the earliest presentation time from I_SAU.
- Uses: random access (start-up, seek, live recovery) and switching (each stream i of the old Representation shown up to T_SAP(i), the new one from T_SAP(i)).
- DASH lets MPD or Segment Index signal I_SAP and T_SAP so the client need not read data after the SAP.

<a id="c4-5-3"></a>

### 4.5.3 Non-overlapping (Sub)Segments (p. 16-17)

With T_E(S,i) and T_L(S,i) as the earliest and latest presentation times of stream i in S: A and B (from any Representations) do not overlap when T_L(A,i) < T_E(B,i) for all
matching streams i, or the reverse with A and B swapped. Segment alignment and Subsegment alignment (Table 5) are defined from this.

<a id="c4-5-4"></a>

### 4.5.4 Conforming Segment track (p. 17)

A sequence of Segments (or Subsegments) conforms if their concatenation is itself a valid bitstream in the media formats used (container and codec), i.e. playable by a conforming player.

<a id="c4-6"></a>

## 4.6 Brands (Table 1, p. 17-18)

ISOBMFF brand identifiers this document defines (facts; clause = where the document defines them, mostly outside the transcribed range):

| Brand | Clause | Meaning (own words) |
|---|---|---|
| `emsg` | 5.10.3.3 | event message box |
| `miss` | 6.2.6 | missing-content Segment |
| `msdh` | 6.3.4.2 | Media Segment of the general ISOBMFF format type |
| `msix` | 6.3.4.3 | Indexed Media Segment |
| `dsms` | 6.3.5.1 | self-initializing Media Segment |
| `dash` | 6.3.5.2 | ISOBMFF file built for DASH (fragments plus Segment Index) |
| `sims` | 6.4.4.4 | Sub-Indexed Media Segment |
| `sisx` | 6.4.6.2 | single Index Segment for MPEG-2 TS Media Segments |
| `risx` | 6.4.6.3 | Representation Index Segment for MPEG-2 TS |
| `ssss` | 6.4.6.4 | Subsegment Index Segment for MPEG-2 TS |
| `lmsg` | 7.3.1 | last Media Segment marker |
| `dums` | 8.11.3 | Segment-sequence marker in the broadcast TV profile |

<a id="c4-7"></a>

## 4.7 Scheme identifiers (Table 2, p. 18-21)

Every scheme URN the document defines, with the clause that defines it (all `urn:mpeg:dash:` prefixed unless shown). The gloss is ours.

| Scheme id (suffix after `urn:mpeg:dash:`) | Clause | Purpose |
|---|---|---|
| `schema:mpd:2011` | Annex B | MPD XML namespace |
| `period-continuity:2015`, `period-connectivity:2015` | 5.3.2.4 | Period continuity / connectivity signalling |
| `adaptation-set-switching:2016` | 5.3.3.5 | switching across Adaptation Sets |
| `preselection:2016` | 5.3.11.2 | Preselection descriptor |
| `reset:2016` | 5.4.2 | MPD reset indicator |
| `resolve-to-zero:2013` | 5.5.3 | xlink resolves to zero elements |
| `mp4protection:2011` | 5.8.5.2 | ISOBMFF protection scheme (from Scheme Type box) |
| `13818:1:CA_descriptor:2011` | 5.8.5.2 | CA system for MPEG-2 TS |
| `14496:10:frame_packing_arrangement_type:2011`, `13818:1:stereo_video_format_type:2011` | 5.8.5.3 | frame packing |
| `23003:3:audio_channel_configuration:2011`, `outputChannelPositionList:2012` | 5.8.5.4 | legacy channel configuration (ISO/IEC 23091-3 recommended instead) |
| `role:2011` | 5.8.5.5 | role scheme |
| `stereoid:2011` | 5.8.5.6 | multi-view description |
| `utc:ntp:2014`, `utc:sntp:2014`, `utc:http-head:2014`, `utc:http-xsdate:2014`, `utc:http-iso:2014`, `utc:http-ntp:2014`, `utc:direct:2014` | 5.8.5.7 | UTC timing sources |
| `audio-receiver-mix:2014` | 5.8.5.8 | receiver mix |
| `mpd-as-linking:2015` | 5.8.5.9 | Adaptation Set linking |
| `sai:2015` | 5.8.5.10 | sub-asset identifier |
| `client-authentication:2015`, `content-authorization:2015` | 5.8.5.11 | client authentication; access authorization |
| `audio-interactivity:2016` | 5.8.5.12 | content interactivity |
| `qr-equivalence:2019` | 5.8.5.13 | quality equivalence |
| `output-protection:hdcp:2020` | 5.8.5.14.2 | HDCP output protection |
| `event:2012`, `event:callback:2015`, `event:ttfn:2016`, `event:catchall:2020` | 5.10.4, 5.10.4.5, 5.10.4.6, A.13 | DASH event signalling, callback, presentation termination, catch-all subscription |
| `mpd-chaining:2016`, `fallback:2016` | 5.11.1, 5.11.2 | MPD chaining; MPD fallback |
| `profile:full:2011`, `profile:isoff-on-demand:2011`, `profile:isoff-live:2011`, `profile:isoff-main:2011`, `profile:mp2t-main:2011`, `profile:mp2t-simple:2011` | 8.2-8.7 | profile ids |
| `profile:isoff-ext-live:2014`, `profile:isoff-ext-on-demand:2014`, `profile:isoff-common:2014`, `profile:isoff-broadcast:2015`, `profile:cmaf:2019`, `profile:cmaf-extended:2019` | 8.8-8.12 | further profile ids |
| `srd:2014`, `srd:dynamic:2016` | H.1 | spatial relationship description (static, dynamic) |
| `urlparam:2014`, `urlparam:2016` | I.2, I.3 | URL query-parameter insertion; extended parameterisation |
| `resolutionSwitching:2016` | Annex J | open-GOP resolution change |

## Transcription notes (spec quirks)

- 4.3 contains the typo "self-initialitizing" (p. 13).
- The 4.5.2 NOTE (the six SAP-type bullets) is a single NOTE in print; the symbols I_SAP etc. are subscripted there.
- Table 1 and Table 2 identifiers wrap across lines in the PDF cells; the rows above are rebuilt from the rendered pages.
