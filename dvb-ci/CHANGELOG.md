# Changelog

All notable changes to `dvb-ci` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/); this crate adheres to semantic
versioning.

## Unreleased

### Added

The remaining EN 50221 application objects, completing every Table 58 `apdu_tag`.
All are symmetric `Parse`/`Serialize` with biting round-trip + mutation tests and
PDF worked-example fixtures; serialize rebuilds every byte from typed fields (no
raw passthrough). The `_last`/`_more` chaining pairs are modeled as a single
struct with a `more: bool` that selects the tag and dispatches both tags to the
same `AnyApdu` variant.

- **Host Control** (§8.5.1, Tables 27-30): `Tune`, `Replace`, `ClearReplace`,
  `AskRelease`.
- **High-level MMI** (§8.6.5, Tables 46-51): `Text` (last/more), `Enq`, `Answ`
  (with `AnswId`), `Menu` (last/more), `MenuAnsw`, `List` (last/more); the nested
  `TEXT()` component is reused across Menu/List.
- **Low-level / display / scene / download MMI** (§8.6.2-8.6.4, Tables 34-45):
  `DisplayControl`, `DisplayReply` (graphics-characteristics / character-table
  list / mmi_mode_ack branches), `KeypadControl`, `Keypress`, `SubtitleSegment`
  (last/more), `DisplayMessage`, `SceneEndMark`, `SceneDoneMessage`,
  `SceneControl`, `SubtitleDownload` (last/more), `FlushDownload`, `DownloadReply`
  — with the `DisplayControlCmd`, `MmiMode`, `DisplayReplyId`, `KeypadControlCmd`,
  `DisplayMessageId` and `DownloadReplyId` value enums.
- **Low-Speed Communications** (§8.7.1, Tables 52-56): `CommsCmd` (with nested
  `ConnectionDescriptor`), `ConnectionDescriptor`, `CommsReply`, `CommsSend`
  (last/more), `CommsRcv` (last/more) — with the `CommsCommandId`,
  `ConnectionDescriptorType` and `CommsReplyId` value enums.

## 0.1.0 — 2026-06-20

### Added

Initial release — DVB Common Interface (ETSI EN 50221) wire protocol, `#![no_std]`
(+ `alloc`). Every type is symmetric `dvb_common::Parse` / `Serialize` with
length fields computed from content and biting round-trip tests.

- **APDU framework**: `ApduTag` (3-byte ASN.1 tag) with named Table 58 constants;
  `length::{decode, encode_into, encoded_len}` (the EN 50221 ASN.1-style
  `length_field`, §7 Table 1); `AnyApdu` tag dispatch built from a single
  `declare_apdus!` list (Def-trait + drift test, mirroring `dvb-si`/`dvb-scte35`);
  4-octet `ResourceId` with the public Table 57 resource constants.
- **CA support** (§8.4.3): `ca_info_enq` / `ca_info`, `ca_pmt`, `ca_pmt_reply`,
  with the `ca_pmt_list_management`, `ca_pmt_cmd_id` and `CA_enable` value enums.
- **CA_PMT builder** (`builder::build_ca_pmt`): projects a `dvb-si` `PmtSection`
  into a `ca_pmt`, stripping all non-CA descriptors and keeping `CA_descriptor`s
  (tag `0x09`) at programme + ES level per §8.4.3.4. Tested against a real
  TSDuck-captured PMT and a multi-ES CA-protected PMT.
- **Application Information** (§8.4.2): `application_info_enq` / `application_info`
  / `enter_menu`, with the `application_type` enum.
- **Resource Manager** (§8.4.1): `profile_enq` / `profile` (reply) /
  `profile_change`.
- **Date-Time** (§8.5.2): `date_time_enq` / `date_time` (optional signed
  `local_offset`).
- **MMI** (§8.6.2.1): `close_mmi` with the `close_mmi_cmd_id` enum.
- **Session layer SPDUs** (§7.2): `open` / `create` / `close` session
  request + response, `session_number`, and the `session_status` value enum.
- **Transport layer TPDUs** (Annex A §A.4.1): C_TPDU / R_TPDU (with the mandatory
  Status Byte), the connection-management objects (Create/Delete/Request/New_T_C,
  C_T_C_Reply/D_T_C_Reply, T_C_Error) and `SB_value`.
- `examples/`: `build_ca_pmt` and `parse_apdu`.

### Deferred to a follow-up

- MMI **high-level** objects (text / enq / answ / menu / list, Tables 46-51) and
  the MMI low-level/display objects.
- **Host Control** (tune / replace) and **Low-Speed Communications** resources.

Their `apdu_tag`s are retained in `docs/en_50221/apdu-tag-values.md`; until typed
they parse as `AnyApdu::Unknown` (raw body preserved, lossless round-trip). CI+
crypto (the CC resource) and the PC-Card hardware transport remain out of scope.
