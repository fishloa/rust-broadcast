# SMPTE ST 2110-21:2022 — Traffic Shaping and Delivery Timing for Video

Source: SMPTE ST 2110-21:2022, "Professional Media Over Managed IP Networks:
Traffic Shaping and Delivery Timing for Video" (revision of ST 2110-21:2017),
19 pages, approved December 14, 2022. Transcribed from the vendored PDF
(`private/specs/smpte_st2110-21_2022_traffic_shaping.pdf`) via `pdf2md`
(verified, exit 0).

**Important transcription note**: this document is formula-heavy (fractions,
subscripts, an `INT()` function, piecewise definitions). The PDF's
text-layer, when run through the automated converter, scrambled the token
*order* within several formulas (subscripts and operators got interleaved
across lines) even though the individual digit/symbol tokens themselves
survived the verifier's digit-run check. Every formula below was therefore
re-derived from a page-rendered image of the actual PDF (`pdftoppm` at 200
dpi) — not from the raw converter output — and every numeric constant below
is quoted directly from that image. Where a formula's structure could not be
independently confirmed by the numeric-token verifier, this doc says so.

## 1. Scope (§1)

A timing model for SMPTE ST 2110-20 and ST 2110-22 video RTP streams, as
measured leaving the RTP sender, plus the sender-side SDP parameters that
signal a stream's timing properties.

## 2. Mathematical functions (§5.1)

| Function | Definition |
|---|---|
| `MAX(a, b)` | the larger of `a` and `b` |
| `INT(a)` | the largest integer not greater than `a`, for positive `a` |

## 3. Stream timing characteristics — overview (§6.1)

Two parametric timing models for a sender's packet delivery, both measured
at the sender's transmission interface:

- **Network Compatibility Model** (§6.6.1) — bounds burst characteristics
  for compatibility with switches of varying buffer sizes.
- **Virtual Receiver Buffer (VRB) Model** (§6.6.2) — packets are deposited at
  actual transmission time and removed on a schedule; outstanding-packet
  count is bounded per sender class.

Two **Packet Read Schedules (PRS)** are defined for the VRB model: Gapped
and Linear. The VRB model informs receiver design but does not itself
specify receiver behavior — "Design of receivers is outside the scope of
this standard" (§6.1) — and does not account for network-induced jitter,
which accumulates separately in transit.

## 4. Virtual Receiver Buffer PRS parameters (§6.2)

| Symbol | Meaning |
|---|---|
| `T_FRAME` | time period between consecutive frames at the prevailing frame rate |
| `N_PACKETS` | number of packets per frame of video (mapping-dependent) |
| `T_VD` | Video Transmission Datum: a time point `(N × T_FRAME) + TR_OFFSET`, `N` integer, timescale origin = SMPTE Epoch (ST 2059-1) |
| `TR_OFFSET` | difference between the most recent integer multiple of `T_FRAME` and `T_VD`; shall be ≥ 0, such that `T_VD = (N × T_FRAME) + TR_OFFSET` for each frame |
| `TRO_DEFAULT` | the model-specific default value of `TR_OFFSET` |
| `T_RS` | time between removing adjacent packets from the VRB during the frame/field ("Time-Read-Spacing"); packet removal is modeled as instantaneous |
| `TPR_0` | time the first packet of the frame is removed from the VRB; `TPR_0 = T_VD` ("Time-Packet-Read-Zero") |
| `TPR_j` | time packet `j` is removed from the VRB ("Time-Packet-Read-j") |

Signaling rule: an Uncompressed Video RTP sender whose stream uses a
`TR_OFFSET` differing from `TRO_DEFAULT` **shall** signal the prevailing
value via the SDP `TROFF` Media Type Parameter. If `TROFF` is absent and the
stream carries Uncompressed Video, receivers assume `TRO_DEFAULT`.

## 5. Gapped Packet Read Schedule (§6.3)

### 5.1 Overview (§6.3.1)

The gapped PRS approximates SDI sample delivery, including a gap
corresponding to SDI vertical blanking: one gap per frame (progressive), two
gaps per frame — one per field/segment — (interlaced / PsF). `TPR_j`
instants are uniformly spaced through the active field/segment/frame
interval.

`R_ACTIVE` = the ratio of active time to total time within the frame period.

**The gapped PRS applies only** to ST 2110-20 streams whose image dimensions
and frame rates derive from formats in ITU-R BT.656-5, BT.1543-1, BT.1847-1,
BT.709-6, or BT.2020-2 — even though ST 2110-20 itself only broadly
constrains height/width/frame rate.

### 5.2 Gapped PRS — progressive images (§6.3.2)

Excludes PsF. For an integer `N`:

```
R_ACTIVE = 1080 / 1125
T_RS     = (T_FRAME × R_ACTIVE) / N_PACKETS
T_VD     = (N × T_FRAME) + TR_OFFSET
TPR_j    = (j × T_RS) + T_VD

TRO_DEFAULT = (43/1125)  × T_FRAME    if Image Height >= 1080 lines
            = (28/750)   × T_FRAME    if Image Height <  1080 lines
```

This single model's inter-frame gap and `TRO_DEFAULT` cover **all** non-PsF
progressive formats: 720p, 1080p, 2160p, 4320p. The `TRO_DEFAULT` values sit
slightly after the start of active video, to leave time to assemble/buffer
packets from a timed SDI signal into the Standard UDP Size Limit.

### 5.3 Gapped PRS — interlaced and PsF images (§6.3.3)

`HEIGHT` = the SDP `height` Media Type Parameter (ST 2110-20 §7.2).
`R_ACTIVE`, `TRO_DEFAULT`, and `T_LINE` depend on the interlace standard —
see Table 1. For an integer `N`:

```
T_RS  = (T_FRAME × R_ACTIVE) / N_PACKETS
T_VD  = (N × T_FRAME) + TR_OFFSET

TPR_j = (j × T_RS) + T_VD,                                              0 <= j < N_PACKETS/2
      = T_FRAME/2 + T_LINE/2 + ((j - N_PACKETS/2) × T_RS) + T_VD,  N_PACKETS/2 <= j < N_PACKETS
```

i.e. the second field/segment's read schedule restarts at half a frame plus
half a line past `T_VD`, then runs the same `T_RS`-spaced sequence.

**Table 1 — Ratio of active to total time for interlaced systems (§6.3.3)**

| System | R_ACTIVE | TRO_DEFAULT | T_LINE |
|---|---|---|---|
| 525-line interlaced (ITU-R BT.656-5) | `HEIGHT / 525` | `(INT((525 − HEIGHT)/2) / 525) × T_FRAME` | `T_FRAME / 525` |
| 625-line interlaced (ITU-R BT.656-5) | `HEIGHT / 625` | `(INT((625 − HEIGHT)/2) / 625) × T_FRAME` | `T_FRAME / 625` |
| 1125-line interlaced and PsF (ITU-R BT.709-6) | `HEIGHT / 1125` | `(INT((1125 − HEIGHT)/2) / 1125) × T_FRAME` | `T_FRAME / 1125` |

Notes (§6.3.3): `TRO_DEFAULT` is chosen slightly after active-video start,
for the same buffering reason as §5.2. The 525/625/1125 formulas presume the
transmitted visual lines align to the **bottom** of the active video area
(per the referenced Recommendations), with the blanking area above active
video sized by `HEIGHT`.

## 6. Linear Packet Read Schedule (§6.4)

Applies uniformly to all images (progressive, interlaced, PsF — no gap).
`TPR_j` values are evenly spaced through the whole frame period `T_FRAME`.
For an integer `N`:

```
T_RS        = T_FRAME / N_PACKETS
T_VD        = (N × T_FRAME) + TR_OFFSET
TPR_j       = (j × T_RS) + T_VD
TRO_DEFAULT = TRO_DEFAULT as defined in the gapped model of §6.3 (i.e. §5.2/§5.3 above)
```

## 7. Relationship between Linear and Gapped PRS (§6.5, informative)

`T_VD` is common to both models. Using the same `TR_OFFSET` in both, given a
sufficient `VRXFULL`, a receiver reading on the linear PRS can also
accommodate a signal meeting the gapped PRS's requirements.

## 8. Transmission Traffic Shape Models (§6.6)

### 8.1 Network Compatibility Model (§6.6.1)

A sender's actual transmission-instant sequence, measured at its network
egress (before any network-induced impairment), must pass this model at all
times/configurations:

- Packets enter a leaky bucket of **infinite capacity** the instant they're
  emitted.
- The bucket drains one packet every `T_DRAIN` seconds, if available; for
  modeling purposes the drain instant is `N × T_DRAIN` seconds since the
  SMPTE Epoch.
- `C_INST` = the instantaneous packet count in the bucket; it shall never
  exceed the sender-type-specific `C_MAX` (§9 below).

Parameters:

```
R_NOMINAL = N_PACKETS / T_FRAME        (long-term average packet rate, packets/s)
β         = a scaling factor applied to R_NOMINAL (per-sender-type, §9)
T_DRAIN   = (T_FRAME / N_PACKETS) × (1 / β)
C_INST    = instantaneous bucket fullness, in packets
```

### 8.2 Virtual Receiver Buffer Model (§6.6.2)

A sender's actual transmission-instant sequence, likewise measured at
egress, must pass this model at all times/configurations:

- Packets enter a leaky bucket of capacity `VRXFULL` the instant they're
  emitted; entry/exit is modeled as instantaneous.
- The bucket drains packet `j` at the Packet Read Schedule instant `TPR_j`.
- The sender must ensure the bucket never overflows, and that packet `j` is
  emitted onto the network no later than `TPR_j` (no underflow).
- `TPR_j` = as defined in §4–§6 above for the prevailing PRS model.
- `VRXFULL` (units: packets) = the VRB's capacity, i.e. the bound on
  outstanding packets between the sender's actual emission times and the
  drain schedule. Per-sender-type values are in §9 below.
- Evaluated using the RTP Clock timebase the sender signals in SDP (ST
  2110-10).

## 9. Compliance definitions — Senders (§7.1)

A compliant sender conforms to one or more of Type N, NL, or W (§7.1.1).

### 9.1 Narrow Senders — Type N (§7.1.2)

Employs the **gapped** PRS (§5). VRB compliance (§8.2) with:

```
VRXFULL = MAX( INT(1500 × 8 / MAXUDP), INT(N_PACKETS / (27000 × T_FRAME)) )
```

Network Compatibility (§8.1) with:

```
C_MAX = MAX( 4, INT( N_PACKETS / (43200 × R_ACTIVE × T_FRAME) ) )
β = 1.10
```

`MAXUDP` = 1500 if the Standard UDP Size Limit is in use, else the Extended
value defined in ST 2110-10. Signaled via `TP=2110TPN`.

Notes (§7.1.2): the `VRXFULL` formula guarantees a minimum 8-packet VRB
(just over 2 lines of 4:2:2/10 in a 1920-wide format), scaling up for
higher-rate signals; for larger UDP packets, values scale down proportionally
via the reduction in `N_PACKETS`. A sender packing a locked/phased SDI
signal's video payload into maximum standard-sized packets, transmitted when
full, can be compliant to this type. See §11 (Annex A) for the `C_MAX`
derivation. The `MAX(4, …)` floor matters for small images/low frame rates
— when it's in effect, switch buffer/loading needs care under simultaneous
multi-stream bursts.

### 9.2 Narrow Linear Senders — Type NL (§7.1.3)

Employs the **linear** PRS (§6). VRB compliance with:

```
VRXFULL = MAX( INT(1500 × 8 / MAXUDP), INT(N_PACKETS / (27000 × T_FRAME)) )
```

(identical formula to Type N). Network Compatibility with:

```
C_MAX = MAX( 4, INT( N_PACKETS / (43200 × T_FRAME) ) )
β = 1.10
```

Note this `C_MAX` formula has **no `R_ACTIVE` factor** — it differs from
Type N's only by that factor's absence (Type NL uses the linear PRS, which
has no active/gap ratio). `MAXUDP` rule and floor-of-4 caveat are the same
as Type N. Signaled via `TP=2110TPNL`.

### 9.3 Wide Senders — Type W (§7.1.4)

Employs the **linear** PRS (§6). VRB compliance with:

```
VRXFULL = MAX( INT(1500 × 720 / MAXUDP), INT(N_PACKETS / (300 × T_FRAME)) )
```

Network Compatibility with:

```
C_MAX = MAX( 16, INT( N_PACKETS / (21600 × T_FRAME) ) )
β = 1.10
```

The `C_MAX` definition **applies only to streams under 900,000
packets/second**. `MAXUDP` rule is the same as Type N/NL. Signaled via
`TP=2110TPW`.

Notes (§7.1.4):

1. The `C_MAX` spec remains a topic of study, particularly scaling at higher
   stream rates.
2. `VRXFULL` guarantees a minimum 720-packet VRB (~20% of a 1920×1080
   4:2:2/10 frame at Standard UDP size), scaling proportionally for
   higher-rate signals; larger UDP packets proportionally shrink the allowed
   buffer via `N_PACKETS`.
3. Type W exists for present/future software-based sources with wider packet
   timing variation than N/NL — the larger `VRXFULL` accommodates both
   increased packet delay variation and sender/`ts-refclk` misalignment.
4. The minimum `C_MAX` (16) is larger than N/NL's (4); when it's in effect,
   the same switch-buffer/loading caveat applies, including under the
   Extended UDP Size Limit.

## 10. Compliance definitions — Receivers (§7.2)

A compliant receiver conforms to one or more of Type N, W, or A (§7.2.2).
While sender models (§8) bound egress traffic shape, jitter/delay
accumulates in transit — practical receivers should accommodate jitter
beyond the sender-side traffic profile (§7.2.1, informative).

| Type | Can receive from | Conditions |
|---|---|---|
| N — Narrow, Synchronous (§7.2.3) | Type N sender (**should** also support Type NL, same conditions) | (a) same clock source as sender's `ts-refclk`; (b) sender's `mediaclk` is `mediaclk:direct`; (c) sender's `TROFF` equals default or is absent. A Type N receiver **should** support alternative `TROFF` values. |
| W — Wide, Synchronous (§7.2.4) | Type N, NL, or W sender | (a) same clock source as sender's `ts-refclk`; (b) sender's `mediaclk` is `mediaclk:direct`. **Shall** support alternative `TROFFSET` values via `TROFF`. (The default `TROFFSET` is common to Type N and Type W senders — §7 note referencing §6.5.) |
| A — Asynchronous (§7.2.5) | Type N, NL, or W sender | none — regardless of `ts-refclk`, `mediaclk`, or `TROFF` |

## 11. Annex A — the `C_MAX` derivation (informative, §Annex A)

Motivation: COTS network switches (circa 2017, the spec's original
publication year) share a fixed buffer pool across egress ports; more
buffer per port tracks with more expensive switches. `C_MAX` scales with a
stream's nominal rate. Model: a simple switch, optimal buffer sharing (real
switches may do worse), worst case = all ports simultaneously at max egress
utilization.

Variables:

| Symbol | Meaning |
|---|---|
| `E_total` | total egress capacity of the switch |
| `E_used` | used egress capacity of the switch |
| `U` | `E_used / E_total` (aggregate utilization factor) |
| `B_total` | total buffer capacity of the switch |
| `BB` | buffer per unit of used bandwidth |
| `R_stream` | data rate of a single stream |
| `B_stream` | buffer for a single stream |

Derivation:

```
BB = B_total / E_used = B_total / (U × E_total)

B_stream = R_stream × BB = R_stream × B_total / (U × E_total)
         = R_stream / ( (U × E_total) / B_total )
```

Plugging in "figures for a typical open-market switch ASIC and a maximum
utilization of 90%":

```
E_total = 3.2 Tbit/s
B_total = 16 Mbytes
U       = 0.90     (90% utilization)

B_stream = R_stream / ( (3.2×10^12) / (1.1 × 16 × 2^20 × 8) )
         = R_stream / 21674
```

(The spec does not explain the `1.1` factor at the point this formula first
appears; it is quoted verbatim from the source. The closing paragraph of
this Annex — "We used the value U = 90% in the model above and this
corresponds to a value of β = 1.1 in the network compatibility
specifications above" — is the only place the document connects a `1.1` to
anything, and it is the same `β = 1.10` used in the §9 `C_MAX` formulas; this
transcription infers, but the source does not explicitly state, that the
`1.1` in the `B_stream` formula is that same β. The `16 × 2^20 × 8` factor is
an unglossed unit conversion — 16 Mbytes to bits — also not spelled out in
the source.)

To be more cautious, the derivation is further adjusted "based on the
relative likelihood of synchronicity between bursting events" — halved for
Type N/NL, left alone (and rounded) for Type W:

```
B_Nstream = R_stream / 43200      (halved from ~21674, then rounded — the denominator doubles)
B_Wstream = R_stream / 21600      (rounded from 21674, not halved)
```

These are exactly the `43200` (Type N/NL) and `21600` (Type W) denominators
in the `C_MAX` formulas of §9.1–9.3. Stated assumptions: highly loaded
networks (`U`=90%), modest likelihood of cross-stream burst coupling; larger
real-world switch buffers may relax these constraints.

Closing the loop on `β`: in a simple switch, egress buffer drains at line
rate; with all ports loaded to utilization `U`, a stream's effective buffer
drain rate is its data rate divided by `U`. The model's `U = 90%`
corresponds to `β = 1.1` in the Network Compatibility parameters above.

## 12. Session Description Considerations (§8)

### 12.1 Required parameters (§8.1)

| Parameter | Meaning |
|---|---|
| `TP` | sender type per §9: `2110TPN`, `2110TPNL`, or `2110TPW` |

### 12.2 Optional parameters (§8.2)

| Parameter | Meaning | Default when absent |
|---|---|---|
| `TROFF` | the sender's `TROFFSET` value, positive integer microseconds | `TRO_DEFAULT` (mandatory to signal in some cases — see §4) |
| `CMAX` | the largest `C_INST` the sender will produce, integer | the `C_MAX` defined for the sender's class (§9) |

## 13. Bibliography (§Bibliography)

- SMPTE ST 2110-22:2019, "Professional Media over IP Networks: Constant
  Bit-Rate Compressed Video" (informative cross-reference only).

## 14. What this document leaves unstated

- The exact interaction/precedence when a sender signals both a non-default
  `TROFF` *and* a non-default `CMAX` is not spelled out beyond each
  parameter's own definition.
- §7.1.4's `C_MAX` formula is explicitly flagged by the spec itself (Note 1)
  as "a topic of study" for higher stream rates — i.e. even the standard
  does not claim this constant is settled for all cases.
- The precise switch ASIC figures in Annex A (`3.2 Tbit/s`, `16 Mbytes`,
  `90%` utilization) are stated as illustrative ("typical open-market switch
  ASIC") — the standard does not claim these as guaranteed minimums for any
  real deployment, only as the basis of the informative derivation that
  produced the normative `43200`/`21600` constants used in §9.
