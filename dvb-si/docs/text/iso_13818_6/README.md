# ISO/IEC 13818-6 §11 BIOP — DVB object-carousel profile (per ETSI TR 101 202)

The BIOP (Broadcast Inter-ORB Protocol) message syntax is normatively defined
in ISO/IEC 13818-6 §11, which is a **paid ISO standard and is not vendored**.
Everything below is transcribed from the **vendored** ETSI guideline
`specs/etsi_tr_101_202_v01.02.01_dvb_data_broadcasting_guidelines.pdf`
(TR 101 202 §4.7), which reproduces the full byte-level syntax tables for the
DVB-profiled subset of BIOP and is the authoritative source for this crate's
`carousel::biop` implementation. Section/table/page numbers below are
TR 101 202's. Where TR 101 202 subordinates to the ISO standard, the
**DVB profile** constraints (alias type_ids, big-endian, fixed tags) make the
ambiguous cases inert on-air — see "CDR / alignment" at the bottom.

This layer sits on top of the DSM-CC framing already transcribed in
`iso_13818_6_carousel.md` (DSI / DII / DDB sections + module reassembly). BIOP
messages live inside the **complete modules** that `ModuleReassembler` produces.

# ISO/IEC 13818-6 DSM-CC download protocol — carousel message syntax

**Provenance.** ISO/IEC 13818-6:1998 is a paid ISO standard and cannot be
vendored into this repository. The message layouts below are hand-transcribed
from the well-known DSM-CC U-N download protocol (ISO/IEC 13818-6 §7.2
message headers, §7.3 download messages, §6.1 compatibilityDescriptor) and are
**not** backed by an in-repo PDF. They are instead cross-checked against three
independent in-repo grounds:

1. **A live capture** — `dvb-si/tests/fixtures/m6-single.ts` (French TNT, M6
   HbbTV carousel, PID 0x00AB) carries a complete DSI and DII, plus the start
   of a DDB (the capture ends mid-section), whose bytes decode exactly per
   these layouts (protocolDiscriminator 0x11, dsmccType 0x03, messageId
   0x1006/0x1002/0x1003, the TR 101 202 §4.7.9 transactionId rules, 20×0xFF
   serverId, DII↔DDB downloadId linkage from the DDB header prefix). The
   `carousel_fixture` integration tests pin this; full DDB/module reassembly
   is exercised end-to-end with synthetic sections built by our serializers.
2. **TR 101 202 v1.2.1** (vendored) — §4.6/§4.7.5 field semantics + DVB
   guidelines, Table 4.1 (transactionId sub-fields), Table 4.1a / 4.16
   (DSM-CC section field encoding).
3. **TS 102 006 v1.7.1** (vendored) — Table 15 reproduces the
   `compatibilityDescriptor()` structure; Table 6 (GroupInfoIndication) is the
   SSU profile's DSI `privateData`.

Carriage: U-N control messages (DSI, DII) ride DSM-CC sections with
**table_id 0x3B**; download data messages (DDB) ride **table_id 0x3C**
(EN 301 192 / TR 101 202 Table 4.1a). The section framing is `tables/dsmcc.rs`;
the payload parsing is `carousel/`.
