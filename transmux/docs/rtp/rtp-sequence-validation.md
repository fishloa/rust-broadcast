# RTP sequence-number validity checks — RFC 3550 Appendix A.1

Source: **RFC 3550**, "RTP: A Transport Protocol for Real-Time Applications"
(Schulzrinne/Casner/Frederick/Jacobson, July 2003), Appendix A.1 "RTP Data
Header Validity Checks". RFC 3550 is an IETF Standards Track RFC and is freely
redistributable; the excerpt below is transcribed verbatim from
`https://www.rfc-editor.org/rfc/rfc3550.txt` (page breaks / running headers
removed). Issue #779.

## Why this exists

`transmux/src/rtp.rs`'s `RtpHeader.sequence` field was parsed but never read
at any non-test call site — an RTP depacketiser that ignores sequence numbers
cannot tell a dropped packet, a reordered packet, or a duplicate packet apart
from a clean stream, so a FU-A fragment lost in transit was silently
concatenated with the fragments around it into a malformed access unit. RFC
3550 §A.1 is the standard's own answer to "how do I tell these cases apart,"
and its central technique — comparing sequence numbers with **wrapping**
arithmetic, never `>` or `<` directly, because the field is 16 bits and wraps
every 65536 packets — is exactly what
[`crate::rtp_stream::RtpStreamDepacketiser`] needed and didn't have.

## The verbatim algorithm (RFC 3550 §A.1)

> An RTP receiver should check the validity of the RTP header on incoming
> packets since they might be encrypted or might be from a different
> application that happens to be misaddressed. [...]
>
> Only weak validity checks are possible on an RTP data packet from a source
> that has not been heard before: [version, payload type, padding, extension,
> length consistency checks — omitted here, not applicable to this crate's
> use, see the full RFC for the complete list].
>
> If the SSRC identifier in the packet is one that has been received before,
> then the packet is probably valid and checking if the sequence number is in
> the expected range provides further validation. If the SSRC identifier has
> not been seen before, then data packets carrying that identifier may be
> considered invalid until a small number of them arrive with consecutive
> sequence numbers.
>
> The routine `update_seq` shown below ensures that a source is declared
> valid only after `MIN_SEQUENTIAL` packets have been received in sequence.
> It also validates the sequence number `seq` of a newly received packet and
> updates the sequence state for the packet's source in the structure to
> which `s` points.
>
> When a new source is heard for the first time [...] `s->probation` is set
> to the number of sequential packets required before declaring a source
> valid (parameter `MIN_SEQUENTIAL`) and other variables are initialized:
>
> ```c
> init_seq(s, seq);
> s->max_seq = seq - 1;
> s->probation = MIN_SEQUENTIAL;
> ```
>
> After a source is considered valid, the sequence number is considered valid
> if it is no more than `MAX_DROPOUT` ahead of `s->max_seq` nor more than
> `MAX_MISORDER` behind. If the new sequence number is ahead of `max_seq`
> modulo the RTP sequence number range (16 bits), but is smaller than
> `max_seq`, it has wrapped around and the (shifted) count of sequence number
> cycles is incremented. A value of one is returned to indicate a valid
> sequence number.
>
> Otherwise, the value zero is returned to indicate that the validation
> failed, and the bad sequence number plus 1 is stored. If the next packet
> received carries the next higher sequence number, it is considered the
> valid start of a new packet sequence presumably caused by an extended
> dropout or a source restart. Since multiple complete sequence number cycles
> may have been missed, the packet loss statistics are reset.
>
> Typical values for the parameters are shown, based on a maximum
> misordering time of 2 seconds at 50 packets/second and a maximum dropout of
> 1 minute. The dropout parameter `MAX_DROPOUT` should be a small fraction of
> the 16-bit sequence number space to give a reasonable probability that new
> sequence numbers after a restart will not fall in the acceptable range for
> sequence numbers from before the restart.

```c
void init_seq(source *s, u_int16 seq)
{
    s->base_seq = seq;
    s->max_seq = seq;
    s->bad_seq = RTP_SEQ_MOD + 1;   /* so seq == bad_seq is false */
    s->cycles = 0;
    s->received = 0;
    s->received_prior = 0;
    s->expected_prior = 0;
    /* other initialization */
}

int update_seq(source *s, u_int16 seq)
{
    u_int16 udelta = seq - s->max_seq;
    const int MAX_DROPOUT = 3000;
    const int MAX_MISORDER = 100;
    const int MIN_SEQUENTIAL = 2;

    /*
     * Source is not valid until MIN_SEQUENTIAL packets with
     * sequential sequence numbers have been received.
     */
    if (s->probation) {
        /* packet is in sequence */
        if (seq == s->max_seq + 1) {
            s->probation--;
            s->max_seq = seq;
            if (s->probation == 0) {
                init_seq(s, seq);
                s->received++;
                return 1;
            }
        } else {
            s->probation = MIN_SEQUENTIAL - 1;
            s->max_seq = seq;
        }
        return 0;
    } else if (udelta < MAX_DROPOUT) {
        /* in order, with permissible gap */
        if (seq < s->max_seq) {
            /*
             * Sequence number wrapped - count another 64K cycle.
             */
            s->cycles += RTP_SEQ_MOD;
        }
        s->max_seq = seq;
    } else if (udelta <= RTP_SEQ_MOD - MAX_MISORDER) {
        /* the sequence number made a very large jump */
        if (seq == s->bad_seq) {
            /*
             * Two sequential packets -- assume that the other side
             * restarted without telling us so just re-sync
             * (i.e., pretend this was the first packet).
             */
            init_seq(s, seq);
        }
        else {
            s->bad_seq = (seq + 1) & (RTP_SEQ_MOD-1);
            return 0;
        }
    } else {
        /* duplicate or reordered packet */
    }
    s->received++;
    return 1;
}
```

Where `RTP_SEQ_MOD` is `(1<<16)` — the 16-bit sequence-number space (defined
earlier in the RFC's `rtp.h` sample header).

> The validity check can be made stronger requiring more than two packets in
> sequence. [...]
>
> A strong "fast-path" check is possible since with high probability the
> first four octets in the header of a newly received RTP data packet will
> be just the same as that of the previous packet from the same SSRC except
> that the sequence number will have increased by one.

## How `transmux` applies this (and where it deliberately diverges)

`update_seq` above is a **validity classifier for RTCP loss statistics** — it
never buffers or reorders packets; it only ever moves `max_seq` forward
(optimistically accepting any gap under `MAX_DROPOUT` as "loss, move on") or
leaves it alone (a small backward `delta` is "reordered, but still valid,
no action needed"). That is sufficient for RFC 3550's own purpose (RTCP
receiver-report loss/jitter statistics) because nothing downstream needs the
packets delivered in a particular order — but H.264 FU-A reassembly
(RFC 6184 §5.8) is exactly the opposite: fragment order **is** the data, so
`RtpStreamDepacketiser` cannot just "accept the gap and move on" — it must
either recover the true order (a genuinely reordered packet) or cleanly drop
the access unit the loss corrupted (a genuinely lost packet), and RFC 3550
does not specify how to tell those apart or how to buffer for the former
(jitter/reorder buffer design is explicitly left to the implementation).

What `RtpStreamDepacketiser` keeps from §A.1, and what it replaces:

- **Kept**: the wrapping-subtraction discipline itself. `udelta = seq -
  s->max_seq` (unsigned 16-bit subtraction) is the *only* correct way to
  compare two 16-bit values that wrap mod 65536 — a plain `>`/`<` breaks the
  instant a stream crosses the 65535→0 boundary. `RtpStreamDepacketiser` uses
  the same idiom (`seq.wrapping_sub(expected) as i16`, signed so both
  "ahead of" and "behind" are visible in one comparison), matching this
  crate's existing `rtp_stream::unwrap_ts`, which unwraps the 32-bit RTP
  *timestamp* with the identical technique.
- **Kept**: SSRC-scoped state, and treating a changed SSRC as a new source to
  resync against (§8.2: "If a new source is heard for the first time")
  rather than a loss event.
- **Replaced**: `probation`/`MIN_SEQUENTIAL` source-validation bootstrapping —
  not applicable here, because a `RtpStreamDepacketiser` track is already
  bound to one SSRC by the caller (an RTSP `SETUP`/SDP negotiation already
  established the session) rather than discovered cold off a shared
  multicast group.
- **Replaced**: `MAX_DROPOUT`/`MAX_MISORDER` magnitude thresholds — those
  bound how large a jump is still "plausible loss" versus "this SSRC clearly
  restarted." `RtpStreamDepacketiser` instead bounds a small **reorder
  buffer** (`RtpStreamTrack::with_reorder_depth`, default
  `DEFAULT_REORDER_DEPTH`): packets arriving ahead of `expected` are held (in
  original wire form) until either the gap fills — restoring true order
  before reassembly — or the buffer's bound is reached (or end of stream),
  at which point the hole is declared genuinely lost (`RtpLossEvent::
  SequenceGap`), the closest held packet becomes the new baseline, and
  anything already-consecutive behind it is drained immediately. This is a
  deliberate, bounded design choice (RTP is untrusted remote input over UDP;
  see the module docs on the buffer's DoS bound), not a transcription of an
  RFC-specified algorithm — RFC 3550 does not define one.
- **Duplicates**: as in §A.1's fall-through `else { /* duplicate or reordered
  packet */ }` arm, a packet behind `expected` (already delivered or already
  given up on) and a packet already sitting in the reorder buffer are both
  discarded silently — RFC 3550 explicitly treats duplicates as legal
  ("Similarly...") and takes no corrective action.

See `transmux/src/rtp_stream.rs`'s module docs for the full design (including
where the resulting loss signal surfaces, and why).
