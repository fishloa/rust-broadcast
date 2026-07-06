# SRT (draft-sharabayko-srt-01) — §5.2 File Transfer Congestion Control (FileCC)

Curated congestion-control rules for a FileCC implementation (the sender-side
window/pacing state machine used for file transfer, §4.2, as opposed to
LiveCC's pacing-only model in `specs/rules/srt-livecc.md`).

Source: `specs/ietf_draft_sharabayko_srt_01.txt`, §5.2 "File Transfer
Congestion Control (FileCC)" (incl. §5.2.1 "SRT's Default FileCC Algorithm",
§5.2.1.1 "Slow Start", §5.2.1.2 "Congestion Avoidance", §5.2.1.3 "Link
Capacity and Receiving Rate Estimation") — **L3283-3699** (§5.2 heading through
the line immediately before §6 "Encryption" begins at L3701). Line cites below
are exact source lines.

Constants/state already defined elsewhere and only cross-referenced here (not
redefined):

- `SYN` / `RC_INTERVAL` = 0.01 s = 10 ms — **primarily defined in this
  section** (L3421-3425, see below) and imported *from here* by
  `specs/rules/srt-livecc.md` (its own L3421-3423 citation). Also restated as
  the Full ACK timer, "10 milliseconds" (§4.8.1, `specs/rules/srt-arq.md`
  rule 11, L2874-2876).
- `RTT` — round-trip time estimate, microseconds, receiver-reported and
  sender-smoothed; full EWMA definition + initial value (100 ms) in
  `specs/rules/srt-arq.md` §4.10 (rules 26-31). This section's formulas
  reuse `RTT` (L3409-3411) but do not redefine its smoothing.
- `MAX_BW` — configured maximum bandwidth; full three-mode definition
  (`MAXBW_SET`/`INPUTBW_SET`/`INPUTBW_ESTIMATED`) in `specs/rules/srt-livecc.md`
  §5.1.1. This section (Step 6, L3549-3566) only adds FileCC-specific usage
  rules (see below).
- `S` (SRT packet size, IP payload bytes, "SRT treats 1500 bytes as a
  standard packet size", L3539-3540) is a **different quantity** from
  LiveCC's computed `PktSize` (`AvgPayloadSize + SRT header size`,
  `specs/rules/srt-livecc.md` L3227-3229) — FileCC's `S` is not derived from
  an EWMA in this text, it is stated as a fixed/standard value.

## §5.2 Overview (L3283-3334)

1. For file transfer (§4.2), any known congestion-control algorithm — e.g.
   **CUBIC [RFC8312]** or **BBR [BBR]** — can be applied, "including SRT's
   default FileCC algorithm described below" (L3285-3287). Neither CUBIC nor
   BBR is described further in this draft; they are named as *alternatives*,
   not restated — using either as the FileCC algorithm is out of scope for
   this note (external-algorithm reference, not filled in here).
2. SRT's default FileCC algorithm is **"a modified version of the UDT native
   congestion control algorithm [GuAnAO], [GHG04b] designed for a bulk data
   transfer over networks with a large bandwidth-delay product (BDP)"**
   (L3291-3293, verbatim). It is **"a hybrid Additive Increase
   Multiplicative Decrease (AIMD) algorithm"** (L3294-3295, verbatim) that
   adjusts both:
   - **`CWND_SIZE`** — congestion window size, in **packets** (L3296-3298).
   - **`PKT_SND_PERIOD`** — packet sending period, in **microseconds**
     (L3296-3298).
3. The algorithm tunes `PKT_SND_PERIOD` to control sending rate; rate is
   **increased on ACK receipt**, **decreased on NAK (loss report) receipt**
   (L3309-3312). **Only full ACKs, not light ACKs (§4.8.1), trigger a rate
   increase** (L3313-3314).
4. Two phases, run in strict sequence: **Slow Start**, then **Congestion
   Avoidance** (L3316-3317).
   - Slow Start: probes the network for available bandwidth / target sending
     rate for the next phase; grows the sending rate absent loss reports,
     shrinks it on detected congestion (L3317-3323).
   - Slow Start **"runs exactly once at the beginning of a connection"**, and
     stops on any of: a packet loss, `CWND_SIZE` reaching its maximum value,
     or a timeout event (L3323-3326, verbatim quote of the stop conditions).
5. As with LiveCC, FileCC reacts to the same three sender-side events:
   (1) sending a data packet, (2) receiving an ACK, (3) a timeout event
   (L3331-3334) — see §5.2.1.1/§5.2.1.2 below for how each phase handles
   events (2) and (3). Event (1) (sending a data packet) is not given a
   distinct FileCC-specific handler in this text beyond what's described
   under the ACK/NAK/RTO steps.

## §5.2.1.1 Slow Start (L3336-3440)

6. **Fixed pacing during slow start**: `PKT_SND_PERIOD` is held at **1
   microsecond** throughout slow start, "in order to send packets as fast as
   possible, but not at an infinite rate" (L3338-3340, verbatim).
7. **`CWND_SIZE` initial value**: **16 packets** (L3340-3341).
8. **`MAX_CWND_SIZE`**: the upper threshold for `CWND_SIZE`; even absent
   packet loss, slow start must stop once `CWND_SIZE` exceeds it
   (L3341-3344). **"The threshold can be set to the maximum receiver buffer
   size (12 MB)"** (L3344-3345, verbatim) — worded as a settable/recommended
   value, not a hardwired constant.

### (1) On ACK packet reception (L3347-3425)

Step 1 (L3349-3354, code L3365-3371):

```
if (currTime - LastRCTime < RC_INTERVAL)
{
    Keep the sending rate at the same level;
    Stop;
}
```

- `currTime`: current time, microseconds. `LastRCTime`: last time the
  sending rate was increased or kept, microseconds (L3373-3375).

Step 2 (L3377-3379):

```
LastRCTime = currTime
```

Step 3 (L3381-3385):

```
CWND_SIZE += ACK_SEQNO - LAST_ACK_SEQNO
```

- `CWND_SIZE` grows by the sequence-number delta between the just-ACKed
  packet (`ACK_SEQNO`) and the previously last-ACKed packet
  (`LAST_ACK_SEQNO`) (L3381-3385).

Step 4 (L3387-3390):

```
LAST_ACK_SEQNO = ACK_SEQNO
```

Step 5 (L3392-3401) — if `CWND_SIZE` (post Step 3) exceeds `MAX_CWND_SIZE`,
**slow start ends**, and `PKT_SND_PERIOD` is set:

```
if (RECEIVING_RATE > 0)
    PKT_SND_PERIOD = 1000000 / RECEIVING_RATE;
else
    PKT_SND_PERIOD = CWND_SIZE / (RTT + RC_INTERVAL);
```

where (L3403-3425):

- `RECEIVING_RATE`: rate packets are being received, packets/sec, receiver-
  reported and sender-smoothed (§3.2.4, §5.2.1.3 below) (L3405-3407).
- `RTT`: round-trip time estimate, microseconds, receiver-reported and
  sender-smoothed (§3.2.4, §4.10 — cross-ref `specs/rules/srt-arq.md`)
  (L3409-3411).
- **`RC_INTERVAL`**: "the fixed rate control interval, in microseconds.
  RC_INTERVAL of SRT is SYN, or synchronization time interval, which is 0.01
  second. An ACK in SRT is sent every fixed time interval. The maximum and
  default ACK time interval is SYN. See Section 4.8.1 for details."
  (L3421-3425, verbatim) — i.e. `RC_INTERVAL = SYN = 0.01 s = 10 ms`.

### (2) On a loss report (NAK) packet reception (L3427-3432)

9. Slow start ends; `PKT_SND_PERIOD` is set exactly as in Step 5 above
   (L3429-3432).

### (3) On a retransmission timeout (RTO) event (L3434-3439)

10. Slow start ends; `PKT_SND_PERIOD` is set exactly as in Step 5 above
    (L3436-3439).

## §5.2.1.2 Congestion Avoidance (L3441-3659)

Entered once slow start ends (L3443-3444).

### (1) On ACK packet reception (L3446-3567)

Step 1 (L3448-3453, code L3455-3461) — identical rate-control gate to Slow
Start Step 1:

```
if (currTime - LastRCTime < RC_INTERVAL)
{
    Keep the sending rate at the same level;
    Stop;
}
```

Step 2 (L3467-3477):

```
LastRCTime = currTime
```

Step 3 (L3479-3481) — recompute `CWND_SIZE` directly (not incrementally, as
in slow start):

```
CWND_SIZE = RECEIVING_RATE * (RTT + RC_INTERVAL) / 1000000 + 16
```

Step 4 (L3483-3493) — loss-in-flight guard:

11. If `bLoss == True` (packet loss reported by the receiver since the last
    rate increase): keep `PKT_SND_PERIOD` unchanged, set `bLoss = False`,
    and stop (L3483-3490).
12. `bLoss` initial value: **False** (L3492-3493).

Step 5 (L3495-3547) — if `bLoss == False`, compute `PKT_SND_PERIOD`:

```
inc = 0;

lossBandwidth = 2 * (1000000 / LastDecPeriod);
linkCapacity = min(lossBandwidth, EST_LINK_CAPACITY);
B = linkCapacity - 1000000 / PKT_SND_PERIOD;

if ((PKT_SND_PERIOD > LastDecPeriod) && ((linkCapacity / 9) < B))
    B = linkCapacity / 9;
if (B <= 0)
    inc = 1 / S;
else
{
    inc = pow(10.0, ceil(log10(B * S * 8))) * 0.0000015 / S;
    inc = max(inc, 1 / S);
}

PKT_SND_PERIOD = (PKT_SND_PERIOD * RC_INTERVAL) /
                  (PKT_SND_PERIOD * inc + RC_INTERVAL);
```
(L3498-3517, verbatim)

where (L3519-3540):

- **`LastDecPeriod`**: value of `PKT_SND_PERIOD` immediately before the last
  rate decrease (on NAK receipt), microseconds. **Initial value: 1
  microsecond** (L3521-3524).
- **`EST_LINK_CAPACITY`**: estimated link capacity, receiver-reported within
  an ACK and sender-smoothed (§5.2.1.3 below), packets/sec (L3533-3535).
- **`B`**: estimated available bandwidth, packets/sec (L3537).
- **`S`**: "the SRT packet size (in terms of IP payload) in bytes. SRT
  treats 1500 bytes as a standard packet size." (L3539-3540, verbatim).

13. **External-algorithm reference, not restated**: "A detailed explanation
    of the formulas used to calculate the increase in sending rate can be
    found in [GuAnAO]" (L3542-3543) — the derivation of the `inc`/`B`
    formulas above is not given in this draft beyond the code block itself;
    [GuAnAO] = Gu, Hong, Grossman, "An Analysis of AIMD Algorithm with
    Decreasing Increases" (GridNets '04, Oct 2004) is cited, not
    reproduced.
14. Note: "UDT's available bandwidth estimation has been modified to take
    into account the bandwidth registered at the moment of packet loss,
    since the estimated link capacity reported by the receiver may
    overestimate the actual link capacity significantly." (L3544-3547,
    verbatim) — i.e. the `lossBandwidth`/`min()` clamp in the code above is
    SRT's modification versus baseline UDT; the baseline UDT formula itself
    is not given here.

Step 6 (L3549-3566) — `MAX_BW` (§5.1) clamp, if set:

```
if (MAX_BW)
    MIN_PERIOD = 1000000 / (MAX_BW / S);

    if (PKT_SND_PERIOD < MIN_PERIOD)
        PKT_SND_PERIOD = MIN_PERIOD;
```
(L3553-3559, verbatim)

15. For file transmission, only **`MAXBW_SET`** mode is applicable for
    `MAX_BW` (§5.1.1, cross-ref `specs/rules/srt-livecc.md`); unlike live
    streaming, **there is no default `MAX_BW` value** for file transfer, and
    the rate is unlimited if `MAX_BW` is not explicitly set (L3561-3566).

### (2) On a loss report (NAK) packet reception (L3568-3658)

Step 1 (L3570):

```
bLoss = True
```

Step 2 (L3572-3593) — loss-ratio tolerance:

16. If the sender's estimated current loss ratio is **less than 2%**: keep
    the sending rate unchanged, set `LastDecPeriod = PKT_SND_PERIOD`, and
    stop (L3572-3579, `LastDecPeriod = PKT_SND_PERIOD` at L3579).
17. Rationale (verbatim, L3591-3593): "This modification has been introduced
    to increase the algorithm tolerance to a random packet loss specific for
    public networks, but not related to the absence of available
    bandwidth."

Step 3 (L3595-3647) — new congestion period detection: if the lost packet's
sequence number is **greater than `LastDecSeq`** (the largest sequence
number sent so far when the last decrease happened), i.e. this NAK starts a
new congestion period (L3595-3597):

18. `LastDecPeriod = PKT_SND_PERIOD` (current value, before the increase
    below) (L3599-3600).
19. **Rate increase**: `PKT_SND_PERIOD = 1.03 * PKT_SND_PERIOD` (L3602-3604,
    verbatim).
20. **`AvgNAKNum` update**: `AvgNAKNum = 0.97 * AvgNAKNum + 0.03 * NAKCount`
    (L3606-3608, verbatim).
21. Reset `NAKCount = 1` and `DecCount = 1` (L3610).
22. Record `LastDecSeq` = current largest sent sequence number (L3612).
23. Compute `DecRandom` = a random, uniformly-distributed number between 1
    and `AvgNAKNum`; if `DecRandom < 1`, clamp `DecRandom = 1` (L3614-3615).
24. Stop (L3617).

State variables introduced here (L3619-3633, verbatim definitions):

- **`AvgNAKNum`** — "the average number of NAKs during a congestion period.
  Initial value: 0."
- **`NAKCount`** — "the number of NAKs received so far in the current
  congestion period. Initial value: 0."
- **`DecCount`** — "the number of times that the sending rate has been
  decreased during the congestion period. Initial value: 0."
- **`DecRandom`** — "a random number used to decide if the rate should be
  decreased or not for the following NAKs (not the first one) during the
  congestion period. DecRandom is a random number between 1 and the average
  number of NAKs per congestion period (AvgNAKNum)."

25. **Congestion period, definition** (verbatim, L3645-3647): "Congestion
    period is defined as the time between two NAKs in which the biggest lost
    packet sequence number carried in the NAK is greater than the
    LastDecSeq."
26. Note (verbatim, L3649-3650): "The coefficients used in the formulas
    above have been slightly modified to reduce the amount by which the
    sending rate decreases." — i.e. the `1.03` multiplier and `0.97/0.03`
    EWMA weights are SRT's own tuning versus baseline UDT; no baseline
    values are given to compare against.

Step 4 (L3652-3658) — repeat decrease within the same congestion period: if
`DecCount <= 5` **and** `NAKCount == DecCount * DecRandom`:

27. `SND = 1.03 * SND` (L3654, verbatim — the draft names the variable
    `SND` here rather than `PKT_SND_PERIOD`; no separate `SND` variable is
    defined anywhere else in this section, so this is transcribed exactly
    as written rather than silently "corrected" to `PKT_SND_PERIOD`).
28. Increase `DecCount` and `NAKCount` each by 1 (L3656).
29. Record `LastDecSeq` = current largest sent sequence number (L3658).

## §5.2.1.3 Link Capacity and Receiving Rate Estimation (L3660-3699)

30. Both estimates — **link capacity** and **receiving rate**, in
    packets/bytes per second — are computed **at the receiver side**, during
    file transmission (§4.2) (L3662-3664).
31. **Receiving-rate estimate usage**: available throughout the transfer,
    but **only consumed during the slow start phase** (§5.2.1.1)
    (L3664-3666). "The latest estimate obtained before the end of the slow
    start period is used by the sender as a reference maximum speed to
    continue data transmission without further congestion." (L3666-3669,
    verbatim).
32. **Link-capacity estimate usage**: estimated continuously and used
    throughout the transmission (not just slow start) for sending-rate
    adjustments, "primarily (as well as packet loss ratio and other
    protocol statistics)" (L3669-3672).
33. **Measurement basis**: as each data packet arrives, the receiver
    records the inter-arrival time delta from the previous data packet, used
    to estimate bandwidth and receiving speed (L3674-3676).
34. **Transport**: these estimates are carried to the sender in ACK packets,
    "sent every 10 milliseconds" (L3676-3678) — consistent with `SYN`/
    `RC_INTERVAL` = 10 ms (L3421-3425) and the Full ACK timer (cross-ref
    `specs/rules/srt-arq.md` rule 11).
35. **Sender-side smoothing**: "upon receiving a new value, an exponentially
    weighted moving average (EWMA) is applied to update the latest estimate
    maintained at the sender side" (L3678-3681, verbatim) — this is the
    smoothing referenced by `RECEIVING_RATE`/`EST_LINK_CAPACITY` above
    (L3405-3407, L3533-3535). **No EWMA weight/coefficient is given** for
    this smoothing in this section (contrast with LiveCC's explicit 7/8+1/8
    `AvgPayloadSize` weights, or §4.10's explicit 7/8+1/8 `RTT` weights) —
    this is a gap in the draft's own text, not filled in here.
36. **Data-probing distinction** (L3683-3689): bandwidth estimation uses
    **only data probing packets**; receiving-speed (delivery rate)
    estimation uses **all data packets** (both plain data and data
    probing). "Data probing refers to the use of the packet pairs
    technique, whereby pairs of probing packets are sent to a server
    back-to-back, thus making it possible to measure the minimum interval
    in receiving consecutive packets." (verbatim). No further packet-pairs
    algorithm detail (e.g. probing frequency, packet-pair spacing) is given
    in this draft.
37. **External-algorithm reference, not restated**: "The detailed
    description of models used to estimate link capacity and receiving rate
    can be found in [GuAnAO], [GHG04b]." (L3691-3692) — the actual
    estimation formulas are **not reproduced in this draft**; they live only
    in the cited external papers (Gu/Hong/Grossman, GridNets '04 and
    SC'04). Do not fabricate an estimation formula here.

## State variables a sans-IO FileCC engine would hold

- **Phase flag**: `SlowStart` / `CongestionAvoidance` (rule 4-5; transitions
  are one-way, slow start runs exactly once).
- `CWND_SIZE` (packets) — init 16 (rule 7); incremented by ACK
  sequence-number delta during slow start (Step 3); recomputed directly
  from `RECEIVING_RATE`/`RTT`/`RC_INTERVAL` during congestion avoidance
  (Step 3).
- `MAX_CWND_SIZE` (packets) — upper bound, e.g. max receiver buffer size /
  12 MB worth of packets (rule 8).
- `PKT_SND_PERIOD` (microseconds) — fixed at 1 µs during slow start (rule
  6); computed per-phase thereafter.
- `LastRCTime` (microseconds) — last rate-control update time, gates the
  `RC_INTERVAL` (`SYN` = 10 ms) throttle on both ACK-handling paths.
- `LAST_ACK_SEQNO` — last acknowledged data-packet sequence number (slow
  start Step 4).
- `bLoss` (bool) — loss-since-last-increase flag, init False (rule 12).
- `LastDecPeriod` (microseconds) — `PKT_SND_PERIOD` value at last decrease,
  init 1 µs (item under Step 5).
- `LastDecSeq` — largest sent sequence number at the last decrease /
  congestion-period boundary.
- `AvgNAKNum`, `NAKCount`, `DecCount`, `DecRandom` — congestion-period NAK
  bookkeeping, all init 0 (`DecRandom` computed per-period, not a fixed
  init).
- `RECEIVING_RATE` (packets/sec) and `EST_LINK_CAPACITY` (packets/sec) —
  receiver-reported-and-EWMA-smoothed-at-sender inputs (§5.2.1.3; weight
  unspecified, rule 35).
- `RTT` (microseconds) — shared with ARQ/LiveCC's RTT estimator
  (`specs/rules/srt-arq.md`).
- `MAX_BW` (bytes/sec, `MAXBW_SET` mode only for file transfer) and `S`
  (SRT packet size / IP payload bytes, standard 1500) — used only in Step 6
  clamp and Step 5's `inc` formula respectively.

## Fidelity check — every constant/formula verified against source

| Item | Value / formula | Source line(s) |
|---|---|---|
| Alternative CC algorithms named, not restated | CUBIC [RFC8312], BBR [BBR] | L3286 |
| FileCC base algorithm | modified UDT native CC [GuAnAO], [GHG04b] | L3291-3293 |
| AIMD classification | "hybrid Additive Increase Multiplicative Decrease (AIMD)" | L3294-3295 |
| CWND_SIZE / PKT_SND_PERIOD units | packets / microseconds | L3296-3298 |
| Rate increase/decrease triggers | ACK increases, NAK decreases, full ACK only | L3309-3314 |
| Slow start stop conditions | loss, CWND_SIZE max, or timeout | L3323-3326 |
| Three sender events (shared w/ LiveCC) | send data packet / ACK / timeout | L3331-3334 |
| Slow start PKT_SND_PERIOD | fixed at 1 microsecond | L3338-3340 |
| Slow start CWND_SIZE initial value | 16 packets | L3340-3341 |
| MAX_CWND_SIZE threshold suggestion | "maximum receiver buffer size (12 MB)" | L3344-3345 |
| Slow-start ACK Step 1 gate | `if (currTime - LastRCTime < RC_INTERVAL) { keep; stop; }` | L3365-3371 |
| Slow-start ACK Step 3 | `CWND_SIZE += ACK_SEQNO - LAST_ACK_SEQNO` | L3385 |
| Slow-start ACK Step 4 | `LAST_ACK_SEQNO = ACK_SEQNO` | L3390 |
| Slow-start ACK Step 5 (end condition + formula) | `PKT_SND_PERIOD = 1000000/RECEIVING_RATE` else `CWND_SIZE/(RTT+RC_INTERVAL)` | L3396-3401 |
| RC_INTERVAL = SYN definition | "RC_INTERVAL of SRT is SYN ... 0.01 second ... maximum and default ACK time interval is SYN" | L3421-3425 |
| NAK during slow start | ends slow start, PKT_SND_PERIOD per Step 5 | L3429-3432 |
| RTO during slow start | ends slow start, PKT_SND_PERIOD per Step 5 | L3436-3439 |
| CA ACK Step 1 gate | identical to slow-start Step 1 | L3455-3461 |
| CA ACK Step 3 CWND_SIZE | `CWND_SIZE = RECEIVING_RATE*(RTT+RC_INTERVAL)/1000000 + 16` | L3481 |
| CA ACK Step 4 bLoss handling | keep period, bLoss=False, stop | L3483-3490 |
| bLoss initial value | False | L3492-3493 |
| CA ACK Step 5 code block | `inc`/`lossBandwidth`/`linkCapacity`/`B`/`PKT_SND_PERIOD` formula | L3498-3517 |
| LastDecPeriod initial value | 1 microsecond | L3521-3524 |
| S definition | "SRT packet size (in terms of IP payload)...1500 bytes as a standard packet size" | L3539-3540 |
| GuAnAO deferred (rate-increase formula derivation) | "detailed explanation...can be found in [GuAnAO]" | L3542-3543 |
| UDT bandwidth-estimate modification note | loss-moment bandwidth registered vs receiver overestimate | L3544-3547 |
| CA ACK Step 6 MAX_BW clamp | `MIN_PERIOD = 1000000/(MAX_BW/S)`; clamp if `PKT_SND_PERIOD < MIN_PERIOD` | L3553-3559 |
| File-transfer MAX_BW mode restriction | only MAXBW_SET applicable; no default | L3561-3566 |
| NAK Step 1 | `bLoss = True` | L3570 |
| NAK Step 2 loss-ratio tolerance | <2% loss ratio -> keep rate, `LastDecPeriod = PKT_SND_PERIOD`, stop | L3572-3579 |
| NAK Step 2 rationale | tolerance to random public-network loss | L3591-3593 |
| NAK Step 3 rate increase | `PKT_SND_PERIOD = 1.03 * PKT_SND_PERIOD` | L3604 |
| NAK Step 3 AvgNAKNum update | `AvgNAKNum = 0.97*AvgNAKNum + 0.03*NAKCount` | L3608 |
| NAK Step 3 resets | NAKCount=1, DecCount=1 | L3610 |
| DecRandom definition | random uniform in [1, AvgNAKNum], clamp <1 to 1 | L3614-3615 |
| AvgNAKNum/NAKCount/DecCount/DecRandom defs | verbatim quotes | L3621-3633 |
| Congestion period definition | verbatim quote | L3645-3647 |
| Coefficient-tuning note | "slightly modified to reduce the amount by which the sending rate decreases" | L3649-3650 |
| NAK Step 4 repeat-decrease condition/formula | `DecCount<=5 && NAKCount==DecCount*DecRandom` -> `SND = 1.03*SND` | L3652-3654 |
| NAK Step 4 counters | DecCount+=1, NAKCount+=1, record LastDecSeq | L3656-3658 |
| Estimation location | receiver-side, during file transmission | L3662-3664 |
| RECEIVING_RATE usage scope | available always, consumed only in slow start | L3664-3669 |
| EST_LINK_CAPACITY usage scope | continuous, used for sending-rate adjustment throughout | L3669-3672 |
| Measurement basis | inter-packet-arrival time delta | L3674-3676 |
| Transport of estimates | via ACK, "sent every 10 milliseconds" | L3676-3678 |
| Sender-side smoothing | EWMA applied on new value receipt (weight unspecified) | L3678-3681 |
| Data-probing vs all-data distinction | bandwidth: probing packets only; receiving speed: all data packets | L3683-3689 |
| Packet-pairs technique gloss | verbatim quote | L3686-3689 |
| Estimation models deferred | "can be found in [GuAnAO], [GHG04b]" | L3691-3692 |
| §5.2 boundary (not included) | "6. Encryption" heading | L3701 |

## Gaps / external references flagged (not fabricated)

- **CUBIC [RFC8312] / BBR [BBR]** (L3286): named as alternative applicable
  CC algorithms for file transfer; neither is described in this draft.
- **[GuAnAO]** (Gu, Hong, Grossman, "An Analysis of AIMD Algorithm with
  Decreasing Increases", GridNets '04, Oct 2004) is cited twice (L3543,
  L3692) as the source of the derivation behind the Step 5 `inc`/`B`
  rate-increase formula and the link-capacity/receiving-rate estimation
  models. The formulas as used by SRT (the code blocks) are transcribed
  verbatim above; their underlying derivation/proof is not in this draft
  and is not reconstructed here.
- **[GHG04b]** (Gu, Hong, Grossman, "Experiences in Design and
  Implementation of a High Performance Transport Protocol", SC'04, Dec
  2004) — the original UDT paper — is cited alongside [GuAnAO] for both the
  base FileCC algorithm (L3292) and the estimation models (L3692).
- **EWMA weight for `RECEIVING_RATE`/`EST_LINK_CAPACITY`** (L3678-3681): the
  draft states EWMA smoothing is applied but never gives the weighting
  coefficient (unlike the explicit 7/8+1/8 weights given elsewhere in the
  draft for `AvgPayloadSize` and `RTT`/`RTTVar`). Implementation-defined per
  draft — do not assume it matches those other 7/8+1/8 weights.
  Left as a genuine gap here rather than invented.
- **`SND` vs `PKT_SND_PERIOD` naming** (L3654): NAK-handling Step 4 writes
  `SND = 1.03 * SND`, using a variable name (`SND`) that appears nowhere
  else in this section — almost certainly meant to be `PKT_SND_PERIOD`
  (mirroring Step 3's `PKT_SND_PERIOD = 1.03 * PKT_SND_PERIOD`, L3604), but
  transcribed exactly as written per the no-invention rule; an
  implementation should treat this as `PKT_SND_PERIOD` but the draft itself
  does not confirm the equivalence.
- **Packet-pairs probing mechanics** (L3686-3689): "data probing" packet
  pairs are named and glossed in one sentence, but sending frequency,
  pair-spacing, and packet marking are not specified in this draft —
  implementation-defined per draft.
