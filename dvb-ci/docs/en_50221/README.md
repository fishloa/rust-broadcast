# EN 50221 — DVB Common Interface spec transcription

Render-verified Markdown transcription of **EN 50221:1997** (Common Interface
Specification for Conditional Access and other Digital Video Broadcasting Decoder
Applications), source PDF `specs/dvb_en50221_v1_common_interface.pdf` (86 pages). These
docs are the authoritative oracle for the planned `dvb-ci` crate implementation.

Each table/value below was read from a rendered PDF page image (not pdftotext) and
transcribed exactly, with section + PDF page citations. Every file carries a
`_Source: ... render-verified_` line.

## Scope

**IN scope** — the wire / protocol layers of the command interface:
- Transport layer (TPDU framing + tag values)
- Session layer (SPDU framing + tag values, open/create/close session)
- Application layer common framing (APDU TLV coding, length field, resource identifiers)
- Resource objects: Resource Manager, Application Information, Conditional Access
  Support (ca_info / **ca_pmt / ca_pmt_reply**), Date-Time, MMI.

**OUT of scope** (for the planned crate):
- The physical PC-Card / PCMCIA hardware transport (electrical, COR, interrupts) —
  only the TPDU wire framing in Annex A is transcribed, not the hardware mechanics.
- CI+ crypto (the Content Control / CC resource, authentication, key ladder) — that
  lives in the separate CI+ specs (`ci_plus_specification_v1.4.3.pdf`,
  `etsi_ts_103_605`), not EN 50221.
- Host Control (tune/replace) and Low-Speed Communications resources — listed in the
  tag/resource tables for completeness but not individually transcribed (lower
  priority; add later if the crate needs them).

## Files

| File | Contents | Source |
|------|----------|--------|
| `apdu-tag-values.md`    | **Full Table 58 APDU tag-value list** + primitive tag coding (Figure 16) | §8.8.2, pp. 56-57 |
| `ca-pmt.md`             | ca_pmt object (Table 25) + ca_pmt_list_management + ca_pmt_cmd_id values | §8.4.3.4, pp. 30-31 |
| `ca-pmt-reply.md`       | ca_pmt_reply object (Table 26) + CA_enable values | §8.4.3.5, p. 32 |
| `ca-info.md`            | ca_info_enq / ca_info (Tables 23-24) | §8.4.3.1-2, p. 29 |
| `application-info.md`   | application_info_enq / application_info / enter_menu (Tables 20-22) + application_type | §8.4.2, pp. 27-28 |
| `resource-manager.md`   | profile_enq / profile (reply) / profile_changed (Tables 17-19) | §8.4.1, pp. 26-27 |
| `spdu-session.md`       | SPDU coding (Table 4) + session tag values (Table 14) + open/create/close session (Tables 5-13) + status values | §7.2.4-7.2.7, pp. 19-23 |
| `apdu-coding.md`        | Length field (Table 1) + APDU coding (Table 16) + chaining | §7 + §8.3, pp. 11, 24-25 |
| `resource-identifier.md`| resource_identifier coding (Table 15) + resource identifier values (Table 57) + low-speed comms types | §8.2.2 + §8.8.1, pp. 24, 54 |
| `tpdu-framing.md`       | TPDU tag values (Table A.16) + C_TPDU/R_TPDU + SB_value + connection mgmt objects (Annex A) | Annex A §A.4.1, pp. 63-70 |
| `datetime.md`           | date_time_enq / date_time (Tables 31-32) | §8.5.2, p. 35 |
| `mmi-close.md`          | close_mmi (Table 33) + close_mmi_cmd_id values | §8.6.2.1, p. 36 |
| `mmi-high-level.md`     | text / enq / answ / menu / menu_answ / list (Tables 46-51) + answ_id values | §8.6.5, pp. 47-50 |

## Headline values (quick reference)

- All public `apdu_tag`s are 3 bytes beginning `0x9F`. CA Support: ca_info_enq
  `9F 80 30`, ca_info `9F 80 31`, **ca_pmt `9F 80 32`**, **ca_pmt_reply `9F 80 33`**.
- `spdu_tag`s (1 byte): session_number `90`, open_session_request `91`,
  open_session_response `92`, create_session `93`, create_session_response `94`,
  close_session_request `95`, close_session_response `96`.
- `tpdu_tag`s (1 byte): SB `80`, RCV `81`, create_t_c `82`, c_t_c_reply `83`,
  delete_t_c `84`, d_t_c_reply `85`, request_t_c `86`, new_t_c `87`, t_c_error `88`,
  data_last `A0`, data_more `A1`.
- Public resource IDs: Resource Manager `00010041`, Application Information
  `00020041`, Conditional Access Support `00030041`, Host Control `00200041`,
  Date-Time `00240041`, MMI `00400041`, Low-Speed Comms `0060xxx1`.

## Spec typos noted during transcription (for the implementer)

- The PDF labels two consecutive transport tables both as **"Table A.4"** (Create_T_C
  and C_T_C_Reply) — see `tpdu-framing.md`.
- The PDF labels **both Table 31 and Table 32** as "Date-Time Enquiry object coding";
  Table 32 is really the `date_time()` object — see `datetime.md`.

## Cross-reference specs

- `specs/dvb_r206-001_v1_ci_guidelines.pdf` — DVB implementation guidelines (R206-001).
- `specs/etsi_ts_101_699_v01.01.01_dvb_ci_extensions.pdf` — CI extensions (TS 101 699).
- External references cited by EN 50221: [1] ISO/IEC 13818-1 (MPEG-2 Systems),
  [4] ETSI EN 300 468 (DVB SI, character coding), [5] ETSI TS 101 162 (CA system IDs).
