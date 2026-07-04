# SRT (draft-sharabayko-srt-01) — TSBPD / Too-Late Packet Drop rules

Curated behavioural + timing rules for `srt-runtime`'s receive-side delivery
scheduling. Source: `specs/ietf_draft_sharabayko_srt_01.txt`:

- §4.5 "Timestamp-Based Packet Delivery" (incl. §4.5.1 "Packet Delivery
  Time" and §4.5.1.1 "TSBPD Time Base Calculation") — **L2469-2651**.
- §4.6 "Too-Late Packet Drop" — **L2652-2810**, which (per the assigned
  extraction range) also captures the first part of §4.7 "Drift
  Management" (L2749-2810) — included here because §4.5.1's `Drift` term
  is defined by §4.7, so the two are inseparable for a delivery-timing
  implementation. The rest of §4.7 (beyond L2810) is not covered.

ACK/NAK/ACKACK behaviour and RTT/RTTVar estimation live in
`specs/rules/srt-arq.md`; wire-level control-packet field layouts live in
`specs/rules/srt-rules.md`.

## §4.5 Timestamp-Based Packet Delivery — overview (L2469-2564)

1. **Goal**: reproduce the sending application's output timing at the
   receiving application's input for live streaming, by scheduling
   received packets for delivery rather than delivering them immediately
   (L2471-2478).
2. The receiver adjusts the sender's packet timestamp to receiver-local
   time (compensating clock drift/timezone) before releasing a packet to
   the application; packets are withheld for a configured receiver delay.
   A higher delay tolerates a larger uniform loss rate or larger burst
   loss (L2480-2488).
3. Packets received after their "play time" are dropped if Too-Late
   Packet Drop (§4.6) is enabled (L2488-2490).
4. The packet timestamp is in microseconds, relative to SRT connection
   creation time; packets are inserted by sequence number. The origin
   time is sampled when the application first submits the packet to the
   sender (unless explicitly provided), and TSBPD uses this same origin
   timestamp for both the first transmission and any retransmission
   (L2496-2502).
5. **Why not "send time"**: using the packet's *send* time (rather than
   its original submission/origin time) to stamp packets would be wrong
   for TSBPD, because a retransmission gets a new (later) send time,
   which would place it out of order relative to its proper position in
   the stream (L2508-2512).
6. **End-to-end latency identity**: once the handshake completes, the
   actual end-to-end link latency becomes fixed and is *approximately*
   equal to `RTT_0/2 + SRT Latency`, where `RTT_0` is the RTT measured
   during the handshake exchange, even though the live RTT may vary over
   time afterward (L2554-2560, verbatim relation: "approximately equal to
   (RTT_0/2 + SRT Latency)").
7. Sending delay (hardware-dependent, "several microseconds") is small
   relative to `RTT_0/2` and SRT latency, both measured in milliseconds
   (L2562-2564).

### Figure 20 — key latency points (L2525-2540, reproduced verbatim)

```
                 |  Sending  |              |                   |
                 |   Delay   |    ~RTT/2    |    SRT Latency    |
                 |<--------->|<------------>|<----------------->|
                 |           |              |                   |
                 |           |              |                   |
                 |           |              |                   |
       ___ Scheduled       Sent         Received           Scheduled
      /    for sending       |              |              for delivery
   Packet        |           |              |                   |
   State         |           |              |                   |
                 |           |              |                   |
                 |           |              |                   |
                 ----------------------------------------------------->
                                                                   Time

        Figure 20: Key latency points during the packet transmission
```

8. The four packet states in Figure 20 (L2542-2552):
   - **Scheduled for sending**: committed by the sending application,
     stamped, ready to send.
   - **Sent**: passed to the UDP socket and sent.
   - **Received**: received and read from the UDP socket.
   - **Scheduled for delivery**: scheduled and ready to be read by the
     receiving application.

## §4.5.1 Packet Delivery Time (L2566-2608)

9. **PktTsbpdTime formula** (verbatim, L2581):
   > PktTsbpdTime = TsbpdTimeBase + PKT_TIMESTAMP + TsbpdDelay + Drift

   Computed on receiving each data packet. Term definitions (L2583-2597):
   - `TsbpdTimeBase` — time base reflecting the clock difference between
     receiver-local clock and the sender's packet-timestamping clock (see
     §4.5.1.1 below).
   - `PKT_TIMESTAMP` — the data packet's timestamp, in microseconds.
   - `TsbpdDelay` — receiver's buffer delay / buffer latency / "SRT
     Latency": how long, in milliseconds, SRT holds a packet from receipt
     until it should be delivered upstream.
   - `Drift` — time drift correction between sender/receiver clocks, in
     microseconds (mechanism detailed in §4.7, see below).
10. **TsbpdDelay minimum and recommendation** (verbatim, L2601-2603):
    > The value of minimum TsbpdDelay is negotiated during the SRT
    > handshake exchange and is equal to 120 milliseconds. The
    > recommended value of TsbpdDelay is 3-4 times RTT.

    i.e. `TsbpdDelay_min = 120 ms`; `TsbpdDelay_recommended ≈ 3×RTT` to
    `4×RTT`.
11. TsbpdDelay's practical purpose is bounding (not eliminating) the
    number of retransmissions possible before a packet's play deadline —
    it is not unlimited retry (L2605-2608).

## §4.5.1.1 TSBPD Time Base Calculation (L2610-2651)

12. **Initial TsbpdTimeBase formula** (verbatim, L2615):
    > TsbpdTimeBase = T_NOW - HSREQ_TIMESTAMP

    Calculated at the moment the second handshake request is received;
    `T_NOW` = current receiver-clock time, `HSREQ_TIMESTAMP` = the
    handshake packet's timestamp, in microseconds (L2612-2618).
13. This initial `TsbpdTimeBase` value approximates the initial one-way
    delay `RTT_0/2` (handshake-time RTT) (L2620-2622).
14. `TsbpdTimeBase` is adjusted during the transmission in exactly two
    cases (L2624-2626): the wrapping period (rule 15) and the drift
    tracer (§4.7, rule 20+).
15. **TSBPD wrapping period** (verbatim, L2637-2644):
    > During the TSBPD wrapping period. The TSBPD wrapping period happens
    > every 01:11:35 hours. This time corresponds to the maximum
    > timestamp value of a packet (MAX_TIMESTAMP). MAX_TIMESTAMP is equal
    > to 0xFFFFFFFF, or the maximum value of 32-bit unsigned integer, in
    > microseconds (Section 3). The TSBPD wrapping period starts 30
    > seconds before reaching the maximum timestamp value of a packet and
    > ends once the packet with timestamp within (30, 60) seconds interval
    > is delivered (read from the buffer).

    Concretely: `MAX_TIMESTAMP = 0xFFFFFFFF` µs (the 32-bit packet
    timestamp field's max value, cross-ref §3 header, `srt-rules.md`);
    the wrap period recurs every **01:11:35** (hh:mm:ss); it begins 30 s
    before the 32-bit timestamp counter would reach `MAX_TIMESTAMP`, and
    ends once a packet whose timestamp falls in the `(30, 60)` second
    window past the wrap point has been delivered.
16. **TsbpdTimeBase wrap update formula** (verbatim, L2648):
    > TsbpdTimeBase = TsbpdTimeBase + MAX_TIMESTAMP + 1

    i.e. `TsbpdTimeBase_new = TsbpdTimeBase_old + 0xFFFFFFFF + 1` (adds
    one full 32-bit timestamp-space wraparound, `2^32` µs).

## §4.6 Too-Late Packet Drop (TLPKTDROP) (L2652-2748)

17. TLPKTDROP lets the sender drop packets with no chance of in-time
    delivery, and lets the receiver skip missing packets not delivered in
    time. The drop timeout derives from the TSBPD mechanism (§4.5)
    (L2654-2658).
18. **Drop condition (sender side)**: when TLPKTDROP is enabled, a packet
    is "too late" and may be dropped by the sender if its timestamp is
    older than `TLPKTDROP_THRESHOLD` (L2660-2662).
19. **TLPKTDROP_THRESHOLD relation and recommended value** (verbatim,
    L2664-2670):
    > TLPKTDROP_THRESHOLD is related to SRT latency (Section 4.4). For
    > the Too-Late Packet Drop mechanism to function effectively, it is
    > recommended that a value higher than the SRT latency is used. ...
    > The recommended threshold value is 1.25 times the SRT latency
    > value.

    i.e. `TLPKTDROP_THRESHOLD_recommended = 1.25 × SRT_latency`. Rationale
    (L2665-2669): this ordering lets the receiver drop missing packets
    first, while the sender only drops as a fallback when the peer isn't
    responding in time (e.g. severe congestion).
20. **Minimum sender retention floor** (verbatim, L2672-2674):
    > Note that the SRT sender keeps packets for at least 1 second in
    > case the latency is not high enough for a large RTT (that is, if
    > TLPKTDROP_THRESHOLD is less than 1 second).

    i.e. sender-side retention is `max(TLPKTDROP_THRESHOLD, 1 second)`.
21. **Receiver behaviour when enabled**: drops packets not delivered or
    retransmitted in time, then delivers subsequent packets at their
    correct play time (L2676-2678).

### Receiver buffer read pseudocode (Figure, verbatim, L2693-2715)

```
<CODE BEGINS>
pos = 0;  /* Current receiver buffer position */
i = 0;    /* Position of the next available in the receiver buffer
             packet relatively to the current buffer position pos */

while(True) {
    // Get the position i of the next available packet
    // in the receiver buffer
    i = next_avail();
    // Calculate packet delivery time PktTsbpdTime
    // for the next available packet
    PktTsbpdTime = delivery_time(i);

    if T_NOW < PktTsbpdTime:
        continue;

    Drop packets which buffer position number is less than i;

    Deliver packet with the buffer position i;

    pos = i + 1;
}
<CODE ENDS>
```

`T_NOW` is the current time per the receiver clock (L2717). `PktTsbpdTime`
here is the rule-9 formula's result for position `i`.

22. **Fake ACK on receiver skip**: when the receiver skips an
    undelivered-in-time packet, it sends a fake ACK as if the packet had
    arrived — cross-ref `srt-arq.md` rule 13, same mechanism described
    from the ARQ side (L2719-2727).
23. TLPKTDROP can be disabled entirely for guaranteed clean delivery, at
    the cost of a lost packet potentially pausing delivery for an
    unbounded time (worse tearing for the player); raising SRT latency is
    the recommended mitigation if TLPKTDROP triggers too often
    (L2729-2733).

## §4.7 Drift Management — context for the `Drift` term (L2749-2810)

Included because §4.5.1's `PktTsbpdTime` formula (rule 9) has a `Drift`
term whose mechanism is defined here; the rest of §4.7 beyond L2810 is out
of scope for this note.

24. Rationale: synchronized time is needed to keep proper sender/receiver
    buffer levels despite timezone differences and RTT (up to 2 seconds
    for satellite links); rounding and unsynchronized system clocks cause
    the agreed time base to drift by a few microseconds per minute, which
    can accumulate over days to the point of buffer overflow/underflow.
    SRT's time management mechanism compensates for this (L2757-2765).
25. On packet receipt, SRT computes the difference between the packet's
    expected arrival time and its timestamp; RTT tells the receiver how
    long transit was "supposed" to take. SRT maintains a reference
    between the send buffer's latency-window leading edge and the
    corresponding receiver-side time, converting packet timestamps to
    local receiver time so events (e.g. delivery) can be scheduled
    (L2767-2774).
26. The receiver periodically samples time-drift data and calculates a
    packet-timestamp correction factor, applied to each received data
    packet by adjusting the inter-packet interval. A received packet is
    not immediately handed to the application; as time advances, the
    receiver can predict the expected arrival time of a missing/dropped
    packet to fill queue "holes" (cross-ref §4.5) (L2776-2782).
27. Drift-sampling period is based on **packet count, not time duration**
    — ensuring enough samples independent of the stream's packet rate;
    using a large sample count attenuates the effect of network jitter on
    the drift estimate. Actual drift is very slow (affects a stream only
    after many hours), so no fast reaction is required (L2784-2790).
28. Receiver-local time is used to schedule events (e.g. "is it time to
    deliver this packet"); in-packet timestamps are references relative
    to session start. On receipt, the receiver recalculates relative to
    session start; session start time derives from receiver-local time
    when the session connected. A packet timestamp equals `"now" -
    "StartTime"`, where `StartTime` is when the socket was created
    (L2792-2809, formula paraphrased from prose — the draft states this
    relation in words, not as a named symbolic formula, at L2807-2809).

## Control packets involved

None directly wire-framed in this scope; TLPKTDROP interacts with ACK via
the fake-ACK mechanism (rule 22 / `srt-arq.md` rule 13) and with the
Message Drop Request control packet (§3.2.9, field layout in
`srt-rules.md`) for the "both peers agree to drop this packet" case
referenced at `srt-arq.md` rule 6.

## State variables a sans-IO Receiver would hold

- `TsbpdTimeBase` (µs) — seeded per rule 12, periodically adjusted per
  the wrap-period formula (rule 16) and by the drift tracer (rule 26).
- `TsbpdDelay` (ms) — negotiated at handshake, floor 120 ms, recommended
  3-4× RTT (rule 10).
- `Drift` (µs) — current drift correction, recomputed periodically from a
  packet-count-based sample window (rule 26-27).
- Per-packet `PktTsbpdTime` derived on arrival via the rule-9 formula;
  used by the receiver-buffer read loop (pseudocode above) to decide
  deliver-vs-continue-waiting and to drop stale buffer positions.
- Wrap-period state: whether currently inside the 30s-before/until-30-60s-
  after window (rule 15), to gate the wrap-update formula (rule 16).
- `TLPKTDROP_THRESHOLD` (recommended `1.25 × SRT_latency`, rule 19) and
  enabled/disabled flag (rule 23).
- Session `StartTime` (receiver-local clock at socket creation, rule 28).
- Skipped-packet statistics counter (rule 22, shared with `srt-arq.md`
  rule 13).

## State variables a sans-IO Sender would hold

- `TLPKTDROP_THRESHOLD` and enabled/disabled flag (rule 18-19), and the
  effective retention floor `max(TLPKTDROP_THRESHOLD, 1 second)` (rule
  20), used to decide when a buffered packet may be dropped without ever
  having been delivered.

## Ambiguous / implementation-defined points

- The wrap-period boundary condition "(30, 60) seconds interval" (L2644)
  is stated as an open interval in the prose but the draft does not
  specify inclusive/exclusive endpoints precisely — implementation-defined
  per draft (L2642-2644).
- §4.7's closing "packet timestamp equals `now - StartTime`" relation
  (L2807-2809) is prose, not presented as a named formula like
  `PktTsbpdTime` or `TsbpdTimeBase` — transcribed here paraphrased with a
  direct line cite rather than as a verbatim block quote, since the
  draft itself does not set it off as a formula.
- The exact drift-tracer sample-count threshold and correction-factor
  computation are described qualitatively ("based on a number of
  packets", L2784-2786) with no numeric value given — implementation-
  defined per draft, not invented here.
