# CI Plus / DVB-CI extensions — spec transcription

Render-verified Markdown transcription of the **freely-redistributable** DVB
Common Interface extension / CI Plus specs, for the planned `dvb-ci` crate
(issue #288). This is the spec-md-first phase: docs only, no Rust.

Each table/value below was read from a **rendered PDF page image** (Read with
`pages=`, never pdftotext — column misalignment) and transcribed exactly, with
section + Table + PDF-page citations. Every file carries a
`_Source: ... render-verified_` line.

## Source specs

| File prefix | Source PDF | Role |
|-------------|-----------|------|
| `resource-manager-v2`, `application-info-v2`, `input-modules`, `status-query`, `power-manager`, `event-manager`, `application-mmi`, `copy-protection`, `software-download`, `ca-pipeline`, `resource-ids` | `specs/etsi_ts_101_699_v01.01.01_dvb_ci_extensions.pdf` (TS 101 699 V1.1.1, DVB CI Extensions) | The APDU syntax-table source — prints full object syntax for the extension resources. |
| `fragment-header` | `specs/etsi_ts_103_605_v01.01.01_dvb_ci_plus.pdf` (TS 103 605 V1.1.1, CI Plus over USB) | Prints the media-interface fragment header (Table 3) + IV/key-id descriptor allocation. Defers command-interface APDUs to the proprietary CI Plus spec. |
| (not transcribed) | `specs/dvb_r206-001_v1_ci_guidelines.pdf` (R206-001, CI guidelines) | Informative rationale + implementation guidance for EN 50221; carries NO new resource syntax tables (its §9 re-discusses EN 50221 resources; Annex A is CA_PMT flowcharts). Not a syntax-table source. |
| (not transcribed) | `specs/ci_plus_specification_v1.4.3.pdf` (proprietary, gitignored) | Detail reference only. The content_control (`cc_*`), SAC, host language/country, operator profile and CAM-firmware-upgrade APDU layouts live here — NOT redistributable, NOT transcribed. |

## Files

- **`resource-ids.md`** — master registry: TS 101 699 Table 87. All extension
  resources, their 32-bit `resource_identifier` values (class/type/version
  breakdown), and every APDU `apdu_tag` with transfer direction.
- **`fragment-header.md`** — TS 103 605 §7.7 media-interface fragment header
  (Table 3), the IV/key-id descriptor allocation (Table 4), the field-usage
  matrix (Table 5), and notes on the four command-interface APDUs that
  TS 103 605 defers to the proprietary CI Plus spec.
- **`resource-manager-v2.md`** — Resource Manager v2 (§4.2.1): Profile
  Enquiry/Reply/Changed, Module ID Send/Command. `0x00010042`.
- **`application-info-v2.md`** — Application Information v2 (§5): extended
  `application_type` enum + unrecognized-type semantics. `0x00020042`.
- **`input-modules.md`** — StreamInput (`0x00801ii1`), Generic Service Gateway,
  Broadcast Service Gateway (`0x00811ii1`) — §6.1 Type A/B input modules.
- **`status-query.md`** — Status Query (`0x00211ii1`) §6.2 + Audience metering.
- **`power-manager.md`** — Power manager (`0x00220041`) §6.3.
- **`event-manager.md`** — Event Manager (`0x00231ii1`) §6.4.
- **`application-mmi.md`** — Application MMI (`0x00410041`) §6.5.
- **`copy-protection.md`** — Copy protection (`0x00041ii1`) §6.6.
- **`software-download.md`** — Download resource (`0x00051041`) §6.7 — the
  first-gen CAM firmware-download story (DSM-CC User-to-Network messages).
- **`ca-pipeline.md`** — CA pipeline resource (`0x00061ii1`) §6.8.

## Out of scope / not redistributable

The CI Plus **content control** path (mutual authentication, SAC, key ladder,
`cc_*` / host language-country / operator-profile / `cam_firmware_upgrade`
resource APDUs) is defined in the proprietary CI Plus v1.4.x specification, not
in TS 101 699 or TS 103 605. Those layouts are NOT transcribed here. TS 103 605
§6.3 only references them (see `fragment-header.md` bottom section).

## Re-check list (flagged, not invented)

Items flagged with ⚠ in the docs — verify against the PDF before encoding:

- `resource-ids.md` — Download resource: Table 87 prints hex `0x000510041`
  (an extra digit); the class/type/version 5/1/1 packs to `0x00051041`. Confirm
  the literal on p. 80.
- `fragment-header.md` — IV descriptor (`0xD0`) and key-id descriptor (`0xD1`)
  byte widths are NOT in TS 103 605; defined in TS 103 205 §7.5.5.4.2 Table 46
  and §7.5.5.4.3 Table 47 (not vendored). Opaque blobs of spec-defined length.
- Several TS 101 699 tables carry an editorial caption typo (the `*Ack` tables
  reuse the `*Req` caption — e.g. Status Query Table 41 captioned
  "DeliverySystemInfoReq syntax", input-module Table 14 captioned
  "DeliverySystemInfoReq syntax"). These are noted inline; the table *body* and
  apdu_tag are authoritative, transcribed faithfully.
- See each file's inline ⚠ notes for the remaining low-confidence sub-field
  widths (input-modules Table 28 `running_status` 3-bit-vs-prose-6-bit;
  status-query port-profile sub-fields; software-download §6.7.5.4 private-data
  length-prefix width).
