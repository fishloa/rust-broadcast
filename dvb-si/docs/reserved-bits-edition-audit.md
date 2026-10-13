# Reserved-bit edition audit (dvb-si)

_Status: research deliverable (2026-10-13). Every claim below carries a citation of the form `spec version (date) clause/table`; where a source could not be obtained the claim is marked **UNVERIFIED**; no bit value is guessed._

## 0. Question, method, evidence base

**Hypothesis (owner):** older editions defined the reserved bits as `0` or as plain `reserved`; newer editions either mandate `1` or **give the bits a meaning** (a field is carved out of the reserved space). Senders following the older edition then look "off-spec" against the edition dvb-si was transcribed from.

**Method.**

1. *Inventory*: every `reserved` / `reserved_future_use` / `reserved_zero_future_use` row in `dvb-si/docs/{descriptors,tables}/**` (372 rows in 161 doc files) plus the same names in the full syntax tables of the source specs (the docs/ transcription omits some descriptors, e.g. AAC, component, linkage), the value dvb-si emits (cross-checked against `const RESERVED_*` masks in `dvb-si/src`, see §6 note) and the cited spec/edition.
2. *Edition identity of the transcriptions*: EN 300 468 **V1.19.1 (2025-02)** (`private/specs/etsi_en_300_468_v01.19.01_dvb_si.pdf`); Rec. ITU-T H.222.0 **(06/2021) | ISO/IEC 13818-1:2021** (md header; the private PDF is the 08/2023 edition); TS 102 809 **V1.3.1 (2017-06)**; TS 102 006 **V1.4.1 (2015-06)** (the private file is named `v01.07.01` but its title page says V1.4.1); EN 301 192 **V1.7.1 (2021-08)**; TS 102 323 **V1.4.1 (2010-01)**; EN 303 560 V1.1.1 (2018-05); TS 102 772 V1.1.1; TS 102 825-4 V1.2.2 (2013-12) / -9 V1.2.1; TS 102 366 V1.4.1 (2017-09); TS 101 154 V2.10.1.
3. *Edition fetch* (all kept under the scratchpad, none committed): every ETSI "published" (`_60`) edition of **EN 300 468 (18: V1.3.1 1998-02 ... V1.20.1 2026-09, i.e. one edition NEWER than the transcription)**, **TS 102 809 (3)**, **TS 102 006 (4)**, **EN 301 192 (9, incl. V1.8.1 2025-06 newer than ours)**, **TS 102 323 (5)**, EN 300 743 (5), EN 303 560 (1), TS 102 772 (1), TS 102 366 (4), TS 102 825-4 (3), TS 102 825-9 (2), **TS 101 812 MHP (5)**, TS 101 154 (17); and **9 editions of Rec. ITU-T H.222.0** from ITU-T's free archive (07/1995, 02/2000, 05/2006, 06/2012, 10/2014, 03/2017, 08/2018, 06/2021, 08/2023).
4. *Per-edition comparison*: `pdftotext -layout` of every edition (used **only to locate**), a script that extracts every `xxx_descriptor()/section()/info()` syntax block per edition and diffs the sequence `(field,width)` with reserved rows tagged `r` (`reserved`), `rfu` (`reserved_future_use`) or `rzfu` (`reserved_zero_future_use`); every flagged difference and every priority field was then **re-read by hand in the affected editions**; the highest-stakes layouts were additionally **checked on rendered page images** (TS 101 812 V1.3.2 p.87, TS 102 809 V1.3.1 p.37, H.222.0 06/2012 hierarchy table, EN 300 468 V1.15.1 p.54 S2 descriptor and V1.5.1 p.43 terrestrial descriptor). Bit *widths* elsewhere come from pdftotext column layout and are therefore **not image-verified** (project rule: not authoritative for tables).
5. *Cross-implementation evidence*: TSDuck (`tsduck/tsduck` master) `serializePayload` of the AC-3, application_signalling and supplementary_audio descriptors, read with `gh api`.

**Not available / UNVERIFIED** (see also §10): ITU-T H.222.0 **(04/2025)** (in force, paid); ETS 300 468 editions before V1.3.1 (1995/1997); NorDig / D-Book / ATSC texts (`nordig/*` rows, `logical_channel.rs`); the real-capture byte survey itself (only its published pattern list on branch `worktree-agent-a1e08166d0299fb28`, `dvb-si/tests/real_capture_byte_identity.rs`, was used - captures were not re-read).

Legend for layouts below: `r(n)` = `reserved`, `rfu(n)` = `reserved_future_use`, `rzfu(n)` = `reserved_zero_future_use`; the value after `=` is what the governing text prescribes by its **global convention** (§2). Classes: **A** stable (reserved layout identical in every edition where the structure exists) - **B** rename only (`reserved`/`DVB_reserved`/`reserved_0_future_use` -> `reserved_future_use`/`reserved_zero_future_use`, value unchanged) - **C** *carve-out*: bits of the reserved space got a meaning in a later edition (or the reverse) - **D** explicit per-field value override of the global convention - **E** extraction noise (edition history of that row not trustworthy; latest layout read directly).

## 1. Verdict summary

| Spec | blocks with reserved fields | A stable | B rename | C carve-out | D value override | E noise |
|---|---|---|---|---|---|---|
| EN 300 468 | 61 | 34 | 12 | 6 | 3 | 6 |
| H.222.0 | 58 | 48 | 0 | 8 | 1 | 1 |
| TS 102 809 | 11 | 11 | 0 | 0 | 0 | 0 |
| TS 102 006 | 3 | 3 | 0 | 0 | 0 | 0 |
| EN 301 192 | 8 | 8 | 0 | 0 | 0 | 0 |
| TS 102 323 | 7 | 5 | 0 | 2 | 0 | 0 |
| TS 102 772 | 2 | 2 | 0 | 0 | 0 | 0 |
| EN 303 560 | 3 | 3 | 0 | 0 | 0 | 0 |
| EN 300 743 | 7 | 7 | 0 | 0 | 0 | 0 |
| **total** | 160 | 121 | 12 | 16 | 4 | 7 |

(Counts are per syntax block, ie. per descriptor/section/structure, in the editions where it exists; a block counts once even when it has several reserved rows. TS 102 809, TS 102 006, EN 301 192, TS 102 772, EN 303 560 and EN 300 743 blocks are listed in §6.3 - they are all class A except as noted.)

**Headline findings**

1. **The hypothesis holds for exactly one of the five descriptor-level real-sender patterns: `application_signalling_descriptor` 0x6F.** Every edition of the MHP spec **TS 101 812** (V1.1.1 2000-07 ... V1.3.2 2006-08) defines the descriptor with a **16-bit `application_type` and no reserved bit**; the 1-bit `reserved_future_use` before `application_type`(15) exists only in **TS 102 809** (V1.1.1 2010-01, 1.2.1, 1.3.1: all identical, clause 5.3.3.2: all `reserved_future_use` = `1`). Because every allocated `application_type` is <= 0x7FFF the MSB is 0 on the wire of an MHP-era sender. See §3.2.
2. **AC-3 `reserved_flags`(4) is NOT an edition effect - dvb-si is wrong.** All 17 editions of EN 300 468 that contain the descriptor (V1.4.1-V1.20.1) and TS 102 366 V1.1.1/V1.2.1 say *"These 1-bit fields are reserved for future use. They **should always be set to 0**"* - an explicit exception to the global "reserved = 1" rule, never changed, never given a meaning. dvb-si emits `1111` (`ac3.rs:349-350`). 47/62 real senders write `0000` (spec), TSDuck writes `0000` (`buf.putBits(0, 4); // reserved bits are zero here`). Fix: emit `0000`. See §3.1.
3. **`supplementary_audio_descriptor` reserved_future_use(1)**: `reserved` (V1.13/V1.14) then `reserved_future_use` (V1.15+, CR 1041) - **value 1 in every edition, never given a meaning**; no other fetched spec (TS 101 154 x17, TS 103 285 x4) mentions it. 37/47 deviating senders are therefore **not explained by any edition** (UNVERIFIED cause; TSDuck writes 1). See §3.3.
4. **`satellite_delivery_system_descriptor` roll_off = 01 on DVB-S**: in **no** edition (V1.3.1 - V1.20.1) is the DVB-S value anything but `00` (V1.3-V1.6 the whole 5-bit `modulation` field cannot be `01xxx`; V1.7-V1.15 literal "00"; V1.16+ `reserved_zero_future_use`(2)). `01` = alpha 0.25 exists only for DVB-S2 (`modulation_system`=1, Table 39). Sender non-conformance, not an edition. See §3.5.
5. **CA descriptor `reserved`(3)** (2/715): `reserved` = `1` by definition in all 9 H.222.0 editions; never a meaning. Noise. See §3.4.
6. **Section-level bits**: the bit after `section_syntax_indicator` is H.222.0's **`private_indicator` - "a 1-bit user definable flag that shall not be specified by ITU-T | ISO/IEC in the future" (all 9 editions)**; EN 300 468 renames that slot `reserved_future_use` and requires `1`. A sender that writes `0` there (NIT `0xB0`, test-016) is H.222.0-compliant and EN 300 468-non-compliant: explained. The other section-level deviations (`reserved(2)=00`, TDT `0x00` vs `0x70`, PAT `reserved(3)=000`) have **no** edition basis (§3.6).
7. **Carve-outs exist and are systematic - 19 structures (§4), always designed so that the legacy "all ones" fill reads as the neutral default**: AAC SAOC_DE_flag, component `stream_content_ext`, FTA `user_defined`, S2 `backwards_compatibility_indicator`/`not_timeslice_flag`/`TS_GS_mode`, S2Xv2 `NCR_reference`, terrestrial `priority`/`Time_Slicing_indicator`/`MPE-FEC_indicator`, hierarchy flags + `tref_present_flag`, AVC `constraint_set3-5`/`Frame_Packing_SEI_not_present_flag`, MVC view flags, HEVC `copied_44bits`/`HDR_WCG_idc`/`target_schedule_idx`, MPEG-H `compatibleProfileSetsPresent`, adaptation-field `af_descriptor_not_present_flag`, TS 102 323 link_info/RCT. **dvb-si already types every one of them that is in scope** (§4); the not-yet-typed ones are descriptors dvb-si does not implement (§7).
8. **No reserved bit of EN 300 468 V1.19.1 (the transcription) changed meaning in V1.20.1 (2026-09)**; no reserved bit of H.222.0 06/2021 changed *except* MPEG-H `compatibleProfileSetsPresent` (08/2023, descriptor not implemented). H.222.0 04/2025: UNVERIFIED.
9. Two **dvb-si discrepancies found on the way** (§9): AC-3 (above) and `hevc_video` byte-12 bit 3 emitted 0 where H.222.0 (2017-2023) lists `reserved`(2) (=1).

## 2. Global "reserved" conventions, edition by edition

| Spec | Editions | `reserved` | `reserved_future_use` | `reserved_zero_future_use` |
|---|---|---|---|---|
| EN 300 468 | V1.3.1 (1998-02), V1.4.1 (2000-11) | "shall be set to '1'" - **normative** sentence in the definition | same | (term absent) |
| EN 300 468 | V1.5.1 (2003-05) - V1.16.1 (2019-08) | the same sentence is demoted to a **NOTE** ("Unless otherwise specified within the present document all "reserved" bits shall be set to "1"" V1.5.1-1.7.1; "is/are set to "1"" V1.8.1-1.16.1) | same NOTE for `reserved_future_use` | defined from **V1.16.1** (no value stated); per-descriptor NOTEs in V1.13.1-V1.16.1 ("The reserved_future_use fields defined above have all their bits set to a value of '0'": T2MI, video_depth_range, AC-4...) |
| EN 300 468 | V1.17.1 (2022-10) - V1.20.1 (2026-09) | "Unless otherwise specified ... all reserved bits are set to 0b1" - normative again | same for `reserved_future_use` | **"All reserved_zero_future_use bits are set to 0b0"** (V1.17.1+) |
| H.222.0 / 13818-1 | all 9: 07/1995 ... 08/2023 (clause "reserved") | "Unless specified otherwise within this Recommendation / International Standard, all reserved bits shall be set to '1'" - verified in each of the 9 texts | n/a | n/a |
| TS 102 809 (AIT) | V1.1.1 (2010-01), V1.2.1, V1.3.1 | V1.3.1 NOTE: all `reserved` bits "set to "1"" | clause 5.3.3.2 (all 3): "Unless otherwise specified within the present clause all "reserved_future_use" bits shall be set to "1"" | V1.3.1 defines the term (no value) |
| TS 101 812 (MHP) | V1.1.1-V1.3.2 | not re-checked (descriptor layout only) | - | - |
| TS 102 006 (SSU) | V1.2.1 (2002-10) - V1.4.1 (2015-06) | **no convention statement in any edition** (UNT section tables carry an explicit value column: `reserved_for_future_use`=1, `reserved`=11, V1.4.1 table 11; the update-info selector `reserved` rows (table 4) have none) | - | - |
| EN 301 192 | V1.1.1-V1.8.1 | no global statement; field-level "shall be set to 0xFF / '1' when not used" for time-slicing/MPE-FEC fields from V1.4.1 | - | - |
| TS 102 323 | V1.1.1 (2004-09) - V1.5.1 (2012-01) | "All fields marked as being reserved shall have all bits set to '1'"; `reserved_other_use` = '0' | same | - |
| EN 300 743 | V1.2.1-V1.5.1 | no convention text found; **V1.6.1 (2018-10)** adds "all reserved bits are expected to be set to '1'" / `reserved_zero_future_use` "expected to be set to '0'" | | |
| EN 303 560 | V1.1.1 (2018-05) | | | "shall be set to value of '0'" |
| TS 102 825-4 / -9 (CPCM) | V1.1.1-V1.2.2 | "shall be set to zero" for CPCM `reserved`/`reserved_for_future_use`; the CPCM USI table (Table 8) has **no reserved bits at all in any of the 3 editions** | | |

Consequences: (a) **the "reserved = 1" rule was only a NOTE in EN 300 468 from 2003 to 2019** (V1.5.1-V1.16.1), so a sender built against that era could legitimately treat the fill value as informative; (b) the **`reserved_zero_future_use` = 0** class appears with V1.16.1/V1.17.1 - a descriptor introduced earlier with plain `reserved_future_use` can have a 0 requirement only through a field-level NOTE/sentence (T2MI, AC-4, AAC, C2 bundle).

## 3. Priority fields (the real-sender deviations), field by field

### 3.1 AC-3 descriptor 0x6A - `reserved_flags`(4)

| Document | Clause | Label | Statement (verbatim) |
|---|---|---|---|
| EN 300 468 V1.4.1 (2000-11) | annex E (AC-3 descriptor) | `reserved flags` | "These 1-bit fields are reserved for future use. They should always be set to "0"." |
| V1.5.1 (2003-05) | annex D | `reserved` (syntax) / `reserved flags` (semantics) | identical |
| V1.6.1 - V1.16.1 | annex D | `reserved_flags` | identical |
| V1.17.1 (2022-10) - V1.20.1 (2026-09) | annex D | `reserved_flags` | "...They should always be set to 0b0." |
| TS 102 366 V1.1.1 (2005-02), V1.2.1 (2008-08) | clause A.3.3.2 (AC-3_descriptor syntax) | `reserved flags` | "...should always be set to "0"." (V1.3.1/V1.4.1 no longer carry the descriptor) |

**Verdict: explicit override of the global convention in all 19 documents (17 EN 300 468 + 2 TS 102 366); value 0; no meaning ever assigned.** "should" is recommendatory, so a parser must stay tolerant; the serializer value, however, is `0000`. dvb-si (`dvb-si/src/descriptors/ac3.rs:349-350`: `// ... reserved_flags and must be set to 1.` / `buf[2] = flags | 0x0F`) contradicts every edition. Real senders: 47/62 `0000` (spec-conformant), 15/62 `1111` (global rule applied by analogy). TSDuck `tsDVBAC3Descriptor.cpp` writes `0000`. **Action: change to 0 (not an edition-compat question).**

### 3.2 `application_signalling_descriptor` 0x6F - `reserved_future_use`(1) before `application_type`, `reserved_future_use`(3) before `AIT_version_number`

| Document | Date | Layout of one loop entry | Notes |
|---|---|---|---|
| **TS 101 812 (MHP)** V1.1.1 | 2000-07 | `application_type`(16) `reserved_future_use`(3) `AIT_version_number`(5) | "application_type: This 16 bit field identifies the application type of an AIT sub-table" |
| V1.1.2 / V1.2.1 / V1.3.1 / V1.3.2 | 2001-10 / 2002-06 / 2003-06 / 2006-08 | identical (V1.3.2 table 15 on p.87 checked on the page image) | in the *AIT* V1.3.1+ already has `test_application_flag`(1)+`application_type`(15), the descriptor kept 16 bits |
| **TS 102 809** V1.1.1 | 2010-01 | `reserved_future_use`(1) `application_type`(15) `reserved_future_use`(3) `AIT_version_number`(5) | "application_type: This 15 bit field"; clause 5.3.3.2: all `reserved_future_use` = "1" |
| V1.2.1 / V1.3.1 | 2013-07 / 2017-06 | identical (V1.3.1 table 17, p.37, checked on the page image) | |

Application types (TS 101 812 V1.3.2 table 10): 0x0000 reserved, 0x0001 DVB-J, 0x0002 DVB-HTML, 0x0003-0x7FFF "subject to registration with DVB" (HbbTV = 0x0010) - **every allocated value has bit 15 = 0**.
**Verdict: bit 15 had a meaning (MSB of a 16-bit `application_type`) in the older MHP editions (all five); TS 102 809 (the SI-signalling successor) redefines it as `reserved_future_use`=1.** A sender following TS 101 812 therefore writes 0 (15/52 deviating, incl. `hotbird-mhp.ts`); TSDuck (`putBit(1); putBits(app_type,15); putBits(0xFF,3)`) and dvb-si follow TS 102 809. Parse stays tolerant (already ignores the bit). The `reserved_future_use`(3) before `AIT_version_number` is `reserved_future_use`(3) in all 8 editions (value 1 via 102 809 §5.3.3.2; TS 101 812 value statement not checked).

### 3.3 `supplementary_audio_descriptor` (extension tag 0x06) - `reserved_future_use`(1)

| EN 300 468 | Date | Label | Value |
|---|---|---|---|
| V1.13.1 | 2012-08 | `reserved` | 1 (NOTE, global) |
| V1.14.1 | 2014-05 | `reserved` | 1 |
| V1.15.1 | 2016-03 | `reserved_future_use` (CR 1041: "a reserved field should be reserved for future use in supplementary audio descriptor") | 1 |
| V1.16.1 - V1.20.1 | 2019-08 ... 2026-09 | `reserved_future_use` (V1.17.1+: "are set to 0b1") | 1 |

Layout in all 8 editions: `mix_type`(1) `editorial_classification`(5) **`reserved`(1)** `language_code_present`(1). **Verdict: still reserved, value 1 in every edition; no edition defines it differently.** TS 101 154 (17 editions) and TS 103 285 (4) contain no `supplementary_audio_descriptor` text. TSDuck writes 1. The 37/47 senders writing 0 are **not explained by any fetched edition** - UNVERIFIED (France TNT profile / encoder default are candidates; the DVB BlueBook A038 change history gives nothing).

### 3.4 CA descriptor 0x09 - `reserved`(3) before `CA_PID`

H.222.0 07/1995 ... 08/2023 (all 9): `CA_system_ID`(16) `reserved`(3) `CA_PID`(13); `reserved` = '1' by the clause-2 definition. **Verdict: still reserved-and-1, never a meaning.** 2/715 deviations (test-128) = sender noise.

### 3.5 `satellite_delivery_system_descriptor` 0x43 - roll_off on DVB-S, and the 01 case

| EN 300 468 | 5 bits after `west_east_flag`(1) `polarization`(2) (descriptor byte 8 counting tag as byte 0) | DVB-S value | roll_off=01 anywhere? |
|---|---|---|---|
| V1.3.1 (1998-02) - V1.4.1 (Table 28) | 5-bit **`modulation`**: `00000` not defined, `00001` QPSK, `00010`-`11111` reserved | bits[4:3]=`00` | `01xxx` = *reserved for future use* |
| V1.5.1 - V1.6.1 (Table 38) | `modulation`(5): QPSK 00001, **8PSK 00010, 16-QAM 00011**, `00100`-`11111` reserved | `00` | `01xxx` reserved |
| V1.7.1 (2006-05) - V1.15.1 | `roll_off`(2) / `modulation_system`(1) / `modulation_type`(2); syntax `if (modulation_system=="1") { roll_off } else { "00" }` | literal `"00"` | roll_off only defined for DVB-S2 (Table 38/39: 00=0.35, **01=0.25**, 10=0.20, 11 reserved) |
| V1.16.1 (2019-08) - V1.20.1 | same but the else-branch is **`reserved_zero_future_use`(2)** | `00` | same |

**Verdict: in no edition does `roll_off=01` have a defined meaning on DVB-S (`modulation_system`=0); the bits are `00`/reserved-zero from the first edition to the latest.** `01` (alpha 0.25) is a DVB-S2 value (EN 302 307-1 roll-off) - a sender copying the S2 roll-off into a DVB-S descriptor (or a mux that writes roll_off irrespective of `modulation_system`) is the likely cause (UNVERIFIED; 1 capture, test-101). dvb-si emits `00` (satellite_delivery_system.rs:399) = correct for all editions.

### 3.6 Section-level reserved bits (TDT, NIT, SDT, EIT, PAT, AIT)

Byte 1 of a section = `section_syntax_indicator`(1) | **bit 6** | `reserved`(2) | `section_length[11:8]`(4):

| Section | H.222.0 (all 9 editions) bit 6 | EN 300 468 (all 18) bit 6 | bits 5:4 | Other reserved |
|---|---|---|---|---|
| PAT/PMT/CAT/TSDT (13818-1) | literal **`'0'`** | - | `reserved`(2) | PAT/PMT `reserved`(2) before version, `reserved`(3) before PIDs, `reserved`(4) before lengths (all editions) |
| NIT/BAT/SDT/EIT/SIT/RST/ST/TDT/TOT/DIT | `private_indicator`: "user definable flag that shall not be specified by ITU-T / ISO/IEC in the future" | **`reserved_future_use`**(1) (=1) | `reserved`(2) (=1) | `reserved`(2) before `version_number`; `reserved_future_use`(4) before descriptor lengths |
| AIT (TS 102 809 V1.1.1-V1.3.1; TS 101 812) | - | `reserved_future_use`(1) | `reserved`(2) | `reserved`(2), `reserved_future_use`(4) |
| UNT (TS 102 006) | - | `reserved_for_future_use`(1)=1 (explicit value column) | `reserved`(2)=11 | |

**Verdict:** bit 6 reads "user-definable" in H.222.0 (never given a meaning in 9 editions) while DVB mandates 1 - this explains `0xB0` NIT headers (test-016) as **H.222.0-conformant**. Nothing in any edition licenses `reserved(2)=00` (EIT/PAT/TSDuck-generated samples) or TDT `0x00` (test-120); `test-062` is a TSDuck random-bits stress capture (all reserved bits random).

## 4. Carve-outs: reserved space that gained (or lost) a meaning

| # | Spec | Structure | Edition of change | Old (reserved) | New field(s) | Old all-ones fill reads as | dvb-si |
|---|---|---|---|---|---|---|---|
| 1 | EN 300 468 | `AAC_descriptor` (0x7C) | V1.15.1 (2016-03) | `reserved`(7) =1 | `SAOC_DE_flag`(1) + `reserved_future_use`(6); V1.16.1 -> `reserved_zero_future_use`(6)=0 | SAOC_DE_flag = **1** ("SAOC-DE data present") - a pre-1.15 sender mis-signals | typed (aac.rs); emits 0 |
| 2 | EN 300 468 | `component_descriptor` (0x50) | V1.15.1 | `reserved_future_use`(4) =1111 | `stream_content_ext`(4) | 0xF = "n/a for stream_content 0x01-0x08" (NOTE 8) - **designed-compatible** | typed (component.rs) |
| 3 | EN 300 468 | `FTA_content_management_descriptor` (0x7E) | V1.12.1 (2011-10) | `reserved_future_use`(4) | `user_defined`(1) + rfu(3) | user_defined = 1 (should be 0 if no semantics defined) | typed |
| 4 | EN 300 468 | `S2_satellite_delivery_system_descriptor` (0x79) | V1.16.1 (2019-08) | bit5 `backwards_compatibility_indicator` (V1.7-1.15) + `reserved_future_use`(5) | bit5 `reserved_zero_future_use`; bit4 `not_timeslice_flag`; bits1:0 `TS_GS_mode`; bits3:2 rfu | old rfu=11111 -> not_timeslice=1 (no slicing), TS_GS_mode=3 (TS) - compatible; **but the old BC-indicator bit is now reserved-zero** (a LP-stream sender loses its flag) | typed (not_timeslice, TS_GS) |
| 5 | EN 300 468 | `S2Xv2_satellite_delivery_system_info` | V1.19.1 (2025-02) | `reserved_zero_future_use`(3) | `NCR_reference`(1) + rzfu(2) | 0 = pre-1.19 behaviour (backwards-compatible revision per history) | typed |
| 6 | EN 300 468 | `terrestrial_delivery_system_descriptor` (0x5A) | V1.6.1 (2004-11) | `reserved_future_use`(5) | `priority`, `Time_Slicing_indicator`, `MPE-FEC_indicator` + rfu(2) | HP / "not used" / "not used" - compatible | typed (priority, time_slicing_used, mpe_fec_used) |
| 7 | H.222.0 | `hierarchy_descriptor` (0x04) | 06/2012 and 03/2017 | `reserved`(4) (2006) | `temporal/spatial/quality_scalability_flag` (2012) -> `no_view/no_temporal/no_spatial/no_quality_scalability_flag` (2017); `tref_present_flag` from byte-4 `reserved`(2) (2012) | "1" = no scalability / no TREF - compatible; "the value of '1' for this flag is reserved" | typed (hierarchy.rs) |
| 8 | H.222.0 | `AVC_video_descriptor` (0x28) | 06/2012 | `AVC_compatible_flags`(5), `reserved`(6) | `constraint_set3/4/5_flag`, `AVC_compatible_flags`(2); `Frame_Packing_SEI_not_present_flag`+`reserved`(5) | n/a | typed (avc_video.rs) |
| 9 | H.222.0 | `MVC_extension_descriptor` (0x31) | 10/2014 | `reserved`(4) (2012) | `view_association_not_present_flag`, `base_view_is_left_eyeview`, `reserved`(2) | n/a | typed |
| 10 | H.222.0 | `HEVC_descriptor` -> `HEVC_video_descriptor` (0x38) | 03/2017 | `reserved_zero_44bits`(44), `reserved`(4) (2014) | `copied_44bits`(44), `reserved`(2)+`HDR_WCG_idc`(2) | n/a | typed |
| 11 | H.222.0 | `HEVC_timing_and_HRD_descriptor` | 03/2017 | `reserved`(6) | `target_schedule_idx_not_present_flag`(1) + `target_schedule_idx`(5) | n/a | **not implemented** |
| 12 | H.222.0 | `MPEG-H_3dAudio_descriptor` | 08/2023 | `reserved`(9) | `compatibleProfileSetsPresent`(1) + `reserved`(8) | n/a | **not implemented** |
| 13 | H.222.0 | `adaptation_field_extension` | 03/2017 | `reserved`(5) | `af_descriptor_not_present_flag`(1) + `reserved`(4) | 1 = "AF descriptor not present" - compatible | **mpeg-ts**: untyped (flags byte `ts.rs`) |
| 14 | H.222.0 | `J2K_video_descriptor` | 08/2018 | `profile_and_level`(16) | `extended_capability_flag`(1)+`profile_and_level`(15); conditional `reserved`("all bits to be set to '0'")(5) | n/a | typed (j2k_video.rs) |
| 15 | TS 102 323 | `link_info` | V1.3.1 (2008-04) | `reserved`(4) before `descriptor_loop_length` | `default_icon_flag`(1)+`icon_id`(3); head restructured | n/a | typed (rct.rs) |
| 16 | TS 102 323 | `related_content_section` | V1.3.1 | byte-1 `reserved`(1) and `reserved`(16) | `table_id_extension_flag`, `service_id`(16) | 1 / 0xFFFF = "single service, ignore service_id" | typed |
| 17 | TS 101 812 -> TS 102 809 | `application_signalling_descriptor` | (inverse) | MHP: `application_type`(16) | TS 102 809: `reserved_future_use`(1)+`application_type`(15) | see §3.2 | typed |
| 18 | TS 102 809 / TS 101 812 V1.3.1 | AIT section | TS 101 812 V1.3.1 (2003-06) | `application_type`(16) | `test_application_flag`(1)+`application_type`(15) | n/a | typed |
| 19 | TS 101 154 | `AD_descriptor` (PES private data, not SI) | V1.9.1 (2009-09) | `Reserved` 0xFFFFFF(24) | `AD_gain_byte_center/front/surround` (version_text_tag 0x32) | n/a | not in dvb-si |

Rows 1-6 and 14 are verified in the page images / text of the cited editions; the others by text layout (see §0.4).

## 5. The descriptors flipped to `1` in dvb-si 11.1.0 and the neighbours listed by the owner

Governing text: H.222.0 clause 2.1 "reserved": **all `reserved` bits are `1`** (all 9 editions, 07/1995-08/2023); the 04/2025 edition is UNVERIFIED. Layout = per-edition syntax table (editions in which the descriptor exists).

| dvb-si descriptor | Tag | Reserved field(s) | Editions checked | Layout change in any edition? | Verdict |
|---|---|---|---|---|---|
| video_stream | 0x02 | `reserved`(5) tail of the MPEG_1_only_flag=0 branch | 9/9 | none | still reserved, `1` |
| audio_stream | 0x03 | `reserved`(3) | 9/9 | none | still reserved, `1` |
| hierarchy | 0x04 | see §4 row 7 | 9/9 | **carve-outs 2012 and 2017** | dvb-si follows 2017+ layout; flags `no_*`=1, `tref_present_flag`=1 default; emitted reserved bits (byte 4 bit 6, bytes 3/5 top 2 bits) are `reserved` in all editions |
| system_clock | 0x0B | `reserved`(1) after `external_clock_reference_indicator`, `reserved`(5) tail | 9/9 | none | still reserved, `1` |
| multiplex_buffer_utilization | 0x0C | `reserved`(1) between the bounds | 9/9 | 1995/2000 upper bound printed 14 bits | still reserved, `1` |
| maximum_bitrate | 0x0E | `reserved`(2) | 9/9 | none | still reserved, `1` |
| smoothing_buffer | 0x10 | `reserved`(2) x2 | 9/9 | none | still reserved, `1` |
| std | 0x11 | `reserved`(7) | 9/9 | none | still reserved, `1` |
| content_labeling | 0x24 | `reserved`(3) after `content_time_base_indicator` | 7/7 (2006+) | none | still reserved, `1` |
| metadata_pointer | 0x25 | `reserved`(5) after `MPEG_carriage_flags`(2) | 7/7 | none (flag semantics widen in 2017) | still reserved, `1` |
| metadata | 0x26 | `reserved`(4) after `DSM-CC_flag` | 7/7 | none | still reserved, `1` |
| metadata_std | 0x27 | `reserved`(2) x3 | 7/7 | none | still reserved, `1` |
| avc_video | 0x28 | `reserved`(5) | 8/8 (2006+) | **carve-out 2012** | follows 2012+ layout |
| avc_timing_and_hrd | 0x2A | `reserved`(6), `reserved`(7), `reserved`(5) | 8/8 (read manually; tool missed the spaced name) | none | still reserved, `1` |
| svc_extension | 0x30 | `reserved`(5) after `dependency_id`(3); trailing `reserved`(1) | 6/6 (2012+) | none | still reserved, `1` |
| mvc_extension | 0x31 | `reserved`(2) | 6/6 | **carve-out 2014** | follows 2014+ layout |
| hevc_video | 0x38 | `reserved`(2) + `HDR_WCG_idc`(2); `reserved`(5) x2 in the temporal branch | 4/4 (2017+) | `copied_44bits`/`HDR_WCG_idc` carved in 2017 | **dvb-si bit 3 of byte 12 = 0 contradicts `reserved`(2)** (§9) |
| SSU selector in data_broadcast_id (0x66, id 0x000A) | - | TS 102 006 `reserved`(4) before `update_type`, `reserved`(2) before `update_versioning_flag` | 4/4 (V1.2.1-V1.4.1) | none | **value not stated by any edition** (selector `reserved` rows carry no value, §2); `1` is an analogy - UNVERIFIED |

## 6. Full inventory (generated from the per-edition syntax diff)

Columns: structure | editions in which the structure was found (EN 300 468: `1.x` = V1.x.1; H.222.0: `YYYYMM`) | reserved layout in the **latest** edition (`=value` per global convention; **dvb-si emit: convention value unless a row/§9 says otherwise - code-checked only for masks listed in the note**) | history of the reserved layout (only when it changes) | dvb-si module | class and note.

*Emit cross-check:* `grep`/script over `dvb-si/src/{descriptors,tables}` finds a `const *RESERVED*` mask that is OR-ed into the output (>=1 use outside tests) in: aac (0), announcement_support, audio_stream, avc_timing_and_hrd, avc_video, cable_delivery_system (rfu(12)), country_availability, data_broadcast_id (SSU), frequency_list, fta_content_management, hevc_video (**bit 3 = 0**), hierarchy, linkage, local_time_offset, logical_channel (NorDig), maximum_bitrate, metadata, metadata_pointer, multiplex_buffer_utilization, mvc_extension, pdc, s2_satellite_delivery_system, service_availability, smoothing_buffer, std, svc_extension, system_clock, telephone, terrestrial_delivery_system (rfu(32)=0xFFFFFFFF), video_stream, protection_message, rct, rnt; plus inline `0x0F`/`0xF0` style fills elsewhere (AC-3 `flags | 0x0F`).

### 6.1 EN 300 468 (18 editions)

| Structure | Editions | Latest reserved layout | History | dvb-si | Class / note |
|---|---|---|---|---|---|
| `aac__descriptor` | 01.07-01.10 | `r(1)=1 r(1)=1 r(1)=1 r(1)=1 r(1)=1 r(1)=1 r(1)=1` | - | not implemented in dvb-si | **E** extraction noise (7 bare `reserved` bits of the MPEG-2 AAC annex text, V1.7-V1.10); not the 0x7C AAC descriptor |
| `aac_descriptor` | 01.11-01.20 | `rzfu(6)=0` | 01.11..01.14: `r(7)=1`; 01.15: `-`; 01.16..01.20: `rzfu(6)=0` | `descriptors/aac.rs` | **C** 1.11-1.14 `AAC_type_flag`+`reserved`(7) =1; **1.15 carves `SAOC_DE_flag` out of bit 6**, `reserved_future_use`(6); 1.16+ `reserved_zero_future_use`(6) **=0**. dvb-si emits 0 and types SAOC_DE_flag (aac.rs:349) |
| `ac-3__descriptor` | 01.04-01.10 | `reserved_flags(4)=0 [explicit "should always be 0"]` | - | `descriptors/ac3.rs` | **D** AC-3 `reserved_flags`(4): "should always be set to 0" in EVERY edition (1.4.1-1.20.1) - see §3.1 |
| `ac-3_descriptor` | 01.11-01.20 | `reserved_flags(4)=0 [explicit "should always be 0"]` | - | `descriptors/ac3.rs` | **D** AC-3 `reserved_flags`(4): "should always be set to 0" in EVERY edition - see §3.1. **dvb-si emits 1111 (ac3.rs:350) = mismatch** |
| `ac-4_descriptor` | 01.15-01.20 | `rzfu(6)=0 rzfu(5)=0` | 01.15: `-`; 01.16..01.20: `rzfu(6)=0 rzfu(5)=0` | `descriptors/extension/ac4.rs` | **A** value 0 in all editions (1.15 "reserved_future_use (see note)" + NOTE=0; 1.16+ `reserved_zero_future_use`) |
| `announcement_support_descriptor` | 01.04-01.20 | `rfu(1)=1` | - | `descriptors/announcement_support.rs` | **A** 1.12 text artefact only |
| `asset_info` | 01.14-01.20 | `rfu(2)=1` | - | not implemented in dvb-si | **A**  |
| `audio_preselection_descriptor` | 01.16-01.20 | `rzfu(3)=0 rzfu(5)=0 rzfu(3)=0` | 01.16: `rzfu(3)=0 rzfu(5)=0 rzfu(3)=0 r(3)=1 r(4)=1 r(5)=1 r(1)=1`; 01.17..01.20: `rzfu(3)=0 rzfu(5)=0 rzfu(3)=0` | `descriptors/extension/audio_preselection.rs` | **A** zero-class since introduction (1.16); 1.16 history row is extraction noise |
| `beamhopping_time_plan_info` | 01.18-01.20 | `rzfu(4)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0 rzfu(1)=0 rzfu(1)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0` | - | `tables/sat.rs` | **A** zero-class |
| `bouquet_association_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 r(2)=1 rfu(4)=1 rfu(4)=1 rfu(4)=1` | - | `tables/bat.rs` | **A**  |
| `c2_bundle_delivery_system_descriptor` | 01.15-01.20 | `rzfu(7)=0` | 01.15: `rfu(7)=1`; 01.16..01.20: `rzfu(7)=0` | `descriptors/extension/c2_bundle_delivery_system.rs` | **B** 1.15 `reserved_0_future_use`(7) -> 1.16+ `reserved_zero_future_use`(7); value 0 throughout (master_channel renamed primary_channel in 1.18) |
| `c2_delivery_system_descriptor` | 01.12-01.20 | `-` | 01.12: `r(2)=1 r(4)=1 r(5)=1 r(1)=1 r(2)=1 rfu(2)=1`; 01.13..01.15: `r(4)=1 r(5)=1 r(1)=1 r(2)=1`; 01.17..01.20: `-` | `descriptors/extension/c2_delivery_system.rs` | **E** blocks merged by extraction; latest layout = no reserved bits in the C2 descriptor itself (1.17+ table) |
| `cable_delivery_system_descriptor` | 01.03-01.20 | `rfu(12)=1` | - | `descriptors/cable_delivery_system.rs` | **A** `reserved_future_use`(12) before FEC_outer, all editions |
| `cell_fragment_info` | 01.18-01.20 | `rzfu(4)=0 rzfu(5)=0 rzfu(6)=0 rzfu(4)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0 rzfu(6)=0` | - | `tables/sat.rs` | **A** 1.20.1: spelling fixes only (occurence->occurrence, center->centre) |
| `cell_frequency_link_descriptor` | 01.04-01.20 | `-` | 01.04..01.13: `rfu(4)=1`; 01.14..01.20: `-` | `descriptors/cell_frequency_link.rs` | **E** block merged with cell_list_descriptor by extraction |
| `component_descriptor` | 01.03-01.20 | `-` | 01.03,01.14: `rfu(4)=1`; 01.15..01.20: `-` | `descriptors/component.rs` | **C** **1.15 carves `stream_content_ext`(4) out of `reserved_future_use`(4)** (NOTE 8: n/a for stream_content 0x01-0x08 and set to 0xF -> the old all-ones fill is the defined "not applicable" value) |
| `country_availability_descriptor` | 01.03-01.20 | `rfu(7)=1` | - | `descriptors/country_availability.rs` | **A**  |
| `cp_descriptor` | 01.09-01.20 | `rfu(3)=1` | 01.09..01.15: `r(3)=1`; 01.17..01.20: `rfu(3)=1` | `descriptors/extension/cp.rs` | **B** `reserved`(3) (1.9-1.15) -> `reserved_future_use`(3) (1.17+) |
| `discontinuity_information_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 rfu(7)=1` | - | `tables/dit.rs` | **A**  |
| `dts-hd_audio_stream_descriptor` | 01.14,01.15 | `rfu(3)=1` | - | not implemented in dvb-si | **A**  |
| `dts-hd_descriptor` | 01.16-01.20 | `rfu(3)=1` | - | `descriptors/extension/dts_hd.rs` | **A**  |
| `event_information_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 r(2)=1` | - | `tables/eit.rs` | **A**  |
| `event_linkage_info` | 01.12-01.20 | `rfu(6)=1` | 01.12..01.16: `r(6)=1`; 01.17..01.20: `rfu(6)=1` | `descriptors/linkage.rs` | **B** `reserved`(6) -> `reserved_future_use`(6) |
| `frequency_list_descriptor` | 01.03-01.20 | `rfu(6)=1` | - | `descriptors/frequency_list.rs` | **A**  |
| `fta_content_management_descriptor` | 01.09-01.20 | `rfu(3)=1` | 01.09..01.11: `-`; 01.12..01.20: `rfu(3)=1` | `descriptors/fta_content_management.rs` | **C** 1.9-1.11 `reserved_future_use`(4); **1.12 carves `user_defined`(1) out (bit 7)**, `reserved_future_use`(3); "if no semantics are defined, it shall be set to zero" (1.20.1) - an old all-ones sender now reads as user_defined=1 |
| `image_icon_descriptor` | 01.09-01.20 | `rfu(5)=1 rfu(2)=1 rfu(5)=1` | 01.09: `rfu(5)=1 rfu(2)=1 rfu(5)=1 rfu(1)=1 r(2)=1 rfu(7)=1 rfu(1)=1 r(2)=1 rfu(16)=1 r(2)=1 rfu(4)=1 rfu(1)=1 rfu(2)=1 rfu(2)=1 rfu(2)=1`; 01.10..01.12: `rfu(5)=1 rfu(2)=1 rfu(5)=1 r(1)=1`; 01.14..01.20: `rfu(5)=1 rfu(2)=1 rfu(5)=1` | `descriptors/extension/image_icon.rs` | **E** blocks merged by extraction; latest per-field layout read from 1.19.1/1.20.1 |
| `linkage_descriptor` | 01.03-01.20 | `-` | 01.03: `-`; 01.04..01.09: `rfu(3)=1`; 01.10,01.11: `rfu(3)=1 r(6)=1`; 01.12..01.20: `-` | `descriptors/linkage.rs` | **E** blocks merged by extraction (hand-over/event-linkage sub-structures listed separately) |
| `local_time_offset_descriptor` | 01.03-01.20 | `rfu(1)=1` | 01.03..01.16: `r(1)=1`; 01.17..01.20: `rfu(1)=1` | `descriptors/local_time_offset.rs` | **B** `reserved`(1) -> `reserved_future_use`(1) |
| `mobile_hand-over_info` | 01.12-01.20 | `rfu(3)=1` | - | `descriptors/linkage.rs` | **A**  |
| `mosaic_descriptor` | 01.03-01.20 | `rfu(1)=1 rfu(7)=1 rfu(2)=1` | - | `descriptors/mosaic.rs` | **A**  |
| `network_information_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 r(2)=1 rfu(4)=1 rfu(4)=1 rfu(4)=1` | - | `tables/nit.rs` | **A**  |
| `partial_transport_stream_descriptor` | 01.03-01.20 | `rfu(2)=1 rfu(2)=1 rfu(2)=1` | - | `descriptors/partial_transport_stream.rs` | **A** `DVB_reserved_future_use`(2) x3 -> `reserved_future_use` (1.17), same value |
| `pdc_descriptor` | 01.04-01.20 | `rfu(4)=1` | - | `descriptors/pdc.rs` | **A** `reserved_future_use`(4) |
| `running_status_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 rfu(5)=1` | - | `tables/rst.rs` | **A**  |
| `s2_satellite_delivery_system_descriptor` | 01.07-01.20 | `rzfu(1)=0 rfu(2)=1 rfu(6)=1` | 01.07..01.15: `rfu(5)=1 r(6)=1`; 01.16..01.20: `rzfu(1)=0 rfu(2)=1 rfu(6)=1` | `descriptors/s2_satellite_delivery_system.rs` | **C** **1.7-1.15 bit 5 = `backwards_compatibility_indicator`** (1=LP stream of a backward-compat carrier); 1.16 turns it into `reserved_zero_future_use`(1) and carves `not_timeslice_flag` (bit 4) + `TS_GS_mode`(2) out of the old `reserved_future_use`(5); bits[3:2] stay `reserved_future_use`(2) |
| `s2x_satellite_delivery_system_descriptor` | 01.16-01.20 | `rzfu(3)=0 rzfu(3)=0 rzfu(6)=0 rzfu(1)=0 rzfu(4)=0 rzfu(7)=0 rzfu(1)=0 rzfu(4)=0 rfu(8)=1` | - | `descriptors/extension/s2x_satellite_delivery_system.rs` | **A** zero-class since introduction (1.16) |
| `s2xv2_satellite_delivery_system_info` | 01.18-01.20 | `rzfu(2)=0 rzfu(1)=0 rzfu(6)=0 rzfu(7)=0 rzfu(2)=0 rzfu(4)=0 rzfu(8)=0` | 01.18: `rzfu(3)=0 rzfu(1)=0 rzfu(6)=0 rzfu(7)=0 rzfu(2)=0 rzfu(4)=0 rzfu(3)=0 rzfu(8)=0`; 01.19,01.20: `rzfu(2)=0 rzfu(1)=0 rzfu(6)=0 rzfu(7)=0 rzfu(2)=0 rzfu(4)=0 rzfu(8)=0` | `descriptors/extension/s2xv2_satellite_delivery_system.rs` | **C** **1.19 carves `NCR_reference`(1) out of `reserved_zero_future_use`(3)** (1.18: roll_off(3) rzfu(3) NCR_version(1); 1.19: roll_off(3) rzfu(2) NCR_reference(1) NCR_version(1)); dvb-si already typed |
| `satellite_access_section` | 01.18-01.20 | `r(2)=1 r(2)=1 rzfu(8)=0 rzfu(8)=0` | 01.18: `r(2)=1 r(2)=1 rzfu(8)=0`; 01.19,01.20: `r(2)=1 r(2)=1 rzfu(8)=0 rzfu(8)=0` | `tables/sat.rs` | **A** 1.19 adds a second satellite_table_id branch with its own `reserved_zero_future_use`(8) |
| `satellite_delivery_system_descriptor` | 01.03-01.20 | `rzfu(2)=0` | 01.03..01.15: `-`; 01.16..01.20: `rzfu(2)=0` | `descriptors/satellite_delivery_system.rs` | **D** DVB-S: the 2 bits are literal "00" (1.7-1.15) / `reserved_zero_future_use`(2) (1.16+); 1.3-1.6 had a 5-bit `modulation` field (QPSK=00001, 8PSK=00010, 16QAM=00011) - see §3.5 |
| `satellite_position_v2_info` | 01.18-01.20 | `rzfu(7)=0` | - | `tables/sat.rs` | **A** zero-class; 1.20.1 reword: "reserved" -> "undefined and not used" when interpolation_flag==0 |
| `satellite_position_v3_info` | 01.19,01.20 | `rzfu(7)=0 rzfu(3)=0 rzfu(7)=0 rzfu(7)=0 rzfu(1)=0 rzfu(7)=0 rzfu(7)=0 rzfu(7)=0 rzfu(7)=0` | - | `tables/sat.rs` | **A** zero-class |
| `selection_information_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 rfu(16)=1 r(2)=1 rfu(4)=1 rfu(1)=1` | - | `tables/sit.rs` | **A**  |
| `service_availability_descriptor` | 01.13-01.20 | `rfu(7)=1` | 01.13..01.16: `r(7)=1`; 01.17..01.20: `rfu(7)=1` | `descriptors/service_availability.rs` | **B** `reserved`(7) -> `reserved_future_use`(7) |
| `service_availbility_descriptor` | 01.05-01.12 | `r(7)=1` | - | `descriptors/service_availability.rs` | **B** same descriptor under its (mis-spelled) 1.5-1.12 name; `reserved`(7) |
| `service_description_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 r(2)=1 rfu(8)=1 rfu(6)=1` | - | `tables/sdt.rs` | **A**  |
| `service_prominence_descriptor` | 01.19,01.20 | `rfu(1)=1 rfu(5)=1` | - | `descriptors/extension/service_prominence.rs` | **A** new in 1.19 |
| `sh_delivery_system_descriptor` | 01.10-01.20 | `rfu(4)=1 rfu(5)=1 rfu(1)=1 rfu(2)=1` | 01.10,01.11: `r(4)=1 r(5)=1 r(1)=1 r(2)=1 rfu(2)=1`; 01.17..01.20: `rfu(4)=1 rfu(5)=1 rfu(1)=1 rfu(2)=1` | `descriptors/extension/sh_delivery_system.rs` | **B** `reserved`(4/5/1/2) -> `reserved_future_use` (1.17); 1.10/1.11 extraction merged neighbouring tables |
| `short_smoothing_buffer_descriptor` | 01.03-01.20 | `rfu(8)=1` | 01.03..01.16: `r(8)=1`; 01.17..01.20: `rfu(8)=1` | `descriptors/short_smoothing_buffer.rs` | **B** `DVB_reserved`(8) -> `reserved_future_use`(8) |
| `stuffing_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1` | - | `tables/st.rs` | **A**  |
| `substream_info` | 01.14-01.20 | `rfu(2)=1` | - | not implemented in dvb-si | **A**  |
| `supplementary_audio_descriptor` | 01.13-01.20 | `rfu(1)=1` | 01.13,01.14: `r(1)=1`; 01.15..01.20: `rfu(1)=1` | `descriptors/extension/supplementary_audio.rs` | **B** 1.13/1.14 `reserved`(1); 1.15+ `reserved_future_use`(1) (CR 1041, V1.15.1 history: "a reserved field should be reserved for future use in supplementary audio descriptor"); value 1 in all - see §3.3 |
| `t2_delivery_system_descriptor` | 01.13-01.20 | `rfu(2)=1` | 01.13: `rfu(2)=1 rfu(5)=1 rfu(2)=1 rfu(5)=1`; 01.14..01.20: `rfu(2)=1` | `descriptors/extension/t2_delivery_system.rs` | **E** blocks merged by extraction; reserved_future_use(2) after bandwidth(4), all editions |
| `t2mi_descriptor` | 01.13-01.20 | `rzfu(5)=0 rzfu(5)=0 rzfu(7)=0 rzfu(8)=0` | 01.13..01.15: `-`; 01.16..01.20: `rzfu(5)=0 rzfu(5)=0 rzfu(7)=0 rzfu(8)=0` | `descriptors/extension/t2mi.rs` | **B** 1.13-1.15 `reserved_future_use (see note)` + NOTE "set to 0"; 1.16+ `reserved_zero_future_use`; value 0 throughout |
| `target_region_descriptor` | 01.11-01.20 | `rfu(5)=1` | 01.11..01.13: `-`; 01.14..01.16: `r(5)=1`; 01.17..01.20: `rfu(5)=1` | `descriptors/extension/target_region.rs` | **B** `reserved`(5) (1.11-1.16) -> `reserved_future_use`(5) (1.17+); **1.17.1 erratum: tertiary_region_code printed as 8 bits (16 elsewhere)** |
| `telephone_descriptor` | 01.03-01.20 | `rfu(2)=1 rfu(1)=1 rfu(1)=1` | 01.03..01.16: `rfu(2)=1 rfu(1)=1 rfu(1)=1`; 01.17: `rfu(2)=1 rfu(5)=1 rfu(1)=1`; 01.18..01.20: `rfu(2)=1 rfu(1)=1 rfu(1)=1` | `descriptors/telephone.rs` | **A** **1.17.1 erratum only**: syntax printed `reserved`(5)+country_prefix_length(1)+international_area_code_length(2) (26 bits); 1.18.1 restores `reserved_future_use`(1)+(2)+(3) |
| `terrestrial_delivery_system_descriptor` | 01.03-01.20 | `rfu(2)=1 rfu(32)=1` | 01.03..01.05: `rfu(5)=1 rfu(32)=1`; 01.06..01.20: `rfu(2)=1 rfu(32)=1` | `descriptors/terrestrial_delivery_system.rs` | **C** 1.3-1.5 `reserved_future_use`(5) after bandwidth; **1.6 carves `priority`,`Time_Slicing_indicator`,`MPE-FEC_indicator`** out of it, leaving (2). Old all-ones fill = HP, "time slicing not used", "MPE-FEC not used" (designed-compatible). dvb-si typed |
| `time_association_info` | 01.18-01.20 | `rzfu(4)=0 rzfu(6)=0` | - | `tables/sat.rs` | **A** zero-class |
| `time_date_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1` | - | `tables/tdt.rs` | **A**  |
| `time_offset_section` | 01.03-01.20 | `rfu(1)=1 r(2)=1 r(4)=1` | - | `tables/tot.rs` | **A**  |
| `vbi_data_descriptor` | 01.04-01.20 | `rfu(2)=1 rfu(8)=1` | 01.04..01.16: `r(2)=1 r(8)=1`; 01.17..01.20: `rfu(2)=1 rfu(8)=1` | `descriptors/vbi_data.rs` | **B** `reserved`(2),(8) -> `reserved_future_use` |
| `vvc_subpictures_descriptor` | 01.18-01.20 | `rzfu(5)=0` | - | `descriptors/extension/vvc_subpictures.rs` | **A** zero-class (new 1.18) |

### 6.2 H.222.0 (9 editions)

| Structure | Editions | Latest reserved layout | History | dvb-si | Class / note |
|---|---|---|---|---|---|
| `adaptation_field` | 199507-202308 | `r(6)=1 r(6)=1 r(4)=1 r(2)=1 r(8)=1` | 199507..201410: `r(6)=1 r(6)=1 r(5)=1 r(2)=1 r(8)=1`; 201703..202308: `r(6)=1 r(6)=1 r(4)=1 r(2)=1 r(8)=1` | not implemented in dvb-si | **C** **2017 carves `af_descriptor_not_present_flag`(1) out of the 5-bit `reserved` in adaptation_field_extension** (<=2014: reserved(5); 2017+: flag(1)+reserved(4)). Lives in mpeg-ts (`ts.rs`), not dvb-si |
| `audio_stream_descriptor` | 199507-202308 | `r(3)=1` | - | `descriptors/audio_stream.rs` | **A**  |
| `avc_timing_and_hrd_descriptor` | 200605-202308 | `r(6)=1 r(7)=1 r(5)=1` | - | `descriptors/avc_timing_and_hrd.rs` | **A** present since 2006 (name printed with spaces in 2006-2014 so the tool missed it; layout read manually: reserved 6 / 7 / 5 in all editions) |
| `avc_video_descriptor` | 200605-202308 | `r(5)=1` | 200605: `r(6)=1`; 201206..202308: `r(5)=1` | `descriptors/avc_video.rs` | **C** 2006 `constraint_set0..2`+`AVC_compatible_flags`(5) and `reserved`(6); **2012 carves `constraint_set3/4/5_flag` out of AVC_compatible_flags (->2 bits) and `Frame_Packing_SEI_not_present_flag` out of reserved (->5)**. dvb-si typed |
| `boundary_descriptor` | 201703-202308 | `r(4)=1 r(6)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `ca_descriptor` | 199507-202308 | `r(3)=1` | - | `tables/cat.rs` | **A** `reserved`(3) before CA_PID, all 9 editions |
| `ca_section` | 199507-202308 | `r(2)=1 r(18)=1` | - | `tables/cat.rs` | **A**  |
| `content_labeling_descriptor` | 200605-202308 | `r(3)=1 r(7)=1 r(7)=1 r(1)=1 r(8)=1` | - | `descriptors/content_labeling.rs` | **A** reserved(3) after content_time_base_indicator, 2006-2023 |
| `evc_timing_and_hrd_descriptor` | 202106,202308 | `r(6)=1 r(7)=1` | - | not implemented in dvb-si | **A** new 2021 |
| `evc_video_descriptor` | 202106,202308 | `r(1)=1 r(2)=1 r(5)=1 r(5)=1` | - | not implemented in dvb-si | **A** new 2021 |
| `extension_descriptor` | 201410-202308 | `r(8)=1` | - | not implemented in dvb-si | **E** tag table grows per edition; no reserved-layout change evidenced |
| `green_access_unit_section_message` | 201703-202308 | `r(2)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `green_extension_descriptor` | 201703-202308 | `r(6)=1 r(6)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `hevc_descriptor` | 201410 | `rzfu(44)=0 r(4)=1 r(5)=1 r(5)=1` | - | not implemented in dvb-si | **C** 2014 draft-era `HEVC_descriptor`: `reserved_zero_44bits`(44) + `reserved`(4); **2017 renames to HEVC_video_descriptor: `copied_44bits`(44) (now carries meaning) and carves `HDR_WCG_idc`(2) out of the 4 reserved bits** |
| `hevc_hierarchy_extension_descriptor` | 201703-202308 | `r(2)=1 r(2)=1 r(2)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `hevc_operation_point_descriptor` | 201703-202308 | `r(2)=1 r(1)=1 r(2)=1 r(1)=1 r(4)=1` | - | not implemented in dvb-si | **A** 2018 print has typos |
| `hevc_subregion_descriptor` | 201808-202308 | `r(1)=1 r(1)=1` | - | not implemented in dvb-si | **A** new 2018 |
| `hevc_timing_and_hrd_descriptor` | 201410-202308 | `r(7)=1` | 201410: `r(6)=1 r(7)=1`; 201703..202308: `r(7)=1` | not implemented in dvb-si | **C** 2014: `hrd_management_valid_flag`, `reserved`(6), `picture_and_timing_info_present_flag`; **2017 carves `target_schedule_idx_not_present_flag`(1) + `target_schedule_idx`(5) out of the 6 reserved bits**. Not implemented in dvb-si |
| `hevc_video_descriptor` | 201703-202308 | `r(2)=1 r(5)=1 r(5)=1` | - | `descriptors/hevc_video.rs` | **C** 2017-2023: `reserved`(2) + `HDR_WCG_idc`(2) (tail of byte 12). **dvb-si emits bit 3 = 0, bit 2 = 1 (hevc_video.rs:26-30) although H.222.0 (2017-2023) lists both as `reserved` (=1 by definition)** - see §9 |
| `hierarchy_descriptor` | 199507-202308 | `r(2)=1 r(1)=1 r(2)=1` | 199507..200605: `r(4)=1 r(2)=1 r(2)=1 r(2)=1`; 201206,201410: `r(1)=1 r(2)=1 r(1)=1 r(2)=1`; 201703..202308: `r(2)=1 r(1)=1 r(2)=1` | `descriptors/hierarchy.rs` | **C** **2006: `reserved`(4)+hierarchy_type(4); 2012 carves `temporal/spatial/quality_scalability_flag` (3 bits) out of it + `tref_present_flag` out of the top reserved bit of byte 4; 2017 carves `no_view_scalability_flag` out of the last reserved bit** and renames the flags `no_*` (value 1 = absent). dvb-si typed |
| `iso_iec_14496_section` | 200002-202308 | `r(2)=1 r(2)=1 r(8)=1` | - | not implemented in dvb-si | **A**  |
| `j2k_elsm_header` | 201808-202308 | `r(8)=1 r(8)=1 r(23)=1` | - | not implemented in dvb-si | **A**  |
| `j2k_video_descriptor` | 201206-202308 | `r(5)=0 [explicit, extended branch] r(6)=1 r(7)=1` | 201206..201703: `r(6)=1`; 201808..202308: `r(6)=1 r(7)=1` | `descriptors/j2k_video.rs` | **D** 2012-2017 `reserved`(6); **2018+ adds `extended_capability_flag` (carved from profile_and_level 16->15) and a branch with `reserved (all bits to be set to '0')`(5)** |
| `jxes_header` | 202106,202308 | `r(7)=1` | - | not implemented in dvb-si | **A**  |
| `jxs_video_descriptor` | 202106,202308 | `r(7)=1` | - | not implemented in dvb-si | **A**  |
| `lcevc_video_descriptor` | 202308 | `r(3)=1 rzfu(2)=0` | - | not implemented in dvb-si | **A** new 2023 |
| `maximum_bitrate_descriptor` | 199507-202308 | `r(2)=1` | - | `descriptors/maximum_bitrate.rs` | **A** `reserved`(2), all 9 editions |
| `media_service_kind_descriptor` | 202308 | `r(1)=1 r(1)=1` | - | not implemented in dvb-si | **A** new 2023 |
| `metadata_descriptor` | 200605-202308 | `r(4)=1 r(8)=1 r(8)=1` | - | `descriptors/metadata.rs` | **A** `reserved`(4) after DSM-CC_flag, 2006-2023 |
| `metadata_pointer_descriptor` | 200605-202308 | `r(5)=1` | - | `descriptors/metadata_pointer.rs` | **A** `reserved`(5) after MPEG_carriage_flags, 2006-2023 (2017 only widens the carriage-flags semantics to <=2) |
| `metadata_section` | 200605-202308 | `r(8)=1` | - | `descriptors/metadata.rs` | **A**  |
| `metadata_std_descriptor` | 200605-202308 | `r(2)=1 r(2)=1 r(2)=1` | - | `descriptors/metadata_std.rs` | **A** `reserved`(2) x3, 2006-2023 |
| `mpeg-4_audio_extension_descriptor` | 201206-202308 | `r(3)=1` | - | `descriptors/mpeg4_audio.rs` | **A**  |
| `mpeg-h_3daudio_descriptor` | 201703-202308 | `r(8)=1 r(8)=1` | 201703..202106: `r(9)=1 r(8)=1`; 202308: `r(8)=1 r(8)=1` | not implemented in dvb-si | **C** 2017-2021 `reserved`(9) (2021 text already tests `compatibleProfileSetsPresent` but never defines it); **2023 carves `compatibleProfileSetsPresent`(1) out of it**. Not implemented in dvb-si |
| `mpeg-h_3daudio_drc_loudness_descriptor` | 201703-202308 | `r(7)=1 r(2)=1 r(2)=1 r(3)=1 r(6)=1 r(1)=1 r(3)=1 r(2)=1 r(1)=1 r(3)=1 r(1)=1 r(1)=1 r(2)=1 r(1)=1 r(1)=1 r(6)=1 r(1)=1 r(3)=1 r(1)=1 r(8)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `mpeg-h_3daudio_multi-stream_descriptor` | 201703-202308 | `r(1)=1 r(1)=1 r(8)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `mpeg-h_3daudio_scene_descriptor` | 201703-202308 | `r(5)=1 r(1)=1 r(1)=1 r(3)=1 r(4)=1 r(1)=1 r(1)=1 r(3)=1 r(3)=1 r(2)=1 r(3)=1 r(3)=1 r(1)=1 r(3)=1 r(1)=1 r(1)=1 r(3)=1 r(3)=1 r(3)=1 r(4)=1 r(4)=1 r(2)=1 r(4)=1 r(8)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `mpeg-h_3daudio_text_label_descriptor` | 201703-202308 | `r(4)=1 r(1)=1 r(1)=1 r(3)=1 r(3)=1 r(3)=1 r(3)=1 r(8)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `mpeg2_stereoscopic_video_format_descriptor` | 201703-202308 | `r(7)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `multiplex_buffer_utilization_descriptor` | 199507-202308 | `r(1)=1` | - | `descriptors/multiplex_buffer_utilization.rs` | **A** `reserved`(1) between lower and upper bound, all editions. **1995 and 2000 print LTW_offset_upper_bound as 14 bits (31-bit body); 2006+ 15 bits** |
| `mvc_extension_descriptor` | 201206-202308 | `r(2)=1` | 201206: `r(4)=1`; 201410..202308: `r(2)=1` | `descriptors/mvc_extension.rs` | **C** 2012 `reserved`(4); **2014 carves `view_association_not_present_flag`,`base_view_is_left_eyeview` out of it** (reserved(2) left). dvb-si typed |
| `mvc_operation_point_descriptor` | 201206-202308 | `r(5)=1 r(2)=1` | - | not implemented in dvb-si | **A**  |
| `pack_header` | 199507-202308 | `r(5)=1` | - | not implemented in dvb-si | **A** PS pack header; mpeg-ps |
| `private_section` | 199507-202308 | `r(2)=1 r(2)=1` | - | not implemented in dvb-si | **A**  |
| `program_association_section` | 199507-202308 | `r(2)=1 r(2)=1 r(3)=1` | - | `tables/pat.rs` | **A**  |
| `smoothing_buffer_descriptor` | 199507-202308 | `r(2)=1 r(2)=1` | - | `descriptors/smoothing_buffer.rs` | **A** `reserved`(2) x2, all editions |
| `std_descriptor` | 199507-202308 | `r(7)=1` | - | `descriptors/std.rs` | **A** `reserved`(7), all editions |
| `stereoscopic__video_info_descriptor` | 201206-202308 | `r(7)=1 r(7)=1 r(7)=1` | - | not implemented in dvb-si | **A**  |
| `stereoscopic_program_info_descriptor` | 201206-202308 | `r(5)=1` | - | not implemented in dvb-si | **A**  |
| `svc_extension_descriptor` | 201206-202308 | `r(5)=1 r(1)=1` | - | `descriptors/svc_extension.rs` | **A** `reserved`(5) after dependency_id(3) and trailing `reserved`(1), 2012-2023 |
| `system_clock_descriptor` | 199507-202308 | `r(1)=1 r(5)=1` | - | `descriptors/system_clock.rs` | **A** `reserved`(1) after external_clock_reference_indicator, `reserved`(5) tail; all editions |
| `system_header` | 199507-202308 | `r(7)=1` | - | not implemented in dvb-si | **A** PS system header; mpeg-ps |
| `ts_description_section` | 200002-202308 | `r(2)=1 r(18)=1` | - | `tables/tsdt.rs` | **A**  |
| `ts_program_map_section` | 199507-202308 | `r(2)=1 r(2)=1 r(3)=1 r(4)=1 r(3)=1 r(4)=1` | - | `tables/pmt.rs` | **A**  |
| `video_stream_descriptor` | 199507-202308 | `r(5)=1` | - | `descriptors/video_stream.rs` | **A** `reserved`(5) in the MPEG_1_only_flag==0 branch, all 9 editions |
| `virtual_segmentation_descriptor` | 201703-202308 | `r(4)=1 r(1)=1 r(4)=1 r(5)=1 r(3)=1` | - | not implemented in dvb-si | **A** new 2017 |
| `vvc_timing_and_hrd_descriptor` | 202106,202308 | `r(6)=1 r(7)=1` | - | not implemented in dvb-si | **A** new 2021 |
| `vvc_video_descriptor` | 202106,202308 | `rzfu(4)=0 r(5)=1 r(2)=1 r(5)=1 r(5)=1` | - | not implemented in dvb-si | **A** new 2021 |

### 6.3 Other specs

| Spec | Structure | Editions | Latest reserved layout | History | dvb-si | Class / note |
|---|---|---|---|---|---|---|
| TS 102 809 | `application_descriptor` | 01.01.01-01.03.01 | `rfu(5)=1` | - | `descriptors/ait/application.rs` | **A**  |
| TS 102 809 | `application_icons_descriptor` | 01.01.01-01.03.01 | `rfu(8)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 809 | `application_information_section` | 01.01.01-01.03.01 | `rfu(1)=1 r(2)=1 r(2)=1 rfu(4)=1 rfu(4)=1 rfu(4)=1` | - | `tables/ait.rs` | **A**  |
| TS 102 809 | `application_recording_descriptor` | 01.01.01-01.03.01 | `r(2)=1 rfu(8)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 809 | `application_signalling_descriptor` | 01.01.01-01.03.01 | `rfu(3)=1` | - | `descriptors/application_signalling.rs` | **A**  |
| TS 102 809 | `application_storage_descriptor` | 01.01.01-01.03.01 | `r(5)=1 r(1)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 809 | `authentication_message_section` | 01.03.01 | `rfu(1)=1 r(2)=1 r(2)=1 r(4)=1` | - | `tables/ait.rs` | **A**  |
| TS 102 809 | `certificate_collection_message_section` | 01.03.01 | `rfu(1)=1 r(2)=1 r(2)=1 r(4)=1 r(4)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 809 | `data_broadcast_descriptor` | 01.01.01-01.03.01 | `rfu(1)=1 r(8)=1 rfu(8)=1` | - | `descriptors/data_broadcast.rs` | **A**  |
| TS 102 809 | `graphics_constraints_descriptor` | 01.01.01-01.03.01 | `rfu(5)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 809 | `protection_message_descriptor` | 01.03.01 | `r(4)=1` | - | `descriptors/extension/protection_message.rs` | **A**  |
| TS 102 006 | `enhanced_message_descriptor` | 01.04.01 | `rfu(3)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 006 | `system_software_update_info` | 01.02.01-01.04.01 | `r(4)=1 r(2)=1` | 01.02.01..01.03.02: `-`; 01.04.01: `r(4)=1 r(2)=1` | `tables/unt.rs` | **A** reserved(4)/(2) rows exist in V1.2.1-V1.4.1 (the extraction missed the unlabelled-mnemonic rows of 1.2.1-1.3.2; verified in text); no value stated in any edition |
| TS 102 006 | `telephone_descriptor` | 01.02.01-01.04.01 | `rfu(2)=1 rfu(1)=1 rfu(1)=1` | - | `descriptors/telephone.rs` | **A**  |
| EN 301 192 | `data_carousel_info` | 01.01.01-01.08.01 | `r(6)=1 r(2)=1` | - | `tables/dsmcc.rs` | **A**  |
| EN 301 192 | `datagram_section` | 01.01.01-01.08.01 | `r(2)=1 r(2)=1` | - | `tables/mpe.rs` | **A**  |
| EN 301 192 | `higher_protocol_asynchronous_data_info` | 01.03.01-01.08.01 | `r(4)=1` | - | not implemented in dvb-si | **A**  |
| EN 301 192 | `mpe-fec_section` | 01.04.01-01.08.01 | `r(2)=1 rfu(8)=1 r(2)=1 rfu(5)=1` | - | `tables/mpe_fec.rs` | **A**  |
| EN 301 192 | `multiprotocol_encapsulation_info` | 01.01.01-01.08.01 | `r(3)=1` | - | `tables/mpe.rs` | **A**  |
| EN 301 192 | `object_carousel_info` | 01.01.01-01.08.01 | `r(6)=1 r(2)=1` | - | `tables/dsmcc.rs` | **A**  |
| EN 301 192 | `telephone_descriptor` | 01.03.01-01.08.01 | `rfu(2)=1 rfu(1)=1 rfu(1)=1` | - | `descriptors/telephone.rs` | **A**  |
| EN 301 192 | `time_slice_fec_identifier_descriptor` | 01.04.01-01.08.01 | `rfu(2)=1` | - | `descriptors/time_slice_fec_identifier.rs` | **A**  |
| TS 102 323 | `container_section` | 01.01.01-01.05.01 | `r(2)=1 r(2)=1` | - | `tables/container.rs` | **A**  |
| TS 102 323 | `content_identifier_section` | 01.01.01-01.05.01 | `r(2)=1 r(2)=1` | - | `tables/cit.rs` | **A**  |
| TS 102 323 | `link_info` | 01.01.01-01.05.01 | `rfu(2)=1 rfu(2)=1` | 01.01.01,01.02.01: `r(4)=1`; 01.03.01..01.05.01: `rfu(2)=1 rfu(2)=1` | `tables/rct.rs` | **C** **1.3.1 (2008-04) carves `default_icon_flag`(1)+`icon_id`(3) out of the 4 `reserved` bits before descriptor_loop_length**, restructures the head (`link_type`(4) rsv(2) `how_related_classification_scheme_id`(6) `term_id`(12) `group_id`(4) `precedence`(4)) and `number_items` 8->rsv(2)+6; 1.1.1/1.2.1: `link_type`(8) `how_related`(8). dvb-si: typed (rct.rs) |
| TS 102 323 | `rar_over_ip_descriptor` | 01.01.01-01.05.01 | `r(1)=1` | - | not implemented in dvb-si | **A**  |
| TS 102 323 | `related_content_section` | 01.01.01-01.05.01 | `r(2)=1 r(2)=1 r(4)=1 rfu(4)=1` | 01.01.01,01.02.01: `r(1)=1 rfu(2)=1 r(16)=1 r(2)=1 r(4)=1`; 01.03.01..01.05.01: `r(2)=1 r(2)=1 r(4)=1 rfu(4)=1` | `tables/rct.rs` | **C** **1.3.1 carves `table_id_extension_flag`(1) out of the byte-1 bit after section_syntax_indicator and replaces `reserved`(16) with `service_id`(16)**; 1.1.1/1.2.1: ssi, `reserved`(1), `reserved_future_use`(2), `reserved`(16). Old all-ones fill = "single service, ignore service_id" (compatible). dvb-si typed |
| TS 102 323 | `resolution_authority_notification_section` | 01.01.01-01.05.01 | `r(1)=1 r(2)=1 r(2)=1 r(4)=1 r(4)=1 r(4)=1 r(2)=1` | 01.01.01: `r(1)=1 r(2)=1 r(2)=1 r(4)=1 r(4)=1 r(4)=1 r(4)=1`; 01.02.01..01.05.01: `r(1)=1 r(2)=1 r(2)=1 r(4)=1 r(4)=1 r(4)=1 r(2)=1` | `tables/rnt.rs` | **A** 1.1.1 vs 1.2.1: one `reserved`(4) moved (before common_descriptors_length); no meaning added |
| TS 102 323 | `tva_id_descriptor` | 01.01.01-01.05.01 | `r(5)=1` | - | `descriptors/tva_id.rs` | **A**  |
| TS 102 772 | `mpe-ifec_section` | 01.01.01 | `r(2)=1 r(2)=1` | - | `tables/mpe_ifec.rs` | **A**  |
| TS 102 772 | `time_slice_fec_identifier_descriptor` | 01.01.01 | `rfu(2)=1` | - | `descriptors/time_slice_fec_identifier.rs` | **A**  |
| EN 303 560 | `downloadable_font_information_section` | 01.01.01 | `rfu(1)=1 r(2)=1 r(2)=1 rzfu(1)=0 rzfu(4)=0` | - | not implemented in dvb-si | **A**  |
| EN 303 560 | `linkage_descriptor` | 01.01.01 | `rzfu(8)=0` | - | `descriptors/linkage.rs` | **A**  |
| EN 303 560 | `ttml_subtitling_descriptor` | 01.01.01 | `rzfu(2)=0 rzfu(1)=0 rzfu(8)=0` | - | `descriptors/extension/ttml_subtitling.rs` | **A**  |
| EN 300 743 | `alternative_clut_segment` | 01.06.01 | `rzfu(4)=0` | - | not implemented in dvb-si | **A**  |
| EN 300 743 | `clut_definition_segment` | 01.02.01-01.06.01 | `r(4)=1 r(4)=1` | - | not implemented in dvb-si | **A**  |
| EN 300 743 | `disparity_signalling_segment` | 01.04.01-01.06.01 | `r(3)=1 r(5)=1 r(4)=1` | - | not implemented in dvb-si | **A**  |
| EN 300 743 | `display_definition_segment` | 01.03.01-01.06.01 | `r(3)=1` | - | not implemented in dvb-si | **A**  |
| EN 300 743 | `object_data_segment` | 01.02.01-01.06.01 | `r(1)=1` | - | not implemented in dvb-si | **A**  |
| EN 300 743 | `page_composition_segment` | 01.02.01-01.06.01 | `r(2)=1 r(8)=1` | - | not implemented in dvb-si | **A**  |
| EN 300 743 | `region_composition_segment` | 01.02.01-01.06.01 | `r(3)=1 r(2)=1 r(2)=1 r(4)=1` | - | not implemented in dvb-si | **A**  |

Not machine-compared (fetched, read for the reserved convention only): TS 102 366 (only frame-header tables in dvb-si), TS 101 154 (`AD_descriptor` Table E.1 `Reserved`=1111/0xFFFFFF; DTS-UHD BroadcastChunk `reserved_bits`(2) x2 - UNVERIFIED across editions), TS 102 825-4 USI (no reserved bits in V1.1.1/V1.2.1/V1.2.2), TS 102 825-9, TS 101 812 (descriptor 0x6F, AIT head), `nordig/*` (UNVERIFIED), `ts_102_772` (1 edition), `en_300_743` (not in dvb-si/src; dvb-subtitle).

Notes on class-A rows of TS 102 809 / TS 102 006 / EN 301 192 / TS 102 323: their reserved layouts are **identical in every fetched edition** (3, 4, 9 and 5 editions respectively) except where a row says otherwise; EN 301 192 from V1.4.1 adds field-level values for *conditionally* reserved fields (`max_burst_duration` = 0xFF when `time_slicing`=0; `frame_size`/`max_average_rate` and MPE-FEC fields "shall be set to '1' / 0xFF / 0x3FFFF when not used") - these are semantics, not `reserved` rows.

## 7. New typed fields that should be added because a newer edition defines them

Everything in scope that dvb-si currently models as `reserved` **and** that any edition defines is **already typed** (§4 "dvb-si" column) - verified for EN 300 468 up to V1.20.1 (no reserved-layout change between V1.19.1 and V1.20.1) and H.222.0 up to 08/2023. Remaining gaps, all in structures dvb-si does not implement (add when the descriptor is added; the field must then be typed, never raw):

| Spec / edition | Structure | Field | Bits | Semantics (source) |
|---|---|---|---|---|
| H.222.0 03/2017 | `HEVC_timing_and_HRD_descriptor` (extension_descriptor_tag 3) | `target_schedule_idx_not_present_flag`(1), `target_schedule_idx`(5) | 2017 clause 2.6.97 Table 2-113: `hrd_management_valid_flag`(1), flag(1), idx(5), `picture_and_timing_info_present_flag`(1) | 2014 had `reserved`(6) in place of flag+idx |
| H.222.0 08/2023 | `MPEG-H_3dAudio_descriptor` | `compatibleProfileSetsPresent` | 1 bit, immediately after `interactivityEnabled`; 2017-2021 `reserved`(9) -> 2023 flag(1)+`reserved`(8); gates `numCompatibleSets`/`CompatibleSetIndication` | 2021 text already references the flag without defining it (print error) |
| H.222.0 03/2017 | `adaptation_field_extension` (mpeg-ts) | `af_descriptor_not_present_flag` | 1 bit, directly after `seamless_splice_flag`; 2014: `reserved`(5) -> 2017: flag(1)+`reserved`(4); gates `af_descriptor` | mpeg-ts currently parses `ltw`/`piecewise_rate`/`seamless_splice` only and writes the old reserved fill |
| H.222.0 08/2023 | `Media_service_kind_descriptor`, `LCEVC_video_descriptor` | new descriptors | - | not in dvb-si |
| EN 300 468 V1.20.1 | AVS3-P3 NGA component types 0x20-0x22; `linkage_type` 0x0A (T/UWA 005) | code points only | - | no reserved-bit impact |
| EN 301 192 V1.8.1 (2025-06) | no reserved-layout change vs V1.7.1 | - | - | verified by per-edition diff |

## 8. Real-sender deviation patterns - which edition explains them

| Pattern (real-capture survey) | We emit | Sender writes | Older edition that matches the sender? | Newer edition? | Explained? |
|---|---|---|---|---|---|
| AC-3 0x6A `reserved_flags`(4), 15/62 deviate | **1111 (bug)** | 0000 (47/62) | **spec in ALL editions = 0**; the 15 senders writing 1111 apply the global rule | none | spec = sender majority; fix dvb-si |
| 0x6F bit before `application_type`, 15/52 | 1 | 0 | **TS 101 812 (MHP) V1.1.1-V1.3.2: 16-bit `application_type`, MSB = 0 for all allocated types** | TS 102 809 = rfu 1 | **yes (older edition)** |
| ext 0x06 supplementary_audio `reserved_future_use`(1), 37/47 | 1 | 0 | none (value 1 in V1.13.1-V1.20.1) | none | **no** (UNVERIFIED) |
| CA 0x09 `reserved`(3), 2/715 | 111 | 000 | none (H.222.0 x9) | - | no (noise) |
| SATELLITE 0x43 roll_off, 10/678 | 00 | 01 | none (00/reserved-zero V1.3.1-V1.20.1) | - | no (S2 roll-off in DVB-S descriptor) |
| TDT byte1 `0x00` vs `0x70` | 0x70 | 0x00 | bit 6 = `private_indicator` (user-definable) in H.222.0 x9 - explains bit 6 only | - | bit 6 yes; `reserved(2)` no |
| NIT long-form byte1 `0xB0` (test-016) | 0xF0 | 0xB0 | H.222.0 `private_indicator` = 0 allowed | - | **yes** (H.222.0-conformant) |
| SDT/EIT/AIT reserved bits 0 (TSDuck-generated samples, test-170 etc.), PAT `reserved(3)`=000, test-062 random bits | 1s | 0s/random | none | - | no (generator behaviour) |

## 9. Discrepancies in dvb-si found while auditing (not edition effects)

1. **`ac3.rs:349-350` writes `reserved_flags` = 1111; every edition says 0** (§3.1). One-line fix; parse already tolerant.
2. **`hevc_video.rs:26-30` emits byte-12 bit 3 = 0 and bit 2 = 1**: H.222.0 2017/2018/2021/2023 list `reserved`(2) then `HDR_WCG_idc`(2) (the 2014 draft had `reserved`(4) before the carve-out), so both bits are `reserved` = 1. The in-code rationale ("ISO/IEC 23001-2 ... leaves it unspecified") is not supported by any H.222.0 edition read here; **UNVERIFIED** whether another governing text (ISO/IEC 23008-2 Annex / TS 101 154 HEVC clause) says otherwise.
3. EN 300 468 **V1.17.1 errata** (self-correcting in V1.18.1): `telephone_descriptor` and `target_region_descriptor` (tertiary_region_code 8 vs 16 bits); H.222.0 1995/2000 `multiplex_buffer_utilization` 14-bit upper bound; H.222.0 2021 `MPEG-H_3dAudio_descriptor` references an undefined flag. dvb-si follows the corrected layouts.
4. `satellite_delivery_system_descriptor` and `s2_satellite_delivery_system_descriptor`: dvb-si emits the V1.16+ forms (reserved-zero); a pre-V1.16 low-priority-stream sender loses `backwards_compatibility_indicator` on round-trip (class C row 4). Decide whether to keep the bit as an untyped-but-preserved field (`Reserved` policy) - that is a product decision, not a spec question.
5. `mpeg-ts` `AdaptationFieldExtension` (row 13) writes the pre-2017 reserved fill and drops `af_descriptor_not_present_flag`.

## 10. UNVERIFIED / limits

* H.222.0 **04/2025** (in force) not obtainable (paid); H.222.0 / 13818-1 ISO twin texts assumed identical to the ITU texts.
* ETS 300 468 (1995, 1997) and ETSI editions not on the `_60` publication list; EN 300 468 draft/interim texts not read.
* NorDig, D-Book, ATSC, Freeview/HbbTV profiles (TS 102 796) not fetched - they can legitimately mandate other values and may explain the supplementary_audio / CA senders.
* Bit widths from `pdftotext -layout` except the five image-checked pages (§0.4); the extraction tool mislabels blocks whose syntax tables share a page (rows marked **E**).
* Real-capture survey not re-run; only its published pattern list used.
* `emit` column is by convention except where cited (`ac3.rs`, `hevc_video.rs`, `aac.rs`, `satellite_delivery_system.rs:399`, `application_signalling.rs:133-136`, `ca.rs:63-91`, `extension/supplementary_audio.rs:82`, and the `RESERVED_*` masks listed in §6).
* The root cause of the supplementary_audio, CA and roll_off deviations is **unknown**; no edition supports them.
