## Adaptation field semantics
_§2.4.3.5, PDF pp. 34-38_

**adaptation_field_length** — number of bytes in the adaptation_field
immediately following the adaptation_field_length field. Value 0 inserts a
single stuffing byte. When adaptation_field_control is `'11'`, the value
shall be in the range **0 to 182**; when `'10'`, it shall be **183**. For
packets carrying PES packets, stuffing is accomplished by defining an
adaptation field longer than the sum of the lengths of its data elements and
filling the extra space with stuffing bytes — this is the **only** stuffing
method allowed for TS packets carrying PES packets. (PSI packets use the
alternative 0xFF stuffing of §2.4.4 instead.)

**discontinuity_indicator** — `'1'` means the discontinuity state is true for
the current packet (`'0'` or absent = false). Two discontinuity types:

- *System time-base discontinuity* — indicated in packets of a PID designated
  as a PCR_PID (§2.4.4.9). When true, the **next PCR** in a packet of that
  PID is a sample of a **new system time clock** for the program. The
  discontinuity point is the arrival instant (T-STD input) of the first byte
  of the packet containing the new time-base PCR. The indicator shall be
  `'1'` in the packet in which the discontinuity occurs; it may also be `'1'`
  in earlier packets of the same PCR_PID, in which case it shall stay `'1'`
  in every packet of that PID up to and including the packet carrying the
  first PCR of the new time base. After a discontinuity, **no fewer than two
  PCRs** of the new time base shall be received before another time-base
  discontinuity can occur; except in trick mode, data from no more than two
  time bases may be present in the T-STD buffers of one program at any time.
  No PTS/DTS of the new time base may arrive before the discontinuity, and
  none of the old time base after it.
- *Continuity-counter discontinuity* — may be signalled in any packet. For a
  non-PCR_PID, when the state is true the continuity_counter may be
  discontinuous with respect to the previous packet of that PID. For a
  PCR_PID, the counter may only be discontinuous in the packet where the
  time-base discontinuity occurs. At most **one** continuity-counter
  discontinuity point per discontinuity state. For non-PCR_PIDs, the
  indicator may be `'1'` in the next packet of the same PID, but shall not be
  `'1'` in **three consecutive** packets of the same PID. After a
  continuity-counter discontinuity in an elementary-stream PID, the first
  byte of elementary stream data shall be the first byte of an elementary
  stream access point (video: sequence header / visual object sequence header
  / AVC access unit, optionally preceded by a sequence_end_code; audio: first
  byte of an audio frame).
- While the discontinuity state is true, if two consecutive packets of the
  same PID have the same continuity_counter and adaptation_field_control
  `'01'`/`'11'`, the second may be discarded; the stream shall not be
  constructed so that discarding it loses PES payload or PSI data.
- PSI: after a discontinuity_indicator `'1'` in a packet carrying PSI, a
  single version_number discontinuity may occur; at it, a
  TS_program_map_section with **section_length == 13**,
  current_next_indicator == 1 (no descriptors, no streams) shall be sent,
  followed by a complete program definition with version_number incremented
  by one.

**random_access_indicator** — `'1'`: the next PES packet to start in this
PID's payload shall contain an elementary stream access point (and for video,
a PTS for the first picture after it; for audio, the PTS shall be in the PES
packet containing the first byte of the audio frame). In the PCR_PID it may
only be set to `'1'` in packets containing the PCR fields.

**elementary_stream_priority_indicator** — `'1'`: this payload has higher
priority among packets of the same PID (MPEG-2 video: only if the payload
contains bytes of an intra-coded slice; AVC: only slice_type 2, 4, 7 or 9).

**PCR_flag / OPCR_flag** — `'1'` indicates that the adaptation field contains
a PCR / OPCR field coded in two parts.

**splicing_point_flag** — `'1'` indicates a splice_countdown field is present,
specifying the occurrence of a splicing point.

**transport_private_data_flag** — `'1'` indicates one or more private_data
bytes are present.

**adaptation_field_extension_flag** — `'1'` indicates the presence of the
adaptation field extension.

**program_clock_reference_base; program_clock_reference_extension** — the
PCR is a 42-bit field coded in two parts (base per equation 2-2, extension
per equation 2-3 — see [PCR arithmetic](#pcr-arithmetic-and-coding-frequency)).
The PCR indicates the intended time of arrival of the byte containing the
**last bit of program_clock_reference_base** at the input of the system
target decoder.

**original_program_clock_reference_base/_extension (OPCR)** — coded
identically to the PCR; shall be coded only in packets in which the PCR is
present. Assists reconstruction of a single-program TS from another TS (copy
OPCR → PCR, valid only if the original stream is reconstructed exactly in its
entirety). `OPCR(i) = OPCR_base(i) × 300 + OPCR_ext(i)` (2-8). Ignored by
decoders; shall not be modified by any multiplexor or decoder.

**splice_countdown** — 8-bit signed (tcimsbf). Positive: the number of
remaining packets of the same PID until the splicing point (duplicates and
adaptation-field-only packets excluded); the splicing point is immediately
after the last byte of the packet in which splice_countdown reaches zero,
whose last payload byte shall be the last byte of a coded audio frame or
coded picture. The next payload-bearing packet of the PID shall start with
the first byte of a PES packet whose payload commences with an access point
(video: or a sequence_end_code followed by one). Value −n: this packet is the
n-th packet following the splicing point.

**transport_private_data_length** — number of private_data bytes immediately
following this field; private data shall not extend beyond the adaptation
field.

**adaptation_field_extension_length** — number of bytes of extended
adaptation field data immediately following this field, including reserved
bytes if present.

**ltw_flag / ltw_valid_flag / ltw_offset** — ltw_offset (15 bits, defined
only when ltw_valid_flag = `'1'`) is the legal-time-window offset in units of
(300/f_s) seconds (f_s = the program's system clock frequency): the upper
bound t1(i) of the Legal Time Window for this packet minus the packet's
T-STD arrival time t(i). Intended for remultiplexers reconstructing MBn
buffer state.

**piecewise_rate** — 22-bit positive integer; defined only when both ltw_flag
and ltw_valid_flag are `'1'`. Hypothetical bitrate R used to extrapolate the
Legal Time Window end times of following packets of the same PID that carry
no ltw_offset: t1(A_{i+j}) = t1(A_i) + j × 188 × 8 / R.

**seamless_splice_flag** — `'1'` indicates splice_type and DTS_next_AU are
present. Shall not be `'1'` where splicing_point_flag is not `'1'`; once set
in a packet with positive splice_countdown, it shall remain set in all
subsequent packets of the PID with splicing_point_flag `'1'` until the
splice_countdown-zero packet inclusive. splice_type shall be `'0000'` unless
the PID carries H.262 video (then it indexes the splice constraint Tables
2-7…2-20 and shall keep the same value until splice_countdown reaches zero).
DTS_next_AU (33 bits across three marker-delimited parts) is the decoding
time of the first access unit after the splicing point.

**stuffing_byte** — fixed 8-bit value `'1111 1111'` (0xFF), discarded by the
decoder.

