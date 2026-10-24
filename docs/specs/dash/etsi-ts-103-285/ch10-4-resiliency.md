---
title: "Clause 10.8 Player resiliency (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.8.1-10.8.6
pdf_pages: 83-91
printed_pages: 83-91
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-4-resiliency.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 10.8 Player resiliency (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Related: BaseURL resolution and `@serviceLocation` (23009-1 5.6.4) in [dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md); provider-side rationale in clause 11.9 ([ch11](ch11-content-provider-guidelines.md)); DASH-IF IOP transport clauses in [dashif-iop-4.3](../dashif-iop-4.3/).

## 10.8.1 Rationale (p.83-84)

Unlike broadcast, a DASH player must detect failures itself and use the alternatives the provider offers. Badly tuned retry logic (e.g. hammering after a 404) can overload servers; the clause pairs multiple server locations with mandatory, bounded player responses and with load spreading through weighted random choice.

## 10.8.2 BaseURL handling

### Extension attributes (10.8.2.1, p.84)

Schema namespace `urn:dvb:dash:dash-extensions:2014-1`, attribute group `baseUrlAttributes` used only on `BaseURL`:
| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `@priority` | positive integer | 1 | lower = preferred |
| `@weight` | positive integer | 1 | relative chance among equal priority (2 = twice as likely as 1) |
| `@serviceLocation` (from 23009-1) | string | for an absolute BaseURL without it, the URL itself | identity of a hosting location for blacklisting |

Rules (shall): multiple BaseURLs at a level are alternatives and shall be used for failover after resolution per 23009-1 5.6. The document base URI counts as a BaseURL with default priority/weight and `serviceLocation` = that URI. Priority/weight/serviceLocation are taken from the BaseURL that supplied the absolute URL; they are ignored on relative BaseURLs.

### Blacklist (p.85)

The player keeps a list of failed `serviceLocation` values: empty when an MPD is first loaded in a session, kept across reloads of that MPD, dropped on a different MPD or session end. When building the BaseURL list, remove entries whose `serviceLocation` is blacklisted and also every other entry with the same `@priority` as a removed one (keeps load ratios from skewing, see 11.9.4).

### Selection (10.8.2.2, p.85)

1. Take resolved, filtered BaseURLs at the current MPD position with the lowest `@priority` value.
2. If several, pick at random with probability proportional to `@weight` (RFC 2782 style is suitable).
3. If none remain, stop playback and report an error.
The choice should be reused until the blacklist or available set changes.

### Changing BaseURL (10.8.2.3, p.85)

Where 10.8.6 requires a change: blacklist the `serviceLocation`; drop it and all same-priority entries; choose among the lowest remaining priority using weights; any other use of that `serviceLocation` (e.g. another AdaptationSet) should switch at the next media request.

### Worked example (10.8.2.4, p.86-87, informative)

An MPD with seven BaseURLs (priority 1 weights 10/30/60 on A, B, C; priority 3 C-alternate; priority 4 B-alternate; priority 5 D and E) shows: weights summing to 100 map a random 0-99 to ranges 0-9, 10-39, 40-99; after B fails, priorities 1 and 4 vanish (B's, plus A and C share priority 1), leaving priority 3; after C fails priority 3 vanishes and priority 5 (two entries, weights 1 and 1, random 0-1) is used; after E fails priority 5 vanishes and the list is empty, so the player reports an error and ends the session. (These boxes are images in the PDF; the private file has them as read from the render. Note: the example's template path prints `$RepresentationId$` and init `1/IS`, while the prose derives `rep1/IS`.)

### DNS (10.8.3, p.87)

Players should use another address from a multi-address DNS answer if the first is unreachable or refuses.

## 10.8.4-10.8.5 Error categories (p.88-89)

Categories: heavy server load; configuration errors; authentication errors; missing segments (equipment failure, player/server time skew on live, intermittent loss); miscellaneous request errors (unexpected statuses, treated like configuration errors); transient connection errors/congestion.

Table 28 inventory (condition -> category):
| Condition | Category |
|---|---|
| DNS failure, host unreachable, connection refused | heavy server load |
| socket timeout, congestion | transient |
| connection closed early (200/206), 200/206 with segment parse error | miscellaneous |
| 401, 402, 403 | authentication |
| 404, 410, 416 | missing segments |
| 405, 406, 407, 408, 409, 411, 412, 413, 414, 415, 417, 501, 505 | miscellaneous |
| 500, 503, 504 | heavy server load |
| 502 | configuration |
Notes: 410 may mean deleted on live but is not guaranteed; 408 is retryable but may mean the player sends an incomplete request.

## 10.8.6 Required reactions (p.89-91)

"Retry" means close and reopen the connection first, with a sensible back-off delay; after a BaseURL change earlier retries need not be remembered.
| Category | MPD type | Reaction (all with retry cap) |
|---|---|---|
| Heavy load, configuration, miscellaneous, authentication | static or dynamic | may retry up to 1 time, then shall change BaseURL |
| Missing segments | static | shall change BaseURL (no retry) |
| Missing segments | dynamic | shall reload the MPD; if it names a time source (4.7.3) shall resynchronise; if the request is now inappropriate shall adjust position to the new MPD/time; otherwise may retry up to 2 times; if still failing shall change BaseURL |
| Transient/congestion | static or dynamic | should be handled by the adaptive-bitrate logic (retry timeouts, lower bit rate) |
Figure 1c (p.91, redrawn): error -> reload manifest -> request still valid? yes: retry up to max, no: recompute live edge and request; then if error again -> change BaseURL (10.8.2.3), else good.

## Notes on the source

- The sentence introducing Table 29 lost the words "Table 29" in print (p.90).
- Figure 1c is labelled with the number 1c although the document's figure numbering continues differently.
