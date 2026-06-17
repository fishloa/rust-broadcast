## PCR arithmetic and coding frequency
_§2.4.2.1–2.4.2.2, PDF pp. 23-24; §2.7.2, PDF p. 106_

The PCR field is encoded in two parts: program_clock_reference_base in units
of 1/300 of the system clock frequency, and program_clock_reference_extension
in units of the system clock frequency. The value encoded indicates the time
t(i), where i is the index of the byte containing the last bit of the
program_clock_reference_base field:

```
PCR(i)      = PCR_base(i) × 300 + PCR_ext(i)                          (2-1)
PCR_base(i) = ((system_clock_frequency × t(i)) DIV 300) % 2^33        (2-2)
PCR_ext(i)  = ((system_clock_frequency × t(i)) DIV 1)   % 300         (2-3)
```

System clock frequency constraints (§2.4.2.1):

```
27 000 000 − 810 ≤ system_clock_frequency ≤ 27 000 000 + 810   (Hz)
rate of change of system_clock_frequency with time ≤ 75 × 10⁻³ Hz/s
```

i.e. the system clock is nominally **27 MHz**; PCR_base ticks at 90 kHz
(27 MHz / 300) and PCR_ext counts the 0–299 remainder of 27 MHz cycles.
Between PCRs, byte arrival times are interpolated linearly at the transport
rate (equations 2-4/2-5). The **PCR tolerance** — the maximum inaccuracy
allowed in received PCRs (imprecision or remultiplexing modification, not
network jitter) — is **± 500 ns** (§2.4.2.2).

**§2.7.2 Frequency of coding the program clock reference.** The Transport
Stream shall be constructed such that the time interval between the bytes
containing the last bit of program_clock_reference_base fields in successive
occurrences of the PCRs in TS packets of the PCR_PID for each program shall
be **less than or equal to 0.1 s**:

```
t(i) − t(i′) ≤ 0.1 s
```

for all consecutive PCR pairs of the PCR_PID. There shall be **at least two
(2) PCRs** from the specified PCR_PID between consecutive PCR
discontinuities (refer to §2.4.3.4) to facilitate phase locking and
extrapolation of byte delivery times.

