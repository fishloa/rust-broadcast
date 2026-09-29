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

`RtpStreamDepacketiser` runs §A.1's `update_seq` **verbatim** as its first
gate on every packet — `init_seq`, `probation`/`MIN_SEQUENTIAL`, `MAX_DROPOUT`,
`MAX_MISORDER`, `bad_seq` and `cycles`, with the RFC's own comparison order and
wrapping arithmetic. The arms map onto behaviour as follows:

| §A.1 outcome | effect here |
| --- | --- |
| probation (`MIN_SEQUENTIAL` not yet met) | the packet is **not** discarded: the caller's RTSP `SETUP`/SDP negotiation already established the session and its SSRC, so there is no unknown source to screen out, and §A.1's rejection is about the *statistics* (the RFC notes packets may be "discarded (or delayed in a queue)"). `max_seq` bookkeeping still tracks the source. |
| valid, `udelta < MAX_DROPOUT` | accepted ("in order, with permissible gap"); a wrapped 64k cycle increments `cycles`. |
| valid, large jump confirmed (`seq == bad_seq`) | **resync**: §A.1's "two sequential packets ... just re-sync (i.e., pretend this was the first packet)". The reorder buffer is cleared, the access unit under construction is dropped and the timeline origin is re-established. |
| 0, large jump unconfirmed | the packet is discarded and `bad_seq` is armed. **One stray far-behind packet therefore cannot move the baseline** — only a second sequential packet at the new numbering does. |
| fall-through (`udelta > RTP_SEQ_MOD - MAX_MISORDER`) | "duplicate or reordered packet" — accepted, no state change; the reorder buffer then drops it if it is behind what it is waiting for. |

Where it deliberately diverges: **§A.1 never reorders**, because RTCP loss
accounting does not need packets *delivered* in order. H.264 FU-A reassembly
does (RFC 6184 §5.8 — fragment order *is* the data). So an accepted packet ahead
of the hole is held in a small **bounded** reorder buffer
(`RtpStreamTrack::with_reorder_depth`, default `DEFAULT_REORDER_DEPTH`) and
replayed in wire order once the hole fills, or, if the buffer's bound is
reached (or end of stream forces it — `flush`), the hole is declared genuinely
lost (`RtpLossEvent::SequenceGap`), the closest held packet becomes the new
baseline, and anything already-consecutive behind it drains immediately. That
buffer is a hard bound: RTP is untrusted remote input over UDP, and this
project has already shipped four unbounded-allocation DoS vectors from RTP/TS
input.

An earlier revision of this module replaced the bounded-buffer resume point with
a sticky "abandoned range" instead of §A.1's `bad_seq` confirmation, which was
wrong in three ways: a single stray packet moved the baseline, the abandoned
range was never cleared (so the *live* run was the thing discarded once it
climbed past the bound — up to the full 32 768-packet blackout it was meant to
fix), and only an SSRC change could clear it. The transcription above has no
such state: a confirmed resync is §A.1's `init_seq` and nothing else carries
over.

See `transmux/src/rtp_stream.rs`'s module docs for the full design (including
where the resulting loss signal surfaces, and why).

## Consequences worth stating

Three costs follow from the RFC's own rules. They are properties of the
algorithm rather than defects to be fixed, and a caller sizing a jitter buffer
or accounting for loss should know them:

1. **Probation takes three packets, not two.** §A.1's `MIN_SEQUENTIAL` is 2, but
   the first packet of a session takes the *new-source* path (`init_seq` plus
   `probation = MIN_SEQUENTIAL`) and never runs `update_seq`, so the two
   sequential packets are counted from the second packet onward. No media is
   delayed for probation — `RtpStreamDepacketiser` releases every packet, since
   the caller's `SETUP`/SDP negotiation already established the session (see the
   divergence table above) — so the effect is only on when the source counts as
   valid.
2. **A backward jump of at most `MAX_MISORDER` (100) is accepted as misordered,
   then dropped.** §A.1 classifies a step up to 100 behind `max_seq` as
   "duplicate or reordered packet" (its fall-through arm), because for RTCP loss
   statistics it is indistinguishable from a genuine reorder. The reorder buffer
   then finds it behind `expected` and discards it, so **up to 100 packets are
   lost with no `RtpLossEvent`**. This is inherent: telling "reordered" from
   "the sender moved backward slightly" needs information RTP does not carry,
   and §A.1's threshold is the boundary the RFC chose. A real seek or restart
   moves far more than 100, which is why it is classified separately.
3. **The arming packet of a wild jump is discarded.** §A.1 returns 0 for the
   first packet of a wild jump and re-syncs only on the second, so the first
   packet of a post-seek run is lost. When that packet was an FU-A start, the
   access unit it began is damaged: it is reported as
   `RtpLossEvent::DamagedAccessUnit` and dropped rather than reassembled from a
   run missing its first fragment.
