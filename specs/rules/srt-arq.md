# SRT (draft-sharabayko-srt-01) — ARQ / ACK / NAK reliability rules

Curated behavioural rules for `srt-runtime`'s retransmission (ARQ) logic.
Source: `specs/ietf_draft_sharabayko_srt_01.txt`, §4.8 "Acknowledgement and
Lost Packet Handling" (incl. §4.8.1 "Packet Acknowledgement (ACKs, ACKACKs)"
and §4.8.2 "Packet Retransmission (NAKs)"), §4.9 "Bidirectional Transmission
Queues", and §4.10 "Round-Trip Time Estimation" — **L2811-3065**.

This is *behaviour* (when/why control packets are sent, how loss is tracked,
how RTT is estimated); wire-level ACK/NAK/ACKACK field layouts already live in
`specs/rules/srt-rules.md` (§3.2.4 ACK, §3.2.5 NAK, §3.2.8 ACKACK,
Appendix A loss-list coding) and are cross-referenced, not duplicated, below.
TSBPD/too-late-drop timing lives in `specs/rules/srt-tsbpd.md`. Congestion
control (§5, incl. the FileCC-specific RTO/NAKInterval restatement at L3270,
L3276) is out of scope for this note.

## §4.8 Acknowledgement and Lost Packet Handling (L2811-2842)

1. The sender buffers every sent data packet to enable retransmission
   (ARQ) (L2813-2815).
2. The receiver periodically sends ACKs for received data packets so the
   sender can free acknowledged packets from its buffer; once removed, a
   packet can no longer be retransmitted (L2816-2820).
3. On receiving a **full** ACK, the sender SHOULD reply with an ACKACK
   carrying that ACK's sequence number (L2822-2825).
4. The receiver also sends NAK control packets to report missing packets,
   either immediately on detecting a sequence-number gap, or periodically
   via the Periodic NAK report mechanism, which lists *all* packets the
   receiver currently considers lost (L2827-2834).
5. On receiving a NAK, the sender prioritizes retransmitting lost packets
   over first-time transmission of regular data (L2836-2838).
6. Retransmission of a missing packet repeats until the receiver
   acknowledges it, or both peers agree to drop it (Too-Late Packet Drop,
   §4.6) (L2840-2842).

## §4.8.1 Packet Acknowledgement — ACKs, ACKACKs (L2844-2892)

7. An ACK causes acknowledged packets to be removed from the sender's
   buffer (L2846-2848).
8. **ACK sequence-number semantics**: an ACK carries the sequence number of
   the packet immediately following the latest packet in the received
   list. If no loss has occurred up to sequence number `n`, the ACK
   contains `n + 1` (L2861-2864).
9. Receiving an ACK triggers an ACKACK from the sender with almost no
   delay; the ACK→ACKACK round trip **is** the RTT measurement
   (L2866-2868, cross-ref §4.10 below).
10. The ACKACK tells the receiver to stop re-sending that ACK position
    (the sender already has it) — otherwise the receiver would keep
    resending outdated ACKs. Symmetrically, if the sender never receives
    an ACK, it does not stop transmitting (L2868-2872).
11. **Full ACK timer**: a full ACK is sent on a timer of **10 milliseconds**
    — "the ACK period or synchronization time interval SYN" (L2874-2876,
    quoted verbatim).
12. **Light ACK threshold**: for high-bitrate transmissions, if **64
    packets** have been sent/received within the 10 ms ACK period (even if
    the period hasn't elapsed), the receiver sends a **light ACK** — a
    short ACK covering a sequence of packets (SRT header + one 32-bit
    field), which does **not** trigger an ACKACK (L2877-2883).
13. **Fake ACK on skip**: when TLPKTDROP causes the receiver to skip an
    undelivered packet (§4.6), the receiver sends a "fake ACK" as if the
    packet had been received; to the sender this is indistinguishable from
    a real ACK, and the skip remains unknown to the sender. Skipped
    packets SHOULD be recorded in receiver statistics (L2885-2892).

## §4.8.2 Packet Retransmission — NAKs (L2894-2982)

14. NAK sending can be triggered immediately on detecting a
    sequence-number gap (L2896-2899, restates rule 4).
15. On NAK reception the sender prioritizes loss-list retransmission over
    first-time sends (L2901-2903, restates rule 5).
16. **Loss list**: the sender maintains a loss list built from NAK
    reports. When scheduling a transmission, the sender checks the loss
    list first and sends a prioritized entry if present; otherwise it
    sends the next packet from the first-transmission queue. A
    transmitted packet stays in the send buffer in case the receiver
    still hasn't received it (L2917-2922).
17. As the latency window advances and packets are dropped from the send
    queue, the sender checks whether any dropped/resent packets are still
    in the loss list and removes them there too, to avoid needless
    retransmission (L2924-2928).
18. **Resend counter**: a per-packet counter tracks retransmissions. A
    packet with no ACK stays in the loss list and can be resent more than
    once; loss-list packets are prioritized (L2930-2932, cross-ref rule
    16).
19. **Send-queue overflow**: if loss-list packets keep blocking the send
    queue, the queue can fill; once full, the sender drops packets
    *without ever sending them the first time* (new application data has
    nowhere to go and is discarded) (L2934-2939).
20. This unsent-packet condition is rare in practice: a maximum
    send-buffer packet count derives from the configured latency, and
    older packets with no chance of in-time retransmission/play are
    dropped to make room for newer real-time packets (§4.5, §4.6)
    (L2941-2946).
21. **Periodic NAK reports** list every packet the receiver currently
    considers lost, at the moment the periodic report is sent
    (L2948-2951, restates rule 4's periodic branch).
22. **NAK period formula** — SRT Periodic NAK reports are sent with period
    (verbatim, L2953-2955):
    > SRT Periodic NAK reports are sent with a period of (RTT + 4 *
    > RTTVar) / 2 (so called NAKInterval), with a 20 milliseconds floor,
    > where RTT and RTTVar are defined in Section 4.10.

    i.e. `NAKInterval = max((RTT + 4*RTTVar) / 2, 20ms)`. A NAK carries a
    *compressed* list of lost packets, so only lost packets are
    retransmitted; using NAKInterval for the period can cause a lost
    packet to be retransmitted more than once, which is accepted to keep
    latency low when NAK packets themselves are lost (L2955-2960).
    - **Cross-reference (out of scope, congestion-control section)**: §5.2
      FileCC restates this with explicit microsecond units and an
      explicit `max()`: `NAKInterval = max((RTT + 4 * RTTVar) / 2, 20000)`
      (L3276), footnoted "The minimum value of NAKInterval is set to 20
      milliseconds in order to avoid sending periodic NAK reports too
      often under low latency conditions" (L3279-3281). Confirms RTT/RTTVar
      here are in microseconds and the floor is 20 ms = 20000 µs. The
      companion RTO formula in the same section, `RTO = RexmitCount *
      (RTT + 4 * RTTVar + 2 * SYN) + SYN` (L3270), is congestion-control
      scope and not curated here.
23. ACKACK restated: tells the receiver to stop sending the ACK position
    since the sender already knows it (L2962-2964, duplicate of rule 10).
24. **RTT-via-ACK/ACKACK, restated**: an ACK acts as a ping, the matching
    ACKACK as the pong; the ACK→ACKACK round trip is the RTT. Every ACK
    has a number; the matching ACKACK carries the same number. The
    receiver queues outstanding ACKs to match against incoming ACKACKs. A
    light ACK carries only the sequence number (no RTT/CIF payload,
    cross-ref `srt-rules.md` §3.2.4 Small/Light ACK variants); ACKACK
    processing time is treated as negligible / folded into the RTT
    measurement (L2973-2982).

## §4.9 Bidirectional Transmission Queues (L2984-2987)

25. Once an SRT connection is established, both peers may send data
    packets simultaneously (L2986-2987) — i.e. ACK/NAK/loss-list state
    above is symmetric per-direction, not sender-vs-receiver-role-fixed.

## §4.10 Round-Trip Time Estimation (L2989-3047)

26. RTT is estimated from the time difference between an ACK being sent
    and its ACKACK being received back at the receiver (L2991-2994).
27. The ACKACK is expected back roughly one RTT after the ACK was sent
    (minimal sender-side processing delay) (L2996-2998).
28. The receiver records ACK send time; the ACK carries a unique sequence
    number (independent of data-packet sequence numbers), echoed by the
    matching ACKACK; RTT = ACKACK arrival time − ACK departure time
    (L3000-3004).
29. **RTT EWMA formula** (verbatim, L3009):
    > RTT = 7/8 * RTT + 1/8 * rtt

    where `RTT` is the receiver's maintained current value and `rtt` is
    the just-measured sample from one ACK/ACKACK pair.
30. **RTTVar EWMA formula** (verbatim, L3011-3013):
    > RTTVar = 3/4 * RTTVar + 1/4 * abs(RTT - rtt)

    (`abs()` = absolute value, per the draft's own gloss at L3015.)
31. **Units and initial values** (verbatim, L3017-3018): "Both RTT and
    RTTVar are measured in microseconds. The initial value of RTT is 100
    milliseconds, RTTVar is 50 milliseconds."
32. RTT/RTTVar as calculated by the receiver are carried in the next full
    ACK (§3.2.4); the very first ACK of a session may still carry the
    initial 100 ms RTT value since early samples aren't yet precise
    (L3029-3033).
33. The sender has no ACK/ACKACK-equivalent ping of its own — it always
    derives RTT from the receiver's reported value. On receiving an ACK,
    the sender updates its own RTT/RTTVar using the *same* formulas
    (rules 29-30), with `rtt` = the value just carried by that ACK
    (L3035-3040).
34. A single SRT socket can be both sender and receiver; its RTT/RTTVar
    state is updated by both roles' algorithms (sender: from ACKs;
    receiver: from ACK/ACKACK pairs), and receiving data updates the
    local RTT/RTTVar usable by that socket's own sender path too
    (L3042-3046).

## Control packets involved

ACK, ACKACK, NAK — field layouts already curated in `specs/rules/srt-rules.md`
(§3.2.4 ACK incl. Full/Small/Light variants, §3.2.5 NAK incl. Appendix A
single/range loss-list coding, §3.2.8 ACKACK). This document only adds *when*
and *why* those packets are sent/consumed and how loss/RTT state evolves.

## State variables a sans-IO Sender would hold

- Send buffer of unacknowledged data packets (keyed by sequence number),
  each with a resend counter (rules 1, 16, 18).
- Loss list (sequence numbers/ranges reported lost via NAK), prioritized
  over first-transmission scheduling (rules 5, 15, 16, 18).
- Send-queue capacity / overflow state, and the max send-buffer size
  derived from configured latency (rules 19-20; latency math itself is
  §4.4/§4.5, see `srt-tsbpd.md`).
- `RTT`, `RTTVar` (microseconds; init 100 ms / 50 ms — rule 31), updated
  from each incoming ACK's reported RTT/RTTVar using the rule-29/30
  formulas with the ACK's value as `rtt` (rule 33).
- Outstanding-ACK number tracking, to emit ACKACK on full-ACK receipt
  (rule 3).

## State variables a sans-IO Receiver would hold

- Received-packet tracking sufficient to compute the next-expected
  sequence number for ACK's `n + 1` semantics (rule 8).
- Full-ACK timer (10 ms period, rule 11) and light-ACK packet counter
  (64-packet threshold, rule 12).
- Loss list built from detected sequence-number gaps, for on-demand and
  periodic NAK generation (rules 4, 14, 21).
- Periodic-NAK timer, period = `NAKInterval = max((RTT + 4*RTTVar)/2,
  20 ms)` (rule 22), using this socket's own RTT/RTTVar estimates.
- Outstanding-ACK queue (sequence number → send time) to match ACKACKs
  and compute RTT samples (rules 24, 26-28).
- `RTT`, `RTTVar` (microseconds; init 100 ms / 50 ms — rule 31), updated
  per rules 29-30 on each ACKACK receipt.
- Skipped-packet statistics counter for TLPKTDROP fake-ACK cases (rule
  13, cross-ref `srt-tsbpd.md`).

## Ambiguous / implementation-defined points

- The draft states RTT/RTTVar are "measured in microseconds" (L3017) yet
  gives initial values in milliseconds (100 ms / 50 ms, same line) and the
  NAKInterval floor as "20 milliseconds" in §4.8.2 (L2954) vs `20000` bare
  in the §5.2 FileCC restatement (L3276, implicitly microseconds there).
  The unit conversion (ms → µs) at the point of use is not spelled out in
  §4.8/§4.10 itself — implementation-defined per draft, resolved by
  cross-referencing §5.2's explicit `20000` (L3276).
- Exact ACK-loop / NAK-loop scheduling interaction (e.g. what happens if a
  light-ACK boundary and the 10 ms full-ACK timer coincide) is described
  narratively (L2877-2883) but no pseudocode is given — implementation-
  defined per draft.
