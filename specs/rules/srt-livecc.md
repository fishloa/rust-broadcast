# SRT (draft-sharabayko-srt-01) — §5.1 Packet Pacing / LiveCC rules

Curated pacing/congestion-control formulas for a LiveCC implementation.
Source: `specs/ietf_draft_sharabayko_srt_01.txt`, §5.1 "SRT Packet Pacing and
Live Congestion Control (LiveCC)", **L3066-3282** (§5.1 heading through the
line immediately before §5.2 "File Transfer Congestion Control (FileCC)"
begins at L3283). Line cites below are exact source lines. One constant
(`SYN`) is defined in §5.2.1 (L3421-3423) but is used by formulas in this
§5.1 section (L3259, L3270), so it is imported here with its own citation.

## Overview (L3068-3114)

- Goal: control the sender's buffer level to avoid overfill/depletion for
  smooth live playback, sending packets as fast as submitted by the video
  application while keeping the buffer level stable (L3068-3072).
- SRT needs bandwidth **overhead** so the sender has room to insert
  retransmissions without materially impacting the main output rate
  (L3085-3087).
- This balance is achieved by adjusting **MAX_BW** (§5.1.1), which the
  LiveCC module uses to compute the minimum inter-packet interval
  **PKT_SND_PERIOD** (L3089-3093). The space between packets is where
  retransmissions get inserted; the overhead is the available margin
  (L3093-3095).
- In live streaming the sender may drop packets that can't be delivered in
  time (§4.6) (L3101-3102).
- Encoder-side fairness: SRT can expose RTT estimate, packet loss level,
  drop counts, etc., to the encoder for real-time bitrate adjustment
  (L3109-3114).

## §5.1.1 Configuring Maximum Bandwidth (L3116-3202)

Three configuration modes for **MAX_BW**:

1. **MAXBW_SET** (L3120-3128): set MAX_BW explicitly.
   - Recommended default: **1 Gbps**, set only for live streaming
     (L3122-3123).
   - Not well-suited to variable input (e.g. changing encoder bitrate);
     MAX_BW must be manually reconfigured alongside the encoder
     (L3125-3128).

2. **INPUTBW_SET** (L3130-3146): set the sender's input rate `INPUT_BW` and
   an `OVERHEAD` percentage. SRT then computes:

   ```
   MAX_BW = INPUT_BW * (1 + OVERHEAD / 100)
   ```
   (L3143, verbatim)

   Reduces to MAXBW_SET's restrictions (L3145-3146) — i.e. still a static
   value until reconfigured.

3. **INPUTBW_ESTIMATED** (L3148-3184): measure the sender's input rate
   internally and set `OVERHEAD`. Each time the internal estimate
   `EST_INPUT_BW` updates, SRT recomputes:

   ```
   MAX_BW = EST_INPUT_BW * (1 + OVERHEAD / 100)
   ```
   (L3154, verbatim)

   - Recommended mode overall since it follows sender input-rate
     fluctuations (L3159-3160), but the moving-average estimate introduces
     delay: black-screen/still-frame content can transiently depress the
     measured bitrate, then under-react when motion resumes, risking
     sender-buffer accumulation, late arrival, and receiver-side drops
     (L3164-3184). This tradeoff is prose-only in the draft — no
     compensating formula is given, so it is noted here rather than
     invented.

- **Units** (L3156-3157): `MAX_BW`, `INPUT_BW`, `EST_INPUT_BW` are in
  **bytes per second**; `OVERHEAD` is in **%**.

**Mode/variable table (L3197-3201, verbatim):**

| Mode / Variable | MAX_BW | INPUT_BW | OVERHEAD |
|---|---|---|---|
| MAXBW_SET | v | - | - |
| INPUTBW_SET | - | v | v |
| INPUTBW_ESTIMATED | - | - | v |

(`v` = set by the user/config; `-` = ignored.)

## §5.1.2 SRT's Default LiveCC Algorithm (L3203-3282)

Goal: adjust the minimum allowed packet sending period **PKT_SND_PERIOD**
(and hence the maximum allowed sending rate) based on average packet payload
size (`AvgPayloadSize`) and `MAX_BW` (L3205-3209).

Three sender-side events drive the algorithm (L3211-3214): (1) sending a
data packet, (2) receiving an ACK, (3) a timeout event.

### (1) On sending a data packet — original or retransmitted (L3216-3223)

Update the average payload size with an EWMA:

```
AvgPayloadSize = 7/8 * AvgPayloadSize + 1/8 * PacketPayloadSize
```
(L3219, verbatim)

- `PacketPayloadSize`: payload size of the just-sent data packet, in bytes
  (L3221-3222).
- **Initial value** of `AvgPayloadSize`: the maximum allowed packet payload
  size, which **cannot be larger than 1456 bytes** (L3222-3223).

### (2) On ACK packet reception (L3225-3238)

Step 1 — compute SRT packet size:

```
PktSize = AvgPayloadSize + <SRT header size (§3)>
```
(L3227-3229, paraphrased — the draft states this as prose: "Calculate SRT
packet size (PktSize) as the sum of average payload size (AvgPayloadSize)
and SRT header size (Section 3), in bytes." The 16-byte fixed SRT header
size is defined in §3, already curated in `specs/rules/srt-rules.md`.)

Step 2 — compute the minimum allowed packet sending period:

```
PKT_SND_PERIOD = PktSize * 1000000 / MAX_BW
```
(L3234, verbatim)

- `MAX_BW`: configured maximum bandwidth (bytes/sec) from §5.1.1
  (L3236-3237).
- `PKT_SND_PERIOD` is in **microseconds** (L3237-3238) — the `1000000`
  factor converts the bytes/(bytes-per-sec) ratio (seconds) to
  microseconds.

### (3) On a retransmission timeout (RTO) event (L3240-3241)

"Follow the same steps as described in method (1) above" (L3240-3241) —
i.e. RTO also triggers the `AvgPayloadSize` EWMA update in (1), no separate
formula.

### RTO definition (L3253-3270)

RTO is the time within which an ACK is expected after a data packet is
sent; if no ACK arrives in that time, a timeout event fires (L3253-3255).
Since SRT only acknowledges every SYN time (§4.8.1) (L3256), RTO is defined
as:

```
RTO = RTT + 4 * RTTVar + 2 * SYN
```
(L3259, verbatim)

- `RTT`: round-trip time estimate, in microseconds, reported by the
  receiver and smoothed at the sender (EWMA — "smoothing means applying an
  exponentially weighted moving average (EWMA)", L3264-3265; see §3.2.4,
  §4.10).
- `RTTVar`: variance of the RTT estimate, in microseconds, same
  reporting/smoothing path.
- `SYN`: the synchronization time interval constant — defined in §5.2.1
  (imported here, see below) as **0.01 second** (L3421-3423).

**Continuous-timeout backoff** (L3267-3270) — a counter `RexmitCount` tracks
consecutive timeouts, and RTO becomes:

```
RTO = RexmitCount * (RTT + 4 * RTTVar + 2 * SYN) + SYN
```
(L3270, verbatim)

### Receiver-side periodic NAK interval (L3272-3281)

When a loss report is sent, the receiver updates its periodic NAK
(§4.8.2) sending interval:

```
NAKInterval = max((RTT + 4 * RTTVar) / 2, 20000)
```
(L3276, verbatim)

- `RTT`/`RTTVar` here are the **receiver's own estimates** (§3.2.4, §4.10)
  (L3278).
- The floor value **20000** is in the same units as the rest of the
  formula's inputs; the draft immediately glosses it in prose as
  "**20 milliseconds**" (L3279-3281: "The minimum value of NAKInterval is
  set to 20 milliseconds in order to avoid sending periodic NAK reports too
  often under low latency conditions.") — i.e. the formula's RTT/RTTVar
  and the `20000` floor are consistently in **microseconds** (matching RTO
  above), and 20000 µs = 20 ms.

## Imported constant: `SYN` (from §5.2.1, L3421-3423)

`SYN` is used by two §5.1 formulas (RTO, and implicitly the NAK-interval
gloss) but is only named/defined in §5.2.1 (FileCC), immediately after this
section:

> "RC_INTERVAL is the fixed rate control interval, in microseconds.
> RC_INTERVAL of SRT is SYN, or synchronization time interval, which is
> 0.01 second. An ACK in SRT is sent every fixed time interval. The maximum
> and default ACK time interval is SYN." (L3421-3424, verbatim)

So: **`SYN` = 0.01 s = 10 ms = 10000 µs**, and doubles as `RC_INTERVAL`
(FileCC's rate-control interval) and the default/maximum ACK interval. This
matches the independent statement elsewhere in the draft that "A Full ACK
control packet is sent every 10 ms" (§3.2.4, L1328) — cited here only for
cross-check, not part of §5.1's own text.

## Fidelity check — every constant/formula verified against source

| Item | Value / formula | Source line(s) |
|---|---|---|
| MAXBW_SET default | 1 Gbps (live streaming only) | L3122-3123 |
| INPUTBW_SET formula | `MAX_BW = INPUT_BW * (1 + OVERHEAD /100)` | L3143 |
| INPUTBW_ESTIMATED formula | `MAX_BW = EST_INPUT_BW * (1 + OVERHEAD /100)` | L3154 |
| Units | MAX_BW/INPUT_BW/EST_INPUT_BW = bytes/sec; OVERHEAD = % | L3156-3157 |
| Mode/variable table | MAXBW_SET: MAX_BW=v; INPUTBW_SET: INPUT_BW=v,OVERHEAD=v; INPUTBW_ESTIMATED: OVERHEAD=v | L3197-3201 |
| AvgPayloadSize EWMA | `AvgPayloadSize = 7/8*AvgPayloadSize + 1/8*PacketPayloadSize` | L3219 |
| AvgPayloadSize initial value cap | max packet payload size, "cannot be larger than 1456 bytes" | L3222-3223 |
| PktSize | `AvgPayloadSize + SRT header size` | L3227-3229 |
| PKT_SND_PERIOD formula | `PKT_SND_PERIOD = PktSize * 1000000 / MAX_BW` (µs) | L3234, L3237-3238 |
| RTO event -> same as event (1) | "follow the same steps as described in method (1) above" | L3240-3241 |
| RTO formula | `RTO = RTT + 4*RTTVar + 2*SYN` | L3259 |
| RTO backoff formula | `RTO = RexmitCount*(RTT + 4*RTTVar + 2*SYN) + SYN` | L3270 |
| EWMA definition (smoothing) | "exponentially weighted moving average (EWMA)" | L3264-3265 |
| NAKInterval formula | `NAKInterval = max((RTT + 4*RTTVar)/2, 20000)` | L3276 |
| NAKInterval floor gloss | "minimum value ... set to 20 milliseconds" | L3279-3281 |
| SYN constant | "SYN, or synchronization time interval, which is 0.01 second" | L3421-3423 (§5.2.1, imported) |
| SYN cross-check | "A Full ACK control packet is sent every 10 ms" | L1328 (§3.2.4, cross-check only) |
| §5.2 boundary (not included) | "File Transfer Congestion Control (FileCC)" heading | L3283 |

No values were invented. The one gap flagged rather than guessed:
`INPUTBW_ESTIMATED`'s described lag/overshoot behavior (L3164-3184) is
prose-only in the draft — the text explicitly describes the failure mode
(black-screen/still-frame under-measurement, then slow ramp-up) without
giving a compensating formula or named constant, so none is fabricated
here.
