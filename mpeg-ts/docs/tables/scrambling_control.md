# transport_scrambling_control — meaning (ETSI TS 100 289 §5.1)

The 2-bit `transport_scrambling_control` field in the TS packet header (and the
matching `PES_scrambling_control` in the PES header) is **only partially defined by
MPEG-2**: ITU-T H.222.0 | ISO/IEC 13818-1 (Table 2-4) defines only the `00`
(not-scrambled) value and leaves `01`/`10`/`11` "user-defined". **DVB assigns the full
meaning** in ETSI TS 100 289 (Support for use of scrambling and Conditional Access
within digital broadcasting systems), vendored at
`specs/etsi_ts_100_289_v01.01.01_dvb_scrambling_ca.pdf`.

Bit semantics (TS 100 289 §5.1): **bit 1 (MSB) = scrambled or not; bit 0 (LSB) =
Even/Odd key.**

## Table 1 — Transport_scrambling_control values (TS packet header)

| Bits | Meaning |
|---|---|
| `00` | No scrambling of TS packet payload (MPEG-2 compliant) |
| `01` | Reserved for future DVB use |
| `10` | TS packet scrambled with **Even** Key |
| `11` | TS packet scrambled with **Odd** Key |

## Table 2 — PES_scrambling_control values (PES packet header)

| Bits | Meaning |
|---|---|
| `00` | No scrambling of PES packet payload (MPEG-2 compliant) |
| `01` | Reserved for future DVB use |
| `10` | PES packet scrambled with **Even** Key |
| `11` | PES packet scrambled with **Odd** Key |

If the TS-level payload is not scrambled (`00`), scrambling may still be defined at the
PES level. The two levels use parallel bit assignments so a descrambler can handle both
uniformly.

## How `mpeg-ts` types it

`TsHeader`/`OwnedTsPacket` expose the raw 2-bit value plus a typed accessor
`scrambling_control() -> ScramblingControl`:

```
pub enum ScramblingControl {
    NotScrambled, // 00 — H.222.0 Table 2-4 (the only MPEG-defined value)
    Reserved,     // 01 — reserved for future DVB use (TS 100 289 Table 1)
    EvenKey,      // 10 — scrambled, even control word
    OddKey,       // 11 — scrambled, odd control word
}
```

Citations: ITU-T H.222.0 (08/2023) Table 2-4 (the `00` value + the field width);
ETSI TS 100 289 V1.1.1 §5.1 Table 1 (the `01`/`10`/`11` Even/Odd/Reserved assignment).
The even/odd-key meaning is a DVB common-scrambling convention layered onto the
MPEG-2 field — surfaced here because the field lives in the MPEG TS header and this is
its universal real-world usage.
