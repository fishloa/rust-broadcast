# H.222.0 / ISO 13818-1 (MPEG-2 Systems) — TS/PES rules

TS/PES behavioural rules for `mpeg-ts`, `mpeg-pes`, `ts-fix`, `dvb-conformance`. Source:
`specs/fulltext/itu_t_h222_0_202308_mpeg2_systems.md` (§ + line cites).

## TS packet header — §2.4.3.3 (fulltext L1708)

- **sync_byte** = `0x47` (L1710); sync-byte emulation in other fields should be avoided.
- **PID table** (Table 2-3, L1730): `0x0000` PAT · `0x0001` CAT · `0x0002` TSDT · `0x0003` IPMP · `0x0004` adaptive-streaming · `0x0005–0x000F` reserved · `0x0010–0x1FFE` assignable · **`0x1FFF` null**. PCR may be carried on `0x0000`/`0x0001`/`0x0010–0x1FFE` (Note 1, L1744).
- **transport_scrambling_control** (Table 2-4, L1748): only `00`=not-scrambled is spec-defined; `01/10/11` are **user-defined** in base 13818-1 (DVB assigns even/odd via ETSI TS 100 289 — see `specs/.../scrambling_control` transcription). Header + adaptation field **shall not be scrambled**; null packets use `00` (L1746).
- **adaptation_field_control** (Table 2-5, L1759): `00` reserved (decoders **shall discard**, L1768) · `01` payload-only · `10` adaptation-only · `11` both. Null packets use `01`.
- **continuity_counter** (L1770) — the rules a CC repair MUST honour:
  - 4-bit, increments per packet **of the same PID**, wraps 15→0.
  - **Shall NOT increment when adaptation_field_control == `00` or `10`** — only payload-bearing (`01`/`11`) advance it.
  - **Duplicate packets** (L1772): a packet MAY be sent **twice** with the **same CC** (afc `01`/`11`, bytes identical except a re-encoded valid PCR). A repair MUST NOT renumber a legitimate duplicate.
  - **Discontinuity** (L1774): CC may be discontinuous when `discontinuity_indicator='1'`. A repair MUST NOT "fix" a signalled discontinuity. Null-packet CC is undefined.

## Adaptation field — §2.4.3.4/5 (fulltext L1778 / L1866)

- **adaptation_field_length** (L1868): afc `11` → 0–182; afc `10` → exactly 183; `0` = one stuffing byte. Stuffing fills the AF so payload exactly fits — the **only** stuffing method for PES-carrying packets. (Grounds mpeg-ts byte-identical stuffing round-trip.)
- **discontinuity_indicator** (L1872) — two meanings:
  - On a **PCR_PID**: a **system-time-base discontinuity** — the next PCR on that PID samples a *new* clock (L1874). Once set, it stays `1` through to the packet carrying the first new-base PCR; ≥2 new-base PCRs before another discontinuity. → a PCR-restamp must **re-anchor** here, not smooth across it.
  - On a **non-PCR_PID**: licenses a CC discontinuity (L1880); at most once per discontinuity state; never set in 3 consecutive packets of one PID.
- **PCR/OPCR** (syntax L1801): 33-bit base (90 kHz) + 6 reserved + 9-bit extension (27 MHz); value = base×300 + ext. **PCR is per-program**: PMT names a `PCR_PID` per program (§2.4.4.10), and §2.7.2 requires PCRs on "the PCR_PID **for each program**" at least every 100 ms → a multi-program TS carries **multiple PCR PIDs**, restamped independently.
- **splice_countdown** (L1817): packets until a splice point (signed). **seamless_splice** (L1849): Splice_type(4) + DTS_next_AU(33, marker-bit-interleaved). transport_private_data + adaptation_field_extension (ltw / piecewise_rate) — typed in mpeg-ts.

## PES packet — §2.4.3.6/7 (fulltext L2233 / L2434)

- **packet_start_code_prefix** = `0x000001` + **stream_id** (Table 2-22, L2444). PES_packet_length `0` allowed only for video ES in TS (L2440).
- **PTS_DTS_flags** (L2521): `10`=PTS only · `11`=PTS+DTS · `00`=neither · **`01` forbidden**. PTS/DTS are 33-bit @ 90 kHz, three marker-bit-interleaved fields (L2541).
- Optional-field flags (L2525): ESCR · ES_rate · DSM_trick_mode(8-bit) · additional_copy_info · PES_CRC · PES_extension — each typed in mpeg-pes. **PES_header_data_length** counts the optional fields **plus stuffing** (L2537) → grounds mpeg-pes byte-identical stuffing round-trip.
- **PES_scrambling_control** (Table 2-23): same `00`/user-defined values as TS; header (incl. optional fields) not scrambled.

## Timing — §2.4.2 / §2.7 (fulltext L1335 / see also dvb-si pcr-frequency transcription)

- **System clock** 27 MHz; PCR/PTS/DTS @ 90 kHz (= 27 MHz / 300). §2.7.2 PCR ≤100 ms spacing per program; §2.7.4 PTS spacing constraints.

## Implications for our crates
- `ts-fix` CC-repair must honour **duplicate packets** (§2.4.3.3 L1772) + **discontinuity_indicator**
  (§2.4.3.5 L1874) — not a strict +1 per PID.
- `ts-fix` PCR-restamp must **re-anchor** on a PCR_PID `discontinuity_indicator` (§2.4.3.5), per-PID
  (§2.7.2 + §2.4.4.9 — a multi-program TS has multiple PCR PIDs).
